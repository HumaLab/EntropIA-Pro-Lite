//! Behavior tests for the IS5a bounded research sync activation
//! (`sync::research_cycle`) against MockSyncApi wrapped in `ResearchApi` and a
//! temporary SQLite fixture loaded from the real generated schema fixture.
//!
//! They lock the activation rules: a missing research state is a complete
//! no-op, legacy servers are quietly skipped after `NotAdvertised`, capability
//! discovery only ever comes from an ordinary response, catch-up is since-zero
//! and precedes incremental pulls, corpus rows ride the corpus path without
//! moving the shared cursor, a stale research cursor is discarded once and
//! restarts at zero, the report blob installs BEFORE the projection applies
//! (and a pending blob retains the queue row), tombstones make no blob call,
//! terminal jobs seed into the outbox only after the recorded catch-up (never
//! active or human-gated ones), and a session change keeps queued rows pending.
//!
//! The IS5b push half is locked here too: pushes run only after the recorded
//! catch-up, the report blob uploads (HEAD→PUT) BEFORE its row, tombstones
//! push with no blob call, requests and responses are validated (size limit,
//! epoch, exact capability, exactly one matching result, positive sequence,
//! known status), `applied`/`lww_won` acknowledge only the exact captured
//! generation (a newer generation racing the response survives), `lww_lost`
//! installs the winner's report blob before applying it and preserves the
//! loser in `sync_conflicts`, and one session change or the per-cycle budget
//! stops every send with the outbox retained.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::http::{
    BlobExists, DeleteAccountRequest, DevicesResponse, HealthLimits, HealthResponse, LoginRequest,
    LoginResponse, NotificationItem, PlanCatalogItem, PlanChangeRequestResponse, PullResponse,
    PullRow, PushChange, PushRequest, PushResponse, PushResult, RegisterRequest, RegisterResponse,
    SyncApi, SyncError, UsageResponse, RESEARCH_ENVELOPE_V1_CAPABILITY,
};
use super::research_capture::{
    outbox_entries, record_capability, record_catchup_done, supports_research, ENVELOPE_TABLE,
    OUTBOX_PREFIX,
};
use super::research_cycle::{run_research_cycle, ResearchDiscovery};
use super::research_envelope::REPORT_FILE_REL_PATH;
use super::research_pull_tests::ResearchApi;
use super::research_receive::queued_research_receives;
use super::session::{meta_get, meta_set, write_sync_session, SYNC_SESSION_INCARNATION_KEY};
use super::test_support::{MockBlobEvent, MockSyncApi, SCHEMA_FIXTURE};

const SERVER_EPOCH: &str = "mock-epoch";
const ACCOUNT_ID: &str = "account-a";
const DEVICE_ID: &str = "device-a";
const SERVER_URL: &str = "https://sync.example.test";
const SESSION_INCARNATION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    _temp: TempDir,
    db_path: PathBuf,
    data_root: PathBuf,
    research_state: PathBuf,
    artifacts_root: PathBuf,
}

impl Fixture {
    /// A workspace with a live `research/estado.sqlite`.
    fn new() -> Self {
        let fixture = Self::bare();
        fixture.create_research_state();
        fixture
    }

    /// A workspace where the Investigations engine never ran (no state file).
    fn bare() -> Self {
        let temp = tempfile::tempdir().expect("temporary fixture");
        let db_path = temp.path().join("archive.sqlite");
        let data_root = temp.path().join("app-data");
        fs::create_dir_all(&data_root).expect("data root");
        let research_dir = data_root.join("research");
        fs::create_dir_all(&research_dir).expect("research dir");

        let conn = open_db(&db_path);
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("real generated application schema");
        crate::sync::schema::ensure_sync_schema(&conn).expect("sync schema");
        drop(conn);

        let fixture = Self {
            _temp: temp,
            db_path,
            research_state: research_dir.join("estado.sqlite"),
            artifacts_root: research_dir.join("artifacts"),
            data_root,
        };
        let conn = fixture.connect();
        write_sync_session(
            &conn,
            SERVER_URL,
            ACCOUNT_ID,
            "reader@example.test",
            DEVICE_ID,
        )
        .expect("session");
        // Deterministic incarnation so tests can assert the exact identity a
        // staged cursor is bound to.
        meta_set(&conn, SYNC_SESSION_INCARNATION_KEY, SESSION_INCARNATION).expect("incarnation");
        meta_set(&conn, "server_epoch", SERVER_EPOCH).expect("epoch");
        fixture
    }

    fn create_research_state(&self) {
        entropia_agent::estado::EstadoDb::abrir(self.research_state.to_str().expect("utf-8 path"))
            .expect("estado schema");
        fs::create_dir_all(&self.artifacts_root).expect("artifacts root");
    }

    fn connect(&self) -> Connection {
        open_db(&self.db_path)
    }

    fn enable_capability(&self) {
        record_capability(&self.connect(), SERVER_EPOCH, true).expect("record capability");
    }

    fn complete_catchup(&self) {
        record_catchup_done(&self.connect(), SERVER_EPOCH).expect("record catchup");
    }

    /// Inserts one job row into the real `estado.sqlite` schema.
    fn insert_job(&self, id: &str, modo: &str, status: &str) {
        let state = Connection::open(&self.research_state).expect("state connection");
        state
            .execute(
                "INSERT INTO jobs (id, modo, pregunta, status, close_reason, config_snapshot,
                                  corpus_snapshot_id, project, corpus, created_at, updated_at)
                 VALUES (?1, ?2, '¿Pregunta?', ?3, 'completed', '{}', 'snap-1', 'demo', 'desktop', 100, 200)",
                rusqlite::params![id, modo, status],
            )
            .expect("insert job");
    }

    fn state_job_ids(&self) -> Vec<String> {
        let state = Connection::open(&self.research_state).expect("state connection");
        let mut stmt = state
            .prepare("SELECT id FROM jobs ORDER BY id")
            .expect("jobs statement");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("jobs query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("jobs rows");
        rows
    }
}

// ---------------------------------------------------------------------------
// ResearchPushApi — MockSyncApi (through ResearchApi) plus the opted-in
// research push the trait default fails closed on. Scripted responses drive
// the validation scenarios, hooks model races that must happen during the send
// round trip, and the ordered event log proves blob-before-row ordering.
// ---------------------------------------------------------------------------

struct PushApi {
    inner: ResearchApi,
    push_calls: Mutex<usize>,
    pushed_changes: Mutex<Vec<PushChange>>,
    events: Mutex<Vec<String>>,
    responses: Mutex<VecDeque<Result<PushResponse, SyncError>>>,
    hooks: Mutex<VecDeque<Box<dyn Fn() + Send + Sync>>>,
}

impl PushApi {
    fn new(inner: ResearchApi) -> Self {
        Self {
            inner,
            push_calls: Mutex::new(0),
            pushed_changes: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::new()),
            hooks: Mutex::new(VecDeque::new()),
        }
    }

    /// Scripts the next opted-in push response (FIFO).
    fn script(&self, response: PushResponse) {
        self.responses
            .lock()
            .expect("responses")
            .push_back(Ok(response));
    }

    /// Scripts the next opted-in push failure (FIFO).
    fn script_error(&self, error: SyncError) {
        self.responses
            .lock()
            .expect("responses")
            .push_back(Err(error));
    }

    /// Runs once when the next opted-in push is served — the window between
    /// draft preparation and settlement.
    fn on_push(&self, hook: Box<dyn Fn() + Send + Sync>) {
        self.hooks.lock().expect("hooks").push_back(hook);
    }

    fn event_log(&self) -> Vec<String> {
        self.events.lock().expect("events").clone()
    }

    fn push_count(&self) -> usize {
        *self.push_calls.lock().expect("push calls")
    }

    fn pushed(&self) -> Vec<PushChange> {
        self.pushed_changes.lock().expect("pushed changes").clone()
    }
}

impl SyncApi for PushApi {
    async fn register(&self, req: RegisterRequest) -> Result<RegisterResponse, SyncError> {
        self.inner.register(req).await
    }

    async fn login(&self, req: LoginRequest) -> Result<LoginResponse, SyncError> {
        self.inner.login(req).await
    }

    async fn logout(&self, token: &str) -> Result<(), SyncError> {
        self.inner.logout(token).await
    }

    async fn devices(&self, token: &str) -> Result<DevicesResponse, SyncError> {
        self.inner.devices(token).await
    }

    async fn revoke(&self, token: &str, device_id: &str) -> Result<(), SyncError> {
        self.inner.revoke(token, device_id).await
    }

    async fn delete_account(
        &self,
        token: &str,
        req: DeleteAccountRequest,
    ) -> Result<(), SyncError> {
        self.inner.delete_account(token, req).await
    }

    async fn usage(&self, token: &str) -> Result<UsageResponse, SyncError> {
        self.inner.usage(token).await
    }

    async fn list_plans(&self, token: &str) -> Result<Vec<PlanCatalogItem>, SyncError> {
        self.inner.list_plans(token).await
    }

    async fn request_plan_change(
        &self,
        token: &str,
        requested_plan_id: &str,
        note: Option<&str>,
    ) -> Result<PlanChangeRequestResponse, SyncError> {
        self.inner
            .request_plan_change(token, requested_plan_id, note)
            .await
    }

    async fn list_notifications(
        &self,
        token: &str,
        since: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<NotificationItem>, SyncError> {
        self.inner.list_notifications(token, since, limit).await
    }

    async fn mark_notification_read(&self, token: &str, id: &str) -> Result<(), SyncError> {
        self.inner.mark_notification_read(token, id).await
    }

    async fn delete_notification(&self, token: &str, id: &str) -> Result<(), SyncError> {
        self.inner.delete_notification(token, id).await
    }

    async fn health(&self) -> Result<HealthResponse, SyncError> {
        self.inner.health().await
    }

    async fn push(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        self.inner.push(token, schema_tag, req).await
    }

    async fn push_with_research_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        *self.push_calls.lock().expect("push calls") += 1;
        {
            let mut events = self.events.lock().expect("events");
            let mut pushed = self.pushed_changes.lock().expect("pushed changes");
            for change in &req.changes {
                events.push(format!("push:{}", change.row_id));
                pushed.push(change.clone());
            }
        }
        if let Some(hook) = self.hooks.lock().expect("hooks").pop_front() {
            hook();
        }
        if let Some(response) = self.responses.lock().expect("responses").pop_front() {
            return response;
        }
        self.inner.push(token, schema_tag, req).await
    }

    async fn pull(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.inner.pull(token, schema_tag, since, limit).await
    }

    async fn pull_with_research_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.inner
            .pull_with_research_envelope_v1(token, schema_tag, since, limit)
            .await
    }

    async fn blob_head(&self, token: &str, sha256: &str) -> Result<BlobExists, SyncError> {
        self.events
            .lock()
            .expect("events")
            .push(format!("head:{sha256}"));
        self.inner.blob_head(token, sha256).await
    }

    async fn blob_put(&self, token: &str, sha256: &str, bytes: Vec<u8>) -> Result<(), SyncError> {
        self.events
            .lock()
            .expect("events")
            .push(format!("put:{sha256}"));
        self.inner.blob_put(token, sha256, bytes).await
    }

    async fn blob_get(&self, token: &str, sha256: &str) -> Result<reqwest::Response, SyncError> {
        self.events
            .lock()
            .expect("events")
            .push(format!("get:{sha256}"));
        self.inner.blob_get(token, sha256).await
    }
}

fn open_db(db_path: &std::path::Path) -> Connection {
    let conn = Connection::open(db_path).expect("open temporary sqlite");
    conn.pragma_update(None, "journal_mode", "WAL")
        .expect("WAL mode");
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .expect("configure sqlite");
    conn
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

fn health() -> HealthResponse {
    HealthResponse {
        status: "ok".to_string(),
        version: "test".to_string(),
        epoch: SERVER_EPOCH.to_string(),
        server_now_ms: 1_700_000_000_000,
        limits: HealthLimits {
            max_push_bytes: 1_000_000,
            max_blob_mb: 1,
        },
    }
}

fn capable_api() -> ResearchApi {
    let mut api = MockSyncApi::default();
    api.server_epoch = SERVER_EPOCH.to_string();
    *api.server_capabilities.lock().expect("server capabilities") =
        vec![super::http::RESEARCH_ENVELOPE_V1_CAPABILITY.to_string()];
    ResearchApi::new(api)
}

fn plain_api() -> ResearchApi {
    let mut api = MockSyncApi::default();
    api.server_epoch = SERVER_EPOCH.to_string();
    ResearchApi::new(api)
}

fn envelope_payload(job_id: &str, report_file: Value) -> Value {
    json!({
        "id": job_id,
        "envelope_version": 1,
        "job": {
            "id": job_id,
            "status": "done",
            "close_reason": "completed",
            "title": "Estudio de campo",
            "question": "¿Pregunta de investigación?",
            "project": "demo",
            "corpus_snapshot_id": null,
            "created_at": 100,
            "updated_at": 200
        },
        "report": {"version": 1, "content_json": {"markdown": "# Informe"}},
        "sources": [],
        "report_file": report_file
    })
}

fn research_row(seq: i64, job_id: &str) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(envelope_payload(job_id, Value::Null)),
    }
}

fn research_row_with_report(seq: i64, job_id: &str, manifest: Value) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(envelope_payload(job_id, manifest)),
    }
}

fn research_tombstone(seq: i64, job_id: &str) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq: seq,
        deleted: true,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: None,
    }
}

fn corpus_row(seq: i64, row_id: &str) -> PullRow {
    PullRow {
        table: "collections".to_string(),
        row_id: row_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 100,
        device_id: "device-remote".to_string(),
        payload: Some(json!({ "id": row_id, "name": "Remota", "created_at": 1, "updated_at": 1 })),
    }
}

fn page(rows: Vec<PullRow>, next_since: i64, has_more: bool, advertise: bool) -> PullResponse {
    PullResponse {
        rows,
        next_since,
        has_more,
        schema_tag: "0023_sync_ids".to_string(),
        server_epoch: SERVER_EPOCH.to_string(),
        server_now_ms: 1_700_000_000_000,
        capabilities: if advertise {
            vec![super::http::RESEARCH_ENVELOPE_V1_CAPABILITY.to_string()]
        } else {
            Vec::new()
        },
    }
}

fn report_bytes() -> Vec<u8> {
    b"# Informe de investigacion\n".to_vec()
}

fn manifest_for(bytes: &[u8]) -> Value {
    json!({
        "rel_path": "report.md",
        "sha256": format!("{:x}", Sha256::digest(bytes)),
        "size": bytes.len()
    })
}

fn hash_of(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn queued_ids(conn: &Connection) -> Vec<String> {
    queued_research_receives(conn)
        .expect("queue")
        .into_iter()
        .map(|row| row.job_id)
        .collect()
}

fn outbox_ids(conn: &Connection) -> Vec<String> {
    let mut ids: Vec<String> = outbox_entries(conn)
        .expect("outbox")
        .into_iter()
        .map(|entry| entry.job_id)
        .collect();
    ids.sort();
    ids
}

fn shared_cursor(conn: &Connection) -> Option<String> {
    meta_get(conn, "last_pull_seq").expect("shared cursor")
}

fn blob_gets(api: &ResearchApi) -> Vec<String> {
    api.inner
        .blob_events
        .lock()
        .expect("blob events")
        .iter()
        .filter_map(|event| match event {
            MockBlobEvent::Get(sha) => Some(sha.clone()),
            _ => None,
        })
        .collect()
}

fn no_warn() -> impl Fn(String) + Sync {
    |_| {}
}

async fn run_with<A: SyncApi>(
    fixture: &Fixture,
    api: &A,
    health: &HealthResponse,
    warn: &(dyn Fn(String) + Sync),
) -> super::research_cycle::ResearchCycleOutcome {
    let conn = fixture.connect();
    run_research_cycle(
        api,
        "token",
        &conn,
        &fixture.data_root,
        &fixture.research_state,
        &fixture.artifacts_root,
        health,
        warn,
    )
    .await
}

async fn run<A: SyncApi>(
    fixture: &Fixture,
    api: &A,
    warn: &(dyn Fn(String) + Sync),
) -> super::research_cycle::ResearchCycleOutcome {
    run_with(fixture, api, &health(), warn).await
}

// ---------------------------------------------------------------------------
// No-state no-op and capability discovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_research_state_is_a_complete_no_op() {
    let fixture = Fixture::bare();
    let api = capable_api();
    api.inner
        .queue_pull_page(page(vec![research_row(5, "job-1")], 5, true, true));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.discovery, Some(ResearchDiscovery::NoState));
    assert_eq!(*api.ordinary_pull_calls.lock().unwrap(), 0);
    assert_eq!(*api.research_pull_calls.lock().unwrap(), 0);
    assert!(outcome.pending.is_empty(), "the phase is silent");
}

#[tokio::test]
async fn legacy_server_is_quietly_skipped_after_not_advertised() {
    let fixture = Fixture::new();
    let api = plain_api();

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.discovery, Some(ResearchDiscovery::NotAdvertised));
    assert_eq!(*api.ordinary_pull_calls.lock().unwrap(), 1);
    assert_eq!(
        *api.research_pull_calls.lock().unwrap(),
        0,
        "a legacy server never sees an opted-in request"
    );
    assert!(outcome.pending.is_empty(), "legacy skip is quiet");
}

#[tokio::test]
async fn discovery_samples_capabilities_only_from_an_ordinary_pull() {
    let fixture = Fixture::new();
    let api = capable_api();
    api.inner.queue_pull_page(page(vec![], 0, true, true));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.discovery, Some(ResearchDiscovery::Recorded));
    assert_eq!(*api.ordinary_pull_calls.lock().unwrap(), 1);
    let conn = fixture.connect();
    assert!(supports_research(&conn, SERVER_EPOCH).expect("capability"));
    // Discovery never opts in before the record exists; the first opted-in
    // call happens afterwards, in the catch-up below.
    assert!(
        *api.research_pull_calls.lock().unwrap() >= 1,
        "catch-up runs after the capability is recorded"
    );
}

// ---------------------------------------------------------------------------
// Since-zero catch-up before incremental pages; corpus cursor non-advance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn catchup_requests_start_at_since_zero_before_incremental_pages() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    let api = capable_api();
    api.inner
        .queue_pull_page(page(vec![research_row(5, "job-1")], 5, true, true));
    api.inner
        .queue_pull_page(page(vec![research_row(7, "job-2")], 7, true, true));
    // The page AFTER the full drain is the one that records the catch-up.
    api.inner.queue_pull_page(page(vec![], 7, false, true));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert!(outcome.catchup_needed_at_start);
    assert!(outcome.catchup_recorded);
    assert_eq!(outcome.pages_staged, 3);
    assert_eq!(outcome.research_rows_staged, 2);
    assert_eq!(
        *api.research_pull_sinces.lock().unwrap(),
        vec![0, 5, 7],
        "catch-up starts at zero on the dedicated cursor and settles each page"
    );
    let conn = fixture.connect();
    assert!(
        queued_ids(&conn).is_empty(),
        "queued rows settled each page"
    );
    assert_eq!(fixture.state_job_ids(), vec!["job-1", "job-2"]);
}

#[tokio::test]
async fn corpus_rows_ride_the_corpus_path_without_moving_the_shared_cursor() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    let conn = fixture.connect();
    meta_set(&conn, "last_pull_seq", "3").expect("shared cursor");
    drop(conn);
    let api = capable_api();
    api.inner.queue_pull_page(page(
        vec![corpus_row(10, "col-remote"), research_row(15, "job-1")],
        15,
        false,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.corpus_rows_routed, 1);
    let conn = fixture.connect();
    assert_eq!(
        shared_cursor(&conn).as_deref(),
        Some("3"),
        "the research phase never advances last_pull_seq"
    );
    let applied: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM collections WHERE id = 'col-remote'",
            [],
            |row| row.get(0),
        )
        .expect("corpus row applied");
    assert_eq!(applied, 1, "corpus rows are applied, not discarded");
}

// ---------------------------------------------------------------------------
// Stale cursor recovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stale_research_cursor_is_discarded_once_and_restarts_at_zero() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    // A cursor bound to a dead session incarnation.
    let stale = json!({
        "scope": {
            "account_id": ACCOUNT_ID,
            "server_url": SERVER_URL,
            "device_id": DEVICE_ID,
            "server_epoch": SERVER_EPOCH,
            "session_incarnation": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
        },
        "since": 42,
    })
    .to_string();
    let conn = fixture.connect();
    meta_set(&conn, super::research_capture::PULL_CURSOR_KEY, &stale).expect("stale cursor");
    drop(conn);

    let api = capable_api();
    api.inner
        .queue_pull_page(page(vec![research_row(5, "job-1")], 5, false, true));

    let outcome = run(&fixture, &api, &no_warn()).await;

    // The stale page failed before HTTP, the cursor was discarded once, and
    // the retry restarted at zero and completed the catch-up.
    assert_eq!(
        *api.research_pull_sinces.lock().unwrap(),
        vec![0, 5],
        "the discarded cursor restarts the catch-up at zero"
    );
    assert!(outcome.catchup_recorded);
    let conn = fixture.connect();
    let raw = meta_get(&conn, super::research_capture::PULL_CURSOR_KEY)
        .expect("cursor")
        .expect("cursor rewritten");
    assert!(
        raw.contains(SESSION_INCARNATION),
        "the new cursor binds to the current session: {raw}"
    );
}

// ---------------------------------------------------------------------------
// Report blob-before-apply ordering and deferral
// ---------------------------------------------------------------------------

#[tokio::test]
async fn report_blob_installs_before_the_projection_applies() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    let bytes = report_bytes();
    let api = capable_api();
    api.inner.put_blob_bytes(&hash_of(&bytes), bytes.clone());
    api.inner.queue_pull_page(page(
        vec![research_row_with_report(5, "job-1", manifest_for(&bytes))],
        5,
        false,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.receives_settled, 1);
    assert_eq!(blob_gets(&api), vec![hash_of(&bytes)], "one blob GET");
    let installed = fixture.artifacts_root.join("job-1").join("report.md");
    assert_eq!(
        fs::read(&installed).expect("installed report"),
        bytes,
        "the report bytes landed before the projection"
    );
    assert_eq!(fixture.state_job_ids(), vec!["job-1"]);
    let conn = fixture.connect();
    assert!(queued_ids(&conn).is_empty());
}

#[tokio::test]
async fn a_pending_report_blob_retains_the_row_without_applying() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    let bytes = report_bytes();
    let api = capable_api();
    // No blob bytes seeded: the download reports a remote miss.
    api.inner.queue_pull_page(page(
        vec![research_row_with_report(5, "job-1", manifest_for(&bytes))],
        5,
        false,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.receives_settled, 0);
    assert_eq!(outcome.receives_deferred, 1);
    let conn = fixture.connect();
    assert_eq!(
        queued_ids(&conn),
        vec!["job-1".to_string()],
        "the row is retained with a bounded reason, never dropped"
    );
    let queued = queued_research_receives(&conn).expect("queue");
    assert!(
        queued[0]
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("RemoteMissing"),
        "bounded reason retained: {:?}",
        queued[0].last_error
    );
    assert!(
        fixture.state_job_ids().is_empty(),
        "nothing was applied without its report"
    );
}

#[tokio::test]
async fn tombstones_make_no_blob_call() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    let bytes = report_bytes();
    let api = capable_api();
    api.inner.put_blob_bytes(&hash_of(&bytes), bytes.clone());
    api.inner
        .queue_pull_page(page(vec![research_tombstone(5, "job-1")], 5, false, true));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.receives_settled, 1);
    assert!(blob_gets(&api).is_empty(), "a tombstone never downloads");
    let conn = fixture.connect();
    assert!(queued_ids(&conn).is_empty());
}

// ---------------------------------------------------------------------------
// Seeding after catch-up; session changes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn seeding_runs_only_after_catchup_and_only_for_terminal_jobs() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.insert_job("j-done", "research", "done");
    fixture.insert_job("j-failed", "research", "failed");
    fixture.insert_job("j-running", "research", "running");
    fixture.insert_job("j-paused", "research", "paused");
    fixture.insert_job("j-gated", "research", "awaiting_human");
    let api = capable_api();
    // The terminal page records the catch-up (empty queue, has_more=false).
    api.inner.queue_pull_page(page(vec![], 0, false, true));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert!(outcome.catchup_recorded);
    assert_eq!(outcome.jobs_seeded, 2);
    let conn = fixture.connect();
    assert_eq!(outbox_ids(&conn), vec!["j-done", "j-failed"]);
}

#[tokio::test]
async fn no_seeding_happens_before_the_catchup_is_recorded() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.insert_job("j-done", "research", "done");
    let bytes = report_bytes();
    let api = capable_api();
    // The page holds a row that defers (blob missing): the queue stays
    // unresolved, the catch-up cannot record, and nothing seeds.
    api.inner.queue_pull_page(page(
        vec![research_row_with_report(5, "job-1", manifest_for(&bytes))],
        5,
        false,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert!(!outcome.catchup_recorded);
    assert_eq!(outcome.jobs_seeded, 0);
    let conn = fixture.connect();
    assert!(
        outbox_ids(&conn).is_empty(),
        "the outbox only fills after remote tombstones/catch-up"
    );
}

#[tokio::test]
async fn seeded_jobs_are_never_re_seeded_after_a_completed_catchup() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    fixture.insert_job("j-done", "research", "done");
    let api = capable_api();
    api.inner.queue_pull_page(page(vec![], 0, false, true));

    let first = run(&fixture, &api, &no_warn()).await;
    assert_eq!(first.jobs_seeded, 1);
    let second = run(&fixture, &api, &no_warn()).await;
    assert_eq!(second.jobs_seeded, 0, "seeding is idempotent");

    let conn = fixture.connect();
    assert_eq!(outbox_ids(&conn), vec!["j-done"]);
}

#[tokio::test]
async fn session_changes_keep_queued_rows_pending_without_touching_state() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    // A queued row from a previous login incarnation.
    let conn = fixture.connect();
    super::research_receive::enqueue_research_receive(&conn, &research_row(5, "job-1"))
        .expect("enqueue under the old session");
    write_sync_session(
        &conn,
        SERVER_URL,
        ACCOUNT_ID,
        "reader@example.test",
        DEVICE_ID,
    )
    .expect("new session");
    drop(conn);

    let api = capable_api();
    api.inner.queue_pull_page(page(vec![], 0, false, true));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert!(outcome.receives_deferred >= 1, "the foreign row defers");
    let conn = fixture.connect();
    assert_eq!(
        queued_ids(&conn),
        vec!["job-1".to_string()],
        "the row stays pending under its own scope"
    );
    assert!(
        fixture.state_job_ids().is_empty(),
        "no stale-scope row was ever applied"
    );
}

// ---------------------------------------------------------------------------
// IS5b push activation
// ---------------------------------------------------------------------------

const GENERATION_ONE: &str = "3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9";
const GENERATION_TWO: &str = "9c1d3a52-4e8b-4a2c-9d7f-1b6e5c0a8f44";

/// One coalesced outbox entry exactly as capture stores it.
fn enqueue_outbox(conn: &Connection, job_id: &str, op: &str, changed_at: i64, generation: &str) {
    meta_set(
        conn,
        &format!("{OUTBOX_PREFIX}{job_id}"),
        &json!({"op": op, "changed_at": changed_at, "generation": generation}).to_string(),
    )
    .expect("outbox entry");
}

fn write_report(artifacts_root: &std::path::Path, job_id: &str, bytes: &[u8]) {
    let job_dir = artifacts_root.join(job_id);
    fs::create_dir_all(&job_dir).expect("job dir");
    fs::write(job_dir.join(REPORT_FILE_REL_PATH), bytes).expect("report file");
}

fn push_response(results: Vec<PushResult>, epoch: &str, advertise: bool) -> PushResponse {
    PushResponse {
        results,
        max_server_seq: 100,
        server_epoch: epoch.to_string(),
        server_now_ms: 1_700_000_000_000,
        capabilities: if advertise {
            vec![RESEARCH_ENVELOPE_V1_CAPABILITY.to_string()]
        } else {
            Vec::new()
        },
    }
}

fn applied_push_result(job_id: &str, server_seq: i64) -> PushResult {
    PushResult {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        status: "applied".to_string(),
        server_seq,
        winner: None,
    }
}

fn winner_row(job_id: &str, server_seq: i64, manifest: Value) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq,
        deleted: false,
        changed_at: server_seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(envelope_payload(job_id, manifest)),
    }
}

fn recorded_seq(conn: &Connection, job_id: &str) -> i64 {
    conn.query_row(
        "SELECT server_seq FROM sync_row_versions
          WHERE table_name = ?1 AND row_id = ?2",
        rusqlite::params![ENVELOPE_TABLE, job_id],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

fn conflict_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM sync_conflicts", [], |row| row.get(0))
        .expect("conflicts")
}

/// A push-ready workspace: capability recorded, catch-up complete, one terminal
/// job with a pending `U` generation and no report file.
fn push_ready_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    fixture.insert_job("job-u", "research", "done");
    let conn = fixture.connect();
    enqueue_outbox(&conn, "job-u", "U", 1_234, GENERATION_ONE);
    drop(conn);
    fixture
}

#[tokio::test]
async fn pushes_run_only_after_the_catchup_is_recorded() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.insert_job("j-done", "research", "done");
    let conn = fixture.connect();
    enqueue_outbox(&conn, "j-done", "U", 1_234, GENERATION_ONE);
    drop(conn);
    let api = PushApi::new(capable_api());
    // A row that defers (its report blob is missing) keeps the catch-up
    // unrecorded — and an unrecorded catch-up must never push.
    let bytes = report_bytes();
    api.inner.inner.queue_pull_page(page(
        vec![research_row_with_report(5, "job-1", manifest_for(&bytes))],
        5,
        false,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert!(!outcome.catchup_recorded);
    assert_eq!(
        api.push_count(),
        0,
        "nothing pushes before the recorded catch-up"
    );
    assert_eq!(outbox_ids(&fixture.connect()), vec!["j-done"]);
    assert_eq!(outcome.pushes_settled, 0);

    // With the catch-up recorded, the retained generation pushes and settles.
    fixture.complete_catchup();
    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(api.push_count(), 1);
    assert_eq!(outcome.pushes_settled, 1);
    assert!(
        outbox_ids(&fixture.connect()).is_empty(),
        "the exact captured generation was acknowledged"
    );
}

#[tokio::test]
async fn upsert_drafts_upload_the_report_blob_before_the_row() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    fixture.insert_job("job-u", "research", "done");
    let bytes = report_bytes();
    write_report(&fixture.artifacts_root, "job-u", &bytes);
    let conn = fixture.connect();
    enqueue_outbox(&conn, "job-u", "U", 1_234, GENERATION_ONE);
    meta_set(&conn, "clock_offset_ms", "5000").expect("clock offset");
    drop(conn);

    let api = PushApi::new(capable_api());
    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.pushes_settled, 1);
    let sha = hash_of(&bytes);
    assert_eq!(
        api.event_log(),
        vec![
            format!("head:{sha}"),
            format!("put:{sha}"),
            "push:job-u".to_string()
        ],
        "the report blob is HEAD/PUT before its row"
    );
    let pushed = api.pushed();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].table, ENVELOPE_TABLE);
    assert_eq!(pushed[0].row_id, "job-u");
    assert_eq!(pushed[0].op, "upsert");
    assert_eq!(
        pushed[0].changed_at,
        1_234 + 5_000,
        "the clock offset is applied exactly once"
    );
    assert_eq!(pushed[0].base_seq, 0);
    let payload = pushed[0].payload.as_ref().expect("upsert payload");
    assert_eq!(payload["report_file"]["sha256"], json!(sha));
    assert_eq!(
        payload["report_file"]["rel_path"],
        json!(REPORT_FILE_REL_PATH)
    );
}

#[tokio::test]
async fn tombstone_drafts_push_without_any_blob_call() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    let conn = fixture.connect();
    enqueue_outbox(&conn, "job-d", "D", 2_000, GENERATION_ONE);
    drop(conn);

    let api = PushApi::new(capable_api());
    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.pushes_settled, 1);
    assert_eq!(
        api.event_log(),
        vec!["push:job-d".to_string()],
        "a tombstone never touches a blob endpoint"
    );
    let pushed = api.pushed();
    assert_eq!(pushed[0].op, "delete");
    assert!(
        pushed[0].payload.is_none(),
        "a tombstone carries no payload"
    );
    assert!(outbox_ids(&fixture.connect()).is_empty());
}

#[tokio::test]
async fn unprovable_outbox_entries_are_retained_and_reported() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    fixture.insert_job("job-ok", "research", "done");
    fixture.insert_job("job-bad", "research", "done");
    write_report(&fixture.artifacts_root, "job-ok", &report_bytes());
    // A directory where the managed `report.md` must be: the snapshot cannot
    // prove the file and the entry must stay pending verbatim.
    fs::create_dir_all(
        fixture
            .artifacts_root
            .join("job-bad")
            .join(REPORT_FILE_REL_PATH),
    )
    .expect("blocking directory");

    let api = PushApi::new(capable_api());
    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.pushes_settled, 1);
    assert_eq!(
        outcome.pushes_pending, 1,
        "the unprovable entry is surfaced"
    );
    assert!(
        outcome.pending.iter().any(|note| note.contains("job-bad")),
        "the retained entry is named: {:?}",
        outcome.pending
    );
    assert_eq!(
        outbox_ids(&fixture.connect()),
        vec!["job-bad"],
        "the exact pending entry is retained"
    );
}

#[tokio::test]
async fn oversized_requests_stay_pending_before_any_send() {
    let fixture = push_ready_fixture();
    let api = PushApi::new(capable_api());
    let tiny = HealthResponse {
        limits: HealthLimits {
            max_push_bytes: 10,
            max_blob_mb: 1,
        },
        ..health()
    };

    let outcome = run_with(&fixture, &api, &tiny, &no_warn()).await;

    assert_eq!(api.push_count(), 0, "nothing is sent");
    assert_eq!(outcome.pushes_settled, 0);
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome.pending.iter().any(|note| note.contains("10 bytes")),
        "the advertised limit is reported: {:?}",
        outcome.pending
    );
    assert_eq!(outbox_ids(&fixture.connect()), vec!["job-u"]);
}

#[tokio::test]
async fn push_response_epochs_are_validated() {
    let fixture = push_ready_fixture();
    let api = PushApi::new(capable_api());
    api.script(push_response(
        vec![applied_push_result("job-u", 7)],
        "otro-epoch",
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(api.push_count(), 1);
    assert_eq!(outcome.pushes_settled, 0);
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome.pending.iter().any(|note| note.contains("epoch")),
        "{:?}",
        outcome.pending
    );
    assert_eq!(
        outbox_ids(&fixture.connect()),
        vec!["job-u"],
        "a rejected response settles nothing"
    );
}

#[tokio::test]
async fn push_response_capabilities_are_validated() {
    let fixture = push_ready_fixture();
    let api = PushApi::new(capable_api());
    api.script(push_response(
        vec![applied_push_result("job-u", 7)],
        SERVER_EPOCH,
        false,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.pushes_settled, 0);
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome
            .pending
            .iter()
            .any(|note| note.contains("capability")),
        "{:?}",
        outcome.pending
    );
    assert_eq!(outbox_ids(&fixture.connect()), vec!["job-u"]);
}

#[tokio::test]
async fn push_results_are_validated_before_any_settlement() {
    let scenarios: Vec<(&str, Vec<PushResult>)> = vec![
        ("no result", vec![]),
        (
            "two results",
            vec![
                applied_push_result("job-u", 7),
                applied_push_result("job-u", 8),
            ],
        ),
        (
            "zero sequence",
            vec![PushResult {
                server_seq: 0,
                ..applied_push_result("job-u", 7)
            }],
        ),
        ("mismatched row", vec![applied_push_result("otra", 7)]),
        (
            "unsupported status",
            vec![PushResult {
                status: "desconocido".to_string(),
                ..applied_push_result("job-u", 7)
            }],
        ),
        (
            "winner on success",
            vec![PushResult {
                winner: Some(research_tombstone(7, "job-u")),
                ..applied_push_result("job-u", 7)
            }],
        ),
        (
            "lww_lost without winner",
            vec![PushResult {
                status: "lww_lost".to_string(),
                ..applied_push_result("job-u", 7)
            }],
        ),
    ];

    for (label, results) in scenarios {
        let fixture = push_ready_fixture();
        let api = PushApi::new(capable_api());
        api.script(push_response(results, SERVER_EPOCH, true));

        let outcome = run(&fixture, &api, &no_warn()).await;

        assert_eq!(api.push_count(), 1, "{label}");
        assert_eq!(outcome.pushes_settled, 0, "{label}");
        assert_eq!(outcome.pushes_pending, 1, "{label}");
        assert!(
            outcome
                .pending
                .iter()
                .any(|note| note.contains("stays pending")),
            "{label}: {:?}",
            outcome.pending
        );
        assert_eq!(outbox_ids(&fixture.connect()), vec!["job-u"], "{label}");
    }
}

#[tokio::test]
async fn applied_and_lww_won_acknowledge_the_exact_generations() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    fixture.insert_job("job-a", "research", "done");
    fixture.insert_job("job-b", "research", "done");
    let conn = fixture.connect();
    enqueue_outbox(&conn, "job-a", "U", 1_000, GENERATION_ONE);
    enqueue_outbox(&conn, "job-b", "U", 2_000, GENERATION_TWO);
    drop(conn);

    let api = PushApi::new(capable_api());
    api.script(push_response(
        vec![applied_push_result("job-a", 7)],
        SERVER_EPOCH,
        true,
    ));
    api.script(push_response(
        vec![PushResult {
            status: "lww_won".to_string(),
            ..applied_push_result("job-b", 8)
        }],
        SERVER_EPOCH,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.pushes_settled, 2);
    assert_eq!(outcome.pushes_pending, 0);
    let conn = fixture.connect();
    assert!(
        outbox_ids(&conn).is_empty(),
        "only the exact captured generations were cleared"
    );
    assert_eq!(recorded_seq(&conn, "job-a"), 7);
    assert_eq!(recorded_seq(&conn, "job-b"), 8);
}

#[tokio::test]
async fn a_newer_generation_racing_the_response_keeps_its_work() {
    let fixture = push_ready_fixture();
    let api = PushApi::new(capable_api());
    let db_path = fixture.db_path.clone();
    api.on_push(Box::new(move || {
        // A save during the send round trip coalesces a fresh generation.
        let conn = open_db(&db_path);
        enqueue_outbox(&conn, "job-u", "U", 4, GENERATION_TWO);
    }));
    api.script(push_response(
        vec![applied_push_result("job-u", 7)],
        SERVER_EPOCH,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(
        outcome.pushes_settled, 0,
        "the replaced generation settles nothing"
    );
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome
            .pending
            .iter()
            .any(|note| note.contains("newer local generation")),
        "{:?}",
        outcome.pending
    );
    let conn = fixture.connect();
    let entries = outbox_entries(&conn).expect("outbox");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].generation, GENERATION_TWO,
        "the newer generation survives"
    );
    assert_eq!(
        recorded_seq(&conn, "job-u"),
        7,
        "the row version still advances"
    );
}

#[tokio::test]
async fn an_already_settled_replay_reports_without_clearing_anything() {
    let fixture = push_ready_fixture();
    let api = PushApi::new(capable_api());
    let db_path = fixture.db_path.clone();
    api.on_push(Box::new(move || {
        // The exact generation was settled elsewhere during the round trip.
        let conn = open_db(&db_path);
        conn.execute(
            "DELETE FROM sync_meta WHERE key = ?1",
            rusqlite::params![format!("{OUTBOX_PREFIX}job-u")],
        )
        .expect("settle elsewhere");
    }));
    api.script(push_response(
        vec![applied_push_result("job-u", 7)],
        SERVER_EPOCH,
        true,
    ));
    let warnings = Mutex::new(Vec::new());
    let sink = |message: String| warnings.lock().expect("warnings").push(message);

    let outcome = run(&fixture, &api, &sink).await;

    assert_eq!(outcome.pushes_settled, 1);
    assert!(
        warnings
            .lock()
            .expect("warnings")
            .iter()
            .any(|warning| warning.contains("already settled")),
        "the replay is reported as already settled"
    );
    assert_eq!(
        outcome.pushes_pending, 0,
        "the replay leaves nothing pending"
    );
    assert_eq!(
        recorded_seq(&fixture.connect(), "job-u"),
        0,
        "the replay acknowledges and records nothing"
    );
}

#[tokio::test]
async fn lww_lost_installs_the_winner_report_and_preserves_the_loser() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    fixture.insert_job("job-1", "research", "done");
    let conn = fixture.connect();
    enqueue_outbox(&conn, "job-1", "U", 1_234, GENERATION_ONE);
    drop(conn);

    let winner_bytes = b"# Informe ganador\n".to_vec();
    let sha = hash_of(&winner_bytes);
    let api = PushApi::new(capable_api());
    api.inner.inner.put_blob_bytes(&sha, winner_bytes.clone());
    api.script(push_response(
        vec![PushResult {
            table: ENVELOPE_TABLE.to_string(),
            row_id: "job-1".to_string(),
            status: "lww_lost".to_string(),
            server_seq: 7,
            winner: Some(winner_row("job-1", 7, manifest_for(&winner_bytes))),
        }],
        SERVER_EPOCH,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.pushes_settled, 1);
    assert_eq!(
        outcome.conflicts_preserved, 1,
        "the loser survives in sync_conflicts"
    );
    assert_eq!(
        api.event_log(),
        vec!["push:job-1".to_string(), format!("get:{sha}")],
        "the winner report blob installs after the adjudication and before the apply"
    );
    assert_eq!(
        fs::read(
            fixture
                .artifacts_root
                .join("job-1")
                .join(REPORT_FILE_REL_PATH)
        )
        .expect("winner report"),
        winner_bytes,
        "the winner's report.md is installed"
    );
    let conn = fixture.connect();
    assert!(
        outbox_ids(&conn).is_empty(),
        "only the adjudicated generation is cleared"
    );
    assert_eq!(recorded_seq(&conn, "job-1"), 7);
    assert_eq!(
        conflict_count(&conn),
        1,
        "the divergent loser is journaled exactly once"
    );
    let state = Connection::open(&fixture.research_state).expect("state connection");
    let pregunta: String = state
        .query_row("SELECT pregunta FROM jobs WHERE id = 'job-1'", [], |row| {
            row.get(0)
        })
        .expect("projected job");
    assert_eq!(
        pregunta, "¿Pregunta de investigación?",
        "the winner projection landed"
    );
}

#[tokio::test]
async fn an_lww_lost_winner_never_replaces_active_local_work() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    fixture.insert_job("job-1", "research", "done");
    let conn = fixture.connect();
    enqueue_outbox(&conn, "job-1", "U", 1_234, GENERATION_ONE);
    drop(conn);

    let api = PushApi::new(capable_api());
    let state_path = fixture.research_state.clone();
    api.on_push(Box::new(move || {
        // The local job re-activates during the send round trip: the winner
        // may never replace active execution.
        let state = open_db(&state_path);
        state
            .execute("UPDATE jobs SET status = 'running' WHERE id = 'job-1'", [])
            .expect("reactivate job");
    }));
    api.script(push_response(
        vec![PushResult {
            table: ENVELOPE_TABLE.to_string(),
            row_id: "job-1".to_string(),
            status: "lww_lost".to_string(),
            server_seq: 7,
            winner: Some(winner_row("job-1", 7, Value::Null)),
        }],
        SERVER_EPOCH,
        true,
    ));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(outcome.pushes_settled, 0);
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome
            .pending
            .iter()
            .any(|note| note.contains("not terminal")),
        "{:?}",
        outcome.pending
    );
    let conn = fixture.connect();
    assert_eq!(
        outbox_ids(&conn),
        vec!["job-1"],
        "the adjudicated generation is retained on a Deferred outcome"
    );
    assert_eq!(
        conflict_count(&conn),
        0,
        "nothing is overwritten, so nothing is journaled"
    );
    assert_eq!(recorded_seq(&conn, "job-1"), 0);
}

#[tokio::test]
async fn a_session_change_stops_pushes_and_keeps_the_outbox() {
    let fixture = push_ready_fixture();
    let api = PushApi::new(capable_api());
    let db_path = fixture.db_path.clone();
    api.on_push(Box::new(move || {
        let conn = open_db(&db_path);
        meta_set(
            &conn,
            SYNC_SESSION_INCARNATION_KEY,
            "bbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
        )
        .expect("new incarnation");
    }));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(
        api.push_count(),
        1,
        "the loop stops at the first session change"
    );
    assert_eq!(outcome.pushes_settled, 0);
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome
            .pending
            .iter()
            .any(|note| note.contains("session changed")),
        "{:?}",
        outcome.pending
    );
    assert_eq!(
        outbox_ids(&fixture.connect()),
        vec!["job-u"],
        "nothing is acknowledged under a stale session"
    );
}

#[tokio::test]
async fn push_failures_stay_pending_in_the_outbox() {
    let fixture = push_ready_fixture();
    let api = PushApi::new(capable_api());
    api.script_error(SyncError::Network("offline".to_string()));

    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(api.push_count(), 1);
    assert_eq!(outcome.pushes_settled, 0);
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome.pending.iter().any(|note| note.contains("offline")),
        "{:?}",
        outcome.pending
    );
    assert_eq!(
        outbox_ids(&fixture.connect()),
        vec!["job-u"],
        "a failure never drops local work"
    );
}

#[tokio::test]
async fn push_attempts_are_bounded_per_cycle() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.complete_catchup();
    for index in 0..17 {
        fixture.insert_job(&format!("j-{index:02}"), "research", "done");
    }

    let api = PushApi::new(capable_api());
    let outcome = run(&fixture, &api, &no_warn()).await;

    assert_eq!(api.push_count(), 16, "the budget bounds one cycle");
    assert_eq!(outcome.pushes_settled, 16);
    assert_eq!(outcome.pushes_pending, 1);
    assert!(
        outcome.pending.iter().any(|note| note.contains("budget")),
        "{:?}",
        outcome.pending
    );
    assert_eq!(
        outbox_ids(&fixture.connect()).len(),
        1,
        "the unattempted job keeps its outbox entry"
    );
}
