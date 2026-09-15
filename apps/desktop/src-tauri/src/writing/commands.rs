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
use super::{journal, recovery, versions};
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

/// The section's document list (§6.1). `statuses` filters by lifecycle state;
/// an empty list means every state, so the trash view and the active view are
/// the same call with different arguments.
#[tauri::command]
pub async fn writing_list_documents(
    db: State<'_, AppDbState>,
    statuses: Vec<String>,
) -> WritingResult<Vec<DocumentRow>> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::list_documents(&open(&db_path)?, &statuses))
        .await
        .map_err(|e| joined("writing_list_documents", e))?
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

/// A title is metadata, so this does not advance the content revision: renaming
/// while an edit is in flight must not turn that edit into a conflict.
#[tauri::command]
pub async fn writing_rename_document(
    db: State<'_, AppDbState>,
    id: String,
    title: String,
) -> WritingResult<()> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::rename_document(&open(&db_path)?, &id, &title))
        .await
        .map_err(|e| joined("writing_rename_document", e))?
}

/// Moves a document between `active`, `archived` and `trashed` (§9.1). One
/// command rather than three, because they are one transition with three
/// destinations; an unknown value answers `invalid_status` without touching
/// the database.
#[tauri::command]
pub async fn writing_set_status(
    db: State<'_, AppDbState>,
    id: String,
    status: String,
) -> WritingResult<()> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::set_status(&open(&db_path)?, &id, &status))
        .await
        .map_err(|e| joined("writing_set_status", e))?
}

/// Appends one delta to the recovery journal and returns its **sequence
/// number** — never a revision. §16.2 draws the line hard: persisting here
/// leaves the UI on "Cambios pendientes"; only `writing_save_document`
/// advancing the revision earns "Guardado".
#[tauri::command]
pub async fn writing_append_journal(
    db: State<'_, AppDbState>,
    entry: journal::AppendJournal,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || journal::append(&open(&db_path)?, entry))
        .await
        .map_err(|e| joined("writing_append_journal", e))?
}

/// What can be recovered for a document, and what cannot (§16.3). Read-only:
/// building a plan changes nothing, so it is safe to call on every open.
#[tauri::command]
pub async fn writing_recovery_plan(
    db: State<'_, AppDbState>,
    document_id: String,
) -> WritingResult<recovery::RecoveryPlan> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || recovery::plan(&open(&db_path)?, &document_id))
        .await
        .map_err(|e| joined("writing_recovery_plan", e))?
}

/// Drops journal entries a confirmed revision already contains. Deliberately
/// not automatic on save: §16.1 requires the journal protecting a sequence to
/// survive until a canonical version containing it exists, so the caller prunes
/// once it has seen the new revision come back.
#[tauri::command]
pub async fn writing_prune_journal(
    db: State<'_, AppDbState>,
    document_id: String,
    confirmed_revision: i64,
) -> WritingResult<usize> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        journal::prune_confirmed(&open(&db_path)?, &document_id, confirmed_revision)
    })
    .await
    .map_err(|e| joined("writing_prune_journal", e))?
}

/// Discards a recovery offer outright. §16.3 asks for a confirmation before
/// this runs and for the copy to stay recoverable for a while; both belong to
/// the caller, because only it knows the human said yes.
#[tauri::command]
pub async fn writing_discard_journal(
    db: State<'_, AppDbState>,
    document_id: String,
) -> WritingResult<usize> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || journal::discard_all(&open(&db_path)?, &document_id))
        .await
        .map_err(|e| joined("writing_discard_journal", e))?
}

/// Writes a snapshot of the document as it stands (§9.3). `reason` records why:
/// only "auto" is ever compacted by retention.
#[tauri::command]
pub async fn writing_snapshot_version(
    db: State<'_, AppDbState>,
    document_id: String,
    reason: String,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || versions::snapshot(&open(&db_path)?, &document_id, &reason))
        .await
        .map_err(|e| joined("writing_snapshot_version", e))?
}

/// The history panel's list, newest first (§16.4).
#[tauri::command]
pub async fn writing_list_versions(
    db: State<'_, AppDbState>,
    document_id: String,
) -> WritingResult<Vec<versions::VersionSummary>> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || versions::list(&open(&db_path)?, &document_id))
        .await
        .map_err(|e| joined("writing_list_versions", e))?
}

/// One version's content plus the settings that rendered it, so the caller can
/// preview it and derive the projections a restore will need.
#[tauri::command]
pub async fn writing_read_version(
    db: State<'_, AppDbState>,
    document_id: String,
    version_number: i64,
) -> WritingResult<versions::VersionContent> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        versions::read(&open(&db_path)?, &document_id, version_number)
    })
    .await
    .map_err(|e| joined("writing_read_version", e))?
}

/// Reinstates an earlier version as a NEW revision (§16.4). The history after
/// it survives, and the state being replaced is snapshotted first.
#[tauri::command]
pub async fn writing_restore_version(
    db: State<'_, AppDbState>,
    restore: versions::RestoreVersion,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = open(&db_path)?;
        versions::restore(&mut conn, restore)
    })
    .await
    .map_err(|e| joined("writing_restore_version", e))?
}

/// Compacts automatic snapshots, keeping the most recent `keep` (§9.3). A
/// checkpoint, a close, a restore or a migration records a human decision and
/// is never a candidate.
#[tauri::command]
pub async fn writing_apply_retention(
    db: State<'_, AppDbState>,
    document_id: String,
    keep: usize,
) -> WritingResult<usize> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        versions::apply_retention(&open(&db_path)?, &document_id, keep)
    })
    .await
    .map_err(|e| joined("writing_apply_retention", e))?
}

/// Duplicates a document per §8.4: own identity, own citation occurrences, a
/// recorded origin, and none of the original's pending suggestions or history.
#[tauri::command]
pub async fn writing_duplicate_document(
    db: State<'_, AppDbState>,
    source_id: String,
    new_id: String,
    new_title: String,
) -> WritingResult<DocumentRow> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = open(&db_path)?;
        repository::duplicate_document(&mut conn, &source_id, &new_id, &new_title)
    })
    .await
    .map_err(|e| joined("writing_duplicate_document", e))?
}
