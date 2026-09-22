//! E2b-3: durable bibliographic page persistence behind the batch queue.
//!
//! These tests drive the real claim → dispatch → execute → publish path with
//! synthetic libraries and fake page sources — never a live Zotero or private
//! data. They pin page-transaction atomicity, restart convergence, trusted
//! finalization, scheduler publication, and honest stop/error states.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use entropia_desktop_lib::bibliography::processing::{
    BibliographyPage, BibliographyPageItem, BibliographyPageQuery, BibliographySyncExecutor,
    PageFuture, ZoteroPageSource,
};
use entropia_desktop_lib::bibliography::reconciliation::{get_run, ReconciliationState};
use entropia_desktop_lib::bibliography::repository::{upsert_item, BibliographicItemInput};
use entropia_desktop_lib::processing::repository::{self, TaskSubject};
use entropia_desktop_lib::processing::scheduler::{
    run_one, ExecCtx, ExecOutput, ExecResult, Executor, ExecutorRegistry, RunOneOutcome, StopFlag,
};
use entropia_desktop_lib::writing::zotero::{Library, ZoteroState};

const MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0032_batch_processing.sql");
const MIGRATION_0033_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0033_processing_source_invalidation.sql"
);
const MIGRATION_0038_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0038_bibliography_catalog.sql");
const MIGRATION_0039_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0039_bibliography_relations.sql");
const MIGRATION_0040_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0040_bibliography_reconciliation.sql");
const MIGRATION_0041_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0041_processing_task_subject_identity.sql"
);
const MIGRATION_0042_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0042_processing_task_subject_cutover.sql"
);
const MIGRATION_0043_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0043_bibliography_sync_tasks.sql");

/// Archive shape good enough for both claim arms: the corpus tables the
/// eligibility validator reads plus the real processing and bibliography
/// schemas. Catalog/reconciliation tables are mandatory for E2b-3.
fn migrated_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    let conn = entropia_desktop_lib::db_open_for_tests(&path);
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
    for (sql, name) in [
        (MIGRATION_SQL, "0032_batch_processing"),
        (MIGRATION_0033_SQL, "0033_processing_source_invalidation"),
        (MIGRATION_0038_SQL, "0038_bibliography_catalog"),
        (MIGRATION_0039_SQL, "0039_bibliography_relations"),
        (MIGRATION_0040_SQL, "0040_bibliography_reconciliation"),
        (MIGRATION_0041_SQL, "0041_processing_task_subject_identity"),
        (MIGRATION_0042_SQL, "0042_processing_task_subject_cutover"),
        (MIGRATION_0043_SQL, "0043_bibliography_sync_tasks"),
    ] {
        conn.execute_batch(sql).expect("apply migration");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [name],
        )
        .expect("track migration");
    }
    (dir, conn)
}

fn seed_library(conn: &rusqlite::Connection, row_id: &str, version: Option<i64>) {
    conn.execute(
        "INSERT OR IGNORE INTO zotero_connections
           (id, source_origin, source_instance_id, endpoint, capabilities_json,
            state, revision, created_at, updated_at)
         VALUES ('conn-1', 'local', NULL, 'http://synthetic.invalid', '{}',
                 'available', 0, 1, 1)",
        [],
    )
    .expect("seed connection");
    conn.execute(
        "INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name,
                                        last_modified_version, revision, created_at, updated_at)
         VALUES (?1, 'conn-1', 'user', '0', 'Personal', ?2, 1, 1, 1)",
        rusqlite::params![row_id, version],
    )
    .expect("seed library");
}

/// Admits one bibliography_sync task for `row_id` the way E2b-4's manual
/// trigger eventually will: the bibliography system batch plus the
/// subject-explicit admission core.
fn admit_bibliography_task(conn: &rusqlite::Connection, row_id: &str) -> String {
    let batch = repository::ensure_system_batch(conn, "bibliography").expect("system batch");
    let subject = TaskSubject {
        domain: "bibliography".to_string(),
        subject_kind: "library".to_string(),
        subject_id: row_id.to_string(),
    };
    repository::admit_subject_or_attach(
        conn,
        &batch,
        "bibliography_sync",
        &subject,
        0,
        "",
        "",
        None,
    )
    .expect("admit bibliography sync")
    .task_id
}

fn claim_bibliography(conn: &rusqlite::Connection) -> repository::ClaimedTask {
    repository::claim_next(
        conn,
        "bib-session",
        &["bibliography_sync"],
        repository::now_ms(),
    )
    .expect("claim scan")
    .expect("claimable bibliography task")
}

fn page_at_version(
    items: Vec<BibliographyPageItem>,
    total: Option<u64>,
    library_version: u64,
) -> BibliographyPage {
    BibliographyPage {
        library_version: Some(library_version),
        total,
        items,
    }
}

fn page(items: Vec<BibliographyPageItem>, total: Option<u64>) -> BibliographyPage {
    page_at_version(items, total, 99)
}

fn item(key: &str, version: u64) -> BibliographyPageItem {
    // The fake builds the same shape the production parser emits: the
    // original native row serialized in full beside its key, version and
    // CSL text. CSL id deliberately differs from the native key: catalog
    // identity must always remain the qualified Zotero key.
    let csl = serde_json::json!({
        "id": format!("csl-{key}"),
        "type": "book",
        "title": format!("Work {key}"),
        "container-title": "Synthetic Journal",
        "publisher": "CSL fallback publisher",
        "issued": { "date-parts": [[2025, 2, 3]] },
        "DOI": format!("10.0000/{key}"),
        "ISBN": "978-0-00-000000-0",
        "abstract": format!("Abstract {key}"),
        "language": "en",
        "URL": format!("https://synthetic.invalid/{key}")
    });
    let native_row = serde_json::json!({
        "key": key,
        "version": version,
        "csljson": csl.to_string(),
        "data": {
            "itemType": "book",
            "title": format!("Work {key}"),
            "creators": [{
                "creatorType": "author",
                "firstName": "Ada",
                "lastName": "Lovelace"
            }],
            "publicationTitle": "Native Journal",
            "publisher": "Native Publisher",
            "date": "2025",
            "DOI": format!("10.0000/{key}"),
            "ISBN": "978-0-00-000000-0",
            "abstractNote": format!("Abstract {key}"),
            "language": "en",
            "url": format!("https://synthetic.invalid/{key}")
        }
    });
    BibliographyPageItem {
        key: key.to_string(),
        item_version: version,
        csl_json: csl.to_string(),
        native_json_snapshot: native_row.to_string(),
    }
}

enum ScriptStep {
    Page(BibliographyPage),
    Fail(ZoteroState),
}

/// Fake page source: records every request (library identity, start, limit)
/// and answers from a script; an empty script answers a consistent terminal
/// empty page. Can trip a shared `StopFlag` while serving request `n`
/// (1-based) to simulate the supervisor's demand watcher.
struct FakeSource {
    script: Mutex<VecDeque<ScriptStep>>,
    requests: Mutex<Vec<(String, u32, u32)>>,
    stop_while_serving: Option<(usize, Arc<StopFlag>)>,
}

impl FakeSource {
    fn new(script: Vec<ScriptStep>) -> Self {
        Self {
            script: Mutex::new(script.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            stop_while_serving: None,
        }
    }

    /// Trips `flag` while serving request `n` (1-based): the demand watcher
    /// withdrawing demand mid-flight.
    fn trip_stop_while_serving(mut self, request_number: usize, flag: Arc<StopFlag>) -> Self {
        self.stop_while_serving = Some((request_number, flag));
        self
    }

    fn requests(&self) -> Vec<(String, u32, u32)> {
        self.requests.lock().expect("requests").clone()
    }
}

impl ZoteroPageSource for FakeSource {
    fn fetch_page(&self, library: &Library, query: BibliographyPageQuery) -> PageFuture {
        let index = {
            let mut requests = self.requests.lock().expect("requests");
            requests.push((library.storage_key(), query.start(), query.limit()));
            requests.len()
        };
        let step = self
            .script
            .lock()
            .expect("script")
            .pop_front()
            .unwrap_or(ScriptStep::Page(page(Vec::new(), None)));
        let trip = match &self.stop_while_serving {
            Some((at, flag)) if *at == index => Some(Arc::clone(flag)),
            _ => None,
        };
        Box::pin(async move {
            // The stop arrives mid-flight: the request in progress still
            // completes and is recorded — only the next one is forbidden.
            if let Some(flag) = trip {
                flag.stop();
            }
            match step {
                ScriptStep::Page(answer) => Ok(answer),
                ScriptStep::Fail(state) => Err(state),
            }
        })
    }
}

fn executor(source: Arc<FakeSource>) -> BibliographySyncExecutor {
    BibliographySyncExecutor::new(source).with_page_limit(2)
}

fn ctx_of(dir: &tempfile::TempDir) -> ExecCtx {
    ExecCtx {
        db_path: dir.path().join("entropia.sqlite"),
    }
}

/// Runs the executor directly (no `run_one` wrapper) with a fresh claim.
fn run_directly(
    dir: &tempfile::TempDir,
    conn: &rusqlite::Connection,
    source: Arc<FakeSource>,
    stop: &StopFlag,
) -> (ExecResult, repository::ClaimedTask) {
    let task = claim_bibliography(conn);
    let result = executor(source).run(&ctx_of(dir), &task, stop);
    (result, task)
}

/// Every answered page becomes one durable catalog/reconciliation checkpoint
/// before the next cursor is requested. Finalization is deliberately left to
/// the scheduler success transaction, so a direct executor run has canonical
/// pages but no task receipt yet.
#[test]
fn pages_are_persisted_atomically_before_the_cursor_advances() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");

    let fake = Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(6),
        )),
        ScriptStep::Page(page(
            vec![item("CCCC3333", 3), item("DDDD4444", 9)],
            Some(6),
        )),
        ScriptStep::Page(page(
            vec![item("EEEE5555", 1), item("FFFF6666", 2)],
            Some(6),
        )),
    ]));
    let (result, task) = run_directly(&dir, &conn, Arc::clone(&fake), &StopFlag::new());
    assert_eq!(task.task_id, task_id);
    assert!(
        matches!(&result.output, ExecOutput::Success { .. }),
        "trusted enumeration must be publishable, got {:?}",
        result.output
    );
    assert_eq!(
        fake.requests(),
        vec![
            ("0".to_string(), 0, 2),
            ("0".to_string(), 2, 2),
            ("0".to_string(), 4, 2),
        ]
    );
    assert_eq!(result.checkpoints.len(), 3);
    assert_eq!(result.checkpoints[0].unit_key, "page:0");
    assert_eq!(result.checkpoints[2].unit_key, "page:4");
    assert_eq!(result.progress_total, Some(6));

    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("run exists");
    assert_eq!(run.cursor_start, 6);
    assert_eq!(run.remote_total, Some(6));
    assert_eq!(run.state, ReconciliationState::Running);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id='lib-1'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("catalog item count"),
        6
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen WHERE library_id='lib-1'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("seen item count"),
        6
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id=?1",
            [&task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("durable page checkpoint count"),
        3
    );
    let receipt: Option<String> = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE id=?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("task receipt");
    assert_eq!(
        receipt, None,
        "only the scheduler success commit emits a receipt"
    );

    let staged: serde_json::Value =
        serde_json::from_str(&result.checkpoints[0].payload).expect("staged page JSON");
    let snapshot: serde_json::Value = serde_json::from_str(
        staged["items"][0]["native_json_snapshot"]
            .as_str()
            .expect("native snapshot in checkpoint"),
    )
    .expect("snapshot JSON");
    assert_eq!(snapshot["key"].as_str(), Some("AAAA1111"));
    assert!(snapshot["data"].is_object(), "native fields must survive");
}

/// The item writes, seen-set, queue payload and cursor are one page unit. A
/// failure while inserting the seen-set rolls the item upserts back too.
#[test]
fn a_page_rolls_back_items_seen_cursor_and_checkpoint_together() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    conn.execute_batch(
        "CREATE TRIGGER reject_synthetic_seen
         BEFORE INSERT ON zotero_reconciliation_seen
         BEGIN SELECT RAISE(ABORT, 'synthetic seen failure'); END;",
    )
    .expect("install rollback trigger");

    let fake = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item("AAAA1111", 12), item("BBBB2222", 40)],
        Some(2),
    ))]));
    let (result, _) = run_directly(&dir, &conn, fake, &StopFlag::new());
    assert!(
        matches!(&result.output, ExecOutput::Fatal { .. }),
        "storage failure must not report success: {:?}",
        result.output
    );
    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("run begins before page persistence");
    assert_eq!(run.cursor_start, 0);
    for (table, expected) in [
        ("bibliographic_items", 0_i64),
        ("zotero_reconciliation_seen", 0_i64),
        ("processing_checkpoints", 0_i64),
    ] {
        let sql = if table == "processing_checkpoints" {
            format!("SELECT COUNT(*) FROM {table} WHERE task_id='{task_id}'")
        } else {
            format!("SELECT COUNT(*) FROM {table}")
        };
        assert_eq!(
            conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
                .expect("rolled-back row count"),
            expected,
            "{table} must roll back with the page"
        );
    }
    assert!(
        repository::execution_wanted(&conn, &task_id).expect("demand"),
        "a failed page must not consume demand"
    );
}

/// The stop boundary: observed before each request, never mid-request, and
/// once stopped no further request leaves the executor. Confirmed pages
/// survive as checkpoints for the resume.
#[test]
fn stop_between_pages_never_preempts_inflight_and_forbids_the_next_request() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");

    // Five pages exist; the watcher withdraws demand while request 2 is in
    // flight. Page 2 still completes (never preempted), and there is no
    // request 3.
    let shared_stop = Arc::new(StopFlag::new());
    let fake = FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(10),
        )),
        ScriptStep::Page(page(
            vec![item("CCCC3333", 3), item("DDDD4444", 9)],
            Some(10),
        )),
        ScriptStep::Page(page(
            vec![item("EEEE5555", 1), item("FFFF6666", 2)],
            Some(10),
        )),
    ])
    .trip_stop_while_serving(2, Arc::clone(&shared_stop));

    let task = claim_bibliography(&conn);
    let result = executor(Arc::new(fake)).run(&ctx_of(&dir), &task, &shared_stop);
    assert!(matches!(&result.output, ExecOutput::Stopped));

    // Two confirmed pages survive as checkpoint units for the resume: the
    // request that was in flight when demand vanished completed and staged,
    // and no third request exists.
    assert_eq!(result.checkpoints.len(), 2);
    assert_eq!(result.checkpoints[0].unit_key, "page:0");
    assert_eq!(result.checkpoints[1].unit_key, "page:2");
    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("interrupted run");
    assert_eq!(run.cursor_start, 4);
    assert_eq!(run.state, ReconciliationState::Interrupted);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM bibliographic_items", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("committed items"),
        4
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM zotero_item_tombstones", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("tombstones"),
        0,
        "cooperative stop never finalizes deletions"
    );
    assert!(repository::execution_wanted(&conn, &task.task_id).expect("demand"));
}

/// Stop is also observed after the last page: a run whose demand vanished at
/// the end reports `Stopped`, not a completion verdict.
#[test]
fn stop_after_the_last_page_still_reports_stopped() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");

    // Two pages total; the stop arrives while the last request is served.
    let shared_stop = Arc::new(StopFlag::new());
    let fake = FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(4),
        )),
        ScriptStep::Page(page(
            vec![item("CCCC3333", 3), item("DDDD4444", 9)],
            Some(4),
        )),
    ])
    .trip_stop_while_serving(2, Arc::clone(&shared_stop));

    let task = claim_bibliography(&conn);
    let result = executor(Arc::new(fake)).run(&ctx_of(&dir), &task, &shared_stop);
    assert!(matches!(&result.output, ExecOutput::Stopped));
    assert_eq!(
        result.checkpoints.len(),
        2,
        "all pages confirmed before stopping"
    );
    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("interrupted run");
    assert_eq!(run.cursor_start, 4);
    assert_eq!(run.state, ReconciliationState::Interrupted);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM zotero_item_tombstones", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("tombstones"),
        0
    );
}

/// A lost internal library row is a fatal, library-specific verdict — no
/// request is ever made, and the state is distinct from every endpoint
/// state. The loss is observed after the claim (the claim validator would
/// skip the task first, which is exactly the seam below), because the
/// executor must also defend itself against a row that vanishes mid-run.
#[test]
fn a_missing_library_row_is_fatal_before_any_request() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");
    let task = claim_bibliography(&conn);
    conn.execute("DELETE FROM zotero_libraries WHERE id = 'lib-1'", [])
        .expect("lose the library between claim and run");

    let fake = Arc::new(FakeSource::new(Vec::new()));
    let result = executor(Arc::clone(&fake)).run(&ctx_of(&dir), &task, &StopFlag::new());
    assert!(
        matches!(&result.output, ExecOutput::Fatal { code, .. } if code == "library_missing"),
        "fatal library_missing, got {:?}",
        result.output
    );
    assert!(result.checkpoints.is_empty());
    assert!(
        fake.requests().is_empty(),
        "a missing library must not reach the endpoint"
    );
}

/// When the row is already gone at claim time, the claim validator settles
/// the task without ever calling a motor — the executor's fatal above is
/// the second line of defense, not the only one.
#[test]
fn a_library_row_already_missing_at_claim_time_skips_the_task() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    conn.execute("DELETE FROM zotero_libraries WHERE id = 'lib-1'", [])
        .expect("lose the library before the claim");
    let claimed = repository::claim_next(&conn, "s", &["bibliography_sync"], repository::now_ms())
        .expect("claim scan");
    assert!(claimed.is_none(), "no motor may run for a lost library");
    let settled: (String, String) = conn
        .query_row(
            "SELECT state, outcome FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("settled bibliography row");
    assert_eq!(
        settled,
        ("skipped".to_string(), "library_missing".to_string())
    );
}

enum Verdict {
    Retryable,
    Blocked,
    Fatal,
}

/// The endpoint states land on distinct, honest verdicts: unreachable and
/// timeout are retryable (the endpoint may come back), a disabled local API
/// is blocked (a human toggles a setting), and a definitive `404` for this
/// library is fatal (it no longer exists on the Zotero side). None of the
/// messages may claim Zotero is closed or not installed.
#[test]
fn endpoint_states_map_to_honest_distinct_verdicts() {
    let cases: &[(&str, ZoteroState, Verdict)] = &[
        (
            "zotero_unreachable",
            ZoteroState::EndpointUnavailable,
            Verdict::Retryable,
        ),
        ("zotero_timeout", ZoteroState::Timeout, Verdict::Retryable),
        (
            "zotero_api_disabled",
            ZoteroState::ApiDisabled,
            Verdict::Blocked,
        ),
        (
            "zotero_library_not_found",
            ZoteroState::NotFound,
            Verdict::Fatal,
        ),
        (
            "zotero_invalid_response",
            ZoteroState::InvalidResponse {
                detail: "the library's answer was not a list of items".into(),
            },
            Verdict::Retryable,
        ),
    ];
    for (expected_code, state, expected_verdict) in cases {
        let (dir, conn) = migrated_db();
        seed_library(&conn, "lib-1", Some(7));
        admit_bibliography_task(&conn, "lib-1");
        let fake = Arc::new(FakeSource::new(vec![ScriptStep::Fail(state.clone())]));
        let (result, task) = run_directly(&dir, &conn, Arc::clone(&fake), &StopFlag::new());
        assert_eq!(
            fake.requests().len(),
            1,
            "the state surfaces on the first page request"
        );
        let (code, message) = match &result.output {
            ExecOutput::Retryable { code, message }
            | ExecOutput::Fatal { code, message }
            | ExecOutput::Blocked { code, message } => (code.as_str(), message.as_str()),
            other => panic!("expected a state verdict for {expected_code}, got {other:?}"),
        };
        assert_eq!(code, *expected_code, "distinct stable code per state");
        match expected_verdict {
            Verdict::Retryable => {
                assert!(matches!(&result.output, ExecOutput::Retryable { .. }));
            }
            Verdict::Blocked => {
                assert!(matches!(&result.output, ExecOutput::Blocked { .. }));
            }
            Verdict::Fatal => {
                assert!(matches!(&result.output, ExecOutput::Fatal { .. }));
            }
        }
        assert!(
            !message.to_lowercase().contains("closed")
                && !message.to_lowercase().contains("not installed"),
            "no message may claim Zotero is closed or not installed: {message}"
        );
        let run = get_run(&conn, "lib-1")
            .expect("read reconciliation")
            .expect("request state is durable");
        assert_eq!(run.cursor_start, 0, "failed first request cannot advance");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM zotero_item_tombstones", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("tombstones"),
            0
        );
        assert!(repository::execution_wanted(&conn, &task.task_id).expect("demand"));
    }
}

/// A malformed answer is never treated as an empty library: an inconsistent
/// page (zero items while the total says more remain) is a retryable error,
/// not a completed enumeration.
#[test]
fn an_inconsistent_empty_page_is_an_error_not_an_empty_completion() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");

    let fake = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        Vec::new(),
        Some(5),
    ))]));
    let (result, task) = run_directly(&dir, &conn, Arc::clone(&fake), &StopFlag::new());
    assert_eq!(fake.requests().len(), 1);
    assert!(
        matches!(&result.output, ExecOutput::Retryable { code, .. }
            if code == "zotero_invalid_response"),
        "inconsistent empty must be a retryable malformed answer, got {:?}",
        result.output
    );
    assert!(result.checkpoints.is_empty());
    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("run exists");
    assert_eq!(run.cursor_start, 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM zotero_item_tombstones", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("tombstones"),
        0
    );
    assert!(repository::execution_wanted(&conn, &task.task_id).expect("demand"));
}

/// The registry dispatches bibliography output only through its catalog
/// publisher. Trusted finalization and the queue receipt commit together.
#[test]
fn complete_run_publishes_catalog_finalization_and_receipt_together() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(executor(Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(vec![item("AAAA1111", 12)], Some(1))),
    ])))));
    assert_eq!(registry.kinds(), vec!["bibliography_sync".to_string()]);
    assert!(registry.get("bibliography_sync").is_some());
    assert!(registry.get("ocr").is_none());

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("run one bibliography unit");
    assert!(matches!(outcome, RunOneOutcome::Succeeded { task_id: id } if id == task_id));

    let (state, receipt, checkpoint_count): (String, Option<String>, i64) = conn
        .query_row(
            "SELECT t.state, t.result_receipt_json, (
                SELECT COUNT(*) FROM processing_checkpoints c WHERE c.task_id = t.id
             )
             FROM processing_tasks t
             WHERE t.id = ?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("settled task");
    assert_eq!(state, "succeeded");
    let receipt: serde_json::Value =
        serde_json::from_str(&receipt.expect("durable receipt")).expect("receipt JSON");
    assert_eq!(receipt["libraryRowId"].as_str(), Some("lib-1"));
    assert_eq!(receipt["itemsSeen"].as_u64(), Some(1));
    assert_eq!(checkpoint_count, 1, "the confirmed page survives durably");
    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("completed run");
    assert_eq!(run.state, ReconciliationState::Completed);
    assert!(run.completed_at.is_some());
    let stored: (String, String, String, String, String, String) = conn
        .query_row(
            "SELECT item_key, native_json_snapshot, title, creators_json,
                    publication_title, doi
               FROM bibliographic_items
              WHERE library_id='lib-1' AND item_key='AAAA1111'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .expect("published item");
    assert_eq!(
        stored.0, "AAAA1111",
        "CSL id cannot replace native identity"
    );
    assert_eq!(stored.1, item("AAAA1111", 12).native_json_snapshot);
    assert_eq!(stored.2, "Work AAAA1111");
    assert!(stored.3.contains("Lovelace"));
    assert_eq!(stored.4, "Native Journal");
    assert_eq!(stored.5, "10.0000/AAAA1111");
}

/// A retry resumes from the reconciliation cursor committed with the prior
/// page. Upserts and seen keys converge without duplicate catalog identity,
/// and only trusted completion tombstones a prior unseen item.
#[test]
fn retry_resumes_at_the_committed_cursor_and_tombstones_only_on_finalize() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let stale = upsert_item(
        &mut conn,
        "lib-1",
        BibliographicItemInput {
            item_key: "STALE000".to_string(),
            item_version: Some(1),
            native_json_snapshot: r#"{"key":"STALE000","version":1}"#.to_string(),
            csl_json_snapshot: r#"{"id":"STALE000","type":"book"}"#.to_string(),
            title: Some("Previously present".to_string()),
            ..Default::default()
        },
    )
    .expect("seed prior catalog item");
    let task_id = admit_bibliography_task(&conn, "lib-1");

    let first = Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(3),
        )),
        ScriptStep::Fail(ZoteroState::Timeout),
    ]));
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(executor(Arc::clone(&first))));
    let first_outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bib-session-1",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("first attempt");
    assert!(matches!(first_outcome, RunOneOutcome::Waiting { .. }));
    assert_eq!(
        first.requests(),
        vec![("0".to_string(), 0, 2), ("0".to_string(), 2, 2)]
    );
    let interrupted = get_run(&conn, "lib-1")
        .expect("read run")
        .expect("retryable run");
    assert_eq!(interrupted.cursor_start, 2);
    assert_eq!(interrupted.state, ReconciliationState::RetryWait);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_item_tombstones WHERE item_id=?1",
            [&stale.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("partial tombstone count"),
        0,
        "a partial walk cannot infer deletion"
    );
    assert!(repository::execution_wanted(&conn, &task_id).expect("retry demand"));

    conn.execute(
        "UPDATE processing_tasks SET next_retry_at=0 WHERE id=?1",
        [&task_id],
    )
    .expect("make retry due");
    let second = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item("CCCC3333", 3)],
        Some(3),
    ))]));
    let mut retry_registry = ExecutorRegistry::new();
    retry_registry.register(Arc::new(executor(Arc::clone(&second))));
    let second_outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &retry_registry,
        "bib-session-2",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("resumed attempt");
    assert!(matches!(second_outcome, RunOneOutcome::Succeeded { .. }));
    assert_eq!(
        second.requests(),
        vec![("0".to_string(), 2, 2)],
        "resume must not fetch an already committed page"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items
             WHERE library_id='lib-1' AND item_key IN ('AAAA1111','BBBB2222','CCCC3333')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("unique resumed items"),
        3
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen
             WHERE library_id='lib-1' AND entity_kind='item'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("idempotent seen set"),
        3
    );
    let tombstone: (Option<i64>, String) = conn
        .query_row(
            "SELECT remote_version, reason FROM zotero_item_tombstones WHERE item_id=?1",
            [&stale.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("trusted finalization tombstone");
    assert_eq!(tombstone.0, Some(99));
    assert!(!tombstone.1.trim().is_empty());
}

/// A library version fence that changes between pages retires the partial
/// reconciliation. The processing task remains retryable, and its next attempt
/// converges by beginning at zero rather than looping forever on the old
/// cursor/seen-set.
#[test]
fn a_changed_library_version_restarts_the_next_attempt_from_zero() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");

    let first = Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page_at_version(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(3),
            99,
        )),
        ScriptStep::Fail(ZoteroState::Timeout),
    ]));
    let mut first_registry = ExecutorRegistry::new();
    first_registry.register(Arc::new(executor(first)));
    assert!(matches!(
        run_one(
            &conn,
            &ctx_of(&dir),
            &first_registry,
            "bib-session-1",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("partial attempt"),
        RunOneOutcome::Waiting { .. }
    ));
    let partial = get_run(&conn, "lib-1")
        .expect("read partial run")
        .expect("partial run");
    assert_eq!(partial.cursor_start, 2);

    conn.execute(
        "UPDATE processing_tasks SET next_retry_at=0 WHERE id=?1",
        [&task_id],
    )
    .expect("make second attempt due");
    let changed = Arc::new(FakeSource::new(vec![ScriptStep::Page(page_at_version(
        vec![item("CCCC3333", 3)],
        Some(3),
        100,
    ))]));
    let mut changed_registry = ExecutorRegistry::new();
    changed_registry.register(Arc::new(executor(Arc::clone(&changed))));
    assert!(matches!(
        run_one(
            &conn,
            &ctx_of(&dir),
            &changed_registry,
            "bib-session-2",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("changed-version attempt"),
        RunOneOutcome::Waiting { .. }
    ));
    assert_eq!(changed.requests(), vec![("0".to_string(), 2, 2)]);
    let retired = get_run(&conn, "lib-1")
        .expect("read retired run")
        .expect("retired run");
    assert_eq!(retired.run_id, partial.run_id);
    assert_eq!(retired.state, ReconciliationState::Failed);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE item_key='CCCC3333'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("rolled-back changed-version item"),
        0
    );

    conn.execute(
        "UPDATE processing_tasks SET next_retry_at=0 WHERE id=?1",
        [&task_id],
    )
    .expect("make fresh attempt due");
    let fresh = Arc::new(FakeSource::new(vec![ScriptStep::Page(page_at_version(
        vec![item("DDDD4444", 5)],
        Some(1),
        100,
    ))]));
    let mut fresh_registry = ExecutorRegistry::new();
    fresh_registry.register(Arc::new(executor(Arc::clone(&fresh))));
    assert!(matches!(
        run_one(
            &conn,
            &ctx_of(&dir),
            &fresh_registry,
            "bib-session-3",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("fresh attempt"),
        RunOneOutcome::Succeeded { .. }
    ));
    assert_eq!(fresh.requests(), vec![("0".to_string(), 0, 2)]);
    let completed = get_run(&conn, "lib-1")
        .expect("read completed run")
        .expect("completed run");
    assert_ne!(completed.run_id, partial.run_id);
    assert_eq!(completed.state, ReconciliationState::Completed);
    assert_eq!(completed.cursor_start, 1);
    assert_eq!(completed.target_version, Some(100));
}

/// A connection-identity revision cannot inherit an active run's cursor or
/// seen-set. Scheduler convergence retires the foreign fence atomically and
/// begins a fresh run for the new identity.
#[test]
fn a_changed_connection_fence_restarts_an_active_run_from_zero() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");

    let first = Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(3),
        )),
        ScriptStep::Fail(ZoteroState::Timeout),
    ]));
    let mut first_registry = ExecutorRegistry::new();
    first_registry.register(Arc::new(executor(first)));
    assert!(matches!(
        run_one(
            &conn,
            &ctx_of(&dir),
            &first_registry,
            "bib-session-1",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("partial attempt"),
        RunOneOutcome::Waiting { .. }
    ));
    let old = get_run(&conn, "lib-1")
        .expect("read old run")
        .expect("old run");
    assert_eq!(old.cursor_start, 2);

    conn.execute(
        "UPDATE zotero_connections SET revision=revision+1 WHERE id='conn-1'",
        [],
    )
    .expect("change connection identity fence");
    conn.execute(
        "UPDATE processing_tasks SET next_retry_at=0 WHERE id=?1",
        [&task_id],
    )
    .expect("make retry due");

    let fresh = Arc::new(FakeSource::new(vec![ScriptStep::Page(page_at_version(
        vec![item("CCCC3333", 3)],
        Some(1),
        100,
    ))]));
    let mut fresh_registry = ExecutorRegistry::new();
    fresh_registry.register(Arc::new(executor(Arc::clone(&fresh))));
    assert!(matches!(
        run_one(
            &conn,
            &ctx_of(&dir),
            &fresh_registry,
            "bib-session-2",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("new-fence attempt"),
        RunOneOutcome::Succeeded { .. }
    ));
    assert_eq!(fresh.requests(), vec![("0".to_string(), 0, 2)]);
    let completed = get_run(&conn, "lib-1")
        .expect("read completed run")
        .expect("completed run");
    assert_ne!(completed.run_id, old.run_id);
    assert_eq!(completed.connection_revision, old.connection_revision + 1);
    assert_eq!(completed.state, ReconciliationState::Completed);
    assert_eq!(completed.cursor_start, 1);
}

/// The final catalog reconciliation and receipt share the scheduler's success
/// transaction. If the receipt/state update aborts, finalization and inferred
/// tombstones roll back while already committed page data remains.
#[test]
fn finalization_does_not_regress_the_catalog_library_version() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(executor(Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page_at_version(vec![item("AAAA1111", 12)], Some(1), 4)),
    ])))));
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("run one bibliography unit");
    assert!(matches!(outcome, RunOneOutcome::Succeeded { task_id: id } if id == task_id));
    let catalog_pin: (Option<i64>, i64) = conn
        .query_row(
            "SELECT last_modified_version, revision FROM zotero_libraries WHERE id='lib-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("catalog version");
    assert_eq!(catalog_pin, (Some(7), 1));
}

#[test]
fn final_publish_and_receipt_roll_back_together() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let stale = upsert_item(
        &mut conn,
        "lib-1",
        BibliographicItemInput {
            item_key: "STALE000".to_string(),
            item_version: Some(1),
            native_json_snapshot: r#"{"key":"STALE000","version":1}"#.to_string(),
            csl_json_snapshot: r#"{"id":"STALE000","type":"book"}"#.to_string(),
            ..Default::default()
        },
    )
    .expect("seed stale item");
    let task_id = admit_bibliography_task(&conn, "lib-1");
    conn.execute_batch(&format!(
        "CREATE TRIGGER reject_bibliography_receipt
         BEFORE UPDATE OF state ON processing_tasks
         WHEN OLD.id = '{task_id}' AND NEW.state = 'succeeded'
         BEGIN SELECT RAISE(ABORT, 'synthetic receipt failure'); END;"
    ))
    .expect("install receipt failure");

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(executor(Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(vec![item("AAAA1111", 12)], Some(1))),
    ])))));
    let error = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect_err("synthetic receipt write must abort success");
    assert!(error.contains("synthetic receipt failure"), "{error}");

    let (state, receipt): (String, Option<String>) = conn
        .query_row(
            "SELECT state, result_receipt_json FROM processing_tasks WHERE id=?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task after rollback");
    assert_eq!(state, "running");
    assert_eq!(receipt, None);
    let run = get_run(&conn, "lib-1")
        .expect("read run")
        .expect("run after rollback");
    assert_eq!(run.state, ReconciliationState::Running);
    assert_eq!(
        run.cursor_start, 1,
        "page commit survives final publish rollback"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_item_tombstones WHERE item_id=?1",
            [&stale.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("rolled back tombstone"),
        0
    );
}

struct WrongCorpusPublisher;

impl Executor for WrongCorpusPublisher {
    fn kinds(&self) -> &[&str] {
        &["bibliography_sync"]
    }

    fn run(&self, _ctx: &ExecCtx, _task: &repository::ClaimedTask, _stop: &StopFlag) -> ExecResult {
        ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: Some(
                entropia_desktop_lib::processing::scheduler::EngineOutput::Ocr(
                    entropia_desktop_lib::processing::ocr::OcrComputeOutput {
                        text: "must not publish".to_string(),
                        method: "synthetic".to_string(),
                        outcome: "text".to_string(),
                        regions_json: None,
                        blocks_json: None,
                        layout_model: String::new(),
                        image_width: 0,
                        image_height: 0,
                        provider: "synthetic".to_string(),
                        page_count: 1,
                    },
                ),
            ),
            output: ExecOutput::Success {
                outcome: "wrong_publisher".to_string(),
                receipt: "{}".to_string(),
            },
        }
    }
}

#[test]
fn bibliography_task_cannot_pass_through_the_corpus_publisher() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(WrongCorpusPublisher));

    let error = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect_err("bibliography cannot route through OCR publication");
    assert!(error.starts_with("unsupported_subject"), "{error}");
    let (state, receipt): (String, Option<String>) = conn
        .query_row(
            "SELECT state, result_receipt_json FROM processing_tasks WHERE id=?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("unpublished bibliography task");
    assert_eq!(state, "running");
    assert_eq!(receipt, None);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM extractions", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("corpus rows"),
        0
    );
}

/// Corpus admission, claim and commit stay unchanged while publisher routing
/// rejects a bibliography kind/output from a corpus task before mutation.
#[test]
fn corpus_claims_commit_normally_and_reject_bibliography_publish_routing() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");
    // A corpus unit through the ordinary user batch path.
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
        "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
         VALUES ('ocr-a1', 'ocr', 'a1', 'corpus', 'asset', 'a1', 'pending', 1, 1)",
        [],
    )
    .expect("corpus task");
    conn.execute(
        "INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
         VALUES ('b1', 'ocr-a1', 'ocr', 'a1', 'corpus', 'asset', 'a1', 'active')",
        [],
    )
    .expect("corpus link");

    // Corpus claim still works while bibliography work is pending, and each
    // arm claims only its own partition (the queue orders by task id, so a
    // mixed registry claims whichever sorts first — the point is neither
    // arm ever receives the other's row).
    let corpus = repository::claim_next(&conn, "s-ocr", &["ocr"], 100)
        .expect("corpus claim")
        .expect("the corpus unit is claimable");
    assert_eq!(corpus.task_id, "ocr-a1");
    assert_eq!(corpus.domain, "corpus");
    let biblio = repository::claim_next(&conn, "s-biblio", &["bibliography_sync"], 100)
        .expect("bibliography claim")
        .expect("the bibliography unit is claimable");
    assert_eq!(biblio.domain, "bibliography");
    assert_eq!(biblio.subject_kind, "library");
    assert_eq!(biblio.subject_id, "lib-1");

    // A caller cannot smuggle bibliography publication through a corpus
    // lease by lying about the kind. The closure must not run.
    let published = std::cell::Cell::new(false);
    let wrong_route = repository::commit_success_with(
        &conn,
        &corpus.task_id,
        corpus.lease_epoch,
        "bibliography_sync",
        "enumerated",
        "{}",
        |_| {
            published.set(true);
            Ok(())
        },
    )
    .expect_err("corpus task must reject bibliography publication");
    assert!(
        wrong_route.starts_with("unsupported_subject"),
        "{wrong_route}"
    );
    assert!(!published.get());

    // The documentary route remains byte-for-byte usable after that rejection.
    repository::commit_success_with(
        &conn,
        &corpus.task_id,
        corpus.lease_epoch,
        "ocr",
        "text_ready",
        r#"{"kind":"ocr"}"#,
        |_| Ok(()),
    )
    .expect("ordinary corpus commit");
    let corpus_state: String = conn
        .query_row(
            "SELECT state FROM processing_tasks WHERE id=?1",
            [&corpus.task_id],
            |row| row.get(0),
        )
        .expect("corpus task state");
    assert_eq!(corpus_state, "succeeded");

    // The bibliography task remains independently owned and untouched.
    let bibliography_state: String = conn
        .query_row(
            "SELECT state FROM processing_tasks WHERE id=?1",
            [&biblio.task_id],
            |row| row.get(0),
        )
        .expect("bibliography task state");
    assert_eq!(bibliography_state, "running");
}
