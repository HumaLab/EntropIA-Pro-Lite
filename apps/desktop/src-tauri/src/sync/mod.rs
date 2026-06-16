//! Cloud sync client module (DESIGN §3). Owns the local `sync_*` schema and the
//! capture triggers. The sync engine opens its OWN SQLite connection and never
//! contends on `ui_conn`.

pub mod apply;
pub mod blobs;
pub mod capture;
pub mod cascade;
pub mod commands;
pub mod engine;
pub mod http;
pub mod pull;
pub mod push;
pub mod schema;
pub mod session;

#[cfg(test)]
pub mod test_support;

use rusqlite::Connection;
use tauri::{AppHandle, State};

use crate::db::state::AppDbState;

/// Opens a dedicated sync connection with the standard pragmas. The sync module
/// must never share `ui_conn`/`worker_conn` (DESIGN §3, house rules). Shared by
/// the capture bootstrap and the session/push commands so every sync path uses
/// an identically-configured connection.
pub(crate) fn open_sync_connection(db_path: &std::path::Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path)
        .map_err(|e| format!("[sync] failed to open sync connection: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )
    .map_err(|e| format!("[sync] failed to configure sync connection pragmas: {e}"))?;
    Ok(conn)
}

/// Ensures the sync schema and the 45 capture triggers on a fresh connection.
/// Shared by the Tauri command and the Rust `setup()` bootstrap so both paths
/// run identical logic.
pub fn ensure_capture_on_path(db_path: &std::path::Path) -> Result<(), String> {
    let conn = open_sync_connection(db_path)?;
    capture::ensure_capture(&conn)
}

/// Tauri command: ensure the sync schema + capture triggers are installed
/// (DESIGN §6.1). Called by the frontend right after `initStore()` resolves
/// (so JS migrations have finished) and by Rust `setup()` after all
/// `ensure_*`/`migrate_*` patches. Opens its own connection on the blocking
/// pool — it never touches `ui_conn`.
#[tauri::command]
pub async fn sync_ensure_capture(db: State<'_, AppDbState>) -> Result<(), String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || ensure_capture_on_path(&db_path))
        .await
        .map_err(|e| format!("[sync] ensure_capture task failed: {e}"))?
}

/// Tauri command: resets `uploaded=0` for every owned asset so the next push
/// re-confirms each blob via HEAD/PUT (DESIGN §7). Repopulates a restored server
/// (HEAD answers from the filesystem). Opens its own connection on the blocking
/// pool. Returns the number of blob index rows reset.
#[tauri::command]
pub async fn sync_reverify_blobs(
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<usize, String> {
    let db_path = db.db_path.clone();
    let reset = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let conn = open_sync_connection(&db_path)?;
        blobs::reverify_all_blobs(&conn)
    })
    .await
    .map_err(|e| format!("[sync] reverify_blobs task failed: {e}"))??;
    crate::app_logs::info(
        &app_handle,
        "sync",
        format!("Re-verificación de blobs: {reset} marcados para re-subir"),
    );
    Ok(reset)
}
