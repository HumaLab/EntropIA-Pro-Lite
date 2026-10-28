//! An in-memory archive with one personal Zotero library, for the annotation
//! tests.

use rusqlite::Connection;

pub(crate) fn archive_with_library(library_row_id: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../../tests/fixtures/schema_full.sql"))
        .unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn.execute(
        "INSERT INTO zotero_connections (id, source_origin, created_at, updated_at) VALUES ('conn1', 'local', 0, 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at) \
         VALUES (?1, 'conn1', 'user', '0', 'My Library', 0, 0)",
        [library_row_id],
    )
    .unwrap();
    conn
}
