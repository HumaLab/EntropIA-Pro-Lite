//! Process-death recovery for the batch queue (plan-lote.md §8, §13).
//!
//! Unit tests prove the state machine; this test proves the operating-system
//! half: a supervisor killed mid-claim (SIGKILL/TerminateProcess, no cleanup
//! runs) leaves a database the next process recovers without losing confirmed
//! work or re-running it.
//!
//! The test binary re-executes itself as the "child": with
//! `PROCESSING_RECOVERY_CHILD=1` it claims one unit and sleeps; the parent
//! kills it, then runs the same recovery a fresh app start would run.
//! Everything runs on throwaway temp files — never on the user's archive.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use entropia_desktop_lib::processing::repository;

const MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0032_batch_processing.sql");
const MIGRATION_0033_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0033_processing_source_invalidation.sql"
);

fn base_tables(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, size INTEGER, parent_asset_id TEXT, page_number INTEGER, created_at INTEGER NOT NULL);
         CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, method TEXT NOT NULL, created_at INTEGER NOT NULL);
         CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, model TEXT NOT NULL, created_at INTEGER NOT NULL);
         CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);",
    )
    .expect("base tables");
}

fn db_path() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    (dir, path)
}

/// Child mode: claim one unit like the supervisor thread would, then sleep
/// until the parent kills us. No cleanup may run — that is the point.
fn child_main(db_path: &std::path::Path) -> ! {
    use fs2::FileExt as _;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(
            db_path
                .canonicalize()
                .unwrap()
                .with_extension("processing.lock"),
        )
        .unwrap();
    lock.try_lock_exclusive().expect("child owns archive");
    let conn = entropia_desktop_lib::db_open_for_tests(db_path);
    let claimed = repository::claim_next(&conn, "child-session", &["ocr"], repository::now_ms())
        .expect("child claim");
    assert!(
        claimed.is_some(),
        "child must own a unit before the parent kills it"
    );
    let task = claimed.unwrap();
    let ctx = entropia_desktop_lib::processing::scheduler::ExecCtx {
        db_path: db_path.to_path_buf(),
    };
    let value: String = ctx
        .unit(&task, "page:1", || {
            Ok("confirmed page before crash".to_string())
        })
        .expect("checkpoint");
    assert_eq!(value, "confirmed page before crash");
    std::thread::sleep(Duration::from_secs(60));
    // If the parent failed to kill us, fail loudly instead of hanging CI.
    eprintln!("child survived past its kill window");
    std::process::exit(3);
}

#[test]
fn kill_mid_claim_recovers_without_losing_or_rerunning_work() {
    if std::env::var("PROCESSING_RECOVERY_CHILD").is_ok() {
        let path = std::env::var("PROCESSING_RECOVERY_DB").expect("child db path");
        child_main(PathBuf::from(path).as_path());
    }

    let (_dir, path) = db_path();
    {
        let conn = entropia_desktop_lib::db_open_for_tests(&path);
        base_tables(&conn);
        conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
            .expect("apply 0032");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0032_batch_processing', 1)",
            [],
        )
        .expect("track");
        conn.execute_batch(MIGRATION_0033_SQL).expect("apply 0033");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0033_processing_source_invalidation', 1)",
            [],
        )
        .expect("track 0033");
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
    }

    let exe = std::env::current_exe().expect("test binary path");
    let mut child = std::process::Command::new(exe)
        .env("PROCESSING_RECOVERY_CHILD", "1")
        .env("PROCESSING_RECOVERY_DB", &path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn child supervisor");

    // Wait until the child owns the unit (its claim committed).
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        {
            let conn = entropia_desktop_lib::db_open_for_tests(&path);
            let checkpoints: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id = 'ocr-a1'",
                    [],
                    |row| row.get(0),
                )
                .expect("read checkpoint state");
            if checkpoints == 1 {
                break;
            }
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("child never claimed the unit");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // A stalled heartbeat does not authorize a second process to recover
    // this live owner. The OS lock remains authoritative until process death.
    let peer = entropia_desktop_lib::processing::recovery::recover_once_if_needed(&path)
        .unwrap()
        .unwrap();
    assert!(peer.peer_alive);
    assert_eq!(peer.tasks_interrupted, 0);

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let supervisor = entropia_desktop_lib::processing::scheduler::start_scheduler(
        path.clone(),
        "parent-session".to_string(),
        std::sync::Arc::new(entropia_desktop_lib::processing::scheduler::ExecutorRegistry::new()),
        std::sync::Arc::clone(&stop),
        std::sync::Arc::new(|_, _| {}),
        std::sync::Arc::new(|_, _, _, _| {}),
    );

    // SIGKILL semantics: no Drop, no finally, no lease release. The already
    // running standby must acquire ownership and recover without an app restart.
    child.kill().expect("kill child");
    let status = child.wait().expect("reap child");
    assert!(!status.success(), "the child must die by signal, not exit");
    let takeover_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let conn = entropia_desktop_lib::db_open_for_tests(&path);
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 'ocr-a1'",
                [],
                |row| row.get(0),
            )
            .expect("read takeover state");
        if state == "interrupted" {
            break;
        }
        assert!(
            Instant::now() <= takeover_deadline,
            "standby never acquired the released archive lock"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    supervisor.join().expect("stop standby supervisor");

    // Fresh process view: recovery already converged exactly like app startup.
    {
        let conn = entropia_desktop_lib::db_open_for_tests(&path);
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 'ocr-a1'",
                [],
                |row| row.get(0),
            )
            .expect("interrupted unit");
        assert_eq!(state, "interrupted");
        // Resume requeues without recomputing anything confirmed — and, once
        // the supervisor tick promotes the ready batch, the re-claim
        // succeeds, proving the fencing epoch moved on.
        repository::control_batch(&conn, "b1", repository::BatchAction::Resume, None)
            .expect("resume");
        let ctx = entropia_desktop_lib::processing::scheduler::ExecCtx {
            db_path: path.clone(),
        };
        let checkpoints: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id='ocr-a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(checkpoints, 1);
        let registry = entropia_desktop_lib::processing::scheduler::ExecutorRegistry::new();
        entropia_desktop_lib::processing::scheduler::scheduler_tick(
            &conn,
            &ctx,
            &registry,
            "parent-session",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("tick promotes the resumed batch");
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_batches WHERE id = 'b1'",
                [],
                |row| row.get(0),
            )
            .expect("batch running again");
        assert_eq!(state, "running");
        let reclaimed =
            repository::claim_next(&conn, "parent-session", &["ocr"], repository::now_ms())
                .expect("reclaim");
        assert!(
            reclaimed.is_some(),
            "the interrupted unit must be claimable after resume"
        );
        let resumed: String = ctx
            .unit(&reclaimed.unwrap(), "page:1", || {
                panic!("confirmed page repeated after process kill")
            })
            .unwrap();
        assert_eq!(resumed, "confirmed page before crash");
    }
}
