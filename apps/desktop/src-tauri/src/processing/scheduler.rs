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
    maybe_finalize_batch, reconcile_contracts,
};
pub use super::repository::{ClaimedTask, NewCheckpoint};
use crate::db::open::open_archive_connection;

pub(super) static READY: AtomicBool = AtomicBool::new(false);

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
    Success {
        outcome: String,
        receipt: String,
    },
    Retryable {
        code: String,
        message: String,
    },
    Fatal {
        code: String,
        message: String,
    },
    /// Configuration or credential state blocks progress without spending
    /// retries: a human fixes settings, then resumes.
    Blocked {
        code: String,
        message: String,
    },
    Stopped,
}

#[derive(Debug, Clone)]
pub enum EngineOutput {
    Ocr(super::ocr::OcrComputeOutput),
    Embedding(super::embedding::EmbeddingComputeOutput),
}

/// Execution context handed to every engine run: archive location for
/// short-lived read connections (snapshots, resume scans). Engines never
/// receive the supervisor connection and never write queue tables.
#[derive(Debug, Clone)]
pub struct ExecCtx {
    pub db_path: PathBuf,
}

impl ExecCtx {
    pub fn unit<T: serde::Serialize + serde::de::DeserializeOwned>(
        &self,
        task: &ClaimedTask,
        key: &str,
        compute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        use rusqlite::OptionalExtension;
        use sha2::{Digest, Sha256};
        let conn = open_archive_connection(&self.db_path)?;
        let saved: Option<(String, String)> = conn.query_row(
            "SELECT payload,payload_checksum FROM processing_checkpoints WHERE task_id=?1 AND unit_key=?2 AND input_fingerprint=?3 AND contract_hash=?4",
            rusqlite::params![task.task_id,key,task.input_fingerprint,task.contract_hash],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional().map_err(|e| e.to_string())?;
        drop(conn);
        if let Some((payload, checksum)) = saved {
            if format!("{:x}", Sha256::digest(payload.as_bytes())) == checksum {
                if let Ok(value) = serde_json::from_str(&payload) {
                    return Ok(value);
                }
            }
        }
        let value = compute()?;
        let payload = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        let checkpoint = NewCheckpoint {
            unit_key: key.to_string(),
            input_fingerprint: task.input_fingerprint.clone(),
            contract_hash: task.contract_hash.clone(),
            payload_checksum: format!("{:x}", Sha256::digest(payload.as_bytes())),
            payload,
        };
        self.checkpoint(task, &checkpoint)?;
        Ok(value)
    }
    pub fn checkpoint(&self, task: &ClaimedTask, checkpoint: &NewCheckpoint) -> Result<(), String> {
        let conn = open_archive_connection(&self.db_path)?;
        repository::save_checkpoint(
            &conn,
            &task.task_id,
            task.lease_epoch,
            checkpoint,
            repository::now_ms(),
        )
    }

    pub fn progress_total(&self, task: &ClaimedTask, total: i64) -> Result<(), String> {
        let conn = open_archive_connection(&self.db_path)?;
        repository::set_progress_total(&conn, &task.task_id, task.lease_epoch, total)
    }
}

/// The full product of one execution: staged checkpoints plus the verdict.
/// The supervisor persists checkpoints and publishes; the executor never
/// writes the archive itself (it may open short-lived read connections).
#[derive(Debug)]
pub struct ExecResult {
    pub checkpoints: Vec<NewCheckpoint>,
    pub progress_total: Option<i64>,
    pub engine_output: Option<EngineOutput>,
    pub output: ExecOutput,
}

/// One engine behind the queue. OCR and embedding adapters implement this;
/// tests implement fakes. `run` blocks the supervisor worker thread, so
/// heavy compute belongs on blocking threads inside the executor.
pub trait Executor: Send + Sync {
    /// Task kinds this executor runs (`"ocr"`, `"embedding"`).
    fn kinds(&self) -> &[&str];
    /// Executes one claimed unit to a verdict, honoring `stop` between
    /// output units. Must not write queue tables.
    fn run(&self, ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult;
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
    Blocked { task_id: String },
}

/// Claims one unit, executes it, and publishes the verdict — the whole
/// state machine transition for a single unit of work. Deterministic and
/// thread-free so tests drive it directly; the background thread only loops
/// it (see `scheduler_tick`).
///
/// `on_commit` runs after a durable success with the published output still
/// in hand: the single place for post-commit observers (compat events,
/// OCR→embedding follow-ups). `on_terminal` runs after a Failed or Blocked
/// verdict lands durably, with (task, state, code, message) for the legacy
/// error events. Both must be quick and infallible — queue state already
/// committed, so observer failures only log.
pub fn run_one(
    conn: &Connection,
    ctx: &ExecCtx,
    registry: &ExecutorRegistry,
    session_id: &str,
    now_ms: i64,
    on_commit: &dyn Fn(&ClaimedTask, &EngineOutput),
    on_terminal: &dyn Fn(&ClaimedTask, &str, &str, &str),
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
    let stop = std::sync::Arc::new(StopFlag::new());
    if !execution_wanted(conn, &task.task_id)? {
        repository::interrupt_task(conn, &task.task_id, task.lease_epoch)?;
        return Ok(RunOneOutcome::Stopped {
            task_id: task.task_id,
        });
    }
    // Demand watcher: long native inferences cannot be preempted mid-call,
    // but chunk/page loops observe `stop` between units and abort early
    // instead of burning provider budget after a pause or cancel.
    let watcher_done = std::sync::Arc::new(AtomicBool::new(false));
    {
        let db_path = ctx.db_path.clone();
        let session_id = session_id.to_string();
        let task_id = task.task_id.clone();
        let stop = std::sync::Arc::clone(&stop);
        let done = std::sync::Arc::clone(&watcher_done);
        std::thread::Builder::new()
            .name("entropia-processing-watch".to_string())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                if done.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(conn) = open_archive_connection(&db_path) {
                    let _ = heartbeat_owned(&conn, &session_id, repository::now_ms());
                }
                let wanted = open_archive_connection(&db_path)
                    .ok()
                    .and_then(|conn| execution_wanted(&conn, &task_id).ok())
                    .unwrap_or(true);
                if !wanted {
                    stop.stop();
                    return;
                }
            })
            .ok();
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        executor.run(ctx, &task, &stop)
    }));
    watcher_done.store(true, Ordering::SeqCst);
    let result = result.unwrap_or_else(|_| ExecResult {
        checkpoints: Vec::new(),
        progress_total: None,
        engine_output: None,
        output: ExecOutput::Fatal {
            code: "executor_panicked".to_string(),
            message: "Processing engine panicked; confirmed units remain available for retry"
                .to_string(),
        },
    });
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
            let Some(engine_output) = result.engine_output else {
                // An engine that reports success must stage its output: a
                // receipt without canonical rows would lie about durability.
                repository::requeue_task(conn, &task_id, task.lease_epoch)?;
                return Ok(RunOneOutcome::Requeued { task_id });
            };
            let commit = commit_success_with(
                conn,
                &task_id,
                task.lease_epoch,
                &task.kind,
                &outcome,
                &receipt,
                |conn| publish_engine_output(conn, &task, &engine_output),
            );
            match commit {
                Ok(()) => {
                    finalize_links(conn, &task_id)?;
                    on_commit(&task, &engine_output);
                    Ok(RunOneOutcome::Succeeded { task_id })
                }
                Err(error) if error.starts_with("source_changed") => {
                    match repository::record_source_change(
                        conn,
                        &task_id,
                        task.lease_epoch,
                        &error,
                    )? {
                        repository::SourceChangeOutcome::Requeued => {
                            Ok(RunOneOutcome::Requeued { task_id })
                        }
                        repository::SourceChangeOutcome::Blocked => {
                            finalize_links(conn, &task_id)?;
                            on_terminal(&task, "blocked", "source_unstable", &error);
                            Ok(RunOneOutcome::Blocked { task_id })
                        }
                    }
                }
                Err(error) if error.starts_with("demand_lost") => {
                    repository::interrupt_task(conn, &task_id, task.lease_epoch)?;
                    Ok(RunOneOutcome::Stopped { task_id })
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
                    on_terminal(&task, "failed", &code, &message);
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
            on_terminal(&task, "failed", &code, &message);
            Ok(RunOneOutcome::Failed { task_id })
        }
        ExecOutput::Blocked { code, message } => {
            repository::block_running_task(conn, &task_id, task.lease_epoch, &code, &message)?;
            finalize_links(conn, &task_id)?;
            on_terminal(&task, "blocked", &code, &message);
            Ok(RunOneOutcome::Blocked { task_id })
        }
        ExecOutput::Stopped => {
            repository::interrupt_task(conn, &task_id, task.lease_epoch)?;
            Ok(RunOneOutcome::Stopped { task_id })
        }
    }
}

/// Routes staged engine output to its canonical publisher. Runs inside the
/// commit transaction: rows and receipt confirm together or not at all.
fn publish_engine_output(
    conn: &Connection,
    task: &ClaimedTask,
    output: &EngineOutput,
) -> Result<(), String> {
    match output {
        EngineOutput::Ocr(ocr) => super::ocr::publish_ocr_output(conn, &task.asset_id, ocr),
        EngineOutput::Embedding(embedding) => {
            super::embedding::publish_embedding_output(conn, &task.asset_id, embedding)
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
///
/// A live peer short-circuits the pass to a heartbeat-free Idle: the standby
/// instance performs no writes at all, so two processes never compute the
/// same unit or finalize each other's batches. It takes over (planning,
/// claiming, finalizing) once the peer heartbeat goes stale.
pub fn scheduler_tick(
    conn: &Connection,
    ctx: &ExecCtx,
    registry: &ExecutorRegistry,
    session_id: &str,
    now_ms: i64,
    on_commit: &dyn Fn(&ClaimedTask, &EngineOutput),
    on_terminal: &dyn Fn(&ClaimedTask, &str, &str, &str),
) -> Result<RunOneOutcome, String> {
    repository::write_scheduler_heartbeat(conn, session_id, now_ms)?;
    reconcile_contracts(conn)?;
    for batch_id in repository::planning_batches(conn)? {
        // One page per batch per tick: planning shares the loop with
        // execution instead of starving it on huge collections.
        repository::advance_planning(conn, &batch_id, 1, 200)?;
    }
    repository::promote_ready_batches(conn)?;
    heartbeat_owned(conn, session_id, now_ms)?;
    let outcome = run_one(
        conn,
        ctx,
        registry,
        session_id,
        now_ms,
        on_commit,
        on_terminal,
    )?;
    repository::cancel_orphaned_tasks(conn)?;
    for batch_id in repository::open_batches(conn)? {
        maybe_finalize_batch(conn, &batch_id)?;
    }
    Ok(outcome)
}

/// Starts the background supervisor. The thread opens its own archive
/// connection and never touches Tauri state; a poisoned loop sleeps and
/// retries instead of dying silent, and storage errors pause claiming
/// without fabricating queue state. Both observers run on the supervisor
/// thread after queue state committed, and must never block it for long.
pub fn start_scheduler(
    db_path: PathBuf,
    session_id: String,
    registry: Arc<ExecutorRegistry>,
    stop: Arc<AtomicBool>,
    on_commit: Arc<dyn Fn(ClaimedTask, EngineOutput) + Send + Sync>,
    on_terminal: Arc<dyn Fn(ClaimedTask, String, String, String) + Send + Sync>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("entropia-processing".to_string())
        .spawn(move || {
            let ctx = ExecCtx {
                db_path: db_path.clone(),
            };
            let mut backoff: Duration;
            loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if !READY.load(Ordering::Acquire) {
                    match super::recovery::recover_once_if_needed(&db_path) {
                        Ok(Some(summary)) if summary.peer_alive => {}
                        Ok(_) => {}
                        Err(error) => eprintln!("[processing] ownership retry failed: {error}"),
                    }
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                match open_archive_connection(&db_path) {
                    Ok(conn) => {
                        match scheduler_tick(
                            &conn,
                            &ctx,
                            &registry,
                            &session_id,
                            repository::now_ms(),
                            &|task, output| on_commit(task.clone(), output.clone()),
                            &|task, state, code, message| {
                                on_terminal(
                                    task.clone(),
                                    state.to_string(),
                                    code.to_string(),
                                    message.to_string(),
                                )
                            },
                        ) {
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

        fn run(&self, _ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
            let mut checkpoints = Vec::new();
            for index in 0..self.checkpoints_per_run {
                if stop.stopped() {
                    return ExecResult {
                        checkpoints,
                        progress_total: None,
                        engine_output: None,
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
            let engine_output = matches!(output, ExecOutput::Success { .. }).then(test_ocr_output);
            ExecResult {
                checkpoints,
                progress_total,
                engine_output,
                output,
            }
        }
    }

    /// Minimal staged OCR output: the commit path publishes real extraction
    /// rows for it, which keeps these scheduler tests honest about the
    /// receipt sharing a transaction with canonical writes.
    fn test_ocr_output() -> EngineOutput {
        EngineOutput::Ocr(crate::processing::ocr::OcrComputeOutput {
            text: "hola".to_string(),
            method: "ocr".to_string(),
            outcome: "text".to_string(),
            regions_json: None,
            blocks_json: None,
            layout_model: String::new(),
            image_width: 0,
            image_height: 0,
            provider: "test".to_string(),
            page_count: 1,
        })
    }

    fn noop_commit(_task: &ClaimedTask, _output: &EngineOutput) {}

    fn noop_terminal(_task: &ClaimedTask, _state: &str, _code: &str, _message: &str) {}

    fn test_ctx(dir: &tempfile::TempDir) -> ExecCtx {
        ExecCtx {
            db_path: dir.path().join("entropia.sqlite"),
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
             CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, size INTEGER, parent_asset_id TEXT, page_number INTEGER, created_at INTEGER NOT NULL);
             CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);
             CREATE TABLE layouts (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, regions TEXT NOT NULL, blocks TEXT NOT NULL, model TEXT NOT NULL, image_width INTEGER NOT NULL, image_height INTEGER NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, model TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);
             CREATE UNIQUE INDEX idx_extractions_asset_id_unique ON extractions(asset_id);
             CREATE UNIQUE INDEX idx_layouts_asset_id_unique ON layouts(asset_id);
             CREATE TABLE llm_results (id TEXT PRIMARY KEY, target_id TEXT NOT NULL, target_type TEXT NOT NULL DEFAULT 'unknown', job_type TEXT NOT NULL, result TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE ocr_correction_backups (asset_id TEXT PRIMARY KEY, text_content TEXT NOT NULL);",
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
        let (dir, conn) = running_db();
        let ctx = test_ctx(&dir);
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
            run_one(
                &conn,
                &ctx,
                &failing,
                "s1",
                1000,
                &noop_commit,
                &noop_terminal
            )
            .expect("run 1"),
            RunOneOutcome::Waiting { .. }
        ));
        assert!(matches!(
            run_one(
                &conn,
                &ctx,
                &failing,
                "s1",
                40_000,
                &noop_commit,
                &noop_terminal
            )
            .expect("run 2"),
            RunOneOutcome::Succeeded { .. }
        ));
        // The second unit was never blocked by the first unit's failure, and
        // the drained batch observes itself completed.
        assert!(matches!(
            run_one(&conn, &ctx, &failing, "s1", 40_000, &noop_commit, &noop_terminal).expect("run 3"),
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
        let (dir, conn) = running_db();
        let ctx = test_ctx(&dir);
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
            run_one(
                &conn,
                &ctx,
                &stoppable,
                "s1",
                1000,
                &noop_commit,
                &noop_terminal
            )
            .expect("run 1"),
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
            run_one(
                &conn,
                &ctx,
                &stoppable,
                "s1",
                2000,
                &noop_commit,
                &noop_terminal
            )
            .expect("run 2"),
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
        let (dir, conn) = running_db();
        let ctx = test_ctx(&dir);
        let mut fatal = ExecutorRegistry::new();
        fatal.register(Arc::new(ScriptExecutor {
            script: std::sync::Mutex::new(vec![ExecOutput::Fatal {
                code: "corrupt_pdf".to_string(),
                message: "encrypted and locked".to_string(),
            }]),
            checkpoints_per_run: 0,
        }));
        assert!(matches!(
            run_one(
                &conn,
                &ctx,
                &fatal,
                "s1",
                1000,
                &noop_commit,
                &noop_terminal
            )
            .expect("run"),
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
    #[test]
    fn a_committed_unit_survives_later_failure_and_lease_reclaim() {
        let (dir, conn) = running_db();
        let ctx = test_ctx(&dir);
        let task = claim_next(&conn, "s1", &["ocr"], 1000).unwrap().unwrap();
        let first: String = ctx
            .unit(&task, "page:1", || Ok("confirmed text".to_string()))
            .unwrap();
        assert_eq!(first, "confirmed text");
        assert!(ctx
            .unit::<String>(&task, "page:2", || Err("provider failed".to_string()))
            .is_err());
        let stored: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id=?1",
                [&task.task_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, 1);
        repo::interrupt_task(&conn, &task.task_id, task.lease_epoch).unwrap();
        repo::control_batch(&conn, "b1", repo::BatchAction::Pause, None).unwrap();
        repo::control_batch(&conn, "b1", repo::BatchAction::Resume, None).unwrap();
        repo::promote_ready_batches(&conn).unwrap();
        let next = claim_next(&conn, "s2", &["ocr"], 2000).unwrap().unwrap();
        let resumed: String = ctx
            .unit(&next, "page:1", || {
                panic!("confirmed unit must not invoke engine again")
            })
            .unwrap();
        assert_eq!(resumed, "confirmed text");
        assert!(ctx
            .unit::<String>(&task, "late", || Ok("old writer".to_string()))
            .is_err());
    }
}
