//! Persistence contract for the E1b-1a bibliography catalog foundation.
//!
//! These tests use only synthetic rows. They exercise the public repository seam
//! rather than private SQL helpers, while the migration itself is also applied
//! twice to prove that the checked-in schema is safe to replay.

use entropia_desktop_lib::bibliography::repository::{
    upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
    SourceOrigin, UpsertConnection, UpsertLibrary,
};
use rusqlite::Connection;

const MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0038_bibliography_catalog.sql");
const MIGRATION_NAME: &str = "0038_bibliography_catalog";

fn migrated_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory database");
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE _migrations (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             name TEXT NOT NULL UNIQUE,
             applied_at INTEGER NOT NULL
         );",
    )
    .expect("tracking table");

    for _ in 0..2 {
        conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
            .expect("apply bibliography migration");
    }
    conn.execute(
        "INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?1, 1)",
        [MIGRATION_NAME],
    )
    .expect("record migration");
    conn
}

fn connection(id: &str, instance: Option<&str>) -> UpsertConnection {
    UpsertConnection {
        id: id.to_string(),
        source_origin: SourceOrigin::Local,
        source_instance_id: instance.map(str::to_string),
        endpoint: Some("http://synthetic.invalid".to_string()),
        capabilities_json: r#"{"read":true}"#.to_string(),
    }
}

fn library(connection_id: &str, library_type: LibraryType, library_id: &str) -> UpsertLibrary {
    UpsertLibrary {
        connection_id: connection_id.to_string(),
        library_type,
        library_id: library_id.to_string(),
        name: format!("Synthetic {library_id}"),
        last_modified_version: Some(7),
    }
}

fn item(key: &str, version: i64, native: &str, csl: &str) -> BibliographicItemInput {
    BibliographicItemInput {
        item_key: key.to_string(),
        item_version: Some(version),
        native_json_snapshot: native.to_string(),
        csl_json_snapshot: csl.to_string(),
        item_type: Some("book".to_string()),
        title: Some("Synthetic work".to_string()),
        ..Default::default()
    }
}

#[test]
fn catalog_migration_is_idempotent_and_records_the_three_foundation_tables() {
    let conn = migrated_db();

    for table in [
        "zotero_connections",
        "zotero_libraries",
        "bibliographic_items",
    ] {
        let present: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(present.as_deref(), Some(table), "missing table {table}");
    }

    let foreign_keys: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('bibliographic_items')
             WHERE \"table\" = 'zotero_libraries'",
            [],
            |row| row.get(0),
        )
        .expect("foreign-key metadata");
    assert_eq!(foreign_keys, 1);
}

#[test]
fn the_same_native_key_in_different_libraries_stays_distinct() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal = upsert_library(&mut conn, library(&source.id, LibraryType::User, "0"))
        .expect("personal library");
    let group = upsert_library(
        &mut conn,
        library(&source.id, LibraryType::Group, "6680944"),
    )
    .expect("group library");

    let first = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "SAMEKEY1",
            1,
            r#"{"key":"SAMEKEY1","version":1}"#,
            r#"{"id":"personal-csl","type":"book"}"#,
        ),
    )
    .expect("personal item");
    let second = upsert_item(
        &mut conn,
        &group.id,
        item(
            "SAMEKEY1",
            1,
            r#"{"key":"SAMEKEY1","version":1}"#,
            r#"{"id":"group-csl","type":"book"}"#,
        ),
    )
    .expect("group item");

    assert_ne!(first.id, second.id);
    assert_eq!(first.identity.library_id, "0");
    assert_eq!(second.identity.library_id, "6680944");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE item_key='SAMEKEY1'",
            [],
            |row| row.get(0),
        )
        .expect("count items");
    assert_eq!(count, 2);
}

#[test]
fn native_and_csl_snapshots_round_trip_without_rewriting_identity() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let library =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    let native = r#"{"key":"ROUNDTRP","version":9,"data":{"title":"Synthetic"}}"#;
    let csl = r#"{"id":"different-csl-id","type":"book","title":"Synthetic"}"#;

    let saved =
        upsert_item(&mut conn, &library.id, item("ROUNDTRP", 9, native, csl)).expect("item");

    assert_eq!(saved.identity.source_origin, SourceOrigin::Local);
    assert_eq!(saved.identity.source_instance_id, None);
    assert_eq!(saved.identity.library_type, LibraryType::User);
    assert_eq!(saved.identity.library_id, "0");
    assert_eq!(saved.identity.item_key, "ROUNDTRP");
    assert_eq!(saved.native_json_snapshot, native);
    assert_eq!(saved.csl_json_snapshot, csl);
    assert!(saved.created_at > 0);
    assert!(saved.updated_at >= saved.created_at);
}

#[test]
fn malformed_creators_json_is_rejected() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let zotero_library =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    let mut malformed = item(
        "BADJSON1",
        1,
        r#"{"key":"BADJSON1","version":1}"#,
        r#"{"id":"bad-json-csl","type":"book"}"#,
    );
    malformed.creators_json = Some(r#"[{"name":"Ada"}"#.to_string());

    let error = upsert_item(&mut conn, &zotero_library.id, malformed)
        .expect_err("malformed optional creators_json must be rejected");

    assert_eq!(error.code, "invalid_json");
}

#[test]
fn repeated_upsert_updates_one_row_in_place() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", Some("fixture-instance")))
        .expect("connection");
    let zotero_library =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    assert_eq!(
        upsert_connection(&mut conn, connection("conn-1", Some("fixture-instance")))
            .expect("repeated connection upsert"),
        source
    );
    assert_eq!(
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0"))
            .expect("repeated library upsert"),
        zotero_library
    );

    let first = upsert_item(
        &mut conn,
        &zotero_library.id,
        item(
            "REPEAT01",
            1,
            r#"{"key":"REPEAT01","version":1}"#,
            r#"{"id":"repeat-csl-v1","type":"book"}"#,
        ),
    )
    .expect("first upsert");
    let second = upsert_item(
        &mut conn,
        &zotero_library.id,
        item(
            "REPEAT01",
            2,
            r#"{"key":"REPEAT01","version":2}"#,
            r#"{"id":"repeat-csl-v2","type":"book"}"#,
        ),
    )
    .expect("second upsert");
    let third = upsert_item(
        &mut conn,
        &zotero_library.id,
        item(
            "REPEAT01",
            2,
            r#"{"key":"REPEAT01","version":2}"#,
            r#"{"id":"repeat-csl-v2","type":"book"}"#,
        ),
    )
    .expect("unchanged repeated upsert");

    assert_eq!(first.id, second.id);
    assert_eq!(second, third);
    assert_eq!(second.item_version, Some(2));
    assert_eq!(second.revision, first.revision + 1);
    assert_eq!(
        second.identity.source_instance_id.as_deref(),
        Some("fixture-instance")
    );
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id=?1 AND item_key=?2",
            rusqlite::params![zotero_library.id, "REPEAT01"],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(count, 1);
}
