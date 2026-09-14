//! Durable storage behind the batch-processing queue (plan-lote.md §5).
//!
//! Every queue mutation in later units commits through this module on a
//! connection opened with [`crate::db::open::open_archive_connection`], so a
//! task receipt and its canonical result share one transaction. This file owns
//! the small read primitives Unidad 1 needs — migration presence, effective
//! PRAGMAs, and queue counters — plus the interface the scheduler builds on:
//! `prepare_batch`, `admit_or_attach`, `claim_next`, `save_checkpoint`,
//! `commit_success`, `finish_attempt`, `control_batch`, `recover_session` and
//! `read_snapshot` (those arrive in Unidades 2–3).

use rusqlite::Connection;

/// Migration that creates the processing tables. The frontend
/// `runMigrations()` applies it; the backend never runs DDL itself — it only
/// verifies presence before admitting or recovering work.
pub const MIGRATION_NAME: &str = "0032_batch_processing";

/// Error code returned when the queue schema is not applied yet. The frontend
/// treats it as a recoverable state (schema still migrating), never as a
/// per-item failure.
pub const SCHEMA_NOT_READY: &str = "schema_not_ready";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pragmas {
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: i64,
    pub busy_timeout: i64,
}

/// Reads the effective PRAGMAs of `conn`. Verification reads them back instead
/// of trusting the open call: a connection that silently lost one of these
/// must never admit queue work.
pub fn read_pragmas(conn: &Connection) -> Result<Pragmas, String> {
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read journal_mode: {e}"))?;
    let synchronous: i64 = conn
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read synchronous: {e}"))?;
    let foreign_keys: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read foreign_keys: {e}"))?;
    let busy_timeout: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read busy_timeout: {e}"))?;
    Ok(Pragmas {
        journal_mode,
        synchronous,
        foreign_keys,
        busy_timeout,
    })
}

/// True when the durable queue is usable on this connection: WAL is active,
/// commits are FULL, foreign keys are enforced, and the migration row exists.
pub fn is_schema_ready(conn: &Connection) -> Result<bool, String> {
    let pragmas = read_pragmas(conn)?;
    if !pragmas.journal_mode.eq_ignore_ascii_case("wal") {
        return Ok(false);
    }
    if pragmas.synchronous != 2 || pragmas.foreign_keys != 1 {
        return Ok(false);
    }
    Ok(is_migration_applied(conn, MIGRATION_NAME)?)
}

/// True when `_migrations` records `name`. A missing tracking table means no
/// migration has ever run here, which is also "not applied" — not an error.
pub fn is_migration_applied(conn: &Connection, name: &str) -> Result<bool, String> {
    use rusqlite::OptionalExtension as _;
    let table: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to inspect sqlite_master: {e}"))?;
    if table.is_none() {
        return Ok(false);
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM _migrations WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read _migrations: {e}"))?;
    Ok(count > 0)
}

/// Queue counters for the initialize response and the recovery banner.
/// Every count is a plain aggregate over the durable tables — no events, no
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueSummary {
    pub pending_batches: i64,
    pub active_tasks: i64,
    pub interrupted_tasks: i64,
    pub failed_tasks: i64,
    pub succeeded_tasks: i64,
}

fn count(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(|e| format!("Failed to count queue rows: {e}"))
}

/// Reads the queue summary. Callers must check [`is_schema_ready`] first: on a
/// database without the migration these tables do not exist.
pub fn read_summary(conn: &Connection) -> Result<QueueSummary, String> {
    Ok(QueueSummary {
        pending_batches: count(
            conn,
            "SELECT COUNT(*) FROM processing_batches WHERE state IN ('preparing', 'ready', 'running', 'pausing', 'paused', 'interrupted')",
        )?,
        active_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state IN ('pending', 'blocked', 'running', 'retry_wait')",
        )?,
        interrupted_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'interrupted'",
        )?,
        failed_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'failed'",
        )?,
        succeeded_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'succeeded'",
        )?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mirrored migration file, exercised here so registry/file drift
    /// breaks a test instead of reaching a user database.
    const MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0032_batch_processing.sql");

    fn migrated_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        let conn = crate::db::open::open_archive_connection(&db_path).expect("open");
        conn.execute_batch("CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
            .expect("minimal collections");
        conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
            .expect("minimal items");
        conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);")
            .expect("minimal assets");
        conn.execute_batch("CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);")
            .expect("minimal extractions");
        conn.execute_batch("CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, language TEXT, duration_ms INTEGER, model TEXT NOT NULL, segments TEXT, confidence REAL, created_at INTEGER NOT NULL);")
            .expect("minimal transcriptions");
        conn.execute_batch("CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);")
            .expect("migrations tracking");
        // Same single-batch application the frontend runner uses for 0032.
        conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
            .expect("apply 0032 mirror");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [MIGRATION_NAME],
        )
        .expect("track 0032");
        (dir, conn)
    }

    #[test]
    fn an_unmigrated_database_is_not_schema_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = crate::db::open::open_archive_connection(&dir.path().join("entropia.sqlite"))
            .expect("open");
        assert_eq!(
            is_schema_ready(&conn).expect("check"),
            false,
            "no migration row, no tables: the gate must stay closed"
        );
    }

    #[test]
    fn the_mirrored_migration_applies_and_opens_the_gate() {
        let (_dir, conn) = migrated_db();
        assert!(
            is_schema_ready(&conn).expect("check"),
            "tables + tracking row + durable PRAGMAs must read ready"
        );
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'processing_%' ORDER BY name")
            .expect("tables query")
            .query_map([], |row| row.get(0))
            .expect("map")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(tables.len(), 9, "all nine processing tables: {tables:?}");
    }

    #[test]
    fn a_committed_task_survives_closing_and_reopening_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        {
            let conn = crate::db::open::open_archive_connection(&db_path).expect("open");
            conn.execute_batch("CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
                .expect("collections");
            conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
                .expect("items");
            conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);")
                .expect("assets");
            conn.execute_batch("CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);")
                .expect("extractions");
            conn.execute_batch("CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, language TEXT, duration_ms INTEGER, model TEXT NOT NULL, segments TEXT, confidence REAL, created_at INTEGER NOT NULL);")
                .expect("transcriptions");
            conn.execute_batch("CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);")
                .expect("tracking");
            conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
                .expect("apply 0032");
            conn.execute(
                "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
                [MIGRATION_NAME],
            )
            .expect("track");
            conn.execute(
                "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t1', 'ocr', 'a1', 'succeeded', 1, 1)",
                [],
            )
            .expect("insert task");
        }
        // Process "death": drop every connection, then reopen the same file.
        let reopened = crate::db::open::open_archive_connection(&db_path).expect("reopen");
        assert!(is_schema_ready(&reopened).expect("check"));
        let state: String = reopened
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("task survived the restart");
        assert_eq!(state, "succeeded");
    }

    #[test]
    fn the_active_task_exclusion_rejects_a_second_live_writer() {
        let (_dir, conn) = migrated_db();
        conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t1', 'ocr', 'a1', 'pending', 1, 1)",
            [],
        )
        .expect("first admission");
        let second = conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t2', 'ocr', 'a1', 'pending', 1, 1)",
            [],
        );
        assert!(
            second.is_err(),
            "two live rows for one operation+asset would let two workers compute twice"
        );
    }

    #[test]
    fn source_text_writes_bump_the_asset_revision() {
        let (_dir, conn) = migrated_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a1', 'i1', 'p', 'image', 1)",
            [],
        )
        .expect("asset");
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('e1', 'a1', 'hola', 'ocr', 1)",
            [],
        )
        .expect("extraction");
        let rev: i64 = conn
            .query_row(
                "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("revision bumped by the trigger");
        assert_eq!(rev, 1);
        conn.execute(
            "UPDATE extractions SET text_content = 'hola mundo' WHERE id = 'e1'",
            [],
        )
        .expect("edit");
        let rev: i64 = conn
            .query_row(
                "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("revision bumped again");
        assert_eq!(rev, 2);
    }

    #[test]
    fn invalid_states_are_rejected_by_check_constraints() {
        let (_dir, conn) = migrated_db();
        let bad = conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t9', 'ocr', 'a9', 'finished', 1, 1)",
            [],
        );
        assert!(
            bad.is_err(),
            "'finished' is not a task state: the UI must never invent one"
        );
    }
}
