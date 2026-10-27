//! Tauri commands of the Biblioteca annotations.
//!
//! Every command names a library row and an annotation key; the API key is
//! read here, in Rust, and never reaches the frontend. Writes to Zotero need
//! a key with write access to the personal library and the open Zotero signed
//! in to that same account (plan section 2).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use super::engine::{sync_library, CatalogParents, SyncReport};
use super::http::{WebAnnotationApi, API_BASE};
use super::machine::{self, Decision, RowAction};
use super::model::AnnotationRow;
use super::repo::{self, Scope};
use super::wire::{self, Data};
use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;
use crate::navegador::zotero_copy::port::{ConnectorPort, ZoteroPort};
use crate::zotero_web::{fetch_key_info, stored_credentials, KeyCheck, ZoteroCredentials};

/// Emitted after any change to a library's annotations.
pub const CHANGED_EVENT: &str = "bibliography-annotations:changed";

/// A pull triggered by opening a work is skipped this soon after the last one.
const OPEN_DEBOUNCE: Duration = Duration::from_secs(30);

/// Annotation types EntropIA creates (ink comes later).
const CREATABLE_TYPES: [&str; 5] = ["highlight", "underline", "note", "image", "text"];

/// Server limits mirrored before anything is queued (plan section 7).
const MAX_POSITION_BYTES: usize = 65_000;
const MAX_PAGE_LABEL_CHARS: usize = 50;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationView {
    pub key: String,
    pub parent_key: String,
    /// The data the user currently sees: the pending edit, or the base.
    pub data: Option<Data>,
    pub sync_state: &'static str,
    pub conflict_fields: Vec<String>,
    /// Zotero's values of the disputed fields; `__existence` holds the reason.
    pub conflict_remote: Option<Data>,
    pub remote_trashed: bool,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationList {
    /// `ready`, `no_key` (no verified Zotero key), or `not_personal` (a group
    /// library, out of scope).
    pub status: &'static str,
    pub annotations: Vec<AnnotationView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSyncResult {
    /// `synced`, `skipped`, `no_key`, `no_write`, `not_personal`,
    /// `other_account`, `account_unknown`, `invalid_key` or `unreachable`.
    pub status: &'static str,
    pub report: Option<SyncReport>,
}

async fn blocking<F, T>(task: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| format!("annotation task failed: {error}"))?
}

fn now_ms() -> i64 {
    crate::processing::repository::now_ms()
}

/// `true` when the library row is a personal library.
fn is_personal(conn: &Connection, library_row_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT library_type FROM zotero_libraries WHERE id = ?1",
        [library_row_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|kind| kind.as_deref() == Some("user"))
    .map_err(|error| format!("library lookup failed: {error}"))
}

/// The scope of a personal library for the stored key, or the status that
/// stops the caller.
fn scope_for(conn: &Connection, library_row_id: &str) -> Result<Result<(Scope, ZoteroCredentials), &'static str>, String> {
    if !is_personal(conn, library_row_id)? {
        return Ok(Err("not_personal"));
    }
    let Some(credentials) = stored_credentials(conn) else {
        return Ok(Err("no_key"));
    };
    Ok(Ok((
        Scope { library_row_id: library_row_id.to_string(), web_user_id: credentials.user_id.to_string() },
        credentials,
    )))
}

fn updated_at(conn: &Connection, scope: &Scope, key: &str) -> i64 {
    conn.query_row(
        "SELECT updated_at FROM zotero_annotation_items WHERE library_row_id = ?1 AND annotation_key = ?2",
        [&scope.library_row_id, key],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

fn view(conn: &Connection, scope: &Scope, row: &AnnotationRow) -> AnnotationView {
    AnnotationView {
        key: row.annotation_key.clone(),
        parent_key: row.parent_key.clone(),
        data: row.desired().cloned(),
        sync_state: row.sync_state.as_str(),
        conflict_fields: row.conflict_fields.clone(),
        conflict_remote: row.conflict_json.clone(),
        remote_trashed: row.remote_trashed,
        last_error: row.last_error.clone(),
        updated_at: updated_at(conn, scope, &row.annotation_key),
    }
}

fn emit_changed(app: &AppHandle, library_row_id: &str) {
    let _ = app.emit(CHANGED_EVENT, serde_json::json!({ "libraryRowId": library_row_id }));
}

/// Checks a desired `data` object before it is queued.
pub fn validate_data(data: &Data) -> Result<(), String> {
    let kind = data.get("annotationType").and_then(Value::as_str).unwrap_or_default();
    if !CREATABLE_TYPES.contains(&kind) && kind != "ink" {
        return Err(format!("unsupported annotation type `{kind}`"));
    }
    let position = data.get(wire::POSITION_FIELD).ok_or("the annotation has no position")?;
    let position_len = match position {
        Value::String(text) => text.len(),
        other => other.to_string().len(),
    };
    if position_len > MAX_POSITION_BYTES {
        return Err("the annotation position is too large for Zotero".into());
    }
    if wire::parse_position(data).is_none() {
        return Err("the annotation position is not valid JSON".into());
    }
    let label = data.get("annotationPageLabel").and_then(Value::as_str).unwrap_or_default();
    if label.chars().count() > MAX_PAGE_LABEL_CHARS {
        return Err("the page label is too long for Zotero".into());
    }
    Ok(())
}

/// Normalizes what the frontend sends: the position as Zotero's JSON string,
/// and no server-owned fields.
fn normalize(mut data: Data) -> Data {
    if let Some(Value::Object(position)) = data.get(wire::POSITION_FIELD).cloned() {
        wire::write_position(&mut data, &position);
    }
    data.remove("version");
    data.remove("dateAdded");
    data.remove("dateModified");
    data
}

/// Runs a local mutation on one row in one transaction.
fn mutate(
    db_path: std::path::PathBuf,
    library_row_id: String,
    key: String,
    change: impl FnOnce(&mut AnnotationRow) -> Result<RowAction, String> + Send + 'static,
) -> impl FnOnce() -> Result<Option<AnnotationView>, String> + Send + 'static {
    move || {
        let conn = open_archive_connection(&db_path)?;
        let (scope, _) = match scope_for(&conn, &library_row_id)? {
            Ok(found) => found,
            Err(status) => return Err(status.to_string()),
        };
        // IMMEDIATE, like the sync engine: read and write of one row are one
        // step, so neither side ever writes from a stale copy.
        let tx = super::engine::immediate(&conn)?;
        let mut row = repo::load(&conn, &scope, &key)?.ok_or("annotation not found")?;
        let action = change(&mut row)?;
        repo::apply(&conn, &scope, &key, &row, &action, now_ms())?;
        tx.commit().map_err(|error| format!("annotation write failed: {error}"))?;
        Ok((action != RowAction::Remove).then(|| view(&conn, &scope, &row)))
    }
}

#[tauri::command]
pub async fn bibliography_annotations_list(
    library_row_id: String,
    parent_key: String,
    db: State<'_, AppDbState>,
) -> Result<AnnotationList, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let (scope, _) = match scope_for(&conn, &library_row_id)? {
            Ok(found) => found,
            Err(status) => return Ok(AnnotationList { status, annotations: Vec::new() }),
        };
        let annotations = repo::by_parent(&conn, &scope, &parent_key)?
            .iter()
            .map(|row| view(&conn, &scope, row))
            .collect();
        Ok(AnnotationList { status: "ready", annotations })
    })
    .await
}

#[tauri::command]
pub async fn bibliography_annotations_create(
    app: AppHandle,
    library_row_id: String,
    parent_key: String,
    data: Data,
    db: State<'_, AppDbState>,
) -> Result<AnnotationView, String> {
    let db_path = db.db_path.clone();
    let library = library_row_id.clone();
    let created = blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let (scope, _) = match scope_for(&conn, &library_row_id)? {
            Ok(found) => found,
            Err(status) => return Err(status.to_string()),
        };
        let kind = data.get("annotationType").and_then(Value::as_str).unwrap_or_default();
        if !CREATABLE_TYPES.contains(&kind) {
            return Err(format!("unsupported annotation type `{kind}`"));
        }
        let mut data = normalize(data);
        data.insert("itemType".into(), Value::String("annotation".into()));
        data.insert("parentItem".into(), Value::String(parent_key));
        validate_data(&data)?;
        let key = wire::generate_key();
        data.insert("key".into(), Value::String(key.clone()));
        let row = AnnotationRow::new_local(&key, data);
        repo::save(&conn, &scope, &row, now_ms())?;
        Ok(view(&conn, &scope, &row))
    })
    .await?;
    emit_changed(&app, &library);
    Ok(created)
}

#[tauri::command]
pub async fn bibliography_annotations_update(
    app: AppHandle,
    library_row_id: String,
    key: String,
    data: Data,
    db: State<'_, AppDbState>,
) -> Result<Option<AnnotationView>, String> {
    let library = library_row_id.clone();
    let task = mutate(db.db_path.clone(), library_row_id, key.clone(), move |row| {
        let mut desired = normalize(data);
        // The type never changes on an existing key.
        if let Some(kind) = row.desired().and_then(|d| d.get("annotationType")).cloned() {
            desired.insert("annotationType".into(), kind);
        }
        desired.insert("key".into(), Value::String(row.annotation_key.clone()));
        validate_data(&desired)?;
        Ok(machine::local_edit(row, desired))
    });
    let updated = blocking(task).await?;
    emit_changed(&app, &library);
    Ok(updated)
}

#[tauri::command]
pub async fn bibliography_annotations_delete(
    app: AppHandle,
    library_row_id: String,
    key: String,
    confirmed: bool,
    db: State<'_, AppDbState>,
) -> Result<Option<AnnotationView>, String> {
    let library = library_row_id.clone();
    let task = mutate(db.db_path.clone(), library_row_id, key, move |row| {
        if confirmed && row.in_conflict() {
            return Ok(machine::local_delete_confirmed(row));
        }
        machine::local_delete(row).map_err(|error| match error {
            machine::MachineError::DeleteNeedsConfirmation => "needs_confirmation".to_string(),
            other => other.to_string(),
        })
    });
    let result = blocking(task).await?;
    emit_changed(&app, &library);
    Ok(result)
}

#[tauri::command]
pub async fn bibliography_annotations_restore(
    app: AppHandle,
    library_row_id: String,
    key: String,
    db: State<'_, AppDbState>,
) -> Result<Option<AnnotationView>, String> {
    let library = library_row_id.clone();
    let task = mutate(db.db_path.clone(), library_row_id, key, |row| {
        machine::local_restore(row).map_err(|error| error.to_string())
    });
    let result = blocking(task).await?;
    emit_changed(&app, &library);
    Ok(result)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionInput {
    Keep,
    Delete,
    Local,
    Zotero,
}

#[tauri::command]
pub async fn bibliography_annotations_resolve(
    app: AppHandle,
    library_row_id: String,
    key: String,
    decisions: BTreeMap<String, DecisionInput>,
    db: State<'_, AppDbState>,
) -> Result<Option<AnnotationView>, String> {
    let library = library_row_id.clone();
    let task = mutate(db.db_path.clone(), library_row_id, key, move |row| {
        let decisions = decisions
            .into_iter()
            .map(|(field, decision)| {
                let decision = match decision {
                    DecisionInput::Keep => Decision::Keep,
                    DecisionInput::Delete => Decision::Delete,
                    DecisionInput::Local => Decision::Local,
                    DecisionInput::Zotero => Decision::Zotero,
                };
                (field, decision)
            })
            .collect();
        machine::resolve(row, &decisions).map_err(|error| error.to_string())
    });
    let result = blocking(task).await?;
    emit_changed(&app, &library);
    Ok(result)
}

fn last_sync_times() -> &'static Mutex<HashMap<String, Instant>> {
    static TIMES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    TIMES.get_or_init(Default::default)
}

/// One sync run of a library. `reason` is `open` (debounced), `edit` or
/// `catalog_sync`.
#[tauri::command]
pub async fn bibliography_annotations_sync(
    app: AppHandle,
    library_row_id: String,
    reason: String,
    db: State<'_, AppDbState>,
) -> Result<AnnotationSyncResult, String> {
    if reason == "open" {
        let times = last_sync_times().lock().unwrap_or_else(|e| e.into_inner());
        if times.get(&library_row_id).is_some_and(|at| at.elapsed() < OPEN_DEBOUNCE) {
            return Ok(AnnotationSyncResult { status: "skipped", report: None });
        }
    }
    let db_path = db.db_path.clone();
    let library = library_row_id.clone();
    let scope = blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        scope_for(&conn, &library)
    })
    .await?;
    let (scope, credentials) = match scope {
        Ok(found) => found,
        Err(status) => return Ok(AnnotationSyncResult { status, report: None }),
    };
    match fetch_key_info(API_BASE, &credentials.key).await? {
        KeyCheck::Valid(info) if info.user_id == credentials.user_id && info.can_write_user() => {}
        KeyCheck::Valid(_) => return Ok(AnnotationSyncResult { status: "no_write", report: None }),
        KeyCheck::InvalidKey => return Ok(AnnotationSyncResult { status: "invalid_key", report: None }),
        KeyCheck::Unreachable => return Ok(AnnotationSyncResult { status: "unreachable", report: None }),
    }
    let db_path = db.db_path.clone();
    let result = blocking(move || {
        // The personal library is whichever account the open Zotero is signed
        // in to; a key of another account would write someone else's library.
        match ConnectorPort::local().local_user_id() {
            Ok(Some(open)) if open == credentials.user_id => {}
            Ok(Some(_)) => return Ok(AnnotationSyncResult { status: "other_account", report: None }),
            Ok(None) | Err(_) => return Ok(AnnotationSyncResult { status: "account_unknown", report: None }),
        }
        let conn = open_archive_connection(&db_path)?;
        let api = WebAnnotationApi::new(API_BASE, &credentials.key, credentials.user_id)?;
        let parents = CatalogParents { conn: &conn, library_row_id: &scope.library_row_id };
        let report = sync_library(&conn, &scope, &api, &parents, now_ms())?;
        Ok(AnnotationSyncResult { status: "synced", report: Some(report) })
    })
    .await?;
    last_sync_times()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(library_row_id.clone(), Instant::now());
    if result.status == "synced" {
        emit_changed(&app, &library_row_id);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data(value: Value) -> Data {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn validation_rejects_what_zotero_would_refuse() {
        let good = data(json!({"annotationType": "highlight", "annotationPosition": "{\"pageIndex\":0,\"rects\":[]}"}));
        assert_eq!(validate_data(&good), Ok(()));
        let no_position = data(json!({"annotationType": "note"}));
        assert!(validate_data(&no_position).is_err());
        let huge = data(json!({"annotationType": "image", "annotationPosition": "x".repeat(70_000)}));
        assert!(validate_data(&huge).is_err());
        let label = data(json!({"annotationType": "note", "annotationPosition": "{}", "annotationPageLabel": "x".repeat(51)}));
        assert!(validate_data(&label).is_err());
        let unknown = data(json!({"annotationType": "comment", "annotationPosition": "{}"}));
        assert!(validate_data(&unknown).is_err());
    }

    #[test]
    fn normalize_turns_the_position_into_zotero_s_json_string() {
        let normalized = normalize(data(json!({
            "annotationPosition": {"pageIndex": 2, "rects": [[1, 2, 3, 4]]},
            "version": 9,
            "dateModified": "x"
        })));
        assert!(normalized["annotationPosition"].is_string());
        assert!(!normalized.contains_key("version"));
        assert!(!normalized.contains_key("dateModified"));
    }

    #[test]
    fn group_libraries_and_missing_keys_stop_before_any_work() {
        let conn = super::super::repo_test_support::archive_with_library("lib1");
        assert_eq!(scope_for(&conn, "lib1").unwrap().err(), Some("no_key"));
        conn.execute("UPDATE zotero_libraries SET library_type = 'group', library_id = '7' WHERE id = 'lib1'", [])
            .unwrap();
        assert_eq!(scope_for(&conn, "lib1").unwrap().err(), Some("not_personal"));
    }
}
