//! E2b: durable bibliographic synchronization behind the batch queue.
//!
//! These tests drive manual admission and the real claim → dispatch → execute
//! → publish path with synthetic libraries and fake page sources — never a
//! live Zotero or private data. They pin shared demand, page-transaction
//! atomicity, retry/restart convergence, trusted finalization, scheduler
//! publication, and honest stop/error states.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use entropia_desktop_lib::bibliography::processing::{
    BibliographyPage, BibliographyPageItem, BibliographyPageQuery, BibliographySyncExecutor,
    PageFuture, ZoteroPageSource,
};
use entropia_desktop_lib::bibliography::reconciliation::{
    begin_run, checkpoint_page, get_run, BeginReconciliationInput, ReconciliationEntityKind,
    ReconciliationPageInput, ReconciliationPhase, ReconciliationRunRef, ReconciliationSeenInput,
    ReconciliationState,
};
use entropia_desktop_lib::bibliography::repository::{upsert_item, BibliographicItemInput};
use entropia_desktop_lib::processing::commands::apply_bibliography_sync_request;
use entropia_desktop_lib::processing::ocr::OcrComputeOutput;
use entropia_desktop_lib::processing::repository::{self, BatchAction, NewCheckpoint, TaskSubject};
use entropia_desktop_lib::processing::scheduler::{
    run_one, EngineOutput, ExecCtx, ExecOutput, ExecResult, Executor, ExecutorRegistry,
    RunOneOutcome, StopFlag,
};
use entropia_desktop_lib::writing::zotero::{Library, ZoteroState};
use sha2::{Digest, Sha256};

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
const MIGRATION_0044_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0044_processing_priority.sql");
const MIGRATION_0045_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0045_bibliographic_semantic_profiles.sql"
);
const MIGRATION_0046_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0046_bibliography_profile_tasks.sql");

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
        (MIGRATION_0044_SQL, "0044_processing_priority"),
        (MIGRATION_0045_SQL, "0045_bibliographic_semantic_profiles"),
        (MIGRATION_0046_SQL, "0046_bibliography_profile_tasks"),
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

/// E2b-4 RED: the manual boundary resolves the selected external namespace to
/// exactly one internal library row. Repeated demand shares its live physical
/// task; missing or cross-connection ambiguous namespaces fail without work.
#[test]
fn manual_demand_resolves_one_library_namespace_and_shares_the_live_task() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));

    let unknown = repository::admit_bibliography_sync_demand(&conn, "group", "404")
        .expect_err("an unknown library must not be scheduled");
    assert!(unknown.contains("unknown_library"), "{unknown}");

    let first = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect("first manual demand");
    let duplicate = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect("duplicate manual demand");
    assert!(first.created);
    assert!(!first.requeued);
    assert!(!duplicate.created);
    assert!(!duplicate.requeued);
    assert_eq!(duplicate.batch_id, first.batch_id);
    assert_eq!(duplicate.task_id, first.task_id);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks
             WHERE domain='bibliography' AND subject_kind='library'
               AND subject_id='lib-1' AND kind='bibliography_sync'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("physical task count"),
        1
    );

    conn.execute(
        "UPDATE processing_tasks
            SET state='blocked', outcome='zotero_api_disabled',
                last_error_code='zotero_api_disabled', last_error_message='enable the API'
          WHERE id=?1",
        [&first.task_id],
    )
    .expect("simulate a human-fixable block");
    let unblocked = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect("fresh manual demand after intervention");
    assert_eq!(unblocked.task_id, first.task_id);
    assert!(unblocked.requeued);
    assert_eq!(
        conn.query_row(
            "SELECT state, outcome, last_error_code FROM processing_tasks WHERE id=?1",
            [&first.task_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            )),
        )
        .expect("explicitly requeued block"),
        ("pending".to_string(), "".to_string(), None)
    );

    conn.execute(
        "INSERT INTO zotero_connections
           (id, source_origin, source_instance_id, endpoint, capabilities_json,
            state, revision, created_at, updated_at)
         VALUES ('conn-2', 'local', 'instance-2', 'http://synthetic-2.invalid', '{}',
                 'available', 0, 1, 1)",
        [],
    )
    .expect("second connection");
    conn.execute(
        "INSERT INTO zotero_libraries
           (id, connection_id, library_type, library_id, name,
            last_modified_version, revision, created_at, updated_at)
         VALUES ('lib-2', 'conn-2', 'user', '0', 'Other personal', 9, 1, 1, 1)",
        [],
    )
    .expect("ambiguous external namespace");

    let ambiguous = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect_err("an ambiguous library must not choose a connection");
    assert!(ambiguous.contains("ambiguous_library"), "{ambiguous}");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE kind='bibliography_sync'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("no ambiguous admission"),
        1
    );
}

/// E2b-4 RED: the command core records one durable answer per request id.
/// Replaying a lost response is observational only; a genuinely new demand
/// requeues the interrupted shared task instead of creating another writer.
#[test]
fn manual_request_is_idempotent_while_new_demand_requeues_shared_work() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));

    let first = apply_bibliography_sync_request(&conn, "sync-request-1", "user", "0")
        .expect("first request");
    conn.execute(
        "UPDATE processing_tasks SET state='interrupted' WHERE id=?1",
        [&first.task_id],
    )
    .expect("simulate recovered task");

    let replay = apply_bibliography_sync_request(&conn, "sync-request-1", "user", "0")
        .expect("lost-response replay");
    assert_eq!(replay.task_id, first.task_id);
    assert_eq!(replay.created, first.created);
    assert_eq!(replay.requeued, first.requeued);
    assert_eq!(
        conn.query_row(
            "SELECT state FROM processing_tasks WHERE id=?1",
            [&first.task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("replay leaves state alone"),
        "interrupted"
    );

    let conflict = apply_bibliography_sync_request(&conn, "sync-request-1", "group", "404")
        .expect_err("one request id cannot select another namespace");
    assert!(conflict.contains("invalid_selection"), "{conflict}");

    let resumed = apply_bibliography_sync_request(&conn, "sync-request-2", "user", "0")
        .expect("new manual demand");
    assert_eq!(resumed.task_id, first.task_id);
    assert!(!resumed.created);
    assert!(resumed.requeued);
    assert_eq!(
        conn.query_row(
            "SELECT state FROM processing_tasks WHERE id=?1",
            [&first.task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("new demand requeues"),
        "pending"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM processing_requests", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("durable request count"),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE kind='bibliography_sync'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("shared physical task"),
        1
    );
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

#[test]
fn batch_task_reads_surface_the_confirmed_bibliography_cursor_and_remote_total() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    let fake = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item("AAAA1111", 12)],
        Some(1),
    ))]));

    let (result, _) = run_directly(&dir, &conn, fake, &StopFlag::new());
    assert!(matches!(&result.output, ExecOutput::Success { .. }));
    let batch_id: String = conn
        .query_row(
            "SELECT batch_id FROM processing_batch_tasks WHERE task_id=?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("bibliography batch id");

    let snapshot = repository::read_batch_snapshot(&conn, &batch_id).expect("batch snapshot");
    assert_eq!(
        snapshot.progress_done, 0,
        "bibliography page counts are not OCR/embedding units"
    );
    assert_eq!(
        snapshot.progress_total, None,
        "remote item totals are not OCR/embedding unit totals"
    );
    assert_eq!(snapshot.progress_unknown_tasks, 1);

    let (tasks, next) = repository::list_tasks(&conn, &batch_id, None, None, None, 50)
        .expect("bibliography task list");
    assert!(next.is_none());
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].items_seen, Some(1));
    assert_eq!(tasks[0].remote_total, Some(1));

    let detail = repository::read_task_detail(&conn, &batch_id, &task_id, 10)
        .expect("bibliography task detail");
    assert_eq!(detail.items_seen, Some(1));
    assert_eq!(detail.remote_total, Some(1));
}

#[test]
fn batch_task_reads_keep_an_unknown_bibliography_total_explicit() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    let fake = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item("AAAA1111", 12)],
        None,
    ))]));

    let (result, _) = run_directly(&dir, &conn, fake, &StopFlag::new());
    assert!(matches!(&result.output, ExecOutput::Success { .. }));
    let page_zero_created_at: i64 = conn
        .query_row(
            "SELECT created_at FROM processing_checkpoints WHERE task_id=?1 AND unit_key='page:0'",
            [&task_id],
            |row| row.get(0),
        )
        .expect("current first-page checkpoint");
    let stale_payload = r#"{"next_start":2,"total":99,"items":[]}"#;
    let stale_checksum = format!("{:x}", Sha256::digest(stale_payload.as_bytes()));
    conn.execute(
        "INSERT INTO processing_checkpoints
            (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
         SELECT id, 'page:1', input_fingerprint, contract_hash, ?2, ?3, ?4
         FROM processing_tasks WHERE id=?1",
        rusqlite::params![
            &task_id,
            stale_payload,
            stale_checksum,
            page_zero_created_at - 1
        ],
    )
    .expect("stale checkpoint from a prior run");
    let batch_id: String = conn
        .query_row(
            "SELECT batch_id FROM processing_batch_tasks WHERE task_id=?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("bibliography batch id");
    let (tasks, _) = repository::list_tasks(&conn, &batch_id, None, None, None, 50)
        .expect("bibliography task list");

    assert_eq!(tasks[0].items_seen, Some(1));
    assert_eq!(tasks[0].remote_total, None);
    assert_eq!(
        tasks[0].progress_total, 0,
        "the legacy task sentinel stays unchanged while the read model reports unknown"
    );
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
    let batch_id: String = conn
        .query_row(
            "SELECT batch_id FROM processing_batch_tasks WHERE task_id=?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("bibliography batch id");
    let (tasks, _) = repository::list_tasks(&conn, &batch_id, None, None, None, 50)
        .expect("completed bibliography task list");
    assert_eq!(tasks[0].items_seen, Some(1));
    assert_eq!(tasks[0].remote_total, Some(1));
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

/// E2b-4 RED: an explicit manual retry reopens the failed scheduler task
/// instead of minting a second writer. The failed page is fetched again from
/// the committed cursor, while prior seen keys survive and deletions wait for
/// trusted finalization.
#[test]
fn explicit_manual_retry_reuses_the_failed_task_and_the_committed_page_cursor() {
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
    let initial = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect("initial manual demand");

    let first = Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(3),
        )),
        ScriptStep::Fail(ZoteroState::NotFound),
    ]));
    let mut first_registry = ExecutorRegistry::new();
    first_registry.register(Arc::new(executor(Arc::clone(&first))));
    let first_outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &first_registry,
        "bib-failed-page-1",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("failed page attempt");
    assert!(matches!(first_outcome, RunOneOutcome::Failed { .. }));
    assert_eq!(
        first.requests(),
        vec![("0".to_string(), 0, 2), ("0".to_string(), 2, 2)]
    );
    let failed_run = get_run(&conn, "lib-1")
        .expect("read failed run")
        .expect("failed run exists");
    assert_eq!(failed_run.state, ReconciliationState::Failed);
    assert_eq!(failed_run.cursor_start, 2);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen
             WHERE library_id='lib-1' AND run_id=?1",
            [&failed_run.run_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("committed seen keys"),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_item_tombstones WHERE item_id=?1",
            [&stale.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("no partial tombstone"),
        0
    );

    let retried = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect("explicit failed-task retry");
    assert_eq!(retried.task_id, initial.task_id);
    assert!(!retried.created);
    assert!(retried.requeued);
    assert_eq!(
        conn.query_row(
            "SELECT state FROM processing_tasks WHERE id=?1",
            [&initial.task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("retried task state"),
        "pending"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks
             WHERE domain='bibliography' AND subject_kind='library'
               AND subject_id='lib-1' AND kind='bibliography_sync'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("one physical retry task"),
        1
    );

    let second = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item("CCCC3333", 3)],
        Some(3),
    ))]));
    let mut second_registry = ExecutorRegistry::new();
    second_registry.register(Arc::new(executor(Arc::clone(&second))));
    let second_outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &second_registry,
        "bib-failed-page-2",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("retried page attempt");
    assert!(matches!(second_outcome, RunOneOutcome::Succeeded { .. }));
    assert_eq!(
        second.requests(),
        vec![("0".to_string(), 2, 2)],
        "the failed page must be requested again without replaying page zero"
    );
    let completed = get_run(&conn, "lib-1")
        .expect("read completed run")
        .expect("completed run exists");
    assert_eq!(completed.run_id, failed_run.run_id);
    assert_eq!(completed.state, ReconciliationState::Completed);
    assert_eq!(completed.cursor_start, 3);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen
             WHERE library_id='lib-1' AND run_id=?1",
            [&completed.run_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("preserved and completed seen-set"),
        3
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_item_tombstones WHERE item_id=?1",
            [&stale.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("trusted finalization tombstone"),
        1
    );
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

fn seed_corpus_asset(conn: &rusqlite::Connection, asset_id: &str) {
    let collection_id = format!("collection-{asset_id}");
    let item_id = format!("item-{asset_id}");
    conn.execute(
        "INSERT INTO collections (id, name, created_at, updated_at)
         VALUES (?1, 'Synthetic corpus', 1, 1)",
        [&collection_id],
    )
    .expect("seed corpus collection");
    conn.execute(
        "INSERT INTO items (id, title, collection_id, created_at, updated_at)
         VALUES (?1, 'Synthetic document', ?2, 1, 1)",
        rusqlite::params![&item_id, &collection_id],
    )
    .expect("seed corpus item");
    conn.execute(
        "INSERT INTO assets (id, item_id, path, type, size, created_at)
         VALUES (?1, ?2, ?3, 'image', 10, 1)",
        rusqlite::params![asset_id, &item_id, format!("{asset_id}.png")],
    )
    .expect("seed corpus asset");
}

fn admit_ocr_task(conn: &rusqlite::Connection, asset_id: &str) -> String {
    let batch = repository::ensure_system_batch(conn, "manual").expect("manual system batch");
    repository::admit_subject_or_attach(
        conn,
        &batch,
        "ocr",
        &TaskSubject::corpus_asset(asset_id),
        0,
        "",
        "ocr:light",
        None,
    )
    .expect("admit corpus OCR task")
    .task_id
}

struct SyntheticOcrExecutor {
    text: &'static str,
}

impl Executor for SyntheticOcrExecutor {
    fn kinds(&self) -> &[&str] {
        &["ocr"]
    }

    fn run(&self, _ctx: &ExecCtx, _task: &repository::ClaimedTask, _stop: &StopFlag) -> ExecResult {
        ExecResult {
            checkpoints: Vec::new(),
            progress_total: Some(1),
            engine_output: Some(EngineOutput::Ocr(OcrComputeOutput {
                text: self.text.to_string(),
                method: "synthetic_ocr".to_string(),
                outcome: "text".to_string(),
                regions_json: None,
                blocks_json: None,
                layout_model: String::new(),
                image_width: 0,
                image_height: 0,
                provider: "synthetic".to_string(),
                page_count: 1,
            })),
            output: ExecOutput::Success {
                outcome: "text".to_string(),
                receipt: r#"{"kind":"synthetic_ocr"}"#.to_string(),
            },
        }
    }
}

fn succeeded_task_ids(outcomes: [RunOneOutcome; 2]) -> Vec<String> {
    let mut task_ids = outcomes
        .into_iter()
        .map(|outcome| match outcome {
            RunOneOutcome::Succeeded { task_id } => task_id,
            other => panic!("each mixed-registry run must succeed, got {other:?}"),
        })
        .collect::<Vec<_>>();
    task_ids.sort();
    task_ids
}

/// E2b-5-WU1: one serial scheduler drains both registered domains and routes
/// each successful output exclusively to its canonical publisher.
#[test]
fn e2b5_wu1_mixed_registry_publishes_each_domain_to_its_canonical_tables() {
    const ASSET_ID: &str = "corpus-asset-1";
    const LIBRARY_ID: &str = "zotero-library-1";
    const OCR_TEXT: &str = "OCR output belongs to the corpus asset only";

    let (dir, conn) = migrated_db();
    seed_corpus_asset(&conn, ASSET_ID);
    seed_library(&conn, LIBRARY_ID, Some(7));
    let ocr_task_id = admit_ocr_task(&conn, ASSET_ID);
    let bibliography_task_id = admit_bibliography_task(&conn, LIBRARY_ID);

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(SyntheticOcrExecutor { text: OCR_TEXT }));
    registry.register(Arc::new(executor(Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(vec![item("BIBLIO001", 11)], Some(1))),
    ])))));
    let mut kinds = registry.kinds();
    kinds.sort();
    assert_eq!(
        kinds,
        vec!["bibliography_sync".to_string(), "ocr".to_string()]
    );

    let first = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "mixed-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("first mixed-registry run");
    let second = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "mixed-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("second mixed-registry run");

    let mut expected_task_ids = vec![ocr_task_id.clone(), bibliography_task_id.clone()];
    expected_task_ids.sort();
    assert_eq!(succeeded_task_ids([first, second]), expected_task_ids);
    for task_id in [&ocr_task_id, &bibliography_task_id] {
        assert_eq!(
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id=?1",
                [task_id],
                |row| row.get::<_, String>(0),
            )
            .expect("terminal task state"),
            "succeeded"
        );
    }

    let extraction: (String, String) = conn
        .query_row(
            "SELECT text_content, method FROM extractions WHERE asset_id=?1",
            [ASSET_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("published corpus extraction");
    assert_eq!(
        extraction,
        (OCR_TEXT.to_string(), "synthetic_ocr".to_string())
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM extractions WHERE asset_id=?1",
            [LIBRARY_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("no bibliography extraction"),
        0,
        "bibliography output must not enter the corpus publisher"
    );

    let catalog_item: (String, String) = conn
        .query_row(
            "SELECT item_key, title FROM bibliographic_items WHERE library_id=?1",
            [LIBRARY_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("published bibliography item");
    assert_eq!(
        catalog_item,
        ("BIBLIO001".to_string(), "Work BIBLIO001".to_string())
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id=?1",
            [ASSET_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("no corpus catalog rows"),
        0,
        "OCR output must not enter the bibliography publisher"
    );
    let reconciliation = get_run(&conn, LIBRARY_ID)
        .expect("read bibliography reconciliation")
        .expect("bibliography reconciliation exists");
    assert_eq!(reconciliation.state, ReconciliationState::Completed);
    assert_eq!(reconciliation.cursor_start, 1);
    assert!(
        get_run(&conn, ASSET_ID)
            .expect("read absent corpus reconciliation")
            .is_none(),
        "corpus publication must not create bibliography reconciliation"
    );
}

/// E2b-5-WU1: identical opaque strings in separate subject domains are not a
/// single-flight key. Both physical tasks execute and each output keeps its
/// own publisher even though the asset and library row share the same id.
#[test]
fn e2b5_wu1_equal_string_subjects_remain_distinct_and_route_by_domain() {
    const SHARED_ID: &str = "shared-subject-identity";
    const OCR_TEXT: &str = "OCR_ONLY_MARKER";

    let (dir, conn) = migrated_db();
    seed_corpus_asset(&conn, SHARED_ID);
    seed_library(&conn, SHARED_ID, Some(7));
    let ocr_task_id = admit_ocr_task(&conn, SHARED_ID);
    let bibliography_task_id = admit_bibliography_task(&conn, SHARED_ID);

    assert_ne!(
        ocr_task_id, bibliography_task_id,
        "equal subject strings in different domains need separate physical tasks"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE subject_id=?1",
            [SHARED_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("same-string task count"),
        2,
        "composite subject identity must prevent a cross-domain single-flight collision"
    );
    let ocr_identity: (String, String, String) = conn
        .query_row(
            "SELECT domain, subject_kind, kind FROM processing_tasks WHERE id=?1",
            [&ocr_task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("OCR task identity");
    assert_eq!(
        ocr_identity,
        ("corpus".to_string(), "asset".to_string(), "ocr".to_string())
    );
    let bibliography_identity: (String, String, String) = conn
        .query_row(
            "SELECT domain, subject_kind, kind FROM processing_tasks WHERE id=?1",
            [&bibliography_task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("bibliography task identity");
    assert_eq!(
        bibliography_identity,
        (
            "bibliography".to_string(),
            "library".to_string(),
            "bibliography_sync".to_string(),
        )
    );

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(SyntheticOcrExecutor { text: OCR_TEXT }));
    registry.register(Arc::new(executor(Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(vec![item("BIBONLY1", 13)], Some(1))),
    ])))));
    let committed_routes = Mutex::new(Vec::<(String, String)>::new());
    let on_commit = |task: &repository::ClaimedTask, output: &EngineOutput| {
        let output_domain = match output {
            EngineOutput::Ocr(_) => "ocr",
            EngineOutput::Bibliography(_) => "bibliography",
            EngineOutput::Embedding(_) => "embedding",
            EngineOutput::BibliographyProfile(_) => "bibliography_profile",
        };
        committed_routes
            .lock()
            .expect("commit routes")
            .push((task.domain.clone(), output_domain.to_string()));
    };

    let first = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "same-string-session",
        repository::now_ms(),
        &on_commit,
        &|_, _, _, _| {},
    )
    .expect("first same-string run");
    let second = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "same-string-session",
        repository::now_ms(),
        &on_commit,
        &|_, _, _, _| {},
    )
    .expect("second same-string run");

    let mut expected_task_ids = vec![ocr_task_id.clone(), bibliography_task_id.clone()];
    expected_task_ids.sort();
    assert_eq!(succeeded_task_ids([first, second]), expected_task_ids);
    for task_id in [&ocr_task_id, &bibliography_task_id] {
        assert_eq!(
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id=?1",
                [task_id],
                |row| row.get::<_, String>(0),
            )
            .expect("same-string terminal state"),
            "succeeded"
        );
    }

    let mut routes = committed_routes.lock().expect("committed routes").clone();
    routes.sort();
    assert_eq!(
        routes,
        vec![
            ("bibliography".to_string(), "bibliography".to_string()),
            ("corpus".to_string(), "ocr".to_string()),
        ],
        "each domain must reach only the matching successful publisher"
    );
    assert_eq!(
        conn.query_row(
            "SELECT text_content FROM extractions WHERE asset_id=?1",
            [SHARED_ID],
            |row| row.get::<_, String>(0),
        )
        .expect("same-string OCR extraction"),
        OCR_TEXT
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM extractions WHERE asset_id=?1",
            [SHARED_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("same-string extraction count"),
        1
    );
    let catalog_item: (String, String, String) = conn
        .query_row(
            "SELECT item_key, title, native_json_snapshot
             FROM bibliographic_items WHERE library_id=?1",
            [SHARED_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("same-string bibliography item");
    assert_eq!(catalog_item.0, "BIBONLY1");
    assert_eq!(catalog_item.1, "Work BIBONLY1");
    assert!(
        !catalog_item.2.contains(OCR_TEXT),
        "OCR output must not leak into bibliography snapshots"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id=?1",
            [SHARED_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("same-string bibliography count"),
        1
    );
    let reconciliation = get_run(&conn, SHARED_ID)
        .expect("read same-string reconciliation")
        .expect("same-string reconciliation exists");
    assert_eq!(reconciliation.state, ReconciliationState::Completed);
    assert_eq!(reconciliation.cursor_start, 1);
}

/// E2b-5-WU2: withdrawing the manual corpus batch cancels only its orphaned
/// OCR task; bibliography demand remains runnable through the mixed registry.
#[test]
fn e2b5_wu2_cancelling_corpus_batch_preserves_bibliography_demand() {
    const ASSET_ID: &str = "cancelled-corpus-asset";
    const LIBRARY_ID: &str = "surviving-bibliography-library";

    let (dir, conn) = migrated_db();
    seed_corpus_asset(&conn, ASSET_ID);
    seed_library(&conn, LIBRARY_ID, Some(7));
    let corpus_batch_id =
        repository::ensure_system_batch(&conn, "manual").expect("manual system batch");
    let bibliography_batch_id =
        repository::ensure_system_batch(&conn, "bibliography").expect("bibliography system batch");
    let ocr_task_id = admit_ocr_task(&conn, ASSET_ID);
    let bibliography_task_id = admit_bibliography_task(&conn, LIBRARY_ID);

    assert!(repository::execution_wanted(&conn, &ocr_task_id).expect("initial OCR demand"));
    assert!(repository::execution_wanted(&conn, &bibliography_task_id)
        .expect("initial bibliography demand"));

    repository::control_batch(&conn, &corpus_batch_id, BatchAction::Cancel, None)
        .expect("cancel only the manual corpus batch");

    let cancelled_corpus: (String, String) = conn
        .query_row(
            "SELECT t.state, l.request_state
               FROM processing_tasks t
               JOIN processing_batch_tasks l ON l.task_id = t.id
              WHERE t.id = ?1 AND l.batch_id = ?2",
            rusqlite::params![&ocr_task_id, &corpus_batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("cancelled corpus task and batch link");
    assert_eq!(
        cancelled_corpus,
        ("cancelled".to_string(), "cancelled".to_string()),
        "with no surviving corpus link, the OCR task must be orphan-cancelled"
    );
    assert!(
        !repository::execution_wanted(&conn, &ocr_task_id).expect("withdrawn OCR demand"),
        "the cancelled corpus batch must withdraw only its own demand"
    );

    let surviving_bibliography: (String, String) = conn
        .query_row(
            "SELECT t.state, l.request_state
               FROM processing_tasks t
               JOIN processing_batch_tasks l ON l.task_id = t.id
              WHERE t.id = ?1 AND l.batch_id = ?2",
            rusqlite::params![&bibliography_task_id, &bibliography_batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("surviving bibliography task and batch link");
    assert_eq!(
        surviving_bibliography,
        ("pending".to_string(), "active".to_string()),
        "cancelling the corpus batch must not alter the bibliography link"
    );
    assert!(
        repository::execution_wanted(&conn, &bibliography_task_id)
            .expect("surviving bibliography demand"),
        "bibliography scheduler demand must survive corpus cancellation"
    );

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(SyntheticOcrExecutor {
        text: "cancelled OCR must never publish",
    }));
    registry.register(Arc::new(executor(Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(vec![item("BIBSURV1", 17)], Some(1))),
    ])))));

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "corpus-cancel-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("mixed registry runs surviving bibliography work");
    match outcome {
        RunOneOutcome::Succeeded { task_id } => assert_eq!(task_id, bibliography_task_id),
        other => panic!("surviving bibliography task must succeed, got {other:?}"),
    }

    assert_eq!(
        conn.query_row(
            "SELECT state FROM processing_tasks WHERE id=?1",
            [&bibliography_task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("bibliography terminal state"),
        "succeeded"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM extractions WHERE asset_id=?1",
            [ASSET_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("cancelled OCR extraction count"),
        0,
        "the cancelled corpus task must not publish"
    );
    let catalog_item: (String, String) = conn
        .query_row(
            "SELECT item_key, title FROM bibliographic_items WHERE library_id=?1",
            [LIBRARY_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("surviving bibliography catalog item");
    assert_eq!(
        catalog_item,
        ("BIBSURV1".to_string(), "Work BIBSURV1".to_string())
    );
    let reconciliation = get_run(&conn, LIBRARY_ID)
        .expect("read surviving bibliography reconciliation")
        .expect("surviving bibliography reconciliation exists");
    assert_eq!(reconciliation.state, ReconciliationState::Completed);
    assert_eq!(reconciliation.cursor_start, 1);
}

/// E2b-5-WU2: withdrawing the bibliography system batch cancels only its
/// orphaned sync task; corpus demand remains runnable through the mixed registry.
#[test]
fn e2b5_wu2_cancelling_bibliography_batch_preserves_corpus_demand() {
    const ASSET_ID: &str = "surviving-corpus-asset";
    const LIBRARY_ID: &str = "cancelled-bibliography-library";
    const OCR_TEXT: &str = "surviving OCR output";

    let (dir, conn) = migrated_db();
    seed_corpus_asset(&conn, ASSET_ID);
    seed_library(&conn, LIBRARY_ID, Some(7));
    let corpus_batch_id =
        repository::ensure_system_batch(&conn, "manual").expect("manual system batch");
    let bibliography_batch_id =
        repository::ensure_system_batch(&conn, "bibliography").expect("bibliography system batch");
    let ocr_task_id = admit_ocr_task(&conn, ASSET_ID);
    let bibliography_task_id = admit_bibliography_task(&conn, LIBRARY_ID);

    assert!(repository::execution_wanted(&conn, &ocr_task_id).expect("initial OCR demand"));
    assert!(repository::execution_wanted(&conn, &bibliography_task_id)
        .expect("initial bibliography demand"));

    repository::control_batch(&conn, &bibliography_batch_id, BatchAction::Cancel, None)
        .expect("cancel only the bibliography system batch");

    let cancelled_bibliography: (String, String) = conn
        .query_row(
            "SELECT t.state, l.request_state
               FROM processing_tasks t
               JOIN processing_batch_tasks l ON l.task_id = t.id
              WHERE t.id = ?1 AND l.batch_id = ?2",
            rusqlite::params![&bibliography_task_id, &bibliography_batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("cancelled bibliography task and batch link");
    assert_eq!(
        cancelled_bibliography,
        ("cancelled".to_string(), "cancelled".to_string()),
        "with no surviving bibliography link, the sync task must be orphan-cancelled"
    );
    assert!(
        !repository::execution_wanted(&conn, &bibliography_task_id)
            .expect("withdrawn bibliography demand"),
        "the cancelled bibliography batch must withdraw only its own demand"
    );

    let surviving_corpus: (String, String) = conn
        .query_row(
            "SELECT t.state, l.request_state
               FROM processing_tasks t
               JOIN processing_batch_tasks l ON l.task_id = t.id
              WHERE t.id = ?1 AND l.batch_id = ?2",
            rusqlite::params![&ocr_task_id, &corpus_batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("surviving corpus task and batch link");
    assert_eq!(
        surviving_corpus,
        ("pending".to_string(), "active".to_string()),
        "cancelling the bibliography batch must not alter the corpus link"
    );
    assert!(
        repository::execution_wanted(&conn, &ocr_task_id).expect("surviving OCR demand"),
        "corpus scheduler demand must survive bibliography cancellation"
    );

    let fake_source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item("CANCELB1", 23)],
        Some(1),
    ))]));
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(SyntheticOcrExecutor { text: OCR_TEXT }));
    registry.register(Arc::new(executor(Arc::clone(&fake_source))));

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bibliography-cancel-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("mixed registry runs surviving corpus work");
    match outcome {
        RunOneOutcome::Succeeded { task_id } => assert_eq!(task_id, ocr_task_id),
        other => panic!("surviving OCR task must succeed, got {other:?}"),
    }

    assert_eq!(
        conn.query_row(
            "SELECT state FROM processing_tasks WHERE id=?1",
            [&ocr_task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("OCR terminal state"),
        "succeeded"
    );
    let extraction: (String, String) = conn
        .query_row(
            "SELECT text_content, method FROM extractions WHERE asset_id=?1",
            [ASSET_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("surviving OCR extraction");
    assert_eq!(
        extraction,
        (OCR_TEXT.to_string(), "synthetic_ocr".to_string())
    );
    assert!(
        fake_source.requests().is_empty(),
        "the cancelled bibliography task must not execute"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id=?1",
            [LIBRARY_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("cancelled bibliography catalog count"),
        0,
        "the cancelled bibliography task must not publish"
    );
}

/// E2b-5-WU3: startup recovery parks both domains without discarding their
/// committed progress. Explicit user and bibliography demand resume the same
/// physical tasks, and the mixed scheduler converges them exactly once.
#[test]
fn e2b5_wu3_recovery_resumes_both_domains_without_duplicate_publication() {
    const ASSET_ID: &str = "recovered-corpus-asset";
    const LIBRARY_ID: &str = "recovered-bibliography-library";
    const CORPUS_BATCH_ID: &str = "recovered-user-batch";
    const PRESERVED_ITEM_KEY: &str = "PRE00001";
    const RESUMED_ITEM_KEY: &str = "RES00002";
    const OCR_TEXT: &str = "recovered OCR output";

    let (dir, mut conn) = migrated_db();
    seed_corpus_asset(&conn, ASSET_ID);
    seed_library(&conn, LIBRARY_ID, Some(7));
    conn.execute(
        "INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations,
            planning_done, created_at, updated_at)
         VALUES (?1, 'e2b5-wu3-user-request', 'user', 'running', 'run',
                 '[\"ocr\"]', 1, 1, 1)",
        [CORPUS_BATCH_ID],
    )
    .expect("seed completed user planning snapshot");
    let ocr_task_id = repository::admit_subject_or_attach(
        &conn,
        CORPUS_BATCH_ID,
        "ocr",
        &TaskSubject::corpus_asset(ASSET_ID),
        0,
        "",
        "ocr:light",
        None,
    )
    .expect("admit user-batch OCR task")
    .task_id;
    let initial_bibliography = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect("admit bibliography system demand");
    assert!(initial_bibliography.created);
    assert!(!initial_bibliography.requeued);
    let bibliography_task_id = initial_bibliography.task_id.clone();

    let ocr_claim = repository::claim_next(&conn, "crashed-ocr-session", &["ocr"], 100)
        .expect("claim OCR before restart")
        .expect("OCR task is claimable");
    let bibliography_claim = repository::claim_next(
        &conn,
        "crashed-bibliography-session",
        &["bibliography_sync"],
        100,
    )
    .expect("claim bibliography before restart")
    .expect("bibliography task is claimable");
    assert_eq!(ocr_claim.task_id, ocr_task_id);
    assert_eq!(bibliography_claim.task_id, bibliography_task_id);

    repository::save_checkpoint(
        &conn,
        &ocr_claim.task_id,
        ocr_claim.lease_epoch,
        &NewCheckpoint {
            unit_key: "page:0".to_string(),
            input_fingerprint: ocr_claim.input_fingerprint.clone(),
            contract_hash: ocr_claim.contract_hash.clone(),
            payload: r#"{"text":"committed before restart"}"#.to_string(),
            payload_checksum: "synthetic-ocr-page-0".to_string(),
        },
        110,
    )
    .expect("commit corpus checkpoint before restart");

    let preserved_page_item = item(PRESERVED_ITEM_KEY, 11);
    upsert_item(
        &mut conn,
        LIBRARY_ID,
        BibliographicItemInput {
            item_key: PRESERVED_ITEM_KEY.to_string(),
            item_version: Some(11),
            native_json_snapshot: preserved_page_item.native_json_snapshot,
            csl_json_snapshot: preserved_page_item.csl_json,
            title: Some(format!("Work {PRESERVED_ITEM_KEY}")),
            ..Default::default()
        },
    )
    .expect("seed catalog item from committed bibliography page");
    let initial_run = begin_run(
        &mut conn,
        BeginReconciliationInput {
            library_id: LIBRARY_ID.to_string(),
            connection_revision: 0,
            cursor_start: 0,
            cursor_limit: 2,
            remote_total: Some(2),
            target_version: Some(99),
        },
    )
    .expect("begin active bibliography reconciliation");
    let checkpointed_run = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            run: ReconciliationRunRef {
                library_id: initial_run.library_id.clone(),
                run_id: initial_run.run_id.clone(),
                connection_revision: initial_run.connection_revision,
            },
            phase: ReconciliationPhase::Versions,
            cursor_start: 0,
            next_cursor_start: 1,
            remote_total: Some(2),
            seen: vec![ReconciliationSeenInput {
                entity_kind: ReconciliationEntityKind::Item,
                entity_key: PRESERVED_ITEM_KEY.to_string(),
                parent_key: None,
                remote_version: Some(11),
                observed_at: 115,
            }],
        },
    )
    .expect("checkpoint first bibliography page");
    assert_eq!(checkpointed_run.state, ReconciliationState::Running);
    assert_eq!(checkpointed_run.cursor_start, 1);
    repository::save_checkpoint(
        &conn,
        &bibliography_claim.task_id,
        bibliography_claim.lease_epoch,
        &NewCheckpoint {
            unit_key: "page:0".to_string(),
            input_fingerprint: bibliography_claim.input_fingerprint.clone(),
            contract_hash: bibliography_claim.contract_hash.clone(),
            payload: format!(r#"{{"start":0,"nextStart":1,"itemKey":"{PRESERVED_ITEM_KEY}"}}"#),
            payload_checksum: "synthetic-bibliography-page-0".to_string(),
        },
        120,
    )
    .expect("commit bibliography queue checkpoint before restart");

    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks
              WHERE id IN (?1, ?2) AND state='running'",
            rusqlite::params![&ocr_task_id, &bibliography_task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("running task count before recovery"),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_attempts
              WHERE task_id IN (?1, ?2) AND outcome='open'",
            rusqlite::params![&ocr_task_id, &bibliography_task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("open attempts before recovery"),
        2
    );

    let recovery = entropia_desktop_lib::processing::recovery::recover_session(
        &conn,
        "restarted-session",
        200,
    )
    .expect("recover interrupted scheduler session");
    assert_eq!(recovery.tasks_interrupted, 2);
    assert_eq!(recovery.attempts_closed, 2);
    assert_eq!(recovery.batches_interrupted, 1);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks
              WHERE id IN (?1, ?2) AND state='interrupted'",
            rusqlite::params![&ocr_task_id, &bibliography_task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("recovered task states"),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_attempts
              WHERE task_id IN (?1, ?2) AND outcome='interrupted'
                AND finished_at IS NOT NULL",
            rusqlite::params![&ocr_task_id, &bibliography_task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("recovered attempt states"),
        2
    );
    let recovered_run = get_run(&conn, LIBRARY_ID)
        .expect("read recovered reconciliation")
        .expect("reconciliation survives recovery");
    assert_eq!(recovered_run.run_id, checkpointed_run.run_id);
    assert_eq!(recovered_run.state, ReconciliationState::Interrupted);
    assert_eq!(recovered_run.cursor_start, 1);
    for task_id in [&ocr_task_id, &bibliography_task_id] {
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id=?1",
                [task_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("preserved checkpoint count"),
            1,
            "recovery must retain the committed checkpoint for {task_id}"
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen
              WHERE library_id=?1 AND run_id=?2 AND entity_kind='item'",
            rusqlite::params![LIBRARY_ID, &recovered_run.run_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("preserved bibliography seen row"),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id=?1",
            [LIBRARY_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("preserved bibliography catalog row"),
        1
    );

    repository::control_batch(&conn, CORPUS_BATCH_ID, BatchAction::Resume, None)
        .expect("resume interrupted user corpus batch");
    repository::promote_ready_batches(&conn).expect("promote resumed corpus batch");
    let resumed_bibliography = repository::admit_bibliography_sync_demand(&conn, "user", "0")
        .expect("explicitly resume bibliography demand");
    assert_eq!(resumed_bibliography.task_id, bibliography_task_id);
    assert!(!resumed_bibliography.created);
    assert!(resumed_bibliography.requeued);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks
              WHERE (domain='corpus' AND subject_kind='asset' AND subject_id=?1 AND kind='ocr')
                 OR (domain='bibliography' AND subject_kind='library' AND subject_id=?2
                     AND kind='bibliography_sync')",
            rusqlite::params![ASSET_ID, LIBRARY_ID],
            |row| row.get::<_, i64>(0),
        )
        .expect("same physical tasks after resume"),
        2,
        "resume must reuse both task identities instead of duplicating work"
    );
    for task_id in [&ocr_task_id, &bibliography_task_id] {
        assert_eq!(
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id=?1",
                [task_id],
                |row| row.get::<_, String>(0),
            )
            .expect("resumed task state"),
            "pending"
        );
        assert!(repository::execution_wanted(&conn, task_id).expect("resumed demand"));
    }

    let fake_source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item(RESUMED_ITEM_KEY, 12)],
        Some(2),
    ))]));
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(SyntheticOcrExecutor { text: OCR_TEXT }));
    registry.register(Arc::new(executor(Arc::clone(&fake_source))));

    let first = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "recovered-mixed-session",
        300,
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("run first recovered survivor");
    let second = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "recovered-mixed-session",
        301,
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("run second recovered survivor");
    let mut expected_task_ids = vec![ocr_task_id.clone(), bibliography_task_id.clone()];
    expected_task_ids.sort();
    assert_eq!(succeeded_task_ids([first, second]), expected_task_ids);
    assert_eq!(
        run_one(
            &conn,
            &ctx_of(&dir),
            &registry,
            "recovered-mixed-session",
            302,
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("check drained recovered queue"),
        RunOneOutcome::Idle,
        "a later scheduler pass must find no duplicated or lost work"
    );
    assert_eq!(
        fake_source.requests(),
        vec![("0".to_string(), 1, 2)],
        "the resumed bibliography request must start at the preserved cursor"
    );

    let extraction: (String, String) = conn
        .query_row(
            "SELECT text_content, method FROM extractions WHERE asset_id=?1",
            [ASSET_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("one recovered OCR extraction");
    assert_eq!(
        extraction,
        (OCR_TEXT.to_string(), "synthetic_ocr".to_string())
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM extractions", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("unique extraction count"),
        1
    );

    let catalog_items = {
        let mut statement = conn
            .prepare(
                "SELECT item_key, title FROM bibliographic_items
                  WHERE library_id=?1 ORDER BY item_key",
            )
            .expect("prepare catalog rows");
        statement
            .query_map([LIBRARY_ID], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query catalog rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect catalog rows")
    };
    assert_eq!(
        catalog_items,
        vec![
            (
                PRESERVED_ITEM_KEY.to_string(),
                format!("Work {PRESERVED_ITEM_KEY}"),
            ),
            (
                RESUMED_ITEM_KEY.to_string(),
                format!("Work {RESUMED_ITEM_KEY}"),
            ),
        ]
    );
    let completed_run = get_run(&conn, LIBRARY_ID)
        .expect("read completed reconciliation")
        .expect("completed reconciliation exists");
    assert_eq!(completed_run.run_id, recovered_run.run_id);
    assert_eq!(completed_run.state, ReconciliationState::Completed);
    assert_eq!(completed_run.cursor_start, 2);
    assert_eq!(completed_run.remote_total, Some(2));
    assert!(completed_run.completed_at.is_some());
    let seen_keys = {
        let mut statement = conn
            .prepare(
                "SELECT entity_key FROM zotero_reconciliation_seen
                  WHERE library_id=?1 AND run_id=?2 AND entity_kind='item'
                  ORDER BY entity_key",
            )
            .expect("prepare seen rows");
        statement
            .query_map(
                rusqlite::params![LIBRARY_ID, &completed_run.run_id],
                |row| row.get::<_, String>(0),
            )
            .expect("query seen rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect seen rows")
    };
    assert_eq!(
        seen_keys,
        vec![PRESERVED_ITEM_KEY.to_string(), RESUMED_ITEM_KEY.to_string()]
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id=?1",
            [&ocr_task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("final corpus checkpoint count"),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM processing_checkpoints WHERE task_id=?1",
            [&bibliography_task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("final bibliography checkpoint count"),
        2,
        "the resumed page adds one checkpoint without replaying page zero"
    );

    let (ocr_state, ocr_receipt): (String, Option<String>) = conn
        .query_row(
            "SELECT state, result_receipt_json FROM processing_tasks WHERE id=?1",
            [&ocr_task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("OCR terminal receipt");
    assert_eq!(ocr_state, "succeeded");
    assert_eq!(ocr_receipt.as_deref(), Some(r#"{"kind":"synthetic_ocr"}"#));
    let (bibliography_state, bibliography_receipt): (String, Option<String>) = conn
        .query_row(
            "SELECT state, result_receipt_json FROM processing_tasks WHERE id=?1",
            [&bibliography_task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("bibliography terminal receipt");
    assert_eq!(bibliography_state, "succeeded");
    let bibliography_receipt: serde_json::Value =
        serde_json::from_str(&bibliography_receipt.expect("durable bibliography receipt"))
            .expect("bibliography receipt JSON");
    assert_eq!(
        bibliography_receipt["libraryRowId"].as_str(),
        Some(LIBRARY_ID)
    );
    assert_eq!(bibliography_receipt["itemsSeen"].as_u64(), Some(2));

    for task_id in [&ocr_task_id, &bibliography_task_id] {
        let attempt_outcomes = {
            let mut statement = conn
                .prepare(
                    "SELECT outcome FROM processing_attempts
                      WHERE task_id=?1 ORDER BY attempt_number",
                )
                .expect("prepare attempt outcomes");
            statement
                .query_map([task_id], |row| row.get::<_, String>(0))
                .expect("query attempt outcomes")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect attempt outcomes")
        };
        assert_eq!(
            attempt_outcomes,
            vec!["interrupted".to_string(), "succeeded".to_string()],
            "each survivor must close exactly one recovered attempt and one resumed attempt"
        );
    }
}

/// E2c-WU3: batch priority is global across domains — an interactive
/// bibliography batch outranks a background corpus batch, deterministically
/// regardless of physical id order. No unit is preempted: the survivor
/// claims next.
#[test]
fn interactive_bibliography_batch_outranks_background_corpus() {
    let (_dir, conn) = migrated_db();
    seed_corpus_asset(&conn, "priority-asset");
    seed_library(&conn, "lib-prio", Some(7));
    let ocr_id = admit_ocr_task(&conn, "priority-asset");
    let biblio_id = admit_bibliography_task(&conn, "lib-prio");
    // Roles follow the physical id order so the test is deterministic:
    // the larger id goes interactive, the smaller stays background.
    let (hi_batch, hi_task, bg_task) = if ocr_id < biblio_id {
        (
            "batch-system-bibliography",
            biblio_id.as_str(),
            ocr_id.as_str(),
        )
    } else {
        ("batch-system-manual", ocr_id.as_str(), biblio_id.as_str())
    };
    conn.execute(
        "UPDATE processing_batches SET priority = 2 WHERE id = ?1",
        [hi_batch],
    )
    .expect("raise the interactive batch");

    let first = repository::claim_next(
        &conn,
        "prio-session",
        &["ocr", "bibliography_sync"],
        repository::now_ms(),
    )
    .expect("claim scan")
    .expect("a unit must be runnable");
    assert_eq!(
        first.task_id, hi_task,
        "the interactive batch jumps ahead of the background backlog across domains"
    );
    let second = repository::claim_next(
        &conn,
        "prio-session",
        &["ocr", "bibliography_sync"],
        repository::now_ms(),
    )
    .expect("claim scan")
    .expect("the survivor must be runnable");
    assert_eq!(second.task_id, bg_task);
}

/// Page source that withdraws real scheduler demand while serving one
/// request: cancels its batch on a separate connection before delegating,
/// so the executor page commit that follows observes demand loss exactly
/// where a racing cancellation would land it.
struct CancelDemandWhileServing {
    inner: FakeSource,
    db_path: std::path::PathBuf,
    batch_id: String,
    at_request: usize,
}

impl ZoteroPageSource for CancelDemandWhileServing {
    fn fetch_page(&self, library: &Library, query: BibliographyPageQuery) -> PageFuture {
        let upcoming = self.inner.requests().len() + 1;
        if upcoming == self.at_request {
            let conn = rusqlite::Connection::open(&self.db_path).expect("cancel connection");
            repository::control_batch(&conn, &self.batch_id, BatchAction::Cancel, None)
                .expect("withdraw demand mid-walk");
        }
        self.inner.fetch_page(library, query)
    }
}

/// E2c-WU4: demand withdrawn mid-walk parks Stopped, not Fatal. Cancelling
/// the bibliography batch after the first page commits must not fail the
/// task as a storage error: the confirmed page survives as resume evidence
/// and the reconciliation run is interrupted, never failed.
#[test]
fn demand_withdrawn_mid_walk_parks_stopped_not_fatal() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    let task = claim_bibliography(&conn);
    assert_eq!(task.task_id, task_id);
    // Withdraw the only demand after the first page commits: the source
    // cancels the batch while serving the second request, so the page-2
    // commit below observes demand loss exactly like a racing cancel.
    let fake = Arc::new(CancelDemandWhileServing {
        inner: FakeSource::new(vec![
            ScriptStep::Page(page(
                vec![item("AAAA1111", 12), item("BBBB2222", 40)],
                Some(4),
            )),
            ScriptStep::Page(page(
                vec![item("CCCC3333", 3), item("DDDD4444", 9)],
                Some(4),
            )),
        ]),
        db_path: dir.path().join("entropia.sqlite"),
        batch_id: "batch-system-bibliography".to_string(),
        at_request: 2,
    });
    let result = BibliographySyncExecutor::new(fake).with_page_limit(2).run(
        &ctx_of(&dir),
        &task,
        &StopFlag::new(),
    );
    assert!(
        matches!(&result.output, ExecOutput::Stopped),
        "demand loss mid-walk must park Stopped, got {:?}",
        result.output
    );
    assert_eq!(
        result.checkpoints.len(),
        1,
        "the confirmed page survives as resume evidence"
    );
    assert_eq!(
        result.checkpoints[0].unit_key, "page:0",
        "only the committed page is staged"
    );
    let cataloged: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id = 'lib-1'",
            [],
            |row| row.get(0),
        )
        .expect("catalog count");
    assert_eq!(cataloged, 2, "the committed page stays durable");
    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("run exists");
    assert_eq!(
        run.state,
        ReconciliationState::Interrupted,
        "demand loss interrupts the run instead of failing it"
    );
    let tombstones: i64 = conn
        .query_row("SELECT COUNT(*) FROM zotero_item_tombstones", [], |row| {
            row.get(0)
        })
        .expect("tombstones");
    assert_eq!(tombstones, 0, "a parked walk infers no deletions");
}

/// E2c-WU4 lock-in: a library deleted between the last page and the commit
/// refuses `source_changed` and publishes nothing — no receipt, no tombstone
/// inference from the partial walk.
#[test]
fn deleted_library_between_pages_and_commit_refuses_source_changed() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    let task = claim_bibliography(&conn);
    assert_eq!(task.task_id, task_id);
    assert!(repository::execution_wanted(&conn, &task_id).expect("demand"));

    conn.execute("DELETE FROM zotero_libraries WHERE id = 'lib-1'", [])
        .expect("revoke the library mid-flight");
    let published = std::cell::Cell::new(false);
    let err = repository::commit_success_with(
        &conn,
        &task_id,
        task.lease_epoch,
        "bibliography_sync",
        "bibliography_synced",
        "{}",
        |_| {
            published.set(true);
            Ok(())
        },
    )
    .expect_err("a commit for a deleted library must fail");
    assert!(
        err.starts_with("source_changed"),
        "a deleted library must fail source_changed, got: {err}"
    );
    assert!(!published.get(), "a refused commit publishes nothing");
    let tombstones: i64 = conn
        .query_row("SELECT COUNT(*) FROM zotero_item_tombstones", [], |row| {
            row.get(0)
        })
        .expect("tombstones");
    assert_eq!(tombstones, 0, "a refused walk infers no deletions");
    let receipt: Option<String> = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("receipt");
    assert!(receipt.is_none(), "a refused commit stores no receipt");
}

/// E2c-WU4 lock-in: cancelling the bibliography batch before a late
/// executor success reaches the commit refuses `demand_lost` and publishes
/// nothing — the bibliography flavor of the corpus survivor-demand lock-in.
#[test]
fn cancelled_bibliography_batch_refuses_late_publish() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    let task = claim_bibliography(&conn);
    assert_eq!(task.task_id, task_id);
    repository::control_batch(
        &conn,
        "batch-system-bibliography",
        BatchAction::Cancel,
        None,
    )
    .expect("cancel bibliography demand");
    assert!(
        !repository::execution_wanted(&conn, &task_id).expect("demand"),
        "the cancelled batch must withdraw demand"
    );

    let published = std::cell::Cell::new(false);
    let err = repository::commit_success_with(
        &conn,
        &task_id,
        task.lease_epoch,
        "bibliography_sync",
        "bibliography_synced",
        "{}",
        |_| {
            published.set(true);
            Ok(())
        },
    )
    .expect_err("a commit with no survivor demand must fail");
    assert!(
        err.starts_with("demand_lost"),
        "a late commit after cancel must fail demand_lost, got: {err}"
    );
    assert!(!published.get(), "a refused commit publishes nothing");
}

/// Page source that bumps the connection revision while serving one
/// request, simulating a Zotero reconnect racing the walk.
struct BumpConnectionWhileServing {
    inner: FakeSource,
    db_path: std::path::PathBuf,
    at_request: usize,
}

impl ZoteroPageSource for BumpConnectionWhileServing {
    fn fetch_page(&self, library: &Library, query: BibliographyPageQuery) -> PageFuture {
        let upcoming = self.inner.requests().len() + 1;
        if upcoming == self.at_request {
            let conn = rusqlite::Connection::open(&self.db_path).expect("bump connection");
            conn.execute(
                "UPDATE zotero_connections SET revision = revision + 1, updated_at = 1 WHERE id = 'conn-1'",
                [],
            )
            .expect("bump the connection fence mid-walk");
        }
        self.inner.fetch_page(library, query)
    }
}

/// E2c-WU4 lock-in: a connection bump mid-walk retires the stale run and
/// restarts enumeration from zero — never a mixed snapshot. The first walk
/// reports retryable, the converged retry completes, and the catalog holds
/// each item exactly once.
#[test]
fn connection_bump_restarts_enumeration_from_zero_without_mixing() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    let task = claim_bibliography(&conn);
    assert_eq!(task.task_id, task_id);

    let racing = Arc::new(BumpConnectionWhileServing {
        inner: FakeSource::new(vec![
            ScriptStep::Page(page(
                vec![item("AAAA1111", 12), item("BBBB2222", 40)],
                Some(4),
            )),
            ScriptStep::Page(page(
                vec![item("CCCC3333", 3), item("DDDD4444", 9)],
                Some(4),
            )),
        ]),
        db_path: dir.path().join("entropia.sqlite"),
        at_request: 2,
    });
    let first = BibliographySyncExecutor::new(racing)
        .with_page_limit(2)
        .run(&ctx_of(&dir), &task, &StopFlag::new());
    assert!(
        matches!(
            &first.output,
            ExecOutput::Retryable { code, .. } if code == "zotero_snapshot_changed"
        ),
        "a mid-walk fence bump must retry, got {:?}",
        first.output
    );

    // The converged retry rebuilds from zero under the new fence.
    let retry = BibliographySyncExecutor::new(Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![item("AAAA1111", 12), item("BBBB2222", 40)],
            Some(4),
        )),
        ScriptStep::Page(page(
            vec![item("CCCC3333", 3), item("DDDD4444", 9)],
            Some(4),
        )),
    ])))
    .with_page_limit(2)
    .run(&ctx_of(&dir), &task, &StopFlag::new());
    assert!(
        matches!(&retry.output, ExecOutput::Success { .. }),
        "the converged retry must complete, got {:?}",
        retry.output
    );
    let cataloged: Vec<(String, i64)> = conn
        .prepare("SELECT item_key, item_version FROM bibliographic_items WHERE library_id = 'lib-1' ORDER BY item_key")
        .expect("catalog query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("catalog map")
        .collect::<Result<Vec<_>, _>>()
        .expect("catalog collect");
    assert_eq!(
        cataloged,
        vec![
            ("AAAA1111".to_string(), 12),
            ("BBBB2222".to_string(), 40),
            ("CCCC3333".to_string(), 3),
            ("DDDD4444".to_string(), 9),
        ],
        "each item lands exactly once — no mixed snapshot"
    );
    let run = get_run(&conn, "lib-1")
        .expect("read reconciliation")
        .expect("run exists");
    assert_eq!(
        run.cursor_start, 4,
        "the converged run enumerates the whole snapshot from zero"
    );
}

// ── E3b-WU2: durable per-work profile tasks ────────────────────────────────

use entropia_desktop_lib::bibliography::processing::{
    BibliographyProfileExecutor, ProfileEmbedder,
};

/// Fake embedder: a fixed identity and scripted per-call results, so the
/// durable profile path is verifiable without a network or model files.
struct FakeProfileEmbedder {
    failures: Mutex<VecDeque<String>>,
    model: String,
    contract: String,
    dimensions: usize,
}

impl FakeProfileEmbedder {
    fn ok(dimensions: usize) -> Arc<Self> {
        Arc::new(Self {
            failures: Mutex::new(VecDeque::new()),
            model: "fake/model".to_string(),
            contract: "fake-contract".to_string(),
            dimensions,
        })
    }

    fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            failures: Mutex::new(VecDeque::new()),
            model: "fake/model".to_string(),
            contract: "fake-contract".to_string(),
            dimensions: 4,
        })
        .with_failure(message)
    }

    fn with_failure(self: Arc<Self>, message: &str) -> Arc<Self> {
        self.failures
            .lock()
            .expect("failures")
            .push_back(message.to_string());
        self
    }
}

impl ProfileEmbedder for FakeProfileEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
        match self.failures.lock().expect("failures").pop_front() {
            Some(message) => Err(message),
            None => Ok(vec![0.5; self.dimensions]),
        }
    }

    fn identity(&self) -> Result<(String, String, usize), String> {
        Ok((self.model.clone(), self.contract.clone(), self.dimensions))
    }
}

/// Seeds one connection + library + work with a full metadata snapshot and
/// returns the internal item row id. No attachment rows exist at all: the
/// catalog entry is the only profile input.
fn seed_catalog(
    conn: &mut rusqlite::Connection,
    item_key: &str,
    title: &str,
    abstract_text: &str,
) -> String {
    use entropia_desktop_lib::bibliography::repository::{
        upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
        SourceOrigin, UpsertConnection, UpsertLibrary,
    };
    let source = upsert_connection(
        conn,
        UpsertConnection {
            id: "conn-1".to_string(),
            source_origin: SourceOrigin::Local,
            source_instance_id: None,
            endpoint: Some("http://synthetic.invalid".to_string()),
            capabilities_json: r#"{"read":true}"#.to_string(),
        },
    )
    .expect("seed connection");
    let library = upsert_library(
        conn,
        UpsertLibrary {
            connection_id: source.id,
            library_type: LibraryType::User,
            library_id: "0".to_string(),
            name: "Personal".to_string(),
            last_modified_version: Some(7),
        },
    )
    .expect("seed library");
    let csl = serde_json::json!({
        "id": item_key,
        "type": "book",
        "title": title,
        "abstract": abstract_text,
        "publisher": "Editorial Universitaria",
        "issued": { "date-parts": [[2018]] },
        "author": [
            { "family": "Pérez", "given": "Ana" },
            { "family": "Gómez", "given": "Luis" }
        ],
    });
    let native = serde_json::json!({
        "key": item_key,
        "itemType": "book",
        "tags": [{ "tag": "asociaciones" }, { "tag": "cultura política" }],
    });
    let item = upsert_item(
        conn,
        &library.id,
        BibliographicItemInput {
            item_key: item_key.to_string(),
            item_version: Some(3),
            native_json_snapshot: native.to_string(),
            csl_json_snapshot: csl.to_string(),
            title: Some(title.to_string()),
            ..Default::default()
        },
    )
    .expect("seed catalog item");
    item.id
}

fn admit_profile_demand(conn: &rusqlite::Connection, item_id: &str) -> String {
    let batch = repository::ensure_system_batch(conn, "bibliography").expect("system batch");
    repository::admit_subject_or_attach(
        conn,
        &batch,
        "bibliography_profile",
        &TaskSubject {
            domain: "bibliography".to_string(),
            subject_kind: "item".to_string(),
            subject_id: item_id.to_string(),
        },
        0,
        "",
        "",
        None,
    )
    .expect("admit profile demand")
    .task_id
}

fn profile_registry(embedder: Arc<FakeProfileEmbedder>) -> ExecutorRegistry {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(BibliographyProfileExecutor::new(embedder)));
    registry
}

/// A work without any attachment profiles from catalog metadata alone, and
/// the full claim → run → commit path publishes profile and vector together
/// with a receipt naming the identity. Repeated demand before the run shares
/// one physical task.
#[test]
fn profile_task_publishes_profile_and_vector_atomically() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(
        &mut conn,
        "NOPDF0001",
        "Obra sin adjunto",
        "Estudio sintético.",
    );
    let task_id = admit_profile_demand(&conn, &item_id);
    assert_eq!(
        admit_profile_demand(&conn, &item_id),
        task_id,
        "profile demand must share one physical task"
    );

    let registry = profile_registry(FakeProfileEmbedder::ok(4));
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("profile run");
    match outcome {
        RunOneOutcome::Succeeded { task_id: done } => assert_eq!(done, task_id),
        other => panic!("the profile task must succeed, got {other:?}"),
    }

    let profile =
        entropia_desktop_lib::bibliography::repository::get_semantic_profile(&conn, &item_id)
            .expect("profile read")
            .expect("profile stored");
    assert_eq!(profile.profile_revision, 1);
    assert_eq!(profile.template_version, "bibliography-profile-v1");
    assert!(
        profile
            .canonical_text
            .starts_with("Título: Obra sin adjunto\nAutores: Pérez, Ana; Gómez, Luis"),
        "catalog metadata renders through the template: {}",
        profile.canonical_text
    );
    assert!(
        profile
            .canonical_text
            .contains("Palabras clave y etiquetas: asociaciones; cultura política"),
        "tags render sorted and stable"
    );

    let embedding: (String, String, i64, String) = conn
        .query_row(
            "SELECT embedding_contract, embedding_model, dimensions, input_hash
             FROM bibliographic_item_embeddings WHERE item_id = ?1",
            [&item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("embedding row");
    assert_eq!(embedding.0, "fake-contract");
    assert_eq!(embedding.1, "fake/model");
    assert_eq!(embedding.2, 4);
    assert_eq!(
        embedding.3, profile.input_hash,
        "the vector stamps the profile hash it was computed from"
    );
    let receipt: String = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("receipt read");
    assert!(
        receipt.contains(&item_id) && receipt.contains("fake-contract"),
        "the receipt names the identity: {receipt}"
    );
}

/// An unavailable embedder parks the task blocked — never failed — so the
/// user can fix configuration and resume.
#[test]
fn profile_task_blocks_on_embedder_configuration_errors() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NOPDF0002", "Otra obra", "");
    let task_id = admit_profile_demand(&conn, &item_id);
    let registry = profile_registry(FakeProfileEmbedder::failing(
        "OpenRouter API error (401): unauthorized",
    ));
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("profile run");
    match outcome {
        RunOneOutcome::Blocked { task_id: blocked } => assert_eq!(blocked, task_id),
        other => panic!("a configuration error must park blocked, got {other:?}"),
    }
    let stored: Option<String> = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("receipt read");
    assert!(stored.is_none(), "a blocked run stores no receipt");
    let profiles: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_semantic_profiles",
            [],
            |row| row.get(0),
        )
        .expect("profile count");
    assert_eq!(profiles, 0, "a blocked run publishes nothing");
}

/// A metadata edit between claim and commit refuses source_changed: the
/// staged vector describes text the catalog no longer holds.
#[test]
fn profile_commit_refuses_a_metadata_edit_mid_flight() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NOPDF0003", "Obra volátil", "Resumen original.");
    let task_id = admit_profile_demand(&conn, &item_id);
    let task = repository::claim_next(
        &conn,
        "profile-session",
        &["bibliography_profile"],
        repository::now_ms(),
    )
    .expect("claim scan")
    .expect("claimable");
    let result = BibliographyProfileExecutor::new(FakeProfileEmbedder::ok(4)).run(
        &ctx_of(&dir),
        &task,
        &StopFlag::new(),
    );
    assert!(matches!(result.output, ExecOutput::Success { .. }));

    // The profile renders the trusted CSL snapshot, so that is the surface
    // a metadata edit moves.
    conn.execute(
        "UPDATE bibliographic_items SET csl_json_snapshot = json_set(csl_json_snapshot, '$.title', 'Obra corregida') WHERE id = ?1",
        [&item_id],
    )
    .expect("edit metadata mid-flight");

    let engine_output = result.engine_output.expect("staged profile output");
    let error = repository::commit_success_with(
        &conn,
        &task.task_id,
        task.lease_epoch,
        "bibliography_profile",
        "bibliography_profiled",
        "{}",
        |tx| match &engine_output {
            entropia_desktop_lib::processing::scheduler::EngineOutput::BibliographyProfile(
                profile,
            ) => {
                entropia_desktop_lib::bibliography::processing::publish_bibliography_profile_output(
                    tx, &task, profile,
                )
            }
            _ => unreachable!("test only stages profile output"),
        },
    )
    .expect_err("a moved profile input must fail");
    assert!(
        error.starts_with("source_changed"),
        "a metadata edit must refuse source_changed, got: {error}"
    );
    let profiles: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_semantic_profiles WHERE item_id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .expect("profile count");
    assert_eq!(profiles, 0, "nothing was published for the stale input");
}

// ── E3b-WU3: sync success chains profile demand for stale works only ──────

fn custom_page_item(
    key: &str,
    version: u64,
    title: &str,
    abstract_text: &str,
) -> BibliographyPageItem {
    let csl = serde_json::json!({
        "id": key,
        "type": "book",
        "title": title,
        "abstract": abstract_text,
        "publisher": "Editorial Universitaria",
        "issued": { "date-parts": [[2018]] },
        "author": [{ "family": "Pérez", "given": "Ana" }],
    });
    let native = serde_json::json!({
        "key": key,
        "version": version,
        "itemType": "book",
        "tags": [{ "tag": "asociaciones" }],
    });
    BibliographyPageItem {
        key: key.to_string(),
        item_version: version,
        csl_json: csl.to_string(),
        native_json_snapshot: native.to_string(),
    }
}

/// After a successful library sync, every live work whose profile is
/// missing or stale gets durable profile demand — and only those works. A
/// corrected abstract re-profiles exactly that work: a new task mints (the
/// succeeded one is immutable history), older profiles keep their revision.
#[test]
fn sync_success_chains_profile_demand_only_for_stale_works() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let sync_task = admit_bibliography_task(&conn, "lib-1");

    // First sync: two works, no profiles yet — both get chained demand.
    let first_source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![
            custom_page_item("WORKA0001", 1, "Obra A", "Resumen original."),
            custom_page_item("WORKB0001", 1, "Obra B", "Otro resumen."),
        ],
        Some(2),
    ))]));
    let first = run_one(
        &conn,
        &ctx_of(&dir),
        &registry_with(executor(first_source)),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("first sync");
    assert!(
        matches!(&first, RunOneOutcome::Succeeded { task_id } if task_id == &sync_task),
        "first sync must succeed, got {first:?}"
    );

    let chained: Vec<String> = conn
        .prepare(
            "SELECT t.subject_id FROM processing_tasks t
              JOIN bibliographic_items i ON i.id = t.subject_id
              WHERE t.domain='bibliography' AND t.subject_kind='item' AND t.kind='bibliography_profile'
              ORDER BY i.item_key",
        )
        .expect("chained query")
        .query_map([], |row| row.get(0))
        .expect("chained map")
        .collect::<Result<Vec<_>, _>>()
        .expect("chained collect");
    assert_eq!(
        chained,
        vec![
            items_by_key(&conn, "WORKA0001"),
            items_by_key(&conn, "WORKB0001"),
        ],
        "both works get chained profile demand after the first sync"
    );

    // Drain the profile demand through the mixed registry.
    drain_profiles(&dir, &conn);

    for key in ["WORKA0001", "WORKB0001"] {
        let item_id = items_by_key(&conn, key);
        let profile =
            entropia_desktop_lib::bibliography::repository::get_semantic_profile(&conn, &item_id)
                .expect("profile read")
                .unwrap_or_else(|| panic!("work {key} must have a profile"));
        assert_eq!(profile.profile_revision, 1);
    }

    // Second sync: only Obra A changed (new version + corrected abstract).
    // The succeeded sync task is immutable history, so the fresh demand
    // mints a new sync task; the succeeded profile task of A is likewise
    // immutable, so a NEW profile task must chain for A only - B is
    // unchanged and gets nothing.
    let sync_task2 = admit_bibliography_task(&conn, "lib-1");
    assert_ne!(
        sync_task2, sync_task,
        "a completed sync re-syncs as a new task"
    );
    // A re-sync re-enumerates the whole library: A corrected, B as it was.
    let second_source = Arc::new(FakeSource::new(vec![
        ScriptStep::Page(page(
            vec![custom_page_item(
                "WORKA0001",
                2,
                "Obra A",
                "Resumen corregido.",
            )],
            Some(2),
        )),
        ScriptStep::Page(page(
            vec![custom_page_item("WORKB0001", 1, "Obra B", "Otro resumen.")],
            Some(2),
        )),
    ]));
    let second = run_one(
        &conn,
        &ctx_of(&dir),
        &registry_with(executor(second_source)),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("second sync");
    assert!(
        matches!(&second, RunOneOutcome::Succeeded { task_id } if task_id == &sync_task2),
        "second sync must succeed, got {second:?}"
    );

    let (a_item, b_item) = (
        items_by_key(&conn, "WORKA0001"),
        items_by_key(&conn, "WORKB0001"),
    );
    let live_profiles: Vec<(String, String)> = conn
        .prepare(
            "SELECT t.subject_id, t.state FROM processing_tasks t
              JOIN bibliographic_items i ON i.id = t.subject_id
              WHERE t.domain='bibliography' AND t.subject_kind='item' AND t.kind='bibliography_profile'
                AND t.state NOT IN ('succeeded','failed','skipped','cancelled')
              ORDER BY i.item_key",
        )
        .expect("live query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("live map")
        .collect::<Result<Vec<_>, _>>()
        .expect("live collect");
    assert_eq!(
        live_profiles,
        vec![(a_item.clone(), "pending".to_string())],
        "only the corrected work re-profiles; the unchanged one gets nothing"
    );

    drain_profiles(&dir, &conn);

    let a_profile =
        entropia_desktop_lib::bibliography::repository::get_semantic_profile(&conn, &a_item)
            .expect("profile read")
            .expect("profile stored");
    assert_eq!(
        a_profile.profile_revision, 2,
        "the corrected abstract bumps the profile revision"
    );
    assert!(
        a_profile.canonical_text.contains("Resumen corregido."),
        "the new text is what got published"
    );
    let b_profile =
        entropia_desktop_lib::bibliography::repository::get_semantic_profile(&conn, &b_item)
            .expect("profile read")
            .expect("profile stored");
    assert_eq!(
        b_profile.profile_revision, 1,
        "the unchanged work never re-profiles"
    );
    let _ = item_version_of(&conn, &a_item);
}

fn registry_with(sync: BibliographySyncExecutor) -> ExecutorRegistry {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(sync));
    registry
}

fn profile_only_registry() -> ExecutorRegistry {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(BibliographyProfileExecutor::new(
        FakeProfileEmbedder::ok(4),
    )));
    registry
}

/// Drives every chained profile task to completion through the real
/// claim → run → commit path.
fn drain_profiles(dir: &tempfile::TempDir, conn: &rusqlite::Connection) {
    for _ in 0..8 {
        let registry = profile_only_registry();
        match run_one(
            conn,
            &ctx_of(dir),
            &registry,
            "profile-session",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("profile drain")
        {
            RunOneOutcome::Idle => break,
            _ => continue,
        }
    }
}

fn items_by_key(conn: &rusqlite::Connection, item_key: &str) -> String {
    conn.query_row(
        "SELECT id FROM bibliographic_items WHERE item_key = ?1",
        [item_key],
        |row| row.get(0),
    )
    .expect("item by key")
}

#[allow(dead_code)]
fn item_version_of(conn: &rusqlite::Connection, item_id: &str) -> i64 {
    conn.query_row(
        "SELECT COALESCE(item_version, 0) FROM bibliographic_items WHERE id = ?1",
        [item_id],
        |row| row.get(0),
    )
    .expect("item version")
}
