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

    pub(super) fn sql(context: &str, err: rusqlite::Error) -> Self {
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

/// One row of the Zotero-citation projection (§9.5).
///
/// Like the corpus one, this is derived from the manuscript and replaced
/// wholesale on every save. `item_key` is a snapshot without a foreign key —
/// there is nothing in this database to point at, and §11.2 forbids re-linking
/// a citation by key match anyway.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ZoteroCitationInput {
    pub id: String,
    pub citation_node_id: String,
    pub citation_cluster_id: String,
    pub item_position: i64,
    pub library_type: String,
    pub library_id: String,
    pub item_key: String,
    pub item_version: Option<i64>,
    pub locator_type: Option<String>,
    pub locator: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub suppress_author: bool,
    pub author_only: bool,
    pub item_csl_json_snapshot: Option<String>,
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
    pub zotero_citations: Vec<ZoteroCitationInput>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEventInput>,
}

pub(super) fn now_ms() -> i64 {
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

pub(super) fn require_schema(conn: &Connection) -> WritingResult<()> {
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

    tx.execute(
        "DELETE FROM writing_zotero_citations WHERE document_id = ?1",
        rusqlite::params![save.document_id],
    )
    .map_err(|e| WritingError::sql("Failed to clear Zotero citation projection", e))?;

    for z in &save.zotero_citations {
        tx.execute(
            "INSERT INTO writing_zotero_citations
               (id, document_id, citation_node_id, citation_cluster_id, item_position,
                source_origin, library_type, library_id, item_key, item_version,
                locator_type, locator, prefix, suffix, suppress_author, author_only,
                item_csl_json_snapshot, integrity_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'local', ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, 'valid', ?17, ?17)",
            rusqlite::params![
                z.id,
                save.document_id,
                z.citation_node_id,
                z.citation_cluster_id,
                z.item_position,
                z.library_type,
                z.library_id,
                z.item_key,
                z.item_version,
                z.locator_type,
                z.locator,
                z.prefix,
                z.suffix,
                z.suppress_author as i64,
                z.author_only as i64,
                z.item_csl_json_snapshot
                    .clone()
                    .unwrap_or_else(|| "{}".to_string()),
                now
            ],
        )
        .map_err(|e| WritingError::sql("Failed to write Zotero citation projection", e))?;
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

/// A status outside the schema's CHECK. Caught before touching the database so
/// the caller gets an actionable code instead of a constraint message.
pub const INVALID_STATUS: &str = "invalid_status";

/// The statuses §9.1 defines. Kept beside the migration's CHECK on purpose:
/// if one changes, the other must, and the tests exercise both.
pub const DOCUMENT_STATUSES: &[&str] = &["active", "archived", "trashed"];

/// Renames a document. A title is metadata, not content, so this deliberately
/// does **not** advance `revision`: renaming while an edit is in flight must
/// not turn that edit into a conflict.
pub fn rename_document(conn: &Connection, id: &str, title: &str) -> WritingResult<()> {
    require_schema(conn)?;
    let changed = conn
        .execute(
            "UPDATE writing_documents SET title = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![title, now_ms(), id],
        )
        .map_err(|e| WritingError::sql("Failed to rename document", e))?;
    if changed == 0 {
        return Err(WritingError::new(
            DOCUMENT_NOT_FOUND,
            format!("no document with id {id}"),
        ));
    }
    Ok(())
}

/// Moves a document between active, archived and trashed (§9.1). Like a
/// rename, this is metadata and leaves `revision` alone.
pub fn set_status(conn: &Connection, id: &str, status: &str) -> WritingResult<()> {
    require_schema(conn)?;
    if !DOCUMENT_STATUSES.contains(&status) {
        return Err(WritingError::new(
            INVALID_STATUS,
            format!(
                "unknown status {status:?}; expected one of {}",
                DOCUMENT_STATUSES.join(", ")
            ),
        ));
    }
    let changed = conn
        .execute(
            "UPDATE writing_documents SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![status, now_ms(), id],
        )
        .map_err(|e| WritingError::sql("Failed to set status", e))?;
    if changed == 0 {
        return Err(WritingError::new(
            DOCUMENT_NOT_FOUND,
            format!("no document with id {id}"),
        ));
    }
    Ok(())
}

/// Duplicates a document per §8.4: the copy gets its own document identity and
/// its own citation occurrences, records where it came from, and shares
/// nothing that belongs to the original's editing session.
///
/// Carried over: content, bibliography settings, collection associations, and
/// both citation projections — with fresh row ids, so the copy's occurrences
/// are its own while still pointing at the same sources.
///
/// Deliberately not carried over: version history (the copy starts its own),
/// and pending agent suggestions (each is pinned to the revision and anchors
/// that produced it, so it means nothing in a different document).
pub fn duplicate_document(
    conn: &mut Connection,
    source_id: &str,
    new_id: &str,
    new_title: &str,
) -> WritingResult<DocumentRow> {
    require_schema(conn)?;
    let now = now_ms();
    let tx = conn
        .transaction()
        .map_err(|e| WritingError::sql("Failed to open transaction", e))?;

    let inserted = tx
        .execute(
            "INSERT INTO writing_documents
               (id, title, document_type, status, schema_version, current_content_json,
                revision, plain_text_cache, citation_style_id, citation_locale,
                bibliography_enabled, created_at, updated_at)
             SELECT ?1, ?2, document_type, 'active', schema_version, current_content_json,
                    0, plain_text_cache, citation_style_id, citation_locale,
                    bibliography_enabled, ?3, ?3
               FROM writing_documents WHERE id = ?4",
            rusqlite::params![new_id, new_title, now, source_id],
        )
        .map_err(|e| WritingError::sql("Failed to duplicate document", e))?;
    if inserted == 0 {
        return Err(WritingError::new(
            DOCUMENT_NOT_FOUND,
            format!("no document with id {source_id}"),
        ));
    }

    tx.execute(
        "INSERT INTO writing_document_collections (document_id, collection_id, is_primary, created_at)
         SELECT ?1, collection_id, is_primary, ?2
           FROM writing_document_collections WHERE document_id = ?3",
        rusqlite::params![new_id, now, source_id],
    )
    .map_err(|e| WritingError::sql("Failed to copy collection associations", e))?;

    // New occurrence identities, same sources. The node ids stay as they are:
    // they address nodes inside the copied content, and uniqueness is per
    // document, so the copy's nodes keep pointing at the right citations.
    copy_document_citations(&tx, source_id, new_id, now)?;
    copy_zotero_citations(&tx, source_id, new_id, now)?;

    tx.execute(
        "INSERT INTO writing_provenance_events
           (id, document_id, origin_type, operation_type, source_reference_json, created_at)
         VALUES (?1, ?2, 'import', 'other', ?3, ?4)",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            new_id,
            format!("{{\"duplicated_from\":\"{source_id}\"}}"),
            now
        ],
    )
    .map_err(|e| WritingError::sql("Failed to record the duplication", e))?;

    tx.commit()
        .map_err(|e| WritingError::sql("Failed to commit duplication", e))?;
    load_document(conn, new_id)
}

fn copy_document_citations(
    tx: &rusqlite::Transaction<'_>,
    source_id: &str,
    new_id: &str,
    now: i64,
) -> WritingResult<()> {
    let mut stmt = tx
        .prepare(
            "SELECT citation_node_id, collection_id, item_id, asset_id, page_number,
                    start_char, end_char, source_region_json, quoted_text, source_text_hash,
                    locator_json, metadata_snapshot_json, integrity_status
               FROM writing_document_citations WHERE document_id = ?1",
        )
        .map_err(|e| WritingError::sql("Failed to read citation projection", e))?;
    let rows: Vec<Vec<rusqlite::types::Value>> = stmt
        .query_map(rusqlite::params![source_id], |row| {
            (0..13).map(|i| row.get(i)).collect()
        })
        .and_then(|rows| rows.collect())
        .map_err(|e| WritingError::sql("Failed to read citation projection", e))?;
    drop(stmt);

    for r in rows {
        tx.execute(
            "INSERT INTO writing_document_citations
               (id, document_id, citation_node_id, collection_id, item_id, asset_id,
                page_number, start_char, end_char, source_region_json, quoted_text,
                source_text_hash, locator_json, metadata_snapshot_json, integrity_status,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                new_id,
                r[0],
                r[1],
                r[2],
                r[3],
                r[4],
                r[5],
                r[6],
                r[7],
                r[8],
                r[9],
                r[10],
                r[11],
                r[12],
                now
            ],
        )
        .map_err(|e| WritingError::sql("Failed to copy citation projection", e))?;
    }
    Ok(())
}

fn copy_zotero_citations(
    tx: &rusqlite::Transaction<'_>,
    source_id: &str,
    new_id: &str,
    now: i64,
) -> WritingResult<()> {
    let mut stmt = tx
        .prepare(
            "SELECT citation_node_id, citation_cluster_id, item_position, source_origin,
                    source_instance_id, library_type, library_id, item_key, item_version,
                    locator_type, locator, prefix, suffix, suppress_author, author_only,
                    item_csl_json_snapshot, integrity_status
               FROM writing_zotero_citations WHERE document_id = ?1",
        )
        .map_err(|e| WritingError::sql("Failed to read Zotero projection", e))?;
    let rows: Vec<Vec<rusqlite::types::Value>> = stmt
        .query_map(rusqlite::params![source_id], |row| {
            (0..17).map(|i| row.get(i)).collect()
        })
        .and_then(|rows| rows.collect())
        .map_err(|e| WritingError::sql("Failed to read Zotero projection", e))?;
    drop(stmt);

    for r in rows {
        tx.execute(
            "INSERT INTO writing_zotero_citations
               (id, document_id, citation_node_id, citation_cluster_id, item_position,
                source_origin, source_instance_id, library_type, library_id, item_key,
                item_version, locator_type, locator, prefix, suffix, suppress_author,
                author_only, item_csl_json_snapshot, integrity_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?20)",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                new_id,
                r[0],
                r[1],
                r[2],
                r[3],
                r[4],
                r[5],
                r[6],
                r[7],
                r[8],
                r[9],
                r[10],
                r[11],
                r[12],
                r[13],
                r[14],
                r[15],
                r[16],
                now
            ],
        )
        .map_err(|e| WritingError::sql("Failed to copy Zotero projection", e))?;
    }
    Ok(())
}

/// Documents for the section's list, newest activity first. `statuses` filters
/// by lifecycle state; an empty slice means every state.
pub fn list_documents(conn: &Connection, statuses: &[String]) -> WritingResult<Vec<DocumentRow>> {
    require_schema(conn)?;
    let (clause, params): (String, Vec<&dyn rusqlite::ToSql>) = if statuses.is_empty() {
        (String::new(), Vec::new())
    } else {
        let marks = (1..=statuses.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        (
            format!(" WHERE status IN ({marks})"),
            statuses.iter().map(|s| s as &dyn rusqlite::ToSql).collect(),
        )
    };

    let sql = format!(
        "SELECT id, title, document_type, status, schema_version, current_content_json,
                revision, created_at, updated_at
           FROM writing_documents{clause}
          ORDER BY updated_at DESC, title ASC"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| WritingError::sql("Failed to list documents", e))?;
    stmt.query_map(params.as_slice(), |row| {
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
    })
    .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
    .map_err(|e| WritingError::sql("Failed to list documents", e))
}

/// One manuscript that cites an asset, and how many times (§10.3).
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct AssetDependency {
    pub document_id: String,
    pub document_title: String,
    pub citation_count: i64,
}

/// Which manuscripts cite `asset_id`, so a deletion can say what it will cost.
///
/// §10.3 asks for the dependencies to be *announced* before the asset goes, not
/// for the deletion to be refused: §29.1 settled the policy as deletion with
/// confirmation and a preserved snapshot, which is why the citation keeps its
/// quoted text and metadata and why `asset_id` is a snapshot column with no
/// foreign key behind it. A citation outlives its source by design.
///
/// An absent writing schema is an empty answer rather than an error. Someone
/// who has never opened Escritura still deletes assets, and failing their
/// deletion because a table they never needed is missing would be absurd.
pub fn citations_for_asset(
    conn: &Connection,
    asset_id: &str,
) -> WritingResult<Vec<AssetDependency>> {
    if !is_schema_ready(conn)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT d.id, d.title, COUNT(c.id)
               FROM writing_document_citations c
               JOIN writing_documents d ON d.id = c.document_id
              WHERE c.asset_id = ?1
              GROUP BY d.id, d.title
              ORDER BY d.title",
        )
        .map_err(|e| WritingError::sql("Failed to prepare asset dependency query", e))?;

    let rows = stmt
        .query_map([asset_id], |row| {
            Ok(AssetDependency {
                document_id: row.get(0)?,
                document_title: row.get(1)?,
                citation_count: row.get(2)?,
            })
        })
        .map_err(|e| WritingError::sql("Failed to read asset dependencies", e))?;

    let mut dependencies = Vec::new();
    for row in rows {
        dependencies.push(row.map_err(|e| WritingError::sql("Failed to read a dependency", e))?);
    }
    Ok(dependencies)
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
            zotero_citations: Vec::new(),
            provenance: Vec::new(),
        }
    }

    fn zotero_cite(node: &str, key: &str, position: i64) -> ZoteroCitationInput {
        ZoteroCitationInput {
            id: node.to_string(),
            citation_node_id: node.to_string(),
            citation_cluster_id: format!("cluster-{node}"),
            item_position: position,
            library_type: "user".into(),
            library_id: "0".into(),
            item_key: key.to_string(),
            item_csl_json_snapshot: Some(r#"{"id":"X","type":"book"}"#.into()),
            ..Default::default()
        }
    }

    fn zotero_keys(conn: &Connection, document_id: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT item_key FROM writing_zotero_citations
                  WHERE document_id = ?1 ORDER BY item_key",
            )
            .expect("prepare");
        let rows = stmt
            .query_map([document_id], |row| row.get::<_, String>(0))
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    }

    /// §9.5 is a projection of the current revision, exactly like §9.4: replaced
    /// wholesale in the transaction that writes the content, so it can never
    /// describe a revision the document no longer has.
    #[test]
    fn the_zotero_projection_is_replaced_by_each_save() {
        let (dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("d1");
        let path = dir.path().join("entropia.sqlite");

        let mut first = save_of("d1", 0, r#"{"type":"doc","content":[]}"#);
        first.zotero_citations = vec![zotero_cite("z1", "AAAA1111", 0)];
        save_document(&mut open_at(&path), first).expect("first");
        assert_eq!(zotero_keys(&conn, "d1"), vec!["AAAA1111".to_string()]);

        // The second save cites a different work. The first row must go, not
        // accumulate: a projection that merges describes two revisions at once.
        let mut second = save_of("d1", 1, r#"{"type":"doc","content":[]}"#);
        second.zotero_citations = vec![zotero_cite("z2", "BBBB2222", 0)];
        save_document(&mut open_at(&path), second).expect("second");

        assert_eq!(zotero_keys(&conn, "d1"), vec!["BBBB2222".to_string()]);
    }

    /// Removing the last citation must leave no rows behind, or a manuscript
    /// that cites nothing still has a bibliography.
    #[test]
    fn a_save_with_no_zotero_citations_clears_the_projection() {
        let (dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("d1");
        let path = dir.path().join("entropia.sqlite");

        let mut first = save_of("d1", 0, r#"{"type":"doc","content":[]}"#);
        first.zotero_citations = vec![zotero_cite("z1", "AAAA1111", 0)];
        save_document(&mut open_at(&path), first).expect("first");

        save_document(&mut open_at(&path), save_of("d1", 1, r#"{"type":"doc","content":[]}"#))
            .expect("second");

        assert!(zotero_keys(&conn, "d1").is_empty());
    }

    /// A cluster is several works in one citation, ordered. The unique
    /// constraint is on (document, cluster, position), so both must be honoured.
    #[test]
    fn several_works_in_one_cluster_keep_their_order() {
        let (dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("d1");

        let mut save = save_of("d1", 0, r#"{"type":"doc","content":[]}"#);
        let mut a = zotero_cite("z1", "AAAA1111", 0);
        let mut b = zotero_cite("z1b", "BBBB2222", 1);
        a.citation_cluster_id = "cluster-1".into();
        b.citation_cluster_id = "cluster-1".into();
        save.zotero_citations = vec![a, b];
        save_document(&mut open_at(&dir.path().join("entropia.sqlite")), save).expect("save");

        let mut stmt = conn
            .prepare(
                "SELECT item_key FROM writing_zotero_citations
                  WHERE citation_cluster_id = 'cluster-1' ORDER BY item_position",
            )
            .expect("prepare");
        let keys: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect();

        assert_eq!(keys, vec!["AAAA1111".to_string(), "BBBB2222".to_string()]);
    }

    /// A citation on `asset_id`, so a dependency can be found.
    fn cite(node: &str, asset: &str) -> DocumentCitationInput {
        DocumentCitationInput {
            id: node.to_string(),
            citation_node_id: node.to_string(),
            asset_id: Some(asset.to_string()),
            ..Default::default()
        }
    }

    /// §10.3: a deletion has to announce what cites the asset before it runs.
    /// The policy settled in §29.1 is deletion with a preserved snapshot, not
    /// refusal, so the point of the query is the warning, never a veto.
    #[test]
    fn reports_which_manuscripts_cite_an_asset() {
        let (_dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("d1");
        create_document(&conn, new_doc("d2")).expect("d2");
        rename_document(&conn, "d1", "Primero").expect("rename d1");
        rename_document(&conn, "d2", "Segundo").expect("rename d2");

        let mut first = save_of("d1", 0, r#"{"type":"doc","content":[]}"#);
        first.citations = vec![cite("c1", "as1"), cite("c2", "as1")];
        save_document(&mut open_at(&_dir.path().join("entropia.sqlite")), first).expect("save d1");

        let mut second = save_of("d2", 0, r#"{"type":"doc","content":[]}"#);
        second.citations = vec![cite("c3", "as1"), cite("c4", "as2")];
        save_document(&mut open_at(&_dir.path().join("entropia.sqlite")), second).expect("save d2");

        let deps = citations_for_asset(&conn, "as1").expect("query");

        assert_eq!(
            deps,
            vec![
                AssetDependency {
                    document_id: "d1".into(),
                    document_title: "Primero".into(),
                    citation_count: 2,
                },
                AssetDependency {
                    document_id: "d2".into(),
                    document_title: "Segundo".into(),
                    citation_count: 1,
                },
            ]
        );
    }

    #[test]
    fn reports_nothing_for_an_asset_nobody_cites() {
        let (_dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("d1");

        assert_eq!(citations_for_asset(&conn, "as9").expect("query"), Vec::new());
    }

    /// Someone who has never opened Escritura still deletes assets. Failing
    /// their deletion because a table they never needed is missing would be
    /// absurd, so an absent schema is an empty answer rather than an error.
    #[test]
    fn reports_nothing_when_the_writing_schema_was_never_applied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn =
            crate::db::open::open_archive_connection(&dir.path().join("empty.sqlite")).expect("open");

        assert_eq!(citations_for_asset(&conn, "as1").expect("query"), Vec::new());
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

    // -- lifecycle: rename, status, duplicate --------------------------------

    #[test]
    fn listing_returns_documents_with_the_most_recent_activity_first() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");
        create_document(&conn, new_doc("d2")).expect("create");
        // Touching d1 makes it the most recent.
        save_document(&mut conn, save_of("d1", 0, "{}")).expect("save");

        let ids: Vec<String> = list_documents(&conn, &[])
            .expect("list")
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec!["d1".to_string(), "d2".to_string()]);
    }

    #[test]
    fn listing_filters_by_lifecycle_state() {
        let (_dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");
        create_document(&conn, new_doc("d2")).expect("create");
        set_status(&conn, "d2", "trashed").expect("trash");

        let active = list_documents(&conn, &["active".to_string()]).expect("list");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "d1");

        let trashed = list_documents(&conn, &["trashed".to_string()]).expect("list");
        assert_eq!(trashed.len(), 1);
        assert_eq!(trashed[0].id, "d2");

        assert_eq!(list_documents(&conn, &[]).expect("list").len(), 2);
    }

    #[test]
    fn renaming_touches_the_title_and_leaves_the_content_revision_alone() {
        let (_dir, mut conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");
        save_document(&mut conn, save_of("d1", 0, r#"{"v":1}"#)).expect("save");

        rename_document(&conn, "d1", "Otro titulo").expect("rename");

        let doc = load_document(&conn, "d1").expect("load");
        assert_eq!(doc.title, "Otro titulo");
        assert_eq!(
            doc.revision, 1,
            "a title is metadata: renaming must not invalidate an in-flight content save"
        );
        assert_eq!(doc.current_content_json, r#"{"v":1}"#);
    }

    #[test]
    fn renaming_an_unknown_document_is_reported() {
        let (_dir, conn) = migrated_db();
        let err = rename_document(&conn, "ghost", "x").expect_err("must fail");
        assert_eq!(err.code, DOCUMENT_NOT_FOUND);
    }

    #[test]
    fn a_document_moves_between_active_archived_and_trashed() {
        let (_dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");

        for target in ["archived", "trashed", "active"] {
            set_status(&conn, "d1", target).expect("set status");
            assert_eq!(load_document(&conn, "d1").expect("load").status, target);
        }
    }

    #[test]
    fn an_unknown_status_is_refused_by_the_schema() {
        let (_dir, conn) = migrated_db();
        create_document(&conn, new_doc("d1")).expect("create");
        let err = set_status(&conn, "d1", "inventado").expect_err("must fail");
        assert_eq!(err.code, INVALID_STATUS, "got: {err:?}");
        assert_eq!(load_document(&conn, "d1").expect("load").status, "active");
    }

    /// Fixture: a document carrying one of everything, so duplication can be
    /// checked field by field rather than by counting rows.
    fn populated_document(conn: &mut Connection, id: &str) {
        create_document(conn, new_doc(id)).expect("create");
        conn.execute(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1','Col',1,1)",
            [],
        )
        .ok();
        conn.execute(
            "INSERT INTO writing_document_collections (document_id, collection_id, is_primary, created_at)
             VALUES (?1,'c1',1,1)",
            rusqlite::params![id],
        )
        .expect("associate collection");

        let mut save = save_of(id, 0, r#"{"type":"doc","v":1}"#);
        save.citations.push(DocumentCitationInput {
            id: format!("{id}-cit"),
            citation_node_id: "node-1".into(),
            asset_id: Some("a1".into()),
            page_number: Some(17),
            ..Default::default()
        });
        save.provenance.push(ProvenanceEventInput {
            id: format!("{id}-prov"),
            origin_type: "corpus".into(),
            operation_type: "insert".into(),
            range_anchor_json: None,
            source_reference_json: None,
            model_provider: None,
            model_name: None,
        });
        save_document(conn, save).expect("save");

        conn.execute(
            "INSERT INTO writing_agent_suggestions
               (id, document_id, source_revision, selected_content_hash, action_type, status, created_at)
             VALUES (?1, ?2, 1, 'h', 'rewrite', 'pending', 1)",
            rusqlite::params![format!("{id}-sug"), id],
        )
        .expect("pending suggestion");
    }

    #[test]
    fn duplicating_gives_the_copy_its_own_identity_at_revision_zero() {
        let (_dir, mut conn) = migrated_db();
        populated_document(&mut conn, "d1");

        let copy = duplicate_document(&mut conn, "d1", "d2", "Copia").expect("duplicate");

        assert_eq!(copy.id, "d2");
        assert_eq!(copy.title, "Copia");
        assert_eq!(copy.revision, 0, "a copy starts its own revision history");
        assert_eq!(copy.current_content_json, r#"{"type":"doc","v":1}"#);
        assert_eq!(
            load_document(&conn, "d1").expect("load original").revision,
            1,
            "duplicating must not disturb the original"
        );
    }

    #[test]
    fn duplicating_carries_the_collection_associations() {
        let (_dir, mut conn) = migrated_db();
        populated_document(&mut conn, "d1");
        duplicate_document(&mut conn, "d1", "d2", "Copia").expect("duplicate");

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM writing_document_collections WHERE document_id='d2' AND collection_id='c1'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 1);
    }

    #[test]
    fn duplicating_gives_citations_new_row_ids_but_keeps_their_source() {
        let (_dir, mut conn) = migrated_db();
        populated_document(&mut conn, "d1");
        duplicate_document(&mut conn, "d1", "d2", "Copia").expect("duplicate");

        let (row_id, node_id, asset, page): (String, String, Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT id, citation_node_id, asset_id, page_number
                   FROM writing_document_citations WHERE document_id='d2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("copied citation");

        assert_ne!(
            row_id, "d1-cit",
            "the copy needs its own occurrence identity"
        );
        assert_eq!(node_id, "node-1", "the node it belongs to is unchanged");
        assert_eq!(
            asset.as_deref(),
            Some("a1"),
            "the link to the source survives"
        );
        assert_eq!(page, Some(17));
    }

    #[test]
    fn duplicating_does_not_carry_pending_suggestions() {
        let (_dir, mut conn) = migrated_db();
        populated_document(&mut conn, "d1");
        duplicate_document(&mut conn, "d1", "d2", "Copia").expect("duplicate");

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM writing_agent_suggestions WHERE document_id='d2'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            n, 0,
            "a suggestion is bound to the revision that produced it"
        );
    }

    #[test]
    fn duplicating_starts_a_fresh_history_and_records_its_origin() {
        let (_dir, mut conn) = migrated_db();
        populated_document(&mut conn, "d1");
        conn.execute(
            "INSERT INTO writing_document_versions
               (id, document_id, version_number, content_json, schema_version, document_settings_json, reason, content_hash, created_at)
             VALUES ('v1','d1',1,'{}',1,'{}','checkpoint','h',1)",
            [],
        )
        .expect("a version on the original");

        duplicate_document(&mut conn, "d1", "d2", "Copia").expect("duplicate");

        let versions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM writing_document_versions WHERE document_id='d2'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            versions, 0,
            "the copy does not inherit the original's history"
        );

        // §9.6's operation list ends in "u otra", so duplication rides the
        // sanctioned escape hatch rather than forcing a CHECK change on an
        // already-committed migration. The specifics live in the reference.
        let (origin_type, reference): (String, String) = conn
            .query_row(
                "SELECT origin_type, source_reference_json FROM writing_provenance_events
                  WHERE document_id='d2' AND operation_type='other'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("a provenance event naming the origin");
        assert_eq!(origin_type, "import");
        assert!(reference.contains("d1"), "got: {reference}");
    }

    #[test]
    fn duplicating_an_unknown_document_is_reported() {
        let (_dir, mut conn) = migrated_db();
        let err = duplicate_document(&mut conn, "ghost", "d2", "Copia").expect_err("must fail");
        assert_eq!(err.code, DOCUMENT_NOT_FOUND);
    }
}
