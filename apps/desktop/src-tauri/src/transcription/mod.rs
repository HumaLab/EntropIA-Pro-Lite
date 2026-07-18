// Audio decoding module — currently unused (faster-whisper handles audio decoding internally).
// Will be re-enabled if we need audio duration/preview in Rust.
// #[allow(dead_code)]
// mod audio;
mod assemblyai;
pub mod commands;
mod engine;

use crate::nlp::{lookup_item_id_for_asset, NlpJob, NlpQueue};
#[cfg(feature = "local-ml")]
use crate::path_utils::normalize_windows_path;
#[cfg(feature = "local-ml")]
use crate::runtime::{managed_hf_cache_dir, managed_script_path, RuntimeManager};
use assemblyai::AssemblyAiClient;
use engine::TranscriptionResult;
#[cfg(feature = "local-ml")]
use engine::{WhisperConfig, WhisperEngine};
use serde::Serialize;
use std::path::Path;
#[cfg(feature = "local-ml")]
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

const STT_MODE_LOCAL: &str = "local";
const STT_MODE_ASSEMBLYAI: &str = "assemblyai";
const STT_MODE_AUTO: &str = "auto";
const STT_SETTING_MODE: &str = "stt_mode";
const STT_SETTING_ASSEMBLYAI_API_KEY: &str = "assemblyai_api_key";

pub(super) struct ManagedTranscriptionResult {
    pub(super) transcription: TranscriptionResult,
    pub(super) model_name: &'static str,
}

// ── Event payloads ──────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct TranscriptionProgressPayload {
    pub asset_id: String,
    pub pct: u8,
    pub stage: String,
}

#[derive(Clone, Serialize)]
pub struct TranscriptionCompletePayload {
    pub asset_id: String,
    pub text: String,
    pub language: String,
    pub duration_ms: u64,
    pub segments_count: usize,
}

#[derive(Clone, Serialize)]
pub struct TranscriptionErrorPayload {
    pub asset_id: String,
    pub error: String,
}

// ── Job & Queue ─────────────────────────────────────────────────────────────

/// A single transcription work unit submitted to the background worker.
pub struct TranscriptionJob {
    pub asset_id: String,
    pub asset_path: String,
}

/// Handle for submitting jobs to the background transcription worker.
///
/// Managed as Tauri state — the `transcribe_audio` command grabs this via
/// `State<TranscriptionQueue>`.
pub struct TranscriptionQueue {
    sender: tokio::sync::mpsc::Sender<TranscriptionJob>,
}

impl TranscriptionQueue {
    /// Create a new queue and return `(TranscriptionQueue, Receiver)`.
    pub fn new() -> (Self, tokio::sync::mpsc::Receiver<TranscriptionJob>) {
        let (sender, receiver) = tokio::sync::mpsc::channel::<TranscriptionJob>(64);
        (Self { sender }, receiver)
    }

    /// Submit a job to the queue. Returns immediately.
    pub fn submit(&self, job: TranscriptionJob) -> Result<(), String> {
        self.sender
            .try_send(job)
            .map_err(|e| format!("Failed to enqueue transcription job: {e}"))
    }

    /// Spawn the background worker loop on a dedicated thread.
    ///
    /// Each transcription call spawns a Python subprocess, so no persistent
    /// WhisperContext is held. The thread just drains the queue and spawns
    /// processes sequentially.
    pub fn start_worker(
        db_path: std::path::PathBuf,
        mut receiver: tokio::sync::mpsc::Receiver<TranscriptionJob>,
        app_handle: AppHandle,
    ) {
        std::thread::Builder::new()
            .name("transcription-worker".to_string())
            .stack_size(8 * 1024 * 1024) // 8 MB — subprocess only, no heavy stack needed
            .spawn(move || {
                #[cfg(feature = "local-ml")]
                let mut engine: Option<WhisperEngine> = None;
                #[cfg(feature = "local-ml")]
                let mut init_error: Option<String> = None;

                // ── Open dedicated DB connection ────────────────────────────
                let conn = match rusqlite::Connection::open(&db_path) {
                    Ok(c) => {
                        let _ = c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;");
                        c
                    }
                    Err(e) => {
                        eprintln!("[transcription] Failed to open DB connection: {e}");
                        crate::app_logs::error(
                            &app_handle,
                            "transcription",
                            format!("No se pudo abrir conexión DB del worker de transcripción: {e}"),
                        );
                        while let Some(job) = receiver.blocking_recv() {
                            let _ = app_handle.emit(
                                "transcription:error",
                                TranscriptionErrorPayload {
                                    asset_id: job.asset_id,
                                    error: format!("DB connection failed: {e}"),
                                },
                            );
                        }
                        return;
                    }
                };

                // ── Main work loop ──────────────────────────────────────────
                while let Some(job) = receiver.blocking_recv() {
                    let asset_id = job.asset_id.clone();
                    let stt_mode = get_stt_mode(&conn);

                    #[cfg(feature = "local-ml")]
                    if stt_mode != STT_MODE_ASSEMBLYAI && engine.is_none() && init_error.is_none() {
                        match create_whisper_engine(&app_handle, Some(&db_path)) {
                            Ok(resolved_engine) => {
                                eprintln!("[transcription] Engine ready (lazy init)");
                                crate::app_logs::info(
                                    &app_handle,
                                    "transcription",
                                    "Motor local de transcripción listo",
                                );
                                engine = Some(resolved_engine);
                            }
                            Err(error) => {
                                eprintln!("[transcription] Failed to initialize transcription engine: {error}");
                                crate::app_logs::warn(
                                    &app_handle,
                                    "transcription",
                                    format!("Motor local de transcripción no disponible: {error}"),
                                );
                                init_error = Some(format!("Transcription engine initialization failed: {error}"));
                            }
                        }
                    }

                    let result = {
                        #[cfg(feature = "local-ml")]
                        {
                            match engine.as_ref() {
                                Some(_) => process_job(&conn, &job, &db_path, &app_handle),
                                None => {
                                    let has_cloud_fallback =
                                        !get_assemblyai_api_key(&conn).is_empty();

                                    if stt_mode == STT_MODE_ASSEMBLYAI
                                        || (stt_mode == STT_MODE_AUTO && has_cloud_fallback)
                                    {
                                        process_job(&conn, &job, &db_path, &app_handle)
                                    } else {
                                        Err(init_error.clone().unwrap_or_else(|| {
                                            "Transcription engine unavailable after lazy init"
                                                .to_string()
                                        }))
                                    }
                                }
                            }
                        }
                        // Lean build: no local engine ever exists, so every job
                        // routes through the cloud (AssemblyAI) fallback path.
                        #[cfg(not(feature = "local-ml"))]
                        {
                            let has_cloud_fallback = !get_assemblyai_api_key(&conn).is_empty();

                            if stt_mode == STT_MODE_ASSEMBLYAI
                                || (stt_mode == STT_MODE_AUTO && has_cloud_fallback)
                            {
                                process_job(&conn, &job, &db_path, &app_handle)
                            } else {
                                Err(
                                    "Transcripción local no disponible en esta build y AssemblyAI no está configurado. Andá a Configuración > STT para resolverlo."
                                        .to_string(),
                                )
                            }
                        }
                    };

                    match result {
                        Ok(transcription) => {
                            crate::app_logs::info(
                                &app_handle,
                                "transcription",
                                format!("Transcripción completada: asset_id={asset_id}, segmentos={}", transcription.segments.len()),
                            );
                            let _ = app_handle.emit(
                                "transcription:complete",
                                TranscriptionCompletePayload {
                                    asset_id: asset_id.clone(),
                                    text: transcription.text.clone(),
                                    language: transcription.language.clone(),
                                    duration_ms: transcription.duration_ms,
                                    segments_count: transcription.segments.len(),
                                },
                            );
                        }
                        Err(err) => {
                            eprintln!("[transcription] Error for {asset_id}: {err}");
                            crate::app_logs::error(
                                &app_handle,
                                "transcription",
                                format!("Transcripción falló: asset_id={asset_id}, error={err}"),
                            );
                            let _ = app_handle.emit(
                                "transcription:error",
                                TranscriptionErrorPayload {
                                    asset_id,
                                    error: err,
                                },
                            );
                        }
                    }
                }
            })
            .expect("Failed to spawn transcription worker thread");
    }
}

pub(crate) fn ensure_transcription_runtime_ready(app_handle: &AppHandle) -> Result<(), String> {
    // Dev fallback is acceptable: Ok(None) means managed runtime is not healthy
    // but callers will fall back to CARGO_MANIFEST_DIR / system Python.
    #[cfg(feature = "local-ml")]
    {
        managed_runtime_root_for_transcription(app_handle).map(|_| ())
    }
    // Lean build has no local runtime — this is a no-op so the unconditional
    // command surface (transcribe_audio / transcribe_dictation) still links.
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = app_handle;
        Ok(())
    }
}

#[cfg(feature = "local-ml")]
fn managed_runtime_root_for_transcription(
    app_handle: &AppHandle,
) -> Result<Option<PathBuf>, String> {
    managed_runtime_root_for_transcription_with(
        || RuntimeManager::new().ensure_ready_or_bootstrap(app_handle),
        || RuntimeManager::new().hydrated_runtime_root(app_handle),
    )
}

#[cfg(feature = "local-ml")]
fn managed_runtime_root_for_transcription_with<E, H>(
    ensure_ready_or_bootstrap: E,
    hydrated_runtime_root: H,
) -> Result<Option<PathBuf>, String>
where
    E: FnOnce() -> Result<crate::runtime::status::RuntimeStatus, String>,
    H: FnOnce() -> Result<Option<std::path::PathBuf>, String>,
{
    let status = ensure_ready_or_bootstrap()?;
    if status.state != crate::runtime::status::RuntimeState::Healthy {
        // Dev fallback: return None so callers fall back to CARGO_MANIFEST_DIR resources.
        // Honest blocking (e.g. no Python available) is handled at engine init time.
        return Ok(None);
    }

    hydrated_runtime_root()
}

#[cfg(feature = "local-ml")]
fn resolve_transcription_script_path(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let runtime_root = managed_runtime_root_for_transcription(app_handle)?;
    Ok(resolve_transcription_script_path_from_roots(
        runtime_root.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    ))
}

#[cfg(feature = "local-ml")]
fn resolve_model_cache_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let runtime_root = managed_runtime_root_for_transcription(app_handle)?;
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir for model cache: {e}"))?;
    let model_cache_dir =
        resolve_transcription_model_cache_dir(runtime_root.as_deref(), &app_data_dir)?;

    std::fs::create_dir_all(&model_cache_dir).map_err(|e| {
        format!(
            "Could not create model cache dir {}: {e}",
            model_cache_dir.display()
        )
    })?;

    Ok(model_cache_dir)
}

#[cfg(feature = "local-ml")]
fn resolve_transcription_script_path_from_roots(
    managed_root: Option<&Path>,
    manifest_dir: &Path,
) -> PathBuf {
    if let Some(root) = managed_root {
        let managed = managed_script_path(root, "transcribe.py");
        if managed.exists() {
            return managed;
        }
    }

    normalize_windows_path(manifest_dir.join("scripts/transcribe.py"))
}

#[cfg(feature = "local-ml")]
fn resolve_transcription_model_cache_dir(
    managed_root: Option<&Path>,
    app_data_dir: &Path,
) -> Result<PathBuf, String> {
    Ok(managed_root
        .map(managed_hf_cache_dir)
        .unwrap_or_else(|| app_data_dir.join("hf_cache")))
}

#[cfg(feature = "local-ml")]
fn create_whisper_engine(
    app_handle: &AppHandle,
    settings_db_path: Option<&Path>,
) -> Result<WhisperEngine, String> {
    let python_path = which_python(settings_db_path).ok_or_else(|| {
        "No Python interpreter with faster_whisper found. Please install Python and run: pip install faster-whisper"
            .to_string()
    })?;

    WhisperEngine::init(WhisperConfig {
        python_path,
        script_path: resolve_transcription_script_path(app_handle)?,
        model_size: "base".to_string(),
        language: "es".to_string(),
        compute_type: "int8".to_string(),
        model_dir: Some(resolve_model_cache_dir(app_handle)?),
    })
}

#[cfg(feature = "local-ml")]
pub fn transcribe_audio_file(
    app_handle: &AppHandle,
    settings_db_path: Option<&Path>,
    audio_path: &str,
) -> Result<TranscriptionResult, String> {
    let engine = create_whisper_engine(app_handle, settings_db_path)?;
    engine.transcribe(audio_path, 0)
}

fn get_stt_mode(conn: &rusqlite::Connection) -> String {
    crate::settings::get_setting(conn, STT_SETTING_MODE)
        .unwrap_or_else(|| STT_MODE_LOCAL.to_string())
        .to_lowercase()
}

fn get_assemblyai_api_key(conn: &rusqlite::Connection) -> String {
    crate::settings::get_setting(conn, STT_SETTING_ASSEMBLYAI_API_KEY)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(super) fn ensure_selected_cloud_key(conn: &rusqlite::Connection) -> Result<(), String> {
    let mode = get_stt_mode(conn);
    if mode == STT_MODE_ASSEMBLYAI && get_assemblyai_api_key(conn).is_empty() {
        return Err(
            "AssemblyAI no está configurado. Andá a Configuración > STT y cargá una API key antes de transcribir."
                .to_string(),
        );
    }

    Ok(())
}

#[cfg(feature = "local-ml")]
pub fn local_transcription_available(settings_db_path: Option<&Path>) -> bool {
    which_python(settings_db_path).is_some()
}

#[cfg(feature = "local-ml")]
fn transcribe_with_local_provider(
    app_handle: &AppHandle,
    settings_db_path: Option<&Path>,
    audio_path: &str,
) -> Result<ManagedTranscriptionResult, String> {
    let result = transcribe_audio_file(app_handle, settings_db_path, audio_path)?;
    Ok(ManagedTranscriptionResult {
        transcription: result,
        model_name: "faster-whisper/base",
    })
}

fn transcribe_with_assemblyai_provider(
    audio_path: &str,
    asset_id: Option<&str>,
    api_key: &str,
    enable_role_speaker_identification: bool,
    on_progress: impl FnMut(u8, &str),
) -> Result<ManagedTranscriptionResult, String> {
    let client = AssemblyAiClient::new(api_key.to_string());
    let result = tauri::async_runtime::block_on(client.transcribe_file(
        Path::new(audio_path),
        asset_id,
        enable_role_speaker_identification,
        on_progress,
    ))?;

    Ok(ManagedTranscriptionResult {
        transcription: result,
        model_name: "assemblyai/universal",
    })
}

pub(super) fn transcribe_with_selected_provider(
    app_handle: &AppHandle,
    settings_db_path: Option<&Path>,
    conn: &rusqlite::Connection,
    asset_id: Option<&str>,
    audio_path: &str,
) -> Result<ManagedTranscriptionResult, String> {
    let mode = get_stt_mode(conn);
    let assemblyai_api_key = get_assemblyai_api_key(conn);

    // Lean build never touches the local provider, so this param is only used
    // under local-ml. Silence the unused-variable warning when gated out.
    #[cfg(not(feature = "local-ml"))]
    let _ = settings_db_path;

    let emit_provider_progress = |pct: u8, stage: &str| {
        if let Some(asset_id) = asset_id {
            emit_progress(app_handle, asset_id, pct, stage);
        }
    };
    let enable_role_speaker_identification = asset_id.is_some();

    match mode.as_str() {
        #[cfg(feature = "local-ml")]
        STT_MODE_LOCAL => transcribe_with_local_provider(app_handle, settings_db_path, audio_path),
        STT_MODE_ASSEMBLYAI => {
            if assemblyai_api_key.is_empty() {
                return Err(
                    "AssemblyAI no está configurado. Andá a Configuración > STT y cargá una API key antes de transcribir."
                        .to_string(),
                );
            }

            transcribe_with_assemblyai_provider(
                audio_path,
                asset_id,
                &assemblyai_api_key,
                enable_role_speaker_identification,
                emit_provider_progress,
            )
        }
        STT_MODE_AUTO => {
            #[cfg(feature = "local-ml")]
            {
                let local_available = local_transcription_available(settings_db_path);

                if local_available {
                    match transcribe_with_local_provider(app_handle, settings_db_path, audio_path) {
                        Ok(result) => return Ok(result),
                        Err(local_error) => {
                            eprintln!("[transcription] Local STT failed in auto mode, trying AssemblyAI fallback: {local_error}");

                            if assemblyai_api_key.is_empty() {
                                return Err(format!(
                                    "La transcripción local falló y no hay fallback cloud configurado. Error local: {local_error}"
                                ));
                            }

                            return transcribe_with_assemblyai_provider(
                                audio_path,
                                asset_id,
                                &assemblyai_api_key,
                                enable_role_speaker_identification,
                                emit_provider_progress,
                            )
                                .map_err(|cloud_error| {
                                    format!(
                                        "La transcripción local falló y el fallback con AssemblyAI también falló. Error local: {local_error}\nError AssemblyAI: {cloud_error}"
                                    )
                                });
                        }
                    }
                }
            }

            if assemblyai_api_key.is_empty() {
                return Err(
                    "No hay motor local disponible y AssemblyAI no está configurado. Andá a Configuración > STT para resolverlo."
                        .to_string(),
                );
            }

            eprintln!(
                "[transcription] Local STT unavailable in auto mode, using AssemblyAI fallback"
            );
            transcribe_with_assemblyai_provider(
                audio_path,
                asset_id,
                &assemblyai_api_key,
                enable_role_speaker_identification,
                emit_provider_progress,
            )
        }
        #[cfg(feature = "local-ml")]
        _ => transcribe_with_local_provider(app_handle, settings_db_path, audio_path),
        // Lean build: LOCAL + unknown modes have no local engine, so route to
        // AssemblyAI (mirrors Lite's AssemblyAI-only shape).
        #[cfg(not(feature = "local-ml"))]
        _ => {
            if assemblyai_api_key.is_empty() {
                return Err(
                    "AssemblyAI no está configurado. Andá a Configuración > STT y cargá una API key antes de transcribir."
                        .to_string(),
                );
            }

            transcribe_with_assemblyai_provider(
                audio_path,
                asset_id,
                &assemblyai_api_key,
                enable_role_speaker_identification,
                emit_provider_progress,
            )
        }
    }
}

pub fn cleanup_temp_audio_file(audio_path: &str) -> Result<(), String> {
    match std::fs::remove_file(audio_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to remove temporary audio file {audio_path}: {error}"
        )),
    }
}

/// Find the Python interpreter on the system that has `faster_whisper` available.
///
/// Uses the shared Python candidate cache to avoid redundant filesystem scans
/// and log noise. Probes each candidate for the `faster_whisper` module.
#[cfg(feature = "local-ml")]
fn which_python(settings_db_path: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    crate::python_discovery::which_python_for_module(
        "transcription",
        "faster_whisper",
        "faster_whisper",
        "import faster_whisper; print('ok')",
        settings_db_path,
    )
}

#[cfg(all(test, feature = "local-ml"))]
mod tests {
    use super::*;
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use std::cell::RefCell;
    use tempfile::tempdir;

    #[test]
    fn resolve_transcription_script_prefers_managed_runtime_copy() {
        let runtime_dir = tempdir().expect("runtime dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let managed_script = runtime_dir.path().join("scripts").join("transcribe.py");
        std::fs::create_dir_all(managed_script.parent().expect("script parent"))
            .expect("create script dir");
        std::fs::write(&managed_script, "print('ok')").expect("write script");

        let resolved = resolve_transcription_script_path_from_roots(
            Some(runtime_dir.path()),
            manifest_dir.path(),
        );

        assert_eq!(resolved, managed_script);
    }

    #[test]
    fn resolve_transcription_cache_prefers_managed_hf_cache() {
        let runtime_dir = tempdir().expect("runtime dir");
        let app_data_dir = tempdir().expect("app data dir");
        let managed_cache = runtime_dir.path().join("caches").join("hf");

        let resolved =
            resolve_transcription_model_cache_dir(Some(runtime_dir.path()), app_data_dir.path())
                .expect("cache dir should resolve");

        assert_eq!(resolved, managed_cache);
    }

    #[test]
    fn transcription_runtime_resolution_bootstraps_before_using_managed_assets() {
        let calls = RefCell::new(Vec::new());
        let expected = PathBuf::from("/tmp/runtime-ready");

        let resolved = managed_runtime_root_for_transcription_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::Healthy,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: true,
                    summary: "Runtime listo".to_string(),
                    blocked_capabilities: vec![],
                    details: vec![],
                    guidance: vec![],
                    bootstrap_eligible: false,
                    bootstrap_required: false,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(expected.clone()))
            },
        )
        .expect("managed runtime should resolve");

        assert_eq!(resolved, Some(expected));
        assert_eq!(calls.into_inner(), vec!["ensure_ready", "hydrated_root"]);
    }

    #[test]
    fn transcription_runtime_resolution_returns_none_when_not_healthy_allowing_dev_fallback() {
        let calls = RefCell::new(Vec::new());

        let resolved = managed_runtime_root_for_transcription_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedSourceUnavailable,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "No hay una fuente confiable disponible para bootstrap".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Transcription],
                    details: vec!["manifest remoto no publicado".to_string()],
                    guidance: vec!["Reintentá cuando exista una fuente firmada".to_string()],
                    bootstrap_eligible: false,
                    bootstrap_required: true,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(PathBuf::from("/tmp/stale-runtime")))
            },
        )
        .expect("non-healthy runtime should not raise transport errors");

        assert_eq!(resolved, None);
        assert_eq!(calls.into_inner(), vec!["ensure_ready"]);
    }

    #[test]
    fn transcription_path_resolution_falls_back_to_dev_when_runtime_root_is_none() {
        let manifest_dir = tempdir().expect("manifest dir");
        let legacy_script = manifest_dir.path().join("resources/scripts/transcribe.py");
        let canonical_script = manifest_dir.path().join("scripts/transcribe.py");
        std::fs::create_dir_all(legacy_script.parent().unwrap()).unwrap();
        std::fs::create_dir_all(canonical_script.parent().unwrap()).unwrap();
        std::fs::write(&legacy_script, "print('legacy')").unwrap();
        std::fs::write(&canonical_script, "print('canonical')").unwrap();

        // Development must use the repository source, never an obsolete resource copy.
        let resolved = resolve_transcription_script_path_from_roots(None, manifest_dir.path());
        assert_eq!(resolved, canonical_script);
    }

    #[test]
    fn ensure_transcription_runtime_ready_accepts_dev_fallback() {
        // ensure_transcription_runtime_ready should return Ok(()) even when managed runtime is not healthy,
        // because callers fall back to dev paths / system Python.
        let result = managed_runtime_root_for_transcription_with(
            || {
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedSourceUnavailable,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "No hay una fuente confiable disponible para bootstrap".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Transcription],
                    details: vec!["fixture".to_string()],
                    guidance: vec![],
                    bootstrap_eligible: false,
                    bootstrap_required: true,
                    active_operation: None,
                })
            },
            || Ok(None),
        );
        assert!(
            result.is_ok(),
            "ensure_transcription_runtime_ready should accept dev fallback"
        );
        assert_eq!(result.unwrap(), None);
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

/// Save a transcription result to the database.
fn save_transcription(
    conn: &rusqlite::Connection,
    asset_id: &str,
    result: &TranscriptionResult,
    model_name: &str,
) -> Result<Option<String>, String> {
    // Serialize segments as JSON (using the same Segment struct)
    let segments_json = serde_json::to_string(&result.segments)
        .map_err(|e| format!("Failed to serialize segments: {e}"))?;

    // Deterministic id: one transcription per asset (UNIQUE index on
    // transcriptions.asset_id), so derive the id from asset_id to converge
    // duplicates across devices. The UPSERT re-asserts `id = excluded.id` and
    // fires only an UPDATE (no DELETE+INSERT tombstone in sync_oplog).
    let id = format!("trx-{asset_id}");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    conn.execute(
        "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(asset_id) DO UPDATE SET
           id = excluded.id,
           text_content = excluded.text_content,
           language = excluded.language,
           duration_ms = excluded.duration_ms,
           model = excluded.model,
           segments = excluded.segments,
           confidence = excluded.confidence,
           created_at = excluded.created_at",
        rusqlite::params![
            id,
            asset_id,
            result.text,
            result.language,
            result.duration_ms as i64,
            model_name,
            segments_json,
            None::<f64>, // confidence — not provided by faster-whisper directly
            now,
        ],
    )
    .map_err(|e| format!("Failed to upsert transcription: {e}"))?;

    lookup_item_id_for_asset(conn, asset_id)
}

// ── Job Processing ──────────────────────────────────────────────────────────

/// Process a single transcription job.
fn process_job(
    conn: &rusqlite::Connection,
    job: &TranscriptionJob,
    settings_db_path: &Path,
    app_handle: &AppHandle,
) -> Result<TranscriptionResult, String> {
    emit_progress(app_handle, &job.asset_id, 10, "reading");

    eprintln!("[transcription] Transcribing: {}", job.asset_path);
    emit_progress(app_handle, &job.asset_id, 30, "transcribing");

    let ManagedTranscriptionResult {
        transcription: result,
        model_name,
    } = transcribe_with_selected_provider(
        app_handle,
        Some(settings_db_path),
        conn,
        Some(&job.asset_id),
        &job.asset_path,
    )?;

    emit_progress(app_handle, &job.asset_id, 80, "saving");

    // Stage 2 — persist to SQLite
    if let Some(item_id) = save_transcription(conn, &job.asset_id, &result, model_name)? {
        // Asset-level NER + triples: only re-extract for the transcribed asset,
        // not the entire item. Avoids reprocessing unchanged pages.
        let nlp_queue = app_handle.state::<NlpQueue>();
        if let Err(e) = nlp_queue.submit(NlpJob::ExtractEntitiesForAsset {
            item_id: item_id.clone(),
            asset_id: job.asset_id.clone(),
        }) {
            eprintln!("[nlp] Failed to auto-enqueue ExtractEntitiesForAsset after transcription save: {e}");
        } else {
            eprintln!(
                "[nlp] Auto-enqueued ExtractEntitiesForAsset after transcription save: asset_id={}, item_id={}",
                job.asset_id, item_id
            );
        }
        // FTS indexing: ensures the new transcript is searchable immediately.
        if let Err(e) = nlp_queue.submit(NlpJob::IndexFts {
            item_id: item_id.clone(),
        }) {
            eprintln!("[nlp] Failed to auto-enqueue IndexFts after transcription save: {e}");
        } else {
            eprintln!(
                "[nlp] Auto-enqueued IndexFts after transcription save: item_id={}",
                item_id
            );
        }
        // Asset-level embedding keeps similarity in sync for the specific
        // transcribed asset.
        if let Err(e) = nlp_queue.submit(NlpJob::ComputeAssetEmbedding {
            item_id: item_id.clone(),
            asset_id: job.asset_id.clone(),
        }) {
            eprintln!(
                "[nlp] Failed to auto-enqueue ComputeAssetEmbedding after transcription save: {e}"
            );
        } else {
            eprintln!(
                "[nlp] Auto-enqueued ComputeAssetEmbedding after transcription save: asset_id={}, item_id={}",
                job.asset_id, item_id
            );
        }
    }

    emit_progress(app_handle, &job.asset_id, 100, "done");

    Ok(result)
}

/// Emit a `transcription:progress` event to the frontend.
fn emit_progress(app_handle: &AppHandle, asset_id: &str, pct: u8, stage: &str) {
    if pct == 0 || pct == 10 || pct == 100 {
        crate::app_logs::info(
            app_handle,
            "transcription",
            format!("Transcripción asset_id={asset_id} etapa={stage} progreso={pct}%"),
        );
    }
    let _ = app_handle.emit(
        "transcription:progress",
        TranscriptionProgressPayload {
            asset_id: asset_id.to_string(),
            pct,
            stage: stage.to_string(),
        },
    );
}
