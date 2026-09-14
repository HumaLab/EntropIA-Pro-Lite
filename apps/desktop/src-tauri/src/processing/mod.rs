//! Batch processing queue: durable background OCR + embeddings (plan-lote.md).
//!
//! One module owns admission, scheduling, recovery and observation. The Svelte
//! UI only reads snapshots and sends control intents; it never executes work
//! or holds queue state. Unidad 1 establishes the durable schema gate —
//! [`processing_initialize`] — that every later unit builds on:
//!
//! - the frontend calls it once after `initStore()` resolves (real migrations
//!   have run), and the backend verifies the migration row plus the effective
//!   PRAGMAs before any admission or recovery;
//! - until it reports ready, queue admission answers `schema_not_ready` and no
//!   scheduler claims work (claim/recovery arrive in Unidad 3, engines in
//!   Unidad 4).

pub mod eligibility;
pub mod repository;

use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;
use serde::Serialize;
use tauri::State;

/// Snapshot returned by [`processing_initialize`]. Counts come from the durable
/// tables, so a reopened app rebuilds the same recovery banner without
/// replaying events.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingInitResponse {
    pub ready: bool,
    pub migration: String,
    pub pending_batches: i64,
    pub active_tasks: i64,
    pub interrupted_tasks: i64,
    pub failed_tasks: i64,
    pub succeeded_tasks: i64,
}

/// Verifies the queue schema gate and returns the recovery summary.
///
/// Idempotent: safe to call on every startup and after every migration run.
/// Returns `Err("schema_not_ready: ...")` while the frontend migrations have
/// not applied `0032_batch_processing` yet — the caller must surface a
/// recoverable state, never start queue workers regardless.
#[tauri::command]
pub async fn processing_initialize(
    db: State<'_, AppDbState>,
) -> Result<ProcessingInitResponse, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        if !repository::is_schema_ready(&conn)? {
            return Err(format!(
                "{}: {} is not applied yet",
                repository::SCHEMA_NOT_READY,
                repository::MIGRATION_NAME
            ));
        }
        let summary = repository::read_summary(&conn)?;
        Ok(ProcessingInitResponse {
            ready: true,
            migration: repository::MIGRATION_NAME.to_string(),
            pending_batches: summary.pending_batches,
            active_tasks: summary.active_tasks,
            interrupted_tasks: summary.interrupted_tasks,
            failed_tasks: summary.failed_tasks,
            succeeded_tasks: summary.succeeded_tasks,
        })
    })
    .await
    .map_err(|e| format!("processing_initialize task failed: {e}"))?
}
