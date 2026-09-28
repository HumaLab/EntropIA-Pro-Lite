use std::fs;
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::http::HealthLimits;
use super::session::{clear_sync_state, write_sync_session, SYNC_SESSION_INCARNATION_KEY};
use super::test_support::{MockBlobEvent, MockBlobFailure, MockSyncApi};
use super::writing_blobs::WritingBlobPendingKind;
use super::writing_receive::{
    download_prepared_writing_receive, enqueue_writing_receive, prepare_writing_receive,
    queued_writing_receives, settle_writing_receive,
};
use crate::sync::http::PullRow;
use crate::sync::schema::ensure_sync_schema;
use crate::sync::session::{meta_get, meta_set};
use crate::sync::test_support::new_synced_test_db;
use crate::writing::repository::{save_document, SaveDocument};
use crate::writing::sync_capture::outbox_entries;
use crate::writing::sync_envelope::{
    AttachmentFileV1, AttachmentManifestV1, CitationProjectionsV1, CollectionAssociationV1,
    DocumentSettingsV1, WritingEnvelopeV1,
};
use crate::writing::sync_transport::PullApplyOutcome;

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

fn row(seq: i64, deleted: bool) -> PullRow {
    PullRow {
        table: "writing_envelopes".to_string(),
        row_id: "doc-1".to_string(),
        server_seq: seq,
        deleted,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: (!deleted).then(|| json!({"id": "doc-1", "text": seq})),
    }
}

#[test]
fn queued_receive_survives_reopening_the_same_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("queue.sqlite");
    {
        let conn = Connection::open(&path).expect("create");
        ensure_sync_schema(&conn).expect("schema");
        session(&conn);
        enqueue_writing_receive(&conn, &row(7, false)).expect("enqueue");
    }

    let reopened = Connection::open(&path).expect("reopen");
    let queued = queued_writing_receives(&reopened).expect("read");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].document_id, "doc-1");
    assert_eq!(queued[0].server_seq, 7);
    fs::remove_file(&path).ok();
}

#[test]
fn newest_sequence_replaces_older_and_equal_payload_is_idempotent() {
    let conn = new_synced_test_db();
    session(&conn);
    enqueue_writing_receive(&conn, &row(3, false)).expect("first");
    enqueue_writing_receive(&conn, &row(3, false)).expect("equal replay");
    enqueue_writing_receive(&conn, &row(8, true)).expect("newer tombstone");
    enqueue_writing_receive(&conn, &row(4, false)).expect("older ignored");

    let queued = queued_writing_receives(&conn).expect("read");
    assert_eq!(queued.len(), 1);
    assert!(queued[0].deleted);
    assert_eq!(queued[0].server_seq, 8);
    assert!(queued[0].payload.is_none());
}

#[test]
fn equal_sequence_with_different_payload_is_rejected() {
    let conn = new_synced_test_db();
    session(&conn);
    enqueue_writing_receive(&conn, &row(5, false)).expect("first");
    let mut conflicting = row(5, false);
    conflicting.payload = Some(json!({"id": "doc-1", "text": "different"}));

    let error = enqueue_writing_receive(&conn, &conflicting).expect_err("conflict");
    assert_eq!(error.code, "writing_receive_payload_conflict");
    assert_eq!(
        queued_writing_receives(&conn).expect("read")[0].server_seq,
        5
    );
}

#[test]
fn enqueue_failure_rolls_back_without_partial_queue_state() {
    let conn = new_synced_test_db();
    session(&conn);
    conn.execute_batch(
        "CREATE TEMP TRIGGER fail_writing_receive
           BEFORE INSERT ON sync_meta
           WHEN NEW.key = 'writing_receive:doc-1'
           BEGIN SELECT RAISE(ABORT, 'injected receive failure'); END;",
    )
    .expect("trigger");

    let error = enqueue_writing_receive(&conn, &row(2, false)).expect_err("rollback");
    assert_eq!(error.code, "writing_receive_local_state");
    assert!(queued_writing_receives(&conn).expect("read").is_empty());
}

#[test]
fn malformed_queue_record_is_retained_and_reported() {
    let conn = new_synced_test_db();
    session(&conn);
    meta_set(&conn, "writing_receive:doc-bad", "{not-json").expect("corrupt");

    let error = queued_writing_receives(&conn).expect_err("corrupt");
    assert_eq!(error.code, "writing_receive_corrupt");
    assert_eq!(
        meta_get(&conn, "writing_receive:doc-bad")
            .expect("read")
            .as_deref(),
        Some("{not-json")
    );
}

#[test]
fn missing_session_incarnation_rejects_enqueue() {
    let conn = new_synced_test_db();
    session(&conn);
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1",
        [SYNC_SESSION_INCARNATION_KEY],
    )
    .expect("remove incarnation");

    let error = enqueue_writing_receive(&conn, &row(1, false)).expect_err("missing");
    assert_eq!(error.code, "writing_receive_missing_session");
}

#[test]
fn logout_clears_receive_prefix_but_retains_literal_lookalike() {
    let conn = new_synced_test_db();
    session(&conn);
    meta_set(&conn, "writing_receive:doc-1", "queued").expect("queue");
    meta_set(&conn, "writingXreceive:doc-1", "lookalike").expect("decoy");

    clear_sync_state(&conn).expect("logout");
    assert_eq!(
        meta_get(&conn, "writing_receive:doc-1").expect("read"),
        None
    );
    assert_eq!(
        meta_get(&conn, "writingXreceive:doc-1")
            .expect("read")
            .as_deref(),
        Some("lookalike")
    );
}

#[test]
fn scope_uses_current_session_incarnation() {
    let conn = new_synced_test_db();
    session(&conn);
    enqueue_writing_receive(&conn, &row(1, false)).expect("enqueue");
    let raw = meta_get(&conn, "writing_receive:doc-1")
        .expect("raw")
        .expect("stored");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let stored = parsed["scope"]["session_incarnation"]
        .as_str()
        .expect("uuid");
    assert!(Uuid::parse_str(stored).is_ok());
}

// ---------------------------------------------------------------------------
// Prepare / download / settle fixtures. Real schema, temp SQLite, mock API.
// ---------------------------------------------------------------------------

/// A real-schema database with the writing migration head recorded and a live
/// sync session, so receive settlement can apply rows for real.
fn receive_db() -> Connection {
    let conn = new_synced_test_db();
    conn.execute_batch(
        "INSERT INTO _migrations(name, applied_at) VALUES('0035_writing_workspace', 1);
         INSERT INTO _migrations(name, applied_at) VALUES('0036_writing_journal', 2);",
    )
    .expect("writing migration head");
    session(&conn);
    conn
}

fn limits() -> HealthLimits {
    HealthLimits {
        max_push_bytes: 8 * 1024 * 1024,
        max_blob_mb: 1,
    }
}

fn tiny_png() -> Vec<u8> {
    // A complete, valid 1x1 PNG rather than a signature-only media fixture.
    STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .expect("valid embedded PNG")
}

fn hash_of(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn image_entry(bytes: &[u8]) -> AttachmentFileV1 {
    let sha256 = hash_of(bytes);
    AttachmentFileV1 {
        rel_path: format!("writing-images/{sha256}.png"),
        sha256,
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    }
}

fn envelope(id: &str, text: &str, files: Vec<AttachmentFileV1>) -> WritingEnvelopeV1 {
    let mut content = vec![json!({
        "type": "paragraph",
        "content": [{ "type": "text", "text": text }]
    })];
    content.extend(files.iter().map(|entry| {
        json!({
            "type": "writingImage",
            "attrs": { "src": entry.rel_path }
        })
    }));
    WritingEnvelopeV1 {
        id: id.to_string(),
        envelope_version: 1,
        content_json: json!({
            "schemaVersion": 1,
            "doc": { "type": "doc", "content": content }
        }),
        title: "Manuscrito".to_string(),
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
    }
}

fn payload_of(envelope: &WritingEnvelopeV1) -> Value {
    serde_json::from_str(&envelope.to_canonical_json().expect("canonical envelope"))
        .expect("envelope value")
}

fn upsert(id: &str, seq: i64, envelope: &WritingEnvelopeV1) -> PullRow {
    PullRow {
        table: "writing_envelopes".to_string(),
        row_id: id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(payload_of(envelope)),
    }
}

fn tombstone(id: &str, seq: i64) -> PullRow {
    PullRow {
        table: "writing_envelopes".to_string(),
        row_id: id.to_string(),
        server_seq: seq,
        deleted: true,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: None,
    }
}

fn content_text(text: &str) -> String {
    json!({
        "schemaVersion": 1,
        "doc": {
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }]
        }
    })
    .to_string()
}

fn insert_document(conn: &Connection, id: &str, text: &str) {
    conn.execute(
        "INSERT INTO writing_documents
           (id, title, document_type, status, schema_version, current_content_json,
            revision, plain_text_cache, created_at, updated_at)
         VALUES (?1, 'Manuscrito', 'article', 'active', 1, ?2, 0, '', 1, 1)",
        rusqlite::params![id, content_text(text)],
    )
    .expect("insert document");
}

fn save(conn: &mut Connection, id: &str, text: &str) {
    save_document(
        conn,
        SaveDocument {
            document_id: id.to_string(),
            expected_revision: 0,
            content_json: content_text(text),
            schema_version: 1,
            plain_text_cache: None,
            citations: Vec::new(),
            zotero_citations: Vec::new(),
            provenance: Vec::new(),
        },
    )
    .expect("save document");
}

fn text_of(conn: &Connection, id: &str) -> Option<String> {
    let content: Option<String> = conn
        .query_row(
            "SELECT current_content_json FROM writing_documents WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .optional()
        .expect("document content");
    content.map(|content| {
        let parsed: Value = serde_json::from_str(&content).expect("content json");
        parsed["doc"]["content"][0]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    })
}

fn document_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM writing_documents", [], |row| {
        row.get(0)
    })
    .expect("document count")
}

fn recorded_server_seq(conn: &Connection, id: &str) -> Option<i64> {
    conn.query_row(
        "SELECT server_seq FROM sync_row_versions
          WHERE table_name = 'writing_envelopes' AND row_id = ?1",
        rusqlite::params![id],
        |row| row.get(0),
    )
    .optional()
    .expect("recorded server sequence")
}

fn journal_count(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM writing_journal WHERE document_id = ?1",
        rusqlite::params![id],
        |row| row.get(0),
    )
    .expect("journal count")
}

fn outbox_ids(conn: &Connection) -> Vec<String> {
    outbox_entries(conn)
        .expect("outbox")
        .into_iter()
        .map(|entry| entry.document_id)
        .collect()
}

#[tokio::test]
async fn valid_download_settle_installs_blobs_applies_and_removes_the_exact_row() {
    let conn = receive_db();
    let data_root = tempfile::tempdir().expect("data root");
    let png = tiny_png();
    let entry = image_entry(&png);
    let fixture = envelope("doc-image", "Texto remoto", vec![entry.clone()]);
    enqueue_writing_receive(&conn, &upsert("doc-image", 7, &fixture)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-image")
        .expect("prepare")
        .expect("queued row");

    let api = MockSyncApi::default();
    api.put_blob_bytes(&entry.sha256, png.clone());
    download_prepared_writing_receive(&api, "token", data_root.path(), &prepared, &limits())
        .await
        .expect("verified download");
    assert_eq!(
        *api.blob_events.lock().expect("events"),
        vec![MockBlobEvent::Get(entry.sha256.clone())],
        "upsert download goes through the verified blob installer"
    );
    assert_eq!(
        fs::read(data_root.path().join(&entry.rel_path)).expect("installed blob"),
        png,
        "the tiny PNG is installed content-addressed and hash-verified"
    );

    let outcome = settle_writing_receive(&conn, &prepared, data_root.path()).expect("settle");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_document_id: None
        }
    );
    assert_eq!(text_of(&conn, "doc-image").as_deref(), Some("Texto remoto"));
    assert_eq!(document_count(&conn), 1);
    assert_eq!(recorded_server_seq(&conn, "doc-image"), Some(7));
    assert!(
        queued_writing_receives(&conn).expect("queue").is_empty(),
        "the settled row is deleted exactly"
    );
}

#[tokio::test]
async fn tombstone_download_skips_http_and_dirty_conflict_settles_atomically() {
    let mut conn = receive_db();
    insert_document(&conn, "doc-dirty", "Inicial");
    save(&mut conn, "doc-dirty", "Texto local");
    enqueue_writing_receive(&conn, &tombstone("doc-dirty", 9)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-dirty")
        .expect("prepare")
        .expect("queued row");

    let api = MockSyncApi::default();
    let data_root = tempfile::tempdir().expect("data root");
    download_prepared_writing_receive(&api, "token", data_root.path(), &prepared, &limits())
        .await
        .expect("tombstone download");
    assert!(
        api.blob_events.lock().expect("events").is_empty(),
        "tombstones perform no HTTP"
    );

    let outcome = settle_writing_receive(&conn, &prepared, data_root.path()).expect("settle");
    let PullApplyOutcome::Applied {
        created: false,
        conflict_document_id: Some(conflict_id),
    } = outcome
    else {
        panic!("dirty tombstone must preserve a conflict copy, got {outcome:?}");
    };
    assert_eq!(
        text_of(&conn, "doc-dirty"),
        None,
        "the tombstoned source is deleted"
    );
    assert_eq!(
        text_of(&conn, &conflict_id).as_deref(),
        Some("Texto local"),
        "the copy carries the losing local text"
    );
    assert_eq!(document_count(&conn), 1, "only the preserved copy remains");
    assert_eq!(
        outbox_ids(&conn),
        vec![conflict_id],
        "the conflict copy replaces the deleted source in the outbox"
    );
    assert_eq!(
        recorded_server_seq(&conn, "doc-dirty"),
        Some(9),
        "copy, delete, outbox, and version landed together"
    );
    assert!(
        queued_writing_receives(&conn).expect("queue").is_empty(),
        "the queue row was deleted in the same settlement"
    );
}

#[tokio::test]
async fn download_404_and_network_failures_retain_the_queue() {
    let conn = receive_db();
    let data_root = tempfile::tempdir().expect("data root");
    let png = tiny_png();
    let entry = image_entry(&png);
    let fixture = envelope("doc-missing", "Texto", vec![entry.clone()]);
    enqueue_writing_receive(&conn, &upsert("doc-missing", 4, &fixture)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-missing")
        .expect("prepare")
        .expect("queued row");
    let before = meta_get(&conn, "writing_receive:doc-missing")
        .expect("raw")
        .expect("stored");

    let missing = MockSyncApi::default();
    let error = download_prepared_writing_receive(
        &missing,
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect_err("404 download");
    assert_eq!(error.kind, WritingBlobPendingKind::RemoteMissing);
    assert_eq!(
        *missing.blob_events.lock().expect("events"),
        vec![MockBlobEvent::Get(entry.sha256.clone())]
    );

    let network = MockSyncApi::default();
    *network.blob_failure.lock().expect("failure") =
        Some(MockBlobFailure::Network("connection reset".to_string()));
    let error = download_prepared_writing_receive(
        &network,
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect_err("network download");
    assert_eq!(error.kind, WritingBlobPendingKind::Network);

    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1, "download failures retain the queue");
    assert_eq!(queued[0].attempts, 0, "download never mutates the row");
    assert_eq!(
        queued[0].captured_value, before,
        "the exact stored value survives both failures"
    );
    assert_eq!(
        meta_get(&conn, "writing_receive:doc-missing")
            .expect("raw")
            .as_deref(),
        Some(before.as_str())
    );
    assert_eq!(recorded_server_seq(&conn, "doc-missing"), None);
}

#[tokio::test]
async fn settle_defers_missing_collections_with_a_bounded_error_and_no_version_ack() {
    let conn = receive_db();
    let data_root = tempfile::tempdir().expect("data root");
    let mut fixture = envelope("doc-collections", "Con colecciones", Vec::new());
    fixture.collection_associations = (0..60)
        .map(|index| CollectionAssociationV1 {
            collection_id: format!("missing-collection-{index:03}"),
            is_primary: index == 0,
        })
        .collect();

    enqueue_writing_receive(&conn, &upsert("doc-collections", 3, &fixture)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-collections")
        .expect("prepare")
        .expect("queued row");
    let api = MockSyncApi::default();
    download_prepared_writing_receive(&api, "token", data_root.path(), &prepared, &limits())
        .await
        .expect("nothing to download");
    assert!(api.blob_events.lock().expect("events").is_empty());

    let outcome =
        settle_writing_receive(&conn, &prepared, data_root.path()).expect("typed deferral");
    assert!(
        matches!(outcome, PullApplyOutcome::Deferred { .. }),
        "missing collections defer, got {outcome:?}"
    );

    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1, "deferral retains the queue row");
    assert_eq!(queued[0].attempts, 1);
    let stored_error = queued[0].last_error.as_deref().expect("bounded error");
    assert!(
        stored_error.contains("MissingCollections"),
        "{stored_error}"
    );
    assert_eq!(
        stored_error.chars().count(),
        512,
        "retained errors are bounded"
    );
    assert_eq!(document_count(&conn), 0, "nothing was applied");
    assert_eq!(
        recorded_server_seq(&conn, "doc-collections"),
        None,
        "no version ack"
    );
}

#[tokio::test]
async fn settle_defers_a_pending_journal_without_acknowledging_the_version() {
    let mut conn = receive_db();
    insert_document(&conn, "doc-journal", "Inicial");
    save(&mut conn, "doc-journal", "Texto local");
    conn.execute_batch(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
           VALUES('writing_envelopes', 'doc-journal', 5);
         INSERT INTO writing_journal
           (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
           VALUES('doc-journal', 1, 1, 1, '[{\"step\":\"local\"}]', 'checksum', 1);",
    )
    .expect("seed journal and version");

    enqueue_writing_receive(&conn, &tombstone("doc-journal", 9)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-journal")
        .expect("prepare")
        .expect("queued row");
    let data_root = tempfile::tempdir().expect("data root");
    download_prepared_writing_receive(
        &MockSyncApi::default(),
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect("tombstone download");
    let outcome = settle_writing_receive(&conn, &prepared, data_root.path()).expect("deferral");

    assert_eq!(
        outcome,
        PullApplyOutcome::Deferred {
            reason: "PendingJournal { entries: 1 }".to_string()
        },
        "the journal deferral reason is explicit"
    );
    assert_eq!(
        text_of(&conn, "doc-journal").as_deref(),
        Some("Texto local")
    );
    assert_eq!(document_count(&conn), 1, "no conflict copy created");
    assert_eq!(journal_count(&conn, "doc-journal"), 1);
    assert_eq!(
        recorded_server_seq(&conn, "doc-journal"),
        Some(5),
        "the deferral acknowledges no new version"
    );
    assert_eq!(outbox_ids(&conn), vec!["doc-journal".to_string()]);
    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1, "deferral retains the queue row");
    assert_eq!(queued[0].attempts, 1);
    assert_eq!(
        queued[0].last_error.as_deref(),
        Some("PendingJournal { entries: 1 }")
    );
}

#[tokio::test]
async fn newer_queued_row_survives_settlement_of_an_older_prepared_row() {
    let conn = receive_db();
    let data_root = tempfile::tempdir().expect("data root");
    let older = envelope("doc-race", "Versión vieja", Vec::new());
    let newer = envelope("doc-race", "Versión nueva", Vec::new());
    enqueue_writing_receive(&conn, &upsert("doc-race", 5, &older)).expect("enqueue older");
    let prepared = prepare_writing_receive(&conn, "doc-race")
        .expect("prepare")
        .expect("queued row");
    download_prepared_writing_receive(
        &MockSyncApi::default(),
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect("download");

    enqueue_writing_receive(&conn, &upsert("doc-race", 8, &newer)).expect("enqueue newer");
    let error = settle_writing_receive(&conn, &prepared, data_root.path())
        .expect_err("the exact stored value moved");
    assert_eq!(error.code, "writing_receive_queue_changed");

    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].server_seq, 8, "the newer queued row survives");
    assert_eq!(queued[0].payload, Some(payload_of(&newer)));
    assert_eq!(document_count(&conn), 0, "the prepared row never applied");
    assert_eq!(recorded_server_seq(&conn, "doc-race"), None);
}

#[tokio::test]
async fn settle_sql_failure_rolls_back_document_copy_outbox_version_and_queue() {
    // Scenario A: the winner-version write fails after the conflict copy and
    // winner document were written inside the settlement transaction.
    let mut conn = receive_db();
    insert_document(&conn, "doc-sql-version", "Inicial");
    save(&mut conn, "doc-sql-version", "Texto local");
    conn.execute_batch(
        "CREATE TEMP TRIGGER fail_receive_version
           BEFORE INSERT ON sync_row_versions
           WHEN NEW.table_name = 'writing_envelopes' AND NEW.row_id = 'doc-sql-version'
           BEGIN SELECT RAISE(ABORT, 'forced version failure'); END;",
    )
    .expect("install version failure trigger");
    let winner = envelope("doc-sql-version", "Texto remoto", Vec::new());
    enqueue_writing_receive(&conn, &upsert("doc-sql-version", 12, &winner)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-sql-version")
        .expect("prepare")
        .expect("queued row");
    let data_root = tempfile::tempdir().expect("data root");
    download_prepared_writing_receive(
        &MockSyncApi::default(),
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect("download");
    let error = settle_writing_receive(&conn, &prepared, data_root.path())
        .expect_err("forced version failure");
    assert!(
        error.message.contains("forced version failure"),
        "{error:?}"
    );
    assert_rolled_back(&conn, "doc-sql-version", "forced version failure");

    // Scenario B: the queue-row removal fails after apply succeeded.
    let mut conn = receive_db();
    insert_document(&conn, "doc-sql-queue", "Inicial");
    save(&mut conn, "doc-sql-queue", "Texto local");
    conn.execute_batch(
        "CREATE TEMP TRIGGER fail_receive_queue_delete
           BEFORE DELETE ON sync_meta
           WHEN OLD.key = 'writing_receive:doc-sql-queue'
           BEGIN SELECT RAISE(ABORT, 'forced queue delete failure'); END;",
    )
    .expect("install queue delete failure trigger");
    let winner = envelope("doc-sql-queue", "Texto remoto", Vec::new());
    enqueue_writing_receive(&conn, &upsert("doc-sql-queue", 12, &winner)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-sql-queue")
        .expect("prepare")
        .expect("queued row");
    download_prepared_writing_receive(
        &MockSyncApi::default(),
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect("download");
    let error = settle_writing_receive(&conn, &prepared, data_root.path())
        .expect_err("forced queue delete failure");
    assert!(
        error.message.contains("forced queue delete failure"),
        "{error:?}"
    );
    assert_rolled_back(&conn, "doc-sql-queue", "forced queue delete failure");
}

fn assert_rolled_back(conn: &Connection, id: &str, _failure: &str) {
    assert_eq!(
        text_of(conn, id).as_deref(),
        Some("Texto local"),
        "the winner and conflict copy rolled back"
    );
    assert_eq!(document_count(conn), 1, "no conflict copy survives");
    assert_eq!(
        outbox_ids(conn),
        vec![id.to_string()],
        "the outbox was restored"
    );
    assert_eq!(
        recorded_server_seq(conn, id),
        None,
        "the version write rolled back"
    );
    assert_eq!(
        meta_get(conn, &format!("writing_manifest:{id}")).expect("manifest"),
        None,
        "accepted manifest bookkeeping rolled back"
    );
    let queued = queued_writing_receives(conn).expect("queue");
    assert_eq!(queued.len(), 1, "the queue row is retained");
    assert_eq!(
        queued[0].captured_value,
        meta_get(conn, &format!("writing_receive:{id}"))
            .expect("raw")
            .expect("stored"),
        "the exact stored queue value is unchanged"
    );
    assert_eq!(queued[0].attempts, 0);
}

#[test]
fn settle_rejects_same_identity_relogin_and_retains_the_queue() {
    let conn = receive_db();
    let fixture = envelope("doc-relogin", "Texto", Vec::new());
    enqueue_writing_receive(&conn, &upsert("doc-relogin", 6, &fixture)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-relogin")
        .expect("prepare")
        .expect("queued row");

    // Same account, server, device, and epoch: only the incarnation rotates.
    session(&conn);

    let error = settle_writing_receive(&conn, &prepared, Path::new(""))
        .expect_err("relogin must not settle a stale-session row");
    assert_eq!(error.code, "writing_receive_scope_changed");
    let error = prepare_writing_receive(&conn, "doc-relogin")
        .expect_err("prepare rechecks the session too");
    assert_eq!(error.code, "writing_receive_scope_changed");

    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1, "the queue row survives the rejection");
    assert_eq!(queued[0].attempts, 0, "the queue row is untouched");
    assert_eq!(document_count(&conn), 0);
    assert_eq!(recorded_server_seq(&conn, "doc-relogin"), None);
}

#[test]
fn settle_rejects_server_epoch_change_and_retains_the_queue() {
    let conn = receive_db();
    let fixture = envelope("doc-epoch", "Texto", Vec::new());
    enqueue_writing_receive(&conn, &upsert("doc-epoch", 6, &fixture)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-epoch")
        .expect("prepare")
        .expect("queued row");

    meta_set(&conn, "server_epoch", "epoch-b").expect("epoch change");

    let error = settle_writing_receive(&conn, &prepared, Path::new(""))
        .expect_err("epoch rotation must not settle a stale-scope row");
    assert_eq!(error.code, "writing_receive_scope_changed");
    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1, "the queue row survives the rejection");
    assert_eq!(queued[0].attempts, 0);
    assert_eq!(recorded_server_seq(&conn, "doc-epoch"), None);
}

#[tokio::test]
async fn unsupported_rows_retain_a_bounded_error_without_version_ack() {
    // A settle-side Unsupported outcome is retained with a bounded error.
    let conn = receive_db();
    let mismatched = envelope("doc-other", "Identidad ajena", Vec::new());
    enqueue_writing_receive(&conn, &upsert("doc-unsupported", 2, &mismatched)).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-unsupported")
        .expect("prepare")
        .expect("queued row");
    let data_root = tempfile::tempdir().expect("data root");
    download_prepared_writing_receive(
        &MockSyncApi::default(),
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect("no-op download");
    let outcome = settle_writing_receive(&conn, &prepared, data_root.path())
        .expect("unsupported outcome is a retained result");
    assert!(
        matches!(outcome, PullApplyOutcome::Unsupported { .. }),
        "id mismatch is unsupported, got {outcome:?}"
    );

    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1, "unsupported rows are retained");
    assert_eq!(queued[0].attempts, 1);
    let stored_error = queued[0].last_error.as_deref().expect("bounded error");
    assert!(stored_error.contains("does not match"), "{stored_error}");
    assert!(stored_error.chars().count() <= 512, "errors stay bounded");
    assert_eq!(document_count(&conn), 0);
    assert_eq!(
        recorded_server_seq(&conn, "doc-unsupported"),
        None,
        "no version ack"
    );

    // A future envelope version never reaches download or settlement: it is
    // rejected at prepare and retained without any version ack.
    let mut future_row = upsert("doc-future", 2, &envelope("doc-future", "x", Vec::new()));
    future_row.payload = Some(json!({
        "id": "doc-future", "envelope_version": 2, "content_json": {}, "title": "x",
        "type": "article", "status": "active", "schema_version": 1,
        "settings": { "citation_style_id": null, "citation_locale": null, "bibliography_enabled": true },
        "collection_associations": [], "citation_projections": { "corpus": [], "zotero": [] },
        "attachments_manifest": { "state": "validated", "files": [] }
    }));
    enqueue_writing_receive(&conn, &future_row).expect("enqueue future");
    let error = prepare_writing_receive(&conn, "doc-future").expect_err("future envelope version");
    assert_eq!(error.code, "writing_receive_unsupported");
    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 2, "the unsupported row is retained");
    assert_eq!(
        queued
            .iter()
            .find(|entry| entry.document_id == "doc-future")
            .map(|entry| entry.attempts),
        Some(0)
    );
    assert_eq!(
        recorded_server_seq(&conn, "doc-future"),
        None,
        "no version ack"
    );
}

#[tokio::test]
async fn settle_deletes_the_exact_row_for_an_own_echo() {
    let conn = receive_db();
    insert_document(&conn, "doc-echo", "Texto propio");
    let fixture = envelope("doc-echo", "Texto propio", Vec::new());
    let mut echoed = upsert("doc-echo", 6, &fixture);
    echoed.device_id = "device-own".to_string();
    enqueue_writing_receive(&conn, &echoed).expect("enqueue");
    let prepared = prepare_writing_receive(&conn, "doc-echo")
        .expect("prepare")
        .expect("queued row");
    let data_root = tempfile::tempdir().expect("data root");
    download_prepared_writing_receive(
        &MockSyncApi::default(),
        "token",
        data_root.path(),
        &prepared,
        &limits(),
    )
    .await
    .expect("download");

    let outcome = settle_writing_receive(&conn, &prepared, data_root.path()).expect("settle");
    assert_eq!(outcome, PullApplyOutcome::OwnChangeObserved);
    assert!(
        queued_writing_receives(&conn).expect("queue").is_empty(),
        "every successful outcome deletes the exact row"
    );
    assert_eq!(recorded_server_seq(&conn, "doc-echo"), Some(6));
}
