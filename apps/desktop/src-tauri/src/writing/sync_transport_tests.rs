//! End-to-end-ish tests for the transport glue: outbox drafts on the push
//! side, guarded receives and conflict copies on the pull side. Everything
//! runs against real migrations plus the real sync schema.

use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::repository::{save_document, SaveDocument};
use super::sync_capture::{
    enqueue_document, outbox_entries, record_capability, seed_outbox, supports_writing,
};
use super::sync_envelope::{
    AttachmentFileV1, AttachmentManifestV1, CitationProjectionsV1, CollectionAssociationV1,
    DocumentSettingsV1, WritingEnvelopeV1,
};
use super::sync_transport::{
    acknowledge_push_draft, apply_lww_lost_winner, apply_pulled_row, build_push_changes,
    catchup_needed, pending_local_writes, record_catchup_done, LwwLostSettlement,
    PendingLocalWrites, PullApplyOutcome, PulledWritingRow,
};

const MIGRATION_0035: &str =
    include_str!("../../../../../packages/store/src/migrations/0035_writing_workspace.sql");
const MIGRATION_0036: &str =
    include_str!("../../../../../packages/store/src/migrations/0036_writing_journal.sql");

fn connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE collections (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               description TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE _migrations (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               name TEXT NOT NULL UNIQUE,
               applied_at INTEGER NOT NULL
             );",
        )
        .expect("minimal real migration prerequisites");
    connection
        .execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{MIGRATION_0035}\n{MIGRATION_0036}\n\
             INSERT INTO _migrations (name, applied_at) VALUES ('0035_writing_workspace', 0);\n\
             INSERT INTO _migrations (name, applied_at) VALUES ('0036_writing_journal', 0);\n\
             COMMIT;"
        ))
        .expect("apply real writing migrations");
    crate::sync::schema::ensure_sync_schema(&connection).expect("sync schema");
    connection
        .execute_batch("INSERT INTO sync_meta(key, value) VALUES ('capture_enabled', '1');")
        .expect("enable capture");
    connection
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

fn insert_document(connection: &Connection, id: &str, content: &str) {
    connection
        .execute(
            "INSERT INTO writing_documents
               (id, title, document_type, status, schema_version, current_content_json,
                revision, plain_text_cache, created_at, updated_at)
             VALUES (?1, 'Manuscrito', 'article', 'active', 1, ?2, 0, '', 1, 1)",
            rusqlite::params![id, content],
        )
        .expect("insert document");
}

fn save(connection: &mut Connection, id: &str, content: &str) {
    save_document(
        connection,
        SaveDocument {
            document_id: id.to_string(),
            expected_revision: 0,
            content_json: content.to_string(),
            schema_version: 1,
            plain_text_cache: None,
            citations: Vec::new(),
            zotero_citations: Vec::new(),
            provenance: Vec::new(),
        },
    )
    .expect("save document");
}

fn envelope_with(id: &str, text: &str, manifest: AttachmentManifestV1) -> WritingEnvelopeV1 {
    WritingEnvelopeV1 {
        id: id.to_string(),
        envelope_version: 1,
        content_json: json!({
            "schemaVersion": 1,
            "doc": {
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }]
            }
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
        attachments_manifest: manifest,
    }
}

fn envelope(id: &str, text: &str) -> WritingEnvelopeV1 {
    envelope_with(
        id,
        text,
        AttachmentManifestV1::Validated { files: Vec::new() },
    )
}

fn png_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(payload);
    bytes
}

fn hash_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn attachment_fixture(payload: &[u8]) -> (Vec<u8>, AttachmentFileV1) {
    let bytes = png_bytes(payload);
    let sha256 = hash_of(&bytes);
    let entry = AttachmentFileV1 {
        rel_path: format!("writing-images/{sha256}.png"),
        sha256,
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };
    (bytes, entry)
}

fn content_with_image(text: &str, rel_path: &str) -> Value {
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

fn envelope_with_image(id: &str, text: &str, entry: AttachmentFileV1) -> WritingEnvelopeV1 {
    let mut envelope = envelope_with(
        id,
        text,
        AttachmentManifestV1::Validated {
            files: vec![entry.clone()],
        },
    );
    envelope.content_json = content_with_image(text, &entry.rel_path);
    envelope
}

fn write_attachment(data_root: &Path, rel_path: &str, bytes: &[u8]) {
    let target = data_root.join(rel_path);
    fs::create_dir_all(target.parent().expect("attachment parent")).expect("create attachment dir");
    fs::write(target, bytes).expect("write attachment fixture");
}

fn row_from(
    envelope: &WritingEnvelopeV1,
    server_seq: i64,
    device: &str,
    changed_at: i64,
) -> PulledWritingRow {
    let payload: Value =
        serde_json::from_str(&envelope.to_canonical_json().expect("canonical")).expect("value");
    PulledWritingRow {
        document_id: envelope.id.clone(),
        server_seq,
        deleted: false,
        changed_at,
        device_id: device.to_string(),
        payload: Some(payload),
    }
}

fn tombstone(document_id: &str, server_seq: i64, device: &str) -> PulledWritingRow {
    PulledWritingRow {
        document_id: document_id.to_string(),
        server_seq,
        deleted: true,
        changed_at: 9_000,
        device_id: device.to_string(),
        payload: None,
    }
}

fn text_of(connection: &Connection, id: &str) -> String {
    let content: String = connection
        .query_row(
            "SELECT current_content_json FROM writing_documents WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .expect("document content");
    let parsed: Value = serde_json::from_str(&content).expect("content json");
    parsed["doc"]["content"][0]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn document_count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM writing_documents", [], |row| {
            row.get(0)
        })
        .expect("document count")
}

fn meta_value(connection: &Connection, key: &str) -> Option<String> {
    connection
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .optional()
        .expect("metadata value")
}

fn recorded_server_seq(connection: &Connection, document_id: &str) -> Option<i64> {
    connection
        .query_row(
            "SELECT server_seq FROM sync_row_versions
              WHERE table_name = 'writing_envelopes' AND row_id = ?1",
            rusqlite::params![document_id],
            |row| row.get(0),
        )
        .optional()
        .expect("recorded server sequence")
}

fn journal_count(connection: &Connection, document_id: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM writing_journal WHERE document_id = ?1",
            rusqlite::params![document_id],
            |row| row.get(0),
        )
        .expect("journal count")
}

fn assert_new_attachment_deferral_preserves_database(
    envelope: &WritingEnvelopeV1,
    data_root: &Path,
) {
    let connection = connection();
    let manifest_key = format!("writing_manifest:{}", envelope.id);
    let pending_key = format!("writing_pending_assets:{}", envelope.id);
    connection
        .execute(
            "INSERT INTO sync_meta(key, value) VALUES (?1, 'accepted-before'), (?2, 'pending-before')",
            rusqlite::params![manifest_key, pending_key],
        )
        .expect("seed prior attachment metadata");

    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &row_from(envelope, 5, "device-other", 5_000),
        data_root,
    )
    .expect("attachment verification deferral");

    assert!(matches!(outcome, PullApplyOutcome::Deferred { .. }));
    assert_eq!(document_count(&connection), 0, "no manuscript was created");
    assert!(
        outbox_entries(&connection)
            .expect("unchanged outbox")
            .is_empty(),
        "no outbox generation was created"
    );
    assert_eq!(recorded_server_seq(&connection, &envelope.id), None);
    assert_eq!(
        meta_value(&connection, &manifest_key).as_deref(),
        Some("accepted-before")
    );
    assert_eq!(
        meta_value(&connection, &pending_key).as_deref(),
        Some("pending-before")
    );
}

#[test]
fn push_drafts_expose_an_exact_idempotent_acknowledgment() {
    let mut connection = connection();
    insert_document(&connection, "doc-1", &content_text("Inicial"));
    save(&mut connection, "doc-1", &content_text("Local"));
    seed_outbox(&connection).expect("seed");

    let build = build_push_changes(&connection, Path::new("")).expect("build");
    assert!(build.pending_files.is_empty());
    assert_eq!(build.ready.len(), 1, "one draft per document");
    let draft = build.ready[0].clone();
    assert_eq!(draft.document_id, "doc-1");
    assert_eq!(draft.payload["id"], "doc-1");
    assert_eq!(draft.op, 'U');
    assert_eq!(draft.base_seq, 0, "never acknowledged before");
    assert!(draft.acknowledgment.generation.is_some());
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-1"),
        None,
        "building a draft is not proof that its files were uploaded"
    );

    // Simulate the server response bookkeeping for this exact pushed draft.
    connection
        .execute(
            "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
             VALUES ('writing_envelopes', 'doc-1', 42)",
            [],
        )
        .expect("record version");
    assert!(acknowledge_push_draft(&connection, &draft).expect("exact acknowledgment"));
    assert!(
        !acknowledge_push_draft(&connection, &draft).expect("duplicate acknowledgment"),
        "duplicate acknowledgment is a no-op"
    );

    let build = build_push_changes(&connection, Path::new("")).expect("build again");
    assert!(
        build.ready.is_empty(),
        "acknowledged documents leave the outbox"
    );
}

#[test]
fn old_stored_manifest_cannot_make_missing_local_files_pushable() {
    let mut connection = connection();
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (_, entry) = attachment_fixture(b"missing-outgoing");
    let content = content_with_image("Local", &entry.rel_path).to_string();
    let stored_manifest = AttachmentManifestV1::Validated { files: vec![entry] };
    insert_document(&connection, "doc-1", &content_text("Inicial"));
    save(&mut connection, "doc-1", &content);
    let stored_raw = serde_json::to_string(&stored_manifest).expect("stored manifest");
    connection
        .execute(
            "INSERT INTO sync_meta(key, value) VALUES ('writing_manifest:doc-1', ?1)",
            [&stored_raw],
        )
        .expect("seed old accepted manifest");

    let build = build_push_changes(&connection, data_root.path()).expect("build");

    assert!(build.ready.is_empty(), "a missing image blocks the draft");
    assert_eq!(build.pending_files, vec!["doc-1".to_string()]);
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-1").as_deref(),
        Some(stored_raw.as_str()),
        "the pending build does not rewrite accepted metadata"
    );
}

#[test]
fn peer_manifest_with_missing_local_file_defers_without_database_changes() {
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (_, entry) = attachment_fixture(b"missing-incoming");
    let winner = envelope_with_image("doc-remote", "Texto remoto", entry);

    assert_new_attachment_deferral_preserves_database(&winner, data_root.path());
}

#[test]
fn corrupt_incoming_file_defers_without_database_or_file_changes() {
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (_, entry) = attachment_fixture(b"expected-incoming");
    let corrupt = png_bytes(b"corrupt-incoming");
    write_attachment(data_root.path(), &entry.rel_path, &corrupt);
    let winner = envelope_with_image("doc-corrupt", "Texto remoto", entry.clone());

    assert_new_attachment_deferral_preserves_database(&winner, data_root.path());
    assert_eq!(
        fs::read(data_root.path().join(entry.rel_path)).expect("corrupt fixture remains"),
        corrupt,
        "verification is read-only"
    );
}

#[test]
fn incoming_manifest_size_mismatch_defers_without_database_changes() {
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (bytes, mut entry) = attachment_fixture(b"size-mismatch");
    write_attachment(data_root.path(), &entry.rel_path, &bytes);
    entry.size += 1;
    let winner = envelope_with_image("doc-size", "Texto remoto", entry);

    assert_new_attachment_deferral_preserves_database(&winner, data_root.path());
}

#[test]
fn valid_locally_installed_image_applies_and_records_accepted_manifest() {
    let connection = connection();
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (bytes, entry) = attachment_fixture(b"valid-incoming");
    write_attachment(data_root.path(), &entry.rel_path, &bytes);
    let winner = envelope_with_image("doc-image", "Texto remoto", entry);
    let accepted = serde_json::to_string(&winner.attachments_manifest).expect("accepted manifest");

    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&winner, 6, "device-other", 6_000),
        data_root.path(),
    )
    .expect("apply verified attachment");

    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_document_id: None
        }
    );
    assert_eq!(text_of(&connection, "doc-image"), "Texto remoto");
    assert_eq!(recorded_server_seq(&connection, "doc-image"), Some(6));
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-image").as_deref(),
        Some(accepted.as_str())
    );
    assert_eq!(
        meta_value(&connection, "writing_pending_assets:doc-image"),
        None
    );
}

#[test]
fn locally_installed_but_unmanifested_image_is_blocked() {
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (bytes, entry) = attachment_fixture(b"unmanifested");
    write_attachment(data_root.path(), &entry.rel_path, &bytes);
    let mut winner = envelope("doc-unmanifested", "Texto remoto");
    winner.content_json = content_with_image("Texto remoto", &entry.rel_path);

    assert_new_attachment_deferral_preserves_database(&winner, data_root.path());
}

#[test]
fn applying_a_new_document_creates_it_and_records_its_version() {
    let connection = connection();
    let winner = envelope("doc-1", "Texto remoto");
    let row = row_from(&winner, 7, "device-other", 5_000);

    let outcome = apply_pulled_row(&connection, "device-own", &row, Path::new("")).expect("apply");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_document_id: None
        }
    );
    assert_eq!(text_of(&connection, "doc-1"), "Texto remoto");

    let again = apply_pulled_row(&connection, "device-own", &row, Path::new("")).expect("apply");
    assert_eq!(
        again,
        PullApplyOutcome::Stale,
        "version recorded; retry is a no-op"
    );
}

#[test]
fn own_echo_timestamps_never_acknowledge_a_current_or_newer_draft() {
    let mut connection = connection();
    insert_document(&connection, "doc-1", &content_text("Inicial"));
    save(&mut connection, "doc-1", &content_text("Local"));
    let first = build_push_changes(&connection, Path::new(""))
        .expect("first build")
        .ready
        .remove(0);

    enqueue_document(&connection, "doc-1").expect("newer capture");
    let newer = build_push_changes(&connection, Path::new(""))
        .expect("newer build")
        .ready
        .remove(0);
    assert_ne!(first.acknowledgment, newer.acknowledgment);

    let mine = envelope("doc-1", "Texto local");
    let future_echo = row_from(&mine, 11, "device-own", i64::MAX);
    let outcome =
        apply_pulled_row(&connection, "device-own", &future_echo, Path::new("")).expect("apply");
    assert_eq!(outcome, PullApplyOutcome::OwnChangeObserved);

    let clamped_echo = row_from(&mine, 12, "device-own", newer.changed_at);
    let outcome =
        apply_pulled_row(&connection, "device-own", &clamped_echo, Path::new("")).expect("apply");
    assert_eq!(outcome, PullApplyOutcome::OwnChangeObserved);
    assert_eq!(
        outbox_entries(&connection).expect("outbox")[0].acknowledgment,
        newer.acknowledgment,
        "neither a future nor clock-clamped own echo proves the pushed generation"
    );
    assert!(
        !acknowledge_push_draft(&connection, &first).expect("stale draft acknowledgment"),
        "the old exact acknowledgment also retains the newer draft"
    );
}

#[test]
fn own_device_row_restores_a_missing_local_document() {
    let connection = connection();
    let mine = envelope("doc-restored", "Texto restaurado");
    let row = row_from(&mine, 4, "device-own", 4_000);

    let outcome =
        apply_pulled_row(&connection, "device-own", &row, Path::new("")).expect("restore");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_document_id: None
        }
    );
    assert_eq!(text_of(&connection, "doc-restored"), "Texto restaurado");
}

#[test]
fn a_clean_local_document_follows_the_remote_winner_without_a_copy() {
    let connection = connection();
    let first = envelope("doc-1", "Primera versión");
    apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&first, 3, "device-other", 1_000),
        Path::new(""),
    )
    .expect("first apply");

    let second = envelope("doc-1", "Segunda versión");
    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&second, 4, "device-other", 2_000),
        Path::new(""),
    )
    .expect("second apply");
    assert!(matches!(
        outcome,
        PullApplyOutcome::Applied { created: false, .. }
    ));
    assert_eq!(text_of(&connection, "doc-1"), "Segunda versión");
    assert_eq!(
        document_count(&connection),
        1,
        "no conflict copy for clean updates"
    );
}

#[test]
fn divergent_local_edits_survive_as_one_visible_conflict_copy() {
    let mut connection = connection();
    insert_document(&connection, "doc-1", &content_text("Texto local"));
    save(&mut connection, "doc-1", &content_text("Texto local"));

    let winner = envelope("doc-1", "Texto remoto");
    let row = row_from(&winner, 21, "device-other", 9_000);
    let outcome = apply_pulled_row(&connection, "device-own", &row, Path::new("")).expect("apply");
    let PullApplyOutcome::Applied {
        conflict_document_id: Some(conflict_id),
        ..
    } = outcome
    else {
        panic!("expected a preserved conflict copy, got {outcome:?}");
    };

    assert_eq!(
        text_of(&connection, "doc-1"),
        "Texto remoto",
        "winner lands"
    );
    assert_eq!(document_count(&connection), 2, "both manuscripts exist");
    assert_eq!(
        text_of(&connection, &conflict_id),
        "Texto local",
        "the copy carries the losing text"
    );

    // The copy is new work that must sync back.
    let build = build_push_changes(&connection, Path::new("")).expect("build");
    assert!(
        build
            .ready
            .iter()
            .any(|draft| draft.document_id == conflict_id),
        "the preserved copy joins the outbox"
    );
}

#[test]
fn old_stored_manifest_cannot_authorize_overwriting_an_unproven_local_loser() {
    let mut connection = connection();
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (_, entry) = attachment_fixture(b"missing-local-loser");
    let local_content = content_with_image("Texto local", &entry.rel_path).to_string();
    let stored_manifest = AttachmentManifestV1::Validated { files: vec![entry] };
    let stored_raw = serde_json::to_string(&stored_manifest).expect("stored manifest");
    insert_document(&connection, "doc-unproven", &content_text("Inicial"));
    save(&mut connection, "doc-unproven", &local_content);
    connection
        .execute(
            "INSERT INTO sync_meta(key, value) VALUES ('writing_manifest:doc-unproven', ?1)",
            [&stored_raw],
        )
        .expect("seed old accepted manifest");
    let winner = envelope("doc-unproven", "Texto remoto");

    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&winner, 22, "device-other", 9_000),
        data_root.path(),
    )
    .expect("safe deferral");

    assert!(matches!(outcome, PullApplyOutcome::Deferred { .. }));
    assert_eq!(text_of(&connection, "doc-unproven"), "Texto local");
    assert_eq!(document_count(&connection), 1, "no conflict copy was made");
    assert_eq!(
        outbox_entries(&connection)
            .expect("outbox retained")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec!["doc-unproven".to_string()]
    );
    assert_eq!(recorded_server_seq(&connection, "doc-unproven"), None);
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-unproven").as_deref(),
        Some(stored_raw.as_str())
    );
}

#[test]
fn dirty_tombstone_does_not_delete_an_unproven_local_loser() {
    let mut connection = connection();
    let data_root = tempfile::tempdir().expect("temporary data root");
    let (_, entry) = attachment_fixture(b"missing-tombstone-loser");
    let local_content = content_with_image("Texto local", &entry.rel_path).to_string();
    let stored_manifest = AttachmentManifestV1::Validated { files: vec![entry] };
    let stored_raw = serde_json::to_string(&stored_manifest).expect("stored manifest");
    insert_document(&connection, "doc-unproven-delete", &content_text("Inicial"));
    save(&mut connection, "doc-unproven-delete", &local_content);
    connection
        .execute(
            "INSERT INTO sync_meta(key, value) VALUES ('writing_manifest:doc-unproven-delete', ?1)",
            [&stored_raw],
        )
        .expect("seed old accepted manifest");

    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &tombstone("doc-unproven-delete", 23, "device-other"),
        data_root.path(),
    )
    .expect("safe tombstone deferral");

    assert!(matches!(outcome, PullApplyOutcome::Deferred { .. }));
    assert_eq!(text_of(&connection, "doc-unproven-delete"), "Texto local");
    assert_eq!(document_count(&connection), 1, "no conflict copy was made");
    assert_eq!(
        outbox_entries(&connection)
            .expect("outbox retained")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec!["doc-unproven-delete".to_string()]
    );
    assert_eq!(
        recorded_server_seq(&connection, "doc-unproven-delete"),
        None
    );
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-unproven-delete").as_deref(),
        Some(stored_raw.as_str())
    );
}

#[test]
fn receive_bookkeeping_failure_rolls_back_winner_copy_and_outbox_changes() {
    let mut connection = connection();
    insert_document(&connection, "doc-atomic", &content_text("Inicial"));
    save(&mut connection, "doc-atomic", &content_text("Texto local"));
    connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_receive_version
             BEFORE INSERT ON sync_row_versions
             WHEN NEW.table_name = 'writing_envelopes' AND NEW.row_id = 'doc-atomic'
             BEGIN
               SELECT RAISE(ABORT, 'forced receive version failure');
             END;",
        )
        .expect("install failure trigger");

    let winner = envelope("doc-atomic", "Texto remoto");
    let error = apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&winner, 12, "device-other", 9_000),
        Path::new(""),
    )
    .expect_err("forced version write failure");

    assert_eq!(error.code, "sql_error");
    assert!(error.message.contains("forced receive version failure"));
    assert_eq!(text_of(&connection, "doc-atomic"), "Texto local");
    assert_eq!(document_count(&connection), 1, "conflict copy rolled back");
    assert_eq!(
        outbox_entries(&connection)
            .expect("outbox restored")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec!["doc-atomic".to_string()]
    );
    assert_eq!(recorded_server_seq(&connection, "doc-atomic"), None);
}

#[test]
fn accepted_metadata_failure_rolls_back_document_version_and_outbox_bookkeeping() {
    let mut connection = connection();
    insert_document(&connection, "doc-meta-fail", &content_text("Inicial"));
    save(
        &mut connection,
        "doc-meta-fail",
        &content_text("Texto local"),
    );
    connection
        .execute_batch(
            "INSERT INTO sync_meta(key, value)
               VALUES ('writing_pending_assets:doc-meta-fail', 'pending-before');
             CREATE TEMP TRIGGER fail_accepted_metadata
             BEFORE DELETE ON sync_meta
             WHEN OLD.key = 'writing_pending_assets:doc-meta-fail'
             BEGIN
               SELECT RAISE(ABORT, 'forced accepted metadata failure');
             END;",
        )
        .expect("install accepted metadata failure trigger");
    let winner = envelope("doc-meta-fail", "Texto remoto");

    let error = apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&winner, 13, "device-other", 9_000),
        Path::new(""),
    )
    .expect_err("accepted metadata cleanup must propagate");

    assert_eq!(error.code, "sql_error");
    assert!(error.message.contains("forced accepted metadata failure"));
    assert_eq!(text_of(&connection, "doc-meta-fail"), "Texto local");
    assert_eq!(document_count(&connection), 1, "conflict copy rolled back");
    assert_eq!(
        outbox_entries(&connection)
            .expect("outbox restored")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec!["doc-meta-fail".to_string()]
    );
    assert_eq!(recorded_server_seq(&connection, "doc-meta-fail"), None);
    assert_eq!(
        meta_value(&connection, "writing_pending_assets:doc-meta-fail").as_deref(),
        Some("pending-before")
    );
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-meta-fail"),
        None,
        "accepted manifest insert rolled back"
    );
}

#[test]
fn unsupported_envelopes_and_missing_collections_leave_nothing_behind() {
    let connection = connection();
    let base = envelope("doc-x", "Futuro");
    let mut row = row_from(&base, 2, "device-other", 1_000);
    row.payload = Some(json!({
        "id": "doc-x", "envelope_version": 2, "content_json": {}, "title": "x",
        "type": "article", "status": "active", "schema_version": 1,
        "settings": { "citation_style_id": null, "citation_locale": null, "bibliography_enabled": true },
        "collection_associations": [], "citation_projections": { "corpus": [], "zotero": [] },
        "attachments_manifest": { "state": "validated", "files": [] }
    }));
    let outcome = apply_pulled_row(&connection, "device-own", &row, Path::new("")).expect("apply");
    assert!(matches!(outcome, PullApplyOutcome::Unsupported { .. }));
    assert_eq!(document_count(&connection), 0);

    connection
        .execute_batch(
            "INSERT INTO sync_meta(key, value) VALUES
               ('writing_manifest:doc-y', 'accepted-before'),
               ('writing_pending_assets:doc-y', 'pending-before');",
        )
        .expect("seed accepted attachment metadata");
    let mut needs_collection = envelope("doc-y", "Con colección");
    needs_collection.collection_associations = vec![CollectionAssociationV1 {
        collection_id: "nope".to_string(),
        is_primary: true,
    }];
    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&needs_collection, 3, "device-other", 1_500),
        Path::new(""),
    )
    .expect("apply");
    assert!(
        matches!(outcome, PullApplyOutcome::Deferred { .. }),
        "typed deferral"
    );
    assert_eq!(document_count(&connection), 0, "no partial state");
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-y").as_deref(),
        Some("accepted-before"),
        "a deferred aggregate cannot replace the accepted manifest"
    );
    assert_eq!(
        meta_value(&connection, "writing_pending_assets:doc-y").as_deref(),
        Some("pending-before")
    );
    assert_eq!(recorded_server_seq(&connection, "doc-y"), None);
}

#[test]
fn clean_tombstone_applies_standalone() {
    let connection = connection();
    let clean = envelope("doc-standalone", "Limpio");
    apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&clean, 2, "device-other", 1_000),
        Path::new(""),
    )
    .expect("apply clean");

    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &tombstone("doc-standalone", 3, "device-other"),
        Path::new(""),
    )
    .expect("standalone tombstone");

    assert!(matches!(outcome, PullApplyOutcome::Applied { .. }));
    assert_eq!(document_count(&connection), 0);
    assert_eq!(recorded_server_seq(&connection, "doc-standalone"), Some(3));
}

#[test]
fn clean_tombstone_applies_inside_a_caller_transaction() {
    let mut connection = connection();
    let clean = envelope("doc-clean", "Limpio");
    apply_pulled_row(
        &connection,
        "device-own",
        &row_from(&clean, 2, "device-other", 1_000),
        Path::new(""),
    )
    .expect("apply clean");

    let transaction = connection.transaction().expect("caller transaction");
    let outcome = apply_pulled_row(
        &transaction,
        "device-own",
        &tombstone("doc-clean", 3, "device-other"),
        Path::new(""),
    )
    .expect("tombstone clean");
    assert!(matches!(outcome, PullApplyOutcome::Applied { .. }));
    assert_eq!(
        document_count(&transaction),
        0,
        "delete is visible inside the caller transaction"
    );
    transaction.commit().expect("caller commit");

    assert_eq!(document_count(&connection), 0, "clean tombstone deletes");
    assert_eq!(recorded_server_seq(&connection, "doc-clean"), Some(3));
}

#[test]
fn dirty_tombstone_preserves_a_copy_inside_a_caller_transaction() {
    let mut connection = connection();
    insert_document(&connection, "doc-dirty", &content_text("Inicial"));
    save(&mut connection, "doc-dirty", &content_text("Texto local"));

    let transaction = connection.transaction().expect("caller transaction");
    let outcome = apply_pulled_row(
        &transaction,
        "device-own",
        &tombstone("doc-dirty", 4, "device-other"),
        Path::new(""),
    )
    .expect("tombstone dirty");
    let PullApplyOutcome::Applied {
        conflict_document_id: Some(conflict_id),
        ..
    } = outcome
    else {
        panic!("dirty tombstone must preserve the local text, got {outcome:?}");
    };
    assert_eq!(text_of(&transaction, &conflict_id), "Texto local");
    assert_eq!(
        document_count(&transaction),
        1,
        "only the preserved copy remains"
    );
    assert_eq!(
        outbox_entries(&transaction)
            .expect("transaction outbox")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec![conflict_id.clone()],
        "the conflict copy replaces the deleted source in the outbox"
    );
    transaction.commit().expect("caller commit");

    assert_eq!(text_of(&connection, &conflict_id), "Texto local");
    assert_eq!(recorded_server_seq(&connection, "doc-dirty"), Some(4));
}

#[test]
fn caller_rollback_restores_dirty_tombstone_and_removes_its_conflict_copy() {
    let mut connection = connection();
    insert_document(&connection, "doc-rollback", &content_text("Inicial"));
    save(
        &mut connection,
        "doc-rollback",
        &content_text("Texto local"),
    );
    connection
        .execute_batch(
            "INSERT INTO sync_meta(key, value) VALUES
               ('writing_manifest:doc-rollback', 'manifest-before'),
               ('writing_pending_assets:doc-rollback', 'pending-before');",
        )
        .expect("seed writing metadata");

    let transaction = connection.transaction().expect("caller transaction");
    let outcome = apply_pulled_row(
        &transaction,
        "device-own",
        &tombstone("doc-rollback", 8, "device-other"),
        Path::new(""),
    )
    .expect("tombstone inside caller transaction");
    let PullApplyOutcome::Applied {
        conflict_document_id: Some(conflict_id),
        ..
    } = outcome
    else {
        panic!("dirty tombstone must preserve a copy, got {outcome:?}");
    };
    assert_eq!(text_of(&transaction, &conflict_id), "Texto local");
    transaction.rollback().expect("caller rollback");

    assert_eq!(text_of(&connection, "doc-rollback"), "Texto local");
    assert_eq!(document_count(&connection), 1, "conflict copy rolled back");
    assert_eq!(
        outbox_entries(&connection)
            .expect("restored outbox")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec!["doc-rollback".to_string()]
    );
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-rollback").as_deref(),
        Some("manifest-before")
    );
    assert_eq!(
        meta_value(&connection, "writing_pending_assets:doc-rollback").as_deref(),
        Some("pending-before")
    );
    assert_eq!(recorded_server_seq(&connection, "doc-rollback"), None);
}

#[test]
fn pending_journal_defers_tombstone_without_changing_document_or_sync_metadata() {
    let mut connection = connection();
    insert_document(&connection, "doc-journal", &content_text("Inicial"));
    save(&mut connection, "doc-journal", &content_text("Texto local"));
    connection
        .execute_batch(
            "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
               VALUES('writing_envelopes', 'doc-journal', 5);
             INSERT INTO sync_meta(key, value) VALUES
               ('writing_manifest:doc-journal', 'manifest-before'),
               ('writing_pending_assets:doc-journal', 'pending-before');
             INSERT INTO writing_journal
               (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
               VALUES('doc-journal', 1, 1, 1, '[{\"step\":\"local\"}]', 'checksum', 1);",
        )
        .expect("seed pending journal");

    let outcome = apply_pulled_row(
        &connection,
        "device-own",
        &tombstone("doc-journal", 9, "device-other"),
        Path::new(""),
    )
    .expect("journal deferral");

    assert_eq!(
        outcome,
        PullApplyOutcome::Deferred {
            reason: "PendingJournal { entries: 1 }".to_string()
        }
    );
    assert_eq!(text_of(&connection, "doc-journal"), "Texto local");
    assert_eq!(document_count(&connection), 1, "no conflict copy created");
    assert_eq!(journal_count(&connection, "doc-journal"), 1);
    assert_eq!(recorded_server_seq(&connection, "doc-journal"), Some(5));
    assert_eq!(
        outbox_entries(&connection)
            .expect("outbox retained")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec!["doc-journal".to_string()]
    );
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-journal").as_deref(),
        Some("manifest-before")
    );
    assert_eq!(
        meta_value(&connection, "writing_pending_assets:doc-journal").as_deref(),
        Some("pending-before")
    );
}

#[test]
fn tombstone_sql_failure_rolls_back_copy_delete_metadata_and_version() {
    let mut connection = connection();
    insert_document(&connection, "doc-fail", &content_text("Inicial"));
    save(&mut connection, "doc-fail", &content_text("Texto local"));
    connection
        .execute_batch(
            "INSERT INTO sync_meta(key, value) VALUES
               ('writing_manifest:doc-fail', 'manifest-before'),
               ('writing_pending_assets:doc-fail', 'pending-before');
             CREATE TEMP TRIGGER fail_tombstone_version
             BEFORE INSERT ON sync_row_versions
             WHEN NEW.table_name = 'writing_envelopes' AND NEW.row_id = 'doc-fail'
             BEGIN
               SELECT RAISE(ABORT, 'forced tombstone version failure');
             END;",
        )
        .expect("install failure trigger");

    let error = apply_pulled_row(
        &connection,
        "device-own",
        &tombstone("doc-fail", 12, "device-other"),
        Path::new(""),
    )
    .expect_err("forced version write failure");

    assert_eq!(error.code, "sql_error");
    assert!(error.message.contains("forced tombstone version failure"));
    assert_eq!(text_of(&connection, "doc-fail"), "Texto local");
    assert_eq!(document_count(&connection), 1, "conflict copy rolled back");
    assert_eq!(
        outbox_entries(&connection)
            .expect("outbox restored")
            .into_iter()
            .map(|entry| entry.document_id)
            .collect::<Vec<_>>(),
        vec!["doc-fail".to_string()]
    );
    assert_eq!(
        meta_value(&connection, "writing_manifest:doc-fail").as_deref(),
        Some("manifest-before")
    );
    assert_eq!(
        meta_value(&connection, "writing_pending_assets:doc-fail").as_deref(),
        Some("pending-before")
    );
    assert_eq!(recorded_server_seq(&connection, "doc-fail"), None);
}

#[test]
fn catchup_and_capability_state_gate_activation_per_epoch() {
    let connection = connection();
    assert!(catchup_needed(&connection, "epoch-a").expect("read"));
    record_catchup_done(&connection, "epoch-a").expect("record");
    assert!(!catchup_needed(&connection, "epoch-a").expect("read"));
    assert!(
        catchup_needed(&connection, "epoch-b").expect("read"),
        "new epoch catches up again"
    );

    record_capability(&connection, "epoch-a", true).expect("advertised");
    assert!(supports_writing(&connection, "epoch-a").expect("read"));
    assert!(!supports_writing(&connection, "epoch-b").expect("read"));
}

#[test]
fn seeding_then_building_never_loses_a_pending_document() {
    let mut connection = connection();
    insert_document(&connection, "doc-1", &content_text("Inicial"));
    save(&mut connection, "doc-1", &content_text("Local"));
    enqueue_document(&connection, "doc-2").expect("explicit enqueue of an unknown doc");

    let build = build_push_changes(&connection, Path::new("")).expect("build");
    assert_eq!(
        build.pending_files,
        vec!["doc-2".to_string()],
        "an outbox entry without a document is reported, never silently dropped"
    );
    assert_eq!(build.ready.len(), 1);
}

// ─────────────── W-GUARD2: automatic-apply dirty barrier ───────────────

fn seed_journal_delta(connection: &Connection, document_id: &str, base_revision: i64) {
    connection
        .execute(
            "INSERT INTO writing_journal
               (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
             VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM writing_journal
                           WHERE document_id = ?1),
                     ?2, 1, '[{\"step\":\"typing\"}]', 'checksum', 1)",
            rusqlite::params![document_id, base_revision],
        )
        .expect("seed journal delta");
}

#[test]
fn pending_recovery_journal_blocks_automatic_apply() {
    let connection = connection();
    insert_document(&connection, "doc-journal", &content_text("Borrador"));
    seed_journal_delta(&connection, "doc-journal", 0);

    let pending = pending_local_writes(&connection, "doc-journal").expect("barrier read");
    assert_eq!(
        pending,
        PendingLocalWrites {
            pending_journal_entries: 1,
            pending_outbox_entry: false,
        }
    );
    assert!(
        pending.blocks_automatic_apply(),
        "unrecovered journal deltas must defer automatic application"
    );
}

#[test]
fn pending_outbox_entry_blocks_automatic_apply() {
    let mut connection = connection();
    insert_document(&connection, "doc-outbox", &content_text("Inicial"));
    save(&mut connection, "doc-outbox", &content_text("Guardado"));

    let pending = pending_local_writes(&connection, "doc-outbox").expect("barrier read");
    assert_eq!(
        pending,
        PendingLocalWrites {
            pending_journal_entries: 0,
            pending_outbox_entry: true,
        }
    );
    assert!(
        pending.blocks_automatic_apply(),
        "an unacknowledged save must defer automatic application"
    );
}

#[test]
fn clean_document_allows_automatic_apply() {
    let connection = connection();
    insert_document(
        &connection,
        "doc-clean",
        &content_text("Sin cambios locales"),
    );

    let pending = pending_local_writes(&connection, "doc-clean").expect("barrier read");
    assert_eq!(
        pending,
        PendingLocalWrites {
            pending_journal_entries: 0,
            pending_outbox_entry: false,
        }
    );
    assert!(!pending.blocks_automatic_apply());
}

#[test]
fn pending_work_for_another_document_does_not_block() {
    let mut connection = connection();
    insert_document(&connection, "doc-busy", &content_text("Editando"));
    insert_document(
        &connection,
        "doc-other",
        &content_text("Sin cambios locales"),
    );
    save(&mut connection, "doc-busy", &content_text("Guardado"));
    seed_journal_delta(&connection, "doc-busy", 1);

    let busy = pending_local_writes(&connection, "doc-busy").expect("barrier read");
    assert_eq!(busy.pending_journal_entries, 1);
    assert!(busy.pending_outbox_entry);
    assert!(busy.blocks_automatic_apply());

    let other = pending_local_writes(&connection, "doc-other").expect("barrier read");
    assert_eq!(
        other,
        PendingLocalWrites {
            pending_journal_entries: 0,
            pending_outbox_entry: false,
        },
        "another document's journal and outbox work must not block this one"
    );
    assert!(!other.blocks_automatic_apply());
}

#[test]
fn journal_deltas_folded_into_a_revision_do_not_block() {
    let connection = connection();
    insert_document(&connection, "doc-folded", &content_text("Revisado"));
    seed_journal_delta(&connection, "doc-folded", 0);
    connection
        .execute(
            "UPDATE writing_documents SET revision = 1 WHERE id = 'doc-folded'",
            [],
        )
        .expect("fold the delta into a canonical revision");

    let pending = pending_local_writes(&connection, "doc-folded").expect("barrier read");
    assert_eq!(pending.pending_journal_entries, 0);
    assert!(
        !pending.blocks_automatic_apply(),
        "a folded delta is content, not pending recovery work"
    );
}

#[test]
fn outbox_entry_for_a_missing_document_still_blocks() {
    let mut connection = connection();
    insert_document(&connection, "doc-vanished", &content_text("Inicial"));
    save(&mut connection, "doc-vanished", &content_text("Guardado"));
    connection
        .execute(
            "DELETE FROM writing_documents WHERE id = 'doc-vanished'",
            [],
        )
        .expect("remove the document row");

    let pending = pending_local_writes(&connection, "doc-vanished").expect("barrier read");
    assert_eq!(pending.pending_journal_entries, 0);
    assert!(pending.pending_outbox_entry);
    assert!(
        pending.blocks_automatic_apply(),
        "pending work whose document vanished is never silently dropped"
    );
}

// ──────────────────── validated lww_lost settlement ────────────────────

#[test]
fn lww_lost_winner_from_our_own_device_still_preserves_the_loser() {
    let mut connection = connection();
    insert_document(&connection, "doc-1", &content_text("Texto local"));
    save(&mut connection, "doc-1", &content_text("Texto local"));
    let adjudicated = build_push_changes(&connection, Path::new(""))
        .expect("adjudicated draft")
        .ready
        .remove(0);

    // The winner is our own earlier landed row: the push response proves it is
    // NOT the adjudicated generation's echo, so the losing generation still
    // survives as a visible conflict copy.
    let winner = envelope("doc-1", "Texto remoto ganador");
    let row = row_from(&winner, 21, "device-own", 9_000);
    let outcome = apply_lww_lost_winner(
        &connection,
        "device-own",
        &row,
        Some(&adjudicated.acknowledgment),
        Path::new(""),
    )
    .expect("adjudicated settlement");

    let LwwLostSettlement::Routed(PullApplyOutcome::Applied {
        conflict_document_id: Some(conflict_id),
        ..
    }) = outcome
    else {
        panic!("expected one preserved conflict copy, got {outcome:?}");
    };
    assert_eq!(
        text_of(&connection, "doc-1"),
        "Texto remoto ganador",
        "the adjudicated winner lands"
    );
    assert_eq!(
        text_of(&connection, &conflict_id),
        "Texto local",
        "the losing generation survives in the conflict copy"
    );
    let entries = outbox_entries(&connection).expect("outbox");
    assert_eq!(
        entries.len(),
        1,
        "only the conflict copy remains pending work"
    );
    assert_eq!(
        entries[0].document_id, conflict_id,
        "the conflict copy is enqueued to sync back and the adjudicated \
         generation settled with its content preserved"
    );
    assert_eq!(recorded_server_seq(&connection, "doc-1"), Some(21));

    // A replay of the same result is a stale no-op: never a second copy.
    let replay = apply_lww_lost_winner(
        &connection,
        "device-own",
        &row,
        Some(&adjudicated.acknowledgment),
        Path::new(""),
    )
    .expect("replay");
    assert_eq!(replay, LwwLostSettlement::Routed(PullApplyOutcome::Stale));
    assert_eq!(
        document_count(&connection),
        2,
        "exactly one conflict copy after the replay"
    );
    assert_eq!(
        outbox_entries(&connection).expect("outbox").len(),
        1,
        "the replay changes no pending work"
    );
}

#[test]
fn lww_lost_never_clears_a_newer_local_generation() {
    let mut connection = connection();
    insert_document(&connection, "doc-1", &content_text("Texto local"));
    save(&mut connection, "doc-1", &content_text("Texto local"));
    let adjudicated = build_push_changes(&connection, Path::new(""))
        .expect("adjudicated draft")
        .ready
        .remove(0);

    // A save lands during the send round trip: a newer generation replaces the
    // adjudicated one and must survive this result untouched.
    enqueue_document(&connection, "doc-1").expect("newer generation");
    let newer = build_push_changes(&connection, Path::new(""))
        .expect("newer draft")
        .ready
        .remove(0);
    assert_ne!(adjudicated.acknowledgment, newer.acknowledgment);

    let winner = envelope("doc-1", "Texto remoto");
    let row = row_from(&winner, 21, "device-other", 9_000);
    let outcome = apply_lww_lost_winner(
        &connection,
        "device-own",
        &row,
        Some(&adjudicated.acknowledgment),
        Path::new(""),
    )
    .expect("guarded settlement");

    assert_eq!(outcome, LwwLostSettlement::NewerGenerationPending);
    assert_eq!(
        text_of(&connection, "doc-1"),
        "Texto local",
        "a newer local generation is never overwritten by an older result"
    );
    assert_eq!(
        document_count(&connection),
        1,
        "nothing settles, so no conflict copy is created"
    );
    assert_eq!(
        outbox_entries(&connection).expect("outbox")[0].acknowledgment,
        newer.acknowledgment,
        "the newer local generation keeps its outbox entry for its own push"
    );
    assert_eq!(recorded_server_seq(&connection, "doc-1"), None);
}

#[test]
fn lww_lost_route_defers_on_a_pending_recovery_journal() {
    let mut connection = connection();
    insert_document(&connection, "doc-1", &content_text("Texto local"));
    save(&mut connection, "doc-1", &content_text("Texto local"));
    seed_journal_delta(&connection, "doc-1", 1);
    let adjudicated = build_push_changes(&connection, Path::new(""))
        .expect("adjudicated draft")
        .ready
        .remove(0);

    let winner = envelope("doc-1", "Texto remoto");
    let row = row_from(&winner, 21, "device-other", 9_000);
    let outcome = apply_lww_lost_winner(
        &connection,
        "device-own",
        &row,
        Some(&adjudicated.acknowledgment),
        Path::new(""),
    )
    .expect("journal-guarded settlement");

    let LwwLostSettlement::Routed(PullApplyOutcome::Deferred { reason }) = outcome else {
        panic!("expected a journal deferral, got {outcome:?}");
    };
    assert!(
        reason.contains("PendingJournal"),
        "typed journal deferral, got {reason}"
    );
    assert_eq!(
        text_of(&connection, "doc-1"),
        "Texto local",
        "unsaved recovery state is never overwritten"
    );
    assert_eq!(
        document_count(&connection),
        1,
        "no conflict copy while the journal is pending"
    );
    assert_eq!(
        outbox_entries(&connection).expect("outbox").len(),
        1,
        "the adjudicated generation is retained"
    );
    assert_eq!(recorded_server_seq(&connection, "doc-1"), None);
}
