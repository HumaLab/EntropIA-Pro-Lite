//! Atomicity tests for canonical writing mutations and their sync outbox token.

use rusqlite::Connection;

use super::repository::{create_document, load_document, rename_document, set_status, NewDocument};
use super::sync_capture::outbox_entries;

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

fn synced_connection() -> Connection {
    let connection = migrated_connection();
    crate::sync::schema::ensure_sync_schema(&connection).expect("sync schema");
    connection
}

fn new_document(id: &str) -> NewDocument {
    NewDocument {
        id: id.to_string(),
        title: "Original title".to_string(),
        document_type: "article".to_string(),
        schema_version: 1,
        content_json: r#"{"type":"doc","content":[]}"#.to_string(),
    }
}

fn set_meta(connection: &Connection, key: &str, value: &str) {
    connection
        .execute(
            "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )
        .expect("set sync metadata");
}

fn enable_capture(connection: &Connection) {
    set_meta(connection, "capture_enabled", "1");
}

fn install_outbox_failure(connection: &Connection) {
    connection
        .execute_batch(
            "CREATE TRIGGER fail_writing_outbox
             BEFORE INSERT ON sync_meta
             WHEN NEW.key = 'writing_outbox:doc-1'
             BEGIN
               SELECT RAISE(ABORT, 'synthetic writing outbox failure');
             END;",
        )
        .expect("install deterministic outbox failure");
}

fn document_count(connection: &Connection, id: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM writing_documents WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .expect("count documents")
}

#[test]
fn create_enqueues_exactly_one_current_generation_when_capture_is_enabled() {
    let connection = synced_connection();
    enable_capture(&connection);

    let created = create_document(&connection, new_document("doc-1")).expect("create document");
    let entries = outbox_entries(&connection).expect("read outbox");

    assert_eq!(created.id, "doc-1");
    assert_eq!(entries.len(), 1, "one canonical mutation has one token");
    assert_eq!(entries[0].document_id, "doc-1");
    assert_eq!(entries[0].op, 'U');
    assert!(
        entries[0].acknowledgment.generation.is_some(),
        "the current token has a generation identity"
    );
}

#[test]
fn create_remains_fully_functional_without_the_sync_schema() {
    let connection = migrated_connection();

    let created = create_document(&connection, new_document("doc-1")).expect("local create");
    let sync_meta_exists: i64 = connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sync_meta'
             )",
            [],
            |row| row.get(0),
        )
        .expect("inspect schema");

    assert_eq!(created, load_document(&connection, "doc-1").expect("load"));
    assert_eq!(sync_meta_exists, 0, "local-only create adds no sync schema");
}

#[test]
fn create_rolls_back_when_the_outbox_insert_fails() {
    let connection = synced_connection();
    enable_capture(&connection);
    install_outbox_failure(&connection);

    let error = create_document(&connection, new_document("doc-1"))
        .expect_err("outbox failure must reject create");

    assert_eq!(error.code, "sql_error");
    assert!(error.message.contains("synthetic writing outbox failure"));
    assert_eq!(document_count(&connection, "doc-1"), 0);
    assert!(
        outbox_entries(&connection).expect("read outbox").is_empty(),
        "a failed create leaks no outbox token"
    );
}

#[test]
fn rename_rolls_back_when_the_outbox_insert_fails() {
    let connection = synced_connection();
    let original = create_document(&connection, new_document("doc-1")).expect("seed document");
    enable_capture(&connection);
    install_outbox_failure(&connection);

    let error = rename_document(&connection, "doc-1", "Replacement title")
        .expect_err("outbox failure must reject rename");

    assert_eq!(error.code, "sql_error");
    assert_eq!(
        load_document(&connection, "doc-1").expect("load unchanged document"),
        original,
        "title, timestamps, content, and revision all roll back"
    );
    assert!(outbox_entries(&connection).expect("read outbox").is_empty());
}

#[test]
fn status_rolls_back_when_the_outbox_insert_fails() {
    let connection = synced_connection();
    let original = create_document(&connection, new_document("doc-1")).expect("seed document");
    enable_capture(&connection);
    install_outbox_failure(&connection);

    let error = set_status(&connection, "doc-1", "archived")
        .expect_err("outbox failure must reject status change");

    assert_eq!(error.code, "sql_error");
    assert_eq!(
        load_document(&connection, "doc-1").expect("load unchanged document"),
        original,
        "status, timestamps, content, and revision all roll back"
    );
    assert!(outbox_entries(&connection).expect("read outbox").is_empty());
}

#[test]
fn malformed_capture_gate_schema_is_reported_and_rolls_back_the_mutation() {
    let connection = migrated_connection();
    let original = create_document(&connection, new_document("doc-1")).expect("seed document");
    connection
        .execute_batch("CREATE TABLE sync_meta (malformed_column TEXT NOT NULL);")
        .expect("install malformed readable sync table");

    let error = rename_document(&connection, "doc-1", "Must not persist")
        .expect_err("gate read failure must reject rename");

    assert_eq!(error.code, "sql_error");
    assert!(error
        .message
        .contains("Failed to read writing capture gates"));
    assert_eq!(
        load_document(&connection, "doc-1").expect("load unchanged document"),
        original,
        "a gate read failure cannot leave a canonical-only rename"
    );
}

#[test]
fn caller_rollback_restores_both_document_and_outbox_after_nested_capture() {
    let connection = synced_connection();
    enable_capture(&connection);
    connection
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("open caller transaction");

    create_document(&connection, new_document("doc-1")).expect("nested create");
    assert_eq!(document_count(&connection, "doc-1"), 1);
    assert_eq!(
        outbox_entries(&connection)
            .expect("read nested outbox")
            .len(),
        1
    );

    connection
        .execute_batch("ROLLBACK;")
        .expect("caller rollback");

    assert_eq!(document_count(&connection, "doc-1"), 0);
    assert!(
        outbox_entries(&connection)
            .expect("read rolled-back outbox")
            .is_empty(),
        "the inner savepoint must not commit its caller"
    );
}

#[test]
fn rename_and_status_each_replace_the_single_current_generation() {
    let connection = synced_connection();
    create_document(&connection, new_document("doc-1")).expect("seed document");
    enable_capture(&connection);

    rename_document(&connection, "doc-1", "Renamed").expect("rename");
    let renamed_entries = outbox_entries(&connection).expect("renamed outbox");
    assert_eq!(renamed_entries.len(), 1);
    let renamed_generation = renamed_entries[0]
        .acknowledgment
        .generation
        .clone()
        .expect("rename generation");

    set_status(&connection, "doc-1", "archived").expect("set status");
    let status_entries = outbox_entries(&connection).expect("status outbox");
    assert_eq!(status_entries.len(), 1);
    let status_generation = status_entries[0]
        .acknowledgment
        .generation
        .as_ref()
        .expect("status generation");
    let current = load_document(&connection, "doc-1").expect("load current document");

    assert_ne!(&renamed_generation, status_generation);
    assert_eq!(current.title, "Renamed");
    assert_eq!(current.status, "archived");
    assert_eq!(
        current.revision, 0,
        "metadata never advances content revision"
    );
}

#[test]
fn applying_gate_still_suppresses_capture_without_suppressing_local_writes() {
    let connection = synced_connection();
    enable_capture(&connection);
    set_meta(&connection, "applying", "1");

    create_document(&connection, new_document("doc-1")).expect("suppressed create");

    assert_eq!(document_count(&connection, "doc-1"), 1);
    assert!(outbox_entries(&connection).expect("read outbox").is_empty());

    set_meta(&connection, "applying", "0");
    rename_document(&connection, "doc-1", "Captured again").expect("captured rename");
    assert_eq!(outbox_entries(&connection).expect("read outbox").len(), 1);
}
