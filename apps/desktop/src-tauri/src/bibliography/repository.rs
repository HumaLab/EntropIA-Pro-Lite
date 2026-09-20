//! Transactional persistence for the E1b-1a Zotero catalog foundation.
//!
//! A connection is a source namespace. A library qualifies the native Zotero
//! item key, so the same key in two libraries remains two catalog rows. Native
//! Zotero JSON and CSL JSON are stored as separate snapshots; CSL `id` is never
//! used as a substitute for the native key.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const MIGRATION_NAME: &str = "0038_bibliography_catalog";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BibliographyError {
    pub code: String,
    pub message: String,
}

impl BibliographyError {
    fn new(code: &str, message: impl Into<String>) -> Self {
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

fn read_item(
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
    tx.commit()
        .map_err(|error| BibliographyError::sql("Failed to commit bibliographic item", error))?;
    Ok(item)
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
        .expect("apply bibliography migration");

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
