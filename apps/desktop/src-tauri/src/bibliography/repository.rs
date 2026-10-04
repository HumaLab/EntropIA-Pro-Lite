//! Transactional persistence for the E1b-1b Zotero catalog relations.
//!
//! A connection is a source namespace. A library qualifies native Zotero keys,
//! so the same key in two libraries remains two catalog rows. Native Zotero
//! JSON and CSL JSON are stored as separate snapshots; CSL `id` is never used
//! as a substitute for the native key. Tombstones are explicit side records;
//! they never remove snapshots or membership edges.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const MIGRATION_NAME: &str = "0041_bibliography_relations";

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

    pub(crate) fn sql(context: &str, error: rusqlite::Error) -> Self {
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

/// Returns true when a library row with this internal id exists.
///
/// E2b-1 admission seam: the processing queue validates bibliography subjects
/// through this helper so it never queries bibliography tables inline. A
/// missing `zotero_libraries` table (pre-0040 database) reads as absent — the
/// caller then rejects with an honest `unknown_library` instead of a schema
/// error. Any other storage failure is returned as `sql_error`.
pub fn library_row_exists(conn: &Connection, library_row_id: &str) -> Result<bool, String> {
    if library_row_id.is_empty() {
        return Ok(false);
    }
    match conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM zotero_libraries WHERE id = ?1)",
        [library_row_id],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(exists) => Ok(exists != 0),
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") {
                return Ok(false);
            }
            Err(format!(
                "Failed to check Zotero library {library_row_id}: {message}"
            ))
        }
    }
}

/// Reads the sync pin for one library row: `Some(version)` when the row
/// exists (the version is `None` when `last_modified_version` is SQL NULL),
/// `None` when the row — or the table itself — is absent.
///
/// E2b-1 pins bibliography tasks conservatively from this value (see
/// `processing::repository::admit_subject_or_attach`): `input_revision` is the
/// version or 0, and the fingerprint is `library|<id>|<version-or-0>`. One
/// query serves both existence and pin so admission never reads bibliography
/// state inline.
pub fn library_sync_pin(
    conn: &Connection,
    library_row_id: &str,
) -> Result<Option<Option<i64>>, String> {
    if library_row_id.is_empty() {
        return Ok(None);
    }
    match required_library_sync_pin(conn, library_row_id) {
        Err(message) if message.contains("no such table") => Ok(None),
        result => result,
    }
}

/// Strict sync pin for post-migration processing paths. Unlike the E2b-1
/// admission compatibility helper above, a missing catalog table is a storage
/// error: active bibliography work must never interpret schema loss as a
/// vanished library.
pub(crate) fn required_library_sync_pin(
    conn: &Connection,
    library_row_id: &str,
) -> Result<Option<Option<i64>>, String> {
    if library_row_id.is_empty() {
        return Ok(None);
    }
    match conn.query_row(
        "SELECT last_modified_version FROM zotero_libraries WHERE id = ?1",
        [library_row_id],
        |row| row.get::<_, Option<i64>>(0),
    ) {
        Ok(version) => Ok(Some(version)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(format!(
            "Failed to read Zotero library pin {library_row_id}: {error}"
        )),
    }
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
pub(crate) fn upsert_item_in_transaction(
    conn: &Connection,
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

    conn.execute(
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

    let item = read_item(conn, library_row_id, &input.item_key)?;
    clear_tombstone(
        conn,
        "zotero_item_tombstones",
        "item_id",
        &item.id,
        "Failed to clear bibliographic item tombstone",
    )?;
    Ok(item)
}

/// Public E1b wrapper: preserves the one-call/one-transaction contract while
/// the sync executor can compose [`upsert_item_in_transaction`] with its
/// reconciliation checkpoint in a larger page transaction.
pub fn upsert_item(
    conn: &mut Connection,
    library_row_id: &str,
    input: BibliographicItemInput,
) -> BibliographyResult<BibliographicItem> {
    // Preserve validation-before-BEGIN behavior for existing callers.
    require_non_empty(library_row_id, "library row id")?;
    require_non_empty(&input.item_key, "item key")?;
    validate_json(&input.native_json_snapshot, "native_json_snapshot")?;
    validate_json(&input.csl_json_snapshot, "csl_json_snapshot")?;
    if let Some(creators_json) = input.creators_json.as_deref() {
        validate_json(creators_json, "creators_json")?;
    }
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open item transaction", error))?;
    let item = upsert_item_in_transaction(&tx, library_row_id, input)?;
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

/// Adds one item/collection edge. The composite foreign keys in migration 0041
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
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("apply bibliography foundation migration");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
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

// ── Semantic profiles (E3b-WU1) ────────────────────────────────────────────

/// One durable `bibliographic_semantic_profiles` row. `field_provenance_json`
/// is validated JSON so a corrupt writer can never store unreadable
/// provenance next to the text it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticProfileRow {
    pub item_id: String,
    pub profile_revision: i64,
    pub template_version: String,
    pub canonical_text: String,
    pub input_hash: String,
    pub field_provenance_json: String,
}

/// Reads the stored profile for one work, if any.
pub fn get_semantic_profile(
    conn: &Connection,
    item_id: &str,
) -> BibliographyResult<Option<SemanticProfileRow>> {
    require_non_empty(item_id, "item id")?;
    let row = conn
        .query_row(
            "SELECT item_id, profile_revision, template_version, canonical_text,
                    input_hash, field_provenance_json
             FROM bibliographic_semantic_profiles WHERE item_id = ?1",
            [item_id],
            |row| {
                Ok(SemanticProfileRow {
                    item_id: row.get(0)?,
                    profile_revision: row.get(1)?,
                    template_version: row.get(2)?,
                    canonical_text: row.get(3)?,
                    input_hash: row.get(4)?,
                    field_provenance_json: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|error| BibliographyError::sql("Failed to read semantic profile", error))?;
    Ok(row)
}

/// Upserts the profile of one work inside the caller's transaction. The
/// profile revision is monotonic per work (previous + 1); an unchanged hash
/// keeps the stored row untouched and returns its revision, so repeated
/// syncs never churn history. The work must exist in the verified catalog —
/// the FK enforces it and a missing row fails honestly.
pub fn upsert_semantic_profile_in_transaction(
    tx: &Connection,
    item_id: &str,
    template_version: &str,
    canonical_text: &str,
    input_hash: &str,
    field_provenance_json: &str,
    now_ms: i64,
) -> BibliographyResult<i64> {
    require_non_empty(item_id, "item id")?;
    require_non_empty(template_version, "template version")?;
    require_non_empty(canonical_text, "canonical text")?;
    require_non_empty(input_hash, "input hash")?;
    validate_json(field_provenance_json, "field_provenance_json")?;
    let exists: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .map_err(|error| BibliographyError::sql("Failed to check profiled work", error))?;
    if exists == 0 {
        return Err(BibliographyError::new(
            "unknown_item",
            format!("work {item_id} does not exist in the verified catalog"),
        ));
    }
    let previous = get_semantic_profile(tx, item_id)?;
    if let Some(previous) = &previous {
        if previous.template_version == template_version && previous.input_hash == input_hash {
            return Ok(previous.profile_revision);
        }
    }
    let revision = match &previous {
        Some(previous) => previous.profile_revision + 1,
        None => 1,
    };
    tx.execute(
        "INSERT INTO bibliographic_semantic_profiles
           (item_id, profile_revision, template_version, canonical_text,
            input_hash, field_provenance_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(item_id) DO UPDATE SET
           profile_revision = excluded.profile_revision,
           template_version = excluded.template_version,
           canonical_text = excluded.canonical_text,
           input_hash = excluded.input_hash,
           field_provenance_json = excluded.field_provenance_json,
           updated_at = excluded.updated_at",
        rusqlite::params![
            item_id,
            revision,
            template_version,
            canonical_text,
            input_hash,
            field_provenance_json,
            now_ms
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert semantic profile", error))?;
    Ok(revision)
}

/// Public E3b wrapper: one call, one transaction.
pub fn upsert_semantic_profile(
    conn: &mut Connection,
    item_id: &str,
    template_version: &str,
    canonical_text: &str,
    input_hash: &str,
    field_provenance_json: &str,
    now_ms: i64,
) -> BibliographyResult<i64> {
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::sql("Failed to open profile transaction", error))?;
    let revision = upsert_semantic_profile_in_transaction(
        &tx,
        item_id,
        template_version,
        canonical_text,
        input_hash,
        field_provenance_json,
        now_ms,
    )?;
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit semantic profile", error))?;
    Ok(revision)
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use crate::bibliography::profile::{
        build_profile, ProfileInput, BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
    };

    fn catalog_db() -> (Connection, String, String) {
        let mut conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("apply bibliography foundation migration");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
        ))
        .expect("apply bibliography relations migration");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0047_bibliographic_semantic_profiles.sql"
        ))
        .expect("apply semantic profiles migration");
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
        let mut ids = Vec::new();
        for (key, title) in [("AAAA1111", "Obra sin adjunto"), ("BBBB2222", "Otra obra")] {
            let item = upsert_item(
                &mut conn,
                &library.id,
                BibliographicItemInput {
                    item_key: key.to_string(),
                    item_version: Some(1),
                    native_json_snapshot: format!("{{\"key\":\"{key}\"}}"),
                    csl_json_snapshot: format!("{{\"id\":\"{key}\",\"type\":\"book\"}}"),
                    title: Some(title.to_string()),
                    ..Default::default()
                },
            )
            .expect("catalog item");
            ids.push(item.id);
        }
        let (first, second) = (ids.remove(0), ids.remove(0));
        (conn, first, second)
    }

    fn provenance_json(built: &crate::bibliography::profile::BuiltProfile) -> String {
        serde_json::to_string(
            &built
                .field_provenance
                .iter()
                .map(|(field, line)| serde_json::json!({ "field": field, "line": line }))
                .collect::<Vec<_>>(),
        )
        .expect("provenance json")
    }

    #[test]
    fn profile_upsert_revisions_are_monotonic_and_unchanged_hashes_are_stable() {
        let (mut conn, item_id, _other) = catalog_db();
        let built = build_profile(&ProfileInput {
            title: "Obra sin adjunto".to_string(),
            item_type: "book".to_string(),
            ..Default::default()
        });
        let revision = upsert_semantic_profile(
            &mut conn,
            &item_id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &built.canonical_text,
            &built.input_hash,
            &provenance_json(&built),
            1_000,
        )
        .expect("first publish");
        assert_eq!(revision, 1);
        // Same hash: no churn.
        let again = upsert_semantic_profile(
            &mut conn,
            &item_id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &built.canonical_text,
            &built.input_hash,
            &provenance_json(&built),
            2_000,
        )
        .expect("idempotent publish");
        assert_eq!(again, 1);
        // A metadata edit bumps the revision and rewrites the text.
        let edited = build_profile(&ProfileInput {
            title: "Obra sin adjunto".to_string(),
            item_type: "book".to_string(),
            abstract_text: "Resumen corregido".to_string(),
            ..Default::default()
        });
        let bumped = upsert_semantic_profile(
            &mut conn,
            &item_id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &edited.canonical_text,
            &edited.input_hash,
            &provenance_json(&edited),
            3_000,
        )
        .expect("edited publish");
        assert_eq!(bumped, 2);
        let stored = get_semantic_profile(&conn, &item_id)
            .expect("read")
            .expect("stored");
        assert_eq!(stored.profile_revision, 2);
        assert_eq!(stored.canonical_text, edited.canonical_text);
        // A work without any attachment profiles identically: the row only
        // needs the verified catalog entry.
        assert!(stored
            .canonical_text
            .starts_with("Título: Obra sin adjunto"));
    }

    #[test]
    fn profile_upsert_rejects_unknown_works_and_bad_provenance() {
        let (mut conn, _item_id, other_id) = catalog_db();
        let built = build_profile(&ProfileInput {
            title: "Fantasma".to_string(),
            ..Default::default()
        });
        let error = upsert_semantic_profile(
            &mut conn,
            "item-missing",
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &built.canonical_text,
            &built.input_hash,
            &provenance_json(&built),
            1_000,
        )
        .expect_err("unknown work refused");
        assert_eq!(error.code, "unknown_item");
        let error = upsert_semantic_profile(
            &mut conn,
            &other_id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            &built.canonical_text,
            &built.input_hash,
            "not-json",
            1_000,
        )
        .expect_err("invalid provenance refused");
        assert_eq!(error.code, "invalid_json");
        assert!(get_semantic_profile(&conn, &other_id).unwrap().is_none());
    }
}

/// True when the verified catalog still holds the work.
pub fn bibliographic_item_exists(conn: &Connection, item_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM bibliographic_items WHERE id = ?1",
        [item_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(|error| format!("Failed to check catalog item {item_id}: {error}"))
}

/// Upserts one (work, contract) vector inside the caller's transaction.
/// Each publish stamps the profile revision and input hash it was computed
/// from, so a stale vector is always explainable — and never silently
/// reused across contracts or metadata edits.
// E3c-WU2: every vector names the generation it was computed in
// (uniqueness per object/generation, plan section 6).
pub struct ItemEmbeddingRow {
    pub item_id: String,
    pub generation_id: String,
    pub embedding_contract: String,
    pub embedding_model: String,
    pub dimensions: usize,
    pub embedding: Vec<u8>,
    pub input_hash: String,
    pub profile_revision: i64,
}

pub fn upsert_item_embedding_in_transaction(
    tx: &Connection,
    row: &ItemEmbeddingRow,
    now_ms: i64,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO bibliographic_item_embeddings
           (item_id, generation_id, embedding_contract, embedding_model, dimensions, embedding,
            input_hash, profile_revision, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(item_id, generation_id) DO UPDATE SET
           embedding_model = excluded.embedding_model,
           dimensions = excluded.dimensions,
           embedding = excluded.embedding,
           input_hash = excluded.input_hash,
           profile_revision = excluded.profile_revision,
           updated_at = excluded.updated_at",
        rusqlite::params![
            row.item_id,
            row.generation_id,
            row.embedding_contract,
            row.embedding_model,
            row.dimensions as i64,
            row.embedding,
            row.input_hash,
            row.profile_revision,
            now_ms
        ],
    )
    .map_err(|error| format!("Failed to upsert work embedding: {error}"))?;
    Ok(())
}

// ── Native extractions (E4a-WU2) ───────────────────────────────────────────

/// One durable `bibliographic_extractions` row: whole-document native text
/// plus the source file identity it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionRow {
    pub attachment_id: String,
    pub item_id: String,
    pub page_count: i64,
    pub method: String,
    pub text_content: String,
    pub text_hash: String,
    pub text_chars: i64,
    pub quality: String,
    pub source_mtime: Option<i64>,
    pub source_bytes: i64,
}

/// True when the stored extraction of `attachment_id` was read from the
/// source identity given: the catalog mtime of the attachment row (what the
/// extractor pins as `source_mtime`) and the byte length of the file read.
/// Cheap by design: it never loads the text. Both the admission gate and the
/// executor decide "current" through this one predicate, so they cannot drift.
pub fn extraction_matches_source(
    conn: &Connection,
    attachment_id: &str,
    catalog_mtime: Option<i64>,
    file_bytes: i64,
) -> BibliographyResult<bool> {
    let stored: Option<(Option<i64>, i64)> = conn
        .query_row(
            "SELECT source_mtime, source_bytes FROM bibliographic_extractions
             WHERE attachment_id = ?1",
            [attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| BibliographyError::sql("Failed to read extraction identity", error))?;
    Ok(stored == Some((catalog_mtime, file_bytes)))
}

/// True when the stored extraction needs nothing more for this source: it
/// matches the source identity AND is not an `empty` verdict that never went
/// through a real OCR pass. A scan stored as `empty` by a build that could not
/// read it (or while no OCR provider answered) is demanded again; once a
/// provider has answered for it, even with no text, the verdict is final, so
/// blank documents do not cost an OCR request on every sync. Rich and sparse
/// extractions are never re-demanded. Admission and the executor both decide
/// through this predicate.
pub fn extraction_is_settled(
    conn: &Connection,
    attachment_id: &str,
    catalog_mtime: Option<i64>,
    file_bytes: i64,
) -> BibliographyResult<bool> {
    if !extraction_matches_source(conn, attachment_id, catalog_mtime, file_bytes)? {
        return Ok(false);
    }
    let unresolved_empty: bool = conn
        .query_row(
            "SELECT e.quality = 'empty'
                AND NOT EXISTS (
                    SELECT 1 FROM processing_tasks t
                    WHERE t.kind = 'bibliography_extract'
                      AND t.subject_id = e.attachment_id
                      AND t.state = 'succeeded'
                      AND t.result_receipt_json LIKE '%\"ocrAttempted\":true%')
             FROM bibliographic_extractions e WHERE e.attachment_id = ?1",
            [attachment_id],
            |row| row.get(0),
        )
        .map_err(|error| BibliographyError::sql("Failed to read extraction quality", error))?;
    Ok(!unresolved_empty)
}

/// Reads the stored extraction for one attachment, if any.
pub fn get_extraction(
    conn: &Connection,
    attachment_id: &str,
) -> BibliographyResult<Option<ExtractionRow>> {
    require_non_empty(attachment_id, "attachment id")?;
    let row = conn
        .query_row(
            "SELECT attachment_id, item_id, page_count, method, text_content,
                    text_hash, text_chars, quality, source_mtime, source_bytes
             FROM bibliographic_extractions WHERE attachment_id = ?1",
            [attachment_id],
            |row| {
                Ok(ExtractionRow {
                    attachment_id: row.get(0)?,
                    item_id: row.get(1)?,
                    page_count: row.get(2)?,
                    method: row.get(3)?,
                    text_content: row.get(4)?,
                    text_hash: row.get(5)?,
                    text_chars: row.get(6)?,
                    quality: row.get(7)?,
                    source_mtime: row.get(8)?,
                    source_bytes: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|error| BibliographyError::sql("Failed to read extraction", error))?;
    Ok(row)
}

/// Upserts one extraction inside the caller's transaction. The attachment
/// must exist — the FK enforces it and a missing row fails honestly.
#[allow(clippy::too_many_arguments)]
pub fn upsert_extraction_in_transaction(
    tx: &Connection,
    row: &ExtractionRow,
    now_ms: i64,
) -> BibliographyResult<()> {
    require_non_empty(&row.attachment_id, "attachment id")?;
    require_non_empty(&row.item_id, "item id")?;
    if row.method != "native" {
        return Err(BibliographyError::new(
            "invalid_input",
            "E4a publishes native extractions only",
        ));
    }
    if row.quality != "rich" && row.quality != "sparse" && row.quality != "empty" {
        return Err(BibliographyError::new(
            "invalid_input",
            "extraction quality must be rich, sparse, or empty",
        ));
    }
    tx.execute(
        "INSERT INTO bibliographic_extractions
           (attachment_id, item_id, page_count, method, text_content,
            text_hash, text_chars, quality, source_mtime, source_bytes,
            created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT(attachment_id) DO UPDATE SET
           item_id = excluded.item_id,
           page_count = excluded.page_count,
           method = excluded.method,
           text_content = excluded.text_content,
           text_hash = excluded.text_hash,
           text_chars = excluded.text_chars,
           quality = excluded.quality,
           source_mtime = excluded.source_mtime,
           source_bytes = excluded.source_bytes,
           updated_at = excluded.updated_at",
        rusqlite::params![
            row.attachment_id,
            row.item_id,
            row.page_count,
            row.method,
            row.text_content,
            row.text_hash,
            row.text_chars,
            row.quality,
            row.source_mtime,
            row.source_bytes,
            now_ms
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert extraction", error))?;
    Ok(())
}

// ── Per-page native texts (E4b-WU2) ────────────────────────────────────────

/// One durable `bibliographic_page_texts` row: the native text layer of
/// exactly one page with its own hash and quality, so selective OCR can
/// skip rich pages without re-reading the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTextRow {
    pub attachment_id: String,
    pub page_number: i64,
    pub method: String,
    pub text_content: String,
    pub text_hash: String,
    pub text_chars: i64,
    pub quality: String,
}

/// Upserts one page-text row inside the caller's transaction.
pub fn upsert_page_text_in_transaction(
    tx: &Connection,
    row: &PageTextRow,
    now_ms: i64,
) -> BibliographyResult<()> {
    require_non_empty(&row.attachment_id, "attachment id")?;
    if row.page_number < 1 {
        return Err(BibliographyError::new(
            "invalid_input",
            "page numbers are 1-based",
        ));
    }
    if row.method != "native" && row.method != "ocr" {
        return Err(BibliographyError::new(
            "invalid_input",
            "page text method must be native or ocr",
        ));
    }
    tx.execute(
        "INSERT INTO bibliographic_page_texts
           (attachment_id, page_number, method, text_content, text_hash,
            text_chars, quality, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(attachment_id, page_number) DO UPDATE SET
           method = excluded.method,
           text_content = excluded.text_content,
           text_hash = excluded.text_hash,
           text_chars = excluded.text_chars,
           quality = excluded.quality,
           updated_at = excluded.updated_at",
        rusqlite::params![
            row.attachment_id,
            row.page_number,
            row.method,
            row.text_content,
            row.text_hash,
            row.text_chars,
            row.quality,
            now_ms
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert page text", error))?;
    Ok(())
}

/// Reads every stored page-text row of one attachment, in page order.
pub fn page_texts_for_attachment(
    conn: &Connection,
    attachment_id: &str,
) -> BibliographyResult<Vec<PageTextRow>> {
    require_non_empty(attachment_id, "attachment id")?;
    let mut stmt = conn
        .prepare(
            "SELECT attachment_id, page_number, method, text_content, text_hash,
                    text_chars, quality
             FROM bibliographic_page_texts
             WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .map_err(|error| BibliographyError::sql("Failed to read page texts", error))?;
    let rows = stmt
        .query_map([attachment_id], |row| {
            Ok(PageTextRow {
                attachment_id: row.get(0)?,
                page_number: row.get(1)?,
                method: row.get(2)?,
                text_content: row.get(3)?,
                text_hash: row.get(4)?,
                text_chars: row.get(5)?,
                quality: row.get(6)?,
            })
        })
        .map_err(|error| BibliographyError::sql("Failed to read page texts", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| BibliographyError::sql("Failed to read page texts", error))?;
    Ok(rows)
}

// ── Structural chunks (E4c-WU2) ────────────────────────────────────────────

/// One `bibliographic_chunks` row with its spans, as the publisher writes
/// them: chunk ids are deterministic per (work, ordinal) so a re-chunk is
/// an atomic replace, never an append.
pub struct ChunkRow {
    pub id: String,
    pub item_id: String,
    pub attachment_id: String,
    pub ordinal: i64,
    pub text_content: String,
    pub text_hash: String,
    pub chunking_contract: String,
    pub spans: Vec<(i64, i64, i64)>,
}

/// Reconciles one work's whole chunk set (all its attachments) inside the
/// caller's transaction. Chunks whose ids vanished are deleted — their
/// vectors cascade — while surviving ids are upserted in place, so vectors
/// of previous generations stay queryable until their generation retires.
/// Chunk ids must already be assigned per (work, ordinal) by the caller.
pub fn replace_work_chunks_in_transaction(
    tx: &Connection,
    item_id: &str,
    chunks: &[ChunkRow],
    now_ms: i64,
) -> BibliographyResult<()> {
    require_non_empty(item_id, "item id")?;
    let fresh_ids: Vec<&str> = chunks.iter().map(|chunk| chunk.id.as_str()).collect();
    if fresh_ids.is_empty() {
        tx.execute(
            "DELETE FROM bibliographic_chunks WHERE item_id = ?1",
            [item_id],
        )
        .map_err(|error| BibliographyError::sql("Failed to clear stale chunks", error))?;
        return Ok(());
    }
    let placeholders = fresh_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let mut params: Vec<&dyn rusqlite::ToSql> = vec![&item_id];
    for id in &fresh_ids {
        params.push(id);
    }
    tx.execute(
        &format!(
            "DELETE FROM bibliographic_chunks WHERE item_id = ? AND id NOT IN ({placeholders})"
        ),
        params.as_slice(),
    )
    .map_err(|error| BibliographyError::sql("Failed to clear stale chunks", error))?;
    for chunk in chunks {
        tx.execute(
            "INSERT INTO bibliographic_chunks
               (id, item_id, attachment_id, ordinal, text_content, text_hash,
                chunking_contract, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET
               attachment_id = excluded.attachment_id,
               ordinal = excluded.ordinal,
               text_content = excluded.text_content,
               text_hash = excluded.text_hash,
               chunking_contract = excluded.chunking_contract,
               updated_at = excluded.updated_at",
            rusqlite::params![
                chunk.id,
                chunk.item_id,
                chunk.attachment_id,
                chunk.ordinal,
                chunk.text_content,
                chunk.text_hash,
                chunk.chunking_contract,
                now_ms
            ],
        )
        .map_err(|error| BibliographyError::sql("Failed to insert chunk", error))?;
        tx.execute(
            "DELETE FROM bibliographic_chunk_spans WHERE chunk_id = ?1",
            [&chunk.id],
        )
        .map_err(|error| BibliographyError::sql("Failed to refresh chunk spans", error))?;
        for (page_number, start_char, end_char) in &chunk.spans {
            tx.execute(
                "INSERT INTO bibliographic_chunk_spans
                   (chunk_id, page_number, start_char, end_char)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![chunk.id, page_number, start_char, end_char],
            )
            .map_err(|error| BibliographyError::sql("Failed to insert chunk span", error))?;
        }
    }
    Ok(())
}

/// Reads one work's chunks with spans, in ordinal order.
pub fn chunks_for_item(conn: &Connection, item_id: &str) -> BibliographyResult<Vec<ChunkRow>> {
    require_non_empty(item_id, "item id")?;
    let mut chunks = conn
        .prepare(
            "SELECT id, item_id, attachment_id, ordinal, text_content, text_hash,
                    chunking_contract
             FROM bibliographic_chunks WHERE item_id = ?1 ORDER BY ordinal",
        )
        .map_err(|error| BibliographyError::sql("Failed to read chunks", error))?;
    let rows = chunks
        .query_map([item_id], |row| {
            Ok(ChunkRow {
                id: row.get(0)?,
                item_id: row.get(1)?,
                attachment_id: row.get(2)?,
                ordinal: row.get(3)?,
                text_content: row.get(4)?,
                text_hash: row.get(5)?,
                chunking_contract: row.get(6)?,
                spans: Vec::new(),
            })
        })
        .map_err(|error| BibliographyError::sql("Failed to read chunks", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| BibliographyError::sql("Failed to read chunks", error))?;
    drop(chunks);
    let mut out = Vec::with_capacity(rows.len());
    for mut chunk in rows {
        let mut spans = conn
            .prepare(
                "SELECT page_number, start_char, end_char FROM bibliographic_chunk_spans
                 WHERE chunk_id = ?1 ORDER BY page_number, start_char",
            )
            .map_err(|error| BibliographyError::sql("Failed to read spans", error))?;
        chunk.spans = spans
            .query_map([&chunk.id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| BibliographyError::sql("Failed to read spans", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| BibliographyError::sql("Failed to read spans", error))?;
        out.push(chunk);
    }
    Ok(out)
}

/// One `bibliographic_chunk_embeddings` row: a chunk's vector under one
/// generation, stamping the chunk hash it was computed from.
pub struct ChunkEmbeddingRow {
    pub chunk_id: String,
    pub generation_id: String,
    pub embedding_contract: String,
    pub embedding_model: String,
    pub dimensions: usize,
    pub embedding: Vec<u8>,
    pub input_hash: String,
}

pub fn upsert_chunk_embedding_in_transaction(
    tx: &Connection,
    row: &ChunkEmbeddingRow,
    now_ms: i64,
) -> BibliographyResult<()> {
    require_non_empty(&row.chunk_id, "chunk id")?;
    require_non_empty(&row.generation_id, "generation id")?;
    tx.execute(
        "INSERT INTO bibliographic_chunk_embeddings
           (chunk_id, generation_id, embedding_contract, embedding_model,
            dimensions, embedding, input_hash, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(chunk_id, generation_id) DO UPDATE SET
           embedding_contract = excluded.embedding_contract,
           embedding_model = excluded.embedding_model,
           dimensions = excluded.dimensions,
           embedding = excluded.embedding,
           input_hash = excluded.input_hash,
           updated_at = excluded.updated_at",
        rusqlite::params![
            row.chunk_id,
            row.generation_id,
            row.embedding_contract,
            row.embedding_model,
            row.dimensions as i64,
            row.embedding,
            row.input_hash,
            now_ms
        ],
    )
    .map_err(|error| BibliographyError::sql("Failed to upsert chunk embedding", error))?;
    Ok(())
}

/// Vectors already stored for this work's current chunks under the same
/// `(model, contract, dimensions)`, keyed by chunk text hash. A re-profile
/// reuses them instead of paying the provider again; a different model or
/// contract never matches, and neither does a chunk whose text moved (its
/// hash no longer equals the stamp the vector was computed from).
pub fn reusable_chunk_embeddings(
    conn: &Connection,
    item_id: &str,
    model: &str,
    contract: &str,
    dimensions: usize,
) -> BibliographyResult<std::collections::HashMap<String, Vec<u8>>> {
    let mut statement = conn
        .prepare(
            "SELECT e.input_hash, e.embedding
               FROM bibliographic_chunk_embeddings e
               JOIN bibliographic_chunks c ON c.id = e.chunk_id
              WHERE c.item_id = ?1
                AND e.input_hash = c.text_hash
                AND e.embedding_model = ?2
                AND e.embedding_contract = ?3
                AND e.dimensions = ?4",
        )
        .map_err(|error| BibliographyError::sql("Failed to read reusable chunk vectors", error))?;
    let rows = statement
        .query_map(
            rusqlite::params![item_id, model, contract, dimensions as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .map_err(|error| BibliographyError::sql("Failed to read reusable chunk vectors", error))?;
    let mut reusable = std::collections::HashMap::new();
    for row in rows {
        let (hash, blob) = row.map_err(|error| {
            BibliographyError::sql("Failed to read a reusable chunk vector", error)
        })?;
        reusable.insert(hash, blob);
    }
    Ok(reusable)
}

// ── Page texts for chunking (E4c-WU2) ──────────────────────────────────────

/// One page text preferred for chunking: the OCR row when one exists,
/// otherwise the native row. Unreadable and empty texts never chunk.
pub struct ChunkablePage {
    pub attachment_id: String,
    pub page_number: i64,
    pub text_content: String,
}

/// Every chunkable page of one work's attachments, ordered by attachment
/// then page. OCR rows win over native rows per page; unreadable pages
/// (empty text) are skipped — there is nothing to segment.
pub fn chunkable_pages_for_item(
    conn: &Connection,
    item_id: &str,
) -> BibliographyResult<Vec<ChunkablePage>> {
    require_non_empty(item_id, "item id")?;
    let mut stmt = conn
        .prepare(
            "SELECT p.attachment_id, p.page_number, p.text_content
             FROM bibliographic_page_texts p
             JOIN zotero_attachments a ON a.id = p.attachment_id
             WHERE a.item_id = ?1 AND p.text_content != ''
             ORDER BY p.attachment_id, p.page_number,
                      CASE p.method WHEN 'ocr' THEN 0 ELSE 1 END",
        )
        .map_err(|error| BibliographyError::sql("Failed to read chunkable pages", error))?;
    let rows = stmt
        .query_map([item_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| BibliographyError::sql("Failed to read chunkable pages", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| BibliographyError::sql("Failed to read chunkable pages", error))?;
    drop(stmt);
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<(String, i64)> = std::collections::HashSet::new();
    for (attachment_id, page_number, text_content) in rows {
        if seen.insert((attachment_id.clone(), page_number)) {
            out.push(ChunkablePage {
                attachment_id,
                page_number,
                text_content,
            });
        }
    }
    Ok(out)
}
