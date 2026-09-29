//! Behavior tests for the W-ENGINE bounded writing sync activation
//! (`sync::writing_cycle`) against MockSyncApi and a temporary SQLite file
//! loaded from the real generated schema fixture.
//!
//! They lock the activation rules: legacy servers never see a writing call,
//! capability discovery only ever comes from an ordinary response, catch-up is
//! since-zero and precedes incremental pulls and pushes, corpus rows ride the
//! corpus path without moving the shared cursor, existing local manuscripts
//! seed into the outbox only after the catch-up is recorded (never re-seeding
//! acknowledged ones), dirty documents defer
//! automatic apply with their queue entry retained, pushes settle only under
//! the prepared session, and every failure keeps its work pending. They also
//! lock the upgrade self-heal: a pre-incarnation session gets exactly one
//! minted incarnation and writing activates on the next cycle.
//!
//! Two tests wrap MockSyncApi in `CycleApi` to advertise non-zero health limits
//! and to swap the session identity between a writing send and its settlement.

use std::fs;
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::engine::run_cycle;
use super::http::{
    BlobExists, DeleteAccountRequest, DevicesResponse, HealthLimits, HealthResponse, LoginRequest,
    LoginResponse, NotificationItem, PlanCatalogItem, PlanChangeRequestResponse, PullResponse,
    PullRow, PushRequest, PushResponse, PushResult, RegisterRequest, RegisterResponse, SyncApi,
    SyncError, UsageResponse, WRITING_ENVELOPE_V1_CAPABILITY,
};
use super::session::{
    ensure_session_incarnation, meta_delete, meta_get, meta_get_i64, meta_set,
    read_session_incarnation, write_sync_session, SYNC_SESSION_INCARNATION_KEY,
};
use super::test_support::{MockBlobFailure, MockSyncApi, SCHEMA_FIXTURE};
use super::writing_cycle::{run_writing_cycle, WritingDiscovery};
use super::writing_receive::queued_writing_receives;
use crate::writing::sync_capture::{
    enqueue_document_at_for_test, outbox_entries, record_capability, supports_writing,
    ENVELOPE_TABLE, PULL_CURSOR_KEY,
};
use crate::writing::sync_envelope::{
    AttachmentFileV1, AttachmentManifestV1, CitationProjectionsV1, DocumentSettingsV1,
    WritingEnvelopeV1,
};
use crate::writing::sync_transport::{catchup_needed, record_catchup_done};

const SERVER_EPOCH: &str = "mock-epoch";
const ACCOUNT_ID: &str = "account-a";
const DEVICE_ID: &str = "device-a";
const SERVER_URL: &str = "https://sync.example.test";
const SESSION_INCARNATION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OTHER_INCARNATION: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    _temp: TempDir,
    db_path: PathBuf,
    data_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary fixture");
        let db_path = temp.path().join("archive.sqlite");
        let data_root = temp.path().join("app-data");
        fs::create_dir_all(&data_root).expect("data root");

        let conn = open_db(&db_path);
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("real generated application schema");
        crate::sync::schema::ensure_sync_schema(&conn).expect("sync schema");
        conn.execute_batch(
            "INSERT INTO _migrations(name, applied_at) VALUES('0035_writing_workspace', 1);
             INSERT INTO _migrations(name, applied_at) VALUES('0036_writing_journal', 2);",
        )
        .expect("migration head");
        drop(conn);

        let fixture = Self {
            _temp: temp,
            db_path,
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

    fn connect(&self) -> Connection {
        open_db(&self.db_path)
    }

    fn enable_capability(&self) {
        record_capability(&self.connect(), SERVER_EPOCH, true).expect("record capability");
    }

    fn complete_catchup(&self) {
        record_catchup_done(&self.connect(), SERVER_EPOCH).expect("record catchup");
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

fn set_meta(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .expect("set sync metadata");
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

fn capable_api() -> MockSyncApi {
    let api = MockSyncApi::default();
    *api.server_capabilities.lock().expect("server capabilities") =
        vec![WRITING_ENVELOPE_V1_CAPABILITY.to_string()];
    api
}

fn text_content(text: &str) -> Value {
    json!({
        "schemaVersion": 1,
        "doc": {
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }]
        }
    })
}

fn text_with_image(text: &str, rel_path: &str) -> Value {
    json!({
        "schemaVersion": 1,
        "doc": {
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": text }] },
                { "type": "writingImage", "attrs": { "src": rel_path } }
            ]
        }
    })
}

fn envelope_payload_with(document_id: &str, content: Value, files: Vec<AttachmentFileV1>) -> Value {
    let envelope = WritingEnvelopeV1 {
        id: document_id.to_string(),
        envelope_version: 1,
        content_json: content,
        title: "Remoto".to_string(),
        document_type: "article".to_string(),
        status: "active".to_string(),
        schema_version: 1,
        settings: DocumentSettingsV1 {
            citation_style_id: None,
            citation_locale: None,
            bibliography_enabled: true,
        },
        collection_associations: Vec::new(),
        citation_projections: CitationProjectionsV1 {
            corpus: Vec::new(),
            zotero: Vec::new(),
        },
        attachments_manifest: AttachmentManifestV1::Validated { files },
    };
    serde_json::from_str(&envelope.to_canonical_json().expect("envelope json"))
        .expect("envelope value")
}

fn envelope_payload(document_id: &str, text: &str) -> Value {
    envelope_payload_with(document_id, text_content(text), Vec::new())
}

fn png_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(payload);
    bytes
}

fn attachment_entry(payload: &[u8]) -> AttachmentFileV1 {
    let bytes = png_bytes(payload);
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    AttachmentFileV1 {
        rel_path: format!("writing-images/{sha256}.png"),
        sha256,
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    }
}

fn writing_row(seq: i64, document_id: &str, text: &str) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: document_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-remote".to_string(),
        payload: Some(envelope_payload(document_id, text)),
    }
}

fn writing_row_with_attachment(
    seq: i64,
    document_id: &str,
    text: &str,
    entry: &AttachmentFileV1,
) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: document_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-remote".to_string(),
        payload: Some(envelope_payload_with(
            document_id,
            text_with_image(text, &entry.rel_path),
            vec![AttachmentFileV1 {
                rel_path: entry.rel_path.clone(),
                sha256: entry.sha256.clone(),
                size: entry.size,
                media_type: entry.media_type.clone(),
            }],
        )),
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
            vec![WRITING_ENVELOPE_V1_CAPABILITY.to_string()]
        } else {
            Vec::new()
        },
    }
}

// ---------------------------------------------------------------------------
// Local state helpers
// ---------------------------------------------------------------------------

fn insert_document(conn: &Connection, document_id: &str, text: &str) {
    conn.execute(
        "INSERT INTO writing_documents
           (id, title, document_type, status, schema_version, current_content_json,
            revision, plain_text_cache, created_at, updated_at)
         VALUES (?1, 'Local', 'article', 'active', 1, ?2, 0, '', 1, 1)",
        rusqlite::params![document_id, text_content(text).to_string()],
    )
    .expect("insert writing document");
}

fn mark_acknowledged(conn: &Connection, document_id: &str, server_seq: i64) {
    conn.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq) VALUES (?1, ?2, ?3)",
        rusqlite::params![ENVELOPE_TABLE, document_id, server_seq],
    )
    .expect("mark document acknowledged");
}

fn seed_journal_delta(conn: &Connection, document_id: &str) {
    conn.execute(
        "INSERT INTO writing_journal
           (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
         VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM writing_journal
                       WHERE document_id = ?1),
                 0, 1, '[{\"step\":\"typing\"}]', 'checksum', 1)",
        [document_id],
    )
    .expect("seed journal delta");
}

fn stale_cursor_json() -> String {
    json!({
        "scope": {
            "account_id": ACCOUNT_ID,
            "server_url": SERVER_URL,
            "device_id": DEVICE_ID,
            "server_epoch": SERVER_EPOCH,
            "session_incarnation": OTHER_INCARNATION,
        },
        "since": 42
    })
    .to_string()
}

fn shared_cursor(conn: &Connection) -> i64 {
    meta_get_i64(conn, "last_pull_seq").expect("shared cursor")
}

fn writing_cursor(conn: &Connection) -> Option<(String, i64)> {
    meta_get(conn, PULL_CURSOR_KEY)
        .expect("staging cursor")
        .map(|raw| {
            let value: Value = serde_json::from_str(&raw).expect("cursor json");
            let incarnation = value["scope"]["session_incarnation"]
                .as_str()
                .expect("cursor incarnation")
                .to_string();
            let since = value["since"].as_i64().expect("cursor since");
            (incarnation, since)
        })
}

fn queued_ids(conn: &Connection) -> Vec<String> {
    queued_writing_receives(conn)
        .expect("receive queue")
        .into_iter()
        .map(|row| row.document_id)
        .collect()
}

fn outbox_ids(conn: &Connection) -> Vec<String> {
    outbox_entries(conn)
        .expect("outbox")
        .into_iter()
        .map(|entry| entry.document_id)
        .collect()
}

fn document_ids(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT id FROM writing_documents ORDER BY id")
        .expect("documents statement")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("document rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("document ids")
}

fn text_of(conn: &Connection, document_id: &str) -> Option<String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT current_content_json FROM writing_documents WHERE id = ?1",
            [document_id],
            |row| row.get(0),
        )
        .optional()
        .expect("document content");
    raw.map(|raw| {
        let value: Value = serde_json::from_str(&raw).expect("content json");
        value["doc"]["content"][0]["content"][0]["text"]
            .as_str()
            .expect("text")
            .to_string()
    })
}

fn collections_count(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM collections WHERE id = ?1",
        [id],
        |row| row.get(0),
    )
    .expect("count collections")
}

fn recorded_writing_seq(conn: &Connection, document_id: &str) -> Option<i64> {
    conn.query_row(
        "SELECT server_seq FROM sync_row_versions
          WHERE table_name = ?1 AND row_id = ?2",
        rusqlite::params![ENVELOPE_TABLE, document_id],
        |row| row.get(0),
    )
    .optional()
    .expect("recorded writing seq")
}

fn no_warn() -> impl Fn(String) + Sync {
    |_| {}
}

// ---------------------------------------------------------------------------
// CycleApi: MockSyncApi with advertised health limits and an optional session
// swap that fires between a writing send and its settlement.
// ---------------------------------------------------------------------------

struct CycleApi {
    inner: MockSyncApi,
    max_push_bytes: i64,
    swap_session_db: Option<PathBuf>,
    /// When set to `(row_id, result)`, a writing push for exactly that row is
    /// answered with this exact per-row result (e.g. one validated `lww_lost`
    /// carrying its winner in pull format); every other push delegates to the
    /// mock's ordinary `applied` response.
    lww_lost_result: Option<(String, PushResult)>,
}

impl CycleApi {
    fn new(inner: MockSyncApi) -> Self {
        Self {
            inner,
            max_push_bytes: 1_000_000,
            swap_session_db: None,
            lww_lost_result: None,
        }
    }
}

impl SyncApi for CycleApi {
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
        Ok(HealthResponse {
            status: "ok".to_string(),
            version: "test".to_string(),
            epoch: self.inner.server_epoch.clone(),
            server_now_ms: self.inner.server_now_ms,
            limits: HealthLimits {
                max_push_bytes: self.max_push_bytes,
                max_blob_mb: 1,
            },
        })
    }

    async fn push(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        self.inner.push(token, schema_tag, req).await
    }

    async fn push_with_writing_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        if let Some(db_path) = &self.swap_session_db {
            // A login in the middle of the send: the settlement must refuse to
            // acknowledge anything under the new incarnation.
            let conn = open_db(db_path);
            write_sync_session(
                &conn,
                SERVER_URL,
                ACCOUNT_ID,
                "reader@example.test",
                DEVICE_ID,
            )
            .expect("swap session");
            meta_set(&conn, "server_epoch", SERVER_EPOCH).expect("swap epoch");
        }
        if let Some((row_id, result)) = &self.lww_lost_result {
            if req.changes.len() == 1 && req.changes[0].row_id == *row_id {
                // One validated lww_lost result carrying the winner in pull
                // format, bound to the requested row and sequence.
                return Ok(PushResponse {
                    results: vec![result.clone()],
                    max_server_seq: result.server_seq,
                    server_epoch: SERVER_EPOCH.to_string(),
                    server_now_ms: 1_700_000_000_000,
                    capabilities: vec![WRITING_ENVELOPE_V1_CAPABILITY.to_string()],
                });
            }
        }
        self.inner
            .push_with_writing_envelope_v1(token, schema_tag, req)
            .await
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

    async fn pull_with_writing_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.inner
            .pull_with_writing_envelope_v1(token, schema_tag, since, limit)
            .await
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
// Legacy server: no writing calls, corpus behavior unchanged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn legacy_server_gets_no_writing_calls_and_corpus_behavior_is_unchanged() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    set_meta(&conn, "seeded_account", ACCOUNT_ID);
    set_meta(&conn, "last_pull_seq", "5");

    let api = MockSyncApi::default(); // never advertises the capability
    api.queue_pull_page(page(vec![corpus_row(10, "cr1")], 10, false, false));

    run_cycle(&api, "tok", &conn, &fixture.data_root, &no_warn())
        .await
        .expect("cycle ok");

    // Corpus behaves exactly as before the writing phase existed.
    assert_eq!(collections_count(&conn, "cr1"), 1, "corpus row applied");
    assert_eq!(shared_cursor(&conn), 10, "corpus cursor advanced normally");

    // Not one writing request of any kind ran.
    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 0);
    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 0);
    assert!(api.sync_events.lock().unwrap().is_empty());

    // Discovery saw "not advertised" on the ordinary response and left no
    // writing state behind.
    assert!(!supports_writing(&conn, SERVER_EPOCH).expect("capability record"));
    assert!(meta_get(&conn, PULL_CURSOR_KEY).unwrap().is_none());
    assert!(queued_writing_receives(&conn).unwrap().is_empty());
    assert!(outbox_ids(&conn).is_empty());
}

// ---------------------------------------------------------------------------
// Upgrade self-heal: pre-incarnation sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgraded_session_without_incarnation_self_heals_and_activates_writing() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    // Production upgrade shape: a complete pre-feature session whose only
    // missing piece is the incarnation.
    meta_delete(&conn, SYNC_SESSION_INCARNATION_KEY).expect("drop incarnation");
    assert_eq!(meta_get(&conn, SYNC_SESSION_INCARNATION_KEY).unwrap(), None);

    let api = capable_api();
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    // The phase ran (not NoSession) and minted exactly one incarnation.
    assert_eq!(outcome.discovery, Some(WritingDiscovery::AlreadyKnown));
    assert!(outcome.catchup_recorded);
    let minted = meta_get(&conn, SYNC_SESSION_INCARNATION_KEY)
        .unwrap()
        .expect("minted incarnation");
    let (incarnation, since) = writing_cursor(&conn).expect("staging cursor");
    assert_eq!(incarnation, minted, "cursor bound to the minted identity");
    assert_eq!(since, 0, "catch-up ran from since-zero under the mint");

    // The next cycle reuses the same incarnation and keeps writing active.
    let next = fixture.connect();
    let next_api = capable_api();
    let next_outcome = run_writing_cycle(
        &next_api,
        "tok",
        &next,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;
    assert_eq!(next_outcome.discovery, Some(WritingDiscovery::AlreadyKnown));
    assert!(!next_outcome.catchup_needed_at_start);
    assert_eq!(
        meta_get(&next, SYNC_SESSION_INCARNATION_KEY)
            .unwrap()
            .as_deref(),
        Some(minted.as_str()),
        "no second incarnation was minted"
    );
    assert_eq!(*next_api.writing_capability_pull_calls.lock().unwrap(), 1);
}

#[test]
fn the_minted_incarnation_is_shared_across_connections_and_never_reminted() {
    let fixture = Fixture::new();
    let first = fixture.connect();
    meta_delete(&first, SYNC_SESSION_INCARNATION_KEY).expect("drop incarnation");

    let minted = ensure_session_incarnation(&first)
        .expect("first mint")
        .expect("minted incarnation");
    drop(first);

    // A later cycle opens a fresh connection: the persisted incarnation wins
    // and a second one is never minted.
    let second = fixture.connect();
    assert_eq!(
        ensure_session_incarnation(&second).expect("second call"),
        Some(minted)
    );
    assert_eq!(read_session_incarnation(&second).unwrap(), Some(minted));
}

#[tokio::test]
async fn no_session_stays_missing_and_the_writing_phase_reports_no_session() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    // A logged-out database: no identity and no incarnation at all.
    for key in [
        "account_id",
        "server_url",
        "device_id",
        SYNC_SESSION_INCARNATION_KEY,
    ] {
        meta_delete(&conn, key).expect("drop session metadata");
    }

    let api = capable_api();
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(outcome.discovery, Some(WritingDiscovery::NoSession));
    assert_eq!(
        meta_get(&conn, SYNC_SESSION_INCARNATION_KEY).unwrap(),
        None,
        "nothing is ever minted without a session"
    );
    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 0);
    assert!(api.sync_events.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Discovery → since-zero catch-up → staged and applied rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capability_discovery_then_since_zero_catchup_stages_and_applies_rows() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    set_meta(&conn, "last_pull_seq", "5");
    // A staging cursor from a vanished session: stale, never reusable, so the
    // catch-up must restart from since=0.
    set_meta(&conn, PULL_CURSOR_KEY, &stale_cursor_json());

    let api = capable_api();
    // Ordinary discovery page (no opt-in): one corpus row + the capability.
    api.queue_pull_page(page(vec![corpus_row(11, "cr1")], 11, false, true));
    api.queue_pull_page(page(
        vec![writing_row(3, "doc-a", "Remoto A")],
        3,
        true,
        true,
    ));
    api.queue_pull_page(page(
        vec![writing_row(7, "doc-b", "Remoto B")],
        7,
        false,
        true,
    ));

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(outcome.discovery, Some(WritingDiscovery::Recorded));
    assert!(outcome.catchup_needed_at_start);
    assert!(outcome.catchup_recorded);
    assert_eq!(outcome.pages_staged, 3);
    assert_eq!(outcome.writing_rows_staged, 2);
    assert_eq!(outcome.receives_settled, 2);
    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 3);

    // Staged durably, then applied.
    assert!(queued_ids(&conn).is_empty());
    assert_eq!(text_of(&conn, "doc-a").as_deref(), Some("Remoto A"));
    assert_eq!(text_of(&conn, "doc-b").as_deref(), Some("Remoto B"));

    // The discovery response's corpus row rode the corpus path...
    assert_eq!(outcome.corpus_rows_routed, 1);
    assert_eq!(collections_count(&conn, "cr1"), 1);
    // ...without advancing the shared cursor.
    assert_eq!(shared_cursor(&conn), 5);

    // The stale cursor never survived: the new one belongs to this very session
    // and sits BELOW the stale 42, proving the restart from since=0.
    let (incarnation, since) = writing_cursor(&conn).expect("staging cursor");
    assert_eq!(incarnation, SESSION_INCARNATION);
    assert_eq!(since, 7);
    assert!(!catchup_needed(&conn, SERVER_EPOCH).expect("catchup state"));
}

#[tokio::test]
async fn catchup_requests_start_at_since_zero() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();

    let api = capable_api();
    // No staged rows: the opted-in fetch is answered with an empty terminal page
    // that echoes the requested `since` back into the staging cursor.
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 1);
    assert!(outcome.catchup_recorded);
    let (incarnation, since) = writing_cursor(&conn).expect("staging cursor");
    assert_eq!(incarnation, SESSION_INCARNATION);
    assert_eq!(since, 0, "the since-zero catch-up requests since=0");
}

// ---------------------------------------------------------------------------
// Incremental pulls: staged + applied, corpus rows preserved, cursor safe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn incremental_writing_page_is_staged_then_applied_without_moving_the_corpus_cursor() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();
    set_meta(&conn, "last_pull_seq", "5");

    let api = capable_api();
    api.queue_pull_page(page(
        vec![writing_row(9, "doc-c", "Remoto C"), corpus_row(12, "cr2")],
        9,
        false,
        true,
    ));

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(outcome.discovery, Some(WritingDiscovery::AlreadyKnown));
    assert!(!outcome.catchup_needed_at_start);
    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 1);
    assert_eq!(outcome.writing_rows_staged, 1);
    assert_eq!(outcome.receives_settled, 1);
    assert!(queued_ids(&conn).is_empty());
    assert_eq!(text_of(&conn, "doc-c").as_deref(), Some("Remoto C"));

    // The page's corpus row is applied through the corpus path...
    assert_eq!(outcome.corpus_rows_routed, 1);
    assert_eq!(collections_count(&conn, "cr2"), 1);
    // ...and the shared cursor never moves for a writing page.
    assert_eq!(shared_cursor(&conn), 5);

    let (_, since) = writing_cursor(&conn).expect("staging cursor");
    assert_eq!(since, 9);
}

// ---------------------------------------------------------------------------
// W-GUARD2 dirty barrier: defer automatic apply, retain the queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dirty_journal_and_outbox_defer_automatic_apply_and_keep_the_queue() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    set_meta(&conn, "last_pull_seq", "5");

    // Local durable work: an unrecovered journal delta on doc-j and an
    // unacknowledged save on doc-o.
    insert_document(&conn, "doc-j", "Texto local J");
    seed_journal_delta(&conn, "doc-j");
    insert_document(&conn, "doc-o", "Texto local O");
    enqueue_document_at_for_test(&conn, "doc-o", 50_000).expect("outbox");

    let api = capable_api();
    api.queue_pull_page(page(
        vec![
            writing_row(20, "doc-j", "Remoto J"),
            writing_row(21, "doc-o", "Remoto O"),
        ],
        21,
        false,
        true,
    ));

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(outcome.receives_settled, 0);
    assert_eq!(outcome.receives_deferred, 2);
    assert_eq!(
        queued_ids(&conn),
        vec!["doc-j".to_string(), "doc-o".to_string()],
        "deferred rows stay durably queued"
    );
    assert_eq!(text_of(&conn, "doc-j").as_deref(), Some("Texto local J"));
    assert_eq!(text_of(&conn, "doc-o").as_deref(), Some("Texto local O"));
    assert!(
        catchup_needed(&conn, SERVER_EPOCH).expect("catchup state"),
        "deferred rows keep the catch-up open"
    );

    // With catch-up open, push stays gated: nothing was sent and the outbox is
    // retained.
    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 0);
    assert_eq!(outbox_ids(&conn), vec!["doc-o".to_string()]);
}

// ---------------------------------------------------------------------------
// Push runs only after the epoch's catch-up is recorded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn writing_push_uses_opt_in_only_after_the_catchup_is_recorded() {
    // BEFORE: capability known, catch-up still open → no opt-in push at all.
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    insert_document(&conn, "doc-push", "Local");
    enqueue_document_at_for_test(&conn, "doc-push", 50_000).expect("outbox");

    let api = capable_api();
    api.set_cursor_ahead(1); // the catch-up fetch fails; catch-up stays open

    run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert!(catchup_needed(&conn, SERVER_EPOCH).expect("catchup state"));
    assert_eq!(
        *api.writing_capability_push_calls.lock().unwrap(),
        0,
        "no writing push opt-in before the catch-up is recorded"
    );
    assert_eq!(outbox_ids(&conn), vec!["doc-push".to_string()]);

    // AFTER: same work runs once the epoch's catch-up is on record.
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();
    insert_document(&conn, "doc-push", "Local");
    enqueue_document_at_for_test(&conn, "doc-push", 50_000).expect("outbox");

    let api = capable_api();
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(outcome.pushes_settled, 1);
    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 1);
    assert!(
        outbox_ids(&conn).is_empty(),
        "the settled push acknowledges its outbox generation"
    );
    assert_eq!(recorded_writing_seq(&conn, "doc-push"), Some(1));
}

// ---------------------------------------------------------------------------
// Seeding existing manuscripts runs only after the catch-up is recorded
// ---------------------------------------------------------------------------

/// The production gap: manuscripts that predate sync capture have no outbox
/// entry, so nothing ever pushed them. The cycle seeds them through
/// `seed_outbox` — but only after the epoch's catch-up is recorded, so remote
/// tombstones apply first and a deleted manuscript is never resurrected.
#[tokio::test]
async fn existing_unacknowledged_documents_seed_and_push_only_after_catchup() {
    // WHILE the catch-up is open: nothing is seeded and nothing is pushed.
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    insert_document(&conn, "doc-seed", "Local"); // no outbox entry: pre-capture

    let api = capable_api();
    api.set_cursor_ahead(1); // the catch-up fetch fails; catch-up stays open

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert!(catchup_needed(&conn, SERVER_EPOCH).expect("catchup state"));
    assert_eq!(
        outcome.documents_seeded, 0,
        "no seeding while the catch-up is still open"
    );
    assert!(
        outbox_ids(&conn).is_empty(),
        "the manuscript is never enqueued before remote tombstones apply"
    );
    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 0);

    // The cycle that COMPLETES the catch-up seeds and pushes in the same run.
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    insert_document(&conn, "doc-seed", "Local");

    let api = capable_api();
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert!(outcome.catchup_recorded, "this cycle recorded the catch-up");
    assert_eq!(outcome.documents_seeded, 1);
    assert_eq!(
        outcome.pushes_settled, 1,
        "the seeded manuscript is pushed in the same cycle"
    );
    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 1);
    assert!(
        outbox_ids(&conn).is_empty(),
        "the push acknowledges the seed"
    );
    assert_eq!(recorded_writing_seq(&conn, "doc-seed"), Some(1));

    // LATER cycles (catch-up already on record) seed and push too.
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();
    insert_document(&conn, "doc-late", "Local");

    let api = capable_api();
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert!(!outcome.catchup_needed_at_start);
    assert_eq!(outcome.documents_seeded, 1);
    assert_eq!(outcome.pushes_settled, 1);
    assert_eq!(recorded_writing_seq(&conn, "doc-late"), Some(1));
}

/// Seeding is a one-time backfill: an already acknowledged manuscript is never
/// re-seeded, and a seeded entry is never seeded twice.
#[tokio::test]
async fn seeding_skips_acknowledged_documents_and_never_repeats() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();

    // Acknowledged by the server already: must never return to the outbox.
    insert_document(&conn, "doc-ack", "Local");
    mark_acknowledged(&conn, "doc-ack", 7);
    // Never acknowledged: seeded exactly once.
    insert_document(&conn, "doc-new", "Local");

    let api = capable_api();
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(
        outcome.documents_seeded, 1,
        "only the unacknowledged manuscript is seeded"
    );
    assert_eq!(outcome.pushes_settled, 1);
    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 1);
    assert!(outbox_ids(&conn).is_empty());

    // A later cycle: idempotent, nothing re-seeded, nothing re-pushed.
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(outcome.documents_seeded, 0, "seeding never repeats");
    assert_eq!(outcome.pushes_settled, 0);
    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 1);
    assert!(
        outbox_ids(&conn).is_empty(),
        "the acknowledged manuscript is never re-enqueued"
    );
}

// ---------------------------------------------------------------------------
// Session change prevents stale settlement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_change_between_send_and_settle_prevents_stale_settlement() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();
    insert_document(&conn, "doc-push", "Local");
    enqueue_document_at_for_test(&conn, "doc-push", 50_000).expect("outbox");

    let api = CycleApi {
        swap_session_db: Some(fixture.db_path.clone()),
        ..CycleApi::new(capable_api())
    };

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(
        *api.inner.writing_capability_push_calls.lock().unwrap(),
        1,
        "the request was sent under the prepared session"
    );
    assert_eq!(
        outbox_ids(&conn),
        vec!["doc-push".to_string()],
        "settlement never runs under the new session; the outbox stays"
    );
    assert_eq!(recorded_writing_seq(&conn, "doc-push"), None);
    assert!(outcome.pushes_pending >= 1);
    assert!(
        !outcome.pending.is_empty(),
        "the session change is reported"
    );
}

// ---------------------------------------------------------------------------
// Failures stay pending
// ---------------------------------------------------------------------------

#[tokio::test]
async fn push_and_download_failures_keep_outbox_and_queue_pending() {
    // Push failure: the outbox entry survives and settles on a later cycle.
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();
    insert_document(&conn, "doc-push", "Local");
    enqueue_document_at_for_test(&conn, "doc-push", 50_000).expect("outbox");

    let api = capable_api();
    *api.writing_push_failure.lock().unwrap() =
        Some(MockBlobFailure::Network("connection reset".to_string()));

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(*api.writing_capability_push_calls.lock().unwrap(), 1);
    assert_eq!(
        outbox_ids(&conn),
        vec!["doc-push".to_string()],
        "a failed push never drops the outbox"
    );
    assert!(outcome.pushes_pending >= 1);
    assert!(!outcome.pending.is_empty());

    *api.writing_push_failure.lock().unwrap() = None;
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(
        outcome.pushes_settled, 1,
        "the retry settles the same outbox entry"
    );
    assert!(outbox_ids(&conn).is_empty());

    // Download failure: the staged row stays queued and no document is applied.
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();

    let api = capable_api(); // the attachment hash is not served → blob GET 404
    let entry = attachment_entry(&[1, 2, 3, 4]);
    api.queue_pull_page(page(
        vec![writing_row_with_attachment(
            30,
            "doc-img",
            "Con imagen",
            &entry,
        )],
        30,
        false,
        true,
    ));

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(outcome.receives_settled, 0);
    assert_eq!(outcome.receives_deferred, 1);
    assert_eq!(
        queued_ids(&conn),
        vec!["doc-img".to_string()],
        "a failed download keeps the row durably queued"
    );
    assert_eq!(
        text_of(&conn, "doc-img"),
        None,
        "no document is applied before its blobs install"
    );
    assert!(!outcome.pending.is_empty());
}

// ---------------------------------------------------------------------------
// The engine hook: the writing phase runs inside the real cycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_cycle_runs_the_writing_phase_after_the_corpus_pull() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    set_meta(&conn, "seeded_account", ACCOUNT_ID);
    fixture.enable_capability();
    fixture.complete_catchup();
    insert_document(&conn, "doc-push", "Local");
    enqueue_document_at_for_test(&conn, "doc-push", 50_000).expect("outbox");

    let api = CycleApi::new(capable_api());

    run_cycle(&api, "tok", &conn, &fixture.data_root, &no_warn())
        .await
        .expect("cycle ok");

    assert_eq!(
        *api.inner.writing_capability_pull_calls.lock().unwrap(),
        1,
        "the cycle performed its incremental writing pull"
    );
    assert_eq!(
        *api.inner.writing_capability_push_calls.lock().unwrap(),
        1,
        "the cycle pushed the pending writing change with opt-in"
    );
    assert!(outbox_ids(&conn).is_empty());
    assert_eq!(recorded_writing_seq(&conn, "doc-push"), Some(1));
    assert_eq!(
        shared_cursor(&conn),
        0,
        "the corpus cursor stayed untouched"
    );
}

// ---------------------------------------------------------------------------
// WS6 deadlock: a validated lww_lost winner settles the divergent edit
// ---------------------------------------------------------------------------

/// The real two-device deadlock: the automatic cycle defers a queued remote
/// winner while a local outbox entry exists, and the `lww_lost` push retains
/// that same outbox entry — so the visible conflict copy never runs. The
/// validated winner must settle through the preserve-loser-then-receive path
/// instead: exactly ONE visible conflict copy carrying the losing manuscript,
/// the winner on the document, the copy enqueued to sync back, and no
/// duplicate copy when the retained staged row drains on a later cycle.
#[tokio::test]
async fn lww_lost_settles_the_divergent_edit_deadlock_with_one_conflict_copy() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();

    // The divergent local edit: saved, unpushed.
    insert_document(&conn, "doc-d", "Texto local B");
    enqueue_document_at_for_test(&conn, "doc-d", 50_000).expect("outbox");

    // The remote LWW winner arrives twice: staged by the pull (deferred behind
    // the outbox barrier) and validated inside the lww_lost push result.
    let winner = writing_row(25, "doc-d", "Texto remoto A");
    let api = CycleApi {
        lww_lost_result: Some((
            "doc-d".to_string(),
            PushResult {
                table: ENVELOPE_TABLE.to_string(),
                row_id: "doc-d".to_string(),
                status: "lww_lost".to_string(),
                server_seq: 25,
                winner: Some(winner.clone()),
            },
        )),
        ..CycleApi::new(capable_api())
    };
    api.inner
        .queue_pull_page(page(vec![winner], 25, false, true));

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    // The deadlock is gone in the same cycle: the winner lands and the losing
    // manuscript survives exactly once.
    assert_eq!(
        text_of(&conn, "doc-d").as_deref(),
        Some("Texto remoto A"),
        "the LWW winner lands on the document"
    );
    let mut copies = document_ids(&conn);
    copies.retain(|id| id != "doc-d");
    assert_eq!(copies.len(), 1, "exactly one visible conflict copy");
    let copy_id = copies.remove(0);
    assert!(
        copy_id.starts_with("writing-conflict-"),
        "the conflict copy keeps its deterministic id, got {copy_id}"
    );
    assert_eq!(
        text_of(&conn, &copy_id).as_deref(),
        Some("Texto local B"),
        "the conflict copy carries the losing manuscript"
    );

    // The copy syncs back and the lost push is never retried.
    assert!(
        outbox_ids(&conn).is_empty(),
        "the adjudicated generation settled with its content preserved"
    );
    assert!(
        api.inner
            .pushed
            .lock()
            .unwrap()
            .iter()
            .any(|change| change.row_id == copy_id),
        "the conflict copy was enqueued to sync back and pushed"
    );
    assert_eq!(
        queued_ids(&conn),
        vec!["doc-d".to_string()],
        "the staged winner stays queued until its next drain"
    );
    assert_eq!(outcome.pushes_settled, 2, "both documents settled once");

    // A later cycle drains the staged row as stale and never duplicates the
    // conflict copy.
    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;
    assert!(
        queued_ids(&conn).is_empty(),
        "the staged winner drains as a stale no-op"
    );
    assert_eq!(
        document_ids(&conn).len(),
        2,
        "still exactly one conflict copy after the drain"
    );
    assert_eq!(outcome.pushes_settled, 0, "nothing was left to push");
}

/// Unsaved-work protection on the adjudicated route: a pending recovery
/// journal still defers instead of being overwritten — no conflict copy,
/// nothing cleared, everything retained.
#[tokio::test]
async fn pending_recovery_journal_defers_the_lww_lost_conflict_settlement() {
    let fixture = Fixture::new();
    let conn = fixture.connect();
    fixture.enable_capability();
    fixture.complete_catchup();

    insert_document(&conn, "doc-j", "Texto local J");
    seed_journal_delta(&conn, "doc-j");
    enqueue_document_at_for_test(&conn, "doc-j", 50_000).expect("outbox");

    let winner = writing_row(30, "doc-j", "Texto remoto J");
    let api = CycleApi {
        lww_lost_result: Some((
            "doc-j".to_string(),
            PushResult {
                table: ENVELOPE_TABLE.to_string(),
                row_id: "doc-j".to_string(),
                status: "lww_lost".to_string(),
                server_seq: 30,
                winner: Some(winner.clone()),
            },
        )),
        ..CycleApi::new(capable_api())
    };
    api.inner
        .queue_pull_page(page(vec![winner], 30, false, true));

    let outcome = run_writing_cycle(
        &api,
        "tok",
        &conn,
        &fixture.data_root,
        &health(),
        &no_warn(),
    )
    .await;

    assert_eq!(
        outcome.conflict_copies_preserved, 0,
        "nothing settles while the recovery journal is unrecovered"
    );
    assert_eq!(
        text_of(&conn, "doc-j").as_deref(),
        Some("Texto local J"),
        "the pending manuscript is never overwritten"
    );
    assert_eq!(document_ids(&conn).len(), 1, "no conflict copy is created");
    assert_eq!(
        outbox_ids(&conn),
        vec!["doc-j".to_string()],
        "the adjudicated generation is retained"
    );
    assert_eq!(
        queued_ids(&conn),
        vec!["doc-j".to_string()],
        "the staged winner stays queued"
    );
    assert!(!outcome.pending.is_empty(), "the deferral is reported");
}
