//! Durable storage for the writing workspace (plan-editor.md §9).
//!
//! The canonical manuscript is the ProseMirror JSON in
//! `writing_documents.current_content_json`. The citation tables are
//! projections of the current revision (§8.4): this module rewrites them in
//! the same transaction that advances the content, so they can never describe
//! a revision the document no longer has.
//!
//! Every mutation runs on a connection from
//! [`crate::db::open::open_archive_connection`], which is the only place that
//! enables WAL, `foreign_keys` and `synchronous=FULL`.
//!
//! Concurrency follows the lease pattern the batch queue already proved
//! (`processing::repository`): a conditional `UPDATE ... WHERE revision = ?`
//! whose affected-row count is checked. The generic `db_execute_transaction`
//! IPC cannot express this — it discards every statement's row count, so a
//! zero-row conditional update commits as a silent no-op. That is why the
//! manuscript is owned here rather than through `packages/store`.

use rusqlite::Connection;

/// Migration that creates the writing tables. The frontend `runMigrations()`
/// applies it; the backend never runs DDL, it only verifies presence.
pub const MIGRATION_NAME: &str = "0035_writing_workspace";

/// The schema is not applied yet. Recoverable: migrations may still be running.
pub const SCHEMA_NOT_READY: &str = "schema_not_ready";
/// The save carried a `expected_revision` the document has moved past.
pub const REVISION_CONFLICT: &str = "revision_conflict";
/// No document with that id.
pub const DOCUMENT_NOT_FOUND: &str = "document_not_found";

/// A coded error, so the frontend can branch on `code` instead of matching
/// message text. Mirrors `processing::scheduler::ExecOutput`'s vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WritingError {
    pub code: String,
    pub message: String,
}

impl WritingError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn sql(context: &str, err: rusqlite::Error) -> Self {
        Self::new("sql_error", format!("{context}: {err}"))
    }
}

pub type WritingResult<T> = Result<T, WritingError>;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DocumentRow {
    pub id: String,
    pub title: String,
    pub document_type: String,
    pub status: String,
    pub schema_version: i64,
    pub current_content_json: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewDocument {
    pub id: String,
    pub title: String,
    pub document_type: String,
    pub schema_version: i64,
    pub content_json: String,
}

/// One row of the document-citation projection (§9.4). Corpus ids are plain
/// snapshots, never foreign keys: a citation outlives its source.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DocumentCitationInput {
    pub id: String,
    pub citation_node_id: String,
    pub collection_id: Option<String>,
    pub item_id: Option<String>,
    pub asset_id: Option<String>,
    pub page_number: Option<i64>,
    pub start_char: Option<i64>,
    pub end_char: Option<i64>,
    pub quoted_text: Option<String>,
    pub source_text_hash: Option<String>,
    pub metadata_snapshot_json: Option<String>,
}

/// One appended provenance event (§9.6). Append-only: never rewritten by a
/// later save, and never cleared when the text it describes is edited away.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProvenanceEventInput {
    pub id: String,
    pub origin_type: String,
    pub operation_type: String,
    pub range_anchor_json: Option<String>,
    pub source_reference_json: Option<String>,
    pub model_provider: Option<String>,
    pub model_name: Option<String>,
}

/// A save carries the revision it believes it is replacing. Content,
/// projections and new provenance commit together or not at all (§8.4).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SaveDocument {
    pub document_id: String,
    pub expected_revision: i64,
    pub content_json: String,
    pub schema_version: i64,
    pub plain_text_cache: Option<String>,
    #[serde(default)]
    pub citations: Vec<DocumentCitationInput>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEventInput>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// True when `_migrations` records `name`. A missing tracking table means no
/// migration has ever run here, which is also "not applied", not an error.
pub fn is_migration_applied(conn: &Connection, name: &str) -> WritingResult<bool> {
    let exists: Result<i64, rusqlite::Error> = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_migrations'",
        [],
        |row| row.get(0),
    );
    match exists {
        Ok(0) => Ok(false),
        Err(e) => Err(WritingError::sql("Failed to probe _migrations", e)),
        Ok(_) => conn
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE name = ?1",
                rusqlite::params![name],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(|e| WritingError::sql("Failed to read _migrations", e)),
    }
}

/// The writing tables are usable on this connection.
pub fn is_schema_ready(conn: &Connection) -> WritingResult<bool> {
    is_migration_applied(conn, MIGRATION_NAME)
}

fn require_schema(conn: &Connection) -> WritingResult<()> {
    if is_schema_ready(conn)? {
        Ok(())
    } else {
        Err(WritingError::new(
            SCHEMA_NOT_READY,
            "the writing schema has not been migrated yet",
        ))
    }
}

pub fn create_document(conn: &Connection, input: NewDocument) -> WritingResult<DocumentRow> {
    require_schema(conn)?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO writing_documents
           (id, title, document_type, status, schema_version, current_content_json,
            revision, bibliography_enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'active', ?4, ?5, 0, 1, ?6, ?6)",
        rusqlite::params![
            input.id,
            input.title,
            input.document_type,
            input.schema_version,
            input.content_json,
            now
        ],
    )
    .map_err(|e| WritingError::sql("Failed to create document", e))?;
    load_document(conn, &input.id)
}

pub fn load_document(conn: &Connection, id: &str) -> WritingResult<DocumentRow> {
    require_schema(conn)?;
    conn.query_row(
        "SELECT id, title, document_type, status, schema_version, current_content_json,
                revision, created_at, updated_at
           FROM writing_documents WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(DocumentRow {
                id: row.get(0)?,
                title: row.get(1)?,
                document_type: row.get(2)?,
                status: row.get(3)?,
                schema_version: row.get(4)?,
                current_content_json: row.get(5)?,
                revision: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            WritingError::new(DOCUMENT_NOT_FOUND, format!("no document with id {id}"))
        }
        other => WritingError::sql("Failed to load document", other),
    })
}

/// Advances the manuscript to the next revision, rewriting its projections and
/// appending provenance in the same transaction.
///
/// The conditional `UPDATE` is the whole point: if the document has moved past
/// `expected_revision`, zero rows change, nothing is written, and the caller
/// gets [`REVISION_CONFLICT`] with the revision that actually holds.
pub fn save_document(conn: &mut Connection, save: SaveDocument) -> WritingResult<i64> {
    require_schema(conn)?;
    let now = now_ms();
    let tx = conn
        .transaction()
        .map_err(|e| WritingError::sql("Failed to open transaction", e))?;

    let changed = tx
        .execute(
            "UPDATE writing_documents
                SET current_content_json = ?1,
                    schema_version = ?2,
                    plain_text_cache = ?3,
                    revision = revision + 1,
                    updated_at = ?4
              WHERE id = ?5 AND revision = ?6",
            rusqlite::params![
                save.content_json,
                save.schema_version,
                save.plain_text_cache,
                now,
                save.document_id,
                save.expected_revision
            ],
        )
        .map_err(|e| WritingError::sql("Failed to advance revision", e))?;

    if changed == 0 {
        // Distinguish "someone else saved first" from "no such document"
        // before rolling back, so the caller gets an actionable code.
        let current: Option<i64> = tx
            .query_row(
                "SELECT revision FROM writing_documents WHERE id = ?1",
                rusqlite::params![save.document_id],
                |row| row.get(0),
            )
            .ok();
        return Err(match current {
            Some(actual) => WritingError::new(
                REVISION_CONFLICT,
                format!(
                    "document {} is at revision {actual}, the save expected {}",
                    save.document_id, save.expected_revision
                ),
            ),
            None => WritingError::new(
                DOCUMENT_NOT_FOUND,
                format!("no document with id {}", save.document_id),
            ),
        });
    }

    // Projections describe the current revision only, so they are replaced
    // wholesale rather than merged (§8.4).
    tx.execute(
        "DELETE FROM writing_document_citations WHERE document_id = ?1",
        rusqlite::params![save.document_id],
    )
    .map_err(|e| WritingError::sql("Failed to clear citation projection", e))?;

    for c in &save.citations {
        tx.execute(
            "INSERT INTO writing_document_citations
               (id, document_id, citation_node_id, collection_id, item_id, asset_id,
                page_number, start_char, end_char, quoted_text, source_text_hash,
                metadata_snapshot_json, integrity_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'valid', ?13, ?13)",
            rusqlite::params![
                c.id,
                save.document_id,
                c.citation_node_id,
                c.collection_id,
                c.item_id,
                c.asset_id,
                c.page_number,
                c.start_char,
                c.end_char,
                c.quoted_text,
                c.source_text_hash,
                c.metadata_snapshot_json
                    .clone()
                    .unwrap_or_else(|| "{}".to_string()),
                now
            ],
        )
        .map_err(|e| WritingError::sql("Failed to write citation projection", e))?;
    }

    // Provenance is a log, not a projection: only appended, never cleared.
    for p in &save.provenance {
        tx.execute(
            "INSERT INTO writing_provenance_events
               (id, document_id, range_anchor_json, origin_type, operation_type,
                source_reference_json, model_provider, model_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                p.id,
                save.document_id,
                p.range_anchor_json,
                p.origin_type,
                p.operation_type,
                p.source_reference_json,
                p.model_provider,
                p.model_name,
                now
            ],
        )
        .map_err(|e| WritingError::sql("Failed to append provenance event", e))?;
    }

    let new_revision = save.expected_revision + 1;
    tx.commit()
        .map_err(|e| WritingError::sql("Failed to commit save", e))?;
    Ok(new_revision)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mirrored migration file, exercised here so registry/file drift
    /// breaks a test instead of reaching a user database.
    const MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0035_writing_workspace.sql");

    fn migrated_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open_at(&dir.path().join("entropia.sqlite"));
        (dir, conn)
    }

    fn open_at(path: &std::path::Path) -> Connection {
        let fresh = !path.exists();
        let conn = crate::db::open::open_archive_connection(path).expect("open");
        if fresh {
            conn.execute_batch(
                "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
                 CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);",
            )
            .expect("minimal corpus");
            conn.execute_batch(&format!(
                "BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nINSERT INTO _migrations (name, applied_at) VALUES ('{MIGRATION_NAME}', 0);\nCOMMIT;"
            ))
            .expect("apply 0035");
        }
        conn
    }

    fn new_doc(id: &str) -> NewDocument {
        NewDocument {
            id: id.to_string(),
            title: "Articulo".into(),
            document_type: "article".into(),
            schema_version: 1,
            content_json: r#"{"type":"doc","content":[]}"#.into(),
        }
    }

    fn save_of(id: &str, expected: i64, content: &str) -> SaveDocument {
        SaveDocument {
            document_id: id.to_string(),
            expected_revision: expected,
            content_json: content.to_string(),
            schema_version: 1,
            plain_text_cache: None,
            citations: Vec::new(),
            provenance: Vec::new(),
        }
    }

    #[test]
    fn refuses_to_work_before_the_migration_is_applied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = crate::db::open::open_archive_connection(&dir.path().join("empty.sqlite"))
            .expect("open");
        let err = create_document(&conn, new_doc("d1")).expect_err("must refuse");
        assert_eq!(err.code, SCHEMA_NOT_READY);
    }

    #[test]
    fn a_new_document_starts_at_revision_zero() {
        let (_dir, conn) = migrated_db();
        let doc = create_document(&conn, new_doc("d1")).expect("create");
        assert_eq!(doc.revision, 0);
        assert_eq!(doc.status, "active");
    }

    #[test]
    fn saving_advances_the_revision() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");

        let r1 =
            save_document(&mut conn, save_of("d1", 0, r#"{"type":"doc","v":1}"#)).expect("save");
        assert_eq!(r1, 1);
        let r2 =
            save_document(&mut conn, save_of("d1", 1, r#"{"type":"doc","v":2}"#)).expect("save");
        assert_eq!(r2, 2);

        let doc = load_document(&conn, "d1").expect("load");
        assert_eq!(doc.revision, 2);
        assert_eq!(doc.current_content_json, r#"{"type":"doc","v":2}"#);
    }

    #[test]
    fn a_stale_save_is_rejected_and_changes_nothing() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");
        save_document(&mut conn, save_of("d1", 0, r#"{"win":true}"#)).expect("first save wins");

        // A second window still believes it holds revision 0.
        let err = save_document(&mut conn, save_of("d1", 0, r#"{"lose":true}"#))
            .expect_err("stale save must fail");
        assert_eq!(err.code, REVISION_CONFLICT);
        assert!(err.message.contains("revision 1"), "got: {}", err.message);

        let doc = load_document(&conn, "d1").expect("load");
        assert_eq!(doc.revision, 1, "the losing save must not advance anything");
        assert_eq!(
            doc.current_content_json, r#"{"win":true}"#,
            "the winning content must survive untouched"
        );
    }

    #[test]
    fn saving_an_unknown_document_is_not_a_conflict() {
        let (_dir, mut conn) = migrated_db();
        let err = save_document(&mut conn, save_of("ghost", 0, "{}")).expect_err("must fail");
        assert_eq!(err.code, DOCUMENT_NOT_FOUND);
    }

    #[test]
    fn content_projections_and_provenance_commit_together() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");

        let mut save = save_of("d1", 0, r#"{"type":"doc","v":1}"#);
        save.citations.push(DocumentCitationInput {
            id: "c1".into(),
            citation_node_id: "node-1".into(),
            asset_id: Some("a1".into()),
            page_number: Some(17),
            ..Default::default()
        });
        save.provenance.push(ProvenanceEventInput {
            id: "p1".into(),
            origin_type: "corpus".into(),
            operation_type: "insert".into(),
            range_anchor_json: None,
            source_reference_json: None,
            model_provider: None,
            model_name: None,
        });
        save_document(&mut conn, save).expect("save");

        let citations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM writing_document_citations WHERE document_id = 'd1'",
                [],
                |r| r.get(0),
            )
            .expect("count citations");
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM writing_provenance_events WHERE document_id = 'd1'",
                [],
                |r| r.get(0),
            )
            .expect("count events");
        assert_eq!(citations, 1);
        assert_eq!(events, 1);
    }

    #[test]
    fn a_failed_save_leaves_neither_content_nor_projections() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");

        let mut save = save_of("d1", 0, r#"{"poisoned":true}"#);
        // Two citations sharing one node id violate the projection's unique
        // index, so the insert fails after the revision has been bumped.
        for id in ["c1", "c2"] {
            save.citations.push(DocumentCitationInput {
                id: id.into(),
                citation_node_id: "same-node".into(),
                ..Default::default()
            });
        }
        let err = save_document(&mut conn, save).expect_err("must fail");
        assert_eq!(err.code, "sql_error", "got: {err:?}");

        let doc = load_document(&conn, "d1").expect("load");
        assert_eq!(
            doc.revision, 0,
            "a failed save must not advance the revision"
        );
        assert_eq!(doc.current_content_json, r#"{"type":"doc","content":[]}"#);
        let citations: i64 = conn
            .query_row("SELECT COUNT(*) FROM writing_document_citations", [], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(citations, 0, "no projection row may survive a failed save");
    }

    #[test]
    fn projections_are_replaced_wholesale_not_merged() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");

        let mut first = save_of("d1", 0, "{}");
        first.citations.push(DocumentCitationInput {
            id: "c1".into(),
            citation_node_id: "node-1".into(),
            ..Default::default()
        });
        save_document(&mut conn, first).expect("first");

        // The user deleted that citation and added another.
        let mut second = save_of("d1", 1, "{}");
        second.citations.push(DocumentCitationInput {
            id: "c2".into(),
            citation_node_id: "node-2".into(),
            ..Default::default()
        });
        save_document(&mut conn, second).expect("second");

        let nodes: Vec<String> = conn
            .prepare(
                "SELECT citation_node_id FROM writing_document_citations WHERE document_id='d1'",
            )
            .and_then(|mut s| {
                s.query_map([], |r| r.get::<_, String>(0))
                    .map(|rows| rows.filter_map(Result::ok).collect())
            })
            .expect("query");
        assert_eq!(nodes, vec!["node-2".to_string()]);
    }

    #[test]
    fn provenance_survives_a_later_save_that_drops_the_text() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");

        let mut first = save_of("d1", 0, "{}");
        first.provenance.push(ProvenanceEventInput {
            id: "p1".into(),
            origin_type: "agent".into(),
            operation_type: "rewrite".into(),
            range_anchor_json: None,
            source_reference_json: None,
            model_provider: Some("openrouter".into()),
            model_name: Some("some-model".into()),
        });
        save_document(&mut conn, first).expect("first");

        save_document(&mut conn, save_of("d1", 1, "{}")).expect("second save carries no events");

        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM writing_provenance_events", [], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(events, 1, "provenance is a log, not a projection");
    }

    #[test]
    fn a_confirmed_revision_survives_reopening_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("entropia.sqlite");
        {
            let mut conn = open_at(&path);
            create_document(&conn, new_doc("d1")).expect("create");
            save_document(&mut conn, save_of("d1", 0, r#"{"kept":true}"#)).expect("save");
        }
        let reopened = open_at(&path);
        let doc = load_document(&reopened, "d1").expect("load after reopen");
        assert_eq!(doc.revision, 1);
        assert_eq!(doc.current_content_json, r#"{"kept":true}"#);
    }
}
