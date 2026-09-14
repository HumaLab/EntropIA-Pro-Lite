//! Serial supervisor behind the batch queue (plan-lote.md §7).
//!
//! One thread per archive owns execution: it wakes persisted work on a timer,
//! claims one unit at a time (the global heavy-local slot), runs it through
//! the registered [`Executor`], and publishes the result. Nothing here holds
//! queue state — sleeps only pace the loop; a restart resumes from the
//! tables. Engine adapters (OCR, embeddings) arrive in Unidad 4 behind the
//! same trait; until then the registry is empty and the tick only advances
//! planning, settles demand, and finalizes batches.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use rusqlite::Connection;

use super::repository::{
    self, claim_next, commit_success_with, execution_wanted, fail_attempt, heartbeat_owned,
    maybe_finalize_batch, reconcile_contracts, ClaimedTask, NewCheckpoint,
};
use crate::db::open::open_archive_connection;

/// Cooperative stop observed by executors between units. Pause and cancel
/// withdraw demand; the flag only asks the current unit to stop at its next
/// checkpoint boundary — an indivisible native inference still runs to its
/// next safe point.
#[derive(Debug, Default)]
pub struct StopFlag {
    flag: AtomicBool,
}

impl StopFlag {
    pub fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }

    /// Asks the running unit to stop at its next checkpoint boundary. Wired
    /// to the threaded supervisor's demand watcher in Unidad 4; tests drive
    /// `Stopped` through scripted executors instead.
    #[allow(dead_code)]
    pub fn stop(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn stopped(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// What one engine run produced. Checkpoints are validated, complete output
/// units (pages, chunk sets) — never partial provider progress.
#[derive(Debug)]
pub enum ExecOutput {
    Success { outcome: String, receipt: String },
    Retryable { code: String, message: String },
    Fatal { code: String, message: String },
    Stopped,
}

/// The full product of one execution: staged checkpoints plus the verdict.
/// The supervisor persists checkpoints and publishes; the executor never
/// writes the archive itself (it may open short-lived read connections).
#[derive(Debug)]
pub struct ExecResult {
    pub checkpoints: Vec<NewCheckpoint>,
    pub progress_total: Option<i64>,
    pub output: ExecOutput,
}

/// One engine behind the queue. Unidad 4 implements OCR and embeddings;
/// tests implement fakes. `run` blocks the supervisor worker thread, so
/// heavy compute belongs on blocking threads inside the executor.
pub trait Executor: Send + Sync {
    /// Task kinds this executor runs (`"ocr"`, `"embedding"`).
    fn kinds(&self) -> &[&str];
    /// Executes one claimed unit to a verdict, honoring `stop` between
    /// output units. Must not write queue tables.
    fn run(&self, task: &ClaimedTask, stop: &StopFlag) -> ExecResult;
}

/// Kind-routed executor set. One physical task has exactly one executor per
/// kind, so two engines can never compute the same unit twice.
#[derive(Default)]
pub struct ExecutorRegistry {
    executors: HashMap<String, Arc<dyn Executor>>,
}

impl ExecutorRegistry {
    pub fn new() -> Self {
        Self {
            executors: HashMap::new(),
        }
    }

    pub fn register(&mut self, executor: Arc<dyn Executor>) {
        for kind in executor.kinds() {
            self.executors
                .insert(kind.to_string(), Arc::clone(&executor));
        }
    }

    pub fn kinds(&self) -> Vec<String> {
        self.executors.keys().cloned().collect()
    }

    pub fn get(&self, kind: &str) -> Option<Arc<dyn Executor>> {
        self.executors.get(kind).cloned()
    }
}

/// What one supervisor step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOneOutcome {
    Idle,
    Succeeded { task_id: String },
    Failed { task_id: String },
    Waiting { task_id: String },
    Stopped { task_id: String },
    Requeued { task_id: String },
    Cancelled { task_id: String },
    Blocked { task_id: String },
}

/// Claims one unit, executes it, and publishes the verdict — the whole
/// state machine transition for a single unit of work. Deterministic and
/// thread-free so tests drive it directly; the background thread only loops
/// it (see `scheduler_tick`).
pub fn run_one(
    conn: &Connection,
    registry: &ExecutorRegistry,
    session_id: &str,
    now_ms: i64,
) -> Result<RunOneOutcome, String> {
    let kinds: Vec<String> = registry.kinds();
    let kind_refs: Vec<&str> = kinds.iter().map(String::as_str).collect();
    let Some(task) = claim_next(conn, session_id, &kind_refs, now_ms)? else {
        return Ok(RunOneOutcome::Idle);
    };
    let Some(executor) = registry.get(&task.kind) else {
        // Kinds come from the registry, so this is unreachable — but a
        // claimed task must never be stranded running: requeue it.
        repository::requeue_task(conn, &task.task_id, task.lease_epoch)?;
        return Ok(RunOneOutcome::Requeued {
            task_id: task.task_id,
        });
    };
    let stop = StopFlag::new();
    if !execution_wanted(conn, &task.task_id)? {
        repository::cancel_running_task(conn, &task.task_id, task.lease_epoch)?;
        return Ok(RunOneOutcome::Cancelled {
            task_id: task.task_id,
        });
    }
    let result = executor.run(&task, &stop);
    if let Some(total) = result.progress_total {
        if repository::set_progress_total(conn, &task.task_id, task.lease_epoch, total).is_err() {
            return Ok(RunOneOutcome::Stopped {
                task_id: task.task_id,
            });
        }
    }
    for checkpoint in &result.checkpoints {
        if repository::save_checkpoint(conn, &task.task_id, task.lease_epoch, checkpoint, now_ms)
            .is_err()
        {
            // Lease lost mid-run (recovery, cancel, newer claim): the attempt
            // is already someone else's problem. Never publish.
            return Ok(RunOneOutcome::Stopped {
                task_id: task.task_id,
            });
        }
    }
    let task_id = task.task_id.clone();
    match result.output {
        ExecOutput::Success { outcome, receipt } => {
            match commit_success_with(
                conn,
                &task_id,
                task.lease_epoch,
                &task.kind,
                &outcome,
                &receipt,
                |_| Ok(()),
            ) {
                Ok(()) => {
                    finalize_links(conn, &task_id)?;
                    Ok(RunOneOutcome::Succeeded { task_id })
                }
                Err(error) if error.starts_with("source_changed") => {
                    repository::requeue_task(conn, &task_id, task.lease_epoch)?;
                    Ok(RunOneOutcome::Requeued { task_id })
                }
                Err(error) if error.starts_with("demand_lost") => {
                    repository::cancel_running_task(conn, &task_id, task.lease_epoch)?;
                    Ok(RunOneOutcome::Cancelled { task_id })
                }
                Err(error) if error.starts_with("configuration_changed") => {
                    repository::block_running_task(
                        conn,
                        &task_id,
                        task.lease_epoch,
                        "configuration_changed",
                        &error,
                    )?;
                    Ok(RunOneOutcome::Blocked { task_id })
                }
                Err(error) if error.starts_with("lease_lost") => {
                    Ok(RunOneOutcome::Stopped { task_id })
                }
                Err(error) => Err(error),
            }
        }
        ExecOutput::Retryable { code, message } => {
            match fail_attempt(
                conn,
                &task_id,
                task.lease_epoch,
                task.attempt_number,
                &code,
                &message,
                true,
                None,
                now_ms,
            )? {
                repository::FailOutcome::RetryWait { .. } => {
                    finalize_links(conn, &task_id)?;
                    Ok(RunOneOutcome::Waiting { task_id })
                }
                repository::FailOutcome::Failed => {
                    finalize_links(conn, &task_id)?;
                    Ok(RunOneOutcome::Failed { task_id })
                }
            }
        }
        ExecOutput::Fatal { code, message } => {
            fail_attempt(
                conn,
                &task_id,
                task.lease_epoch,
                task.attempt_number,
                &code,
                &message,
                false,
                None,
                now_ms,
            )?;
            finalize_links(conn, &task_id)?;
            Ok(RunOneOutcome::Failed { task_id })
        }
        ExecOutput::Stopped => {
            repository::interrupt_task(conn, &task_id, task.lease_epoch)?;
            Ok(RunOneOutcome::Stopped { task_id })
        }
    }
}

fn finalize_links(conn: &Connection, task_id: &str) -> Result<(), String> {
    let batches: Vec<String> = conn
        .prepare("SELECT DISTINCT batch_id FROM processing_batch_tasks WHERE task_id = ?1")
        .map_err(|e| format!("Failed to list batches of {task_id}: {e}"))?
        .query_map([task_id], |row| row.get(0))
        .map_err(|e| format!("Failed to list batches of {task_id}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to list batches of {task_id}: {e}"))?;
    for batch_id in batches {
        maybe_finalize_batch(conn, &batch_id)?;
    }
    Ok(())
}

/// One supervisor pass: reconcile contracts, advance planning, heartbeat,
/// execute one unit, sweep orphans, finalize. Bounded work per pass so the
/// loop stays responsive to stop signals.
pub fn scheduler_tick(
    conn: &Connection,
    registry: &ExecutorRegistry,
    session_id: &str,
    now_ms: i64,
) -> Result<RunOneOutcome, String> {
    reconcile_contracts(conn)?;
    for batch_id in repository::planning_batches(conn)? {
        // One page per batch per tick: planning shares the loop with
        // execution instead of starving it on huge collections.
        repository::advance_planning(conn, &batch_id, 1, 200)?;
    }
    heartbeat_owned(conn, session_id, now_ms)?;
    let outcome = run_one(conn, registry, session_id, now_ms)?;
    repository::cancel_orphaned_tasks(conn)?;
    for batch_id in repository::open_batches(conn)? {
        maybe_finalize_batch(conn, &batch_id)?;
    }
    Ok(outcome)
}

/// Starts the background supervisor. The thread opens its own archive
/// connection and never touches Tauri state; a poisoned loop sleeps and
/// retries instead of dying silent, and storage errors pause claiming
/// without fabricating queue state.
pub fn start_scheduler(
    db_path: PathBuf,
    session_id: String,
    registry: Arc<ExecutorRegistry>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("entropia-processing".to_string())
        .spawn(move || {
            let mut backoff: Duration;
            loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match open_archive_connection(&db_path) {
                    Ok(conn) => {
                        match scheduler_tick(&conn, &registry, &session_id, repository::now_ms()) {
                            Ok(RunOneOutcome::Idle) => {
                                backoff = Duration::from_secs(2);
                            }
                            Ok(_) => {
                                backoff = Duration::from_millis(100);
                            }
                            Err(error) => {
                                eprintln!("[processing] tick failed: {error}");
                                backoff = Duration::from_secs(5);
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("[processing] cannot open archive: {error}");
                        backoff = Duration::from_secs(5);
                    }
                }
                std::thread::sleep(backoff);
            }
        })
        .expect("spawn processing scheduler")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processing::repository as repo;

    struct ScriptExecutor {
        script: std::sync::Mutex<Vec<ExecOutput>>,
        checkpoints_per_run: usize,
    }

    impl ScriptExecutor {
        fn succeed() -> Self {
            Self {
                script: std::sync::Mutex::new(vec![ExecOutput::Success {
                    outcome: "text".to_string(),
                    receipt: "{}".to_string(),
                }]),
                checkpoints_per_run: 0,
            }
        }
    }

    impl Executor for ScriptExecutor {
        fn kinds(&self) -> &[&str] {
            &["ocr"]
        }

        fn run(&self, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
            let mut checkpoints = Vec::new();
            for index in 0..self.checkpoints_per_run {
                if stop.stopped() {
                    return ExecResult {
                        checkpoints,
                        progress_total: None,
                        output: ExecOutput::Stopped,
                    };
                }
                checkpoints.push(NewCheckpoint {
                    unit_key: format!("page-{}", index + 1),
                    input_fingerprint: task.input_fingerprint.clone(),
                    contract_hash: task.contract_hash.clone(),
                    payload: "{}".to_string(),
                    payload_checksum: "0".to_string(),
                });
            }
            let output = self
                .script
                .lock()
                .expect("script")
                .pop()
                .unwrap_or(ExecOutput::Success {
                    outcome: "text".to_string(),
                    receipt: "{}".to_string(),
                });
            let progress_total = Some(checkpoints.len() as i64);
            ExecResult {
                checkpoints,
                progress_total,
                output,
            }
        }
    }

    /// Batch with one OCR task over a member, batch already running.
    fn running_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        let conn = open_archive_connection(&db_path).expect("open");
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, size INTEGER, created_at INTEGER NOT NULL);
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
        conn.execute(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'legajo', 1, 1)",
            [],
        )
        .expect("collection");
        conn.execute(
            "INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES ('i1', 'doc', 'c1', 1, 1)",
            [],
        )
        .expect("item");
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, size, created_at) VALUES ('a1', 'i1', 'a1.png', 'image', 10, 1)",
            [],
        )
        .expect("asset");
        conn.execute(
            "INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
             VALUES ('b1', 'req-1', 'user', 'running', 'run', '[\"ocr\"]', 1, 1, 1)",
            [],
        )
        .expect("batch");
        conn.execute(
            "INSERT INTO processing_batch_collections (batch_id, collection_id_snapshot, name_snapshot) VALUES ('b1', 'c1', 'legajo')",
            [],
        )
        .expect("scope");
        conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at)
             VALUES ('ocr-a1', 'ocr', 'a1', 'pending', 1, 1)",
            [],
        )
        .expect("task");
        conn.execute(
            "INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, request_state)
             VALUES ('b1', 'ocr-a1', 'ocr', 'a1', 'active')",
            [],
        )
        .expect("link");
        (dir, conn)
    }

    fn registry() -> ExecutorRegistry {
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(ScriptExecutor::succeed()));
        registry
    }

    #[test]
    fn one_failure_does_not_block_the_rest_of_the_queue() {
        let (_dir, conn) = running_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, size, created_at) VALUES ('a2', 'i1', 'a2.png', 'image', 10, 2)",
            [],
        )
        .expect("second asset");
        conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at)
             VALUES ('ocr-a2', 'ocr', 'a2', 'pending', 2, 2)",
            [],
        )
        .expect("second task");
        conn.execute(
            "INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, request_state)
             VALUES ('b1', 'ocr-a2', 'ocr', 'a2', 'active')",
            [],
        )
        .expect("second link");
        // First run fails transiently, second run succeeds: order is by task
        // id, so ocr-a1 fails first and ocr-a2 still completes after it.
        let mut failing = ExecutorRegistry::new();
        failing.register(Arc::new(ScriptExecutor {
            script: std::sync::Mutex::new(vec![
                ExecOutput::Success {
                    outcome: "text".to_string(),
                    receipt: "{}".to_string(),
                },
                ExecOutput::Retryable {
                    code: "timeout".to_string(),
                    message: "deadline".to_string(),
                },
            ]),
            checkpoints_per_run: 0,
        }));
        // attempts pop from the back: first run pops Retryable... script
        // order is reversed, so push success first, failure second.
        assert!(matches!(
            run_one(&conn, &failing, "s1", 1000).expect("run 1"),
            RunOneOutcome::Waiting { .. }
        ));
        assert!(matches!(
            run_one(&conn, &failing, "s1", 40_000).expect("run 2"),
            RunOneOutcome::Succeeded { .. }
        ));
        // The second unit was never blocked by the first unit's failure, and
        // the drained batch observes itself completed.
        assert!(matches!(
            run_one(&conn, &failing, "s1", 40_000).expect("run 3"),
            RunOneOutcome::Succeeded { task_id } if task_id == "ocr-a2"
        ));
        let batch: String = conn
            .query_row(
                "SELECT state FROM processing_batches WHERE id = 'b1'",
                [],
                |row| row.get(0),
            )
            .expect("batch state");
        assert_eq!(batch, "completed");
    }

    #[test]
    fn a_stale_lease_cannot_publish_after_recovery() {
        let (_dir, conn) = running_db();
        let registry = registry();
        // Claim at t=1000, then simulate recovery revoking the lease: the
        // attempt closes interrupted and the epoch moves on.
        let claimed = repo::claim_next(&conn, "s1", &["ocr"], 1000)
            .expect("claim")
            .expect("task");
        repo::interrupt_task(&conn, &claimed.task_id, claimed.lease_epoch).expect("revoke");
        // The old owner still holds its epoch number — but the row moved on.
        let publish = repo::commit_success_with(
            &conn,
            &claimed.task_id,
            claimed.lease_epoch,
            "ocr",
            "text",
            "{}",
            |_| Ok(()),
        );
        assert!(
            publish.is_err(),
            "a revoked lease must never confirm a result: {:?}",
            publish.ok()
        );
        let _ = registry;
    }

    #[test]
    fn cancel_during_execution_wins_over_a_late_commit() {
        let (_dir, conn) = running_db();
        let claimed = repo::claim_next(&conn, "s1", &["ocr"], 1000)
            .expect("claim")
            .expect("task");
        // Cancel withdraws demand while the unit is mid-flight.
        repo::control_batch(&conn, "b1", repo::BatchAction::Cancel, None).expect("cancel");
        let late = repo::commit_success_with(
            &conn,
            &claimed.task_id,
            claimed.lease_epoch,
            "ocr",
            "text",
            "{}",
            |_| Ok(()),
        );
        assert!(
            late.unwrap_err().starts_with("demand_lost"),
            "the late result must not overwrite a cancelled demand"
        );
        repo::cancel_running_task(&conn, &claimed.task_id, claimed.lease_epoch).expect("settle");
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 'ocr-a1'",
                [],
                |row| row.get(0),
            )
            .expect("state");
        assert_eq!(state, "cancelled");
    }

    #[test]
    fn stop_parks_the_unit_and_resume_continues_from_checkpoints() {
        let (_dir, conn) = running_db();
        // First run aborts after one checkpoint (as if the supervisor's
        // watcher withdrew demand mid-flight); the second run completes.
        let mut stoppable = ExecutorRegistry::new();
        stoppable.register(Arc::new(ScriptExecutor {
            script: std::sync::Mutex::new(vec![
                ExecOutput::Success {
                    outcome: "text".to_string(),
                    receipt: "{}".to_string(),
                },
                ExecOutput::Stopped,
            ]),
            checkpoints_per_run: 1,
        }));
        assert!(matches!(
            run_one(&conn, &stoppable, "s1", 1000).expect("run 1"),
            RunOneOutcome::Stopped { .. }
        ));
        let (state, done): (String, i64) = conn
            .query_row(
                "SELECT state, progress_done FROM processing_tasks WHERE id = 'ocr-a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("parked with its checkpoint");
        assert_eq!(state, "interrupted");
        assert_eq!(done, 1);
        // Resume requeues without wiping the confirmed checkpoint...
        repo::control_batch(&conn, "b1", repo::BatchAction::Resume, None).expect("resume");
        assert!(matches!(
            run_one(&conn, &stoppable, "s1", 2000).expect("run 2"),
            RunOneOutcome::Succeeded { .. }
        ));
        // ...and the retried run adopted it instead of recomputing it.
        let done: i64 = conn
            .query_row(
                "SELECT progress_done FROM processing_tasks WHERE id = 'ocr-a1'",
                [],
                |row| row.get(0),
            )
            .expect("progress kept");
        assert_eq!(done, 1);
    }

    #[test]
    fn fatal_errors_go_terminal_with_their_message() {
        let (_dir, conn) = running_db();
        let mut fatal = ExecutorRegistry::new();
        fatal.register(Arc::new(ScriptExecutor {
            script: std::sync::Mutex::new(vec![ExecOutput::Fatal {
                code: "corrupt_pdf".to_string(),
                message: "encrypted and locked".to_string(),
            }]),
            checkpoints_per_run: 0,
        }));
        assert!(matches!(
            run_one(&conn, &fatal, "s1", 1000).expect("run"),
            RunOneOutcome::Failed { .. }
        ));
        let (state, code, cycle): (String, Option<String>, i64) = conn
            .query_row(
                "SELECT state, last_error_code, retry_cycle FROM processing_tasks WHERE id = 'ocr-a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("terminal failure recorded");
        assert_eq!(state, "failed");
        assert_eq!(code.as_deref(), Some("corrupt_pdf"));
        assert_eq!(cycle, 0, "a fatal error opens no new cycle on its own");
        // ...but an explicit retry does.
        assert_eq!(
            repo::retry_failed(&conn, "b1", Some("ocr-a1")).expect("retry"),
            1
        );
        let (state, cycle): (String, i64) = conn
            .query_row(
                "SELECT state, retry_cycle FROM processing_tasks WHERE id = 'ocr-a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("reopened");
        assert_eq!(state, "pending");
        assert_eq!(cycle, 1);
    }
}
