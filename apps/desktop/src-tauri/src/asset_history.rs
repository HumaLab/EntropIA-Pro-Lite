//! Process history of each asset (`asset_process_events`, migration 0061).
//!
//! Every process an asset goes through leaves one row: what ran (`process`),
//! what did the work (`tool`, `tool_version`), which model when there is one
//! (`model`, `model_version`), the EntropIA build (`app_version`), its
//! parameters, and the hashes of the file or text before and after. The
//! person reads it in the asset's metadata to trace and audit what was done to
//! a document.
//!
//! Writers call [`record`] inside the same transaction as the change they
//! describe whenever there is one, so a change and its history land together.
//! [`record_best_effort`] is for paths where history must never block the
//! work itself (it logs instead of failing); a database built before 0061
//! (the partial schemas of unit tests) simply records nothing.
//!
//! Local only: the table is not in the device sync set yet.

use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};

/// The EntropIA build every row is stamped with. `pnpm version:bump` keeps
/// the crate version equal to the app version.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Saves of the transcription/OCR text closer than this to the previous one
/// belong to the same editing session and share one `text_edit` row.
const TEXT_EDIT_SESSION_MS: i64 = 30 * 60 * 1000;

// Process codes. Stable: the UI translates them and exports carry them.
pub const PROCESS_IMPORT: &str = "import";
pub const PROCESS_PAGE_SPLIT: &str = "page_split";
pub const PROCESS_OCR: &str = "ocr";
pub const PROCESS_LAYOUT_BOX_OCR: &str = "layout_box_ocr";
pub const PROCESS_TEXT_EDIT: &str = "text_edit";
pub const PROCESS_TEXT_REPLACE: &str = "text_replace";
pub const PROCESS_OCR_CORRECTION: &str = "ocr_correction";
pub const PROCESS_OCR_RESTORE: &str = "ocr_restore";
pub const PROCESS_TRANSCRIPTION: &str = "transcription";
pub const PROCESS_TRANSCRIPTION_EDIT: &str = "transcription_edit";
pub const PROCESS_EMBEDDING: &str = "embedding";
pub const PROCESS_ENTITIES: &str = "entities";
pub const PROCESS_TRIPLES: &str = "triples";
pub const PROCESS_SUMMARY: &str = "summary";
/// Maintenance pointed the asset back at the image version its layout belongs
/// to ([`crate::image_version_repair`]). Written by the backend only: it is not
/// in [`renderer_tool`], so the renderer cannot report it.
pub const PROCESS_IMAGE_VERSION_REPAIR: &str = "image_version_repair";
pub const TOOL_IMAGE_VERSION_REPAIR: &str = "EntropIA revisión de versiones de imágenes";

/// One history row before it is written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessEvent {
    pub asset_id: String,
    pub process: String,
    pub tool: String,
    pub tool_version: Option<String>,
    pub model: Option<String>,
    pub model_version: Option<String>,
    pub params: Option<Value>,
    pub input_sha256: Option<String>,
    pub output_sha256: Option<String>,
}

impl ProcessEvent {
    pub fn new(asset_id: &str, process: &str, tool: &str) -> Self {
        Self {
            asset_id: asset_id.to_string(),
            process: process.to_string(),
            tool: tool.to_string(),
            ..Self::default()
        }
    }

    pub fn tool_version(mut self, version: impl Into<String>) -> Self {
        self.tool_version = Some(version.into()).filter(|v: &String| !v.trim().is_empty());
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into()).filter(|m: &String| !m.trim().is_empty());
        self
    }

    pub fn model_version(mut self, version: impl Into<String>) -> Self {
        self.model_version = Some(version.into()).filter(|v: &String| !v.trim().is_empty());
        self
    }

    pub fn params(mut self, params: Value) -> Self {
        self.params = (!params.is_null()).then_some(params);
        self
    }

    pub fn input_sha256(mut self, hash: Option<String>) -> Self {
        self.input_sha256 = hash;
        self
    }

    pub fn output_sha256(mut self, hash: Option<String>) -> Self {
        self.output_sha256 = hash;
        self
    }

    /// Hash of the text a process produced.
    pub fn output_text(self, text: &str) -> Self {
        self.output_sha256(Some(sha256_hex(text.as_bytes())))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Hash of a file, or `None` when it cannot be read (history never fails on it).
pub fn sha256_file(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(hex::encode(hasher.finalize()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Writes one history row. Use inside the transaction of the change itself.
pub fn record(conn: &Connection, event: &ProcessEvent) -> Result<(), String> {
    record_at(conn, event, now_ms())
}

fn record_at(conn: &Connection, event: &ProcessEvent, at: i64) -> Result<(), String> {
    let params_json = event
        .params
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| format!("Failed to serialize process parameters: {e}"))?;
    conn.execute(
        "INSERT INTO asset_process_events
           (id, asset_id, process, tool, tool_version, model, model_version, app_version,
            params_json, input_sha256, output_sha256, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            uuid::Uuid::new_v4().to_string(),
            event.asset_id,
            event.process,
            event.tool,
            event.tool_version,
            event.model,
            event.model_version,
            APP_VERSION,
            params_json,
            event.input_sha256,
            event.output_sha256,
            at,
        ],
    )
    .map_err(|e| format!("Failed to record the asset process history: {e}"))?;
    Ok(())
}

/// Whether the latest extraction-text writer was a manual edit. Unrelated
/// history (embeddings, summaries, file edits) cannot release this protection.
/// Match the history writer's ordering, including writes in the same millisecond.
/// Archives predating process history have no manual-edit signal.
pub fn text_is_edited_by_hand(conn: &Connection, asset_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT process FROM asset_process_events
         WHERE asset_id = ?1 AND process IN (?2, ?3, ?4, ?5, ?6, ?7)
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
        params![
            asset_id,
            PROCESS_TEXT_EDIT,
            PROCESS_OCR,
            PROCESS_OCR_CORRECTION,
            PROCESS_OCR_RESTORE,
            PROCESS_LAYOUT_BOX_OCR,
            PROCESS_TEXT_REPLACE
        ],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|process| process.as_deref() == Some(PROCESS_TEXT_EDIT))
    .or_else(|error| {
        if is_missing_table(&error.to_string()) {
            Ok(false)
        } else {
            Err(format!("Failed to read the asset process history: {error}"))
        }
    })
}

/// Whether the latest transcription writer was a manual edit (local history only).
pub fn transcription_is_edited_by_hand(conn: &Connection, asset_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT process FROM asset_process_events
         WHERE asset_id = ?1 AND process IN (?2, ?3)
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
        params![asset_id, PROCESS_TRANSCRIPTION_EDIT, PROCESS_TRANSCRIPTION],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|process| process.as_deref() == Some(PROCESS_TRANSCRIPTION_EDIT))
    .or_else(|error| {
        if is_missing_table(&error.to_string()) {
            Ok(false)
        } else {
            Err(format!("Failed to read the asset process history: {error}"))
        }
    })
}

fn is_missing_table(error: &str) -> bool {
    error.contains("no such table: asset_process_events")
}

/// [`record`] that logs instead of failing: for writers whose work must not be
/// undone because its history could not be written. A schema without the
/// table (before 0061) records nothing silently.
pub fn record_best_effort(conn: &Connection, event: &ProcessEvent) {
    if let Err(error) = record(conn, event) {
        if !is_missing_table(&error) {
            log::warn!(
                target: "asset_history",
                "could not record {} for {}: {error}",
                event.process,
                event.asset_id
            );
        }
    }
}

/// Records a manual save of an asset's text (`process` is
/// [`PROCESS_TEXT_EDIT`] or [`PROCESS_TRANSCRIPTION_EDIT`]). Saves within
/// [`TEXT_EDIT_SESSION_MS`] of the latest one, with nothing else recorded in
/// between, update that row's save count and output hash instead of adding a
/// row per debounced save.
pub fn record_text_edit(conn: &Connection, asset_id: &str, process: &str, text: &str) {
    if let Err(error) = record_text_edit_at(conn, asset_id, process, text, now_ms()) {
        if !is_missing_table(&error) {
            log::warn!(target: "asset_history", "could not record {process} for {asset_id}: {error}");
        }
    }
}

fn record_text_edit_at(
    conn: &Connection,
    asset_id: &str,
    process: &str,
    text: &str,
    at: i64,
) -> Result<(), String> {
    let output = sha256_hex(text.as_bytes());
    let latest: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT id, process, params_json FROM asset_process_events
              WHERE asset_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
            [asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to read the asset process history: {e}"))?;

    if let Some((id, latest_process, params_json)) = latest {
        let mut session: Value = params_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_else(|| json!({}));
        let last_saved = session
            .get("lastSavedAt")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if latest_process == process && at - last_saved <= TEXT_EDIT_SESSION_MS {
            let saves = session.get("saves").and_then(Value::as_i64).unwrap_or(1) + 1;
            session["saves"] = json!(saves);
            session["lastSavedAt"] = json!(at);
            session["characters"] = json!(text.chars().count());
            conn.execute(
                "UPDATE asset_process_events SET params_json = ?1, output_sha256 = ?2 WHERE id = ?3",
                params![session.to_string(), output, id],
            )
            .map_err(|e| format!("Failed to update the asset process history: {e}"))?;
            return Ok(());
        }
    }

    record_at(
        conn,
        &ProcessEvent::new(asset_id, process, "EntropIA (edición manual)")
            .params(json!({
                "saves": 1,
                "lastSavedAt": at,
                "characters": text.chars().count(),
            }))
            .output_sha256(Some(output)),
        at,
    )
}

// ── Processes the renderer performs ─────────────────────────────────────────

/// What the renderer reports: viewer edits that only take effect when the
/// asset adopts the new file (the edit commands themselves never see the
/// asset id), undo/redo and duplication.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererProcessReport {
    pub asset_id: String,
    pub process: String,
    #[serde(default)]
    pub params: Option<Value>,
    /// The asset's file before the process (hashed as the input).
    #[serde(default)]
    pub previous_path: Option<String>,
}

/// The tool behind each process the renderer may report. Anything else is
/// refused, so the renderer cannot write arbitrary history.
fn renderer_tool(process: &str) -> Option<&'static str> {
    match process {
        "image_crop" | "image_erase" | "image_rotate" | "image_rotate_fine" => {
            Some("EntropIA image_edit (crate image)")
        }
        "pdf_crop" | "pdf_erase" | "pdf_rotate" => Some("EntropIA edit_pdf (pdfium + lopdf)"),
        "image_undo" | "image_redo" => Some("EntropIA historial de edición"),
        "image_use_version" => Some("EntropIA versiones de la imagen"),
        "duplicate" => Some("EntropIA (duplicar asset)"),
        _ => None,
    }
}

fn build_renderer_event(
    report: &RendererProcessReport,
    current_file: Option<&std::path::Path>,
    previous_file: Option<&std::path::Path>,
) -> Result<ProcessEvent, String> {
    let tool = renderer_tool(&report.process)
        .ok_or_else(|| format!("invalid_argument: unknown process {:?}", report.process))?;
    if let Some(params) = &report.params {
        if !params.is_object() {
            return Err("invalid_argument: process parameters must be an object".to_string());
        }
    }
    Ok(ProcessEvent::new(&report.asset_id, &report.process, tool)
        .params(report.params.clone().unwrap_or(Value::Null))
        .input_sha256(previous_file.and_then(sha256_file))
        .output_sha256(current_file.and_then(sha256_file)))
}

/// Records a process the renderer performed on an asset. The output hash is
/// the asset's current file; the input hash is `previous_path` when given.
#[tauri::command]
pub async fn asset_history_record(
    report: RendererProcessReport,
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    renderer_tool(&report.process)
        .ok_or_else(|| format!("invalid_argument: unknown process {:?}", report.process))?;
    let db = db.inner().clone();
    let read_db = db.clone();
    let asset_id = report.asset_id.clone();
    let stored_path: String = run_blocking_db_task(move || {
        let conn = read_db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {e}"))?;
        conn.query_row(
            "SELECT path FROM assets WHERE id = ?1",
            [&asset_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("not_found: asset {asset_id}: {e}"))
    })
    .await?;

    let current = crate::path_utils::resolve_asset_path_at_boundary(&stored_path, &app)?;
    let previous = report
        .previous_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(|path| crate::path_utils::resolve_asset_path_at_boundary(path, &app))
        .transpose()?;
    let event = tauri::async_runtime::spawn_blocking(move || {
        build_renderer_event(
            &report,
            Some(std::path::Path::new(&current)),
            previous.as_deref().map(std::path::Path::new),
        )
    })
    .await
    .map_err(|e| format!("History hash task failed: {e}"))??;

    run_blocking_db_task(move || {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {e}"))?;
        record(&conn, &event)
    })
    .await
}

#[cfg(test)]
pub(crate) const TEST_DDL: &str =
    include_str!("../../../../packages/store/src/migrations/0061_asset_process_events.sql");

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY);
             INSERT INTO assets (id) VALUES ('a1');",
        )
        .unwrap();
        conn.execute_batch(TEST_DDL).unwrap();
        conn
    }

    /// (process, tool, model, params_json, app_version)
    type Row = (String, String, Option<String>, Option<String>, String);

    fn rows(conn: &Connection) -> Vec<Row> {
        let mut stmt = conn
            .prepare(
                "SELECT process, tool, model, params_json, app_version
                   FROM asset_process_events ORDER BY created_at, rowid",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    #[test]
    fn record_writes_tool_model_versions_and_params() {
        let conn = conn();
        record(
            &conn,
            &ProcessEvent::new("a1", PROCESS_OCR, "GLM-OCR API")
                .tool_version("v4")
                .model("glm-ocr")
                .params(json!({"mode": "high"}))
                .output_text("hola"),
        )
        .unwrap();
        let (tool_version, output): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT tool_version, output_sha256 FROM asset_process_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tool_version.as_deref(), Some("v4"));
        assert_eq!(output, Some(sha256_hex(b"hola")));
        assert_eq!(
            rows(&conn),
            vec![(
                "ocr".to_string(),
                "GLM-OCR API".to_string(),
                Some("glm-ocr".to_string()),
                Some(r#"{"mode":"high"}"#.to_string()),
                APP_VERSION.to_string()
            )]
        );
    }

    #[test]
    fn manual_text_rule_ignores_unrelated_history_and_orders_ties() {
        let conn = conn();
        assert!(!text_is_edited_by_hand(&conn, "a1").unwrap());
        record_text_edit_at(&conn, "a1", PROCESS_TEXT_EDIT, "one", 1000).unwrap();
        record_text_edit_at(&conn, "a1", PROCESS_TEXT_EDIT, "two", 1001).unwrap();
        record_at(
            &conn,
            &ProcessEvent::new("a1", PROCESS_EMBEDDING, "test"),
            1002,
        )
        .unwrap();
        assert!(text_is_edited_by_hand(&conn, "a1").unwrap());
        record_at(&conn, &ProcessEvent::new("a1", PROCESS_OCR, "test"), 1002).unwrap();
        assert!(!text_is_edited_by_hand(&conn, "a1").unwrap());
        record_text_edit_at(&conn, "a1", PROCESS_TEXT_EDIT, "three", 1002).unwrap();
        assert!(text_is_edited_by_hand(&conn, "a1").unwrap());
        assert!(!text_is_edited_by_hand(&conn, "missing").unwrap());
        assert!(!text_is_edited_by_hand(&Connection::open_in_memory().unwrap(), "a1").unwrap());
    }

    #[test]
    fn transcription_manual_edit_rule() {
        let conn = conn();
        assert!(!transcription_is_edited_by_hand(&conn, "a1").unwrap());
        record_text_edit_at(&conn, "a1", PROCESS_TRANSCRIPTION_EDIT, "edited", 1000).unwrap();
        assert!(transcription_is_edited_by_hand(&conn, "a1").unwrap());
        record_at(
            &conn,
            &ProcessEvent::new("a1", PROCESS_EMBEDDING, "test"),
            1001,
        )
        .unwrap();
        assert!(transcription_is_edited_by_hand(&conn, "a1").unwrap());
        record_at(
            &conn,
            &ProcessEvent::new("a1", PROCESS_TRANSCRIPTION, "test"),
            1001,
        )
        .unwrap();
        assert!(!transcription_is_edited_by_hand(&conn, "a1").unwrap());
        assert!(
            !transcription_is_edited_by_hand(&Connection::open_in_memory().unwrap(), "a1").unwrap()
        );
    }

    #[test]
    fn empty_model_strings_are_stored_as_null() {
        let event = ProcessEvent::new("a1", PROCESS_OCR, "pdfium").model("  ");
        assert_eq!(event.model, None);
    }

    #[test]
    fn best_effort_is_silent_without_the_table() {
        let conn = Connection::open_in_memory().unwrap();
        record_best_effort(&conn, &ProcessEvent::new("a1", PROCESS_OCR, "pdfium"));
        record_text_edit(&conn, "a1", PROCESS_TEXT_EDIT, "x");
    }

    #[test]
    fn manual_saves_of_one_session_share_a_row() {
        let conn = conn();
        let start = 1_000_000;
        record_text_edit_at(&conn, "a1", PROCESS_TEXT_EDIT, "uno", start).unwrap();
        record_text_edit_at(&conn, "a1", PROCESS_TEXT_EDIT, "uno dos", start + 2_000).unwrap();
        let history = rows(&conn);
        assert_eq!(history.len(), 1);
        let params: Value = serde_json::from_str(history[0].3.as_deref().unwrap()).unwrap();
        assert_eq!(params["saves"], 2);
        assert_eq!(params["characters"], 7);

        // Another process in between starts a new session.
        record_at(
            &conn,
            &ProcessEvent::new("a1", PROCESS_OCR, "pdfium"),
            start + 3_000,
        )
        .unwrap();
        record_text_edit_at(&conn, "a1", PROCESS_TEXT_EDIT, "tres", start + 4_000).unwrap();
        // So does a long pause.
        record_text_edit_at(
            &conn,
            "a1",
            PROCESS_TEXT_EDIT,
            "cuatro",
            start + 4_000 + TEXT_EDIT_SESSION_MS + 1,
        )
        .unwrap();
        let processes: Vec<String> = rows(&conn).into_iter().map(|row| row.0).collect();
        assert_eq!(
            processes,
            vec!["text_edit", "ocr", "text_edit", "text_edit"]
        );
    }

    #[test]
    fn renderer_reports_are_allowlisted_and_hash_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let before = dir.path().join("a.png");
        let after = dir.path().join("a_v2.png");
        std::fs::write(&before, b"antes").unwrap();
        std::fs::write(&after, b"despues").unwrap();

        let report = RendererProcessReport {
            asset_id: "a1".to_string(),
            process: "image_crop".to_string(),
            params: Some(json!({"x": 1, "y": 2, "width": 3, "height": 4})),
            previous_path: None,
        };
        let event = build_renderer_event(&report, Some(&after), Some(&before)).unwrap();
        assert_eq!(event.tool, "EntropIA image_edit (crate image)");
        assert_eq!(event.input_sha256, Some(sha256_hex(b"antes")));
        assert_eq!(event.output_sha256, Some(sha256_hex(b"despues")));

        let forged = RendererProcessReport {
            process: "ocr".to_string(),
            ..report.clone()
        };
        assert!(build_renderer_event(&forged, None, None).is_err());
        let bad_params = RendererProcessReport {
            params: Some(json!([1, 2])),
            ..report
        };
        assert!(build_renderer_event(&bad_params, None, None).is_err());
    }

    #[test]
    fn history_goes_with_its_asset() {
        let conn = conn();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        record(&conn, &ProcessEvent::new("a1", PROCESS_IMPORT, "EntropIA")).unwrap();
        conn.execute("DELETE FROM assets WHERE id = 'a1'", [])
            .unwrap();
        assert!(rows(&conn).is_empty());
    }
}
