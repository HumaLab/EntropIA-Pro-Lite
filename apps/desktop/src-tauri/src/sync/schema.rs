//! Local sync schema (DESIGN §6). All `sync_*` bookkeeping tables, owned and
//! managed exclusively by the Rust sync module. Idempotent: every statement is
//! `IF NOT EXISTS`, so this is safe to run on every `sync_ensure_capture` and
//! after every JS/Rust migration pass.

use rusqlite::Connection;

/// The full DDL for the local sync schema (DESIGN §6). Kept as one batch so the
/// fixture/test helpers and the runtime path share a single source of truth.
const SYNC_SCHEMA_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS sync_oplog (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  table_name TEXT NOT NULL,
  row_id TEXT NOT NULL,
  op TEXT NOT NULL CHECK (op IN ('I','U','D')),
  changed_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sync_oplog_row ON sync_oplog(table_name, row_id);

CREATE TABLE IF NOT EXISTS sync_row_versions (
  table_name TEXT NOT NULL,
  row_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL,
  PRIMARY KEY (table_name, row_id)
);

CREATE TABLE IF NOT EXISTS sync_conflicts (
  id TEXT PRIMARY KEY,
  table_name TEXT NOT NULL,
  row_id TEXT NOT NULL,
  reason TEXT NOT NULL,
  loser_payload TEXT,
  winner_summary TEXT,
  created_at INTEGER NOT NULL,
  acknowledged INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sync_pending_rows (
  table_name TEXT NOT NULL,
  row_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL,
  deleted INTEGER NOT NULL,
  changed_at INTEGER NOT NULL,
  device_id TEXT NOT NULL,
  payload TEXT,
  retry_count INTEGER NOT NULL DEFAULT 0,
  parked_schema_head TEXT,
  PRIMARY KEY (table_name, row_id)
);

CREATE TABLE IF NOT EXISTS sync_pending_blobs (
  asset_id TEXT PRIMARY KEY,
  sha256 TEXT NOT NULL,
  rel_path TEXT NOT NULL,
  size INTEGER NOT NULL,
  retry_count INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  last_attempt_at INTEGER
);

CREATE TABLE IF NOT EXISTS sync_pending_fts (item_id TEXT PRIMARY KEY);

CREATE TABLE IF NOT EXISTS sync_topic_aliases (
  remote_id TEXT PRIMARY KEY,
  local_id TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_blob_index (
  asset_id TEXT PRIMARY KEY,
  sha256 TEXT NOT NULL,
  size INTEGER NOT NULL,
  file_mtime_ms INTEGER NOT NULL,
  uploaded INTEGER NOT NULL DEFAULT 0
);
"#;

/// Names of every `sync_*` table this module creates. Used by tests (and later
/// slices, e.g. logout per DESIGN §6.3) to enumerate the schema deterministically.
#[allow(dead_code)]
pub const SYNC_TABLES: &[&str] = &[
    "sync_meta",
    "sync_oplog",
    "sync_row_versions",
    "sync_conflicts",
    "sync_pending_rows",
    "sync_pending_blobs",
    "sync_pending_fts",
    "sync_topic_aliases",
    "sync_blob_index",
];

/// Creates all `sync_*` tables and indexes (DESIGN §6). Idempotent.
pub fn ensure_sync_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(SYNC_SCHEMA_DDL)
        .map_err(|e| format!("Failed to ensure sync schema: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("pragmas");
        conn
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1",
            [name],
            |_| Ok(true),
        )
        .unwrap_or(false)
    }

    #[test]
    fn ensure_sync_schema_creates_all_tables_and_is_idempotent() {
        let conn = open();
        ensure_sync_schema(&conn).expect("first ensure");
        ensure_sync_schema(&conn).expect("second ensure is idempotent");

        for table in SYNC_TABLES {
            assert!(table_exists(&conn, table), "missing table {table}");
        }
    }

    #[test]
    fn sync_oplog_has_row_index() {
        let conn = open();
        ensure_sync_schema(&conn).expect("ensure");
        let has_index: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_sync_oplog_row'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(has_index, "idx_sync_oplog_row should exist");
    }

    #[test]
    fn sync_oplog_op_check_constraint_rejects_invalid_op() {
        let conn = open();
        ensure_sync_schema(&conn).expect("ensure");
        let err = conn.execute(
            "INSERT INTO sync_oplog(table_name, row_id, op, changed_at) VALUES ('items','r','X',1)",
            [],
        );
        assert!(err.is_err(), "op CHECK should reject values outside I/U/D");
    }
}
