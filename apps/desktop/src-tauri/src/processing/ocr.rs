//! OCR engine behind the batch queue (plan-lote.md §3.2, §8).
//!
//! The executor computes one asset through the EXISTING pipelines without
//! changing them — lean always routes whole-file GLM (native PDFs included,
//! matching current behavior), Pro reuses `process_job` with lazily owned
//! local engines — and returns a plain [`OcrComputeOutput`]. The supervisor
//! checkpoints it and publishes extraction/layout rows atomically with the
//! task receipt via [`publish_ocr_output`].
//!
//! Crash behavior: a "full" checkpoint with a matching fingerprint makes a
//! resumed run skip compute entirely, so a crash between compute and COMMIT
//! never pays the provider twice. Intra-document resume (per-page local
//! OCR) is future work: unsplit multipage PDFs compute whole-document like
//! today, while split page imports already resume per page as separate
//! assets.

use std::path::PathBuf;
#[cfg(feature = "paddle-ocr")]
use std::sync::Mutex;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use super::repository::NewCheckpoint;
use super::scheduler::{ClaimedTask, ExecCtx, ExecOutput, ExecResult, Executor, StopFlag};
use crate::db::open::open_archive_connection;
use crate::ocr;

/// Whole-asset OCR result in plain data. Checkpoint-serializable: no
/// Pro-only types cross this boundary, only opaque layout JSON strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrComputeOutput {
    pub text: String,
    pub method: String,
    /// `"text"` or `"no_text"` — an empty success is final, never requeued.
    pub outcome: String,
    pub regions_json: Option<String>,
    pub blocks_json: Option<String>,
    pub layout_model: String,
    pub image_width: u32,
    pub image_height: u32,
    pub provider: String,
    pub page_count: i64,
}

/// Writes extraction/layout rows for one asset. Runs INSIDE the commit
/// transaction (ambient, never begins its own) so canonical rows and the
/// task receipt confirm together. Mirrors the worker's atomic path exactly:
///
/// - deterministic `ext-{asset}` / `lay-{asset}` UPSERTs (UPDATE-only, no
///   sync tombstones),
/// - layout replaced or deleted to match the output,
/// - OCR-correction state invalidated with the new text.
pub fn publish_ocr_output(
    conn: &Connection,
    asset_id: &str,
    output: &OcrComputeOutput,
) -> Result<(), String> {
    ocr::save_extraction_row(conn, asset_id, &output.text, &output.method)?;
    match (&output.regions_json, &output.blocks_json) {
        (Some(regions), Some(blocks)) => ocr::save_layout_rows(
            conn,
            asset_id,
            regions,
            blocks,
            &output.layout_model,
            output.image_width,
            output.image_height,
        )?,
        _ => ocr::delete_layout(conn, asset_id)?,
    }
    crate::llm::ocr_correction::clear_asset_state(conn, asset_id)?;
    Ok(())
}

/// Resolves the OCR mode a new batch pins for its tasks from the saved OCRH
/// setting: `local` → light, `glm_ocr`/`auto` → high (auto tries GLM first
/// and falls back, exactly like the previous worker). Frozen at admission —
/// a later settings change does not rewrite running batches.
pub(crate) fn resolve_batch_ocr_mode(conn: &Connection) -> String {
    let mode: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'ocrh_mode'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "local".to_string());
    match mode.to_lowercase().as_str() {
        "glm_ocr" | "high" | "auto" => "high".to_string(),
        _ => "light".to_string(),
    }
}

pub(crate) fn ocr_task_contract(mode: &str) -> String {
    format!("ocr:{mode}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// Maps engine failures to the queue's retry vocabulary. Transient transport
/// and provider overload retry with backoff; missing credentials or models
/// park as configuration (a human fixes settings, then resumes); corrupt or
/// unreadable inputs fail terminally with their cause preserved.
pub(crate) fn map_ocr_error(error: &str) -> ExecOutput {
    for prefix in ["timeout:", "connection:", "rate_limited:", "provider_5xx:"] {
        if error.starts_with(prefix) {
            return ExecOutput::Retryable {
                code: prefix.trim_end_matches(':').to_string(),
                message: error.to_string(),
            };
        }
    }
    if error.starts_with("configuration:") {
        return ExecOutput::Blocked {
            code: "configuration_required".to_string(),
            message: error.to_string(),
        };
    }
    if error.contains("Failed to read") || error.contains("no such file") {
        return ExecOutput::Fatal {
            code: "file_missing".to_string(),
            message: error.to_string(),
        };
    }
    ExecOutput::Fatal {
        code: "ocr_failed".to_string(),
        message: error.to_string(),
    }
}

#[cfg(feature = "paddle-ocr")]
struct LocalEngines {
    provider: std::sync::Arc<dyn ocr::provider::OcrProvider>,
    paddle_vl: Mutex<Option<ocr::paddle_vl::PaddleVlEngine>>,
}

/// Queue-owned OCR executor. Local engines initialize lazily on first local
/// use and are reused afterwards — a remote-only batch never pays for (or
/// fails on) Paddle initialization, unlike the retired worker which blocked
/// every job behind it.
pub struct OcrExecutor {
    app: AppHandle,
    db_path: PathBuf,
    #[cfg(feature = "paddle-ocr")]
    local: Mutex<Option<LocalEngines>>,
}

impl OcrExecutor {
    pub fn new(app: AppHandle, db_path: PathBuf) -> Self {
        Self {
            app,
            db_path,
            #[cfg(feature = "paddle-ocr")]
            local: Mutex::new(None),
        }
    }

    fn settings_conn(&self) -> Result<Connection, String> {
        open_archive_connection(&self.db_path)
    }

    fn read_asset(&self, asset_id: &str) -> Result<(String, String), String> {
        let conn = self.settings_conn()?;
        conn.query_row(
            "SELECT path, type FROM assets WHERE id = ?1",
            [asset_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|e| format!("Failed to read asset {asset_id}: {e}"))
    }

    /// Returns the staged "full" output when a resumed run already computed
    /// this exact input. Compute is the expensive half; the checkpoint makes
    /// the second attempt free.
    fn staged_output(&self, task: &ClaimedTask) -> Result<Option<OcrComputeOutput>, String> {
        let conn = self.settings_conn()?;
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM processing_checkpoints
                 WHERE task_id = ?1 AND unit_key = 'full' AND input_fingerprint = ?2 AND contract_hash = ?3",
                rusqlite::params![task.task_id, task.input_fingerprint, task.contract_hash],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(format!("Failed to read checkpoints of {}: {other}", task.task_id)),
            })?;
        payload
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|e| format!("Stored OCR checkpoint is corrupt, recomputing: {e}"))
            })
            .transpose()
            .or_else(|e| {
                eprintln!("[processing] {e}");
                Ok(None)
            })
    }

    fn page_count(&self, asset_type: &str, bytes: &[u8]) -> i64 {
        if asset_type != "pdf" || !bytes.starts_with(b"%PDF-") {
            return 1;
        }
        // Best effort: progress granularity only, never eligibility.
        crate::ocr::pdf::pdf_page_count(bytes).unwrap_or(1).max(1) as i64
    }
}

impl Executor for OcrExecutor {
    fn kinds(&self) -> &[&str] {
        &["ocr"]
    }

    fn run(&self, ctx: &ExecCtx, task: &ClaimedTask, _stop: &StopFlag) -> ExecResult {
        let _ = ctx;
        let failed = |output: ExecOutput| ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: None,
            output,
        };
        if let Some(staged) = self.staged_output(task).unwrap_or(None) {
            let checkpoint = full_checkpoint(task, &staged);
            return ExecResult {
                checkpoints: vec![checkpoint],
                progress_total: Some(staged.page_count),
                engine_output: Some(super::scheduler::EngineOutput::Ocr(staged)),
                output: ExecOutput::Success {
                    outcome: "text".to_string(),
                    receipt: "{}".to_string(),
                },
            };
        }
        let (stored_path, asset_type) = match self.read_asset(&task.asset_id) {
            Ok(asset) => asset,
            Err(error) => {
                return failed(ExecOutput::Fatal {
                    code: "source_deleted".to_string(),
                    message: error,
                })
            }
        };
        let asset_path =
            match crate::path_utils::resolve_asset_path_at_boundary(&stored_path, &self.app) {
                Ok(path) => path,
                Err(error) => {
                    return failed(ExecOutput::Fatal {
                        code: "file_missing".to_string(),
                        message: error,
                    })
                }
            };
        let bytes = match std::fs::read(&asset_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return failed(ExecOutput::Fatal {
                    code: "file_missing".to_string(),
                    message: format!("Failed to read {asset_path}: {error}"),
                })
            }
        };
        let page_count = self.page_count(&asset_type, &bytes);
        let computed = self.compute(task, &asset_path, &asset_type, &bytes);
        let output = match computed {
            Ok(output) => output,
            Err(error) => return failed(map_ocr_error(&error)),
        };
        let checkpoint = full_checkpoint(task, &output);
        ExecResult {
            checkpoints: vec![checkpoint],
            progress_total: Some(page_count.max(1)),
            engine_output: Some(super::scheduler::EngineOutput::Ocr(output)),
            output: ExecOutput::Success {
                outcome: "text".to_string(),
                receipt: "{}".to_string(),
            },
        }
    }
}

fn full_checkpoint(task: &ClaimedTask, output: &OcrComputeOutput) -> NewCheckpoint {
    let payload = serde_json::to_string(output).unwrap_or_else(|_| "{}".to_string());
    let checksum = sha256_hex(payload.as_bytes());
    NewCheckpoint {
        unit_key: "full".to_string(),
        input_fingerprint: task.input_fingerprint.clone(),
        contract_hash: task.contract_hash.clone(),
        payload,
        payload_checksum: checksum,
    }
}

#[cfg(not(feature = "paddle-ocr"))]
impl OcrExecutor {
    fn compute(
        &self,
        task: &ClaimedTask,
        _asset_path: &str,
        asset_type: &str,
        bytes: &[u8],
    ) -> Result<OcrComputeOutput, String> {
        let conn = self.settings_conn()?;
        let api_key = ocr::get_glm_ocr_api_key(&conn);
        if api_key.is_empty() {
            return Err("configuration: GLM-OCR no está configurado. Andá a Configuración > OCRH y cargá una API key antes de usar OCR.".to_string());
        }
        let method = if asset_type == "pdf" {
            "pdf_glm_ocr"
        } else {
            "glm_ocr"
        };
        ocr::emit_progress(&self.app, &task.asset_id, 25, "reading");
        let output = tauri::async_runtime::block_on(ocr::process_with_glm_ocr_provider(
            bytes,
            &task.asset_id,
            &self.app,
            &api_key,
            method,
        ))?;
        to_compute_output(
            output,
            "glm",
            &bytes,
            asset_type,
            self.page_count(asset_type, bytes),
        )
    }
}

#[cfg(feature = "paddle-ocr")]
impl OcrExecutor {
    fn ensure_local(&self) -> Result<(), String> {
        let mut guard = self
            .local
            .lock()
            .map_err(|e| format!("OCR engine lock poisoned: {e}"))?;
        if guard.is_some() {
            return Ok(());
        }
        let model_dir = ocr::resolve_paddle_model_dir(&self.app);
        match ocr::paddle::PaddleOcrProvider::new(model_dir) {
            Ok(provider) => {
                *guard = Some(LocalEngines {
                    provider: std::sync::Arc::new(provider),
                    paddle_vl: Mutex::new(None),
                });
                Ok(())
            }
            Err(error) => Err(format!(
                "configuration: PaddleOCR liviano no está disponible: {error}"
            )),
        }
    }

    fn compute(
        &self,
        task: &ClaimedTask,
        asset_path: &str,
        asset_type: &str,
        bytes: &[u8],
    ) -> Result<OcrComputeOutput, String> {
        let conn = self.settings_conn()?;
        // Remote-first exactly like the retired worker: explicit GLM goes
        // remote, auto tries remote and falls back to local, local stays
        // local. Remote jobs never initialize Paddle.
        let requested_high = task.contract_hash == ocr_task_contract("high");
        let ocrh_mode = ocr::get_ocrh_mode(&conn);
        let api_key = ocr::get_glm_ocr_api_key(&conn);
        let use_remote = requested_high
            && (ocrh_mode == ocr::OCRH_MODE_GLM_OCR
                || (ocrh_mode == ocr::OCRH_MODE_AUTO && !api_key.is_empty()));
        if use_remote {
            let method = if asset_type == "pdf" {
                "pdf_glm_ocr"
            } else {
                "glm_ocr"
            };
            let output = tauri::async_runtime::block_on(ocr::process_with_glm_ocr_provider(
                bytes,
                &task.asset_id,
                &self.app,
                &api_key,
                method,
            ));
            match output {
                Ok(output) => {
                    return to_compute_output(
                        output,
                        "glm",
                        bytes,
                        asset_type,
                        self.page_count(asset_type, bytes),
                    )
                }
                Err(error) => {
                    eprintln!(
                        "[processing] GLM-OCR failed for {}, falling back to local: {error}",
                        task.asset_id
                    );
                    if ocrh_mode == ocr::OCRH_MODE_GLM_OCR {
                        return Err(error);
                    }
                }
            }
        }
        // Anything not served remotely runs the local pipeline, which needs
        // the light provider (High degrades to it when PaddleVL is absent,
        // exactly like the retired worker).
        self.ensure_local()?;
        let guard = self
            .local
            .lock()
            .map_err(|e| format!("OCR engine lock poisoned: {e}"))?;
        let engines = guard
            .as_ref()
            .ok_or_else(|| "configuration: PaddleOCR liviano no está disponible".to_string())?;
        if requested_high {
            let mut vl = engines
                .paddle_vl
                .lock()
                .map_err(|e| format!("OCR engine lock poisoned: {e}"))?;
            if vl.is_none() {
                match ocr::paddle_vl::create_paddle_vl_engine_result(&self.app, &self.db_path) {
                    Ok(engine) => *vl = Some(engine),
                    Err(error) => {
                        eprintln!(
                            "[processing] PaddleOCR-VL unavailable, falling back to light: {error}"
                        );
                    }
                }
            }
            let job = ocr::OcrJob {
                asset_id: task.asset_id.clone(),
                asset_path: asset_path.to_string(),
                asset_type: asset_type.to_string(),
                mode: ocr::OcrMode::High,
            };
            let output = tauri::async_runtime::block_on(ocr::process_job(
                &engines.provider,
                &conn,
                &job,
                &self.app,
                vl.as_ref(),
            ))?;
            return to_compute_output(
                output,
                "paddle",
                bytes,
                asset_type,
                self.page_count(asset_type, bytes),
            );
        }
        let job = ocr::OcrJob {
            asset_id: task.asset_id.clone(),
            asset_path: asset_path.to_string(),
            asset_type: asset_type.to_string(),
            mode: ocr::OcrMode::Light,
        };
        // Borrow-check: re-fetch the vl guard as immutable for process_job.
        let vl_guard = engines
            .paddle_vl
            .lock()
            .map_err(|e| format!("OCR engine lock poisoned: {e}"))?;
        let output = tauri::async_runtime::block_on(ocr::process_job(
            &engines.provider,
            &conn,
            &job,
            &self.app,
            vl_guard.as_ref(),
        ))?;
        drop(vl_guard);
        to_compute_output(
            output,
            "paddle",
            bytes,
            asset_type,
            self.page_count(asset_type, bytes),
        )
    }
}

/// Flattens a pipeline output into checkpoint/plain data. `pdf_pages` never
/// occurs on this path (page fan-out stays dormant by design); a future
/// pagewise pipeline sets it and extends publish accordingly.
fn to_compute_output(
    output: ocr::ProcessedOcrOutput,
    provider: &str,
    _bytes: &[u8],
    asset_type: &str,
    page_count: i64,
) -> Result<OcrComputeOutput, String> {
    if output.pdf_pages.is_some() || output.degradation_reason.is_some() {
        return Err(
            "provider_error: the OCR pipeline returned page outputs the batch path cannot publish"
                .to_string(),
        );
    }
    let (regions_json, blocks_json, layout_model, image_width, image_height) = match &output.layout
    {
        Some(layout) => {
            let (regions, blocks) = ocr::serialize_layout_payload(layout)?;
            (
                Some(regions),
                Some(blocks),
                layout.model.clone(),
                layout.image_width,
                layout.image_height,
            )
        }
        None => (None, None, String::new(), 0, 0),
    };
    let _ = asset_type;
    Ok(OcrComputeOutput {
        outcome: if output.ocr.text.trim().is_empty() {
            "no_text".to_string()
        } else {
            "text".to_string()
        },
        text: output.ocr.text,
        method: output.ocr.method,
        regions_json,
        blocks_json,
        layout_model,
        image_width,
        image_height,
        provider: provider.to_string(),
        page_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_errors_map_to_retry_block_or_fatal() {
        assert!(matches!(
            map_ocr_error("timeout: GLM-OCR request timed out: x"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_ocr_error("connection: reset"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_ocr_error("rate_limited: 429"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_ocr_error("provider_5xx: 503"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_ocr_error("configuration: no key"),
            ExecOutput::Blocked { .. }
        ));
        assert!(matches!(
            map_ocr_error("Failed to read /tmp/x: gone"),
            ExecOutput::Fatal { .. }
        ));
        assert!(matches!(
            map_ocr_error("pdf encrypted"),
            ExecOutput::Fatal { .. }
        ));
    }

    #[test]
    fn batch_mode_defaults_to_light_and_honors_saved_settings() {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("settings table");
        // No row: legacy default is local → light.
        assert_eq!(resolve_batch_ocr_mode(&conn), "light");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('ocrh_mode', 'glm_ocr')",
            [],
        )
        .expect("insert mode");
        assert_eq!(resolve_batch_ocr_mode(&conn), "high");
        assert_eq!(ocr_task_contract("high"), "ocr:high");
    }
    #[test]
    fn publish_writes_extraction_and_layout_atomically() {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(
            "CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);
             CREATE UNIQUE INDEX idx_extractions_asset_id_unique ON extractions(asset_id);
             CREATE TABLE layouts (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, regions TEXT NOT NULL, blocks TEXT NOT NULL, model TEXT NOT NULL, image_width INTEGER NOT NULL, image_height INTEGER NOT NULL, created_at INTEGER NOT NULL);
             CREATE UNIQUE INDEX idx_layouts_asset_id_unique ON layouts(asset_id);
             CREATE TABLE llm_results (id TEXT PRIMARY KEY, target_id TEXT NOT NULL, target_type TEXT NOT NULL DEFAULT 'unknown', job_type TEXT NOT NULL, result TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE ocr_correction_backups (asset_id TEXT PRIMARY KEY, text_content TEXT NOT NULL);",
        )
        .expect("publish fixture");
        // A stale manual correction must not survive a newer OCR publish.
        conn.execute(
            "INSERT INTO llm_results (id, target_id, target_type, job_type, result, created_at)
             VALUES ('r1', 'a1', 'asset', 'correct_ocr', 'corregido', 1)",
            [],
        )
        .expect("stale correction");
        let output = OcrComputeOutput {
            text: "hola mundo".to_string(),
            method: "ocr".to_string(),
            outcome: "text".to_string(),
            regions_json: Some("[]".to_string()),
            blocks_json: Some("[]".to_string()),
            layout_model: "paddle".to_string(),
            image_width: 100,
            image_height: 200,
            provider: "test".to_string(),
            page_count: 1,
        };
        publish_ocr_output(&conn, "a1", &output).expect("publish");
        let (text, method): (String, String) = conn
            .query_row(
                "SELECT text_content, method FROM extractions WHERE asset_id = 'a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("extraction published");
        assert_eq!((text.as_str(), method.as_str()), ("hola mundo", "ocr"));
        let model: String = conn
            .query_row(
                "SELECT model FROM layouts WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("layout published");
        assert_eq!(model, "paddle");
        let corrections: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM llm_results WHERE target_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("correction check");
        assert_eq!(corrections, 0, "stale correction clears with the new text");
    }
}
