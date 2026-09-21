//! Transactional persistence for the E1b-1b Zotero catalog relations.
//!
//! A connection is a source namespace. A library qualifies native Zotero keys,
//! so the same key in two libraries remains two catalog rows. Native Zotero
//! JSON and CSL JSON are stored as separate snapshots; CSL `id` is never used
//! as a substitute for the native key. Tombstones are explicit side records;
//! they never remove snapshots or membership edges.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const MIGRATION_NAME: &str = "0039_bibliography_relations";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BibliographyError {
    pub code: String,
    pub message: String,
}

impl BibliographyError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn sql(context: &str, error: rusqlite::Error) -> Self {
        Self::new("sql_error", format!("{context}: {error}"))
    }
}

pub type BibliographyResult<T> = Result<T, BibliographyError>;

/// The provenance of a catalog connection. An instance id is evidence supplied
/// by the connector, never a value derived from a library version counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceOrigin {
    Local,
    Web,
}

impl SourceOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Web => "web",
        }
    }

    fn from_db(value: String) -> BibliographyResult<Self> {
        match value.as_str() {
            "local" => Ok(Self::Local),
            "web" => Ok(Self::Web),
            other => Err(BibliographyError::new(
                "invalid_storage",
                format!("unknown source origin {other:?}"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryType {
    User,
    Group,
}

impl LibraryType {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Group => "group",
        }
    }

    fn from_db(value: String) -> BibliographyResult<Self> {
        match value.as_str() {
            "user" => Ok(Self::User),
            "group" => Ok(Self::Group),
            other => Err(BibliographyError::new(
                "invalid_storage",
                format!("unknown library type {other:?}"),
            )),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertConnection {
    pub id: String,
    pub source_origin: SourceOrigin,
    #[serde(default)]
    pub source_instance_id: Option<String>,
    pub endpoint: Option<String>,
    #[serde(default = "empty_json_object")]
    pub capabilities_json: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroConnection {
    pub id: String,
    pub source_origin: SourceOrigin,
    pub source_instance_id: Option<String>,
    pub endpoint: Option<String>,
    pub capabilities_json: String,
    pub state: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertLibrary {
    pub connection_id: String,
    pub library_type: LibraryType,
    pub library_id: String,
    pub name: String,
    pub last_modified_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroLibrary {
    pub id: String,
    pub connection_id: String,
    pub library_type: LibraryType,
    pub library_id: String,
    pub name: String,
    pub last_modified_version: Option<i64>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographicItemInput {
    pub item_key: String,
    pub item_version: Option<i64>,
    pub native_json_snapshot: String,
    pub csl_json_snapshot: String,
    pub item_type: Option<String>,
    pub title: Option<String>,
    pub creators_json: Option<String>,
    pub publication_title: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub doi: Option<String>,
    pub isbn: Option<String>,
    pub abstract_text: Option<String>,
    pub language: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroIdentity {
    pub source_origin: SourceOrigin,
    pub source_instance_id: Option<String>,
    pub library_type: LibraryType,
    pub library_id: String,
    pub item_key: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BibliographicItem {
    pub id: String,
    /// Internal FK used by repository callers. `identity.library_id` is the
    /// native Zotero external library id shown at the boundary.
    pub library_row_id: String,
    pub identity: ZoteroIdentity,
    pub item_version: Option<i64>,
    pub native_json_snapshot: String,
    pub csl_json_snapshot: String,
    pub item_type: Option<String>,
    pub title: Option<String>,
    pub creators_json: Option<String>,
    pub publication_title: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub doi: Option<String>,
    pub isbn: Option<String>,
    pub abstract_text: Option<String>,
    pub language: Option<String>,
    pub url: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInput {
    pub collection_key: String,
    pub name: String,
    pub parent_collection_key: Option<String>,
    pub native_json_snapshot: String,
    pub native_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroCollection {
    pub id: String,
    pub library_row_id: String,
    pub library_id: String,
    pub collection_key: String,
    pub name: String,
    pub parent_collection_key: Option<String>,
    pub native_json_snapshot: String,
    pub native_version: Option<i64>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInput {
    pub tag_text: String,
    pub tag_type: Option<String>,
    pub native_json_snapshot: String,
    pub native_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroTag {
    pub id: String,
    pub library_row_id: String,
    pub library_id: String,
    pub tag_text: String,
    pub tag_type: Option<String>,
    pub native_json_snapshot: String,
    pub native_version: Option<i64>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInput {
    pub attachment_key: String,
    pub content_type: Option<String>,
    pub link_mode: Option<String>,
    pub filename: Option<String>,
    pub native_path: Option<String>,
    pub url: Option<String>,
    pub md5: Option<String>,
    pub mtime: Option<i64>,
    pub native_json_snapshot: String,
    pub native_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroAttachment {
    pub id: String,
    pub parent_item_id: String,
    pub attachment_key: String,
    pub content_type: Option<String>,
    pub link_mode: Option<String>,
    pub filename: Option<String>,
    pub native_path: Option<String>,
    pub url: Option<String>,
    pub md5: Option<String>,
    pub mtime: Option<i64>,
    pub native_json_snapshot: String,
    pub native_version: Option<i64>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemCollectionInput {
    pub item_id: String,
    pub collection_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroItemCollection {
    pub library_row_id: String,
    pub item_id: String,
    pub collection_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemTagInput {
    pub item_id: String,
    pub tag_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroItemTag {
    pub library_row_id: String,
    pub item_id: String,
    pub tag_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TombstoneInput {
    #[serde(default)]
    pub remote_version: Option<i64>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TombstoneRecord {
    pub observed_at: i64,
    pub remote_version: Option<i64>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Tombstoned<T> {
    pub entity: T,
    pub tombstone: TombstoneRecord,
}

fn empty_json_object() -> String {
    "{}".to_string()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn require_non_empty(value: &str, field: &str) -> BibliographyResult<()> {
    if value.trim().is_empty() {
        return Err(BibliographyError::new(
            "invalid_input",
            format!("{field} must not be empty"),
        ));
    }
    Ok(())
}

fn clear_tombstone(
    conn: &Connection,
    table: &str,
    entity_column: &str,
    entity_id: &str,
    context: &str,
) -> BibliographyResult<()> {
    let sql = format!("DELETE FROM {table} WHERE {entity_column} = ?1");
    conn.execute(&sql, [entity_id])
        .map(|_| ())
        .map_err(|error| BibliographyError::sql(context, error))
}

fn validate_tombstone_input(input: &TombstoneInput) -> BibliographyResult<()> {
    require_non_empty(&input.reason, "tombstone reason")
}

fn validate_json(value: &str, field: &str) -> BibliographyResult<()> {
    serde_json::from_str::<serde_json::Value>(value).map_err(|error| {
        BibliographyError::new(
            "invalid_json",
            format!("{field} is not valid JSON: {error}"),
        )
    })?;
    Ok(())
}

fn read_connection(conn: &Connection, id: &str) -> BibliographyResult<ZoteroConnection> {
    conn.query_row(
        "SELECT id, source_origin, source_instance_id, endpoint, capabilities_json,
                state, revision, created_at, updated_at
           FROM zotero_connections WHERE id = ?1",
        [id],
        |row| {
            let source_origin: String = row.get(1)?;
            Ok((
                row.get::<_, String>(0)?,
                source_origin,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        },
    )
    .map_err(|error| BibliographyError::sql("Failed to read Zotero connection", error))
    .and_then(
        |(
            id,
            source_origin,
            source_instance_id,
            endpoint,
            capabilities_json,
            state,
            revision,
            created_at,
            updated_at,
        )| {
            Ok(ZoteroConnection {
                id,
                source_origin: SourceOrigin::from_db(source_origin)?,
                source_instance_id,
                endpoint,
                capabilities_json,
                state,
                revision,
                created_at,
                updated_at,
            })
        },
    )
}

/// Creates or refreshes one connection identity in one transaction.
///
/// The caller supplies the connection id and any instance evidence. A missing
/// instance remains SQL NULL; no version header is consulted or synthesized.
pub fn upsert_connection(
    conn: &mut Connection,
    input: UpsertConnection,
) -> BibliographyResult<ZoteroConnection> {
    require_non_empty(&input.id, "connection id")?;
    validate_json(&input.capabilities_json, "capabilities_json")?;
    let now = now_ms();
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open connection transaction", error))?;

    tx.execute(
        "INSERT INTO zotero_connections
           (id, source_origin, source_instance_id, endpoint, capabilities_json,
            created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(id) DO UPDATE SET
           source_origin = excluded.source_origin,
           source_instance_id = excluded.source_instance_id,
           endpoint = excluded.endpoint,
           capabilities_json = excluded.capabilities_json,
           revision = zotero_connections.revision + 1,
           updated_at = excluded.updated_at
         WHERE zotero_connections.source_origin IS NOT excluded.source_origin
            OR zotero_connections.source_instance_id IS NOT excluded.source_instance_id
            OR zotero_connections.endpoint IS NOT excluded.endpoint
            OR zotero_connections.capabilities_json IS NOT excluded.capabilities_json",
        rusqlite::params![
            &input.id,
            input.source_origin.as_str(),
            input.source_instance_id.as_deref(),
            input.endpoint.as_deref(),
            &input.capabilities_json,
            now,
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert Zotero connection", error))?;

    let connection = read_connection(&tx, &input.id)?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit Zotero connection", error))?;
    Ok(connection)
}

fn read_library(
    conn: &Connection,
    connection_id: &str,
    library_type: LibraryType,
    library_id: &str,
) -> BibliographyResult<ZoteroLibrary> {
    conn.query_row(
        "SELECT id, connection_id, library_type, library_id, name,
                last_modified_version, revision, created_at, updated_at
           FROM zotero_libraries
          WHERE connection_id = ?1 AND library_type = ?2 AND library_id = ?3",
        rusqlite::params![connection_id, library_type.as_str(), library_id],
        |row| {
            let stored_type: String = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                stored_type,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        },
    )
    .map_err(|error| BibliographyError::sql("Failed to read Zotero library", error))
    .and_then(
        |(
            id,
            connection_id,
            stored_type,
            library_id,
            name,
            last_modified_version,
            revision,
            created_at,
            updated_at,
        )| {
            Ok(ZoteroLibrary {
                id,
                connection_id,
                library_type: LibraryType::from_db(stored_type)?,
                library_id,
                name,
                last_modified_version,
                revision,
                created_at,
                updated_at,
            })
        },
    )
}

/// Creates or refreshes one external library within a connection namespace.
pub fn upsert_library(
    conn: &mut Connection,
    input: UpsertLibrary,
) -> BibliographyResult<ZoteroLibrary> {
    require_non_empty(&input.connection_id, "connection id")?;
    require_non_empty(&input.library_id, "library id")?;
    require_non_empty(&input.name, "library name")?;
    let now = now_ms();
    let id = uuid::Uuid::new_v4().to_string();
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open library transaction", error))?;

    tx.execute(
        "INSERT INTO zotero_libraries
           (id, connection_id, library_type, library_id, name,
            last_modified_version, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(connection_id, library_type, library_id) DO UPDATE SET
           name = excluded.name,
           last_modified_version = excluded.last_modified_version,
           revision = zotero_libraries.revision + 1,
           updated_at = excluded.updated_at
         WHERE zotero_libraries.name IS NOT excluded.name
            OR zotero_libraries.last_modified_version IS NOT excluded.last_modified_version",
        rusqlite::params![
            &id,
            &input.connection_id,
            input.library_type.as_str(),
            &input.library_id,
            &input.name,
            input.last_modified_version,
            now,
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert Zotero library", error))?;

    let library = read_library(
        &tx,
        &input.connection_id,
        input.library_type,
        &input.library_id,
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit Zotero library", error))?;
    Ok(library)
}

pub(crate) fn read_item(
    conn: &Connection,
    library_row_id: &str,
    item_key: &str,
) -> BibliographyResult<BibliographicItem> {
    conn.query_row(
        "SELECT i.id, i.library_id, c.source_origin, c.source_instance_id,
                l.library_type, l.library_id, i.item_key, i.item_version,
                i.native_json_snapshot, i.csl_json_snapshot, i.item_type, i.title,
                i.creators_json, i.publication_title, i.publisher, i.date, i.doi,
                i.isbn, i.abstract, i.language, i.url, i.revision, i.created_at,
                i.updated_at, i.verified_at
           FROM bibliographic_items i
           JOIN zotero_libraries l ON l.id = i.library_id
           JOIN zotero_connections c ON c.id = l.connection_id
          WHERE i.library_id = ?1 AND i.item_key = ?2",
        rusqlite::params![library_row_id, item_key],
        |row| {
            let source_origin: String = row.get(2)?;
            let library_type: String = row.get(4)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                source_origin,
                row.get::<_, Option<String>>(3)?,
                library_type,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<String>>(16)?,
                row.get::<_, Option<String>>(17)?,
                row.get::<_, Option<String>>(18)?,
                row.get::<_, Option<String>>(19)?,
                row.get::<_, Option<String>>(20)?,
                row.get::<_, i64>(21)?,
                row.get::<_, i64>(22)?,
                row.get::<_, i64>(23)?,
                row.get::<_, i64>(24)?,
            ))
        },
    )
    .map_err(|error| BibliographyError::sql("Failed to read bibliographic item", error))
    .and_then(
        |(
            id,
            library_row_id,
            source_origin,
            source_instance_id,
            library_type,
            library_id,
            item_key,
            item_version,
            native_json_snapshot,
            csl_json_snapshot,
            item_type,
            title,
            creators_json,
            publication_title,
            publisher,
            date,
            doi,
            isbn,
            abstract_text,
            language,
            url,
            revision,
            created_at,
            updated_at,
            verified_at,
        )| {
            Ok(BibliographicItem {
                id,
                library_row_id,
                identity: ZoteroIdentity {
                    source_origin: SourceOrigin::from_db(source_origin)?,
                    source_instance_id,
                    library_type: LibraryType::from_db(library_type)?,
                    library_id,
                    item_key,
                },
                item_version,
                native_json_snapshot,
                csl_json_snapshot,
                item_type,
                title,
                creators_json,
                publication_title,
                publisher,
                date,
                doi,
                isbn,
                abstract_text,
                language,
                url,
                revision,
                created_at,
                updated_at,
                verified_at,
            })
        },
    )
}

/// Reads the confirmed local personal catalog for one external library id.
///
/// The E1b-3 seam is limited to the uninstanced local personal namespace;
/// group, web and non-null source-instance namespaces remain unavailable.
/// `None` means the catalog cannot be trusted for this request: the bibliography
/// tables are not installed, the external id is not exactly one eligible local
/// personal namespace, or the completed reconciliation cannot account for every
/// seen item and live row with an eligible snapshot or explicit tombstone.
/// `Some(vec![])` is a confirmed empty catalog and must not fall back to the
/// legacy filesystem mirror.
///
/// The connection state is intentionally not part of this query. A completed
/// local reconciliation remains useful while its connection is offline.
pub fn confirmed_local_personal_catalog(
    conn: &Connection,
    external_library_id: &str,
) -> BibliographyResult<Option<Vec<BibliographicItem>>> {
    require_non_empty(external_library_id, "external library id")?;

    let required_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM sqlite_master
              WHERE type = 'table'
                AND name IN (
                    'zotero_connections', 'zotero_libraries', 'bibliographic_items',
                    'zotero_reconciliation_runs', 'zotero_reconciliation_seen',
                    'zotero_item_tombstones'
                )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| BibliographyError::sql("Failed to inspect bibliography tables", error))?;
    if required_tables != 6 {
        return Ok(None);
    }

    let namespace_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM zotero_libraries l
               JOIN zotero_connections c ON c.id = l.connection_id
              WHERE c.source_origin = 'local'
                AND c.source_instance_id IS NULL
                AND l.library_type = 'user'
                AND l.library_id = ?1",
            [external_library_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            BibliographyError::sql("Failed to resolve Zotero library namespace", error)
        })?;
    if namespace_count != 1 {
        return Ok(None);
    }

    let library_row_id: String = conn
        .query_row(
            "SELECT l.id
               FROM zotero_libraries l
               JOIN zotero_connections c ON c.id = l.connection_id
              WHERE c.source_origin = 'local'
                AND c.source_instance_id IS NULL
                AND l.library_type = 'user'
                AND l.library_id = ?1",
            [external_library_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            BibliographyError::sql("Failed to read Zotero library namespace", error)
        })?;

    let (run_id, completed_at): (String, i64) = match conn
        .query_row(
            "SELECT run_id, completed_at
               FROM zotero_reconciliation_runs
              WHERE library_id = ?1
                AND state = 'completed'
                AND phase = 'finalize'
                AND completed_at IS NOT NULL",
            [&library_row_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| BibliographyError::sql("Failed to read confirmed reconciliation", error))?
    {
        Some(run) => run,
        None => return Ok(None),
    };

    let seen_item_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM zotero_reconciliation_seen
              WHERE library_id = ?1
                AND run_id = ?2
                AND entity_kind = 'item'",
            rusqlite::params![&library_row_id, &run_id],
            |row| row.get(0),
        )
        .map_err(|error| BibliographyError::sql("Failed to count confirmed Zotero items", error))?;

    let tombstoned_item_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM zotero_reconciliation_seen s
               JOIN bibliographic_items i
                 ON i.library_id = s.library_id
                AND i.item_key = s.entity_key
               JOIN zotero_item_tombstones t ON t.item_id = i.id
              WHERE s.library_id = ?1
                AND s.run_id = ?2
                AND s.entity_kind = 'item'",
            rusqlite::params![&library_row_id, &run_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            BibliographyError::sql("Failed to count tombstoned Zotero items", error)
        })?;

    let unseen_live_item_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM bibliographic_items i
               LEFT JOIN zotero_item_tombstones t ON t.item_id = i.id
               LEFT JOIN zotero_reconciliation_seen s
                 ON s.library_id = i.library_id
                AND s.run_id = ?2
                AND s.entity_kind = 'item'
                AND s.entity_key = i.item_key
              WHERE i.library_id = ?1
                AND t.item_id IS NULL
                AND s.entity_key IS NULL",
            rusqlite::params![&library_row_id, &run_id],
            |row| row.get(0),
        )
        .map_err(|error| BibliographyError::sql("Failed to count unseen Zotero items", error))?;

    let mut statement = conn
        .prepare(
            "SELECT i.item_key
               FROM bibliographic_items i
               JOIN zotero_reconciliation_seen s
                 ON s.library_id = i.library_id
                AND s.run_id = ?2
                AND s.entity_kind = 'item'
                AND s.entity_key = i.item_key
               LEFT JOIN zotero_item_tombstones t ON t.item_id = i.id
              WHERE i.library_id = ?1
                AND i.verified_at <= ?3
                AND i.item_version IS NOT NULL
                AND i.item_version >= 0
                AND (s.remote_version IS NULL OR s.remote_version = i.item_version)
                AND t.item_id IS NULL
              ORDER BY i.item_version DESC,
                       i.item_key ASC",
        )
        .map_err(|error| {
            BibliographyError::sql("Failed to prepare confirmed Zotero catalog", error)
        })?;
    let item_keys: Vec<String> = statement
        .query_map(
            rusqlite::params![&library_row_id, &run_id, completed_at],
            |row| row.get(0),
        )
        .map_err(|error| BibliographyError::sql("Failed to read confirmed Zotero catalog", error))?
        .collect::<Result<_, _>>()
        .map_err(|error| {
            BibliographyError::sql("Failed to decode confirmed Zotero catalog", error)
        })?;

    // Every seen item must be represented by either an eligible live snapshot
    // or an explicit tombstone. A live row outside the seen-set is likewise
    // evidence that this completed run cannot safely describe the namespace.
    if unseen_live_item_count > 0
        || item_keys.len() as i64 + tombstoned_item_count < seen_item_count
    {
        return Ok(None);
    }

    let items = item_keys
        .into_iter()
        .map(|item_key| read_item(conn, &library_row_id, &item_key))
        .collect::<BibliographyResult<Vec<_>>>()?;
    Ok(Some(items))
}

/// Inserts or refreshes one work, qualified by the internal library FK and the
/// native Zotero key. Repeating an unchanged payload is a no-op; a changed
/// payload increments the local revision without creating a second row.
pub fn upsert_item(
    conn: &mut Connection,
    library_row_id: &str,
    input: BibliographicItemInput,
) -> BibliographyResult<BibliographicItem> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(&input.item_key, "item key")?;
    validate_json(&input.native_json_snapshot, "native_json_snapshot")?;
    validate_json(&input.csl_json_snapshot, "csl_json_snapshot")?;
    if let Some(creators_json) = input.creators_json.as_deref() {
        validate_json(creators_json, "creators_json")?;
    }
    let now = now_ms();
    let id = uuid::Uuid::new_v4().to_string();
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open item transaction", error))?;

    tx.execute(
        "INSERT INTO bibliographic_items
           (id, library_id, item_key, item_version, native_json_snapshot,
            csl_json_snapshot, item_type, title, creators_json, publication_title,
            publisher, date, doi, isbn, abstract, language, url,
            created_at, updated_at, verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?18, ?18)
         ON CONFLICT(library_id, item_key) DO UPDATE SET
            item_version = excluded.item_version,
            native_json_snapshot = excluded.native_json_snapshot,
            csl_json_snapshot = excluded.csl_json_snapshot,
            item_type = excluded.item_type,
            title = excluded.title,
            creators_json = excluded.creators_json,
            publication_title = excluded.publication_title,
            publisher = excluded.publisher,
            date = excluded.date,
            doi = excluded.doi,
            isbn = excluded.isbn,
            abstract = excluded.abstract,
            language = excluded.language,
            url = excluded.url,
            revision = bibliographic_items.revision + 1,
            updated_at = excluded.updated_at,
            verified_at = excluded.verified_at
         WHERE bibliographic_items.item_version IS NOT excluded.item_version
            OR bibliographic_items.native_json_snapshot IS NOT excluded.native_json_snapshot
            OR bibliographic_items.csl_json_snapshot IS NOT excluded.csl_json_snapshot
            OR bibliographic_items.item_type IS NOT excluded.item_type
            OR bibliographic_items.title IS NOT excluded.title
            OR bibliographic_items.creators_json IS NOT excluded.creators_json
            OR bibliographic_items.publication_title IS NOT excluded.publication_title
            OR bibliographic_items.publisher IS NOT excluded.publisher
            OR bibliographic_items.date IS NOT excluded.date
            OR bibliographic_items.doi IS NOT excluded.doi
            OR bibliographic_items.isbn IS NOT excluded.isbn
            OR bibliographic_items.abstract IS NOT excluded.abstract
            OR bibliographic_items.language IS NOT excluded.language
            OR bibliographic_items.url IS NOT excluded.url",
        rusqlite::params![
            &id,
            library_row_id,
            &input.item_key,
            input.item_version,
            &input.native_json_snapshot,
            &input.csl_json_snapshot,
            input.item_type.as_deref(),
            input.title.as_deref(),
            input.creators_json.as_deref(),
            input.publication_title.as_deref(),
            input.publisher.as_deref(),
            input.date.as_deref(),
            input.doi.as_deref(),
            input.isbn.as_deref(),
            input.abstract_text.as_deref(),
            input.language.as_deref(),
            input.url.as_deref(),
            now,
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert bibliographic item", error))?;

    let item = read_item(&tx, library_row_id, &input.item_key)?;
    clear_tombstone(
        &tx,
        "zotero_item_tombstones",
        "item_id",
        &item.id,
        "Failed to clear bibliographic item tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit bibliographic item", error))?;
    Ok(item)
}

fn read_collection(
    conn: &Connection,
    library_row_id: &str,
    collection_key: &str,
) -> BibliographyResult<ZoteroCollection> {
    conn.query_row(
        "SELECT c.id, c.library_id, l.library_id, c.collection_key, c.name,
                c.parent_collection_key, c.native_json_snapshot, c.native_version,
                c.revision, c.created_at, c.updated_at, c.verified_at
           FROM zotero_collections c
           JOIN zotero_libraries l ON l.id = c.library_id
          WHERE c.library_id = ?1 AND c.collection_key = ?2",
        rusqlite::params![library_row_id, collection_key],
        |row| {
            Ok(ZoteroCollection {
                id: row.get(0)?,
                library_row_id: row.get(1)?,
                library_id: row.get(2)?,
                collection_key: row.get(3)?,
                name: row.get(4)?,
                parent_collection_key: row.get(5)?,
                native_json_snapshot: row.get(6)?,
                native_version: row.get(7)?,
                revision: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                verified_at: row.get(11)?,
            })
        },
    )
    .map_err(|error| BibliographyError::sql("Failed to read Zotero collection", error))
}

/// Creates or refreshes one native collection. The parent key is stored as
/// opaque source metadata, so a child can arrive before its parent.
pub fn upsert_collection(
    conn: &mut Connection,
    library_row_id: &str,
    input: CollectionInput,
) -> BibliographyResult<ZoteroCollection> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(&input.collection_key, "collection key")?;
    require_non_empty(&input.name, "collection name")?;
    validate_json(&input.native_json_snapshot, "native_json_snapshot")?;
    let now = now_ms();
    let id = uuid::Uuid::new_v4().to_string();
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open collection transaction", error))?;

    tx.execute(
        "INSERT INTO zotero_collections
           (id, library_id, collection_key, name, parent_collection_key,
            native_json_snapshot, native_version, created_at, updated_at, verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?8)
         ON CONFLICT(library_id, collection_key) DO UPDATE SET
            name = excluded.name,
            parent_collection_key = excluded.parent_collection_key,
            native_json_snapshot = excluded.native_json_snapshot,
            native_version = excluded.native_version,
            revision = zotero_collections.revision + 1,
            updated_at = excluded.updated_at,
            verified_at = excluded.verified_at
         WHERE zotero_collections.name IS NOT excluded.name
            OR zotero_collections.parent_collection_key IS NOT excluded.parent_collection_key
            OR zotero_collections.native_json_snapshot IS NOT excluded.native_json_snapshot
            OR zotero_collections.native_version IS NOT excluded.native_version",
        rusqlite::params![
            &id,
            library_row_id,
            &input.collection_key,
            &input.name,
            input.parent_collection_key.as_deref(),
            &input.native_json_snapshot,
            input.native_version,
            now,
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert Zotero collection", error))?;

    let collection = read_collection(&tx, library_row_id, &input.collection_key)?;
    clear_tombstone(
        &tx,
        "zotero_collection_tombstones",
        "collection_id",
        &collection.id,
        "Failed to clear Zotero collection tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit Zotero collection", error))?;
    Ok(collection)
}

fn read_tag(
    conn: &Connection,
    library_row_id: &str,
    tag_text: &str,
) -> BibliographyResult<ZoteroTag> {
    conn.query_row(
        "SELECT t.id, t.library_id, l.library_id, t.tag_text, t.tag_type,
                t.native_json_snapshot, t.native_version, t.revision, t.created_at,
                t.updated_at, t.verified_at
           FROM zotero_tags t
           JOIN zotero_libraries l ON l.id = t.library_id
          WHERE t.library_id = ?1 AND t.tag_text = ?2",
        rusqlite::params![library_row_id, tag_text],
        |row| {
            Ok(ZoteroTag {
                id: row.get(0)?,
                library_row_id: row.get(1)?,
                library_id: row.get(2)?,
                tag_text: row.get(3)?,
                tag_type: row.get(4)?,
                native_json_snapshot: row.get(5)?,
                native_version: row.get(6)?,
                revision: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                verified_at: row.get(10)?,
            })
        },
    )
    .map_err(|error| BibliographyError::sql("Failed to read Zotero tag", error))
}

/// Creates or refreshes one exact native tag string. No case folding,
/// trimming, or other normalization is applied to the identity.
pub fn upsert_tag(
    conn: &mut Connection,
    library_row_id: &str,
    input: TagInput,
) -> BibliographyResult<ZoteroTag> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(&input.tag_text, "tag text")?;
    validate_json(&input.native_json_snapshot, "native_json_snapshot")?;
    let now = now_ms();
    let id = uuid::Uuid::new_v4().to_string();
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open tag transaction", error))?;

    tx.execute(
        "INSERT INTO zotero_tags
           (id, library_id, tag_text, tag_type, native_json_snapshot, native_version,
            created_at, updated_at, verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7)
         ON CONFLICT(library_id, tag_text) DO UPDATE SET
            tag_type = excluded.tag_type,
            native_json_snapshot = excluded.native_json_snapshot,
            native_version = excluded.native_version,
            revision = zotero_tags.revision + 1,
            updated_at = excluded.updated_at,
            verified_at = excluded.verified_at
         WHERE zotero_tags.tag_type IS NOT excluded.tag_type
            OR zotero_tags.native_json_snapshot IS NOT excluded.native_json_snapshot
            OR zotero_tags.native_version IS NOT excluded.native_version",
        rusqlite::params![
            &id,
            library_row_id,
            &input.tag_text,
            input.tag_type.as_deref(),
            &input.native_json_snapshot,
            input.native_version,
            now,
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert Zotero tag", error))?;

    let tag = read_tag(&tx, library_row_id, &input.tag_text)?;
    clear_tombstone(
        &tx,
        "zotero_tag_tombstones",
        "tag_id",
        &tag.id,
        "Failed to clear Zotero tag tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit Zotero tag", error))?;
    Ok(tag)
}

fn read_attachment(
    conn: &Connection,
    parent_item_id: &str,
    attachment_key: &str,
) -> BibliographyResult<ZoteroAttachment> {
    conn.query_row(
        "SELECT id, item_id, attachment_key, content_type, link_mode, filename,
                native_path, url, md5, mtime, native_json_snapshot, native_version,
                revision, created_at, updated_at, verified_at
           FROM zotero_attachments
          WHERE item_id = ?1 AND attachment_key = ?2",
        rusqlite::params![parent_item_id, attachment_key],
        |row| {
            Ok(ZoteroAttachment {
                id: row.get(0)?,
                parent_item_id: row.get(1)?,
                attachment_key: row.get(2)?,
                content_type: row.get(3)?,
                link_mode: row.get(4)?,
                filename: row.get(5)?,
                native_path: row.get(6)?,
                url: row.get(7)?,
                md5: row.get(8)?,
                mtime: row.get(9)?,
                native_json_snapshot: row.get(10)?,
                native_version: row.get(11)?,
                revision: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                verified_at: row.get(15)?,
            })
        },
    )
    .map_err(|error| BibliographyError::sql("Failed to read Zotero attachment", error))
}

/// Creates or refreshes one attachment under an existing parent item. Metadata
/// is persisted as supplied; this function never touches the local filesystem.
pub fn upsert_attachment(
    conn: &mut Connection,
    parent_item_id: &str,
    input: AttachmentInput,
) -> BibliographyResult<ZoteroAttachment> {
    require_non_empty(parent_item_id, "parent item id")?;
    require_non_empty(&input.attachment_key, "attachment key")?;
    validate_json(&input.native_json_snapshot, "native_json_snapshot")?;
    let now = now_ms();
    let id = uuid::Uuid::new_v4().to_string();
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open attachment transaction", error))?;

    tx.execute(
        "INSERT INTO zotero_attachments
           (id, item_id, attachment_key, content_type, link_mode, filename,
            native_path, url, md5, mtime, native_json_snapshot, native_version,
            created_at, updated_at, verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?13)
         ON CONFLICT(item_id, attachment_key) DO UPDATE SET
            content_type = excluded.content_type,
            link_mode = excluded.link_mode,
            filename = excluded.filename,
            native_path = excluded.native_path,
            url = excluded.url,
            md5 = excluded.md5,
            mtime = excluded.mtime,
            native_json_snapshot = excluded.native_json_snapshot,
            native_version = excluded.native_version,
            revision = zotero_attachments.revision + 1,
            updated_at = excluded.updated_at,
            verified_at = excluded.verified_at
         WHERE zotero_attachments.content_type IS NOT excluded.content_type
            OR zotero_attachments.link_mode IS NOT excluded.link_mode
            OR zotero_attachments.filename IS NOT excluded.filename
            OR zotero_attachments.native_path IS NOT excluded.native_path
            OR zotero_attachments.url IS NOT excluded.url
            OR zotero_attachments.md5 IS NOT excluded.md5
            OR zotero_attachments.mtime IS NOT excluded.mtime
            OR zotero_attachments.native_json_snapshot IS NOT excluded.native_json_snapshot
            OR zotero_attachments.native_version IS NOT excluded.native_version",
        rusqlite::params![
            &id,
            parent_item_id,
            &input.attachment_key,
            input.content_type.as_deref(),
            input.link_mode.as_deref(),
            input.filename.as_deref(),
            input.native_path.as_deref(),
            input.url.as_deref(),
            input.md5.as_deref(),
            input.mtime,
            &input.native_json_snapshot,
            input.native_version,
            now,
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert Zotero attachment", error))?;

    let attachment = read_attachment(&tx, parent_item_id, &input.attachment_key)?;
    clear_tombstone(
        &tx,
        "zotero_attachment_tombstones",
        "attachment_id",
        &attachment.id,
        "Failed to clear Zotero attachment tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit Zotero attachment", error))?;
    Ok(attachment)
}

/// Adds one item/collection edge. The composite foreign keys in migration 0039
/// make the library scope part of the invariant, not merely a caller promise.
pub fn upsert_item_collection(
    conn: &mut Connection,
    library_row_id: &str,
    input: ItemCollectionInput,
) -> BibliographyResult<ZoteroItemCollection> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(&input.item_id, "item id")?;
    require_non_empty(&input.collection_id, "collection id")?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open item collection transaction", error)
    })?;
    tx.execute(
        "INSERT INTO zotero_item_collections (library_id, item_id, collection_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(library_id, item_id, collection_id) DO NOTHING",
        rusqlite::params![library_row_id, &input.item_id, &input.collection_id],
    )
    .map_err(|error| {
        BibliographyError::sql("Failed to upsert item collection membership", error)
    })?;
    tx.commit().map_err(|error| {
        BibliographyError::sql("Failed to commit item collection membership", error)
    })?;
    Ok(ZoteroItemCollection {
        library_row_id: library_row_id.to_string(),
        item_id: input.item_id,
        collection_id: input.collection_id,
    })
}

/// Adds one item/tag edge with the same composite library-scope invariant as
/// item/collection membership.
pub fn upsert_item_tag(
    conn: &mut Connection,
    library_row_id: &str,
    input: ItemTagInput,
) -> BibliographyResult<ZoteroItemTag> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(&input.item_id, "item id")?;
    require_non_empty(&input.tag_id, "tag id")?;
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open item tag transaction", error))?;
    tx.execute(
        "INSERT INTO zotero_item_tags (library_id, item_id, tag_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(library_id, item_id, tag_id) DO NOTHING",
        rusqlite::params![library_row_id, &input.item_id, &input.tag_id],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert item tag membership", error))?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit item tag membership", error))?;
    Ok(ZoteroItemTag {
        library_row_id: library_row_id.to_string(),
        item_id: input.item_id,
        tag_id: input.tag_id,
    })
}

fn write_tombstone(
    conn: &Connection,
    table: &str,
    entity_column: &str,
    entity_id: &str,
    input: &TombstoneInput,
    observed_at: i64,
    context: &str,
) -> BibliographyResult<()> {
    let sql = format!(
        "INSERT INTO {table} ({entity_column}, observed_at, remote_version, reason)\n         VALUES (?1, ?2, ?3, ?4)\n         ON CONFLICT({entity_column}) DO UPDATE SET\n            observed_at = excluded.observed_at,\n            remote_version = excluded.remote_version,\n            reason = excluded.reason"
    );
    conn.execute(
        &sql,
        rusqlite::params![entity_id, observed_at, input.remote_version, &input.reason],
    )
    .map(|_| ())
    .map_err(|error| BibliographyError::sql(context, error))
}

fn read_tombstone(
    conn: &Connection,
    table: &str,
    entity_column: &str,
    entity_id: &str,
    context: &str,
) -> BibliographyResult<TombstoneRecord> {
    let sql = format!(
        "SELECT observed_at, remote_version, reason FROM {table} WHERE {entity_column} = ?1"
    );
    conn.query_row(&sql, [entity_id], |row| {
        Ok(TombstoneRecord {
            observed_at: row.get(0)?,
            remote_version: row.get(1)?,
            reason: row.get(2)?,
        })
    })
    .map_err(|error| BibliographyError::sql(context, error))
}

/// Reads the tombstone for one item, when one exists. `None` is a live item,
/// not an error: the detail projection branches on presence.
pub(crate) fn read_item_tombstone(
    conn: &Connection,
    item_id: &str,
) -> BibliographyResult<Option<TombstoneRecord>> {
    conn.query_row(
        "SELECT observed_at, remote_version, reason FROM zotero_item_tombstones WHERE item_id = ?1",
        [item_id],
        |row| {
            Ok(TombstoneRecord {
                observed_at: row.get(0)?,
                remote_version: row.get(1)?,
                reason: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|error| BibliographyError::sql("Failed to read item tombstone", error))
}

pub fn tombstone_item(
    conn: &mut Connection,
    library_row_id: &str,
    item_key: &str,
    input: TombstoneInput,
) -> BibliographyResult<Tombstoned<BibliographicItem>> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(item_key, "item key")?;
    validate_tombstone_input(&input)?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open item tombstone transaction", error)
    })?;
    let entity = read_item(&tx, library_row_id, item_key)?;
    write_tombstone(
        &tx,
        "zotero_item_tombstones",
        "item_id",
        &entity.id,
        &input,
        now_ms(),
        "Failed to write item tombstone",
    )?;
    let tombstone = read_tombstone(
        &tx,
        "zotero_item_tombstones",
        "item_id",
        &entity.id,
        "Failed to read item tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit item tombstone", error))?;
    Ok(Tombstoned { entity, tombstone })
}

pub fn tombstone_collection(
    conn: &mut Connection,
    library_row_id: &str,
    collection_key: &str,
    input: TombstoneInput,
) -> BibliographyResult<Tombstoned<ZoteroCollection>> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(collection_key, "collection key")?;
    validate_tombstone_input(&input)?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open collection tombstone transaction", error)
    })?;
    let entity = read_collection(&tx, library_row_id, collection_key)?;
    write_tombstone(
        &tx,
        "zotero_collection_tombstones",
        "collection_id",
        &entity.id,
        &input,
        now_ms(),
        "Failed to write collection tombstone",
    )?;
    let tombstone = read_tombstone(
        &tx,
        "zotero_collection_tombstones",
        "collection_id",
        &entity.id,
        "Failed to read collection tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit collection tombstone", error))?;
    Ok(Tombstoned { entity, tombstone })
}

pub fn tombstone_tag(
    conn: &mut Connection,
    library_row_id: &str,
    tag_text: &str,
    input: TombstoneInput,
) -> BibliographyResult<Tombstoned<ZoteroTag>> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(tag_text, "tag text")?;
    validate_tombstone_input(&input)?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open tag tombstone transaction", error)
    })?;
    let entity = read_tag(&tx, library_row_id, tag_text)?;
    write_tombstone(
        &tx,
        "zotero_tag_tombstones",
        "tag_id",
        &entity.id,
        &input,
        now_ms(),
        "Failed to write tag tombstone",
    )?;
    let tombstone = read_tombstone(
        &tx,
        "zotero_tag_tombstones",
        "tag_id",
        &entity.id,
        "Failed to read tag tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit tag tombstone", error))?;
    Ok(Tombstoned { entity, tombstone })
}

pub fn tombstone_attachment(
    conn: &mut Connection,
    parent_item_id: &str,
    attachment_key: &str,
    input: TombstoneInput,
) -> BibliographyResult<Tombstoned<ZoteroAttachment>> {
    require_non_empty(parent_item_id, "parent item id")?;
    require_non_empty(attachment_key, "attachment key")?;
    validate_tombstone_input(&input)?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open attachment tombstone transaction", error)
    })?;
    let entity = read_attachment(&tx, parent_item_id, attachment_key)?;
    write_tombstone(
        &tx,
        "zotero_attachment_tombstones",
        "attachment_id",
        &entity.id,
        &input,
        now_ms(),
        "Failed to write attachment tombstone",
    )?;
    let tombstone = read_tombstone(
        &tx,
        "zotero_attachment_tombstones",
        "attachment_id",
        &entity.id,
        "Failed to read attachment tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit attachment tombstone", error))?;
    Ok(Tombstoned { entity, tombstone })
}

pub fn untombstone_item(
    conn: &mut Connection,
    library_row_id: &str,
    item_key: &str,
) -> BibliographyResult<BibliographicItem> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(item_key, "item key")?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open item untombstone transaction", error)
    })?;
    let entity = read_item(&tx, library_row_id, item_key)?;
    clear_tombstone(
        &tx,
        "zotero_item_tombstones",
        "item_id",
        &entity.id,
        "Failed to clear item tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit item untombstone", error))?;
    Ok(entity)
}

pub fn untombstone_collection(
    conn: &mut Connection,
    library_row_id: &str,
    collection_key: &str,
) -> BibliographyResult<ZoteroCollection> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(collection_key, "collection key")?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open collection untombstone transaction", error)
    })?;
    let entity = read_collection(&tx, library_row_id, collection_key)?;
    clear_tombstone(
        &tx,
        "zotero_collection_tombstones",
        "collection_id",
        &entity.id,
        "Failed to clear collection tombstone",
    )?;
    tx.commit().map_err(|error| {
        BibliographyError::sql("Failed to commit collection untombstone", error)
    })?;
    Ok(entity)
}

pub fn untombstone_tag(
    conn: &mut Connection,
    library_row_id: &str,
    tag_text: &str,
) -> BibliographyResult<ZoteroTag> {
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(tag_text, "tag text")?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open tag untombstone transaction", error)
    })?;
    let entity = read_tag(&tx, library_row_id, tag_text)?;
    clear_tombstone(
        &tx,
        "zotero_tag_tombstones",
        "tag_id",
        &entity.id,
        "Failed to clear tag tombstone",
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit tag untombstone", error))?;
    Ok(entity)
}

pub fn untombstone_attachment(
    conn: &mut Connection,
    parent_item_id: &str,
    attachment_key: &str,
) -> BibliographyResult<ZoteroAttachment> {
    require_non_empty(parent_item_id, "parent item id")?;
    require_non_empty(attachment_key, "attachment key")?;
    let tx = conn.transaction().map_err(|error| {
        BibliographyError::sql("Failed to open attachment untombstone transaction", error)
    })?;
    let entity = read_attachment(&tx, parent_item_id, attachment_key)?;
    clear_tombstone(
        &tx,
        "zotero_attachment_tombstones",
        "attachment_id",
        &entity.id,
        "Failed to clear attachment tombstone",
    )?;
    tx.commit().map_err(|error| {
        BibliographyError::sql("Failed to commit attachment untombstone", error)
    })?;
    Ok(entity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_instance_is_not_derived_from_a_library_version() {
        assert_eq!(SourceOrigin::Local.as_str(), "local");
        assert_eq!(SourceOrigin::Web.as_str(), "web");
    }

    #[test]
    fn malformed_optional_creators_json_is_rejected() {
        let mut conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0038_bibliography_catalog.sql"
        ))
        .expect("apply bibliography foundation migration");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0039_bibliography_relations.sql"
        ))
        .expect("apply bibliography relations migration");

        let source = upsert_connection(
            &mut conn,
            UpsertConnection {
                id: "conn-1".to_string(),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://synthetic.invalid".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        let library = upsert_library(
            &mut conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::User,
                library_id: "0".to_string(),
                name: "Synthetic 0".to_string(),
                last_modified_version: Some(7),
            },
        )
        .expect("library");

        let error = upsert_item(
            &mut conn,
            &library.id,
            BibliographicItemInput {
                item_key: "BADJSON1".to_string(),
                item_version: Some(1),
                native_json_snapshot: r#"{"key":"BADJSON1"}"#.to_string(),
                csl_json_snapshot: r#"{"id":"bad-json-csl","type":"book"}"#.to_string(),
                creators_json: Some(r#"[{"name":"Ada"}"#.to_string()),
                ..Default::default()
            },
        )
        .expect_err("malformed optional creators_json must be rejected");

        assert_eq!(error.code, "invalid_json");
    }
}
