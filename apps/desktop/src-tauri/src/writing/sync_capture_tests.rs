//! Tests for the writing sync outbox and capability state. The hooks run from
//! the real repository write paths against a real migrated database, so what
//! is asserted here is exactly what the future transport will consume.

use rusqlite::Connection;

use super::repository::{
    duplicate_document, rename_document, save_document, set_status, SaveDocument,
};
use super::sync_capture::{
    acknowledge_outbox_entry, enqueue_document, enqueue_document_at_for_test, outbox_entries,
    record_capability, seed_outbox, supports_writing, OutboxEntry,
};

const MIGRATION_0035: &str =
    include_str!("../../../../../packages/store/src/migrations/0035_writing_workspace.sql");
const MIGRATION_0036: &str =
    include_str!("../../../../../packages/store/src/migrations/0036_writing_journal.sql");

fn migrated_connection() -> Connection {
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
    connection
}

/// The full stack the capture hooks see: writing migrations plus the real sync
/// schema.
fn synced_connection() -> Connection {
    let connection = migrated_connection();
    crate::sync::schema::ensure_sync_schema(&connection).expect("sync schema");
    connection
}

fn set_meta(connection: &Connection, key: &str, value: &str) {
    connection
        .execute(
            "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )
        .expect("set meta");
}

fn enable_capture(connection: &Connection) {
    set_meta(connection, "capture_enabled", "1");
}

fn insert_document(connection: &Connection, id: &str) {
    connection
        .execute(
            "INSERT INTO writing_documents
               (id, title, document_type, status, schema_version, current_content_json,
                revision, created_at, updated_at)
             VALUES (?1, ?1, 'article', 'active', 1, '{\"doc\":{}}', 0, 1, 1)",
            rusqlite::params![id],
        )
        .expect("insert document");
}

fn save(connection: &mut Connection, id: &str, expected_revision: i64) -> i64 {
    save_document(
        connection,
        SaveDocument {
            document_id: id.to_string(),
            expected_revision,
            content_json: "{\"doc\":{\"text\":\"nuevo\"}}".to_string(),
            schema_version: 1,
            plain_text_cache: Some("nuevo".to_string()),
            citations: Vec::new(),
            zotero_citations: Vec::new(),
            provenance: Vec::new(),
        },
    )
    .expect("save document")
}

fn ids(connection: &Connection) -> Vec<String> {
    outbox_entries(connection)
        .expect("outbox entries")
        .into_iter()
        .map(|entry| entry.document_id)
        .collect()
}

fn entry(connection: &Connection, document_id: &str) -> OutboxEntry {
    outbox_entries(connection)
        .expect("outbox entries")
        .into_iter()
        .find(|entry| entry.document_id == document_id)
        .expect("document outbox entry")
}

fn acknowledge_current(connection: &Connection, document_id: &str) {
    let captured = entry(connection, document_id);
    assert!(
        acknowledge_outbox_entry(connection, &captured.acknowledgment)
            .expect("acknowledge current entry"),
        "the captured entry must still be current"
    );
}

#[test]
fn save_rename_and_status_enqueue_only_while_capture_is_enabled() {
    let mut connection = synced_connection();
    enable_capture(&connection);
    insert_document(&connection, "doc-1");

    save(&mut connection, "doc-1", 0);
    assert_eq!(ids(&connection), vec!["doc-1".to_string()], "save enqueues");

    acknowledge_current(&connection, "doc-1");
    rename_document(&connection, "doc-1", "Otro título").expect("rename");
    assert_eq!(
        ids(&connection),
        vec!["doc-1".to_string()],
        "rename enqueues"
    );

    acknowledge_current(&connection, "doc-1");
    set_status(&connection, "doc-1", "trashed").expect("status");
    assert_eq!(
        ids(&connection),
        vec!["doc-1".to_string()],
        "status enqueues"
    );

    set_meta(&connection, "capture_enabled", "0");
    acknowledge_current(&connection, "doc-1");
    rename_document(&connection, "doc-1", "Sin sesión").expect("rename");
    assert!(
        ids(&connection).is_empty(),
        "no capture without an enabled session"
    );
}

#[test]
fn remote_apply_style_writes_do_not_enqueue_and_applying_gates_capture() {
    let mut connection = synced_connection();
    enable_capture(&connection);
    insert_document(&connection, "doc-1");

    set_meta(&connection, "applying", "1");
    save(&mut connection, "doc-1", 0);
    assert!(
        ids(&connection).is_empty(),
        "applying='1' suppresses capture"
    );

    set_meta(&connection, "applying", "0");
    save(&mut connection, "doc-1", 1);
    assert_eq!(
        ids(&connection),
        vec!["doc-1".to_string()],
        "capture resumes"
    );
}

#[test]
fn hooks_are_a_noop_without_the_sync_schema() {
    let mut connection = migrated_connection();
    insert_document(&connection, "doc-1");
    let revision = save(&mut connection, "doc-1", 0);
    assert_eq!(revision, 1, "saves must not fail when sync is absent");
    rename_document(&connection, "doc-1", "Local").expect("rename");
    set_status(&connection, "doc-1", "archived").expect("status");
}

#[test]
fn duplicate_enqueues_the_copy() {
    let mut connection = synced_connection();
    enable_capture(&connection);
    insert_document(&connection, "doc-1");

    duplicate_document(&mut connection, "doc-1", "doc-2", "Copia").expect("duplicate");
    assert_eq!(
        ids(&connection),
        vec!["doc-2".to_string()],
        "only the copy is new work"
    );
}

#[test]
fn seed_outbox_skips_acknowledged_documents_and_is_idempotent() {
    let connection = synced_connection();
    insert_document(&connection, "doc-1");
    insert_document(&connection, "doc-2");
    insert_document(&connection, "doc-3");
    connection
        .execute(
            "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
             VALUES ('writing_envelopes', 'doc-2', 7)",
            [],
        )
        .expect("acknowledged doc-2");

    let seeded = seed_outbox(&connection).expect("seed");
    assert_eq!(seeded, 2, "doc-2 is already acknowledged");
    let entries = outbox_entries(&connection).expect("seeded entries");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.document_id.as_str())
            .collect::<Vec<_>>(),
        vec!["doc-1", "doc-3"]
    );
    assert_ne!(
        entries[0].acknowledgment.generation, entries[1].acknowledgment.generation,
        "each seeded document gets an independent generation"
    );

    let captured = entries[0].acknowledgment.clone();
    let again = seed_outbox(&connection).expect("seed again");
    assert_eq!(again, 0, "seeding is idempotent");
    assert_eq!(
        entry(&connection, "doc-1").acknowledgment,
        captured,
        "seeding preserves the pending generation"
    );
}

#[test]
fn entries_coalesce_per_document_and_clear_removes_them() {
    let connection = synced_connection();
    enable_capture(&connection);

    enqueue_document(&connection, "doc-1").expect("first");
    enqueue_document(&connection, "doc-2").expect("other");
    enqueue_document(&connection, "doc-1").expect("second for same doc");

    let entries = outbox_entries(&connection).expect("entries");
    assert_eq!(
        entries.len(),
        2,
        "one entry per document, last capture wins"
    );
    assert_eq!(entries[0].op, 'U');

    acknowledge_current(&connection, "doc-1");
    assert_eq!(ids(&connection), vec!["doc-2".to_string()]);
    acknowledge_current(&connection, "doc-2");
    assert!(ids(&connection).is_empty());
}

#[test]
fn captures_in_the_same_millisecond_get_distinct_acknowledgment_generations() {
    let connection = synced_connection();
    enable_capture(&connection);

    enqueue_document_at_for_test(&connection, "doc-1", 4_242).expect("first capture");
    let first = entry(&connection, "doc-1");
    enqueue_document_at_for_test(&connection, "doc-1", 4_242).expect("second capture");
    let second = entry(&connection, "doc-1");

    assert_eq!(
        first.changed_at, second.changed_at,
        "clock is deliberately fixed"
    );
    assert_ne!(
        first.acknowledgment.generation, second.acknowledgment.generation,
        "generation identity is independent of wall-clock time"
    );
}

#[test]
fn stale_acknowledgment_cannot_remove_a_newer_same_timestamp_capture() {
    let connection = synced_connection();
    enable_capture(&connection);

    enqueue_document_at_for_test(&connection, "doc-1", 7_000).expect("first capture");
    let stale = entry(&connection, "doc-1").acknowledgment;
    enqueue_document_at_for_test(&connection, "doc-1", 7_000).expect("newer capture");
    let current = entry(&connection, "doc-1").acknowledgment;

    assert!(
        !acknowledge_outbox_entry(&connection, &stale).expect("stale acknowledgment"),
        "the stale generation is not deleted"
    );
    assert_eq!(entry(&connection, "doc-1").acknowledgment, current);
}

#[test]
fn matching_acknowledgment_removes_only_its_captured_entry() {
    let connection = synced_connection();
    enable_capture(&connection);
    enqueue_document_at_for_test(&connection, "doc-1", 1).expect("doc-1 capture");
    enqueue_document_at_for_test(&connection, "doc-2", 1).expect("doc-2 capture");
    let doc_1 = entry(&connection, "doc-1").acknowledgment;

    assert!(acknowledge_outbox_entry(&connection, &doc_1).expect("matching acknowledgment"));
    assert_eq!(ids(&connection), vec!["doc-2".to_string()]);
}

#[test]
fn duplicate_acknowledgment_is_a_noop() {
    let connection = synced_connection();
    enable_capture(&connection);
    enqueue_document_at_for_test(&connection, "doc-1", 1).expect("capture");
    let captured = entry(&connection, "doc-1").acknowledgment;

    assert!(acknowledge_outbox_entry(&connection, &captured).expect("first acknowledgment"));
    assert!(
        !acknowledge_outbox_entry(&connection, &captured).expect("duplicate acknowledgment"),
        "an already consumed acknowledgment changes nothing"
    );
}

#[test]
fn seed_preserves_a_legacy_pending_entry_and_legacy_cas_is_conservative() {
    let connection = synced_connection();
    insert_document(&connection, "doc-legacy");
    let legacy_raw = r#"{"op":"U","changed_at":77}"#;
    set_meta(&connection, "writing_outbox:doc-legacy", legacy_raw);

    let legacy = entry(&connection, "doc-legacy");
    assert_eq!(legacy.acknowledgment.generation, None);
    assert_eq!(seed_outbox(&connection).expect("seed"), 0);
    assert_eq!(
        entry(&connection, "doc-legacy"),
        legacy,
        "seed keeps legacy raw state"
    );

    enable_capture(&connection);
    enqueue_document_at_for_test(&connection, "doc-legacy", 77).expect("new generation");
    assert!(
        !acknowledge_outbox_entry(&connection, &legacy.acknowledgment)
            .expect("stale legacy acknowledgment"),
        "legacy raw CAS cannot delete the replacement generation"
    );
    assert!(
        entry(&connection, "doc-legacy")
            .acknowledgment
            .generation
            .is_some(),
        "new pending generation remains"
    );
}

#[test]
fn malformed_outbox_entries_are_rejected_without_mutation() {
    let connection = synced_connection();
    let malformed = [
        r#"{"op":"U"}"#,
        r#"{"op":"X","changed_at":1}"#,
        r#"{"op":"U","changed_at":-1}"#,
        r#"{"op":"U","changed_at":1,"generation":"not-a-uuid"}"#,
        r#"{"op":"U","changed_at":1,"unexpected":true}"#,
    ];

    for raw in malformed {
        set_meta(&connection, "writing_outbox:bad", raw);
        let error = outbox_entries(&connection).expect_err("malformed entry must fail");
        assert_eq!(error.code, "writing_outbox_corrupt");
        let retained: String = connection
            .query_row(
                "SELECT value FROM sync_meta WHERE key = 'writing_outbox:bad'",
                [],
                |row| row.get(0),
            )
            .expect("malformed entry retained");
        assert_eq!(
            retained, raw,
            "reader never repairs or deletes malformed work"
        );
    }
}

#[test]
fn literal_prefix_reader_ignores_writing_x_outbox_decoys() {
    let connection = synced_connection();
    enable_capture(&connection);
    enqueue_document_at_for_test(&connection, "real", 1).expect("real capture");
    set_meta(&connection, "writingXoutbox:decoy", "not-json");

    let entries = outbox_entries(&connection).expect("literal prefix read");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].document_id, "real");
}

#[test]
fn capability_is_bound_to_the_server_epoch_of_the_discovery() {
    let connection = synced_connection();

    assert!(
        !supports_writing(&connection, "epoch-a").expect("read"),
        "nothing discovered yet"
    );

    record_capability(&connection, "epoch-a", true).expect("advertised");
    assert!(supports_writing(&connection, "epoch-a").expect("read"));
    assert!(
        !supports_writing(&connection, "epoch-b").expect("read"),
        "a discovery from another epoch never enables the transport"
    );

    record_capability(&connection, "epoch-a", false).expect("withdrawn");
    assert!(!supports_writing(&connection, "epoch-a").expect("read"));

    record_capability(&connection, "epoch-b", true).expect("new epoch");
    assert!(supports_writing(&connection, "epoch-b").expect("read"));
    assert!(!supports_writing(&connection, "epoch-a").expect("read"));
}
