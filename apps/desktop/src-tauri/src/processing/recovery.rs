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
//! - a bibliography reconciliation owned by an interrupted bibliography task
//!   is parked `interrupted` with its committed item pages/cursor intact;
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
    spawn_checkpoint_cleanup(db_path.to_path_buf());
    super::scheduler::READY.store(true, std::sync::atomic::Ordering::Release);
    Ok(Some(summary))
}

/// Tasks whose checkpoints one cleanup transaction deletes.
const CLEANUP_TASKS_PER_BATCH: usize = 25;
/// Pause between cleanup transactions so other writers get the lock.
const CLEANUP_PAUSE: std::time::Duration = std::time::Duration::from_millis(50);
/// Frees the checkpoints of tasks that already ended (builds before the
/// retention fix kept them forever, GBs per bibliography sync). Runs on its
/// own thread and connection in bounded batches, so startup never waits for
/// it; one summary line goes to the app log.
fn spawn_checkpoint_cleanup(db_path: std::path::PathBuf) {
    let spawned = std::thread::Builder::new()
        .name("entropia-checkpoint-cleanup".to_string())
        .spawn(move || {
            match run_checkpoint_cleanup(&db_path) {
                Ok(line) => {
                    if let Some(line) = line {
                        eprintln!("{line}");
                    }
                }
                Err(error) => eprintln!("[processing] checkpoint cleanup skipped: {error}"),
            }
            // Same background thread: build the bibliography vector index now,
            // so the first passage search of the session does not pay for it.
            warm_bibliography_vector_index(&db_path);
        });
    if let Err(error) = spawned {
        eprintln!("[processing] checkpoint cleanup not started: {error}");
    }
}

/// Builds the in-memory vector index of the active bibliography generation, if
/// there is one. Best effort: a failure only means the first search builds it.
fn warm_bibliography_vector_index(db_path: &Path) {
    let Ok(conn) = open_archive_connection(db_path) else {
        return;
    };
    let generation: Option<String> = conn
        .query_row(
            "SELECT id FROM bibliographic_index_generations
             WHERE status = 'active' ORDER BY activated_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    if let Some(generation_id) = generation {
        if let Err(error) = crate::bibliography::vector_index::warm(&conn, &generation_id) {
            eprintln!(
                "[bibliography] vector index warm-up skipped: {}: {}",
                error.code, error.message
            );
        }
    }
}

/// One cleanup pass: purge finished tasks' checkpoints, then hand free pages
/// back in a bounded step. The second part runs even when nothing was purged,
/// so the space an archive freed earlier keeps returning over successive starts
/// once the close-time compaction made it `auto_vacuum = INCREMENTAL`.
/// `None` when there was nothing to do.
pub fn run_checkpoint_cleanup(db_path: &Path) -> Result<Option<String>, String> {
    let conn = open_archive_connection(db_path)?;
    let purge =
        repository::purge_terminal_checkpoints(&conn, CLEANUP_TASKS_PER_BATCH, CLEANUP_PAUSE)?;
    let reclaimed = repository::reclaim_free_pages(
        &conn,
        crate::db::compact::RECLAIM_MAX_PAGES,
        crate::db::compact::RECLAIM_PAUSE,
    )?;
    let released = reclaimed.unwrap_or(0);
    if purge.rows == 0 && released == 0 {
        return Ok(None);
    }
    let space = match reclaimed {
        Some(pages) => format!("returned {pages} pages to the OS"),
        None => "kept as reusable free pages (auto_vacuum is off; the close-time compaction will shrink the file)"
            .to_string(),
    };
    Ok(Some(format!(
        "[processing] checkpoint cleanup: deleted {} checkpoints ({} bytes) of finished tasks in {} batches; {space}",
        purge.rows, purge.bytes, purge.batches
    )))
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
/// fresh; a timestamp never authorizes takeover from a live process. Matching
/// bibliography reconciliation rows converge in this same transaction.
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
        crate::bibliography::reconciliation::interrupt_processing_runs_in_transaction(conn)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
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
        // The 0038 trigger settles dependents as their dependency ends; this
        // one scan repairs units a pre-0038 build left blocked.
        repository::settle_blocked_dependents(conn)?;
        // A unit parked for a missing embedding configuration that is valid
        // now (the key was saved while no scheduler watched) resumes at start.
        repository::resume_embedding_configuration_blocked(conn)?;
        // Work only system batches own has nobody to resume it: requeue it.
        repository::requeue_interrupted_system_tasks(conn, None)?;
        // A generation whose vectors all landed but that no build ever
        // activated (or whose last owed work was deleted) becomes queryable
        // now; a partial one is left alone. Best effort: it never blocks
        // recovery, and the next publish or sync retries it.
        if let Err(error) = crate::bibliography::generation::activate_complete_staging_generations(
            conn,
            repository::now_ms(),
        ) {
            eprintln!(
                "[recovery] generation repair skipped: {}: {}",
                error.code, error.message
            );
        }
        // Batches: running work waits for resume; confirmed intents converge.
        // `user` only. A `repair`/`manual`/`bibliography` batch is a long-lived
        // container that is always running with an empty complete snapshot —
        // parking one has no human to resume it, `ensure_system_batch` only
        // reopens from a terminal state, and `maybe_finalize_batch` ignores
        // non-user origins. It would stay interrupted forever, and every
        // automatic repair or bibliography sync admitted into it would sit in
        // a batch the scheduler cannot claim.
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
             WHERE origin IN ('manual', 'repair', 'bibliography') AND state != 'running'",
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
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0038_processing_settle_on_terminal.sql"
        ))
        .expect("apply 0038");
        for (migration, name) in [
            (
                include_str!(
                    "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
                ),
                "0040_bibliography_catalog",
            ),
            (
                include_str!(
                    "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
                ),
                "0041_bibliography_relations",
            ),
            (
                include_str!(
                    "../../../../../packages/store/src/migrations/0042_bibliography_reconciliation.sql"
                ),
                "0042_bibliography_reconciliation",
            ),
        ] {
            conn.execute_batch(migration).expect("apply bibliography migration");
            conn.execute(
                "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
                [name],
            )
            .expect("track bibliography migration");
        }
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0043_processing_task_subject_identity.sql"
        ))
        .expect("apply 0043");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0043_processing_task_subject_identity', 1)",
            [],
        )
        .expect("track 0043");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0044_processing_task_subject_cutover.sql"
        ))
        .expect("apply 0044");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0044_processing_task_subject_cutover', 1)",
            [],
        )
        .expect("track 0044");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0045_bibliography_sync_tasks.sql"
        ))
        .expect("apply 0045");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0045_bibliography_sync_tasks', 1)",
            [],
        )
        .expect("track 0045");
        // A manual sync demand links into the interactive lane, which names
        // the batch priority column.
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0046_processing_priority.sql"
        ))
        .expect("apply 0046");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0046_processing_priority', 1)",
            [],
        )
        .expect("track 0046");
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
                "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, owner_session, lease_epoch, lease_expires_at, created_at, updated_at)
                 VALUES (?1, 'ocr', ?2, 'corpus', 'asset', ?2, ?3, ?4, 3, ?5, 1, 1)",
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
    fn recovery_converges_interrupted_bibliography_without_losing_pages_or_changing_corpus_rules() {
        let (_dir, conn) = recovery_db();
        conn.execute(
            "INSERT INTO zotero_connections
               (id, source_origin, capabilities_json, state, revision, created_at, updated_at)
             VALUES ('conn-1', 'local', '{}', 'available', 0, 1, 1)",
            [],
        )
        .expect("connection");
        for (library_id, external_id) in [("lib-running", "0"), ("lib-complete", "1")] {
            conn.execute(
                "INSERT INTO zotero_libraries
                   (id, connection_id, library_type, library_id, name,
                    last_modified_version, created_at, updated_at)
                 VALUES (?1, 'conn-1', 'user', ?2, ?2, 7, 1, 1)",
                rusqlite::params![library_id, external_id],
            )
            .expect("library");
        }
        conn.execute(
            "INSERT INTO zotero_reconciliation_runs
               (library_id, run_id, connection_revision, state, phase,
                cursor_start, cursor_limit, remote_total, target_version,
                retry_count, attempt_count, revision, checkpointed_at,
                created_at, updated_at)
             VALUES ('lib-running', 'run-live', 0, 'running', 'versions',
                     2, 2, 4, 99, 0, 1, 1, 10, 1, 10)",
            [],
        )
        .expect("running reconciliation");
        conn.execute(
            "INSERT INTO zotero_reconciliation_seen
               (library_id, run_id, entity_kind, entity_key, parent_key,
                remote_version, observed_at)
             VALUES ('lib-running', 'run-live', 'item', 'AAAA1111', '', 12, 10)",
            [],
        )
        .expect("committed seen item");
        conn.execute(
            "INSERT INTO zotero_reconciliation_runs
               (library_id, run_id, connection_revision, state, phase,
                cursor_start, cursor_limit, remote_total, target_version,
                checkpoint_version, retry_count, attempt_count, revision,
                checkpointed_at, completed_at, created_at, updated_at)
             VALUES ('lib-complete', 'run-done', 0, 'completed', 'finalize',
                     0, 2, 0, 7, 7, 0, 1, 1, 10, 10, 1, 10)",
            [],
        )
        .expect("completed reconciliation");

        let bibliography_batch =
            repository::ensure_system_batch(&conn, "bibliography").expect("bibliography batch");
        conn.execute(
            "INSERT INTO processing_tasks
               (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
                state, owner_session, lease_epoch, created_at, updated_at)
             VALUES ('bib-task', 'bibliography_sync', 'lib-running', 'bibliography',
                     'library', 'lib-running', 'running', 'dead-bib', 3, 1, 1)",
            [],
        )
        .expect("bibliography task");
        conn.execute(
            "INSERT INTO processing_batch_tasks
               (batch_id, task_id, kind, asset_id_snapshot, domain,
                subject_kind, subject_id, request_state)
             VALUES (?1, 'bib-task', 'bibliography_sync', 'lib-running',
                     'bibliography', 'library', 'lib-running', 'active')",
            [&bibliography_batch],
        )
        .expect("bibliography link");
        conn.execute(
            "INSERT INTO processing_attempts
               (task_id, attempt_number, lease_epoch, started_at, outcome)
             VALUES ('bib-task', 1, 3, 1, 'open')",
            [],
        )
        .expect("bibliography attempt");
        conn.execute(
            r#"INSERT INTO processing_checkpoints
               (task_id, unit_key, input_fingerprint, contract_hash,
                payload, payload_checksum, created_at)
             VALUES ('bib-task', 'page:0', 'fp', 'bibliography_sync/v1',
                     '{"start":0}', 'sum', 1)"#,
            [],
        )
        .expect("bibliography checkpoint");

        conn.execute(
            r#"INSERT INTO processing_batches
               (id, request_id, origin, state, desired_state, operations,
                planning_done, created_at, updated_at)
             VALUES ('corpus-batch', 'corpus-request', 'user', 'running',
                     'run', '["ocr"]', 1, 1, 1)"#,
            [],
        )
        .expect("corpus batch");
        conn.execute(
            "INSERT INTO processing_tasks
               (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
                state, owner_session, lease_epoch, created_at, updated_at)
             VALUES ('corpus-task', 'ocr', 'asset-1', 'corpus', 'asset',
                     'asset-1', 'running', 'dead-corpus', 4, 1, 1)",
            [],
        )
        .expect("corpus task");
        conn.execute(
            "INSERT INTO processing_batch_tasks
               (batch_id, task_id, kind, asset_id_snapshot, domain,
                subject_kind, subject_id, request_state)
             VALUES ('corpus-batch', 'corpus-task', 'ocr', 'asset-1',
                     'corpus', 'asset', 'asset-1', 'active')",
            [],
        )
        .expect("corpus link");
        conn.execute(
            "INSERT INTO processing_attempts
               (task_id, attempt_number, lease_epoch, started_at, outcome)
             VALUES ('corpus-task', 1, 4, 1, 'open')",
            [],
        )
        .expect("corpus attempt");
        conn.execute(
            "INSERT INTO processing_checkpoints
               (task_id, unit_key, input_fingerprint, contract_hash,
                payload, payload_checksum, created_at)
             VALUES ('corpus-task', 'page:1', 'fp', 'ocr/v1', '{}', 'sum', 1)",
            [],
        )
        .expect("corpus checkpoint");

        let summary = recover_session(&conn, "", 100).expect("recover");
        assert_eq!(summary.tasks_interrupted, 2);
        let bibliography_task_state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id='bib-task'",
                [],
                |row| row.get(0),
            )
            .expect("bibliography task state");
        // System-owned work resumes by itself: no human has a button for it.
        assert_eq!(bibliography_task_state, "pending");
        let (run_state, cursor): (String, i64) = conn
            .query_row(
                "SELECT state, cursor_start FROM zotero_reconciliation_runs
                 WHERE library_id='lib-running'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("recovered reconciliation");
        assert_eq!((run_state.as_str(), cursor), ("interrupted", 2));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM zotero_reconciliation_seen
                 WHERE library_id='lib-running' AND run_id='run-live'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("seen rows"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT state FROM zotero_reconciliation_runs
                 WHERE library_id='lib-complete'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("completed reconciliation"),
            "completed"
        );
        assert_eq!(
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id='corpus-task'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("corpus recovery state"),
            "interrupted"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM processing_checkpoints
                 WHERE task_id IN ('bib-task','corpus-task')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("preserved checkpoints"),
            2
        );

        let again = recover_session(&conn, "", 101).expect("idempotent recovery");
        assert_eq!(again.tasks_interrupted, 0);
        assert_eq!(
            conn.query_row(
                "SELECT state FROM zotero_reconciliation_runs
                 WHERE library_id='lib-running'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("stable reconciliation"),
            "interrupted"
        );

        // E2b-4 RED: a fresh manual demand is the explicit resume for the
        // long-lived system batch. It requeues this same physical task while
        // leaving the committed page cursor, seen-set and checkpoint intact.
        conn.execute(
            "UPDATE processing_tasks SET state='interrupted' WHERE id='bib-task'",
            [],
        )
        .expect("park it again as an older build would have left it");
        let resumed = repository::admit_bibliography_sync_demand(&conn, "user", "0")
            .expect("manual demand after recovery");
        assert_eq!(resumed.task_id, "bib-task");
        assert!(!resumed.created);
        assert!(resumed.requeued);
        assert_eq!(
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id='bib-task'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("resumable bibliography task"),
            "pending"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM processing_tasks
                 WHERE domain='bibliography' AND subject_kind='library'
                   AND subject_id='lib-running' AND kind='bibliography_sync'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("one physical bibliography task"),
            1
        );
        let preserved: (String, i64, i64) = conn
            .query_row(
                "SELECT r.state, r.cursor_start,
                        (SELECT COUNT(*) FROM zotero_reconciliation_seen s
                          WHERE s.library_id=r.library_id AND s.run_id=r.run_id)
                   FROM zotero_reconciliation_runs r
                  WHERE r.library_id='lib-running'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("preserved reconciliation progress");
        assert_eq!(preserved, ("interrupted".to_string(), 2, 1));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id='bib-task'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("preserved page checkpoint"),
            1
        );
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

    /// Inserts one unit of `kind` in `state`, linked to `batch_id`.
    fn linked_task(conn: &Connection, id: &str, batch_id: &str, state: &str) {
        conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
               state, owner_session, lease_epoch, created_at, updated_at)
             VALUES (?1, 'ocr', ?1, 'corpus', 'asset', ?1, ?2, 'old-session', 3, 1, 1)",
            rusqlite::params![id, state],
        )
        .expect("task");
        conn.execute(
            "INSERT INTO processing_batch_tasks
               (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
             VALUES (?1, ?2, 'ocr', ?2, 'corpus', 'asset', ?2, 'active')",
            rusqlite::params![batch_id, id],
        )
        .expect("link");
    }

    fn task_state(conn: &Connection, id: &str) -> String {
        conn.query_row(
            "SELECT state FROM processing_tasks WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .expect("task state")
    }

    #[test]
    fn recovery_requeues_work_owned_only_by_system_batches() {
        let (_dir, conn) = recovery_db();
        let system = repository::ensure_system_batch(&conn, "bibliography").expect("system batch");
        conn.execute(
            r#"INSERT INTO processing_batches
               (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
             VALUES ('user-batch', 'user-request', 'user', 'running', 'run', '["ocr"]', 1, 1, 1)"#,
            [],
        )
        .expect("user batch");
        // Running at the crash, and already parked by an earlier restart.
        linked_task(&conn, "sys-running", &system, "running");
        linked_task(&conn, "sys-parked", &system, "interrupted");
        // Shared with a user batch, and purely user-owned: stay parked.
        linked_task(&conn, "shared", &system, "running");
        conn.execute(
            "INSERT INTO processing_batch_tasks
               (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
             VALUES ('user-batch', 'shared', 'ocr', 'shared', 'corpus', 'asset', 'shared', 'active')",
            [],
        )
        .expect("share with user batch");
        linked_task(&conn, "user-only", "user-batch", "running");

        recover_session(&conn, "", 0).expect("recover");

        assert_eq!(task_state(&conn, "sys-running"), "pending");
        assert_eq!(task_state(&conn, "sys-parked"), "pending");
        assert_eq!(task_state(&conn, "shared"), "interrupted");
        assert_eq!(task_state(&conn, "user-only"), "interrupted");
        // The interrupted attempt is closed as interrupted, not as a failure.
        let owner: Option<String> = conn
            .query_row(
                "SELECT owner_session FROM processing_tasks WHERE id = 'sys-running'",
                [],
                |row| row.get(0),
            )
            .expect("owner");
        assert!(owner.is_none());
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

    // E2a-3 documentary lock-in: a restart mid-running with mixed
    // pending/pausing/cancelling documentary batches reproduces today's
    // `recover_session` outcomes exactly. No production change here — this
    // test pins the behavior the domain gates must preserve.
    #[test]
    fn e2a3_lockin_restart_with_mixed_batches_converges_exactly() {
        let (_dir, conn) = recovery_db();
        // Two mid-flight corpus units with confirmed checkpoints/attempts.
        // Distinct subjects: the composite single-flight unique forbids two
        // live rows for one (domain, subject_kind, subject_id, kind).
        for (id, subject, epoch) in [("t-run-1", "a1", 3), ("t-run-2", "a2", 5)] {
            conn.execute(
                "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
                   state, owner_session, lease_epoch, created_at, updated_at)
                 VALUES (?1, 'ocr', ?2, 'corpus', 'asset', ?2, 'running', 'old-session', ?3, 1, 1)",
                rusqlite::params![id, subject, epoch],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO processing_attempts (task_id, attempt_number, lease_epoch, started_at, outcome)
                 VALUES (?1, 1, ?2, 900, 'open')",
                rusqlite::params![id, epoch],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO processing_checkpoints (task_id, unit_key, input_fingerprint, contract_hash, payload, created_at)
             VALUES ('t-run-1', 'page:1', 'fp', 'ch', '{}', 16)",
            [],
        )
        .unwrap();
        // One pending unit owned only by the cancelling batch (orphan after
        // its links flip, so recovery cancels it).
        conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
               state, created_at, updated_at)
             VALUES ('t-cancel-pending', 'ocr', 'a9', 'corpus', 'asset', 'a9', 'pending', 1, 1)",
            [],
        )
        .unwrap();
        // Documentary user batches in every live state.
        for (id, state, desired) in [
            ("b-run", "running", "run"),
            ("b-ready", "ready", "run"),
            ("b-pausing", "pausing", "pause"),
            ("b-cancelling", "cancelling", "cancel"),
        ] {
            conn.execute(
                "INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
                 VALUES (?1, ?2, 'user', ?3, ?4, '[\"ocr\"]', 1, 1, 1)",
                rusqlite::params![id, format!("req-{id}"), state, desired],
            )
            .unwrap();
        }
        // Links: running units stay wanted by the running batch; the
        // cancelling batch owns only its orphan.
        for (task, subject) in [("t-run-1", "a1"), ("t-run-2", "a2")] {
            conn.execute(
                "INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot,
                   domain, subject_kind, subject_id, request_state)
                 VALUES ('b-run', ?1, 'ocr', ?2, 'corpus', 'asset', ?2, 'active')",
                rusqlite::params![task, subject],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot,
               domain, subject_kind, subject_id, request_state)
             VALUES ('b-cancelling', 't-cancel-pending', 'ocr', 'a9', 'corpus', 'asset', 'a9', 'active')",
            [],
        )
        .unwrap();
        let manual = repository::ensure_system_batch(&conn, "manual").unwrap();
        let repair = repository::ensure_system_batch(&conn, "repair").unwrap();

        let summary = recover_session(&conn, "", 60_000).unwrap();
        assert!(!summary.peer_alive);
        assert_eq!(summary.tasks_interrupted, 2);
        assert_eq!(summary.attempts_closed, 2);
        assert_eq!(summary.batches_interrupted, 2);
        assert_eq!(summary.batches_paused, 1);
        assert_eq!(summary.cancellations_finished, 1);
        assert_eq!(summary.tasks_cancelled, 1);
        // Running units park interrupted with their fencing epoch bumped;
        // confirmed checkpoints survive.
        for id in ["t-run-1", "t-run-2"] {
            let (state, owner): (String, Option<String>) = conn
                .query_row(
                    "SELECT state, owner_session FROM processing_tasks WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(state.as_str(), "interrupted");
            assert!(owner.is_none());
        }
        let checkpoints: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id = 't-run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(checkpoints, 1);
        let open: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_attempts WHERE outcome = 'open'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(open, 0);
        // User batches: running/ready wait for resume under pause; pausing
        // is observed paused; cancelling converges to cancelled.
        let states: Vec<(String, String, String)> = conn
            .prepare("SELECT id, state, desired_state FROM processing_batches WHERE origin = 'user' ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            states,
            vec![
                (
                    "b-cancelling".to_string(),
                    "cancelled".to_string(),
                    "cancel".to_string()
                ),
                (
                    "b-pausing".to_string(),
                    "paused".to_string(),
                    "pause".to_string()
                ),
                (
                    "b-ready".to_string(),
                    "interrupted".to_string(),
                    "pause".to_string()
                ),
                (
                    "b-run".to_string(),
                    "interrupted".to_string(),
                    "pause".to_string()
                ),
            ]
        );
        assert_eq!(
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id = 't-cancel-pending'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap()
            .as_str(),
            "cancelled"
        );
        // System containers are forced back to running/run.
        for id in [&manual, &repair] {
            let (state, desired): (String, String) = conn
                .query_row(
                    "SELECT state, desired_state FROM processing_batches WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!((state.as_str(), desired.as_str()), ("running", "run"));
        }
        // Idempotent: a second pass converges to nothing.
        let again = recover_session(&conn, "", 60_000).unwrap();
        assert_eq!(again.tasks_interrupted, 0);
        assert_eq!(again.cancellations_finished, 0);
    }

    #[test]
    fn startup_cleanup_returns_free_pages_gradually_even_with_nothing_to_purge() {
        let (dir, conn) = recovery_db();
        conn.execute_batch("CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload BLOB)")
            .unwrap();
        for _ in 0..256 {
            conn.execute("INSERT INTO blobs (payload) VALUES (zeroblob(65536))", [])
                .unwrap();
        }
        conn.execute("DELETE FROM blobs", []).unwrap();
        drop(conn);
        let db_path = dir.path().join("entropia.sqlite");
        let free = |path: &Path| -> i64 {
            open_archive_connection(path)
                .unwrap()
                .query_row("PRAGMA freelist_count", [], |row| row.get(0))
                .unwrap()
        };
        let free_before = free(&db_path);
        assert!(free_before > 1_000);

        // auto_vacuum NONE: nothing to purge and nothing is vacuumed.
        assert_eq!(run_checkpoint_cleanup(&db_path).unwrap(), None);
        assert_eq!(free(&db_path), free_before);

        // After the close-time compaction the same startup hook reclaims.
        let policy = crate::db::compact::Policy {
            min_free_bytes: 256 * 1024,
            disk_margin_bytes: 0,
            ..Default::default()
        };
        crate::db::compact::compact_archive(&db_path, &policy, &|_| Some(u64::MAX / 2)).unwrap();
        // Free pages again, after the compaction.
        let conn = open_archive_connection(&db_path).unwrap();
        for _ in 0..64 {
            conn.execute("INSERT INTO blobs (payload) VALUES (zeroblob(65536))", [])
                .unwrap();
        }
        conn.execute("DELETE FROM blobs", []).unwrap();
        drop(conn);
        assert!(free(&db_path) > 0);

        let line = run_checkpoint_cleanup(&db_path)
            .unwrap()
            .expect("free pages were returned without any checkpoint to purge");
        assert!(line.contains("returned"), "{line}");
        assert_eq!(free(&db_path), 0);
    }

    #[test]
    fn startup_recovery_resumes_units_parked_for_a_configuration_that_is_valid_now() {
        let (_dir, conn) = recovery_db();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO app_settings (key, value) VALUES ('openrouter_api_key', 'sk-test');
             INSERT INTO processing_tasks
               (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, outcome,
                last_error_code, created_at, updated_at)
             VALUES ('t-config', 'embedding', 'a1', 'corpus', 'asset', 'a1', 'blocked',
                     'configuration_required', 'configuration_required', 1, 1);",
        )
        .expect("blocked unit with a valid configuration");
        recover_session(&conn, "", 60_000).expect("recover");
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 't-config'",
                [],
                |row| row.get(0),
            )
            .expect("state");
        assert_eq!(state, "pending");
    }
}
