//! E2b: durable bibliographic synchronization behind the batch queue.
//!
//! These tests drive manual admission and the real claim → dispatch → execute
//! → publish path with synthetic libraries and fake page sources — never a
//! live Zotero or private data. They pin shared demand, page-transaction
//! atomicity, retry/restart convergence, trusted finalization, scheduler
//! publication, and honest stop/error states.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use entropia_desktop_lib::bibliography::ingest::{
    admit_verified_work, record_ingest_decision, run_ingest_operation, verify_ingest_receipt,
    CatalogIngestTransport, IngestDecision, KIND_LINK_MATCH,
};
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
const MIGRATION_0040_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0040_bibliography_catalog.sql");
const MIGRATION_0041_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0041_bibliography_relations.sql");
const MIGRATION_0042_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0042_bibliography_reconciliation.sql");
const MIGRATION_0043_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0043_processing_task_subject_identity.sql"
);
const MIGRATION_0044_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0044_processing_task_subject_cutover.sql"
);
const MIGRATION_0045_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0045_bibliography_sync_tasks.sql");
const MIGRATION_0046_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0046_processing_priority.sql");
const MIGRATION_0047_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0047_bibliographic_semantic_profiles.sql"
);
const MIGRATION_0048_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0048_bibliography_profile_tasks.sql");
const MIGRATION_0049_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0049_bibliographic_index_generations.sql"
);
const MIGRATION_0050_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0050_bibliographic_embedding_generations.sql"
);
const MIGRATION_0051_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0051_bibliographic_profile_fts.sql");
const MIGRATION_0052_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0052_bibliographic_extraction_tasks.sql"
);
const MIGRATION_0053_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0053_bibliographic_page_texts.sql");
const MIGRATION_0054_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0054_bibliographic_chunks.sql");
const MIGRATION_0055_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0055_bibliographic_chunk_embeddings.sql"
);
const MIGRATION_0056_SQL: &str = include_str!(
    "../../../../packages/store/src/migrations/0056_bibliographic_ingest_operations.sql"
);

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
        (MIGRATION_0040_SQL, "0040_bibliography_catalog"),
        (MIGRATION_0041_SQL, "0041_bibliography_relations"),
        (MIGRATION_0042_SQL, "0042_bibliography_reconciliation"),
        (MIGRATION_0043_SQL, "0043_processing_task_subject_identity"),
        (MIGRATION_0044_SQL, "0044_processing_task_subject_cutover"),
        (MIGRATION_0045_SQL, "0045_bibliography_sync_tasks"),
        (MIGRATION_0046_SQL, "0046_processing_priority"),
        (MIGRATION_0047_SQL, "0047_bibliographic_semantic_profiles"),
        (MIGRATION_0048_SQL, "0048_bibliography_profile_tasks"),
        (MIGRATION_0049_SQL, "0049_bibliographic_index_generations"),
        (
            MIGRATION_0050_SQL,
            "0050_bibliographic_embedding_generations",
        ),
        (MIGRATION_0051_SQL, "0051_bibliographic_profile_fts"),
        (MIGRATION_0052_SQL, "0052_bibliographic_extraction_tasks"),
        (MIGRATION_0053_SQL, "0053_bibliographic_page_texts"),
        (MIGRATION_0054_SQL, "0054_bibliographic_chunks"),
        (MIGRATION_0055_SQL, "0055_bibliographic_chunk_embeddings"),
        (MIGRATION_0056_SQL, "0056_bibliographic_ingest_operations"),
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

    let (tasks, next) = repository::list_tasks(&conn, &batch_id, None, None, None, 50, 0)
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
    let (tasks, _) = repository::list_tasks(&conn, &batch_id, None, None, None, 50, 0)
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
    let (tasks, _) = repository::list_tasks(&conn, &batch_id, None, None, None, 50, 0)
        .expect("completed bibliography task list");
    let sync = tasks
        .iter()
        .find(|task| task.task_id == task_id)
        .expect("the completed sync task stays listed beside chained profile tasks");
    assert_eq!(sync.items_seen, Some(1));
    assert_eq!(sync.remote_total, Some(1));
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
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("a failed publication ends the attempt, it does not error the tick");
    assert!(
        matches!(outcome, RunOneOutcome::Waiting { .. }),
        "{outcome:?}"
    );
    let message: String = conn
        .query_row(
            "SELECT last_error_message FROM processing_tasks WHERE id=?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("recorded error");
    assert!(message.contains("synthetic receipt failure"), "{message}");

    let (state, receipt): (String, Option<String>) = conn
        .query_row(
            "SELECT state, result_receipt_json FROM processing_tasks WHERE id=?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task after rollback");
    assert_eq!(
        state, "retry_wait",
        "never succeeded, never stranded running"
    );
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

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("a refused publication ends the attempt");
    assert!(
        matches!(outcome, RunOneOutcome::Waiting { .. }),
        "{outcome:?}"
    );
    let message: String = conn
        .query_row(
            "SELECT last_error_message FROM processing_tasks WHERE id=?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("recorded error");
    assert!(message.starts_with("unsupported_subject"), "{message}");
    let (state, receipt): (String, Option<String>) = conn
        .query_row(
            "SELECT state, result_receipt_json FROM processing_tasks WHERE id=?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("unpublished bibliography task");
    assert_eq!(state, "retry_wait");
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
            EngineOutput::BibliographyExtract(_) => "bibliography_extract",
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
              WHERE id = ?1 AND state='interrupted'",
            rusqlite::params![&ocr_task_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("recovered user task state"),
        1
    );
    // System-owned bibliography work has no resume button: recovery requeues it.
    assert_eq!(
        conn.query_row(
            "SELECT state FROM processing_tasks WHERE id = ?1",
            [&bibliography_task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("recovered system task state"),
        "pending"
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

/// A provider throttle (429) never ends a profile task as failed: however
/// many times it repeats, the task parks in `retry_wait` and is claimed again
/// once its delay has passed.
#[test]
fn profile_task_stays_in_retry_wait_through_repeated_rate_limits() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NOPDF0004", "Obra limitada", "Resumen.");
    let task_id = admit_profile_demand(&conn, &item_id);
    let embedder = FakeProfileEmbedder::ok(4);
    let registry = profile_registry(Arc::clone(&embedder));
    let mut now = repository::now_ms();
    for round in 0..6_i64 {
        embedder
            .failures
            .lock()
            .expect("failures")
            .push_back("OpenRouter embedding API error (429 Too Many Requests): {}".to_string());
        let outcome = run_one(
            &conn,
            &ctx_of(&dir),
            &registry,
            "profile-session",
            now,
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("profile run");
        match outcome {
            RunOneOutcome::Waiting { task_id: waiting } => assert_eq!(waiting, task_id),
            other => panic!("round {round}: a rate limit must wait, got {other:?}"),
        }
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = ?1",
                [&task_id],
                |row| row.get(0),
            )
            .expect("state read");
        assert_eq!(state, "retry_wait", "round {round}");
        now += 3_600_000;
    }
}

/// A malformed request is the caller's bug: it fails the task, honestly.
#[test]
fn profile_task_fails_on_a_bad_request_not_a_rate_limit() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NOPDF0005", "Obra mal pedida", "Resumen.");
    let task_id = admit_profile_demand(&conn, &item_id);
    let registry = profile_registry(FakeProfileEmbedder::failing(
        "OpenRouter embedding API error (400 Bad Request): invalid input",
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
        RunOneOutcome::Failed { task_id: failed } => assert_eq!(failed, task_id),
        other => panic!("a 400 must fail, got {other:?}"),
    }
}

/// A work whose profile task ended `failed` and that has no published
/// profile is picked up again by the profile demand a successful library sync
/// chains (the manual "Sincronizar biblioteca"): terminal history stays, and
/// a fresh task is minted.
#[test]
fn a_sync_re_admits_a_work_whose_profile_task_failed() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NOPDF0006", "Obra fallida", "Resumen.");
    let failed_id = admit_profile_demand(&conn, &item_id);
    let registry = profile_registry(FakeProfileEmbedder::failing(
        "OpenRouter embedding API error (400 Bad Request): invalid input",
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
    assert!(
        matches!(outcome, RunOneOutcome::Failed { .. }),
        "{outcome:?}"
    );

    let created =
        repository::admit_stale_profile_demands(&conn, "lib-1").expect("sync-chained admission");

    assert_eq!(created, 1, "the failed work is demanded again");
    let live: Vec<String> = conn
        .prepare(
            "SELECT id FROM processing_tasks
             WHERE kind = 'bibliography_profile' AND subject_id = ?1
               AND state IN ('pending', 'retry_wait', 'running')",
        )
        .expect("live query")
        .query_map([&item_id], |row| row.get(0))
        .expect("live map")
        .collect::<Result<_, _>>()
        .expect("live rows");
    assert_eq!(live.len(), 1);
    assert_ne!(live[0], failed_id, "terminal history is never reopened");
}

/// A work whose profile task was parked `interrupted` by a restart is not
/// stranded: the profile demand a library sync chains attaches to that same
/// task AND puts it back to `pending`, because the bibliography system batch
/// has no UI to resume it.
#[test]
fn a_sync_resumes_a_work_whose_profile_task_is_interrupted() {
    let (_dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NOPDF0007", "Obra interrumpida", "Resumen.");
    let task_id = admit_profile_demand(&conn, &item_id);
    conn.execute(
        "UPDATE processing_tasks SET state = 'interrupted' WHERE id = ?1",
        [&task_id],
    )
    .expect("park the task as a restart does");

    let created =
        repository::admit_stale_profile_demands(&conn, "lib-1").expect("sync-chained admission");

    assert_eq!(created, 0, "the interrupted task is reused, not duplicated");
    let state: String = conn
        .query_row(
            "SELECT state FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("task state");
    assert_eq!(state, "pending", "the demand puts it back in the queue");
}

/// A metadata edit between claim and commit refuses source_changed: the
/// staged vector describes text the catalog no longer holds.
#[test]
fn profile_commit_refuses_a_metadata_edit_mid_flight() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NOPDF0003", "Obra volátil", "Resumen original.");
    let _task_id = admit_profile_demand(&conn, &item_id);
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
// ── E3c-WU2: profile execution lands in the staging generation ────────────

use entropia_desktop_lib::processing::eligibility::resolve_effective_embedding_contract;

fn staging_generation_of(conn: &rusqlite::Connection, contract_hash: &str) -> Option<String> {
    conn.query_row(
        "SELECT id FROM bibliographic_index_generations
         WHERE contract_hash = ?1 AND status IN ('staging', 'active')",
        [contract_hash],
        |row| row.get(0),
    )
    .ok()
}

fn generation_state(
    conn: &rusqlite::Connection,
    generation_id: &str,
) -> (String, i64, i64, String) {
    conn.query_row(
        "SELECT status, expected_inputs, completed_inputs, contract_hash
         FROM bibliographic_index_generations WHERE id = ?1",
        [generation_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .expect("generation state")
}

/// A profile publish stamps the staging generation of the effective
/// contract, grows the manifest to the eligible set, and counts progress
/// only for new works: re-profiles update their row without inflating the
/// count or the vector revision.
#[test]
fn profile_publish_stamps_staging_generation_and_tracks_manifest() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let a_item = seed_catalog(&mut conn, "GENA0001", "Obra A", "Resumen A.");
    let b_item = seed_catalog(&mut conn, "GENB0001", "Obra B", "Resumen B.");
    let a_task = admit_profile_demand(&conn, &a_item);
    let b_task = admit_profile_demand(&conn, &b_item);
    assert_ne!(a_task, b_task);

    for _ in 0..4 {
        let outcome = run_one(
            &conn,
            &ctx_of(&dir),
            &profile_only_registry(),
            "profile-session",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("profile drain");
        if matches!(outcome, RunOneOutcome::Idle) {
            break;
        }
    }

    let effective = resolve_effective_embedding_contract(&conn).expect("effective contract");
    let staging = staging_generation_of(&conn, &effective.hash)
        .expect("one staging generation must exist for the effective contract");
    for (item_id, expected_title) in [(&a_item, "Obra A"), (&b_item, "Obra B")] {
        let row: (String, String, i64) = conn
            .query_row(
                "SELECT generation_id, input_hash, profile_revision
                 FROM bibliographic_item_embeddings WHERE item_id = ?1",
                [item_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap_or_else(|_| panic!("work {expected_title} must have a vector"));
        assert_eq!(
            row.0, staging,
            "the vector names the staging generation, not a bare contract"
        );
        assert_eq!(row.2, 1);
        let profile =
            entropia_desktop_lib::bibliography::repository::get_semantic_profile(&conn, item_id)
                .expect("profile read")
                .expect("profile stored");
        assert_eq!(profile.input_hash, row.1);
    }
    let (status, expected, completed, contract) = generation_state(&conn, &staging);
    assert_eq!(
        (status.as_str(), contract.as_str()),
        ("active", effective.hash.as_str()),
        "the last publish activated the complete generation"
    );
    assert_eq!(expected, 2, "the manifest covers the eligible set");
    assert_eq!(completed, 2, "progress counts distinct new works");

    // Re-profiling the unchanged work mints a new task but changes nothing
    // durable: same hash, same revision, no progress inflation.
    let a_task2 = admit_profile_demand(&conn, &a_item);
    assert_ne!(a_task2, a_task);
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("re-profile run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "the re-profile must succeed, got {outcome:?}"
    );
    let (_, expected2, completed2, _) = generation_state(&conn, &staging);
    assert_eq!(
        (expected2, completed2),
        (2, 2),
        "a same-hash re-publish changes nothing"
    );
    let a_row: i64 = conn
        .query_row(
            "SELECT profile_revision FROM bibliographic_item_embeddings WHERE item_id = ?1",
            [&a_item],
            |row| row.get(0),
        )
        .expect("revision read");
    assert_eq!(
        a_row, 1,
        "an unchanged re-publish keeps the vector revision"
    );
}

/// A generation that stops being staging between run and commit refuses
/// the publish as a configuration change — the vector never lands in a
/// dead generation.
#[test]
fn profile_commit_refuses_a_dead_generation() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "GENC0001", "Obra C", "Resumen C.");
    let _task_id = admit_profile_demand(&conn, &item_id);
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
    let engine_output = result.engine_output.expect("staged profile output");
    let staged_generation = match &engine_output {
        entropia_desktop_lib::processing::scheduler::EngineOutput::BibliographyProfile(profile) => {
            profile.generation_id.clone()
        }
        _ => unreachable!("test only stages profile output"),
    };

    // Retire the staged generation before the commit: the vector must not
    // land in a dead row.
    entropia_desktop_lib::bibliography::generation::retire_index_generation(
        &conn,
        &staged_generation,
        repository::now_ms(),
    )
    .expect("retire staging");

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
    .expect_err("a publish into a retired generation must fail");
    assert!(
        error.starts_with("configuration_changed"),
        "a dead generation must refuse configuration_changed, got: {error}"
    );
    let vectors: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_item_embeddings WHERE item_id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .expect("vector count");
    assert_eq!(vectors, 0, "nothing lands in a dead generation");
}

// ── E4a-WU2: durable native extraction without OCR or corpus assets ────────

use entropia_desktop_lib::bibliography::processing::BibliographyExtractExecutor;
use lopdf::{dictionary, Document, Object, Stream};

/// Builds a PDF with Helvetica text lines at explicit positions, one entry
/// per page. Pure lopdf synthesis — no fixture files, no OCR anywhere near
/// these tests.
fn make_text_pdf_pages(pages: &[&[(f32, f32, &str)]]) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let pages_id = document.new_object_id();
    let mut page_ids = Vec::with_capacity(pages.len());
    for lines in pages {
        let mut content = String::from("BT /F1 12 Tf ");
        for (x, y, text) in lines.iter() {
            let escaped = text
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");
            content.push_str(&format!("{x} {y} Td ({escaped}) Tj "));
        }
        content.push_str("ET");
        let content_id = document.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = document.new_object_id();
        document.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Contents" => content_id,
            }),
        );
        page_ids.push(Object::Reference(page_id));
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => pages.len() as i64,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .expect("serialize synthetic PDF");
    bytes
}

/// Builds a one-page PDF with Helvetica text lines at explicit positions.
/// Pure lopdf synthesis — no fixture files, no OCR anywhere near this test.
fn make_text_pdf(lines: &[(f32, f32, &str)]) -> Vec<u8> {
    make_text_pdf_pages(&[lines])
}

/// Builds a one-page PDF with Helvetica text lines at explicit positions.
/// Pure lopdf synthesis — no fixture files, no OCR anywhere near this test.
fn seed_attachment(
    conn: &mut rusqlite::Connection,
    item_id: &str,
    attachment_key: &str,
    link_mode: &str,
    native_path: Option<&str>,
    filename: &str,
    content_type: &str,
) -> String {
    use entropia_desktop_lib::bibliography::repository::{upsert_attachment, AttachmentInput};
    upsert_attachment(
        conn,
        item_id,
        AttachmentInput {
            attachment_key: attachment_key.to_string(),
            content_type: Some(content_type.to_string()),
            link_mode: Some(link_mode.to_string()),
            filename: Some(filename.to_string()),
            native_path: native_path.map(String::from),
            url: None,
            md5: None,
            mtime: Some(1_700_000_000),
            native_json_snapshot: serde_json::json!({
                "key": attachment_key, "itemType": "attachment",
                "linkMode": link_mode, "contentType": content_type,
            })
            .to_string(),
            native_version: Some(3),
        },
    )
    .expect("seed attachment")
    .id
}

fn write_temp_pdf(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write synthetic PDF");
    path.to_string_lossy().to_string()
}

fn admit_extract_demand(conn: &rusqlite::Connection, attachment_id: &str) -> String {
    let batch = repository::ensure_system_batch(conn, "bibliography").expect("system batch");
    repository::admit_subject_or_attach(
        conn,
        &batch,
        "bibliography_extract",
        &TaskSubject {
            domain: "bibliography".to_string(),
            subject_kind: "attachment".to_string(),
            subject_id: attachment_id.to_string(),
        },
        0,
        "",
        "",
        None,
    )
    .expect("admit extract demand")
    .task_id
}

fn extract_registry() -> ExecutorRegistry {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(BibliographyExtractExecutor::new()));
    registry
}

fn run_extract(dir: &tempfile::TempDir, conn: &rusqlite::Connection, task_id: &str) {
    let outcome = run_one(
        conn,
        &ctx_of(dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    match outcome {
        RunOneOutcome::Succeeded { task_id: done } => assert_eq!(done, task_id),
        other => panic!("the extract task must succeed, got {other:?}"),
    }
}

const SNAPSHOT_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Nota</title>
<script>var tracking = 1;</script></head><body>
<nav><a href="/">Inicio</a></nav>
<h1>La  crisis
   de 2001</h1>
<p>Primer parrafo del articulo con texto
   repartido en lineas de fuente.</p>
<ul><li>Primer punto de la lista</li><li>Segundo punto</li></ul>
<p>Linea uno<br>Linea dos</p>
<footer>Todos los derechos reservados</footer></body></html>"#;

/// A stored HTML snapshot extracts as one page of block-separated
/// paragraphs through the same durable path, and the paragraph chunker
/// turns it into chunks whose spans all point at page 1.
#[test]
fn extract_task_publishes_html_snapshot_as_one_page_of_paragraphs() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "HTMLWORK1", "Obra con captura", "Resumen.");
    let path = dir.path().join("snapshot.html");
    std::fs::write(&path, SNAPSHOT_HTML).expect("write snapshot");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "HTMLATT01",
        "imported_url",
        Some(&path.to_string_lossy()),
        "snapshot.html",
        "text/html",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &task_id);

    let (page_count, method): (i64, String) = conn
        .query_row(
            "SELECT page_count, method FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("extraction row");
    assert_eq!(page_count, 1, "a whole HTML document is a single page");
    assert_eq!(method, "native");
    let pages: Vec<(i64, String)> = conn
        .prepare(
            "SELECT page_number, text_content FROM bibliographic_page_texts
             WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .expect("pages query")
        .query_map([&attachment_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("pages map")
        .collect::<Result<Vec<_>, _>>()
        .expect("pages");
    assert_eq!(
        pages,
        vec![(
            1,
            "La crisis de 2001\n\nPrimer parrafo del articulo con texto repartido en lineas de fuente.\n\nPrimer punto de la lista\n\nSegundo punto\n\nLinea uno\nLinea dos"
                .to_string()
        )],
        "blocks split paragraphs; source line breaks and page furniture do not"
    );
    let chunkable =
        entropia_desktop_lib::bibliography::repository::chunkable_pages_for_item(&conn, &item_id)
            .expect("chunkable pages");
    let chunks = entropia_desktop_lib::bibliography::chunks::segment_pages(
        &chunkable
            .iter()
            .map(
                |page| entropia_desktop_lib::bibliography::chunks::PageInput {
                    page_number: page.page_number,
                    text: page.text_content.clone(),
                },
            )
            .collect::<Vec<_>>(),
    );
    assert!(!chunks.is_empty(), "the snapshot yields chunks");
    assert!(chunks
        .iter()
        .flat_map(|chunk| chunk.spans.iter())
        .all(|span| span.page_number == 1));
}

/// A page that is all furniture extracts to nothing: the task succeeds, the
/// row says `empty`, and there is no page text to chunk.
#[test]
fn extract_task_html_snapshot_with_only_boilerplate_yields_no_chunks() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "HTMLWORK2", "Obra sin cuerpo", "Resumen.");
    let path = dir.path().join("vacio.html");
    std::fs::write(
        &path,
        "<html><body><nav>Menu</nav><script>x()</script><footer>Pie</footer></body></html>",
    )
    .expect("write snapshot");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "HTMLATT02",
        "imported_url",
        Some(&path.to_string_lossy()),
        "vacio.html",
        "text/html",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &task_id);

    let quality: String = conn
        .query_row(
            "SELECT quality FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("extraction row");
    assert_eq!(quality, "empty");
    let chunkable =
        entropia_desktop_lib::bibliography::repository::chunkable_pages_for_item(&conn, &item_id)
            .expect("chunkable pages");
    assert!(chunkable.is_empty(), "nothing to chunk");
}

/// Latin-1 bytes declared by the page's own meta decode correctly.
#[test]
fn extract_task_html_snapshot_honours_its_declared_charset() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "HTMLWORK3", "Obra latina", "Resumen.");
    let path = dir.path().join("latin.html");
    let mut bytes = b"<meta charset=\"iso-8859-1\"><p>La ca".to_vec();
    bytes.push(0xF1);
    bytes.extend_from_slice(b"ada del a");
    bytes.push(0xF1);
    bytes.extend_from_slice(b"o</p>");
    std::fs::write(&path, bytes).expect("write snapshot");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "HTMLATT03",
        "imported_file",
        Some(&path.to_string_lossy()),
        "latin.html",
        "text/html",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &task_id);
    let text: String = conn
        .query_row(
            "SELECT text_content FROM bibliographic_page_texts WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("page row");
    assert_eq!(text, "La cañada del año");
}

/// A stored PDF extracts its native text through the durable path with no
/// OCR call and no corpus asset: the row carries page count, quality, and
/// the source file identity it was read from.
#[test]
fn extract_task_publishes_native_text_without_ocr() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "PDFWORK01", "Obra con PDF", "Resumen.");
    let pdf = make_text_pdf(&[(50.0, 750.0, "Contenido nativo verificable con peso semantico suficiente para superar el umbral de calidad")]);
    let path = write_temp_pdf(&dir, "paper.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "PDFATT001",
        "linked_file",
        Some(&path),
        "paper.pdf",
        "application/pdf",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    match outcome {
        RunOneOutcome::Succeeded { task_id: done } => assert_eq!(done, task_id),
        other => panic!("the extract task must succeed, got {other:?}"),
    }

    let row: (String, i64, String, String, i64, i64, Option<i64>) = conn
        .query_row(
            "SELECT method, page_count, quality, text_content, text_chars, source_bytes, source_mtime
             FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )
        .expect("extraction row");
    assert_eq!(row.0, "native");
    assert_eq!(row.1, 1);
    assert_eq!(row.2, "rich");
    assert!(
        row.3.contains("Contenido nativo verificable"),
        "the native text layer is what got published: {}",
        row.3.chars().take(120).collect::<String>()
    );
    assert_eq!(row.5 as usize, pdf.len(), "the source identity is pinned");
    // No corpus assets, no OCR tasks: the documentary pipeline is untouched.
    let assets: i64 = conn
        .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
        .expect("asset count");
    assert_eq!(assets, 0, "extraction must not mint corpus assets");
    let ocr_tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE kind = 'ocr'",
            [],
            |row| row.get(0),
        )
        .expect("ocr task count");
    assert_eq!(ocr_tasks, 0, "extraction must not route through OCR");
    let receipt: String = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("receipt read");
    assert!(
        receipt.contains(&attachment_id),
        "the receipt names the attachment: {receipt}"
    );
}

/// `pdf-extract` panics on whole families of valid-but-unusual PDFs
/// (here a PostScript tint transform). The panic used to take the task
/// down as `executor_panicked`; now the whole-document parser is
/// contained and the per-page lopdf layer still delivers the text.
#[test]
fn extract_task_reads_text_when_the_whole_document_parser_panics() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "PDFPANIC1", "Obra con PDF raro", "Resumen.");
    let pdf = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pdf-type4-tint-transform.pdf"
    ))
    .expect("fixture");
    let path = write_temp_pdf(&dir, "raro.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "PDFATT002",
        "linked_file",
        Some(&path),
        "raro.pdf",
        "application/pdf",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);

    run_extract(&dir, &conn, &task_id);

    let (page_count, text): (i64, String) = conn
        .query_row(
            "SELECT page_count, text_content FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("extraction row");
    assert_eq!(page_count, 1);
    assert!(
        text.contains("Texto nativo legible"),
        "the text came through the per-page layer: {text}"
    );
}

/// A permissions-only encrypted PDF whose whole-document parse comes back
/// empty (inline image before the text) still publishes its text: the
/// extraction is `rich`, not the `empty` that used to trigger a pointless
/// re-demand on every sync.
#[test]
fn extract_task_reads_text_the_whole_document_parser_silently_drops() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "PDFEMPTY1", "Informe cifrado", "Resumen.");
    let pdf = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pdf-rc4-40-inline-image-text.pdf"
    ))
    .expect("fixture");
    let path = write_temp_pdf(&dir, "informe.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "PDFATT003",
        "linked_file",
        Some(&path),
        "informe.pdf",
        "application/pdf",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);

    run_extract(&dir, &conn, &task_id);

    let (quality, text): (String, String) = conn
        .query_row(
            "SELECT quality, text_content FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("extraction row");
    assert_eq!(quality, "rich", "{text}");
    assert!(
        text.contains("Informe sociolaboral del Partido de General Pueyrredon"),
        "{text}"
    );
}

/// An attachment with no resolvable file parks blocked with the resolver
/// reason — never failed, never retried blindly.
#[test]
fn extract_task_blocks_when_no_file_resolves() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "URLWORK01", "Obra con enlace", "Resumen.");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "URLATT001",
        "linked_url",
        None,
        "",
        "text/html",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    match outcome {
        RunOneOutcome::Blocked { task_id: blocked } => assert_eq!(blocked, task_id),
        other => panic!("an unresolvable attachment must park blocked, got {other:?}"),
    }
    let code: Option<String> = conn
        .query_row(
            "SELECT last_error_code FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("error code");
    assert_eq!(code.as_deref(), Some("extraction_unavailable"));
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_extractions",
            [],
            |row| row.get(0),
        )
        .expect("extraction count");
    assert_eq!(rows, 0, "a blocked run publishes nothing");
}

/// A non-PDF file fails honestly: it will never become extractable by
/// retrying the native path.
#[test]
fn extract_task_fails_unsupported_mime() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "EPUBWORK1", "Obra con epub", "Resumen.");
    let note_path = write_temp_pdf(&dir, "nota.txt", b"esto no es un pdf");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "EPUBATT01",
        "linked_file",
        Some(&note_path),
        "nota.txt",
        "text/plain",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    match outcome {
        RunOneOutcome::Failed { task_id: failed } => assert_eq!(failed, task_id),
        other => panic!("a non-PDF must fail honestly, got {other:?}"),
    }
}

/// A nearly empty text layer still publishes — flagged sparse so E4b's
/// selective OCR knows exactly where native text runs out.
#[test]
fn sparse_text_publishes_as_sparse_for_selective_ocr() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "SCANWORK1", "Obra casi vacía", "Resumen.");
    let pdf = make_text_pdf(&[(50.0, 750.0, "ok")]);
    let path = write_temp_pdf(&dir, "scan.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "SCANATT01",
        "linked_file",
        Some(&path),
        "scan.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "sparse text is a success with a flag, not a failure: {outcome:?}"
    );
    let quality: String = conn
        .query_row(
            "SELECT quality FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("quality read");
    assert_eq!(quality, "sparse");
}

// ── E4a-WU3: sync chaining and multicolumn proof ───────────────────────────

/// A successful sync chains extraction demand only for attachments whose
/// file resolves locally. Web links get no demand — the executor would
/// only park them blocked.
#[test]
fn sync_success_chains_extraction_demand_for_readable_pdfs_only() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let sync_task = admit_bibliography_task(&conn, "lib-1");
    let first_source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![
            custom_page_item("PDFWORK01", 1, "Obra con PDF", "Resumen."),
            custom_page_item("URLWORK01", 1, "Obra con enlace", "Resumen."),
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

    // Attachments arrive with the catalog: one stored PDF, one web link.
    let a_item = items_by_key(&conn, "PDFWORK01");
    let pdf = make_text_pdf(&[(
        50.0,
        750.0,
        "Texto extraible del adjunto con longitud suficiente para calidad",
    )]);
    let path = write_temp_pdf(&dir, "adjunto.pdf", &pdf);
    let att_a = seed_attachment(
        &mut conn,
        &a_item,
        "CHAINPDF1",
        "linked_file",
        Some(&path),
        "adjunto.pdf",
        "application/pdf",
    );
    let b_item = items_by_key(&conn, "URLWORK01");
    let att_b = seed_attachment(
        &mut conn,
        &b_item,
        "CHAINURL1",
        "linked_url",
        None,
        "",
        "text/html",
    );

    // A re-sync re-enumerates the whole library and chains extraction for
    // the readable file only.
    let sync_task2 = admit_bibliography_task(&conn, "lib-1");
    assert_ne!(sync_task2, sync_task);
    let second_source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![
            custom_page_item("PDFWORK01", 1, "Obra con PDF", "Resumen."),
            custom_page_item("URLWORK01", 1, "Obra con enlace", "Resumen."),
        ],
        Some(2),
    ))]));
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

    let live_extracts: Vec<(String, String)> = conn
        .prepare(
            "SELECT t.subject_id, t.state FROM processing_tasks t
             WHERE t.domain = 'bibliography' AND t.subject_kind = 'attachment'
               AND t.kind = 'bibliography_extract'
               AND t.state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')
             ORDER BY t.subject_id",
        )
        .expect("live query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("live map")
        .collect::<Result<Vec<_>, _>>()
        .expect("live collect");
    assert_eq!(
        live_extracts,
        vec![(att_a.clone(), "pending".to_string())],
        "only the readable file gets chained demand"
    );

    // Draining publishes the extraction for the PDF attachment.
    let drained = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract drain");
    assert!(
        matches!(drained, RunOneOutcome::Succeeded { .. }),
        "the chained extraction must succeed, got {drained:?}"
    );
    let quality: String = conn
        .query_row(
            "SELECT quality FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&att_a],
            |row| row.get(0),
        )
        .expect("quality read");
    assert_eq!(quality, "rich");
    let _ = att_b;
}

// ── Extraction demand is admitted once per source identity ─────────────────

/// Seeds one readable PDF attachment under a catalog work and returns
/// `(library_row_id, attachment_id, item_id, path)`.
fn seed_readable_pdf(
    dir: &tempfile::TempDir,
    conn: &mut rusqlite::Connection,
) -> (String, String, String, String) {
    let item_id = seed_catalog(conn, "DEMANDW01", "Obra con demanda", "Resumen.");
    let pdf = make_text_pdf(&[(
        50.0,
        750.0,
        "Texto extraible del adjunto con longitud suficiente para calidad",
    )]);
    let path = write_temp_pdf(dir, "demanda.pdf", &pdf);
    let attachment_id = seed_attachment(
        conn,
        &item_id,
        "DEMANDA01",
        "linked_file",
        Some(&path),
        "demanda.pdf",
        "application/pdf",
    );
    let library_row_id: String = conn
        .query_row("SELECT id FROM zotero_libraries LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("library row");
    (library_row_id, attachment_id, item_id, path)
}

fn extract_task_count(conn: &rusqlite::Connection, attachment_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM processing_tasks
         WHERE domain = 'bibliography' AND subject_kind = 'attachment'
           AND kind = 'bibliography_extract' AND subject_id = ?1",
        [attachment_id],
        |row| row.get(0),
    )
    .expect("extract task count")
}

/// Two consecutive syncs over the same unchanged attachment leave ONE task:
/// the second attaches to the live one.
#[test]
fn two_syncs_over_an_unchanged_attachment_leave_one_extract_task() {
    let (dir, mut conn) = migrated_db();
    let (library, attachment_id, _item, _path) = seed_readable_pdf(&dir, &mut conn);

    let first = repository::admit_stale_extraction_demands(&conn, &library).expect("first");
    let second = repository::admit_stale_extraction_demands(&conn, &library).expect("second");

    assert_eq!(first, 1, "the first sync admits the missing extraction");
    assert_eq!(second, 0, "the second sync attaches to the live task");
    assert_eq!(extract_task_count(&conn, &attachment_id), 1);
}

/// An attachment whose extraction already reflects the current file gets no
/// new task on any later sync: the catalog identity (mtime/version) and the
/// file's byte length are what the extraction pinned.
#[test]
fn an_attachment_with_a_current_extraction_gets_no_new_task() {
    let (dir, mut conn) = migrated_db();
    let (library, attachment_id, _item, _path) = seed_readable_pdf(&dir, &mut conn);
    let task_id = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &task_id);
    assert_eq!(extract_task_count(&conn, &attachment_id), 1);

    for sync in 1..=2 {
        let created = repository::admit_stale_extraction_demands(&conn, &library)
            .unwrap_or_else(|e| panic!("sync {sync}: {e}"));
        assert_eq!(
            created, 0,
            "sync {sync} must not re-demand a current extraction"
        );
    }
    assert_eq!(
        extract_task_count(&conn, &attachment_id),
        1,
        "no task beyond the one that produced the extraction"
    );
}

/// A file that really changed (new catalog mtime, or different bytes under
/// the same catalog row) gets exactly one new task, however many syncs run.
#[test]
fn a_changed_file_gets_exactly_one_new_extract_task() {
    let (dir, mut conn) = migrated_db();
    let (library, attachment_id, item_id, path) = seed_readable_pdf(&dir, &mut conn);
    let task_id = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &task_id);

    // Zotero reports a newer mtime for the same attachment.
    seed_attachment_with_mtime(&mut conn, &item_id, &path, 1_800_000_000);
    assert_eq!(
        repository::admit_stale_extraction_demands(&conn, &library).expect("mtime sync"),
        1
    );
    assert_eq!(
        repository::admit_stale_extraction_demands(&conn, &library).expect("repeat sync"),
        0
    );
    assert_eq!(extract_task_count(&conn, &attachment_id), 2);

    // Drain it, then replace the bytes on disk behind an unchanged catalog.
    let task = conn
        .query_row(
            "SELECT id FROM processing_tasks WHERE subject_id = ?1 AND state = 'pending'",
            [&attachment_id],
            |row| row.get::<_, String>(0),
        )
        .expect("pending task");
    run_extract(&dir, &conn, &task);
    let bigger = make_text_pdf(&[
        (
            50.0,
            750.0,
            "Texto extraible del adjunto con longitud suficiente para calidad",
        ),
        (
            50.0,
            730.0,
            "Una segunda linea que cambia el tamano del archivo en disco",
        ),
    ]);
    std::fs::write(&path, bigger).expect("replace file");
    assert_eq!(
        repository::admit_stale_extraction_demands(&conn, &library).expect("bytes sync"),
        1
    );
    assert_eq!(extract_task_count(&conn, &attachment_id), 3);
}

fn seed_attachment_with_mtime(
    conn: &mut rusqlite::Connection,
    item_id: &str,
    path: &str,
    mtime: i64,
) {
    use entropia_desktop_lib::bibliography::repository::{upsert_attachment, AttachmentInput};
    upsert_attachment(
        conn,
        item_id,
        AttachmentInput {
            attachment_key: "DEMANDA01".to_string(),
            content_type: Some("application/pdf".to_string()),
            link_mode: Some("linked_file".to_string()),
            filename: Some("demanda.pdf".to_string()),
            native_path: Some(path.to_string()),
            url: None,
            md5: None,
            mtime: Some(mtime),
            native_json_snapshot: serde_json::json!({
                "key": "DEMANDA01", "itemType": "attachment",
                "linkMode": "linked_file", "contentType": "application/pdf",
            })
            .to_string(),
            native_version: Some(4),
        },
    )
    .expect("update attachment");
}

/// Duplicates that were queued before the admission fix finish as
/// succeeded without touching the extraction: no re-extraction, no page or
/// chunk rewrite, no profile chain.
#[test]
fn a_queued_duplicate_for_a_current_extraction_finishes_without_re_extracting() {
    let (dir, mut conn) = migrated_db();
    let (_library, attachment_id, _item, _path) = seed_readable_pdf(&dir, &mut conn);
    let first = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &first);
    let profile_tasks = |conn: &rusqlite::Connection| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE kind = 'bibliography_profile'",
            [],
            |row| row.get(0),
        )
        .expect("profile count")
    };
    // The first extraction legitimately chained its profile.
    let profiles_before = profile_tasks(&conn);
    // A re-extraction would overwrite this sentinel with the file's text.
    conn.execute(
        "UPDATE bibliographic_extractions SET text_content = 'SENTINEL' WHERE attachment_id = ?1",
        [&attachment_id],
    )
    .expect("tamper");
    conn.execute(
        "UPDATE bibliographic_page_texts SET text_content = 'SENTINEL' WHERE attachment_id = ?1",
        [&attachment_id],
    )
    .expect("tamper pages");

    let duplicate = admit_extract_demand(&conn, &attachment_id);
    assert_ne!(duplicate, first, "the first task is terminal history");
    run_extract(&dir, &conn, &duplicate);

    let (text, page_text): (String, String) = conn
        .query_row(
            "SELECT e.text_content, p.text_content FROM bibliographic_extractions e
             JOIN bibliographic_page_texts p ON p.attachment_id = e.attachment_id
             WHERE e.attachment_id = ?1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("rows");
    assert_eq!(text, "SENTINEL", "the extraction row was not rewritten");
    assert_eq!(page_text, "SENTINEL", "the page rows were not rewritten");
    assert_eq!(
        profile_tasks(&conn),
        profiles_before,
        "a current extraction chains no profile demand"
    );
}

/// The shortcut only fires for the same file identity: a queued task whose
/// file changed since the extraction still extracts for real.
#[test]
fn a_queued_task_for_a_changed_file_still_re_extracts() {
    let (dir, mut conn) = migrated_db();
    let (_library, attachment_id, _item, path) = seed_readable_pdf(&dir, &mut conn);
    let first = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &first);
    conn.execute(
        "UPDATE bibliographic_extractions SET text_content = 'SENTINEL' WHERE attachment_id = ?1",
        [&attachment_id],
    )
    .expect("tamper");
    let bigger = make_text_pdf(&[
        (
            50.0,
            750.0,
            "Texto extraible del adjunto con longitud suficiente para calidad",
        ),
        (
            50.0,
            730.0,
            "Una segunda linea que cambia el tamano del archivo en disco",
        ),
    ]);
    std::fs::write(&path, bigger).expect("replace file");

    let second = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &second);

    let text: String = conn
        .query_row(
            "SELECT text_content FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("text");
    assert!(text.contains("segunda linea"), "re-extracted: {text}");
}

/// A two-column native PDF reads in column order — left column before
/// right — with no OCR call and no corpus asset anywhere in the loop.
#[test]
fn multicolumn_pdf_reads_in_column_order_without_ocr() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "COLWORK01", "Obra a dos columnas", "Resumen.");
    let pdf = make_text_pdf(&[
        (50.0, 750.0, "COLUMNA IZQUIERDA alfa"),
        (320.0, 750.0, "COLUMNA DERECHA beta"),
        (50.0, 730.0, "segunda linea izquierda"),
        (320.0, 730.0, "segunda linea derecha"),
    ]);
    let path = write_temp_pdf(&dir, "columnas.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "COLATT001",
        "linked_file",
        Some(&path),
        "columnas.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "the multicolumn extraction must succeed, got {outcome:?}"
    );
    let text: String = conn
        .query_row(
            "SELECT text_content FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("text read");
    let left = text.find("IZQUIERDA").expect("left column text");
    let right = text.find("DERECHA").expect("right column text");
    assert!(
        left < right,
        "the left column must read before the right one: {}",
        text.chars().take(160).collect::<String>()
    );
    let left2 = text.find("izquierda").expect("left second line");
    let right2 = text.find("derecha").expect("right second line");
    assert!(
        left2 < right2,
        "column order holds across lines: {}",
        text.chars().take(160).collect::<String>()
    );
    let assets: i64 = conn
        .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
        .expect("asset count");
    assert_eq!(assets, 0, "no corpus assets in the extraction loop");
    let ocr_tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE kind = 'ocr'",
            [],
            |row| row.get(0),
        )
        .expect("ocr task count");
    assert_eq!(ocr_tasks, 0, "no OCR tasks in the extraction loop");
}

// ── E4b-WU2: per-page native texts ─────────────────────────────────────────

/// Each page's native text lands in its own row with its own hash and
/// quality, beside the whole-document row — the shape selective OCR reads
/// to skip rich pages.
#[test]
fn extract_task_stores_per_page_native_texts() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "PAGEWORK1", "Obra de dos paginas", "Resumen.");
    let pdf = make_text_pdf_pages(&[
        &[(50.0, 750.0, "Primera pagina con contenido nativo suficiente para superar el umbral de calidad sin problemas")],
        &[(50.0, 750.0, "ok")],
    ]);
    let path = write_temp_pdf(&dir, "dos.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "PAGEATT01",
        "linked_file",
        Some(&path),
        "dos.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "the paged extraction must succeed, got {outcome:?}"
    );

    let pages: Vec<(i64, String, String, String)> = conn
        .prepare(
            "SELECT page_number, method, quality, text_content
             FROM bibliographic_page_texts WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .expect("pages query")
        .query_map([&attachment_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("pages map")
        .collect::<Result<Vec<_>, _>>()
        .expect("pages collect");
    assert_eq!(pages.len(), 2, "one row per document page");
    assert_eq!(pages[0].0, 1);
    assert_eq!(pages[0].1, "native");
    assert_eq!(pages[0].2, "rich");
    assert!(
        pages[0].3.contains("Primera pagina"),
        "page one carries its own text"
    );
    assert_eq!(
        (pages[1].0, pages[1].1.as_str(), pages[1].2.as_str()),
        (2, "native", "sparse")
    );
    assert!(pages[1].3.contains("ok"));
    // The whole-document row is untouched by the per-page fill.
    let whole: (i64, String) = conn
        .query_row(
            "SELECT page_count, quality FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("whole-document row");
    assert_eq!(whole, (2, "rich".to_string()));
}

/// A page no decoder can safely read records `unreadable` with empty text
/// instead of failing its siblings: the extraction succeeds with what
/// exists, and selective OCR sees exactly which page needs another path.
#[test]
fn oversized_pages_record_unreadable_without_failing_siblings() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "BOMBWORK1", "Obra con pagina enorme", "Resumen.");
    let mut big_lines: Vec<(f32, f32, String)> = Vec::new();
    for index in 0..150_000u32 {
        big_lines.push((
            50.0,
            750.0,
            format!("linea repetida de relleno numero {index}"),
        ));
    }
    let big_refs: Vec<(f32, f32, &str)> = big_lines
        .iter()
        .map(|(x, y, text)| (*x, *y, text.as_str()))
        .collect();
    let pdf = make_text_pdf_pages(&[
        &[(
            50.0,
            750.0,
            "Pagina primera con contenido nativo suficiente para superar el umbral de calidad",
        )],
        &big_refs,
    ]);
    let path = write_temp_pdf(&dir, "bomba.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "BOMBATT01",
        "linked_file",
        Some(&path),
        "bomba.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "an unreadable page must not fail its siblings, got {outcome:?}"
    );
    let pages: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT page_number, quality, text_content
             FROM bibliographic_page_texts WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .expect("pages query")
        .query_map([&attachment_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("pages map")
        .collect::<Result<Vec<_>, _>>()
        .expect("pages collect");
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].1, "rich");
    assert!(!pages[0].2.is_empty());
    assert_eq!(
        (pages[1].0, pages[1].1.as_str()),
        (2, "unreadable"),
        "the oversized page records unreadable"
    );
    assert!(
        pages[1].2.is_empty(),
        "an unreadable page stores no text rather than a partial lie"
    );
}

// ── E4b-WU3: selective OCR with injected renderer/provider ─────────────────

use entropia_desktop_lib::bibliography::selective_ocr::{PageOcrProvider, PageRenderer};

struct FakeRenderer {
    rendered_pages: Mutex<Vec<u32>>,
}

impl PageRenderer for FakeRenderer {
    fn render_page(&self, _pdf_bytes: &[u8], page_number: u32) -> Result<Vec<u8>, String> {
        self.rendered_pages
            .lock()
            .expect("renders")
            .push(page_number);
        Ok(vec![9, 9, 9])
    }

    fn name(&self) -> &'static str {
        "fake-renderer"
    }
}

struct FakeOcrProvider {
    calls: Mutex<Vec<usize>>,
    text: String,
    failure: Mutex<Option<String>>,
}

impl FakeOcrProvider {
    fn with_text(text: &str) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            text: text.to_string(),
            failure: Mutex::new(None),
        })
    }

    fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            text: String::new(),
            failure: Mutex::new(Some(message.to_string())),
        })
    }
}

impl PageOcrProvider for FakeOcrProvider {
    fn recognize_page(&self, image_bytes: &[u8]) -> Result<String, String> {
        self.calls.lock().expect("calls").push(image_bytes.len());
        match self.failure.lock().expect("failure").take() {
            Some(message) => Err(message),
            None => Ok(self.text.clone()),
        }
    }

    fn name(&self) -> &str {
        "fake-ocr"
    }
}

fn ocr_executor(
    renderer: Arc<FakeRenderer>,
    provider: Arc<FakeOcrProvider>,
) -> BibliographyExtractExecutor {
    BibliographyExtractExecutor::with_selective_ocr(renderer, provider)
}

/// Rich native pages never reach a provider; sparse pages are replaced by
/// the OCR text (method `ocr`) with no native fragment duplicated.
#[test]
fn selective_ocr_skips_rich_pages_and_replaces_sparse_ones() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "MIXWORK01", "Obra mixta", "Resumen.");
    let pdf = make_text_pdf_pages(&[
        &[(
            50.0,
            750.0,
            "Pagina primera con contenido nativo suficiente para superar el umbral de calidad",
        )],
        &[(50.0, 750.0, "ok")],
    ]);
    let path = write_temp_pdf(&dir, "mixto.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "MIXATT001",
        "linked_file",
        Some(&path),
        "mixto.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);

    let renderer = Arc::new(FakeRenderer {
        rendered_pages: Mutex::new(Vec::new()),
    });
    let provider = FakeOcrProvider::with_text(
        "Texto reconocido completo de la segunda pagina con suficiente longitud para ser rico",
    );
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(ocr_executor(
        Arc::clone(&renderer),
        Arc::clone(&provider),
    )));
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "the selective run must succeed, got {outcome:?}"
    );

    assert_eq!(
        renderer.rendered_pages.lock().expect("renders").as_slice(),
        &[2],
        "only the sparse page renders"
    );
    assert_eq!(
        provider.calls.lock().expect("calls").len(),
        1,
        "only the sparse page reaches the provider"
    );
    let pages: Vec<(i64, String, String, String)> = conn
        .prepare(
            "SELECT page_number, method, quality, text_content
             FROM bibliographic_page_texts WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .expect("pages query")
        .query_map([&attachment_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("pages map")
        .collect::<Result<Vec<_>, _>>()
        .expect("pages collect");
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].1, "native", "the rich page keeps its native row");
    assert!(pages[0].3.contains("Pagina primera"));
    assert_eq!(pages[1].1, "ocr", "the sparse page is replaced by OCR text");
    assert!(
        pages[1].3.contains("Texto reconocido completo"),
        "the OCR text is what got published"
    );
    assert!(
        !pages[1].3.contains("\nok\n") && !pages[1].3.starts_with("ok"),
        "no native fragment duplicates inside the OCR row: {}",
        pages[1].3.chars().take(80).collect::<String>()
    );
    // The whole-document native row is untouched by the selective pass.
    let whole: String = conn
        .query_row(
            "SELECT method FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("whole-document row");
    assert_eq!(whole, "native");
}

/// A confirmed OCR checkpoint is reused without re-sending content: resume
/// never pays the provider twice for the same page.
#[test]
fn ocr_checkpoints_resume_without_resending_content() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "RESWORK01", "Obra reanudable", "Resumen.");
    let pdf = make_text_pdf(&[(50.0, 750.0, "ok")]);
    let path = write_temp_pdf(&dir, "reanudar.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "RESATT001",
        "linked_file",
        Some(&path),
        "reanudar.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);
    let task = repository::claim_next(
        &conn,
        "extract-session",
        &["bibliography_extract"],
        repository::now_ms(),
    )
    .expect("claim scan")
    .expect("claimable");

    // A confirmed checkpoint for the OCR page, as a previous attempt would
    // have left it: correct fingerprint, contract, and checksum.
    let cached = "Texto ya reconocido en un intento anterior con longitud suficiente";
    let payload = serde_json::to_string(&cached.to_string()).expect("payload json");
    let checksum = format!("{:x}", sha2::Sha256::digest(payload.as_bytes()));
    conn.execute(
        "INSERT INTO processing_checkpoints
           (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
         VALUES (?1, 'ocr-page:1', ?2, ?3, ?4, ?5, 1)",
        rusqlite::params![
            task.task_id,
            task.input_fingerprint,
            task.contract_hash,
            payload,
            checksum
        ],
    )
    .expect("seed checkpoint");

    let renderer = Arc::new(FakeRenderer {
        rendered_pages: Mutex::new(Vec::new()),
    });
    let provider = FakeOcrProvider::failing("provider must not be called on resume");
    let result = ocr_executor(Arc::clone(&renderer), Arc::clone(&provider)).run(
        &ctx_of(&dir),
        &task,
        &StopFlag::new(),
    );
    assert!(
        matches!(result.output, ExecOutput::Success { .. }),
        "the cached checkpoint carries the run, got {:?}",
        result.output
    );
    assert!(
        renderer.rendered_pages.lock().expect("renders").is_empty(),
        "resume renders nothing"
    );
    assert!(
        provider.calls.lock().expect("calls").is_empty(),
        "resume sends nothing to the provider"
    );
    let engine_output = result.engine_output.expect("staged output");
    let profiles = match &engine_output {
        entropia_desktop_lib::processing::scheduler::EngineOutput::BibliographyExtract(output) => {
            output.pages.clone()
        }
        _ => unreachable!("test only stages extraction output"),
    };
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].method, "ocr");
    assert!(profiles[0].text_content.contains("ya reconocido"));
}

/// Provider failures map honestly: transient retries, auth parks blocked.
#[test]
fn ocr_provider_errors_map_to_retry_or_block() {
    for (message, terminal) in [
        ("request timed out after 30s", "waiting"),
        ("GLM-OCR no está configurado: cargá una API key", "blocked"),
    ] {
        let (dir, mut conn) = migrated_db();
        seed_library(&conn, "lib-1", Some(7));
        let item_id = seed_catalog(&mut conn, "ERRWORK01", "Obra con error", "Resumen.");
        let pdf = make_text_pdf(&[(50.0, 750.0, "ok")]);
        let path = write_temp_pdf(&dir, "error.pdf", &pdf);
        let attachment_id = seed_attachment(
            &mut conn,
            &item_id,
            "ERRATT001",
            "linked_file",
            Some(&path),
            "error.pdf",
            "application/pdf",
        );
        admit_extract_demand(&conn, &attachment_id);
        let renderer = Arc::new(FakeRenderer {
            rendered_pages: Mutex::new(Vec::new()),
        });
        let provider = FakeOcrProvider::failing(message);
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(ocr_executor(renderer, provider)));
        let outcome = run_one(
            &conn,
            &ctx_of(&dir),
            &registry,
            "extract-session",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("extract run");
        match terminal {
            "waiting" => assert!(
                matches!(outcome, RunOneOutcome::Waiting { .. }),
                "{message} must retry, got {outcome:?}"
            ),
            _ => assert!(
                matches!(outcome, RunOneOutcome::Blocked { .. }),
                "{message} must park blocked, got {outcome:?}"
            ),
        }
    }
}
// ── E4b-WU4: partial OCR failures and locked files ─────────────────────────

/// A page whose OCR hard-fails keeps its native row and is named in the
/// receipt: the task succeeds with what exists instead of failing its
/// siblings, and nothing is silently left behind.
#[test]
fn ocr_partial_failure_keeps_native_and_names_failed_pages() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "PARTWORK1", "Obra parcial", "Resumen.");
    let pdf = make_text_pdf_pages(&[
        &[(
            50.0,
            750.0,
            "Pagina primera con contenido nativo suficiente para superar el umbral de calidad",
        )],
        &[(50.0, 750.0, "ok")],
        &[(50.0, 750.0, "ok")],
    ]);
    let path = write_temp_pdf(&dir, "parcial.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "PARTATT001",
        "linked_file",
        Some(&path),
        "parcial.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);

    let renderer = Arc::new(FakeRenderer {
        rendered_pages: Mutex::new(Vec::new()),
    });
    // Fail the first provider call: page 2 (the first sparse page) keeps
    // native while page 3 resolves normally (page 1 never calls at all).
    let provider = Arc::new(FailingNthOcrProvider {
        calls: Mutex::new(0),
        fail_on_call: 0,
        text: "Texto reconocido de la tercera pagina con longitud suficiente para ser rico"
            .to_string(),
    });
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(BibliographyExtractExecutor::with_selective_ocr(
        renderer, provider,
    )));
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry,
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "a per-page OCR failure must not fail its siblings, got {outcome:?}"
    );

    let pages: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT page_number, method, text_content
             FROM bibliographic_page_texts WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .expect("pages query")
        .query_map([&attachment_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("pages map")
        .collect::<Result<Vec<_>, _>>()
        .expect("pages collect");
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[0].1, "native", "the rich page is untouched");
    assert_eq!(pages[1].1, "native", "the failed page keeps its native row");
    assert!(
        pages[1].2.contains("ok"),
        "native content survives the failed OCR"
    );
    assert_eq!(pages[2].1, "ocr", "the third page still resolves");
    assert!(pages[2].2.contains("Texto reconocido"));

    let receipt: String = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE kind = 'bibliography_extract' AND subject_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("receipt read");
    let receipt: serde_json::Value = serde_json::from_str(&receipt).expect("receipt JSON");
    assert_eq!(
        receipt["ocrFailedPages"],
        serde_json::json!([2]),
        "the receipt names exactly the failed page: {receipt}"
    );
}

struct FailingNthOcrProvider {
    calls: Mutex<usize>,
    fail_on_call: usize,
    text: String,
}

impl PageOcrProvider for FailingNthOcrProvider {
    fn recognize_page(&self, _image_bytes: &[u8]) -> Result<String, String> {
        let mut calls = self.calls.lock().expect("calls");
        let index = *calls;
        *calls += 1;
        if index == self.fail_on_call {
            return Err("splines reticulated unexpectedly".to_string());
        }
        Ok(self.text.clone())
    }

    fn name(&self) -> &str {
        "fake-ocr-nth"
    }
}

/// A PDF that needs a real user password fails with unlock guidance, not with a complaint
/// about damage that is not there.
#[test]
fn encrypted_pdf_fails_with_unlock_guidance() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "LOCKWORK1", "Obra bloqueada", "Resumen.");
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pdf-aes128-user-password.pdf");
    assert!(fixture.is_file(), "the locked fixture must exist");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "LOCKATT001",
        "linked_file",
        Some(&fixture.to_string_lossy()),
        "pdf-aes128-user-password.pdf",
        "application/pdf",
    );
    let task_id = admit_extract_demand(&conn, &attachment_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    match outcome {
        RunOneOutcome::Failed { task_id: failed } => assert_eq!(failed, task_id),
        other => panic!("a locked PDF must fail honestly, got {other:?}"),
    }
    let message: Option<String> = conn
        .query_row(
            "SELECT last_error_message FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("error message");
    let message = message.unwrap_or_default();
    assert!(
        message.contains("contrase") || message.contains("protegido"),
        "the error must name the lock and the way out, got: {message}"
    );
}

/// A PDF with `/Encrypt` but an EMPTY user password is permissions-only
/// protection (journal articles ship this way): anyone can open it, so the
/// extractor must read it instead of reporting a lock.
fn run_permissions_only_extraction(fixture_name: &str) -> (String, Vec<(i64, String)>) {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "PERMWORK01", "Obra con permisos", "Resumen.");
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture_name);
    assert!(fixture.is_file(), "the fixture must exist");
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "PERMATT001",
        "linked_file",
        Some(&fixture.to_string_lossy()),
        fixture_name,
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "a permissions-only PDF must be read, got {outcome:?}"
    );
    let text: String = conn
        .query_row(
            "SELECT text_content FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .expect("text read");
    let pages = conn
        .prepare(
            "SELECT page_number, text_content FROM bibliographic_page_texts
             WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .expect("pages query")
        .query_map([&attachment_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("pages map")
        .collect::<Result<Vec<_>, _>>()
        .expect("pages collect");
    (text, pages)
}

#[test]
fn rc4_permissions_only_pdf_is_extracted() {
    let (text, pages) = run_permissions_only_extraction("pdf-rc4-128-empty-user-text.pdf");
    assert!(text.contains("Permissions only"), "whole text: {text}");
    assert_eq!(pages.len(), 1);
    assert!(
        pages[0].1.contains("Permissions only"),
        "page text: {:?}",
        pages[0].1
    );
}

#[test]
fn aes_permissions_only_pdf_is_extracted() {
    let (text, pages) = run_permissions_only_extraction("pdf-aes128-empty-user-text.pdf");
    assert!(text.contains("Permissions only"), "whole text: {text}");
    assert_eq!(pages.len(), 1);
    assert!(
        pages[0].1.contains("Permissions only"),
        "page text: {:?}",
        pages[0].1
    );
}

// ── E4c-WU2: chunk embeddings published atomically with the profile ───────

fn seed_attachment_with_pages(
    conn: &mut rusqlite::Connection,
    item_id: &str,
    attachment_key: &str,
    pages: &[(i64, &str)],
) -> String {
    // Returns the attachment row id for later page edits.
    let attachment = entropia_desktop_lib::bibliography::repository::upsert_attachment(
        conn,
        item_id,
        entropia_desktop_lib::bibliography::repository::AttachmentInput {
            attachment_key: attachment_key.to_string(),
            content_type: Some("application/pdf".to_string()),
            link_mode: Some("linked_file".to_string()),
            filename: Some("doc.pdf".to_string()),
            native_path: None,
            url: None,
            md5: None,
            mtime: None,
            native_json_snapshot: r#"{"key":"x"}"#.to_string(),
            native_version: None,
        },
    )
    .expect("seed attachment");
    for (page_number, text) in pages {
        conn.execute(
            "INSERT INTO bibliographic_page_texts
               (attachment_id, page_number, method, text_content, text_hash,
                text_chars, quality, created_at, updated_at)
             VALUES (?1, ?2, 'native', ?3, ?4, ?5, 'rich', 1, 1)",
            rusqlite::params![
                attachment.id,
                page_number,
                text,
                format!("hash-{page_number}"),
                text.chars().count() as i64
            ],
        )
        .expect("seed page text");
    }
    attachment.id.clone()
}

/// A profile run segments the work's pages, embeds each chunk, and
/// publishes profile, chunks, spans, and vectors atomically — with the
/// receipt naming the chunk count.
#[test]
fn profile_run_publishes_chunks_and_vectors_atomically() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "CHUNK001", "Obra con texto", "Resumen.");
    let page_a = "Contenido de la primera pagina con suficiente longitud para existir por si mismo en el indice. ".repeat(6);
    let page_b = "Contenido de la segunda pagina tambien extenso para forzar un segundo fragmento estructural. ".repeat(6);
    seed_attachment_with_pages(
        &mut conn,
        &item_id,
        "CHUNKATT1",
        &[(1, &page_a), (2, &page_b)],
    );
    let task_id = admit_profile_demand(&conn, &item_id);

    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("profile run");
    match outcome {
        RunOneOutcome::Succeeded { task_id: done } => assert_eq!(done, task_id),
        other => panic!("the profile run must succeed, got {other:?}"),
    }

    let chunks: Vec<(String, i64, String)> = conn
        .prepare(
            "SELECT id, ordinal, text_content FROM bibliographic_chunks
             WHERE item_id = ?1 ORDER BY ordinal",
        )
        .expect("chunks query")
        .query_map([&item_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("chunks map")
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks collect");
    assert_eq!(
        chunks.len(),
        2,
        "two long pages chunk separately, got {}",
        chunks.len()
    );
    assert_eq!(chunks[0].0, format!("{item_id}:000000"));
    assert_eq!(chunks[0].1, 0);
    assert!(chunks[0].2.contains("primera pagina"));
    assert_eq!(chunks[1].0, format!("{item_id}:000001"));
    assert!(chunks[1].2.contains("segunda pagina"));

    let spans: Vec<(String, i64)> = conn
        .prepare(
            "SELECT chunk_id, page_number FROM bibliographic_chunk_spans ORDER BY chunk_id, page_number",
        )
        .expect("spans query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("spans map")
        .collect::<Result<Vec<_>, _>>()
        .expect("spans collect");
    assert_eq!(
        spans,
        vec![(chunks[0].0.clone(), 1), (chunks[1].0.clone(), 2),],
        "each chunk spans exactly its page"
    );

    let effective =
        entropia_desktop_lib::processing::eligibility::resolve_effective_embedding_contract(&conn)
            .expect("effective contract");
    let staging: String = conn
        .query_row(
            "SELECT id FROM bibliographic_index_generations
             WHERE contract_hash = ?1 AND status IN ('staging', 'active')",
            [&effective.hash],
            |row| row.get(0),
        )
        .expect("generation of the run");
    let vectors: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT chunk_id, generation_id, input_hash FROM bibliographic_chunk_embeddings ORDER BY chunk_id",
        )
        .expect("vectors query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("vectors map")
        .collect::<Result<Vec<_>, _>>()
        .expect("vectors collect");
    assert_eq!(vectors.len(), 2);
    for (chunk_id, generation_id, input_hash) in &vectors {
        assert_eq!(
            generation_id, &staging,
            "vectors stamp the run's generation (activated by its last publish)"
        );
        let chunk_hash: String = conn
            .query_row(
                "SELECT text_hash FROM bibliographic_chunks WHERE id = ?1",
                [chunk_id],
                |row| row.get(0),
            )
            .expect("chunk hash");
        assert_eq!(input_hash, &chunk_hash, "each vector stamps its chunk hash");
    }
    let receipt: String = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("receipt read");
    assert!(
        receipt.contains("\"chunkCount\":2"),
        "the receipt names the chunk count: {receipt}"
    );
}

/// Editing a page re-profiles into a replaced chunk set: new text lands,
/// stale rows vanish, and no orphan chunks or vectors survive.
#[test]
fn reprofile_replaces_chunks_without_orphans() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "CHUNK002", "Obra mutable", "Resumen.");
    let page_a =
        "Texto original de la primera pagina con extension suficiente para chunkear. ".repeat(6);
    seed_attachment_with_pages(&mut conn, &item_id, "CHUNKATT2", &[(1, &page_a)]);
    admit_profile_demand(&conn, &item_id);
    let first = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("first profile run");
    assert!(matches!(first, RunOneOutcome::Succeeded { .. }));

    let before: Vec<String> = conn
        .prepare("SELECT id FROM bibliographic_chunks WHERE item_id = ?1 ORDER BY ordinal")
        .expect("chunks query")
        .query_map([&item_id], |row| row.get(0))
        .expect("chunks map")
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks collect");
    assert_eq!(before.len(), 1);

    // The page text moves (same length class, different words): the profile
    // hash is catalog-bound, so re-demand the profile explicitly like a
    // metadata edit would.
    let page_a2 =
        "Texto corregido de la primera pagina con extension suficiente para chunkear. ".repeat(6);
    let attachment_row: String = conn
        .query_row(
            "SELECT id FROM zotero_attachments WHERE attachment_key = 'CHUNKATT2'",
            [],
            |row| row.get(0),
        )
        .expect("attachment row");
    conn.execute(
        "UPDATE bibliographic_page_texts SET text_content = ?1 WHERE attachment_id = ?2",
        rusqlite::params![page_a2, attachment_row],
    )
    .expect("edit page text");
    let task2 = admit_profile_demand(&conn, &item_id);
    let second = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("second profile run");
    assert!(
        matches!(second, RunOneOutcome::Succeeded { task_id: ref done } if done == &task2),
        "the re-profile must succeed, got {second:?}"
    );

    let after: Vec<(String, String)> = conn
        .prepare(
            "SELECT id, text_content FROM bibliographic_chunks WHERE item_id = ?1 ORDER BY ordinal",
        )
        .expect("chunks query")
        .query_map([&item_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("chunks map")
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks collect");
    assert_eq!(after.len(), 1, "the set is replaced, not appended");
    assert_eq!(after[0].0, before[0], "stable ordinals keep stable ids");
    assert!(
        after[0].1.contains("corregido"),
        "the new text is what got published"
    );
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_chunk_embeddings e
             LEFT JOIN bibliographic_chunks c ON c.id = e.chunk_id
             WHERE c.id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("orphan count");
    assert_eq!(orphans, 0, "no orphan vectors survive the replace");
    let vector_hash: String = conn
        .query_row(
            "SELECT input_hash FROM bibliographic_chunk_embeddings WHERE chunk_id = ?1",
            [&after[0].0],
            |row| row.get(0),
        )
        .expect("vector hash");
    let chunk_hash: String = conn
        .query_row(
            "SELECT text_hash FROM bibliographic_chunks WHERE id = ?1",
            [&after[0].0],
            |row| row.get(0),
        )
        .expect("chunk hash");
    assert_eq!(
        vector_hash, chunk_hash,
        "the vector stamps the current chunk hash"
    );
}

// ── E4c-WU3: invalidation on text and contract change ──────────────────────

fn live_profile_tasks_for(conn: &rusqlite::Connection, item_id: &str) -> Vec<String> {
    conn.prepare(
        "SELECT t.id FROM processing_tasks t
         WHERE t.domain = 'bibliography' AND t.subject_kind = 'item'
           AND t.kind = 'bibliography_profile' AND t.subject_id = ?1
           AND t.state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')
         ORDER BY t.created_at, t.id",
    )
    .expect("live profile query")
    .query_map([item_id], |row| row.get(0))
    .expect("live profile map")
    .collect::<Result<Vec<_>, _>>()
    .expect("live profile collect")
}

/// A successful extraction whose pages moved re-demands the profile, and
/// the profile run converges chunks end-to-end: extract → profile →
/// chunks + vectors, all through durable demand.
#[test]
fn extract_success_chains_profile_demand_when_pages_move() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "INVWORK01", "Obra invalida", "Resumen.");
    let pdf = make_text_pdf(&[(
        50.0,
        750.0,
        "Contenido extraible para invalidacion con longitud suficiente",
    )]);
    let path = write_temp_pdf(&dir, "invalida.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "INVATT001",
        "linked_file",
        Some(&path),
        "invalida.pdf",
        "application/pdf",
    );
    let extract_task = admit_extract_demand(&conn, &attachment_id);
    let extracted = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run");
    assert!(
        matches!(&extracted, RunOneOutcome::Succeeded { task_id } if task_id == &extract_task),
        "extract must succeed, got {extracted:?}"
    );

    // The moved page layer chained exactly one live profile demand.
    let live = live_profile_tasks_for(&conn, &item_id);
    assert_eq!(live.len(), 1, "one profile demand must chain, got {live:?}");

    // Draining it converges chunks and vectors.
    let profiled = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("profile run");
    assert!(
        matches!(&profiled, RunOneOutcome::Succeeded { task_id } if task_id == &live[0]),
        "the chained demand must run, got {profiled:?}"
    );
    let chunks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_chunks WHERE item_id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .expect("chunk count");
    assert_eq!(chunks, 1, "the converged run chunks the extracted pages");
    let vectors: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_chunk_embeddings e
             JOIN bibliographic_chunks c ON c.id = e.chunk_id
             WHERE c.item_id = ?1",
            [&item_id],
            |row| row.get(0),
        )
        .expect("vector count");
    assert_eq!(vectors, 1);
}

/// Re-extracting identical bytes moves no pages, so nothing chains: the
/// queue stays silent instead of re-profiling unchanged works.
#[test]
fn identical_reextract_chains_no_profile_demand() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "INVWORK02", "Obra estable", "Resumen.");
    let pdf = make_text_pdf(&[(
        50.0,
        750.0,
        "Contenido estable para reextraccion con longitud suficiente",
    )]);
    let path = write_temp_pdf(&dir, "estable.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "INVATT002",
        "linked_file",
        Some(&path),
        "estable.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);
    let first = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("first extract");
    assert!(matches!(first, RunOneOutcome::Succeeded { .. }));
    // Drain the chained profile demand so only the re-extract remains.
    let _ = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("profile drain");
    assert!(
        live_profile_tasks_for(&conn, &item_id).is_empty(),
        "the chained demand must have drained"
    );

    // A second extraction of identical bytes succeeds silently.
    admit_extract_demand(&conn, &attachment_id);
    let second = run_one(
        &conn,
        &ctx_of(&dir),
        &extract_registry(),
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("second extract");
    assert!(
        matches!(second, RunOneOutcome::Succeeded { .. }),
        "identical re-extract must succeed, got {second:?}"
    );
    assert!(
        live_profile_tasks_for(&conn, &item_id).is_empty(),
        "unchanged pages chain no new profile demand"
    );
}

/// A contract switch moves chunk vectors to a new staging generation while
/// the old generation's rows stay intact — then the switch activates.
#[test]
fn contract_switch_moves_chunks_to_a_new_generation() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "INVWORK03", "Obra versionada", "Resumen.");
    let page = "Texto versionado para cambio de contrato con extension suficiente. ".repeat(6);
    seed_attachment_with_pages(&mut conn, &item_id, "INVATT003", &[(1, &page)]);
    admit_profile_demand(&conn, &item_id);
    let first = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("first profile run");
    assert!(matches!(first, RunOneOutcome::Succeeded { .. }));
    let old_gen: String = conn
        .query_row(
            "SELECT generation_id FROM bibliographic_chunk_embeddings LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("old generation");

    // Switch the effective contract: the old task pin can no longer run,
    // and fresh demand lands in a new staging generation.
    conn.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);")
        .expect("settings table");
    conn.execute(
        "INSERT INTO app_settings(key, value) VALUES ('openrouter_embedding_model', 'custom/model')",
        [],
    )
    .expect("custom model");
    let new_task = admit_profile_demand(&conn, &item_id);
    let second = run_one(
        &conn,
        &ctx_of(&dir),
        &profile_only_registry(),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("second profile run");
    assert!(
        matches!(&second, RunOneOutcome::Succeeded { task_id } if task_id == &new_task),
        "fresh demand under the new contract must succeed, got {second:?}"
    );
    let new_gen: String = conn
        .query_row(
            "SELECT generation_id FROM bibliographic_chunk_embeddings
             WHERE generation_id != ?1 LIMIT 1",
            [&old_gen],
            |row| row.get(0),
        )
        .expect("new generation");
    assert_ne!(old_gen, new_gen, "the switch mints a distinct generation");
    let old_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_chunk_embeddings WHERE generation_id = ?1",
            [&old_gen],
            |row| row.get(0),
        )
        .expect("old rows");
    assert!(
        old_rows > 0,
        "the previous space stays intact until the switch"
    );
    let new_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_chunk_embeddings WHERE generation_id = ?1",
            [&new_gen],
            |row| row.get(0),
        )
        .expect("new rows");
    assert!(new_rows > 0);
}

/// E5c: processing starts only after verified Zotero completion. A linked
/// work with a matching receipt verifies and admits profile demand; the
/// demand lands in the batch queue behind the bibliography subject.
#[test]
fn ingest_gate_admits_profile_demand_after_verified_completion() {
    let (_dir, mut conn) = migrated_db();
    let library_id = "lib-e5c";
    seed_library(&conn, library_id, Some(7));
    let item = upsert_item(
        &mut conn,
        library_id,
        BibliographicItemInput {
            item_key: "GATE0001".to_string(),
            item_version: Some(3),
            native_json_snapshot: r#"{"key":"GATE0001","version":3}"#.to_string(),
            csl_json_snapshot: r#"{"id":"GATE0001","type":"book","title":"Obra puerta"}"#
                .to_string(),
            title: Some("Obra puerta".to_string()),
            ..Default::default()
        },
    )
    .expect("seed work");
    let op = record_ingest_decision(
        &conn,
        "req-e5c",
        &IngestDecision {
            kind: KIND_LINK_MATCH.to_string(),
            library_id: library_id.to_string(),
            payload_json: format!(r#"{{"mode":"link","item_id":"{}"}}"#, item.id),
        },
    )
    .expect("record");
    let done = run_ingest_operation(&mut conn, &op.id, &CatalogIngestTransport).expect("run");
    assert_eq!(done.state, "succeeded");
    let work = verify_ingest_receipt(&conn, &op.id).expect("verify");
    assert_eq!(work.item_key, "GATE0001");
    let admission = admit_verified_work(&conn, &work).expect("admit");
    assert!(
        admission.profile_demands_admitted >= 1,
        "the verified work admits profile demand"
    );
    let profile_tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_tasks WHERE kind = 'bibliography_profile'",
            [],
            |row| row.get(0),
        )
        .expect("count profile demands");
    assert!(profile_tasks >= 1, "the demand reaches the batch queue");
}

/// E2b-live: one real scheduler sync against the isolated `prueba` group
/// through the production local page source. Runs only with
/// `ZSB_LIVE_ZOTERO=1` and a reachable local Zotero. Reads only: it admits
/// a bibliography_sync task, drives run_one to settlement, and asserts the
/// catalog converges on the group's native keys and versions.
#[test]
fn live_sync_converges_on_the_test_group() {
    use entropia_desktop_lib::bibliography::processing::LocalZoteroPageSource;

    if std::env::var("ZSB_LIVE_ZOTERO").is_err() {
        eprintln!("skipping live Zotero sync: ZSB_LIVE_ZOTERO is not set");
        return;
    }
    let (dir, conn) = migrated_db();
    conn.execute(
        "INSERT OR IGNORE INTO zotero_connections
           (id, source_origin, source_instance_id, endpoint, capabilities_json,
            state, revision, created_at, updated_at)
         VALUES ('conn-1', 'local', NULL, 'http://127.0.0.1:23119', '{}',
                 'available', 0, 1, 1)",
        [],
    )
    .expect("seed connection");
    conn.execute(
        "INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name,
                                        last_modified_version, revision, created_at, updated_at)
         VALUES ('lib-live', 'conn-1', 'group', '6680944', 'prueba', NULL, 1, 1, 1)",
        [],
    )
    .expect("seed group library");
    let task_id = admit_bibliography_task(&conn, "lib-live");

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(
        BibliographySyncExecutor::new(Arc::new(LocalZoteroPageSource::new())).with_page_limit(25),
    ));
    let mut settled = String::new();
    for _ in 0..15 {
        run_one(
            &conn,
            &ctx_of(&dir),
            &registry,
            "bib-live-session",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("run one live bibliography unit");
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = ?1",
                [&task_id],
                |row| row.get(0),
            )
            .expect("task state");
        if state == "succeeded" || state == "failed" || state == "stopped" {
            settled = state;
            break;
        }
    }
    assert_eq!(settled, "succeeded", "the live sync settles successfully");

    // Native keys observed live in the group earlier today converge with
    // their Zotero versions; the probe write is among them.
    // Top-level works only: child attachments (e.g. MXSRFWBP) never
    // appear in `/items/top` pages; attachment ingestion is E4's job.
    for (key, min_version) in [("H5ZYZXRU", 18), ("9PMECDNK", 22), ("P3CQ8WCJ", 26)] {
        let version: Option<i64> = conn
            .query_row(
                "SELECT i.item_version FROM bibliographic_items i
                 JOIN zotero_libraries l ON l.id = i.library_id
                 WHERE i.item_key = ?1 AND l.library_id = '6680944'",
                [key],
                |row| row.get(0),
            )
            .expect("synced work");
        assert!(
            version.unwrap_or(0) >= min_version,
            "{key} converges at its Zotero version"
        );
    }
    let library_version: Option<i64> = conn
        .query_row(
            "SELECT last_modified_version FROM zotero_libraries WHERE id = 'lib-live'",
            [],
            |row| row.get(0),
        )
        .expect("library version");
    assert!(
        library_version.unwrap_or(0) >= 26,
        "the library cursor advances past the observed writes"
    );
}

/// E7c first real metrics: the user's 2026-09-24 judgments over the
/// synthetic eval works, scored against an honest lexical-only run (the
/// embed closure fails, so the answer is labeled lexical-only).
///
/// Baseline truth pinned here: q2 (`helechos tropicales`) recalls 1.0 —
/// both words are in the title — while q1 (a natural question whose
/// grammar words miss the profile) recalls 0.0 under the AND combination.
/// That zero is the documented reason the vector leg exists; if lexical
/// improves (stopwords/Any mode), update these numbers with the new run.
#[test]
fn eval_seed_scores_lexical_baseline() {
    use entropia_desktop_lib::bibliography::eval::{evaluate_run, load_eval_seed, EvalRun};
    use entropia_desktop_lib::bibliography::profile::{
        build_profile, ProfileInput, BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
    };
    use entropia_desktop_lib::bibliography::repository::upsert_semantic_profile;
    use entropia_desktop_lib::bibliography::retrieval::{search_works, HybridQuery, WorkFilters};

    let (_dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-eval", Some(1));
    for (key, title, abstract_text, item_type) in [
        (
            "Z6NVPS2J",
            "Revoluciones agrarias del siglo XIX: un estudio inventado",
            "Artículo inventado sobre revoluciones agrarias. Sin contenido real.",
            "journalArticle",
        ),
        (
            "Z3GRPJVN",
            "Manual apócrifo de helechos tropicales",
            "Obra inventada sobre helechos tropicales. Sin contenido real.",
            "book",
        ),
        (
            "3RFSTNUF",
            "Tratado sintético de mareas lunares",
            "Obra inventada sobre la influencia lunar en las mareas. Sin contenido real.",
            "book",
        ),
    ] {
        let item = upsert_item(
            &mut conn,
            "lib-eval",
            BibliographicItemInput {
                item_key: key.to_string(),
                item_version: Some(1),
                native_json_snapshot: format!(r#"{{"key":"{key}","version":1}}"#),
                csl_json_snapshot: format!(
                    r#"{{"id":"{key}","type":"{item_type}","title":{title:?}}}"#
                ),
                title: Some(title.to_string()),
                ..Default::default()
            },
        )
        .expect("seed eval work");
        let built = build_profile(&ProfileInput {
            title: title.to_string(),
            creators: vec![("Supuesta".to_string(), "Carla".to_string())],
            year: Some(2024),
            item_type: item_type.to_string(),
            publication: "Revista Imaginaria".to_string(),
            abstract_text: abstract_text.to_string(),
            tags: vec!["zsb-eval".to_string()],
        });
        upsert_semantic_profile(
            &mut conn,
            &item.id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &built.canonical_text,
            &built.input_hash,
            "[]",
            1,
        )
        .expect("seed eval profile");
    }

    let judged_full =
        load_eval_seed(include_str!("./fixtures/zsb-eval-v1.json")).expect("seed loads");
    // v1 baseline scope: the original two queries over the three original works.
    let judged = judged_full.into_iter().take(2).collect::<Vec<_>>();
    assert_eq!(judged.len(), 2, "two judged queries");
    let mut runs = Vec::new();
    for query in &judged {
        let answer = search_works(
            &conn,
            "zsb-eval-contract",
            &HybridQuery {
                text: query.query_text.clone(),
                top_k: 5,
                filters: WorkFilters::default(),
            },
            &|_| Err("no vectors in the lexical baseline".to_string()),
        )
        .expect("lexical search");
        assert!(!answer.vector_available, "baseline stays lexical-only");
        runs.push(EvalRun {
            query_id: query.query_id.clone(),
            ranked_item_ids: answer.hits.iter().map(|hit| hit.item_key.clone()).collect(),
        });
    }
    let metrics = evaluate_run(&judged, &runs, 5);
    assert_eq!(metrics.queries_scored, 2);
    let q1 = &metrics.per_query[0];
    let q2 = &metrics.per_query[1];
    assert_eq!(
        (q1.recall_at_k, q1.reciprocal_rank),
        (0.0, 0.0),
        "q1 needs the vector leg"
    );
    assert_eq!(
        (q2.recall_at_k, q2.reciprocal_rank),
        (1.0, 1.0),
        "q2 answers lexically"
    );
    eprintln!(
        "E7c lexical baseline over human judgments: recall@{k}={:.3} ndcg@{k}={:.3} mrr={:.3}",
        metrics.mean_recall_at_k,
        metrics.mean_ndcg_at_k,
        metrics.mean_reciprocal_rank,
        k = 5,
    );
}

/// E7c vector-leg measurement: the same judgments scored against real
/// `baai/bge-m3` vectors through the production OpenRouter client.
/// Runs only with `ZSB_LIVE_EMBEDDINGS=1` and the configured OpenRouter key
/// (resolved from the local app credential store exactly like production —
/// never printed, never committed). Five embedding calls total.
#[test]
fn eval_seed_scores_vector_leg_with_bge_m3() {
    use entropia_desktop_lib::bibliography::eval::{
        compare_runs, evaluate_run, load_eval_seed, EvalRun,
    };
    use entropia_desktop_lib::bibliography::generation::{
        begin_index_generation, complete_index_generation, register_embedding_contract,
        set_generation_manifest, EmbeddingContractRow,
    };
    use entropia_desktop_lib::bibliography::profile::{
        build_profile, ProfileInput, BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
    };
    use entropia_desktop_lib::bibliography::repository::upsert_semantic_profile;
    use entropia_desktop_lib::bibliography::retrieval::{search_works, HybridQuery, WorkFilters};
    use entropia_desktop_lib::{EmbeddingConfig, EmbeddingEngine, EmbeddingProvider};

    if std::env::var("ZSB_LIVE_EMBEDDINGS").is_err() {
        eprintln!("skipping live embeddings: ZSB_LIVE_EMBEDDINGS is not set");
        return;
    }
    let app_conn = rusqlite::Connection::open_with_flags(
        "C:/Users/agusn/AppData/Roaming/com.entropia.shared/entropia.sqlite",
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open app db read-only");
    let api_key =
        entropia_desktop_lib::get_setting(&app_conn, entropia_desktop_lib::OPENROUTER_API_KEY)
            .filter(|key| !key.trim().is_empty());
    let Some(api_key) = api_key else {
        eprintln!("skipping live embeddings: no OpenRouter key configured");
        return;
    };
    let engine = EmbeddingEngine::init(EmbeddingConfig {
        provider: EmbeddingProvider::Api,
        api_key,
        model_name: "baai/bge-m3".to_string(),
        // The Pro build adds the local-model fields; the API provider ignores
        // them (8192 mirrors the crate's DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH).
        #[cfg(feature = "local-ml")]
        local_model_dir: None,
        #[cfg(feature = "local-ml")]
        local_model_path: None,
        #[cfg(feature = "local-ml")]
        local_tokenizer_path: None,
        #[cfg(feature = "local-ml")]
        local_max_length: 8192,
    })
    .expect("init embedding engine");

    let (_dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-eval", Some(1));
    let works = [
        (
            "Z6NVPS2J",
            "Revoluciones agrarias del siglo XIX: un estudio inventado",
            "Artículo inventado sobre revoluciones agrarias. Sin contenido real.",
            "journalArticle",
        ),
        (
            "Z3GRPJVN",
            "Manual apócrifo de helechos tropicales",
            "Obra inventada sobre helechos tropicales. Sin contenido real.",
            "book",
        ),
        (
            "3RFSTNUF",
            "Tratado sintético de mareas lunares",
            "Obra inventada sobre la influencia lunar en las mareas. Sin contenido real.",
            "book",
        ),
    ];
    let mut profile_texts = Vec::new();
    for (key, title, abstract_text, item_type) in works {
        let item = upsert_item(
            &mut conn,
            "lib-eval",
            BibliographicItemInput {
                item_key: key.to_string(),
                item_version: Some(1),
                native_json_snapshot: format!(r#"{{"key":"{key}","version":1}}"#),
                csl_json_snapshot: format!(
                    r#"{{"id":"{key}","type":"{item_type}","title":{title:?}}}"#
                ),
                title: Some(title.to_string()),
                ..Default::default()
            },
        )
        .expect("seed eval work");
        let built = build_profile(&ProfileInput {
            title: title.to_string(),
            creators: vec![("Supuesta".to_string(), "Carla".to_string())],
            year: Some(2024),
            item_type: item_type.to_string(),
            publication: "Revista Imaginaria".to_string(),
            abstract_text: abstract_text.to_string(),
            tags: vec!["zsb-eval".to_string()],
        });
        upsert_semantic_profile(
            &mut conn,
            &item.id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &built.canonical_text,
            &built.input_hash,
            "[]",
            1,
        )
        .expect("seed eval profile");
        profile_texts.push((
            item.id,
            key.to_string(),
            built.canonical_text,
            built.input_hash,
        ));
    }

    // Real vectors for the three profiles through the production client.
    register_embedding_contract(
        &conn,
        &EmbeddingContractRow {
            contract_hash: "zsb-eval-bgem3".to_string(),
            provider: "api".to_string(),
            model: "baai/bge-m3".to_string(),
            dimensions: 1024,
            chunking_contract: "bibliography-profile-v1".to_string(),
        },
        1,
    )
    .expect("register contract");
    let generation =
        begin_index_generation(&conn, "zsb-eval-bgem3", "gen-eval", 10).expect("begin");
    for (item_id, _key, text, hash) in &profile_texts {
        let vector = engine.embed_text(text).expect("embed profile");
        assert_eq!(vector.len(), 1024, "bge-m3 dimensionality");
        let blob: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO bibliographic_item_embeddings
               (item_id, generation_id, embedding_contract, embedding_model,
                dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
             VALUES (?1, ?2, 'zsb-eval-bgem3', 'baai/bge-m3', 1024, ?3, ?4, 1, 1, 1)",
            rusqlite::params![item_id, generation.id, blob, hash],
        )
        .expect("publish vector");
    }
    set_generation_manifest(&conn, &generation.id, 3, 4).expect("manifest");
    for _ in 0..3 {
        entropia_desktop_lib::bibliography::generation::note_generation_progress(
            &conn,
            &generation.id,
        )
        .expect("progress");
    }
    complete_index_generation(&mut conn, &generation.id, 20).expect("activate");

    let judged_full =
        load_eval_seed(include_str!("./fixtures/zsb-eval-v1.json")).expect("seed loads");
    // v1 baseline scope: the original two queries over the three original works.
    let judged = judged_full.into_iter().take(2).collect::<Vec<_>>();
    let embed = |text: &str| engine.embed_text(text).map_err(|error| error.to_string());
    let mut vector_runs = Vec::new();
    let mut lexical_runs = Vec::new();
    for query in &judged {
        let hybrid = search_works(
            &conn,
            "zsb-eval-bgem3",
            &HybridQuery {
                text: query.query_text.clone(),
                top_k: 5,
                filters: WorkFilters::default(),
            },
            &embed,
        )
        .expect("hybrid search");
        assert!(hybrid.vector_available, "the space is queryable");
        vector_runs.push(EvalRun {
            query_id: query.query_id.clone(),
            ranked_item_ids: hybrid.hits.iter().map(|hit| hit.item_key.clone()).collect(),
        });
        let lexical = search_works(
            &conn,
            "zsb-eval-bgem3",
            &HybridQuery {
                text: query.query_text.clone(),
                top_k: 5,
                filters: WorkFilters::default(),
            },
            &|_| Err("lexical baseline".to_string()),
        )
        .expect("lexical search");
        lexical_runs.push(EvalRun {
            query_id: query.query_id.clone(),
            ranked_item_ids: lexical
                .hits
                .iter()
                .map(|hit| hit.item_key.clone())
                .collect(),
        });
    }
    let vector_metrics = evaluate_run(&judged, &vector_runs, 5);
    let comparison = compare_runs(&judged, &lexical_runs, &vector_runs, 5);
    eprintln!(
        "E7c vector leg (bge-m3) over human judgments: recall@5={:.3} ndcg@5={:.3} mrr={:.3} | delta vs lexical: recall={:+.3} ndcg={:+.3} mrr={:+.3} improved={} tied={} regressed={}",
        vector_metrics.mean_recall_at_k,
        vector_metrics.mean_ndcg_at_k,
        vector_metrics.mean_reciprocal_rank,
        comparison.mean_delta_recall,
        comparison.mean_delta_ndcg,
        comparison.mean_delta_reciprocal_rank,
        comparison.improved,
        comparison.tied,
        comparison.regressed,
    );
    assert_eq!(vector_metrics.queries_scored, 2);
    assert!(
        vector_metrics.mean_recall_at_k >= 1.0 - 1e-12,
        "vectors answer both judged queries"
    );
    assert!(
        comparison.mean_delta_ndcg > 0.0 && comparison.regressed == 0,
        "the vector leg strictly improves on lexical with no regressions"
    );
}

/// E7c extended set: 5 works / 4 user-judged queries with distractors and
/// paraphrases. Measures lexical, hybrid (bge-m3), and a rerank candidate
/// through compare_runs. The rerank client is MEASUREMENT-ONLY (test-local,
/// OpenRouter rerank endpoint with the production default model):
/// production wiring lands only on sustained positive deltas.
#[test]
fn eval_seed_scores_extended_set_with_rerank_candidate() {
    use entropia_desktop_lib::bibliography::eval::{
        compare_runs, evaluate_run, load_eval_seed, EvalRun,
    };
    use entropia_desktop_lib::bibliography::generation::{
        begin_index_generation, complete_index_generation, note_generation_progress,
        register_embedding_contract, set_generation_manifest, EmbeddingContractRow,
    };
    use entropia_desktop_lib::bibliography::profile::{
        build_profile, ProfileInput, BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
    };
    use entropia_desktop_lib::bibliography::repository::upsert_semantic_profile;
    use entropia_desktop_lib::bibliography::retrieval::{search_works, HybridQuery, WorkFilters};
    use entropia_desktop_lib::{EmbeddingConfig, EmbeddingEngine, EmbeddingProvider};

    if std::env::var("ZSB_LIVE_EMBEDDINGS").is_err() {
        eprintln!("skipping live embeddings: ZSB_LIVE_EMBEDDINGS is not set");
        return;
    }
    let app_conn = rusqlite::Connection::open_with_flags(
        "C:/Users/agusn/AppData/Roaming/com.entropia.shared/entropia.sqlite",
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open app db read-only");
    let api_key =
        entropia_desktop_lib::get_setting(&app_conn, entropia_desktop_lib::OPENROUTER_API_KEY)
            .filter(|key| !key.trim().is_empty());
    let Some(api_key) = api_key else {
        eprintln!("skipping live embeddings: no OpenRouter key configured");
        return;
    };
    let engine = EmbeddingEngine::init(EmbeddingConfig {
        provider: EmbeddingProvider::Api,
        api_key: api_key.clone(),
        model_name: "baai/bge-m3".to_string(),
        // The Pro build adds the local-model fields; the API provider ignores
        // them (8192 mirrors the crate's DEFAULT_LOCAL_EMBEDDING_MAX_LENGTH).
        #[cfg(feature = "local-ml")]
        local_model_dir: None,
        #[cfg(feature = "local-ml")]
        local_model_path: None,
        #[cfg(feature = "local-ml")]
        local_tokenizer_path: None,
        #[cfg(feature = "local-ml")]
        local_max_length: 8192,
    })
    .expect("init embedding engine");

    let (_dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-eval", Some(1));
    let works = [
        (
            "Z6NVPS2J",
            "Revoluciones agrarias del siglo XIX: un estudio inventado",
            "Artículo inventado sobre revoluciones agrarias. Sin contenido real.",
            "journalArticle",
        ),
        (
            "Z3GRPJVN",
            "Manual apócrifo de helechos tropicales",
            "Obra inventada sobre helechos tropicales. Sin contenido real.",
            "book",
        ),
        (
            "3RFSTNUF",
            "Tratado sintético de mareas lunares",
            "Obra inventada sobre la influencia lunar en las mareas. Sin contenido real.",
            "book",
        ),
        (
            "ZSBW0004",
            "Revoluciones industriales y máquinas de vapor",
            "Obra inventada sobre industria y vapor. Sin contenido real.",
            "book",
        ),
        (
            "ZSBW0005",
            "Luchas campesinas decimonónicas en Europa",
            "Obra inventada sobre luchas campesinas. Sin contenido real.",
            "book",
        ),
    ];
    // item_key -> canonical profile text, for rerank documents.
    let mut profile_texts: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for (key, title, abstract_text, item_type) in works {
        let item = upsert_item(
            &mut conn,
            "lib-eval",
            BibliographicItemInput {
                item_key: key.to_string(),
                item_version: Some(1),
                native_json_snapshot: format!(r#"{{"key":"{key}","version":1}}"#),
                csl_json_snapshot: format!(
                    r#"{{"id":"{key}","type":"{item_type}","title":{title:?}}}"#
                ),
                title: Some(title.to_string()),
                ..Default::default()
            },
        )
        .expect("seed eval work");
        let built = build_profile(&ProfileInput {
            title: title.to_string(),
            creators: vec![("Supuesta".to_string(), "Carla".to_string())],
            year: Some(2024),
            item_type: item_type.to_string(),
            publication: "Revista Imaginaria".to_string(),
            abstract_text: abstract_text.to_string(),
            tags: vec!["zsb-eval".to_string()],
        });
        upsert_semantic_profile(
            &mut conn,
            &item.id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &built.canonical_text,
            &built.input_hash,
            "[]",
            1,
        )
        .expect("seed eval profile");
        profile_texts.insert(key.to_string(), built.canonical_text.clone());
    }

    register_embedding_contract(
        &conn,
        &EmbeddingContractRow {
            contract_hash: "zsb-eval-x".to_string(),
            provider: "api".to_string(),
            model: "baai/bge-m3".to_string(),
            dimensions: 1024,
            chunking_contract: "bibliography-profile-v1".to_string(),
        },
        1,
    )
    .expect("register contract");
    let generation = begin_index_generation(&conn, "zsb-eval-x", "gen-eval-x", 10).expect("begin");
    set_generation_manifest(&conn, &generation.id, 5, 4).expect("manifest");
    for (key, _title, _abstract_text, _item_type) in works {
        let item_id: String = conn
            .query_row(
                "SELECT i.id FROM bibliographic_items i WHERE i.item_key = ?1",
                [key],
                |row| row.get(0),
            )
            .expect("eval work id");
        let text = profile_texts.get(key).expect("profile text");
        let vector = engine.embed_text(text).expect("embed profile");
        assert_eq!(vector.len(), 1024, "bge-m3 dimensionality");
        let hash: String = conn
            .query_row(
                "SELECT input_hash FROM bibliographic_semantic_profiles WHERE item_id = ?1",
                [&item_id],
                |row| row.get(0),
            )
            .expect("input hash");
        let blob: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO bibliographic_item_embeddings
               (item_id, generation_id, embedding_contract, embedding_model,
                dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
             VALUES (?1, 'gen-eval-x', 'zsb-eval-x', 'baai/bge-m3', 1024, ?2, ?3, 1, 1, 1)",
            rusqlite::params![item_id, blob, hash],
        )
        .expect("stage vector");
        note_generation_progress(&conn, &generation.id).expect("progress");
    }
    complete_index_generation(&mut conn, &generation.id, 20).expect("activate");

    let judged_full =
        load_eval_seed(include_str!("./fixtures/zsb-eval-v1.json")).expect("seed loads");
    assert_eq!(judged_full.len(), 4, "extended seed judges four queries");
    let embed = |text: &str| engine.embed_text(text).map_err(|error| error.to_string());
    let mut lexical_runs = Vec::new();
    let mut hybrid_runs = Vec::new();
    let mut rerank_runs = Vec::new();
    for query in &judged_full {
        let lexical = search_works(
            &conn,
            "zsb-eval-x",
            &HybridQuery {
                text: query.query_text.clone(),
                top_k: 5,
                filters: WorkFilters::default(),
            },
            &|_| Err("lexical baseline".to_string()),
        )
        .expect("lexical search");
        lexical_runs.push(EvalRun {
            query_id: query.query_id.clone(),
            ranked_item_ids: lexical
                .hits
                .iter()
                .map(|hit| hit.item_key.clone())
                .collect(),
        });
        let hybrid = search_works(
            &conn,
            "zsb-eval-x",
            &HybridQuery {
                text: query.query_text.clone(),
                top_k: 5,
                filters: WorkFilters::default(),
            },
            &embed,
        )
        .expect("hybrid search");
        assert!(hybrid.vector_available, "the space is queryable");
        let documents: Vec<String> = hybrid
            .hits
            .iter()
            .map(|hit| {
                profile_texts
                    .get(&hit.item_key)
                    .cloned()
                    .unwrap_or_else(|| hit.title.clone())
            })
            .collect();
        let reranked_keys =
            rerank_documents(&api_key, &query.query_text, &documents).expect("rerank candidate");
        rerank_runs.push(EvalRun {
            query_id: query.query_id.clone(),
            ranked_item_ids: reranked_keys
                .into_iter()
                .map(|index| hybrid.hits[index].item_key.clone())
                .collect(),
        });
        hybrid_runs.push(EvalRun {
            query_id: query.query_id.clone(),
            ranked_item_ids: hybrid.hits.iter().map(|hit| hit.item_key.clone()).collect(),
        });
    }
    let lexical = evaluate_run(&judged_full, &lexical_runs, 5);
    let hybrid = evaluate_run(&judged_full, &hybrid_runs, 5);
    let reranked = evaluate_run(&judged_full, &rerank_runs, 5);
    let hybrid_vs_lexical = compare_runs(&judged_full, &lexical_runs, &hybrid_runs, 5);
    let rerank_vs_hybrid = compare_runs(&judged_full, &hybrid_runs, &rerank_runs, 5);
    eprintln!(
        "E7c extended (5 works, 4 judgments): lexical r={:.3} n={:.3} m={:.3} | hybrid r={:.3} n={:.3} m={:.3} | rerank r={:.3} n={:.3} m={:.3} | rerank-vs-hybrid d_n={:+.3} w/t/l={}/{}/{}",
        lexical.mean_recall_at_k,
        lexical.mean_ndcg_at_k,
        lexical.mean_reciprocal_rank,
        hybrid.mean_recall_at_k,
        hybrid.mean_ndcg_at_k,
        hybrid.mean_reciprocal_rank,
        reranked.mean_recall_at_k,
        reranked.mean_ndcg_at_k,
        reranked.mean_reciprocal_rank,
        rerank_vs_hybrid.mean_delta_ndcg,
        rerank_vs_hybrid.improved,
        rerank_vs_hybrid.tied,
        rerank_vs_hybrid.regressed,
    );
    assert_eq!(hybrid.queries_scored, 4);
    assert_eq!(reranked.queries_scored, 4);
    assert_eq!(
        hybrid_vs_lexical.regressed, 0,
        "hybrid keeps every lexical win on the extended set"
    );
}

/// Test-local rerank client (measurement only, no production use):
/// OpenRouter rerank endpoint with the production default model.
fn rerank_documents(
    api_key: &str,
    query: &str,
    documents: &[String],
) -> Result<Vec<usize>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("rerank client: {error}"))?;
    let response = client
        .post("https://openrouter.ai/api/v1/rerank")
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "model": "cohere/rerank-4-fast",
            "query": query,
            "documents": documents,
            "top_n": documents.len(),
        }))
        .send()
        .map_err(|error| format!("rerank request: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("rerank rejected: {}", response.status()));
    }
    let body: serde_json::Value = response
        .json()
        .map_err(|error| format!("rerank response: {error}"))?;
    let mut scored: Vec<(usize, f64)> = body
        .get("results")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "rerank response without results".to_string())?
        .iter()
        .filter_map(|entry| {
            Some((
                entry.get("index")?.as_u64()? as usize,
                entry.get("relevance_score")?.as_f64()?,
            ))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored.into_iter().map(|(index, _)| index).collect())
}

/// Regression: nothing in production ever created the connection/library
/// rows, so the first "Sincronizar biblioteca" failed with `unknown_library`.
/// A sync request now registers the local Zotero connection and the
/// requested library itself, idempotently and without moving the fence.
#[test]
fn a_sync_request_registers_the_local_library_in_an_empty_catalog() {
    let (_dir, conn) = migrated_db();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).expect("count") };
    assert_eq!(count("SELECT COUNT(*) FROM zotero_libraries"), 0);

    let first = apply_bibliography_sync_request(&conn, "req-1", "user", "0")
        .expect("an empty catalog still admits the sync");
    assert!(first.created);
    assert_eq!(count("SELECT COUNT(*) FROM zotero_connections"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM zotero_libraries"), 1);
    let (origin, name, ltype, lid): (String, String, String, String) = conn
        .query_row(
            "SELECT c.source_origin, l.name, l.library_type, l.library_id
               FROM zotero_libraries l JOIN zotero_connections c ON c.id = l.connection_id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("registered rows");
    assert_eq!(
        (origin.as_str(), name.as_str(), ltype.as_str(), lid.as_str()),
        ("local", "Mi biblioteca", "user", "0")
    );
    let fence_before = count("SELECT revision FROM zotero_connections");

    // A new request id for the same library attaches to the same live task
    // and neither duplicates rows nor bumps the connection fence.
    let again = apply_bibliography_sync_request(&conn, "req-2", "user", "0").expect("repeat");
    assert_eq!(again.task_id, first.task_id);
    assert!(!again.created);
    assert_eq!(count("SELECT COUNT(*) FROM zotero_connections"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM zotero_libraries"), 1);
    assert_eq!(
        count("SELECT revision FROM zotero_connections"),
        fence_before
    );

    // A group registers under the same connection with a neutral name.
    let group = apply_bibliography_sync_request(&conn, "req-3", "group", "6238085")
        .expect("group admitted");
    assert_ne!(group.task_id, first.task_id);
    assert_eq!(count("SELECT COUNT(*) FROM zotero_connections"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM zotero_libraries"), 2);

    // A namespace owned twice stays an honest ambiguity, never re-registered.
    conn.execute(
        "INSERT INTO zotero_connections
           (id, source_origin, endpoint, capabilities_json, state, revision, created_at, updated_at)
         VALUES ('conn-x', 'local', 'http://x.invalid', '{}', 'available', 0, 1, 1)",
        [],
    )
    .expect("second connection");
    conn.execute(
        "INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name,
                                        revision, created_at, updated_at)
         VALUES ('lib-x', 'conn-x', 'user', '0', 'Other', 0, 1, 1)",
        [],
    )
    .expect("duplicate namespace");
    let ambiguous = apply_bibliography_sync_request(&conn, "req-4", "user", "0")
        .expect_err("ambiguity must surface");
    assert!(ambiguous.contains("ambiguous_library"), "{ambiguous}");
    assert_eq!(count("SELECT COUNT(*) FROM zotero_libraries"), 3);
}

// ---------------------------------------------------------------------------
// B1: the library sync catalogs each work's PDF attachments.
// ---------------------------------------------------------------------------

/// A native attachment row as `/items?itemType=attachment&format=json`
/// answers it: parent in `data.parentItem`, file location in
/// `links.enclosure.href` (a percent-encoded `file:` URL).
fn attachment_row(
    key: &str,
    parent: Option<&str>,
    content_type: &str,
    link_mode: &str,
    filename: &str,
    enclosure_path: Option<&str>,
) -> serde_json::Value {
    let mut data = serde_json::json!({
        "key": key,
        "version": 11,
        "itemType": "attachment",
        "title": filename,
        "linkMode": link_mode,
        "contentType": content_type,
        "filename": filename,
        "md5": "ce907ea9fd7c7b4fadb6735e6949ede1",
        "mtime": 1_790_984_898_525_i64,
        "url": "https://example.invalid/descargar",
    });
    if let Some(parent) = parent {
        data["parentItem"] = serde_json::json!(parent);
    }
    let mut row = serde_json::json!({ "key": key, "version": 11, "links": {}, "data": data });
    if let Some(path) = enclosure_path {
        let normalized = path.replace('\\', "/").replace(' ', "%20");
        let href = if normalized.starts_with('/') {
            format!("file://{normalized}")
        } else {
            format!("file:///{normalized}")
        };
        row["links"]["enclosure"] = serde_json::json!({ "href": href, "type": content_type });
    }
    row
}

#[test]
fn attachment_page_keeps_pdfs_and_stored_web_snapshots_with_a_parent_and_decodes_the_enclosure() {
    use entropia_desktop_lib::bibliography::processing::attachment_page_from_json;
    let mut linked = attachment_row(
        "LINKED01",
        Some("WORK0002"),
        "application/pdf",
        "linked_file",
        "externo.pdf",
        None,
    );
    linked["data"]["path"] = serde_json::json!("C:/Libros/externo.pdf");
    let body = serde_json::json!([
        attachment_row(
            "PDFATT01",
            Some("WORK0001"),
            "application/pdf",
            "imported_url",
            "mi adjunto.pdf",
            Some("C:/Users/ana/Zotero/storage/PDFATT01/mi adjunto.pdf"),
        ),
        attachment_row(
            "HTMLATT1",
            Some("WORK0001"),
            "text/html",
            "imported_url",
            "snap.html",
            Some("C:/Users/ana/Zotero/storage/HTMLATT1/snap.html"),
        ),
        attachment_row(
            "ORPHAN01",
            None,
            "application/pdf",
            "imported_file",
            "solo.pdf",
            None,
        ),
        linked,
        attachment_row(
            "XHTMLAT1",
            Some("WORK0003"),
            "application/xhtml+xml",
            "imported_file",
            "pagina.xhtml",
            Some("C:/Users/ana/Zotero/storage/XHTMLAT1/pagina.xhtml"),
        ),
        // A bare web link has no stored file: nothing to read, nothing to catalog.
        attachment_row(
            "WEBLINK1",
            Some("WORK0001"),
            "text/html",
            "linked_url",
            "",
            None,
        ),
        attachment_row(
            "PNGATT01",
            Some("WORK0001"),
            "image/png",
            "imported_file",
            "figura.png",
            Some("C:/Users/ana/Zotero/storage/PNGATT01/figura.png"),
        ),
    ]);
    let page = attachment_page_from_json(&body, Some(7)).expect("page parses");
    assert_eq!(page.rows_read, 7, "pagination counts every row read");
    assert_eq!(page.total, Some(7));
    let keys: Vec<&str> = page
        .attachments
        .iter()
        .map(|a| a.input.attachment_key.as_str())
        .collect();
    assert_eq!(
        keys,
        ["PDFATT01", "HTMLATT1", "LINKED01", "XHTMLAT1"],
        "PDF and stored HTML snapshots with a parent; no orphans, bare links or images"
    );
    let pdf = &page.attachments[0];
    assert_eq!(pdf.parent_key, "WORK0001");
    assert_eq!(
        pdf.input.native_path.as_deref(),
        Some("C:/Users/ana/Zotero/storage/PDFATT01/mi adjunto.pdf"),
        "the enclosure is percent-decoded to a local path"
    );
    assert_eq!(pdf.input.link_mode.as_deref(), Some("imported_url"));
    assert_eq!(pdf.input.filename.as_deref(), Some("mi adjunto.pdf"));
    assert_eq!(pdf.input.native_version, Some(11));
    assert_eq!(pdf.input.mtime, Some(1_790_984_898_525));
    let snapshot = &page.attachments[1];
    assert_eq!(snapshot.parent_key, "WORK0001");
    assert_eq!(snapshot.input.content_type.as_deref(), Some("text/html"));
    assert_eq!(
        snapshot.input.native_path.as_deref(),
        Some("C:/Users/ana/Zotero/storage/HTMLATT1/snap.html")
    );
    let linked = &page.attachments[2];
    assert_eq!(
        linked.input.native_path.as_deref(),
        Some("C:/Libros/externo.pdf"),
        "an absolute data.path backs a linked file without an enclosure"
    );
}

#[test]
fn attachment_enclosures_resolve_for_unix_paths_and_ignore_remote_urls() {
    use entropia_desktop_lib::bibliography::processing::attachment_page_from_json;
    let mut unix = attachment_row(
        "UNIXATT1",
        Some("W1"),
        "application/pdf",
        "imported_file",
        "a.pdf",
        None,
    );
    unix["links"]["enclosure"] =
        serde_json::json!({ "href": "file:///home/ana/Zotero/storage/UNIXATT1/a%20b.pdf" });
    let mut remote = attachment_row(
        "WEBATT01",
        Some("W1"),
        "application/pdf",
        "linked_url",
        "a.pdf",
        None,
    );
    remote["links"]["enclosure"] = serde_json::json!({ "href": "https://example.invalid/a.pdf" });
    let page = attachment_page_from_json(&serde_json::json!([unix, remote]), None).expect("parses");
    assert_eq!(
        page.attachments[0].input.native_path.as_deref(),
        Some("/home/ana/Zotero/storage/UNIXATT1/a b.pdf")
    );
    assert_eq!(page.attachments[1].input.native_path, None);
}

#[test]
fn attachment_page_with_an_unreadable_row_is_an_invalid_response() {
    use entropia_desktop_lib::bibliography::processing::attachment_page_from_json;
    let not_a_list = attachment_page_from_json(&serde_json::json!({}), None);
    assert!(matches!(
        not_a_list,
        Err(ZoteroState::InvalidResponse { .. })
    ));
    let keyless =
        serde_json::json!([{ "version": 3, "data": { "contentType": "application/pdf" } }]);
    assert!(matches!(
        attachment_page_from_json(&keyless, None),
        Err(ZoteroState::InvalidResponse { .. })
    ));
}

type AttachmentScript = Result<(serde_json::Value, Option<u64>), ZoteroState>;

/// Items come from a script; attachment pages are native JSON bodies parsed
/// through the production parser, so the fake cannot drift from the shape.
struct AttachmentSource {
    items: FakeSource,
    attachments: Mutex<VecDeque<AttachmentScript>>,
    attachment_requests: Mutex<Vec<(u32, u32)>>,
}

impl AttachmentSource {
    fn new(items: Vec<ScriptStep>, attachments: Vec<AttachmentScript>) -> Self {
        Self {
            items: FakeSource::new(items),
            attachments: Mutex::new(attachments.into_iter().collect()),
            attachment_requests: Mutex::new(Vec::new()),
        }
    }
}

impl ZoteroPageSource for AttachmentSource {
    fn fetch_page(&self, library: &Library, query: BibliographyPageQuery) -> PageFuture {
        self.items.fetch_page(library, query)
    }

    fn fetch_attachment_page(
        &self,
        _library: &Library,
        query: BibliographyPageQuery,
    ) -> Option<entropia_desktop_lib::bibliography::processing::AttachmentPageFuture> {
        self.attachment_requests
            .lock()
            .expect("attachment requests")
            .push((query.start(), query.limit()));
        let step = self
            .attachments
            .lock()
            .expect("attachments")
            .pop_front()
            .unwrap_or_else(|| Ok((serde_json::json!([]), Some(0))));
        Some(Box::pin(async move {
            let (body, total) = step?;
            entropia_desktop_lib::bibliography::processing::attachment_page_from_json(&body, total)
        }))
    }
}

fn run_sync_with(
    dir: &tempfile::TempDir,
    conn: &rusqlite::Connection,
    source: Arc<AttachmentSource>,
) -> RunOneOutcome {
    run_one(
        conn,
        &ctx_of(dir),
        &registry_with(BibliographySyncExecutor::new(source).with_page_limit(2)),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("sync run")
}

fn two_works() -> Vec<ScriptStep> {
    vec![ScriptStep::Page(page(
        vec![
            custom_page_item("PDFWORK01", 1, "Obra con PDF", "Resumen."),
            custom_page_item("URLWORK01", 1, "Obra con enlace", "Resumen."),
        ],
        Some(2),
    ))]
}

fn attachment_keys(conn: &rusqlite::Connection) -> Vec<String> {
    conn.prepare("SELECT attachment_key FROM zotero_attachments ORDER BY attachment_key")
        .expect("attachment keys query")
        .query_map([], |row| row.get(0))
        .expect("attachment keys map")
        .collect::<Result<Vec<_>, _>>()
        .expect("attachment keys")
}

#[test]
fn sync_catalogs_pdf_attachments_and_chains_extraction_for_the_readable_one() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let sync_task = admit_bibliography_task(&conn, "lib-1");
    let pdf = make_text_pdf(&[(
        50.0,
        750.0,
        "Texto extraible del adjunto con longitud suficiente para calidad",
    )]);
    let path = write_temp_pdf(&dir, "mi adjunto.pdf", &pdf);
    // Three rows over two pages of two: PDF, HTML snapshot, then an orphan
    // whose parent is not in the catalog.
    let first = serde_json::json!([
        attachment_row(
            "PDFATT01",
            Some("PDFWORK01"),
            "application/pdf",
            "imported_url",
            "mi adjunto.pdf",
            Some(&path),
        ),
        attachment_row(
            "HTMLATT1",
            Some("URLWORK01"),
            "text/html",
            "imported_url",
            "snap.html",
            Some(&path),
        ),
    ]);
    let second = serde_json::json!([attachment_row(
        "GHOSTATT",
        Some("NOTSYNCED"),
        "application/pdf",
        "imported_file",
        "fantasma.pdf",
        Some(&path),
    )]);
    let source = Arc::new(AttachmentSource::new(
        two_works(),
        vec![Ok((first, Some(3))), Ok((second, Some(3)))],
    ));
    let outcome = run_sync_with(&dir, &conn, Arc::clone(&source));
    assert!(
        matches!(&outcome, RunOneOutcome::Succeeded { task_id } if task_id == &sync_task),
        "sync must succeed, got {outcome:?}"
    );
    assert_eq!(
        *source.attachment_requests.lock().expect("requests"),
        vec![(0, 2), (2, 2)],
        "attachments are walked in bounded pages, not one request per work"
    );
    assert_eq!(
        attachment_keys(&conn),
        ["HTMLATT1", "PDFATT01"],
        "PDF and HTML snapshot children of cataloged works are stored"
    );
    let (attachment_id, native_path, parent): (String, String, String) = conn
        .query_row(
            "SELECT id, native_path, item_id FROM zotero_attachments
              WHERE attachment_key = 'PDFATT01'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("stored attachment");
    assert_eq!(parent, items_by_key(&conn, "PDFWORK01"));
    assert_eq!(
        std::path::Path::new(&native_path),
        std::path::Path::new(&path),
        "the decoded enclosure is the readable location"
    );
    let live: Vec<String> = conn
        .prepare(
            "SELECT subject_id FROM processing_tasks
              WHERE domain = 'bibliography' AND subject_kind = 'attachment'
                AND kind = 'bibliography_extract' AND state = 'pending'",
        )
        .expect("live query")
        .query_map([], |row| row.get(0))
        .expect("live map")
        .collect::<Result<Vec<_>, _>>()
        .expect("live collect");
    let html_id: String = conn
        .query_row(
            "SELECT id FROM zotero_attachments WHERE attachment_key = 'HTMLATT1'",
            [],
            |row| row.get(0),
        )
        .expect("stored snapshot");
    let mut expected = vec![attachment_id, html_id];
    expected.sort();
    let mut live = live;
    live.sort();
    assert_eq!(
        live, expected,
        "the sync's own success chains extraction for the stored PDF and the snapshot"
    );
}

#[test]
fn sync_removes_attachments_that_disappeared_and_keeps_them_on_a_failed_read() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");
    let rows = |keys: &[&str]| {
        serde_json::Value::Array(
            keys.iter()
                .map(|key| {
                    attachment_row(
                        key,
                        Some("PDFWORK01"),
                        "application/pdf",
                        "imported_file",
                        "a.pdf",
                        None,
                    )
                })
                .collect(),
        )
    };
    let first = Arc::new(AttachmentSource::new(
        two_works(),
        vec![Ok((rows(&["ATTKEEP1", "ATTGONE1"]), Some(2)))],
    ));
    run_sync_with(&dir, &conn, first);
    assert_eq!(attachment_keys(&conn), ["ATTGONE1", "ATTKEEP1"]);

    // An unreadable attachments answer must not read as "all were removed".
    admit_bibliography_task(&conn, "lib-1");
    let failing = Arc::new(AttachmentSource::new(
        two_works(),
        vec![Err(ZoteroState::Timeout)],
    ));
    let outcome = run_sync_with(&dir, &conn, failing);
    assert!(
        !matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "a failed attachments read is not a completed sync, got {outcome:?}"
    );
    assert_eq!(attachment_keys(&conn), ["ATTGONE1", "ATTKEEP1"]);

    // A complete walk that no longer lists one attachment removes its row.
    let retry_id = admit_bibliography_task(&conn, "lib-1");
    conn.execute(
        "UPDATE processing_tasks SET next_retry_at=0 WHERE id=?1",
        [&retry_id],
    )
    .expect("make retry due");
    let third = Arc::new(AttachmentSource::new(
        // The retry resumes at the committed cursor: items are already read.
        vec![ScriptStep::Page(page(Vec::new(), Some(2)))],
        vec![Ok((rows(&["ATTKEEP1"]), Some(1)))],
    ));
    let outcome = run_sync_with(&dir, &conn, third);
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "got {outcome:?}"
    );
    assert_eq!(attachment_keys(&conn), ["ATTKEEP1"]);
}

#[test]
fn a_source_without_attachment_support_leaves_the_catalog_untouched() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");
    run_one(
        &conn,
        &ctx_of(&dir),
        &registry_with(executor(Arc::new(FakeSource::new(two_works())))),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("first sync");
    let item_id = items_by_key(&conn, "PDFWORK01");
    seed_attachment(
        &mut conn,
        &item_id,
        "MANUAL01",
        "imported_file",
        None,
        "a.pdf",
        "application/pdf",
    );
    admit_bibliography_task(&conn, "lib-1");
    run_one(
        &conn,
        &ctx_of(&dir),
        &registry_with(executor(Arc::new(FakeSource::new(two_works())))),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("second sync");
    assert_eq!(attachment_keys(&conn), ["MANUAL01"]);
}

/// Scale: a real library (2812 works, ~737 PDFs over 2065 attachment rows)
/// must settle, not hang after the last walk.
#[test]
fn a_library_sized_sync_settles_after_both_walks() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");
    let works = 2812usize;
    let item_pages: Vec<ScriptStep> = (0..works)
        .step_by(100)
        .map(|start| {
            ScriptStep::Page(page(
                (start..(start + 100).min(works))
                    .map(|n| {
                        custom_page_item(&format!("W{n:07}"), 1, &format!("Obra {n}"), "Resumen.")
                    })
                    .collect(),
                Some(works as u64),
            ))
        })
        .collect();
    let rows = 2065usize;
    let attachment_pages: Vec<AttachmentScript> = (0..rows)
        .step_by(100)
        .map(|start| {
            Ok((
                serde_json::Value::Array(
                    (start..(start + 100).min(rows))
                        .map(|n| {
                            attachment_row(
                                &format!("A{n:07}"),
                                Some(&format!("W{:07}", n % works)),
                                if n % 3 == 0 {
                                    "application/pdf"
                                } else {
                                    "text/html"
                                },
                                "imported_url",
                                "a.pdf",
                                None,
                            )
                        })
                        .collect(),
                ),
                Some(rows as u64),
            ))
        })
        .collect();
    let source = Arc::new(AttachmentSource::new(item_pages, attachment_pages));
    let started = std::time::Instant::now();
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry_with(BibliographySyncExecutor::new(source)),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("sync run");
    eprintln!("SCALE outcome {outcome:?} after {:?}", started.elapsed());
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
}

fn untitled_page_item(key: &str) -> BibliographyPageItem {
    // A real Zotero work can have no title (a note-like or scanned item).
    let csl = serde_json::json!({ "id": key, "type": "article-newspaper" });
    let native = serde_json::json!({ "key": key, "version": 1, "itemType": "newspaperArticle" });
    BibliographyPageItem {
        key: key.to_string(),
        item_version: 1,
        csl_json: csl.to_string(),
        native_json_snapshot: native.to_string(),
    }
}

/// Hang root cause (owner's real library): one work without a title made the
/// chained profile admission fail ("Invalid column type Null ... title"), so
/// the success publication errored.
#[test]
fn a_work_without_a_title_does_not_break_the_sync_publication() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    admit_bibliography_task(&conn, "lib-1");
    let source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![untitled_page_item("NOTITLE1"), item("TITLED01", 1)],
        Some(2),
    ))]));
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry_with(executor(source)),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("sync run");
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
}

/// Hang symptom: a publication error left the task `running` forever (alive
/// lease, nothing to resume it). It must end the attempt visibly instead.
#[test]
fn a_failing_publication_ends_the_attempt_instead_of_stranding_it_running() {
    let (dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let task_id = admit_bibliography_task(&conn, "lib-1");
    conn.execute_batch(
        "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'bogus');",
    )
    .expect("unsupported provider makes the chained admission fail");
    let source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
        vec![item("TITLED01", 1)],
        Some(1),
    ))]));
    let outcome = run_one(
        &conn,
        &ctx_of(&dir),
        &registry_with(executor(source)),
        "bib-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    );
    let state: String = conn
        .query_row(
            "SELECT state FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |r| r.get(0),
        )
        .expect("state");
    assert_ne!(
        state, "running",
        "stranded running; run_one said {outcome:?}"
    );
}

// ── Manual sync outranks the derived bibliography backlog ──────────────────

/// Inserts `count` pending derived tasks (profile and extract) linked to the
/// system bibliography batch. Their ids sort before any UUID, so a plain
/// id-ordered walk would always take them first.
fn seed_derived_backlog(conn: &rusqlite::Connection, batch_id: &str, count: usize) {
    for n in 0..count {
        for (kind, subject_kind) in [
            ("bibliography_extract", "attachment"),
            ("bibliography_profile", "item"),
        ] {
            let id = format!("00000000-{kind}-{n:03}");
            let subject = format!("subject-{kind}-{n:03}");
            conn.execute(
                "INSERT INTO processing_tasks
                   (id, kind, asset_id_snapshot, domain, subject_kind, subject_id,
                    state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'bibliography', ?4, ?3, 'pending', 1, 1)",
                rusqlite::params![id, kind, subject, subject_kind],
            )
            .expect("derived task");
            conn.execute(
                "INSERT INTO processing_batch_tasks
                   (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind,
                    subject_id, request_state)
                 VALUES (?1, ?2, ?3, ?4, 'bibliography', ?5, ?4, 'active')",
                rusqlite::params![batch_id, id, kind, subject, subject_kind],
            )
            .expect("derived link");
        }
    }
}

const ALL_BIBLIOGRAPHY_KINDS: [&str; 3] = [
    "bibliography_sync",
    "bibliography_profile",
    "bibliography_extract",
];

/// The owner's report: a requested sync sat `pending` for hours behind
/// thousands of derived tasks that share its batch (and its aged priority).
#[test]
fn a_manual_sync_is_claimed_before_the_derived_background_backlog() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let system = repository::ensure_system_batch(&conn, "bibliography").expect("system batch");
    // The aged state of the real archive: starvation aging already raised
    // the shared batch to high.
    repository::set_batch_priority(&conn, &system, 1, None).expect("aged batch");
    seed_derived_backlog(&conn, &system, 40);

    let demand =
        repository::admit_bibliography_sync_demand(&conn, "user", "0").expect("manual demand");

    let claimed = repository::claim_next(
        &conn,
        "bib-session",
        &ALL_BIBLIOGRAPHY_KINDS,
        repository::now_ms(),
    )
    .expect("claim scan")
    .expect("something is claimable");
    assert_eq!(
        claimed.task_id, demand.task_id,
        "the requested sync must not wait behind {} ({})",
        claimed.task_id, claimed.kind
    );
}

/// The lane is one extra link, not a second task: repeated clicks keep one
/// physical task, one expedite batch and one link, and the backlog keeps its
/// own batch and priority.
#[test]
fn repeated_manual_sync_demand_keeps_one_expedite_link_and_leaves_the_backlog_alone() {
    let (_dir, conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let first = repository::admit_bibliography_sync_demand(&conn, "user", "0").expect("first");
    let second = repository::admit_bibliography_sync_demand(&conn, "user", "0").expect("second");
    assert_eq!(first.task_id, second.task_id);
    assert_eq!(
        first.batch_id, second.batch_id,
        "answer keeps the system batch"
    );

    let links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_tasks WHERE task_id = ?1",
            [&first.task_id],
            |row| row.get(0),
        )
        .expect("links");
    assert_eq!(links, 2, "system batch plus the expedite batch");
    let priorities: Vec<(String, i64)> = conn
        .prepare("SELECT id, priority FROM processing_batches ORDER BY id")
        .expect("batches")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("batch map")
        .collect::<Result<_, _>>()
        .expect("batch collect");
    assert_eq!(
        priorities,
        vec![
            ("batch-system-bibliography".to_string(), 0),
            ("batch-system-bibliography-sync".to_string(), 2),
        ]
    );
}

/// A manual sync is the user's retry for work that failed for a reason since
/// fixed (the `executor_panicked` extractions): the sync that follows opens a
/// fresh extraction task for every attachment that still has no extraction.
#[test]
fn a_manual_sync_re_admits_failed_extractions_that_still_have_no_text() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let first = repository::admit_bibliography_sync_demand(&conn, "user", "0").expect("demand");
    let works = || vec![custom_page_item("PDFWORK01", 1, "Obra con PDF", "Resumen.")];
    let run_sync = |conn: &rusqlite::Connection| {
        let source = Arc::new(FakeSource::new(vec![ScriptStep::Page(page(
            works(),
            Some(1),
        ))]));
        run_one(
            conn,
            &ctx_of(&dir),
            &registry_with(executor(source)),
            "bib-session",
            repository::now_ms(),
            &|_, _| {},
            &|_, _, _, _| {},
        )
        .expect("sync run")
    };
    let outcome = run_sync(&conn);
    assert!(
        matches!(&outcome, RunOneOutcome::Succeeded { task_id } if task_id == &first.task_id),
        "first sync must succeed, got {outcome:?}"
    );

    let item_id = items_by_key(&conn, "PDFWORK01");
    let pdf = make_text_pdf(&[(
        50.0,
        750.0,
        "Texto extraible del adjunto con longitud suficiente para calidad",
    )]);
    let path = write_temp_pdf(&dir, "adjunto.pdf", &pdf);
    let attachment = seed_attachment(
        &mut conn,
        &item_id,
        "FAILPDF01",
        "linked_file",
        Some(&path),
        "adjunto.pdf",
        "application/pdf",
    );
    // The extraction panicked once and ended `failed`, leaving no text row.
    let failed_task = admit_extract_demand(&conn, &attachment);
    conn.execute(
        "UPDATE processing_tasks SET state = 'failed', outcome = 'executor_panicked'
         WHERE id = ?1",
        [&failed_task],
    )
    .expect("simulate the panic");

    let second = apply_bibliography_sync_request(&conn, "sync-after-fix", "user", "0")
        .expect("manual request");
    assert!(
        second.created,
        "the previous sync succeeded, so this is a new task"
    );
    let outcome = run_sync(&conn);
    assert!(
        matches!(&outcome, RunOneOutcome::Succeeded { task_id } if task_id == &second.task_id),
        "second sync must succeed, got {outcome:?}"
    );

    let live: Vec<String> = conn
        .prepare(
            "SELECT id FROM processing_tasks
             WHERE kind = 'bibliography_extract' AND subject_id = ?1 AND state = 'pending'",
        )
        .expect("live query")
        .query_map([&attachment], |row| row.get(0))
        .expect("live map")
        .collect::<Result<_, _>>()
        .expect("live collect");
    assert_eq!(live.len(), 1, "a new retry cycle for the failed attachment");
    assert_ne!(live[0], failed_task, "terminal history is never rewritten");
}

// ── Batched chunk embeddings: batches, skip-if-embedded, atomic failure ────

/// Records every `embed_many` call so wave sizes, order, and skipped work are
/// observable. The vector for a text is a pure function of the text, which
/// makes the order mapping checkable on the published rows.
struct RecordingEmbedder {
    model: String,
    singles: Mutex<Vec<String>>,
    batches: Mutex<Vec<Vec<String>>>,
    fail_batch_number: Option<usize>,
}

impl RecordingEmbedder {
    fn new(model: &str, fail_batch_number: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            model: model.to_string(),
            singles: Mutex::new(Vec::new()),
            batches: Mutex::new(Vec::new()),
            fail_batch_number,
        })
    }

    fn vector_for(text: &str) -> Vec<f32> {
        let seed = text.bytes().map(|byte| byte as u32).sum::<u32>() % 97 + 1;
        vec![seed as f32, 0.25, 0.5, 0.75]
    }

    fn batch_sizes(&self) -> Vec<usize> {
        self.batches
            .lock()
            .expect("batches")
            .iter()
            .map(Vec::len)
            .collect()
    }
}

impl ProfileEmbedder for RecordingEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.singles.lock().expect("singles").push(text.to_string());
        Ok(Self::vector_for(text))
    }

    fn embed_many(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let mut batches = self.batches.lock().expect("batches");
        let number = batches.len();
        batches.push(texts.to_vec());
        if self.fail_batch_number == Some(number) {
            return Err("OpenRouter embedding API error (503): upstream down".to_string());
        }
        Ok(texts.iter().map(|text| Self::vector_for(text)).collect())
    }

    fn identity(&self) -> Result<(String, String, usize), String> {
        Ok((self.model.clone(), "fake-contract".to_string(), 4))
    }
}

fn five_page_work(conn: &mut rusqlite::Connection, key: &str) -> String {
    let item_id = seed_catalog(conn, key, "Obra extensa", "Resumen.");
    let pages: Vec<String> = (1..=5)
        .map(|n| format!("Pagina numero {n} con contenido distinto. ").repeat(14))
        .collect();
    let numbered: Vec<(i64, &str)> = pages
        .iter()
        .enumerate()
        .map(|(index, text)| (index as i64 + 1, text.as_str()))
        .collect();
    seed_attachment_with_pages(conn, &item_id, &format!("ATT-{key}"), &numbered);
    item_id
}

fn run_profile_with(
    dir: &tempfile::TempDir,
    conn: &rusqlite::Connection,
    embedder: Arc<RecordingEmbedder>,
    wave: usize,
) -> RunOneOutcome {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(
        BibliographyProfileExecutor::new(embedder).with_embed_wave(wave),
    ));
    run_one(
        conn,
        &ctx_of(dir),
        &registry,
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("profile run")
}

fn chunk_vectors(conn: &rusqlite::Connection, item_id: &str) -> Vec<(String, Vec<u8>)> {
    conn.prepare(
        "SELECT c.text_content, e.embedding FROM bibliographic_chunks c
         JOIN bibliographic_chunk_embeddings e ON e.chunk_id = c.id
         WHERE c.item_id = ?1 ORDER BY c.ordinal",
    )
    .expect("vectors query")
    .query_map([item_id], |row| Ok((row.get(0)?, row.get(1)?)))
    .expect("vectors map")
    .collect::<Result<Vec<_>, _>>()
    .expect("vectors collect")
}

/// N chunks travel in ceil(N / wave) batched calls, never one by one, and
/// each published vector belongs to its own chunk text.
#[test]
fn profile_chunks_embed_in_batches_with_exact_order_mapping() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = five_page_work(&mut conn, "BATCH001");
    admit_profile_demand(&conn, &item_id);
    let embedder = RecordingEmbedder::new("fake/model", None);

    let outcome = run_profile_with(&dir, &conn, Arc::clone(&embedder), 2);

    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(embedder.batch_sizes(), vec![2, 2, 1], "ceil(5 / 2) waves");
    assert_eq!(
        embedder.singles.lock().unwrap().len(),
        1,
        "only the profile text is embedded singly"
    );
    let flattened: Vec<String> = embedder
        .batches
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .cloned()
        .collect();
    let published = chunk_vectors(&conn, &item_id);
    assert_eq!(published.len(), 5);
    for (index, (text, blob)) in published.iter().enumerate() {
        assert_eq!(&flattened[index], text, "waves keep chunk order");
        let expected: Vec<u8> = RecordingEmbedder::vector_for(text)
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert_eq!(blob, &expected, "vector {index} belongs to its own chunk");
    }
}

/// A re-profile whose chunks are unchanged embeds none of them again; a
/// different model is a different key and embeds all of them.
#[test]
fn already_embedded_chunks_are_skipped_per_text_hash_and_model() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = five_page_work(&mut conn, "SKIP0001");
    admit_profile_demand(&conn, &item_id);
    let first = RecordingEmbedder::new("fake/model", None);
    assert!(matches!(
        run_profile_with(&dir, &conn, Arc::clone(&first), 2),
        RunOneOutcome::Succeeded { .. }
    ));
    let before = chunk_vectors(&conn, &item_id);

    admit_profile_demand(&conn, &item_id);
    let second = RecordingEmbedder::new("fake/model", None);
    assert!(matches!(
        run_profile_with(&dir, &conn, Arc::clone(&second), 2),
        RunOneOutcome::Succeeded { .. }
    ));
    assert!(
        second.batches.lock().unwrap().is_empty(),
        "unchanged chunks must not be embedded again"
    );
    assert_eq!(
        chunk_vectors(&conn, &item_id),
        before,
        "vectors republished intact"
    );

    admit_profile_demand(&conn, &item_id);
    let other_model = RecordingEmbedder::new("other/model", None);
    assert!(matches!(
        run_profile_with(&dir, &conn, Arc::clone(&other_model), 2),
        RunOneOutcome::Succeeded { .. }
    ));
    assert_eq!(
        other_model.batch_sizes(),
        vec![2, 2, 1],
        "a different model never reuses another model's vectors"
    );
}

/// A failing wave leaves nothing half-published, and the retry resumes from
/// the waves that already succeeded instead of paying for them again.
#[test]
fn a_failed_wave_publishes_nothing_and_the_retry_resumes_from_checkpoints() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = five_page_work(&mut conn, "FAIL0001");
    admit_profile_demand(&conn, &item_id);
    let task = repository::claim_next(
        &conn,
        "profile-session",
        &["bibliography_profile"],
        repository::now_ms(),
    )
    .expect("claim scan")
    .expect("claimable");

    let failing = RecordingEmbedder::new("fake/model", Some(1));
    let result = BibliographyProfileExecutor::new(Arc::clone(&failing) as Arc<dyn ProfileEmbedder>)
        .with_embed_wave(2)
        .run(&ctx_of(&dir), &task, &StopFlag::new());
    assert!(
        matches!(result.output, ExecOutput::Retryable { .. }),
        "a 503 wave is a transient provider failure: {:?}",
        result.output
    );
    assert!(result.engine_output.is_none(), "nothing staged for publish");
    for table in [
        "bibliographic_semantic_profiles",
        "bibliographic_chunks",
        "bibliographic_chunk_embeddings",
        "bibliographic_item_embeddings",
    ] {
        let rows: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(rows, 0, "{table} must stay empty after a failed wave");
    }

    let healthy = RecordingEmbedder::new("fake/model", None);
    let result = BibliographyProfileExecutor::new(Arc::clone(&healthy) as Arc<dyn ProfileEmbedder>)
        .with_embed_wave(2)
        .run(&ctx_of(&dir), &task, &StopFlag::new());
    assert!(matches!(result.output, ExecOutput::Success { .. }));
    assert_eq!(
        healthy.batch_sizes(),
        vec![2, 1],
        "the first wave came back from its checkpoint"
    );
}

// ── Scanned PDFs: whole-document OCR and re-admission of empty extractions ──

/// A provider that reads whole PDFs. `pdf_failure` makes every whole-document
/// request fail; the per-page path stays available as the fallback, with its
/// own call log.
struct PdfModeProvider {
    pdf_calls: Mutex<Vec<(u32, u32)>>,
    page_calls: Mutex<usize>,
    pdf_failure: Option<String>,
}

impl PdfModeProvider {
    fn new(pdf_failure: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            pdf_calls: Mutex::new(Vec::new()),
            page_calls: Mutex::new(0),
            pdf_failure: pdf_failure.map(str::to_string),
        })
    }
}

impl PageOcrProvider for PdfModeProvider {
    fn recognize_page(&self, _image_bytes: &[u8]) -> Result<String, String> {
        *self.page_calls.lock().expect("page calls") += 1;
        Ok("Texto reconocido por pagina de respaldo con longitud suficiente para ser rico".into())
    }

    fn pdf_pages_per_request(&self) -> Option<usize> {
        Some(100)
    }

    fn recognize_pdf_pages(
        &self,
        _pdf_bytes: &[u8],
        first_page: u32,
        last_page: u32,
    ) -> Result<Vec<String>, String> {
        self.pdf_calls
            .lock()
            .expect("pdf calls")
            .push((first_page, last_page));
        if let Some(message) = &self.pdf_failure {
            return Err(message.clone());
        }
        Ok((first_page..=last_page)
            .map(|page| format!("Contenido reconocido de la pagina {page} del documento escaneado"))
            .collect())
    }

    fn name(&self) -> &str {
        "pdf-mode"
    }
}

fn seed_scanned_pdf(
    dir: &tempfile::TempDir,
    conn: &mut rusqlite::Connection,
    pages: usize,
) -> (String, String) {
    seed_library(conn, "lib-1", Some(7));
    let item_id = seed_catalog(conn, "SCANWORK01", "Obra escaneada", "Resumen.");
    let blank: &[(f32, f32, &str)] = &[];
    let pdf = make_text_pdf_pages(&vec![blank; pages]);
    let path = write_temp_pdf(dir, "escaneado.pdf", &pdf);
    let attachment_id = seed_attachment(
        conn,
        &item_id,
        "SCANATT001",
        "linked_file",
        Some(&path),
        "escaneado.pdf",
        "application/pdf",
    );
    let library: String = conn
        .query_row("SELECT id FROM zotero_libraries LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("library row");
    (library, attachment_id)
}

fn run_extract_with_provider(
    dir: &tempfile::TempDir,
    conn: &rusqlite::Connection,
    renderer: &Arc<FakeRenderer>,
    provider: Arc<dyn PageOcrProvider>,
) -> RunOneOutcome {
    let mut registry = ExecutorRegistry::new();
    let renderer: Arc<dyn PageRenderer> = renderer.clone();
    registry.register(Arc::new(BibliographyExtractExecutor::with_selective_ocr(
        renderer, provider,
    )));
    run_one(
        conn,
        &ctx_of(dir),
        &registry,
        "extract-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("extract run")
}

fn fresh_renderer() -> Arc<FakeRenderer> {
    Arc::new(FakeRenderer {
        rendered_pages: Mutex::new(Vec::new()),
    })
}

/// A fully scanned PDF is read with ONE whole-document request: no page is
/// rendered, every page keeps its own number, and the extraction stops
/// reporting `empty` now that it holds recognized text.
#[test]
fn a_scanned_pdf_is_recognized_with_one_whole_document_request() {
    let (dir, mut conn) = migrated_db();
    let (_library, attachment_id) = seed_scanned_pdf(&dir, &mut conn, 3);
    admit_extract_demand(&conn, &attachment_id);
    let renderer = fresh_renderer();
    let provider = PdfModeProvider::new(None);

    let outcome = run_extract_with_provider(&dir, &conn, &renderer, provider.clone());

    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(provider.pdf_calls.lock().unwrap().as_slice(), &[(1, 3)]);
    assert_eq!(*provider.page_calls.lock().unwrap(), 0, "no per-page call");
    assert!(
        renderer.rendered_pages.lock().unwrap().is_empty(),
        "whole-document mode renders nothing"
    );
    let pages: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT page_number, method, text_content FROM bibliographic_page_texts
             WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .unwrap()
        .query_map([&attachment_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(pages.len(), 3);
    for (index, (number, method, text)) in pages.iter().enumerate() {
        assert_eq!(*number, index as i64 + 1);
        assert_eq!(method, "ocr");
        assert!(
            text.contains(&format!("pagina {number} ")),
            "page {number} must carry its own text, got {text:?}"
        );
    }
    let (quality, chars): (String, i64) = conn
        .query_row(
            "SELECT quality, text_chars FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        quality, "rich",
        "recognized text lifts the extraction out of empty"
    );
    assert!(chars > 0);
    let receipt: String = conn
        .query_row(
            "SELECT result_receipt_json FROM processing_tasks
             WHERE kind = 'bibliography_extract' AND subject_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .unwrap();
    let receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
    assert_eq!(
        receipt["ocrAttempted"],
        serde_json::json!(true),
        "{receipt}"
    );
    assert_eq!(receipt["ocrFailedPages"], serde_json::json!([]));
}

/// Text the whole-document parser dropped is found natively, so a configured
/// OCR provider is not called for a page that already has its text.
#[test]
fn a_page_whose_text_the_parser_dropped_is_not_sent_to_ocr() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "PDFEMPTY2", "Informe cifrado", "Resumen.");
    let pdf = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pdf-rc4-40-inline-image-text.pdf"
    ))
    .expect("fixture");
    let path = write_temp_pdf(&dir, "informe.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "PDFATT004",
        "linked_file",
        Some(&path),
        "informe.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);
    let renderer = fresh_renderer();
    let provider = PdfModeProvider::new(None);

    let outcome = run_extract_with_provider(&dir, &conn, &renderer, provider.clone());

    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert!(provider.pdf_calls.lock().unwrap().is_empty());
    assert_eq!(*provider.page_calls.lock().unwrap(), 0);
    let quality: String = conn
        .query_row(
            "SELECT quality FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(quality, "rich");
}

/// More than 100 pages split into consecutive windows of at most 100.
#[test]
fn a_scanned_pdf_over_the_page_limit_is_split_into_windows() {
    let (dir, mut conn) = migrated_db();
    let (_library, attachment_id) = seed_scanned_pdf(&dir, &mut conn, 205);
    admit_extract_demand(&conn, &attachment_id);
    let renderer = fresh_renderer();
    let provider = PdfModeProvider::new(None);

    let outcome = run_extract_with_provider(&dir, &conn, &renderer, provider.clone());

    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        provider.pdf_calls.lock().unwrap().as_slice(),
        &[(1, 100), (101, 200), (201, 205)]
    );
    let page_205: String = conn
        .query_row(
            "SELECT text_content FROM bibliographic_page_texts
             WHERE attachment_id = ?1 AND page_number = 205",
            [&attachment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(page_205.contains("pagina 205 "), "{page_205}");
}

/// A whole-document request the provider rejects falls back to the
/// per-page path, so a scan is never lost to a document-mode quirk.
#[test]
fn a_rejected_whole_document_request_falls_back_to_per_page_ocr() {
    let (dir, mut conn) = migrated_db();
    let (_library, attachment_id) = seed_scanned_pdf(&dir, &mut conn, 2);
    admit_extract_demand(&conn, &attachment_id);
    let renderer = fresh_renderer();
    let provider = PdfModeProvider::new(Some("provider_error: GLM-OCR API error (400): bad range"));

    let outcome = run_extract_with_provider(&dir, &conn, &renderer, provider.clone());

    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(provider.pdf_calls.lock().unwrap().len(), 1);
    assert_eq!(*provider.page_calls.lock().unwrap(), 2);
    assert_eq!(renderer.rendered_pages.lock().unwrap().as_slice(), &[1, 2]);
}

/// Rate limits and credential problems are not document-mode quirks: they
/// keep their queue verdict and never fan out into per-page requests.
#[test]
fn a_rate_limited_whole_document_request_waits_instead_of_falling_back() {
    let (dir, mut conn) = migrated_db();
    let (_library, attachment_id) = seed_scanned_pdf(&dir, &mut conn, 2);
    admit_extract_demand(&conn, &attachment_id);
    let renderer = fresh_renderer();
    let provider = PdfModeProvider::new(Some("rate_limited: GLM-OCR API error (429): slow down"));

    let outcome = run_extract_with_provider(&dir, &conn, &renderer, provider.clone());

    assert!(
        matches!(outcome, RunOneOutcome::Waiting { .. }),
        "{outcome:?}"
    );
    assert_eq!(*provider.page_calls.lock().unwrap(), 0);
    assert!(renderer.rendered_pages.lock().unwrap().is_empty());
}

/// A mostly-native document keeps page-level OCR: native pages are never
/// re-recognized by a whole-document request.
#[test]
fn a_mostly_native_pdf_stays_on_page_level_ocr() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = seed_catalog(&mut conn, "NATIVEMIX1", "Obra nativa", "Resumen.");
    let rich: &[(f32, f32, &str)] = &[(
        50.0,
        750.0,
        "Pagina con contenido nativo suficiente para superar el umbral de calidad del extractor",
    )];
    let sparse: &[(f32, f32, &str)] = &[(50.0, 750.0, "ok")];
    let pdf = make_text_pdf_pages(&[rich, rich, rich, sparse]);
    let path = write_temp_pdf(&dir, "nativo.pdf", &pdf);
    let attachment_id = seed_attachment(
        &mut conn,
        &item_id,
        "NATIVEATT1",
        "linked_file",
        Some(&path),
        "nativo.pdf",
        "application/pdf",
    );
    admit_extract_demand(&conn, &attachment_id);
    let renderer = fresh_renderer();
    let provider = PdfModeProvider::new(None);

    let outcome = run_extract_with_provider(&dir, &conn, &renderer, provider.clone());

    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert!(provider.pdf_calls.lock().unwrap().is_empty());
    assert_eq!(*provider.page_calls.lock().unwrap(), 1);
    assert_eq!(renderer.rendered_pages.lock().unwrap().as_slice(), &[4]);
}

/// An extraction stored as `empty` before OCR could read it is demanded
/// again by the next sync, exactly once: after an OCR pass that really ran
/// it is settled.
#[test]
fn an_empty_extraction_is_readmitted_once_for_ocr() {
    let (dir, mut conn) = migrated_db();
    let (library, attachment_id) = seed_scanned_pdf(&dir, &mut conn, 2);
    // The extraction as the old build left it: a plain native run, no OCR.
    let first = admit_extract_demand(&conn, &attachment_id);
    run_extract(&dir, &conn, &first);
    let quality: String = conn
        .query_row(
            "SELECT quality FROM bibliographic_extractions WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(quality, "empty");

    assert_eq!(
        repository::admit_stale_extraction_demands(&conn, &library).expect("sync 1"),
        1,
        "an empty extraction that never went through OCR is demanded again"
    );
    let renderer = fresh_renderer();
    let provider = PdfModeProvider::new(None);
    let outcome = run_extract_with_provider(&dir, &conn, &renderer, provider.clone());
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    assert_eq!(provider.pdf_calls.lock().unwrap().len(), 1);

    assert_eq!(
        repository::admit_stale_extraction_demands(&conn, &library).expect("sync 2"),
        0,
        "recognized text settles the extraction"
    );
}

/// A scan whose OCR ran and found nothing is settled too: the sync must not
/// pay for the same blank pages on every run.
#[test]
fn a_blank_document_that_went_through_ocr_is_not_requeued() {
    struct BlankProvider;
    impl PageOcrProvider for BlankProvider {
        fn recognize_page(&self, _: &[u8]) -> Result<String, String> {
            Ok(String::new())
        }
        fn pdf_pages_per_request(&self) -> Option<usize> {
            Some(100)
        }
        fn recognize_pdf_pages(
            &self,
            _: &[u8],
            first: u32,
            last: u32,
        ) -> Result<Vec<String>, String> {
            Ok(vec![String::new(); (last - first + 1) as usize])
        }
        fn name(&self) -> &str {
            "blank"
        }
    }
    let (dir, mut conn) = migrated_db();
    let (library, attachment_id) = seed_scanned_pdf(&dir, &mut conn, 2);
    admit_extract_demand(&conn, &attachment_id);
    let outcome =
        run_extract_with_provider(&dir, &conn, &fresh_renderer(), Arc::new(BlankProvider));
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "{outcome:?}"
    );

    assert_eq!(
        repository::admit_stale_extraction_demands(&conn, &library).expect("sync"),
        0
    );
}

// ── Generation activation: a complete staging generation becomes queryable ──

fn run_profile_task(
    dir: &tempfile::TempDir,
    conn: &rusqlite::Connection,
    embedder: Arc<FakeProfileEmbedder>,
) -> RunOneOutcome {
    run_one(
        conn,
        &ctx_of(dir),
        &profile_registry(embedder),
        "profile-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("profile run")
}

fn generation_statuses(conn: &rusqlite::Connection) -> Vec<String> {
    conn.prepare("SELECT status FROM bibliographic_index_generations ORDER BY created_at, id")
        .expect("statuses query")
        .query_map([], |row| row.get(0))
        .expect("statuses map")
        .collect::<Result<_, _>>()
        .expect("statuses collect")
}

fn embedding_generation_of(conn: &rusqlite::Connection, item_id: &str) -> Vec<String> {
    conn.prepare("SELECT generation_id FROM bibliographic_item_embeddings WHERE item_id = ?1")
        .expect("generation query")
        .query_map([item_id], |row| row.get(0))
        .expect("generation map")
        .collect::<Result<_, _>>()
        .expect("generation collect")
}

/// Two works are queued: the generation must stay staging after the first
/// publish and turn active with the last one, in the same commit.
#[test]
fn the_last_profile_publish_activates_the_generation() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let a = seed_catalog(&mut conn, "ACTA0001", "Obra A", "Resumen A.");
    let b = seed_catalog(&mut conn, "ACTB0001", "Obra B", "Resumen B.");
    admit_profile_demand(&conn, &a);
    admit_profile_demand(&conn, &b);

    assert!(matches!(
        run_profile_task(&dir, &conn, FakeProfileEmbedder::ok(4)),
        RunOneOutcome::Succeeded { .. }
    ));
    assert_eq!(
        generation_statuses(&conn),
        vec!["staging"],
        "one work still owes its vector: the generation is partial"
    );

    assert!(matches!(
        run_profile_task(&dir, &conn, FakeProfileEmbedder::ok(4)),
        RunOneOutcome::Succeeded { .. }
    ));
    assert_eq!(
        generation_statuses(&conn),
        vec!["active"],
        "the publish that completes the manifest activates the generation"
    );
    let activated: Option<i64> = conn
        .query_row(
            "SELECT activated_at FROM bibliographic_index_generations",
            [],
            |row| row.get(0),
        )
        .expect("activation stamp");
    assert!(activated.is_some());
}

/// A live work whose profile failed is genuinely missing: the generation
/// never activates as if it were complete.
#[test]
fn a_live_work_without_a_vector_keeps_the_generation_partial() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let a = seed_catalog(&mut conn, "PARA0001", "Obra A", "Resumen A.");
    let b = seed_catalog(&mut conn, "PARB0001", "Obra B", "Resumen B.");
    admit_profile_demand(&conn, &a);
    admit_profile_demand(&conn, &b);

    assert!(matches!(
        run_profile_task(&dir, &conn, FakeProfileEmbedder::ok(4)),
        RunOneOutcome::Succeeded { .. }
    ));
    let failed = run_profile_task(
        &dir,
        &conn,
        FakeProfileEmbedder::failing("OpenRouter embedding API error (400 Bad Request): bad"),
    );
    assert!(matches!(failed, RunOneOutcome::Failed { .. }), "{failed:?}");

    assert_eq!(generation_statuses(&conn), vec!["staging"]);
    let activated =
        entropia_desktop_lib::bibliography::generation::activate_complete_staging_generations(
            &conn,
            repository::now_ms(),
        )
        .expect("repair pass");
    assert_eq!(activated, 0, "a partial generation is never activated");
    assert_eq!(generation_statuses(&conn), vec!["staging"]);
}

/// A work deleted mid-run (tombstoned) stops being owed: the generation is
/// not stranded by a manifest that can no longer be met.
#[test]
fn a_deleted_work_does_not_strand_the_generation() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let a = seed_catalog(&mut conn, "DELA0001", "Obra A", "Resumen A.");
    let b = seed_catalog(&mut conn, "DELB0001", "Obra B", "Resumen B.");
    admit_profile_demand(&conn, &a);
    admit_profile_demand(&conn, &b);
    assert!(matches!(
        run_profile_task(&dir, &conn, FakeProfileEmbedder::ok(4)),
        RunOneOutcome::Succeeded { .. }
    ));
    let pending: String = conn
        .query_row(
            "SELECT subject_id FROM processing_tasks
             WHERE kind = 'bibliography_profile' AND state = 'pending'",
            [],
            |row| row.get(0),
        )
        .expect("the work that still owes its vector");
    assert_eq!(generation_statuses(&conn), vec!["staging"]);

    conn.execute(
        "INSERT INTO zotero_item_tombstones (item_id, observed_at, reason)
         VALUES (?1, 1, 'deleted upstream')",
        [&pending],
    )
    .expect("tombstone the pending work");
    let activated =
        entropia_desktop_lib::bibliography::generation::activate_complete_staging_generations(
            &conn,
            repository::now_ms(),
        )
        .expect("repair pass");

    assert_eq!(activated, 1);
    assert_eq!(generation_statuses(&conn), vec!["active"]);
    let generation_id: String = conn
        .query_row(
            "SELECT id FROM bibliographic_index_generations",
            [],
            |row| row.get(0),
        )
        .expect("generation id");
    let (_, expected, completed, _) = generation_state(&conn, &generation_id);
    assert_eq!(
        (expected, completed),
        (1, 1),
        "the manifest is re-derived from live works"
    );
}

/// The owner's archive: every vector landed but nothing ever activated the
/// generation. Startup recovery and the next sync publication both repair it.
#[test]
fn a_complete_staging_generation_is_activated_by_recovery_and_by_sync() {
    for repair in ["recovery", "sync"] {
        let (dir, mut conn) = migrated_db();
        seed_library(&conn, "lib-1", Some(7));
        let a = seed_catalog(&mut conn, "OWNA0001", "Obra A", "Resumen A.");
        admit_profile_demand(&conn, &a);
        assert!(matches!(
            run_profile_task(&dir, &conn, FakeProfileEmbedder::ok(4)),
            RunOneOutcome::Succeeded { .. }
        ));
        // Rewind to what the old build left behind: staging, never activated.
        conn.execute(
            "UPDATE bibliographic_index_generations
             SET status = 'staging', activated_at = NULL",
            [],
        )
        .expect("rewind to the stranded state");
        assert_eq!(generation_statuses(&conn), vec!["staging"], "{repair}");

        match repair {
            "recovery" => {
                entropia_desktop_lib::processing::recovery::recover_session(
                    &conn,
                    "",
                    repository::now_ms(),
                )
                .expect("recovery");
            }
            _ => {
                repository::admit_stale_profile_demands(&conn, "lib-1").expect("sync admission");
            }
        }

        assert_eq!(
            generation_statuses(&conn).first().map(String::as_str),
            Some("active"),
            "{repair}: the stranded generation is the active one"
        );
    }
}

/// An incremental sync re-embeds only the work that changed. Its staging
/// generation must fold into the active one, never replace it: unchanged
/// works keep their vectors and exactly one generation stays active.
#[test]
fn an_incremental_sync_folds_into_the_active_generation() {
    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let a = seed_catalog(&mut conn, "INCA0001", "Obra A", "Resumen A.");
    let b = seed_catalog(&mut conn, "INCB0001", "Obra B", "Resumen B.");
    admit_profile_demand(&conn, &a);
    admit_profile_demand(&conn, &b);
    for _ in 0..2 {
        run_profile_task(&dir, &conn, FakeProfileEmbedder::ok(4));
    }
    assert_eq!(generation_statuses(&conn), vec!["active"]);
    let active: String = conn
        .query_row(
            "SELECT id FROM bibliographic_index_generations WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .expect("active generation");

    conn.execute(
        "UPDATE bibliographic_items
         SET csl_json_snapshot = json_set(csl_json_snapshot, '$.title', 'Obra A corregida')
         WHERE id = ?1",
        [&a],
    )
    .expect("edit one work");
    assert_eq!(
        repository::admit_stale_profile_demands(&conn, "lib-1").expect("sync admission"),
        1,
        "only the changed work is demanded again"
    );
    assert!(matches!(
        run_profile_task(&dir, &conn, FakeProfileEmbedder::ok(4)),
        RunOneOutcome::Succeeded { .. }
    ));

    assert_eq!(generation_statuses(&conn), vec!["active", "retired"]);
    assert_eq!(embedding_generation_of(&conn, &a), vec![active.clone()]);
    assert_eq!(
        embedding_generation_of(&conn, &b),
        vec![active.clone()],
        "the unchanged work keeps its vector in the active generation"
    );
    let a_hash: String = conn
        .query_row(
            "SELECT input_hash FROM bibliographic_item_embeddings WHERE item_id = ?1",
            [&a],
            |row| row.get(0),
        )
        .expect("a vector");
    let profile = entropia_desktop_lib::bibliography::repository::get_semantic_profile(&conn, &a)
        .expect("profile")
        .expect("stored");
    assert_eq!(
        a_hash, profile.input_hash,
        "the folded vector is the fresh one"
    );
}

/// Profile publication -> activation -> passage search returns a vector hit
/// from the stored chunk embeddings, under the contract the query side
/// resolves from settings.
#[test]
fn a_published_work_is_found_by_passage_search_once_active() {
    use entropia_desktop_lib::bibliography::retrieval::{search_passages, WorkFilters};

    let (dir, mut conn) = migrated_db();
    seed_library(&conn, "lib-1", Some(7));
    let item_id = five_page_work(&mut conn, "E2E00001");
    admit_profile_demand(&conn, &item_id);
    let effective = resolve_effective_embedding_contract(&conn).expect("effective contract");
    let embed = |text: &str| Ok(RecordingEmbedder::vector_for(text));
    let question = "Pagina numero 3 con contenido distinto.";

    let first = run_profile_with(&dir, &conn, RecordingEmbedder::new("fake/model", None), 2);
    assert!(
        matches!(first, RunOneOutcome::Succeeded { .. }),
        "{first:?}"
    );

    let hits = search_passages(
        &conn,
        &effective.hash,
        question,
        5,
        3,
        5,
        &WorkFilters::default(),
        &embed,
    )
    .expect("passage search");
    assert!(!hits.is_empty(), "the active generation must answer");
    assert_eq!(hits[0].item_id, item_id);
    assert_eq!(hits[0].contract_hash, effective.hash);
    let active: String = conn
        .query_row(
            "SELECT id FROM bibliographic_index_generations WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .expect("active generation");
    assert_eq!(hits[0].generation_id, active);
}
