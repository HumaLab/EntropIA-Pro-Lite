//! E2b-2: the bibliographic page executor behind the batch queue.
//!
//! These tests drive the real claim → dispatch → execute path with synthetic
//! libraries and fake page sources — never a live Zotero, never private
//! data. They pin three contracts:
//!
//! 1. **Honest states.** A missing internal library row, an offline/timeout
//!    endpoint, a disabled local API, and a malformed answer are four
//!    different verdicts with different durable outcomes. Nothing ever
//!    claims Zotero is closed or not installed.
//! 2. **The stop boundary.** `StopFlag` is observed before each page request
//!    and after the last one; an in-flight request is never preempted; once
//!    stopped, no further request leaves the executor.
//! 3. **No lying success.** This build (E2b-2) has no catalog publisher, so
//!    a fully enumerated library parks `blocked` on
//!    `bibliography_publisher_pending` instead of confirming a receipt.
//!    E2b-3 owns per-page durable transactions and the success receipt.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use entropia_desktop_lib::bibliography::processing::{
    BibliographyPage, BibliographyPageItem, BibliographyPageQuery, BibliographySyncExecutor,
    PageFuture, ZoteroPageSource,
};
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
const MIGRATION_0041_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0041_processing_task_subject_identity.sql"
);
const MIGRATION_0042_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0042_processing_task_subject_cutover.sql"
);
const MIGRATION_0043_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0043_bibliography_sync_tasks.sql");

/// Archive shape good enough for both claim arms: the corpus tables the
/// eligibility validator reads, plus the processing migrations, plus the
/// synthetic `zotero_libraries` table E2b-1 admission already used.
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
    conn.execute_batch(
        "CREATE TABLE zotero_libraries (
           id TEXT PRIMARY KEY,
           connection_id TEXT NOT NULL,
           library_type TEXT NOT NULL CHECK(library_type IN ('user', 'group')),
           library_id TEXT NOT NULL,
           name TEXT NOT NULL,
           last_modified_version INTEGER,
           revision INTEGER NOT NULL DEFAULT 0,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         )",
    )
    .expect("synthetic zotero_libraries");
    (dir, conn)
}

fn seed_library(conn: &rusqlite::Connection, row_id: &str, version: Option<i64>) {
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

fn page(items: Vec<BibliographyPageItem>, total: Option<u64>) -> BibliographyPage {
    BibliographyPage {
        library_version: Some(99),
        total,
        items,
    }
}

fn item(key: &str, version: u64) -> BibliographyPageItem {
    // The fake builds the same shape the production parser emits: the
    // original native row serialized in full beside its key, version and
    // CSL text.
    let native_row = serde_json::json!({
        "key": key,
        "version": version,
        "csljson": format!(r#"{{"id":"{key}","title":"Work {key}"}}"#),
        "data": { "itemType": "book", "title": format!("Work {key}") }
    });
    BibliographyPageItem {
        key: key.to_string(),
        item_version: version,
        csl_json: format!(r#"{{"id":"{key}","title":"Work {key}"}}"#),
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

/// Items-first sequential enumeration: every request is a bounded
/// `/items/top` page over the library the internal row resolves to, starts
/// advance without overlap, and a completed enumeration parks honestly
/// instead of confirming a receipt this build cannot make durable.
#[test]
fn pages_are_enumerated_sequentially_and_completion_defers_to_e2b3() {
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

    // Same library identity on every request, starts 0, 2, 4, bounded
    // limit, one page at a time in order.
    assert_eq!(
        fake.requests(),
        vec![
            ("0".to_string(), 0, 2),
            ("0".to_string(), 2, 2),
            ("0".to_string(), 4, 2),
        ]
    );

    // The enumeration is honest about what this build can do: every page was
    // read, but there is no publisher to make the read durable, so the task
    // parks blocked instead of receiving a lying success receipt.
    assert!(
        matches!(&result.output, ExecOutput::Blocked { code, .. }
            if code == "bibliography_publisher_pending"),
        "honest publisher-pending verdict, got {:?}",
        result.output
    );
    assert_eq!(result.checkpoints.len(), 3);
    assert_eq!(result.checkpoints[0].unit_key, "page:0");
    assert_eq!(result.checkpoints[2].unit_key, "page:4");
    assert_eq!(result.progress_total, Some(6));

    // E2b-3's catalog upsert reads the staged pages: the checkpoint payload
    // must retain each item's lossless native Zotero JSON snapshot next to
    // its CSL text, not a reconstructed subset.
    let staged: serde_json::Value =
        serde_json::from_str(&result.checkpoints[0].payload).expect("staged page JSON");
    let staged_items = staged["items"].as_array().expect("staged items");
    assert_eq!(staged_items.len(), 2);
    for (staged_item, (expected_key, expected_version)) in staged_items
        .iter()
        .zip([("AAAA1111", 12u64), ("BBBB2222", 40u64)])
    {
        assert_eq!(staged_item["key"].as_str(), Some(expected_key));
        let snapshot = staged_item["native_json_snapshot"]
            .as_str()
            .expect("snapshot travels with the staged page");
        let snapshot: serde_json::Value =
            serde_json::from_str(snapshot).expect("snapshot is valid JSON");
        assert_eq!(snapshot["key"].as_str(), Some(expected_key));
        assert_eq!(snapshot["version"].as_u64(), Some(expected_version));
        assert!(
            snapshot["data"].is_object(),
            "the uninterpreted native fields survive: {snapshot}"
        );
    }
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
    assert!(matches!(result.output, ExecOutput::Stopped));

    // Two confirmed pages survive as checkpoint units for the resume: the
    // request that was in flight when demand vanished completed and staged,
    // and no third request exists.
    assert_eq!(result.checkpoints.len(), 2);
    assert_eq!(result.checkpoints[0].unit_key, "page:0");
    assert_eq!(result.checkpoints[1].unit_key, "page:2");
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
    assert!(matches!(result.output, ExecOutput::Stopped));
    assert_eq!(
        result.checkpoints.len(),
        2,
        "all pages confirmed before stopping"
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
        let (result, _) = run_directly(&dir, &conn, Arc::clone(&fake), &StopFlag::new());
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
                assert!(matches!(result.output, ExecOutput::Retryable { .. }));
            }
            Verdict::Blocked => {
                assert!(matches!(result.output, ExecOutput::Blocked { .. }));
            }
            Verdict::Fatal => {
                assert!(matches!(result.output, ExecOutput::Fatal { .. }));
            }
        }
        assert!(
            !message.to_lowercase().contains("closed")
                && !message.to_lowercase().contains("not installed"),
            "no message may claim Zotero is closed or not installed: {message}"
        );
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
    let (result, _) = run_directly(&dir, &conn, Arc::clone(&fake), &StopFlag::new());
    assert_eq!(fake.requests().len(), 1);
    assert!(
        matches!(&result.output, ExecOutput::Retryable { code, .. }
            if code == "zotero_invalid_response"),
        "inconsistent empty must be a retryable malformed answer, got {:?}",
        result.output
    );
    assert!(result.checkpoints.is_empty());
}

/// The registry dispatches a claimed bibliography_sync task to the
/// bibliographic executor and parks it blocked on the honest E2b-2 code,
/// with the confirmed page checkpoint durable in the queue's own storage.
#[test]
fn the_registry_dispatches_bibliography_sync_and_parks_the_completion() {
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
    assert!(matches!(outcome, RunOneOutcome::Blocked { task_id } if task_id == task_id));

    let (state, code, checkpoint_count): (String, String, i64) = conn
        .query_row(
            "SELECT t.state, COALESCE(t.last_error_code, ''), (
                SELECT COUNT(*) FROM processing_checkpoints c WHERE c.task_id = t.id
             )
             FROM processing_tasks t
             WHERE t.id = ?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("settled task");
    assert_eq!(state, "blocked");
    assert_eq!(code, "bibliography_publisher_pending");
    assert_eq!(checkpoint_count, 1, "the confirmed page survives durably");
}

/// Corpus admission, claim and commit keep working unchanged while the
/// bibliographic arm exists: the two partitions never borrow each other's
/// work. A commit attempt on a bibliography task still rejects honestly —
/// the E2b-3 publisher is what will make it legal.
#[test]
fn corpus_claims_are_unchanged_and_bibliography_commit_still_rejects() {
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

    // And the bibliographic task commits nowhere until E2b-3: the commit
    // path rejects the domain honestly instead of publishing nothing.
    let committed = repository::commit_success_with(
        &conn,
        &biblio.task_id,
        biblio.lease_epoch,
        "bibliography_sync",
        "enumerated",
        "{}",
        |_| Ok(()),
    );
    assert!(
        committed.unwrap_err().starts_with("unsupported_subject"),
        "commit must reject bibliography until the E2b-3 publisher lands"
    );
}
