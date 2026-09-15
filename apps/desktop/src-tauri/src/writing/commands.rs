//! Tauri surface for the writing workspace.
//!
//! These commands return `Result<T, WritingError>` rather than the
//! `Result<T, String>` the rest of the backend uses. §20 asks for typed,
//! actionable errors, and the one the frontend must branch on — a stale save
//! losing to a concurrent one — is impossible to distinguish reliably by
//! matching message text. `WritingError` serialises as `{ code, message }`,
//! so the caller switches on `code` and shows `message`.

use tauri::State;

use super::repository::{
    self, DocumentRow, NewDocument, SaveDocument, WritingError, WritingResult,
};
use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;

fn open(db_path: &std::path::Path) -> WritingResult<rusqlite::Connection> {
    open_archive_connection(db_path).map_err(|e| WritingError::new("db_unavailable", e))
}

fn joined(context: &str, e: tokio::task::JoinError) -> WritingError {
    WritingError::new("task_failed", format!("{context}: {e}"))
}

/// Whether the writing schema has been migrated. The frontend calls this after
/// `initStore()` resolves and keeps the section disabled until it reports true,
/// exactly as the batch queue gates on `processing_initialize`.
#[tauri::command]
pub async fn writing_is_ready(db: State<'_, AppDbState>) -> WritingResult<bool> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::is_schema_ready(&open(&db_path)?))
        .await
        .map_err(|e| joined("writing_is_ready", e))?
}

#[tauri::command]
pub async fn writing_create_document(
    db: State<'_, AppDbState>,
    input: NewDocument,
) -> WritingResult<DocumentRow> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::create_document(&open(&db_path)?, input))
        .await
        .map_err(|e| joined("writing_create_document", e))?
}

#[tauri::command]
pub async fn writing_load_document(
    db: State<'_, AppDbState>,
    id: String,
) -> WritingResult<DocumentRow> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::load_document(&open(&db_path)?, &id))
        .await
        .map_err(|e| joined("writing_load_document", e))?
}

/// Advances the manuscript one revision. Returns the new revision, so the UI
/// can only ever confirm "Guardado" against a number persistence acknowledged
/// (§16.1). A `revision_conflict` here means another window won; the caller
/// reloads and merges rather than retrying blindly.
#[tauri::command]
pub async fn writing_save_document(
    db: State<'_, AppDbState>,
    save: SaveDocument,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = open(&db_path)?;
        repository::save_document(&mut conn, save)
    })
    .await
    .map_err(|e| joined("writing_save_document", e))?
}
