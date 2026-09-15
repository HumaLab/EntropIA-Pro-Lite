//! Process-death recovery for the manuscript journal (plan-editor.md §16.3).
//!
//! The unit tests prove the rules; this proves the operating-system half. Spike
//! S6 measured the same guarantee against a bare table — here it runs through
//! the real module: a writer killed mid-typing (TerminateProcess, no cleanup,
//! no Drop, no close hook) must leave a database where every delta it announced
//! as durable comes back, and the confirmed document is untouched.
//!
//! The test binary re-executes itself as the writer. It appends deltas as fast
//! as it can and prints each committed sequence number to a file it flushes, so
//! the parent knows exactly what was promised before the kill landed.
//!
//! Everything runs on throwaway temp files — never on the user's archive.
//!
//! What this does NOT prove: power loss. A kill demonstrates process death.
//! `synchronous=FULL` is what buys the rest, and confirming it needs a
//! disposable VM with an instrumented unclean cut.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use entropia_desktop_lib::writing::{journal, recovery, repository};

const MIGRATION_0035: &str =
    include_str!("../../../../packages/store/src/migrations/0035_writing_workspace.sql");
const MIGRATION_0036: &str =
    include_str!("../../../../packages/store/src/migrations/0036_writing_journal.sql");

const CHILD_ENV: &str = "WRITING_RECOVERY_CHILD";
const DB_ENV: &str = "WRITING_RECOVERY_DB";
const LOG_ENV: &str = "WRITING_RECOVERY_LOG";

const DOCUMENT_ID: &str = "d1";
const CONFIRMED_CONTENT: &str = r#"{"type":"doc","confirmed":true}"#;

fn prepare(path: &Path) {
    let conn = entropia_desktop_lib::db_open_for_tests(path);
    conn.execute_batch(
        "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);",
    )
    .expect("base tables");
    conn.execute_batch(&format!(
        "BEGIN IMMEDIATE;\n{MIGRATION_0035}\n{MIGRATION_0036}\n\
         INSERT INTO _migrations (name, applied_at) VALUES ('0035_writing_workspace', 1);\n\
         INSERT INTO _migrations (name, applied_at) VALUES ('0036_writing_journal', 1);\nCOMMIT;"
    ))
    .expect("apply writing migrations");

    repository::create_document(
        &conn,
        repository::NewDocument {
            id: DOCUMENT_ID.to_string(),
            title: "Articulo".into(),
            document_type: "article".into(),
            schema_version: 1,
            content_json: CONFIRMED_CONTENT.to_string(),
        },
    )
    .expect("create document");
}

/// Writer mode: append deltas without pause, announcing each one only after the
/// journal returned. Sleeps forever at the end; the parent is expected to kill
/// it long before that.
fn child_main(db_path: &Path, log_path: &Path) -> ! {
    let conn = entropia_desktop_lib::db_open_for_tests(db_path);
    let mut log = std::fs::File::create(log_path).expect("open log");

    for i in 0..100_000u64 {
        let seq = journal::append(
            &conn,
            journal::AppendJournal {
                document_id: DOCUMENT_ID.to_string(),
                base_revision: 0,
                schema_version: 1,
                delta_json: format!("[{{\"step\":{i}}}]"),
            },
        )
        .expect("append");
        // Announced only after the write committed, so anything in this file
        // is something the journal promised was durable.
        writeln!(log, "{seq}").expect("write log");
        log.flush().expect("flush log");
    }

    std::thread::sleep(Duration::from_secs(60));
    eprintln!("writer survived past its kill window");
    std::process::exit(3);
}

#[test]
fn killing_a_writer_mid_typing_keeps_every_delta_it_promised() {
    if std::env::var(CHILD_ENV).is_ok() {
        let db = std::env::var(DB_ENV).expect("child db path");
        let log = std::env::var(LOG_ENV).expect("child log path");
        child_main(PathBuf::from(db).as_path(), PathBuf::from(log).as_path());
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    let log_path = dir.path().join("committed.txt");
    prepare(&path);

    let exe = std::env::current_exe().expect("test binary path");
    let mut child = std::process::Command::new(exe)
        .env(CHILD_ENV, "1")
        .env(DB_ENV, &path)
        .env(LOG_ENV, &log_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn writer");

    // Let it get properly under way, then kill it with no chance to clean up.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let announced = std::fs::read_to_string(&log_path).unwrap_or_default();
        if announced.lines().count() >= 25 {
            break;
        }
        assert!(Instant::now() < deadline, "writer never got going");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().expect("kill writer");
    let _ = child.wait();

    // Everything the writer announced as durable, straight from its log.
    let announced: Vec<i64> = std::fs::read_to_string(&log_path)
        .expect("read log")
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    assert!(announced.len() >= 25, "expected a meaningful run");
    let last_announced = *announced.last().expect("at least one");

    // A fresh process opens the same file, exactly as the next app start would.
    let reopened = entropia_desktop_lib::db_open_for_tests(&path);
    let plan = recovery::plan(&reopened, DOCUMENT_ID).expect("recovery plan");

    assert_eq!(
        plan.stopped_at_seq, None,
        "a killed writer must not leave a corrupt entry: {plan:?}"
    );

    let recovered: Vec<i64> = plan.replayable.iter().map(|e| e.seq).collect();
    for seq in &announced {
        assert!(
            recovered.contains(seq),
            "sequence {seq} was announced durable and did not survive"
        );
    }

    // The store may legitimately hold ONE more than was announced: the commit
    // landed and the process died before it could say so. That is the safe
    // direction, and recovery must accept it rather than call it corruption.
    let extra = recovered.len() - announced.len();
    assert!(
        extra <= 1,
        "recovered {} entries for {} announced; only the unacknowledged commit is expected",
        recovered.len(),
        announced.len()
    );
    if let Some(&max) = recovered.iter().max() {
        assert!(max == last_announced || max == last_announced + 1);
    }

    // The confirmed document is exactly where it was: the journal never wrote
    // through to it, and the kill changed nothing about it.
    let doc = repository::load_document(&reopened, DOCUMENT_ID).expect("load");
    assert_eq!(doc.revision, 0, "journalling is not a canonical save");
    assert_eq!(doc.current_content_json, CONFIRMED_CONTENT);

    // And the deltas are intact, not just present.
    let replayed: Vec<String> = plan
        .replayable
        .iter()
        .map(|e| e.delta_json.clone())
        .collect();
    assert!(replayed.iter().all(|d| d.starts_with("[{\"step\":")));
}
