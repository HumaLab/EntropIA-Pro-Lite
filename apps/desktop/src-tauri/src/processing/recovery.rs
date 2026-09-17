//! Startup recovery for the batch queue (plan-lote.md §8).
//!
//! A restart must never lose confirmed work and never silently resume
//! billable or destructive calls. Recovery therefore converges every
//! interrupted unit to a state a human can resume from — it never starts
//! motors itself:
//!
//! - confirmed checkpoints survive; every `running` unit whose supervisor is
//!   gone becomes `interrupted` and replays only its missing units. An OS
//!   file lock, held for the process lifetime, excludes other supervisors.
//! - persisted pause/cancel intents finish converging (`pausing` → `paused`,
//!   `cancelling` → `cancelled`);
//! - live `running`/`ready` batches become `interrupted` and wait for an
//!   explicit resume — no auto-restart of potentially billable calls.
//!
//! Runs once per process through [`recover_once_if_needed`], called from
//! `processing_initialize` after the schema gate opens.

use fs2::FileExt;
use rusqlite::Connection;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use super::repository;
use crate::db::open::open_archive_connection;

static OWNER: Mutex<Option<File>> = Mutex::new(None);
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
    /// True when a live peer supervises this archive: nothing was touched.
    pub peer_alive: bool,
}

/// Recovers once per process. Later calls (tests, repeated initializes)
/// report `None`: recovery already converged this process's view.
pub fn recover_once_if_needed(db_path: &Path) -> Result<Option<RecoverySummary>, String> {
    let mut owner = OWNER.lock().map_err(|e| e.to_string())?;
    if owner.is_some() {
        return Ok(None);
    }
    let conn = open_archive_connection(db_path)?;
    if !repository::is_schema_ready(&conn)? {
        return Err(format!(
            "{}: migrations are not ready",
            repository::SCHEMA_NOT_READY
        ));
    }
    let canonical = db_path.canonicalize().map_err(|e| e.to_string())?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(canonical.with_extension("processing.lock"))
        .map_err(|e| e.to_string())?;
    match lock.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if lock_is_contended(&error) => {
            return Ok(Some(RecoverySummary {
                peer_alive: true,
                ..Default::default()
            }));
        }
        Err(error) => return Err(format!("Cannot acquire processing ownership: {error}")),
    }
    let summary = recover_session(&conn, "", repository::now_ms())?;
    *owner = Some(lock);
    super::scheduler::READY.store(true, std::sync::atomic::Ordering::Release);
    Ok(Some(summary))
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    // LockFileEx reports ERROR_LOCK_VIOLATION (33) as PermissionDenied on
    // supported Windows versions, whereas Unix maps contention to WouldBlock.
    #[cfg(windows)]
    return error.raw_os_error() == Some(33);
    #[cfg(not(windows))]
    false
}

/// Converges one archive to a resumable state. Single transaction: either
/// the whole pass applies or nothing does — a crash mid-recovery simply
/// reruns it on the next start.
///
/// The caller must hold the archive's exclusive OS lock. Every `running`
/// unit is then parked `interrupted`, even if its last heartbeat looked
/// fresh; a timestamp never authorizes takeover from a live process.
pub fn recover_session(
    conn: &Connection,
    _session_id: &str,
    _now_ms: i64,
) -> Result<RecoverySummary, String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("Failed to begin recovery: {e}"))?;
    let summary = (|| -> Result<RecoverySummary, String> {
        let mut out = RecoverySummary::default();
        let dead: Vec<String> = conn
            .prepare("SELECT id FROM processing_tasks WHERE state = 'running'")
            .map_err(|e| format!("Failed to scan dead units: {e}"))?
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Failed to scan dead units: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to scan dead units: {e}"))?;
        for task_id in dead {
            conn.execute(
                "UPDATE processing_tasks SET state = 'interrupted', owner_session = NULL,
                   lease_epoch = lease_epoch + 1, updated_at = strftime('%s', 'now') * 1000 WHERE id = ?1",
                [&task_id],
            )
            .map_err(|e| format!("Failed to interrupt {task_id}: {e}"))?;
            out.tasks_interrupted += 1;
        }
        let closed = conn
            .execute(
                "UPDATE processing_attempts SET outcome = 'interrupted',
                   finished_at = strftime('%s', 'now') * 1000
                 WHERE outcome = 'open'
                   AND task_id IN (SELECT id FROM processing_tasks WHERE state = 'interrupted')",
                [],
            )
            .map_err(|e| format!("Failed to close attempts: {e}"))?;
        out.attempts_closed = closed;
        // Batches: running work waits for resume; confirmed intents converge.
        // `user` only. A `repair`/`manual` batch is a long-lived container
        // that is always running with an empty complete snapshot — parking one
        // has no human to resume it, `ensure_system_batch` only reopens from a
        // terminal state, and `maybe_finalize_batch` ignores non-user origins.
        // It would stay interrupted forever, and every automatic repair
        // admitted into it would sit in a batch the scheduler cannot claim.
        let running: Vec<String> = conn
            .prepare(
                "SELECT id FROM processing_batches
                 WHERE state IN ('running','ready','preparing') AND origin = 'user'",
            )
            .map_err(|e| format!("Failed to scan running batches: {e}"))?
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Failed to scan running batches: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to scan running batches: {e}"))?;
        for batch_id in running {
            conn.execute(
                "UPDATE processing_batches SET state = 'interrupted', desired_state = 'pause',
                   updated_at = strftime('%s', 'now') * 1000 WHERE id = ?1",
                [&batch_id],
            )
            .map_err(|e| format!("Failed to interrupt batch {batch_id}: {e}"))?;
            out.batches_interrupted += 1;
        }
        // Heal containers an earlier build parked: they carry no lease state
        // of their own (their units were already interrupted above), so
        // restoring the invariant is all that is needed.
        conn.execute(
            "UPDATE processing_batches SET state = 'running', desired_state = 'run',
               finished_at = NULL, updated_at = strftime('%s', 'now') * 1000
             WHERE origin IN ('manual', 'repair') AND state != 'running'",
            [],
        )
        .map_err(|e| format!("Failed to restore system batches: {e}"))?;
        let pausing = conn.execute(
            "UPDATE processing_batches SET state = 'paused', updated_at = strftime('%s', 'now') * 1000
             WHERE state = 'pausing' AND origin = 'user'",
            [],
        )
        .map_err(|e| format!("Failed to pause batches: {e}"))?;
        out.batches_paused = pausing;
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
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0033_processing_source_invalidation.sql"
        ))
        .expect("apply 0033");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0033_processing_source_invalidation', 1)",
            [],
        )
        .expect("track 0033");
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
        let summary = recover_session(&conn, "", 60_000).expect("recover");
        // No scheduler heartbeat exists: every running unit parks, however
        // fresh its lease looks — leases mean nothing without a living owner.
        assert_eq!(summary.tasks_interrupted, 2);
        assert_eq!(summary.tasks_live_elsewhere, 0);
        assert!(!summary.peer_alive);
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
        let again = recover_session(&conn, "", 60_000).expect("recover again");
        assert_eq!(again.cancellations_finished, 0);
    }

    #[test]
    fn recovery_leaves_system_batches_running() {
        let (_dir, conn) = recovery_db();
        // `repair` and `manual` are long-lived containers, not user work: they
        // are always running with an empty complete snapshot so admitted units
        // flow straight to the scheduler. Parking one has no human to resume
        // it, and every automatic repair admitted afterwards would sit in a
        // paused batch the scheduler refuses to claim from.
        let repair = repository::ensure_system_batch(&conn, "repair").expect("repair batch");
        let summary = recover_session(&conn, "", 0).expect("recover");

        assert_eq!(summary.batches_interrupted, 0);
        let (state, desired): (String, String) = conn
            .query_row(
                "SELECT state, desired_state FROM processing_batches WHERE id = ?1",
                [&repair],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("system batch row");
        assert_eq!((state.as_str(), desired.as_str()), ("running", "run"));
    }

    #[test]
    fn recovery_revives_a_system_batch_an_older_build_parked() {
        let (_dir, conn) = recovery_db();
        let repair = repository::ensure_system_batch(&conn, "repair").expect("repair batch");
        // Exactly what shipped builds left behind on every restart.
        conn.execute(
            "UPDATE processing_batches SET state = 'interrupted', desired_state = 'pause' WHERE id = ?1",
            [&repair],
        )
        .expect("park it");

        recover_session(&conn, "", 0).expect("recover");

        let (state, desired): (String, String) = conn
            .query_row(
                "SELECT state, desired_state FROM processing_batches WHERE id = ?1",
                [&repair],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("system batch row");
        assert_eq!((state.as_str(), desired.as_str()), ("running", "run"));
    }

    #[test]
    fn operating_system_lock_prevents_second_owner_and_releases_on_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.processing.lock");
        let first = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        first.try_lock_exclusive().unwrap();
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(second.try_lock_exclusive().is_err());
        drop(first);
        second.try_lock_exclusive().unwrap();
    }

    #[test]
    fn recovery_parks_ready_and_preparing_until_explicit_resume() {
        let (_dir, conn) = recovery_db();
        for state in ["ready", "preparing"] {
            conn.execute("INSERT INTO processing_batches(id,request_id,origin,state,desired_state,operations,created_at,updated_at) VALUES(?1,?1,'user',?1,'run','[]',1,1)", [state]).unwrap();
        }
        let summary = recover_session(&conn, "", 0).unwrap();
        assert_eq!(summary.batches_interrupted, 2);
        let runnable: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batches WHERE desired_state='run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(runnable, 0);
    }
}
