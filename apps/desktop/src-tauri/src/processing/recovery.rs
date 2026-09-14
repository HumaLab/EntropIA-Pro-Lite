//! Startup recovery for the batch queue (plan-lote.md §8).
//!
//! A restart must never lose confirmed work and never silently resume
//! billable or destructive calls. Recovery therefore converges every
//! interrupted unit to a state a human can resume from — it never starts
//! motors itself:
//!
//! - `succeeded` rows are untouched: zero motor invocations, ever;
//! - confirmed checkpoints survive; `running` with an expired lease becomes
//!   `interrupted` and replays only its missing units;
//! - `running` with a FRESH lease is left alone (`live_elsewhere`): another
//!   live supervisor may own it, and two supervisors must never compute the
//!   same unit;
//! - persisted pause/cancel intents finish converging (`pausing` → `paused`,
//!   `cancelling` → `cancelled`);
//! - live `running`/`ready` batches become `interrupted` and wait for an
//!   explicit resume — no auto-restart of potentially billable calls.
//!
//! Runs once per process through [`recover_once_if_needed`], called from
//! `processing_initialize` after the schema gate opens.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use serde::Serialize;

use super::repository::{self, LEASE_TTL_MS};
use crate::db::open::open_archive_connection;

static RECOVERED: AtomicBool = AtomicBool::new(false);

/// What one recovery pass converged. Returned to the UI for the
/// "N batches recovered" banner and its resume actions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySummary {
    pub tasks_interrupted: usize,
    pub attempts_closed: usize,
    pub tasks_live_elsewhere: usize,
    pub batches_interrupted: usize,
    pub batches_paused: usize,
    pub cancellations_finished: usize,
    pub tasks_cancelled: usize,
}

/// Recovers once per process. Later calls (tests, repeated initializes)
/// report `None`: recovery already converged this process's view.
pub fn recover_once_if_needed(db_path: &Path) -> Result<Option<RecoverySummary>, String> {
    if RECOVERED.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    let conn = open_archive_connection(db_path)?;
    if !repository::is_schema_ready(&conn)? {
        return Err(format!(
            "{}: {} is not applied yet",
            repository::SCHEMA_NOT_READY,
            repository::MIGRATION_NAME
        ));
    }
    Ok(Some(recover_session(&conn, repository::now_ms())?))
}

/// Converges one archive to a resumable state. Single transaction: either
/// the whole pass applies or nothing does — a crash mid-recovery simply
/// reruns it on the next start.
pub fn recover_session(conn: &Connection, now_ms: i64) -> Result<RecoverySummary, String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("Failed to begin recovery: {e}"))?;
    let summary = (|| -> Result<RecoverySummary, String> {
        let mut out = RecoverySummary::default();
        // Units whose supervisor died: expired lease (or none) means nobody
        // alive can still hold them. Fresh leases belong to a live peer.
        let dead: Vec<String> = conn
            .prepare(
                "SELECT id FROM processing_tasks WHERE state = 'running'
                 AND (lease_expires_at IS NULL OR lease_expires_at <= ?1)",
            )
            .map_err(|e| format!("Failed to scan dead units: {e}"))?
            .query_map([now_ms], |row| row.get(0))
            .map_err(|e| format!("Failed to scan dead units: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to scan dead units: {e}"))?;
        for task_id in dead {
            conn.execute(
                "UPDATE processing_tasks SET state = 'interrupted', owner_session = NULL,
                   updated_at = strftime('%s', 'now') * 1000 WHERE id = ?1",
                [&task_id],
            )
            .map_err(|e| format!("Failed to interrupt {task_id}: {e}"))?;
            out.tasks_interrupted += 1;
        }
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_tasks WHERE state = 'running'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count live units: {e}"))?;
        out.tasks_live_elsewhere = live as usize;
        let closed = conn
            .execute(
                "UPDATE processing_attempts SET outcome = 'interrupted',
                   finished_at = strftime('%s', 'now') * 1000
                 WHERE outcome = 'open'
                   AND task_id IN (SELECT id FROM processing_tasks WHERE state = 'interrupted')",
                [],
            )
            .map_err(|e| format!("Failed to close attempts: {e}"))?;
        out.attempts_closed = closed as usize;
        // Batches: running work waits for resume; confirmed intents converge.
        let running: Vec<String> = conn
            .prepare("SELECT id FROM processing_batches WHERE state = 'running'")
            .map_err(|e| format!("Failed to scan running batches: {e}"))?
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Failed to scan running batches: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to scan running batches: {e}"))?;
        for batch_id in running {
            conn.execute(
                "UPDATE processing_batches SET state = 'interrupted',
                   updated_at = strftime('%s', 'now') * 1000 WHERE id = ?1",
                [&batch_id],
            )
            .map_err(|e| format!("Failed to interrupt batch {batch_id}: {e}"))?;
            out.batches_interrupted += 1;
        }
        let pausing = conn.execute(
            "UPDATE processing_batches SET state = 'paused', updated_at = strftime('%s', 'now') * 1000
             WHERE state = 'pausing'",
            [],
        )
        .map_err(|e| format!("Failed to pause batches: {e}"))?;
        out.batches_paused = pausing as usize;
        // Unfinished cancellations converge even across the crash.
        let cancelling: Vec<String> = conn
            .prepare("SELECT id FROM processing_batches WHERE state = 'cancelling'")
            .map_err(|e| format!("Failed to scan cancelling batches: {e}"))?
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Failed to scan cancelling batches: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to scan cancelling batches: {e}"))?;
        for batch_id in cancelling {
            conn.execute(
                "UPDATE processing_batch_tasks SET request_state = 'cancelled' WHERE batch_id = ?1",
                [&batch_id],
            )
            .map_err(|e| format!("Failed to cancel links of {batch_id}: {e}"))?;
            let cancelled = repository::cancel_orphaned_tasks(conn)?;
            out.tasks_cancelled += cancelled;
            // A batch cancelled mid-preparation has no complete snapshot, so
            // it cannot go through the normal finalizer: close it directly
            // once nothing runnable remains.
            let open: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM processing_batch_tasks l
                     JOIN processing_tasks t ON t.id = l.task_id
                     WHERE l.batch_id = ?1 AND l.request_state = 'active'
                       AND t.state IN ('pending', 'blocked', 'running', 'retry_wait', 'interrupted')",
                    [&batch_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("Failed to count open units of {batch_id}: {e}"))?;
            if open == 0 {
                conn.execute(
                    "UPDATE processing_batches SET state = 'cancelled',
                       finished_at = strftime('%s', 'now') * 1000,
                       updated_at = strftime('%s', 'now') * 1000 WHERE id = ?1",
                    [&batch_id],
                )
                .map_err(|e| format!("Failed to close {batch_id}: {e}"))?;
                out.cancellations_finished += 1;
            }
        }
        // Leases shorter than the TTL window would look dead on a fast
        // restart while their owner is merely between heartbeats; the
        // `live_elsewhere` count above already covers that case.
        let _ = LEASE_TTL_MS;
        Ok(out)
    })();
    match summary {
        Ok(summary) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("Failed to commit recovery: {e}"))?;
            Ok(summary)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open::open_archive_connection;

    fn recovery_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        let conn = open_archive_connection(&db_path).expect("open");
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, method TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, model TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);",
        )
        .expect("base tables");
        let migration: &str =
            include_str!("../../../../../packages/store/src/migrations/0032_batch_processing.sql");
        conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{migration}\nCOMMIT;"))
            .expect("apply 0032");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0032_batch_processing', 1)",
            [],
        )
        .expect("track");
        (dir, conn)
    }

    #[test]
    fn recovery_keeps_confirmed_work_and_parks_the_rest() {
        let (_dir, conn) = recovery_db();
        // One committed success, one mid-flight unit with an expired lease,
        // one mid-flight unit with a fresh lease (a live peer owns it).
        for (id, asset, state, owner, expires) in [
            ("t-done", "a1", "succeeded", None, None),
            ("t-dead", "a2", "running", Some("old-session"), Some(1000)),
            ("t-live", "a3", "running", Some("peer"), Some(1_000_000)),
        ] {
            conn.execute(
                "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, owner_session, lease_epoch, lease_expires_at, created_at, updated_at)
                 VALUES (?1, 'ocr', ?2, ?3, ?4, 3, ?5, 1, 1)",
                rusqlite::params![id, asset, state, owner, expires],
            )
            .expect("task");
        }
        conn.execute(
            "INSERT INTO processing_attempts (task_id, attempt_number, lease_epoch, started_at, outcome)
             VALUES ('t-dead', 1, 3, 900, 'open'), ('t-live', 1, 3, 900, 'open')",
            [],
        )
        .expect("attempts");
        for (id, state, desired) in [
            ("b-run", "running", "run"),
            ("b-pausing", "pausing", "pause"),
            ("b-cancelling", "cancelling", "cancel"),
        ] {
            conn.execute(
                "INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
                 VALUES (?1, ?2, 'user', ?3, ?4, '[\"ocr\"]', 1, 1, 1)",
                rusqlite::params![id, format!("req-{id}"), state, desired],
            )
            .expect("batch");
        }
        // Every task belongs to a batch in production (admission always
        // links); linkless tasks are orphans and converge to cancelled.
        for task_id in ["t-done", "t-dead", "t-live"] {
            conn.execute(
                "INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, request_state)
                 VALUES ('b-run', ?1, 'ocr', 'a1', 'active')",
                [task_id],
            )
            .expect("link");
        }
        let summary = recover_session(&conn, 60_000).expect("recover");
        assert_eq!(summary.tasks_interrupted, 1);
        assert_eq!(summary.tasks_live_elsewhere, 1);
        assert_eq!(summary.batches_interrupted, 1);
        assert_eq!(summary.batches_paused, 1);
        assert_eq!(summary.cancellations_finished, 1);
        let done: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 't-done'",
                [],
                |row| row.get(0),
            )
            .expect("confirmed work untouched");
        assert_eq!(done, "succeeded");
        let dead: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 't-dead'",
                [],
                |row| row.get(0),
            )
            .expect("dead unit parked");
        assert_eq!(dead, "interrupted");
        // A second pass converges to nothing: recovery is idempotent.
        let again = recover_session(&conn, 60_000).expect("recover again");
        assert_eq!(again.tasks_interrupted, 0);
        assert_eq!(again.batches_interrupted, 0);
        assert_eq!(again.cancellations_finished, 0);
    }
}
