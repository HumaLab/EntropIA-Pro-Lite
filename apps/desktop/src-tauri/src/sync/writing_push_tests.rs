//! Inactive writing PUSH orchestration tests. They use a temporary SQLite file
//! loaded from the real generated migration fixture and the network-free mock.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::http::{
    HealthLimits, HealthResponse, PullRow, PushResponse, PushResult, WRITING_ENVELOPE_V1_CAPABILITY,
};
use super::session::{
    clear_sync_state, read_session_incarnation, write_sync_session, SYNC_SESSION_INCARNATION_KEY,
};
use super::test_support::{MockBlobFailure, MockSyncApi, MockSyncEvent, SCHEMA_FIXTURE};
use super::writing_blobs::WritingBlobPendingKind;
use super::writing_push::{
    prepare_writing_push, send_prepared_writing_push, settle_writing_push, PreparedWritingPush,
    WritingPushAcceptedStatus, WritingPushOutcome, WritingPushPending, WritingPushPendingKind,
    WritingPushPreparation,
};
use crate::writing::sync_capture::{
    enqueue_document_at_for_test, outbox_entries, record_capability, supports_writing,
};
use crate::writing::sync_envelope::{
    AttachmentFileV1, AttachmentManifestV1, CitationProjectionsV1, DocumentSettingsV1,
    WritingEnvelopeV1,
};
use crate::writing::sync_transport::{build_push_changes, catchup_needed, record_catchup_done};

const DOCUMENT_ID: &str = "doc-push";
const SERVER_EPOCH: &str = "mock-epoch";
const ACCOUNT_ID: &str = "account-a";
const DEVICE_ID: &str = "device-a";
const SERVER_URL: &str = "https://sync.example.test";
const SESSION_INCARNATION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const CAPTURED_AT: i64 = 50_000;
const CLOCK_OFFSET: i64 = 25;

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

        let conn = Connection::open(&db_path).expect("temporary sqlite");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("WAL mode");
        configure_connection(&conn);
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("real generated application schema");
        crate::sync::schema::ensure_sync_schema(&conn).expect("sync schema");
        conn.execute_batch(
            "INSERT INTO _migrations(name, applied_at) VALUES('0035_writing_workspace', 1);
             INSERT INTO _migrations(name, applied_at) VALUES('0036_writing_journal', 2);",
        )
        .expect("migration head");
        for (key, value) in [
            ("account_id", ACCOUNT_ID),
            ("server_url", SERVER_URL),
            ("device_id", DEVICE_ID),
            ("server_epoch", SERVER_EPOCH),
            (SYNC_SESSION_INCARNATION_KEY, SESSION_INCARNATION),
            ("capture_enabled", "1"),
            ("clock_offset_ms", "25"),
        ] {
            set_meta(&conn, key, value);
        }
        drop(conn);

        Self {
            _temp: temp,
            db_path,
            data_root,
        }
    }

    fn connect(&self) -> Connection {
        let conn = Connection::open(&self.db_path).expect("reopen temporary sqlite");
        configure_connection(&conn);
        conn
    }

    fn enable_capability(&self) {
        let conn = self.connect();
        record_capability(&conn, SERVER_EPOCH, true).expect("record capability");
    }

    fn complete_catchup(&self) {
        let conn = self.connect();
        record_catchup_done(&conn, SERVER_EPOCH).expect("record catchup");
    }

    fn ready_gates(&self) {
        self.enable_capability();
        self.complete_catchup();
    }

    fn insert_document(&self, content: &Value) {
        let conn = self.connect();
        conn.execute(
            "INSERT INTO writing_documents
               (id, title, document_type, status, schema_version, current_content_json,
                revision, plain_text_cache, created_at, updated_at)
             VALUES (?1, 'Manuscrito', 'article', 'active', 1, ?2, 0, '', 1, 1)",
            rusqlite::params![DOCUMENT_ID, content.to_string()],
        )
        .expect("insert writing document");
        enqueue_document_at_for_test(&conn, DOCUMENT_ID, CAPTURED_AT)
            .expect("enqueue writing document");
    }

    fn enqueue_missing_document(&self) {
        let conn = self.connect();
        enqueue_document_at_for_test(&conn, DOCUMENT_ID, CAPTURED_AT)
            .expect("enqueue missing writing document");
    }
}

fn configure_connection(conn: &Connection) {
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .expect("configure sqlite");
}

fn set_meta(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .expect("set sync metadata");
}

fn health(max_push_bytes: i64) -> HealthResponse {
    HealthResponse {
        status: "ok".to_string(),
        version: "test".to_string(),
        epoch: SERVER_EPOCH.to_string(),
        server_now_ms: 1_700_000_000_000,
        limits: HealthLimits {
            max_push_bytes,
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

fn png_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(payload);
    bytes
}

fn attachment(payload: &[u8]) -> (Vec<u8>, AttachmentFileV1) {
    let bytes = png_bytes(payload);
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let entry = AttachmentFileV1 {
        rel_path: format!("writing-images/{sha256}.png"),
        sha256,
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };
    (bytes, entry)
}

fn write_attachment(data_root: &Path, entry: &AttachmentFileV1, bytes: &[u8]) {
    let target = data_root.join(&entry.rel_path);
    fs::create_dir_all(target.parent().expect("attachment parent")).expect("attachment directory");
    fs::write(target, bytes).expect("attachment bytes");
}

fn winner_envelope(document_id: &str, text: &str) -> WritingEnvelopeV1 {
    WritingEnvelopeV1 {
        id: document_id.to_string(),
        envelope_version: 1,
        content_json: text_content(text),
        title: "Remote winner".to_string(),
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
        attachments_manifest: AttachmentManifestV1::Validated { files: Vec::new() },
    }
}

fn winner_row(server_seq: i64) -> PullRow {
    let envelope = winner_envelope(DOCUMENT_ID, "Remote text");
    PullRow {
        table: "writing_envelopes".to_string(),
        row_id: DOCUMENT_ID.to_string(),
        server_seq,
        deleted: false,
        changed_at: 60_000,
        device_id: "device-remote".to_string(),
        payload: Some(
            serde_json::from_str(&envelope.to_canonical_json().expect("winner envelope"))
                .expect("winner value"),
        ),
    }
}

fn push_result(status: &str, server_seq: i64, winner: Option<PullRow>) -> PushResult {
    PushResult {
        table: "writing_envelopes".to_string(),
        row_id: DOCUMENT_ID.to_string(),
        status: status.to_string(),
        server_seq,
        winner,
    }
}

fn push_response(results: Vec<PushResult>) -> PushResponse {
    let max_server_seq = results
        .iter()
        .map(|result| result.server_seq)
        .max()
        .unwrap_or(0);
    PushResponse {
        results,
        max_server_seq,
        server_epoch: SERVER_EPOCH.to_string(),
        server_now_ms: 1_700_000_000_000,
        capabilities: vec![WRITING_ENVELOPE_V1_CAPABILITY.to_string()],
    }
}

fn ready(preparation: WritingPushPreparation) -> PreparedWritingPush {
    match preparation {
        WritingPushPreparation::Ready(prepared) => prepared,
        WritingPushPreparation::Idle => panic!("expected a prepared writing request, found idle"),
        WritingPushPreparation::Pending(pending) => {
            panic!("expected a prepared writing request: {pending:?}")
        }
    }
}

fn pending(preparation: WritingPushPreparation) -> WritingPushPending {
    match preparation {
        WritingPushPreparation::Pending(pending) => pending,
        WritingPushPreparation::Idle => panic!("expected pending writing work, found idle"),
        WritingPushPreparation::Ready(_) => panic!("expected pending writing work, found ready"),
    }
}

fn prepare(fixture: &Fixture, limits: &HealthResponse) -> PreparedWritingPush {
    let conn = fixture.connect();
    ready(prepare_writing_push(&conn, &fixture.data_root, limits))
}

fn outbox_ack(fixture: &Fixture) -> Option<crate::writing::sync_capture::OutboxAcknowledgment> {
    let conn = fixture.connect();
    outbox_entries(&conn)
        .expect("outbox")
        .into_iter()
        .find(|entry| entry.document_id == DOCUMENT_ID)
        .map(|entry| entry.acknowledgment)
}

fn recorded_seq(fixture: &Fixture) -> Option<i64> {
    let conn = fixture.connect();
    conn.query_row(
        "SELECT server_seq FROM sync_row_versions
          WHERE table_name = 'writing_envelopes' AND row_id = ?1",
        [DOCUMENT_ID],
        |row| row.get(0),
    )
    .optional()
    .expect("recorded writing seq")
}

fn stored_text(fixture: &Fixture) -> String {
    let conn = fixture.connect();
    let raw: String = conn
        .query_row(
            "SELECT current_content_json FROM writing_documents WHERE id = ?1",
            [DOCUMENT_ID],
            |row| row.get(0),
        )
        .expect("writing content");
    let value: Value = serde_json::from_str(&raw).expect("content json");
    value["doc"]["content"][0]["content"][0]["text"]
        .as_str()
        .expect("text")
        .to_string()
}

#[test]
fn missing_capability_keeps_outbox_and_performs_no_network_calls() {
    let fixture = Fixture::new();
    fixture.complete_catchup();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();

    let blocked = pending(prepare_writing_push(
        &fixture.connect(),
        &fixture.data_root,
        &health(1_000_000),
    ));

    assert_eq!(blocked.kind, WritingPushPendingKind::CapabilityUnavailable);
    assert!(outbox_ack(&fixture).is_some());
    assert!(api.sync_events.lock().expect("events").is_empty());
    assert_eq!(
        *api.writing_capability_push_calls
            .lock()
            .expect("push calls"),
        0
    );
}

#[test]
fn incomplete_catchup_keeps_outbox_and_is_not_marked_complete() {
    let fixture = Fixture::new();
    fixture.enable_capability();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();

    let blocked = pending(prepare_writing_push(
        &fixture.connect(),
        &fixture.data_root,
        &health(1_000_000),
    ));

    assert_eq!(blocked.kind, WritingPushPendingKind::CatchupIncomplete);
    assert!(catchup_needed(&fixture.connect(), SERVER_EPOCH).expect("catchup state"));
    assert!(outbox_ack(&fixture).is_some());
    assert!(api.sync_events.lock().expect("events").is_empty());
}

#[test]
fn stale_health_epoch_prevents_preparation_without_network() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let mut stale = health(1_000_000);
    stale.epoch = "old-epoch".to_string();
    let api = capable_api();

    let blocked = pending(prepare_writing_push(
        &fixture.connect(),
        &fixture.data_root,
        &stale,
    ));

    assert_eq!(blocked.kind, WritingPushPendingKind::EpochMismatch);
    assert!(outbox_ack(&fixture).is_some());
    assert!(api.sync_events.lock().expect("events").is_empty());
}

#[test]
fn missing_session_incarnation_fails_preparation_without_synthesizing_identity() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let conn = fixture.connect();
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1",
        [SYNC_SESSION_INCARNATION_KEY],
    )
    .expect("remove session incarnation");

    let blocked = pending(prepare_writing_push(
        &conn,
        &fixture.data_root,
        &health(1_000_000),
    ));

    assert_eq!(blocked.kind, WritingPushPendingKind::MissingSession);
    assert_eq!(
        read_session_incarnation(&conn).expect("read missing incarnation"),
        None
    );
    assert!(outbox_ack(&fixture).is_some());
}

#[test]
fn malformed_session_incarnation_fails_preparation_closed() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let conn = fixture.connect();
    set_meta(&conn, SYNC_SESSION_INCARNATION_KEY, "not-a-uuid");

    let blocked = pending(prepare_writing_push(
        &conn,
        &fixture.data_root,
        &health(1_000_000),
    ));

    assert_eq!(blocked.kind, WritingPushPendingKind::LocalState);
    assert!(outbox_ack(&fixture).is_some());
    assert_eq!(recorded_seq(&fixture), None);
}

#[tokio::test]
async fn blobs_are_head_put_before_the_explicit_writing_push() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    let (bytes, entry) = attachment(b"upload-order");
    write_attachment(&fixture.data_root, &entry, &bytes);
    fixture.insert_document(&text_with_image("Local text", &entry.rel_path));
    let prepared = prepare(&fixture, &health(1_000_000));
    let api = capable_api();

    send_prepared_writing_push(&api, "token", prepared)
        .await
        .expect("send writing push");

    assert_eq!(
        api.sync_events.lock().expect("events").as_slice(),
        &[
            MockSyncEvent::BlobHead(entry.sha256.clone()),
            MockSyncEvent::BlobPut(entry.sha256.clone(), bytes.len()),
            MockSyncEvent::WritingPush,
        ]
    );
    assert_eq!(
        *api.writing_capability_push_calls
            .lock()
            .expect("push calls"),
        1
    );
    assert_eq!(
        api.pushed.lock().expect("pushed")[0].changed_at,
        CAPTURED_AT + CLOCK_OFFSET
    );
}

#[tokio::test]
async fn current_incarnation_applied_ack_records_seq_and_deletes_captured_generation() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");

    let outcome = settle_writing_push(&fixture.connect(), completed).expect("settle response");

    assert_eq!(
        outcome,
        WritingPushOutcome::Acknowledged {
            document_id: DOCUMENT_ID.to_string(),
            server_seq: 1,
            status: WritingPushAcceptedStatus::Applied,
        }
    );
    assert_eq!(recorded_seq(&fixture), Some(1));
    assert!(outbox_ack(&fixture).is_none());
}

#[tokio::test]
async fn same_timestamp_new_generation_survives_the_old_response() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let captured = outbox_ack(&fixture).expect("captured generation");
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");
    let conn = fixture.connect();
    enqueue_document_at_for_test(&conn, DOCUMENT_ID, CAPTURED_AT)
        .expect("same-ms newer generation");
    let newer = outbox_entries(&conn).expect("outbox")[0]
        .acknowledgment
        .clone();
    assert_ne!(captured, newer);

    let outcome = settle_writing_push(&conn, completed).expect("settle old response");

    assert!(matches!(
        outcome,
        WritingPushOutcome::NewerGenerationPending { server_seq: 1, .. }
    ));
    assert_eq!(recorded_seq(&fixture), Some(1));
    assert_eq!(outbox_ack(&fixture), Some(newer));
}

#[tokio::test]
async fn settlement_sql_failure_rolls_back_seq_and_ack() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let captured = outbox_ack(&fixture).expect("captured generation");
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");
    let conn = fixture.connect();
    conn.execute_batch(
        "CREATE TRIGGER fail_writing_ack
         BEFORE DELETE ON sync_meta
         WHEN OLD.key = 'writing_outbox:doc-push'
         BEGIN
           SELECT RAISE(ABORT, 'forced writing ack failure');
         END;",
    )
    .expect("failure trigger");

    let error = settle_writing_push(&conn, completed).expect_err("settlement must fail");

    assert_eq!(error.kind, WritingPushPendingKind::LocalState);
    assert_eq!(recorded_seq(&fixture), None);
    assert_eq!(outbox_ack(&fixture), Some(captured));
}

#[tokio::test]
async fn account_change_after_network_completion_cannot_ack_old_work() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");
    let conn = fixture.connect();
    set_meta(&conn, "account_id", "account-b");

    let error = settle_writing_push(&conn, completed).expect_err("old account completion");

    assert_eq!(error.kind, WritingPushPendingKind::SessionChanged);
    assert_eq!(recorded_seq(&fixture), None);
    assert!(outbox_ack(&fixture).is_some());
}

#[tokio::test]
async fn server_change_after_network_completion_cannot_ack_old_work() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");
    let conn = fixture.connect();
    set_meta(&conn, "server_url", "https://replacement.example.test");

    let error = settle_writing_push(&conn, completed).expect_err("old server completion");

    assert_eq!(error.kind, WritingPushPendingKind::SessionChanged);
    assert_eq!(recorded_seq(&fixture), None);
    assert!(outbox_ack(&fixture).is_some());
}

#[tokio::test]
async fn device_change_after_network_completion_cannot_ack_old_work() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");
    let conn = fixture.connect();
    set_meta(&conn, "device_id", "replacement-device");

    let error = settle_writing_push(&conn, completed).expect_err("old device completion");

    assert_eq!(error.kind, WritingPushPendingKind::SessionChanged);
    assert_eq!(recorded_seq(&fixture), None);
    assert!(outbox_ack(&fixture).is_some());
}

#[tokio::test]
async fn same_identity_relogin_rejects_old_completion_before_local_mutation() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let original_incarnation = read_session_incarnation(&fixture.connect())
        .expect("read original incarnation")
        .expect("original incarnation");
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");

    let conn = fixture.connect();
    clear_sync_state(&conn).expect("same-identity logout");
    write_sync_session(
        &conn,
        SERVER_URL,
        ACCOUNT_ID,
        "reader@example.test",
        DEVICE_ID,
    )
    .expect("same-identity login");
    set_meta(&conn, "server_epoch", SERVER_EPOCH);
    let current_incarnation = read_session_incarnation(&conn)
        .expect("read current incarnation")
        .expect("current incarnation");
    assert_ne!(current_incarnation, original_incarnation);
    drop(conn);

    fixture.ready_gates();
    let conn = fixture.connect();
    enqueue_document_at_for_test(&conn, DOCUMENT_ID, CAPTURED_AT + 1)
        .expect("enqueue current-session generation");
    let current_generation = outbox_entries(&conn).expect("current outbox")[0]
        .acknowledgment
        .clone();
    assert!(supports_writing(&conn, SERVER_EPOCH).expect("restored capability"));
    assert!(!catchup_needed(&conn, SERVER_EPOCH).expect("restored catchup"));

    let error = settle_writing_push(&conn, completed).expect_err("old incarnation completion");

    assert_eq!(error.kind, WritingPushPendingKind::SessionChanged);
    assert_eq!(recorded_seq(&fixture), None);
    assert_eq!(outbox_ack(&fixture), Some(current_generation));
    assert_eq!(stored_text(&fixture), "Local text");
}

#[tokio::test]
async fn response_epoch_mismatch_cannot_produce_a_settlement_token() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let prepared = prepare(&fixture, &health(1_000_000));
    let mut api = capable_api();
    api.server_epoch = "replacement-epoch".to_string();

    let error = send_prepared_writing_push(&api, "token", prepared)
        .await
        .expect_err("mismatched response epoch");

    assert_eq!(error.kind, WritingPushPendingKind::EpochMismatch);
    assert_eq!(recorded_seq(&fixture), None);
    assert!(outbox_ack(&fixture).is_some());
}

#[tokio::test]
async fn epoch_change_after_network_completion_cannot_ack_old_work() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");
    let conn = fixture.connect();
    set_meta(&conn, "server_epoch", "replacement-epoch");

    let error = settle_writing_push(&conn, completed).expect_err("old epoch completion");

    assert_eq!(error.kind, WritingPushPendingKind::SessionChanged);
    assert_eq!(recorded_seq(&fixture), None);
    assert!(outbox_ack(&fixture).is_some());
}

#[test]
fn exact_serialized_request_size_includes_the_push_wrapper() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let generous = prepare(&fixture, &health(1_000_000));
    let exact_request_bytes = generous.serialized_request_bytes;
    let payload_bytes = build_push_changes(&fixture.connect(), &fixture.data_root)
        .expect("draft")
        .ready[0]
        .payload
        .to_string()
        .len();
    assert!(exact_request_bytes > payload_bytes);

    let blocked = pending(prepare_writing_push(
        &fixture.connect(),
        &fixture.data_root,
        &health((exact_request_bytes - 1) as i64),
    ));

    assert_eq!(blocked.kind, WritingPushPendingKind::RequestTooLarge);
    assert!(outbox_ack(&fixture).is_some());
}

#[tokio::test]
async fn blob_network_failure_keeps_pending_and_never_pushes() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    let (bytes, entry) = attachment(b"offline-upload");
    write_attachment(&fixture.data_root, &entry, &bytes);
    fixture.insert_document(&text_with_image("Local text", &entry.rel_path));
    let prepared = prepare(&fixture, &health(1_000_000));
    let api = capable_api();
    *api.blob_failure.lock().expect("blob failure") =
        Some(MockBlobFailure::Network("loopback unavailable".to_string()));

    let error = send_prepared_writing_push(&api, "token", prepared)
        .await
        .expect_err("blob upload must remain pending");

    assert_eq!(
        error.kind,
        WritingPushPendingKind::Blob(WritingBlobPendingKind::Network)
    );
    assert_eq!(
        api.sync_events.lock().expect("events").as_slice(),
        &[MockSyncEvent::BlobHead(entry.sha256)]
    );
    assert!(outbox_ack(&fixture).is_some());
}

#[tokio::test]
async fn push_auth_failure_keeps_the_exact_outbox_generation() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let captured = outbox_ack(&fixture).expect("captured generation");
    let api = capable_api();
    *api.writing_push_failure.lock().expect("push failure") = Some(MockBlobFailure::Api {
        status: 401,
        code: "unauthorized".to_string(),
        message: "expired token".to_string(),
    });

    let error = send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
        .await
        .expect_err("auth failure");

    assert_eq!(error.kind, WritingPushPendingKind::Unauthorized);
    assert_eq!(outbox_ack(&fixture), Some(captured));
    assert_eq!(recorded_seq(&fixture), None);
}

#[tokio::test]
async fn response_without_capability_never_acknowledges() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = MockSyncApi::default();

    let error = send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
        .await
        .expect_err("missing response capability");

    assert_eq!(error.kind, WritingPushPendingKind::CapabilityUnavailable);
    assert!(outbox_ack(&fixture).is_some());
    assert_eq!(recorded_seq(&fixture), None);
}

#[tokio::test]
async fn lww_lost_retains_outbox_and_surfaces_the_validated_winner() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let prepared = prepare(&fixture, &health(1_000_000));
    let api = capable_api();
    *api.writing_push_response.lock().expect("writing response") =
        Some(push_response(vec![push_result(
            "lww_lost",
            7,
            Some(winner_row(7)),
        )]));
    let completed = send_prepared_writing_push(&api, "token", prepared)
        .await
        .expect("validated conflict response");

    let outcome = settle_writing_push(&fixture.connect(), completed).expect("surface conflict");

    let WritingPushOutcome::ConflictPending {
        server_seq, winner, ..
    } = outcome
    else {
        panic!("expected conflict-pending outcome");
    };
    assert_eq!(server_seq, 7);
    assert_eq!(winner.server_seq, 7);
    assert_eq!(winner.device_id, "device-remote");
    assert_eq!(stored_text(&fixture), "Local text");
    assert!(outbox_ack(&fixture).is_some());
    assert_eq!(recorded_seq(&fixture), None);
}

#[tokio::test]
async fn lww_lost_without_a_winner_is_malformed_and_stays_pending() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();
    *api.writing_push_response.lock().expect("writing response") =
        Some(push_response(vec![push_result("lww_lost", 7, None)]));

    let error = send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
        .await
        .expect_err("winner is required");

    assert_eq!(error.kind, WritingPushPendingKind::MalformedResponse);
    assert!(outbox_ack(&fixture).is_some());
    assert_eq!(recorded_seq(&fixture), None);
}

#[tokio::test]
async fn lww_lost_with_a_malformed_winner_stays_pending() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let api = capable_api();
    let mut malformed = winner_row(7);
    malformed.payload = Some(json!({ "id": DOCUMENT_ID }));
    *api.writing_push_response.lock().expect("writing response") =
        Some(push_response(vec![push_result(
            "lww_lost",
            7,
            Some(malformed),
        )]));

    let error = send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
        .await
        .expect_err("malformed winner");

    assert_eq!(error.kind, WritingPushPendingKind::MalformedResponse);
    assert!(outbox_ack(&fixture).is_some());
    assert_eq!(recorded_seq(&fixture), None);
}

#[tokio::test]
async fn missing_duplicate_and_unexpected_results_never_ack() {
    for case in ["missing", "duplicate", "unexpected"] {
        let fixture = Fixture::new();
        fixture.ready_gates();
        fixture.insert_document(&text_content("Local text"));
        let api = capable_api();
        let results = match case {
            "missing" => Vec::new(),
            "duplicate" => vec![
                push_result("applied", 3, None),
                push_result("applied", 3, None),
            ],
            "unexpected" => vec![PushResult {
                table: "items".to_string(),
                row_id: "other-row".to_string(),
                status: "applied".to_string(),
                server_seq: 3,
                winner: None,
            }],
            _ => unreachable!(),
        };
        *api.writing_push_response.lock().expect("writing response") = Some(push_response(results));

        let error =
            send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
                .await
                .expect_err("invalid result set");

        assert_eq!(
            error.kind,
            WritingPushPendingKind::MalformedResponse,
            "case {case}"
        );
        assert!(outbox_ack(&fixture).is_some(), "case {case}");
        assert_eq!(recorded_seq(&fixture), None, "case {case}");
    }
}

#[tokio::test]
async fn stale_lower_lww_result_never_regresses_or_surfaces_old_winner() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let conn = fixture.connect();
    conn.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
         VALUES('writing_envelopes', ?1, 10)",
        [DOCUMENT_ID],
    )
    .expect("newer recorded seq");
    drop(conn);
    let api = capable_api();
    *api.writing_push_response.lock().expect("writing response") =
        Some(push_response(vec![push_result(
            "lww_lost",
            5,
            Some(winner_row(5)),
        )]));
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send stale response");

    let outcome = settle_writing_push(&fixture.connect(), completed).expect("settle stale");

    assert_eq!(
        outcome,
        WritingPushOutcome::StaleResponse {
            document_id: DOCUMENT_ID.to_string(),
            response_server_seq: 5,
            recorded_server_seq: 10,
        }
    );
    assert_eq!(recorded_seq(&fixture), Some(10));
    assert!(outbox_ack(&fixture).is_some());
    assert_eq!(stored_text(&fixture), "Local text");
}

#[tokio::test]
async fn settlement_savepoint_composes_with_and_rolls_back_in_outer_transaction() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.insert_document(&text_content("Local text"));
    let mut api = capable_api();
    api.result_status = "lww_won".to_string();
    let completed =
        send_prepared_writing_push(&api, "token", prepare(&fixture, &health(1_000_000)))
            .await
            .expect("send writing push");
    let mut conn = fixture.connect();
    let tx = conn.transaction().expect("outer transaction");

    let outcome = settle_writing_push(&tx, completed).expect("nested settlement");
    assert!(matches!(
        outcome,
        WritingPushOutcome::Acknowledged {
            status: WritingPushAcceptedStatus::LwwWon,
            ..
        }
    ));
    tx.rollback().expect("roll back caller transaction");

    assert_eq!(recorded_seq(&fixture), None);
    assert!(outbox_ack(&fixture).is_some());
}

#[test]
fn missing_document_remains_an_explicit_pending_outbox_entry() {
    let fixture = Fixture::new();
    fixture.ready_gates();
    fixture.enqueue_missing_document();

    let blocked = pending(prepare_writing_push(
        &fixture.connect(),
        &fixture.data_root,
        &health(1_000_000),
    ));

    assert_eq!(blocked.kind, WritingPushPendingKind::LocalFiles);
    assert_eq!(blocked.document_ids, vec![DOCUMENT_ID.to_string()]);
    assert!(outbox_ack(&fixture).is_some());
}
