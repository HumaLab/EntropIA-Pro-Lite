//! Behavior tests for the live research pull page staging (IS5a).
//!
//! Every test runs against MockSyncApi (wrapped in `ResearchApi` to count the
//! opted-in research pulls) and a throwaway schema; the mock's defaults stay
//! neutral (no capability advertisement), so each test opts in explicitly to
//! exactly the state it exercises.

use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::{json, Value};

use super::http::{
    BlobExists, DeleteAccountRequest, DevicesResponse, HealthResponse, LoginRequest, LoginResponse,
    NotificationItem, PlanCatalogItem, PlanChangeRequestResponse, PullResponse, PullRow,
    PushRequest, PushResponse, RegisterRequest, RegisterResponse, SyncApi, SyncError,
    UsageResponse, RESEARCH_ENVELOPE_V1_CAPABILITY,
};
use super::research_capture::{
    catchup_needed, record_capability, CATCHUP_KEY, ENVELOPE_TABLE, PULL_CURSOR_KEY,
};
use super::research_pull::{prepare_research_pull, pull_research_page, stage_research_pull_page};
use super::research_receive::{
    enqueue_research_receive, prepare_research_receive, queued_research_receives,
    settle_research_receive,
};
use super::session::{
    meta_delete, meta_get, meta_set, write_sync_session, SYNC_SESSION_INCARNATION_KEY,
};
use super::test_support::{new_synced_test_db, MockSyncApi};

// ---------------------------------------------------------------------------
// ResearchApi — MockSyncApi plus the exact research-envelope-v1 opt-in pull.
// The trait default fails closed, so a test double must implement the opted-in
// call explicitly; the counters prove which call the module used.
// ---------------------------------------------------------------------------

pub(super) struct ResearchApi {
    pub(super) inner: MockSyncApi,
    pub(super) research_pull_calls: Mutex<usize>,
    pub(super) research_pull_sinces: Mutex<Vec<i64>>,
    pub(super) ordinary_pull_calls: Mutex<usize>,
}

impl ResearchApi {
    pub(super) fn new(inner: MockSyncApi) -> Self {
        Self {
            inner,
            research_pull_calls: Mutex::new(0),
            research_pull_sinces: Mutex::new(Vec::new()),
            ordinary_pull_calls: Mutex::new(0),
        }
    }
}

impl SyncApi for ResearchApi {
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

    async fn pull(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        *self.ordinary_pull_calls.lock().unwrap() += 1;
        self.inner.pull(token, schema_tag, since, limit).await
    }

    async fn pull_with_research_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        *self.research_pull_calls.lock().unwrap() += 1;
        self.research_pull_sinces.lock().unwrap().push(since);
        self.inner.pull(token, schema_tag, since, limit).await
    }

    async fn blob_head(&self, token: &str, sha256: &str) -> Result<BlobExists, SyncError> {
        self.inner.blob_head(token, sha256).await
    }

    async fn blob_put(&self, token: &str, sha256: &str, bytes: Vec<u8>) -> Result<(), SyncError> {
        self.inner.blob_put(token, sha256, bytes).await
    }

    async fn blob_get(&self, token: &str, sha256: &str) -> Result<reqwest::Response, SyncError> {
        self.inner.blob_get(token, sha256).await
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn session(conn: &Connection) {
    write_sync_session(
        conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("session");
    meta_set(conn, "server_epoch", "epoch-a").expect("epoch");
}

fn discovered(conn: &Connection) {
    session(conn);
    record_capability(conn, "epoch-a", true).expect("capability");
}

fn envelope_payload(job_id: &str) -> Value {
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
        "report_file": null
    })
}

fn research_row(seq: i64, job_id: &str) -> PullRow {
    research_row_with_payload(seq, job_id, envelope_payload(job_id))
}

fn research_row_with_payload(seq: i64, job_id: &str, payload: Value) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(payload),
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

fn corpus_row(seq: i64, table: &str, row_id: &str) -> PullRow {
    PullRow {
        table: table.to_string(),
        row_id: row_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 100,
        device_id: "device-other".to_string(),
        payload: Some(json!({ "id": row_id })),
    }
}

fn page(rows: Vec<PullRow>, next_since: i64, has_more: bool) -> PullResponse {
    PullResponse {
        rows,
        next_since,
        has_more,
        schema_tag: "0023_sync_ids".to_string(),
        server_epoch: "epoch-a".to_string(),
        server_now_ms: 1_700_000_000_000,
        capabilities: vec![RESEARCH_ENVELOPE_V1_CAPABILITY.to_string()],
    }
}

fn cursor(conn: &Connection) -> Option<i64> {
    let raw = meta_get(conn, PULL_CURSOR_KEY).expect("read research cursor");
    raw.map(|raw| {
        serde_json::from_str::<Value>(&raw)
            .expect("cursor json")
            .get("since")
            .and_then(Value::as_i64)
            .expect("cursor since")
    })
}

fn queued_ids(conn: &Connection) -> Vec<String> {
    queued_research_receives(conn)
        .expect("queue")
        .into_iter()
        .map(|row| row.job_id)
        .collect()
}

// ---------------------------------------------------------------------------
// Capability / session gates (fail closed before any HTTP)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_capability_never_reaches_the_wire() {
    let conn = new_synced_test_db();
    session(&conn); // no capability advertised for epoch-a
    let api = ResearchApi::new(MockSyncApi::default());
    api.inner
        .queue_pull_page(page(vec![research_row(5, "job-1")], 5, true));

    let error = pull_research_page(&conn, &api, "token", 10)
        .await
        .expect_err("capability gate");

    assert_eq!(error.code, "research_pull_capability_missing");
    assert_eq!(*api.research_pull_calls.lock().unwrap(), 0);
    assert_eq!(*api.ordinary_pull_calls.lock().unwrap(), 0);
    assert!(queued_ids(&conn).is_empty());
    assert_eq!(cursor(&conn), None);
}

#[tokio::test]
async fn missing_session_scope_never_reaches_the_wire() {
    let conn = new_synced_test_db();
    discovered(&conn);
    meta_delete(&conn, "device_id").expect("drop device");
    let api = ResearchApi::new(MockSyncApi::default());

    let error = pull_research_page(&conn, &api, "token", 10)
        .await
        .expect_err("scope gate");

    assert_eq!(error.code, "research_pull_missing_session");
    assert_eq!(*api.research_pull_calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn missing_session_incarnation_never_reaches_the_wire() {
    let conn = new_synced_test_db();
    discovered(&conn);
    meta_delete(&conn, SYNC_SESSION_INCARNATION_KEY).expect("drop incarnation");
    let api = ResearchApi::new(MockSyncApi::default());
    api.inner
        .queue_pull_page(page(vec![research_row(5, "job-1")], 5, true));

    let error = pull_research_page(&conn, &api, "token", 10)
        .await
        .expect_err("incarnation gate");

    assert_eq!(error.code, "research_pull_missing_session");
    assert_eq!(*api.research_pull_calls.lock().unwrap(), 0);
    assert!(queued_ids(&conn).is_empty());
}

// ---------------------------------------------------------------------------
// Dedicated cursor / catch-up keys / zero cursor
// ---------------------------------------------------------------------------

#[test]
fn fresh_catchup_starts_at_zero_on_a_dedicated_cursor() {
    let conn = new_synced_test_db();
    discovered(&conn);
    meta_set(&conn, "last_pull_seq", "7").expect("shared cursor");

    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    assert_eq!(
        prepared.since, 0,
        "a fresh research catch-up starts at zero"
    );

    let staged = stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_row(5, "job-1")], 5, true),
    )
    .expect("stage");
    assert_eq!(staged.staged_job_ids, vec!["job-1".to_string()]);

    // The dedicated cursor moved; the shared corpus cursor and the writing
    // cursor are untouched.
    assert_eq!(cursor(&conn), Some(5));
    assert_eq!(
        meta_get(&conn, "last_pull_seq")
            .expect("shared cursor")
            .as_deref(),
        Some("7"),
        "last_pull_seq is never advanced by research staging"
    );
    assert_eq!(meta_get(&conn, "writing_pull_cursor").expect("read"), None);
    assert_eq!(meta_get(&conn, CATCHUP_KEY).expect("read"), None);
}

// ---------------------------------------------------------------------------
// Exact opted-in transport
// ---------------------------------------------------------------------------

#[tokio::test]
async fn opted_in_pull_is_exact_and_stages_the_page() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let mut inner = MockSyncApi::default();
    inner.server_epoch = "epoch-a".to_string();
    *inner.server_capabilities.lock().unwrap() = vec![RESEARCH_ENVELOPE_V1_CAPABILITY.to_string()];
    let api = ResearchApi::new(inner);
    api.inner
        .queue_pull_page(page(vec![research_row(5, "job-1")], 5, true));

    let staged = pull_research_page(&conn, &api, "token", 10)
        .await
        .expect("opted-in pull");

    assert_eq!(*api.research_pull_calls.lock().unwrap(), 1);
    assert_eq!(*api.ordinary_pull_calls.lock().unwrap(), 0);
    assert_eq!(staged.staged_job_ids, vec!["job-1".to_string()]);
    assert_eq!(queued_ids(&conn), vec!["job-1".to_string()]);
    assert!(
        api.inner.pushed.lock().unwrap().is_empty(),
        "no push happened"
    );

    // The next page continues from the exact research staging cursor.
    let staged = pull_research_page(&conn, &api, "token", 10)
        .await
        .expect("second pull");
    assert_eq!(*api.research_pull_calls.lock().unwrap(), 2);
    assert_eq!(
        *api.research_pull_sinces.lock().unwrap(),
        vec![0, 5],
        "the dedicated cursor drives the opted-in since"
    );
    assert!(staged.staged_job_ids.is_empty());
    assert!(!staged.catchup_recorded, "queue still holds job-1");
}

// ---------------------------------------------------------------------------
// One-transaction staging: rollback, equal-seq, foreign scope, invalid rows
// ---------------------------------------------------------------------------

#[test]
fn failed_enqueue_rolls_back_cursor_and_queue_changes() {
    let conn = new_synced_test_db();
    discovered(&conn);
    conn.execute_batch(
        "CREATE TEMP TRIGGER fail_stage_row
           BEFORE INSERT ON sync_meta
           WHEN NEW.key = 'research_receive:job-b'
           BEGIN SELECT RAISE(ABORT, 'injected stage failure'); END;",
    )
    .expect("trigger");
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let response = page(
        vec![research_row(5, "job-a"), research_row(6, "job-b")],
        6,
        true,
    );

    let error = stage_research_pull_page(&conn, &prepared, &response).expect_err("rollback");

    // The queue-contract code passes through unchanged: the durable queue has
    // one semantics no matter which module wrote the entry.
    assert_eq!(error.code, "research_receive_local_state");
    assert!(
        queued_ids(&conn).is_empty(),
        "job-a rolled back with the failed page"
    );
    assert_eq!(
        cursor(&conn),
        None,
        "cursor rolled back with the failed page"
    );
    assert!(catchup_needed(&conn, "epoch-a").expect("catchup"));
}

#[test]
fn replaying_the_same_page_is_idempotent() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let response = page(
        vec![research_row(5, "job-1"), corpus_row(6, "items", "item-1")],
        6,
        true,
    );

    let first = stage_research_pull_page(&conn, &prepared, &response).expect("first stage");
    let first_queue = queued_research_receives(&conn).expect("queue");
    assert_eq!(first_queue.len(), 1);

    let second = stage_research_pull_page(&conn, &prepared, &response).expect("replay");
    let second_queue = queued_research_receives(&conn).expect("queue");

    assert_eq!(second.staged_job_ids, first.staged_job_ids);
    assert_eq!(second_queue.len(), 1);
    assert_eq!(
        second_queue[0].captured_value, first_queue[0].captured_value,
        "equal-seq identical rows are no-ops"
    );
    assert_eq!(cursor(&conn), Some(6));
}

#[test]
fn conflicting_equal_seq_rows_fail_closed_and_retain_durable_state() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    stage_research_pull_page(
        &conn,
        &prepared,
        &page(
            vec![research_row_with_payload(5, "job-1", json!({"v": 1}))],
            5,
            true,
        ),
    )
    .expect("first stage");
    let before = queued_research_receives(&conn).expect("queue");

    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let error = stage_research_pull_page(
        &conn,
        &prepared,
        &page(
            vec![research_row_with_payload(5, "job-1", json!({"v": 2}))],
            5,
            true,
        ),
    )
    .expect_err("conflicting equal-seq row");

    assert_eq!(error.code, "research_receive_payload_conflict");
    let after = queued_research_receives(&conn).expect("queue");
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].captured_value, before[0].captured_value,
        "the conflict fails closed and retains the durable state"
    );
    assert_eq!(cursor(&conn), Some(5), "the failed page advanced nothing");
}

#[test]
fn foreign_scope_rows_fail_closed_and_retain_durable_state() {
    let conn = new_synced_test_db();
    discovered(&conn);
    enqueue_research_receive(&conn, &research_row(5, "job-1")).expect("enqueue under scope A");
    let before = queued_research_receives(&conn).expect("queue");

    // A new login incarnation: the queued row belongs to the old session.
    write_sync_session(
        &conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("new session");

    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let error = stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_row(6, "job-1")], 6, true),
    )
    .expect_err("foreign-scope row");

    assert_eq!(error.code, "research_receive_scope_conflict");
    let after = queued_research_receives(&conn).expect("queue");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].captured_value, before[0].captured_value);
    assert_eq!(cursor(&conn), None);
}

#[test]
fn structurally_invalid_rows_fail_closed() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let response = page(vec![research_row(0, "job-1")], 0, true);

    let error = stage_research_pull_page(&conn, &prepared, &response).expect_err("invalid row");

    assert_eq!(error.code, "research_receive_invalid_row");
    assert!(queued_ids(&conn).is_empty());
    assert_eq!(cursor(&conn), None);
}

// ---------------------------------------------------------------------------
// Corpus rows ride through untouched; shared cursor never advances
// ---------------------------------------------------------------------------

#[test]
fn corpus_rows_are_returned_while_research_rows_are_queued() {
    let conn = new_synced_test_db();
    discovered(&conn);
    meta_set(&conn, "last_pull_seq", "7").expect("shared cursor");
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let response = page(
        vec![
            corpus_row(10, "items", "item-1"),
            research_row(15, "job-1"),
            corpus_row(20, "assets", "asset-1"),
        ],
        20,
        true,
    );

    let staged = stage_research_pull_page(&conn, &prepared, &response).expect("stage");

    let corpus_ids: Vec<&str> = staged
        .corpus_rows
        .iter()
        .map(|row| row.row_id.as_str())
        .collect();
    assert_eq!(
        corpus_ids,
        vec!["item-1", "asset-1"],
        "corpus rows preserved"
    );
    assert_eq!(queued_ids(&conn), vec!["job-1".to_string()]);
    assert_eq!(
        meta_get(&conn, "last_pull_seq")
            .expect("shared cursor")
            .as_deref(),
        Some("7"),
        "shared last_pull_seq is never advanced by research staging"
    );
    assert!(!staged.catchup_recorded);
}

// ---------------------------------------------------------------------------
// Catch-up recording
// ---------------------------------------------------------------------------

#[test]
fn catchup_never_records_across_an_unresolved_queue() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let staged = stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_row(5, "job-1")], 5, false),
    )
    .expect("stage");
    assert!(!staged.catchup_recorded, "queued rows block completion");
    assert!(catchup_needed(&conn, "epoch-a").expect("catchup"));

    // A terminal empty page still cannot complete catch-up: job-1 is an
    // unresolved queued row.
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let staged =
        stage_research_pull_page(&conn, &prepared, &page(vec![], 5, false)).expect("stage");
    assert!(!staged.catchup_recorded);
    assert!(catchup_needed(&conn, "epoch-a").expect("catchup"));
    assert_eq!(meta_get(&conn, CATCHUP_KEY).expect("read"), None);
}

#[test]
fn empty_resolved_page_records_catchup() {
    let root = tempfile::tempdir().expect("temp workspace");
    let conn = new_synced_test_db();
    discovered(&conn);

    // Stage a tombstone page, then resolve it through the explicit settle call.
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let staged = stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_tombstone(5, "job-1")], 5, true),
    )
    .expect("stage");
    assert!(!staged.catchup_recorded);

    let state_path = root.path().join("estado.sqlite");
    entropia_agent::estado::EstadoDb::abrir(state_path.to_str().expect("utf-8 path"))
        .expect("estado schema");
    let state = Connection::open(&state_path).expect("writable state");
    let queued = prepare_research_receive(&conn, "job-1")
        .expect("prepare receive")
        .expect("queued row");
    let outcome = settle_research_receive(&conn, &state, root.path(), &queued).expect("settle");
    assert!(matches!(
        outcome,
        super::research_transport::PullApplyOutcome::NoOp
    ));
    assert!(
        queued_ids(&conn).is_empty(),
        "the settled row left the queue"
    );

    // The terminal page is empty and the queue is resolved: catch-up completes.
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    let staged =
        stage_research_pull_page(&conn, &prepared, &page(vec![], 5, false)).expect("stage");
    assert!(staged.catchup_recorded);
    assert!(!catchup_needed(&conn, "epoch-a").expect("catchup"));
    assert_eq!(
        meta_get(&conn, CATCHUP_KEY).expect("read").as_deref(),
        Some("epoch-a")
    );
    assert_eq!(cursor(&conn), Some(5));
}

// ---------------------------------------------------------------------------
// Session changes and crash safety
// ---------------------------------------------------------------------------

#[test]
fn session_change_rejects_stale_staging() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    write_sync_session(
        &conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("new session");

    let error = stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_row(5, "job-1")], 5, true),
    )
    .expect_err("stale staging");

    assert_eq!(error.code, "research_pull_stale_staging");
    assert!(
        queued_ids(&conn).is_empty(),
        "old rows are never staged or applied"
    );
    assert_eq!(cursor(&conn), None);
}

#[test]
fn stale_staging_cursor_from_a_previous_session_is_rejected() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_row(5, "job-1")], 5, true),
    )
    .expect("stage");
    assert_eq!(cursor(&conn), Some(5));

    write_sync_session(
        &conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("new session");

    let error = prepare_research_pull(&conn, 10).expect_err("stale cursor");
    assert_eq!(error.code, "research_pull_stale_staging");
}

#[test]
fn staged_rows_survive_a_crash_and_replay_idempotently() {
    let root = tempfile::tempdir().expect("temp workspace");
    let path = root.path().join("queue.sqlite");
    {
        let conn = Connection::open(&path).expect("create");
        crate::sync::schema::ensure_sync_schema(&conn).expect("schema");
        session(&conn);
        record_capability(&conn, "epoch-a", true).expect("capability");
        let prepared = prepare_research_pull(&conn, 10).expect("prepare");
        stage_research_pull_page(
            &conn,
            &prepared,
            &page(vec![research_row(5, "job-1")], 5, true),
        )
        .expect("stage");
    }

    // A crash before settlement leaves the staged row and the cursor.
    let reopened = Connection::open(&path).expect("reopen");
    assert_eq!(queued_ids(&reopened), vec!["job-1".to_string()]);
    assert_eq!(cursor(&reopened), Some(5));

    // Re-staging the same page after the crash is idempotent.
    let prepared = prepare_research_pull(&reopened, 10).expect("prepare");
    assert_eq!(prepared.since, 5, "the durable cursor survived the crash");
    let before = queued_research_receives(&reopened).expect("queue");
    stage_research_pull_page(
        &reopened,
        &prepared,
        &page(vec![research_row(5, "job-1")], 5, true),
    )
    .expect("replay");
    let after = queued_research_receives(&reopened).expect("queue");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].captured_value, before[0].captured_value);
}

#[test]
fn staged_rows_round_trip_through_the_receive_queue() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");
    stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_tombstone(7, "job-1")], 7, true),
    )
    .expect("stage");

    let queued = queued_research_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].job_id, "job-1");
    assert_eq!(queued[0].server_seq, 7);
    assert_eq!(queued[0].device_id, "device-other");
    assert!(
        prepare_research_receive(&conn, "job-1")
            .expect("prepare receive")
            .is_some(),
        "the real queue reader accepts the staged value shape"
    );
}

#[test]
fn staging_replays_rows_enqueued_by_the_receive_queue() {
    let conn = new_synced_test_db();
    discovered(&conn);
    enqueue_research_receive(&conn, &research_row(5, "job-1")).expect("enqueue");
    let before = queued_research_receives(&conn).expect("queue");
    let prepared = prepare_research_pull(&conn, 10).expect("prepare");

    let staged = stage_research_pull_page(
        &conn,
        &prepared,
        &page(vec![research_row(5, "job-1")], 5, true),
    )
    .expect("stage");

    let after = queued_research_receives(&conn).expect("queue");
    assert_eq!(staged.staged_job_ids, vec!["job-1".to_string()]);
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].captured_value, before[0].captured_value,
        "both writers agree on the durable queue identity"
    );
    assert_eq!(cursor(&conn), Some(5));
}
