//! What the tools may read, and through which connection.
//!
//! Reads never touch the UI connection: every call opens its own connection
//! from the archive path with `PRAGMA query_only = ON`, so a tool cannot write
//! the archive even by mistake ([D10](../../../../../odd/tasks/agent-mcp-connector.md)).
//! The connection also carries the archive's standard settings, because it is
//! opened by [`crate::db::open::open_archive_connection`] like every other one.
//!
//! Lists page by keyset: the cursor is the opaque encoding of the last record's
//! sort key and id, so a page never repeats or skips a record even when rows are
//! inserted between calls. `limit` is refused outside `1..=MAX_LIMIT` instead of
//! being clamped, because silently answering a different question than the one
//! asked is worse than saying so.

use std::path::PathBuf;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rusqlite::{params, Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Page size when the caller names none (D10).
pub const DEFAULT_LIMIT: usize = 50;

/// Largest page a caller may ask for (D10).
pub const MAX_LIMIT: usize = 200;

/// Why a read failed, in the terms the tool contract uses (D10, S7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The caller's arguments cannot be used as given.
    InvalidArgument(String),
    /// No record has that id.
    NotFound(String),
    /// The archive could not be read right now.
    Unavailable(String),
}

impl ReadError {
    /// The stable code the tool result and the audit line carry.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "invalid_argument",
            Self::NotFound(_) => "not_found",
            Self::Unavailable(_) => "unavailable",
        }
    }

    /// The one sentence the assistant shows the person.
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidArgument(message)
            | Self::NotFound(message)
            | Self::Unavailable(message) => message,
        }
    }
}

/// How many collections and items the archive holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCounts {
    pub collections: i64,
    pub items: i64,
}

/// One collection, as a list record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub item_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One item, as a list record. `updated_at` is the revision the caller compares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemRecord {
    pub id: String,
    pub title: String,
    pub collection_id: String,
    pub updated_at: i64,
}

/// One file attached to an item: metadata only, never the extracted text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssetRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub asset_type: String,
    pub path: String,
    pub sort_index: i64,
    pub size: Option<i64>,
    pub page_number: Option<i64>,
    pub created_at: i64,
}

/// An item with its metadata and its assets (D10: no full text).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetail {
    pub id: String,
    pub title: String,
    pub collection_id: String,
    pub metadata: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
    pub assets: Vec<AssetRecord>,
}

/// One page of collections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionsPage {
    pub records: Vec<CollectionRecord>,
    pub next_cursor: Option<String>,
}

/// One page of items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemsPage {
    pub records: Vec<ItemRecord>,
    pub next_cursor: Option<String>,
}

/// One search hit: the record plus its BM25 position.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchRecord {
    pub id: String,
    pub title: String,
    pub collection_id: String,
    pub updated_at: i64,
    pub rank: f64,
}

/// The search page. `matches` counts every hit before the limit, so an
/// assistant can tell the person that the list it saw was truncated.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub records: Vec<SearchRecord>,
    pub matches: usize,
}

/// The last record a page handed out, encoded so only this module decodes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cursor {
    /// The record's sort key (`name` for collections, `title` for items).
    key: String,
    /// The record's id, which breaks ties between equal sort keys.
    id: String,
}

/// Read-only access to one archive.
#[derive(Debug, Clone)]
pub struct ArchiveReader {
    db_path: PathBuf,
}

impl ArchiveReader {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }

    /// The dedicated read connection D10 requires: the archive's standard
    /// settings, then `query_only`, so every write statement fails.
    pub fn connection(&self) -> Result<Connection, ReadError> {
        let conn = crate::db::open::open_archive_connection(&self.db_path)
            .map_err(|error| unavailable(format!("No se pudo abrir el archivo: {error}")))?;
        conn.execute_batch("PRAGMA query_only = ON;")
            .map_err(|error| {
                unavailable(format!(
                    "No se pudo abrir el archivo en solo lectura: {error}"
                ))
            })?;
        Ok(conn)
    }

    /// How much the archive holds, for `entropia_status`.
    pub fn counts(&self) -> Result<ArchiveCounts, ReadError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT (SELECT COUNT(*) FROM collections), (SELECT COUNT(*) FROM items)",
            [],
            |row| {
                Ok(ArchiveCounts {
                    collections: row.get(0)?,
                    items: row.get(1)?,
                })
            },
        )
        .map_err(|error| unavailable(format!("No se pudo contar el archivo: {error}")))
    }

    /// One page of collections, ordered by name (case-insensitive) and id.
    pub fn collections(
        &self,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<CollectionsPage, ReadError> {
        let limit = resolve_limit(limit)?;
        let cursor = cursor.map(decode_cursor).transpose()?;
        let conn = self.connection()?;
        let mut statement = conn
            .prepare(
                "SELECT c.id, c.name, c.description, c.created_at, c.updated_at,
                        (SELECT COUNT(*) FROM items i WHERE i.collection_id = c.id)
                 FROM collections c
                 WHERE ?1 IS NULL OR (c.name COLLATE NOCASE, c.id) > (?1, ?2)
                 ORDER BY c.name COLLATE NOCASE, c.id
                 LIMIT ?3",
            )
            .map_err(|error| unavailable(format!("No se pudo leer las colecciones: {error}")))?;
        let rows = statement
            .query_map(
                params![cursor_key(&cursor), cursor_id(&cursor), page_len(limit)],
                |row| {
                    Ok(CollectionRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        item_count: row.get(5)?,
                    })
                },
            )
            .map_err(|error| unavailable(format!("No se pudo leer las colecciones: {error}")))?;
        let mut records = collect(rows, "las colecciones")?;
        let next_cursor = finish_page(&mut records, limit, |record: &CollectionRecord| {
            (record.name.as_str(), record.id.as_str())
        });
        Ok(CollectionsPage {
            records,
            next_cursor,
        })
    }

    /// One page of items, optionally only those of one collection. The order is
    /// the one `idx_items_collection_title` covers.
    pub fn items(
        &self,
        collection_id: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<ItemsPage, ReadError> {
        let limit = resolve_limit(limit)?;
        let cursor = cursor.map(decode_cursor).transpose()?;
        let conn = self.connection()?;
        let mut statement = conn
            .prepare(
                "SELECT i.id, i.title, i.collection_id, i.updated_at
                 FROM items i
                 WHERE (?1 IS NULL OR i.collection_id = ?1)
                   AND (?2 IS NULL OR (i.title COLLATE NOCASE, i.id) > (?2, ?3))
                 ORDER BY i.title COLLATE NOCASE, i.id
                 LIMIT ?4",
            )
            .map_err(|error| unavailable(format!("No se pudo leer los ítems: {error}")))?;
        let rows = statement
            .query_map(
                params![
                    collection_id,
                    cursor_key(&cursor),
                    cursor_id(&cursor),
                    page_len(limit)
                ],
                |row| {
                    Ok(ItemRecord {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        collection_id: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .map_err(|error| unavailable(format!("No se pudo leer los ítems: {error}")))?;
        let mut records = collect(rows, "los ítems")?;
        let next_cursor = finish_page(&mut records, limit, |record: &ItemRecord| {
            (record.title.as_str(), record.id.as_str())
        });
        Ok(ItemsPage {
            records,
            next_cursor,
        })
    }

    /// One item with its metadata and assets, or [`ReadError::NotFound`].
    pub fn item(&self, item_id: &str) -> Result<ItemDetail, ReadError> {
        let conn = self.connection()?;
        let row = conn
            .query_row(
                "SELECT id, title, collection_id, metadata, created_at, updated_at
                 FROM items WHERE id = ?1",
                params![item_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| unavailable(format!("No se pudo leer el ítem: {error}")))?;
        let Some((id, title, collection_id, metadata, created_at, updated_at)) = row else {
            return Err(ReadError::NotFound(format!(
                "No existe ningún ítem con el id «{item_id}» en este archivo."
            )));
        };
        let mut statement = conn
            .prepare(
                "SELECT id, type, path, sort_index, size, page_number, created_at
                 FROM assets WHERE item_id = ?1
                 ORDER BY sort_index, page_number, id",
            )
            .map_err(|error| unavailable(format!("No se pudieron leer los archivos: {error}")))?;
        let rows = statement
            .query_map(params![id], |row| {
                Ok(AssetRecord {
                    id: row.get(0)?,
                    asset_type: row.get(1)?,
                    path: row.get(2)?,
                    sort_index: row.get(3)?,
                    size: row.get(4)?,
                    page_number: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|error| unavailable(format!("No se pudieron leer los archivos: {error}")))?;
        let assets = collect(rows, "los archivos del ítem")?;
        Ok(ItemDetail {
            id,
            title,
            collection_id,
            metadata: metadata_value(metadata),
            created_at,
            updated_at,
            assets,
        })
    }

    /// Full-text search through the archive's FTS5 index, optionally scoped to
    /// one collection. Ordered by BM25, so the first `limit` hits are the best.
    pub fn search(
        &self,
        query: &str,
        collection_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<SearchResults, ReadError> {
        let limit = resolve_limit(limit)?;
        let query = query.trim();
        if query.is_empty() {
            return Err(ReadError::InvalidArgument(
                "La búsqueda necesita un texto: escribí qué querés encontrar.".to_string(),
            ));
        }
        let conn = self.connection()?;
        let hits = crate::nlp::fts::fts_search_with_mode(
            &conn,
            query,
            collection_id,
            crate::nlp::fts::FtsMatchMode::Any,
        )
        .map_err(|error| unavailable(format!("No se pudo buscar en el archivo: {error}")))?;
        let matches = hits.len();
        let mut records = Vec::new();
        for hit in hits.into_iter().take(limit) {
            let Some(record) = lookup_item(&conn, &hit.item_id, hit.rank)? else {
                continue; // the index outlived a deleted item; skip it
            };
            records.push(record);
        }
        Ok(SearchResults { records, matches })
    }
}

/// The item behind one search hit, or `None` when it no longer exists.
fn lookup_item(
    conn: &Connection,
    item_id: &str,
    rank: f64,
) -> Result<Option<SearchRecord>, ReadError> {
    conn.query_row(
        "SELECT id, title, collection_id, updated_at FROM items WHERE id = ?1",
        params![item_id],
        |row| {
            Ok(SearchRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                collection_id: row.get(2)?,
                updated_at: row.get(3)?,
                rank,
            })
        },
    )
    .optional()
    .map_err(|error| unavailable(format!("No se pudo leer un resultado: {error}")))
}

/// `metadata` is a JSON blob column. Parse it when it is JSON and hand the raw
/// string over when it is not, so no stored value is ever dropped.
fn metadata_value(raw: Option<String>) -> serde_json::Value {
    match raw {
        None => serde_json::Value::Null,
        Some(raw) => serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw)),
    }
}

/// The page size a caller asked for: the default when silent, refused when it
/// is outside `1..=MAX_LIMIT`.
fn resolve_limit(limit: Option<u32>) -> Result<usize, ReadError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT as u32) as usize;
    if limit == 0 || limit > MAX_LIMIT {
        return Err(ReadError::InvalidArgument(format!(
            "El límite tiene que estar entre 1 y {MAX_LIMIT}: llegó {limit}."
        )));
    }
    Ok(limit)
}

/// How many rows to ask SQLite for: one extra tells us whether a next page
/// exists without a second query.
fn page_len(limit: usize) -> i64 {
    (limit + 1) as i64
}

fn cursor_key(cursor: &Option<Cursor>) -> Option<&str> {
    cursor.as_ref().map(|cursor| cursor.key.as_str())
}

fn cursor_id(cursor: &Option<Cursor>) -> Option<&str> {
    cursor.as_ref().map(|cursor| cursor.id.as_str())
}

fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
    what: &str,
) -> Result<Vec<T>, ReadError> {
    rows.collect::<Result<Vec<T>, _>>()
        .map_err(|error| unavailable(format!("No se pudo leer {what}: {error}")))
}

/// Take the extra row off the page and turn the last kept record into the cursor
/// for the next one. `None` when the page ended exactly at the last record.
fn finish_page<T>(
    records: &mut Vec<T>,
    limit: usize,
    key: fn(&T) -> (&str, &str),
) -> Option<String> {
    if records.len() <= limit {
        return None;
    }
    records.truncate(limit);
    let last = records.last()?;
    let (key, id) = key(last);
    Some(encode_cursor(&Cursor {
        key: key.to_string(),
        id: id.to_string(),
    }))
}

/// Encode a cursor so a caller can only hand it back, never build one.
fn encode_cursor(cursor: &Cursor) -> String {
    let bytes = serde_json::to_vec(cursor).unwrap_or_default();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_cursor(raw: &str) -> Result<Cursor, ReadError> {
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid_cursor())?;
    serde_json::from_slice(&bytes).map_err(|_| invalid_cursor())
}

fn invalid_cursor() -> ReadError {
    ReadError::InvalidArgument(
        "El cursor no es válido: pasá tal cual el `nextCursor` que devolvió la página anterior."
            .to_string(),
    )
}

fn unavailable(message: String) -> ReadError {
    ReadError::Unavailable(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::test_archive;

    #[test]
    fn the_tools_read_through_a_connection_that_cannot_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = test_archive::seeded_archive(dir.path());
        let reader = ArchiveReader::new(&db_path);
        let conn = reader.connection().expect("open the read connection");

        let query_only: i64 = conn
            .query_row("PRAGMA query_only", [], |row| row.get(0))
            .expect("read query_only");
        assert_eq!(query_only, 1, "D10: the tool connection is query-only");

        let write = conn.execute(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES ('col_new', 'Nueva', 1, 1)",
            [],
        );
        let error = write.expect_err("a write through the read connection must fail");
        assert!(
            error.to_string().contains("readonly"),
            "SQLite refuses the write because the connection is read-only: {error}"
        );

        let stored: i64 = reader
            .connection()
            .expect("second read connection")
            .query_row("SELECT COUNT(*) FROM collections", [], |row| row.get(0))
            .expect("count collections");
        assert_eq!(stored, 2, "the refused write changed nothing");
    }

    #[test]
    fn a_cursor_round_trips_and_a_made_up_one_is_refused() {
        let cursor = Cursor {
            key: "Alfa".to_string(),
            id: "col_alfa".to_string(),
        };
        let encoded = encode_cursor(&cursor);
        assert_eq!(decode_cursor(&encoded).expect("decode"), cursor);
        assert!(matches!(
            decode_cursor("no-es-un-cursor"),
            Err(ReadError::InvalidArgument(_))
        ));
    }

    #[test]
    fn the_limit_is_the_documented_default_and_outside_the_range_is_refused() {
        assert_eq!(resolve_limit(None).unwrap(), DEFAULT_LIMIT);
        assert_eq!(resolve_limit(Some(1)).unwrap(), 1);
        assert_eq!(resolve_limit(Some(MAX_LIMIT as u32)).unwrap(), MAX_LIMIT);
        for refused in [0, MAX_LIMIT as u32 + 1] {
            let error = resolve_limit(Some(refused)).unwrap_err();
            assert_eq!(error.code(), "invalid_argument", "{refused}");
            assert!(error.message().contains("entre 1 y 200"), "{error:?}");
        }
    }

    #[test]
    fn metadata_is_parsed_when_it_is_json_and_kept_when_it_is_not() {
        assert_eq!(metadata_value(None), serde_json::Value::Null);
        assert_eq!(
            metadata_value(Some(r#"{"fuente":"prueba"}"#.to_string())),
            serde_json::json!({ "fuente": "prueba" })
        );
        assert_eq!(
            metadata_value(Some("texto suelto".to_string())),
            serde_json::json!("texto suelto")
        );
    }
}
