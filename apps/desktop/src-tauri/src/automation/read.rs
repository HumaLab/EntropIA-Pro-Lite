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

/// One passage that matched a `search_passages` query, located for citing.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PassageRecord {
    pub item_id: String,
    pub title: String,
    pub collection_id: String,
    pub asset_id: String,
    pub page_number: Option<i64>,
    /// `extraction` or `transcription`: which page text the offsets index.
    pub source: String,
    /// Unicode-character offsets into that page text.
    pub start_char: i64,
    pub end_char: i64,
    /// sha256 of the page text the passage was cut from.
    pub text_hash: String,
    /// Whether that page text is still the current one; `false` means the
    /// page was re-read or corrected after the passage was indexed.
    pub current: bool,
    pub text: String,
    pub rank: f64,
}

/// How much of the scope the passage index covers: passages exist only for
/// documents processed with embeddings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PassageCoverage {
    pub items_in_scope: usize,
    pub items_with_passages: usize,
}

/// `search_passages`'s payload.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PassageResults {
    pub records: Vec<PassageRecord>,
    /// Every matching passage before the limit.
    pub matches: usize,
    pub coverage: PassageCoverage,
}

/// Characters a `documents_get_text` call returns when the caller names none.
pub const DEFAULT_TEXT_WINDOW: usize = 4000;

/// Largest text window one call may ask for.
pub const MAX_TEXT_WINDOW: usize = 20_000;

/// The item a text read belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub id: String,
    pub title: String,
}

/// One timed stretch of a transcription, in milliseconds from the start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// One window of one page's text, located so it can be cited.
///
/// Offsets count Unicode characters of the whole page text, the unit the
/// passage index uses, and `textHash` is sha256 of that text: the same value a
/// passage carries as its source hash, so a citation can tell whether the text
/// it points into is still the current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageWindow {
    pub asset_id: String,
    #[serde(rename = "type")]
    pub asset_type: String,
    /// The page of the original document, when the file is one page of it.
    pub page_number: Option<i64>,
    /// `extraction` (native text or OCR) or `transcription` (audio).
    pub source: String,
    /// For an extraction: `native` or `ocr`. `null` for a transcription.
    pub method: Option<String>,
    pub text_hash: String,
    pub total_chars: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub text: String,
    /// The transcription's timed segments, when the page is audio.
    pub segments: Option<Vec<TranscriptSegment>>,
}

/// `documents_get_text`'s payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentText {
    pub item: ItemRef,
    /// The pages with text, in the item's file order. Files without text yet
    /// (not extracted, or nothing readable) are left out; `items_get` lists all.
    pub pages: Vec<PageWindow>,
    /// Hand it back as `cursor` for the next window; `null` at the end.
    pub next_cursor: Option<String>,
}

/// Where the next text window starts: the asset (by position and id, so a
/// changed document is noticed) and the character offset inside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TextCursor {
    i: usize,
    a: String,
    o: usize,
}

/// One page's whole text and where it came from.
struct PageSource {
    source: &'static str,
    method: Option<String>,
    text: String,
    segments: Option<Vec<TranscriptSegment>>,
}

#[derive(Deserialize)]
struct StoredSegment {
    start_ms: i64,
    end_ms: i64,
    text: String,
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// The page text the app itself cites (`lib/page-text.ts`): the latest
/// extraction when it has text, otherwise the latest transcription.
fn page_source(conn: &Connection, asset_id: &str) -> Result<Option<PageSource>, ReadError> {
    let extraction = conn
        .query_row(
            "SELECT text_content, method FROM extractions WHERE asset_id = ?1
             ORDER BY created_at DESC, id DESC LIMIT 1",
            params![asset_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| unavailable("leer el texto extraído", error))?;
    if let Some((text, method)) = extraction {
        if !text.trim().is_empty() {
            return Ok(Some(PageSource {
                source: "extraction",
                method: Some(method),
                text,
                segments: None,
            }));
        }
    }
    let transcription = conn
        .query_row(
            "SELECT text_content, segments FROM transcriptions WHERE asset_id = ?1
             ORDER BY created_at DESC, id DESC LIMIT 1",
            params![asset_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|error| unavailable("leer la transcripción", error))?;
    Ok(transcription
        .filter(|(text, _)| !text.trim().is_empty())
        .map(|(text, segments)| PageSource {
            source: "transcription",
            method: None,
            text,
            segments: segments
                .and_then(|json| serde_json::from_str::<Vec<StoredSegment>>(&json).ok())
                .map(|stored| {
                    stored
                        .into_iter()
                        .map(|segment| TranscriptSegment {
                            start_ms: segment.start_ms,
                            end_ms: segment.end_ms,
                            text: segment.text,
                        })
                        .collect()
                }),
        }))
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
            .map_err(|error| unavailable("abrir el archivo", error))?;
        conn.execute_batch("PRAGMA query_only = ON;")
            .map_err(|error| unavailable("abrir el archivo en solo lectura", error))?;
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
        .map_err(|error| unavailable("contar el archivo", error))
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
            .map_err(|error| unavailable("leer las colecciones", error))?;
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
            .map_err(|error| unavailable("leer las colecciones", error))?;
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
            .map_err(|error| unavailable("leer los ítems", error))?;
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
            .map_err(|error| unavailable("leer los ítems", error))?;
        let mut records = collect(rows, "los ítems")?;
        let next_cursor = finish_page(&mut records, limit, |record: &ItemRecord| {
            (record.title.as_str(), record.id.as_str())
        });
        Ok(ItemsPage {
            records,
            next_cursor,
        })
    }

    /// An item's page texts in file order, one window of at most `max_chars`
    /// characters per call across pages (S4: never the whole file by default).
    /// `asset_id` narrows the read to one of the item's files.
    pub fn document_text(
        &self,
        item_id: &str,
        asset_id: Option<&str>,
        cursor: Option<&str>,
        max_chars: Option<u32>,
    ) -> Result<DocumentText, ReadError> {
        let budget = max_chars.map_or(DEFAULT_TEXT_WINDOW, |max| max as usize);
        if budget == 0 || budget > MAX_TEXT_WINDOW {
            return Err(ReadError::InvalidArgument(format!(
                "`maxChars` tiene que estar entre 1 y {MAX_TEXT_WINDOW}: llegó {budget}."
            )));
        }
        let start = cursor
            .map(|raw| {
                URL_SAFE_NO_PAD
                    .decode(raw)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<TextCursor>(&bytes).ok())
                    .ok_or_else(invalid_cursor)
            })
            .transpose()?;
        let conn = self.connection()?;
        let title = conn
            .query_row(
                "SELECT title FROM items WHERE id = ?1",
                params![item_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| unavailable("leer el ítem", error))?
            .ok_or_else(|| {
                ReadError::NotFound(format!(
                    "No existe ningún ítem con el id «{item_id}» en este archivo."
                ))
            })?;
        let mut statement = conn
            .prepare(
                "SELECT id, type, page_number FROM assets WHERE item_id = ?1
                 ORDER BY sort_index, page_number, id",
            )
            .map_err(|error| unavailable("leer los archivos del ítem", error))?;
        let mut assets: Vec<(String, String, Option<i64>)> = collect(
            statement
                .query_map(params![item_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(|error| unavailable("leer los archivos del ítem", error))?,
            "los archivos del ítem",
        )?;
        drop(statement);
        if let Some(asset_id) = asset_id {
            assets.retain(|(id, _, _)| id == asset_id);
            if assets.is_empty() {
                return Err(ReadError::NotFound(format!(
                    "El ítem «{item_id}» no tiene un archivo con el id «{asset_id}»."
                )));
            }
        }
        let (mut index, mut offset) = match &start {
            Some(cursor) => {
                if assets.get(cursor.i).map(|(id, _, _)| id.as_str()) != Some(cursor.a.as_str()) {
                    return Err(ReadError::InvalidArgument(
                        "El cursor ya no corresponde a este documento (cambiaron sus archivos): volvé a leerlo sin cursor."
                            .to_string(),
                    ));
                }
                (cursor.i, cursor.o)
            }
            None => (0, 0),
        };
        let mut remaining = budget;
        let mut pages = Vec::new();
        let mut next_cursor = None;
        while index < assets.len() {
            let (id, asset_type, page_number) = &assets[index];
            let Some(page) = page_source(&conn, id)? else {
                index += 1;
                offset = 0;
                continue;
            };
            let chars: Vec<char> = page.text.chars().collect();
            let total = chars.len();
            if offset >= total {
                index += 1;
                offset = 0;
                continue;
            }
            if remaining == 0 {
                next_cursor = Some(
                    URL_SAFE_NO_PAD.encode(
                        serde_json::to_vec(&TextCursor {
                            i: index,
                            a: id.clone(),
                            o: offset,
                        })
                        .unwrap_or_default(),
                    ),
                );
                break;
            }
            let end = (offset + remaining).min(total);
            pages.push(PageWindow {
                asset_id: id.clone(),
                asset_type: asset_type.clone(),
                page_number: *page_number,
                source: page.source.to_string(),
                method: page.method,
                text_hash: sha256_hex(&page.text),
                total_chars: total,
                start_char: offset,
                end_char: end,
                text: chars[offset..end].iter().collect(),
                segments: page.segments,
            });
            remaining -= end - offset;
            offset = end;
        }
        Ok(DocumentText {
            item: ItemRef {
                id: item_id.to_string(),
                title,
            },
            pages,
            next_cursor,
        })
    }

    /// Passages that match `query`, best first (BM25 over the chunk index the
    /// app's own chat uses), narrowed to a collection or an item, with how many
    /// items in that scope have passages at all.
    pub fn passages(
        &self,
        query: &str,
        collection_id: Option<&str>,
        item_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<PassageResults, ReadError> {
        let limit = resolve_limit(limit)?;
        let query = query.trim();
        if query.is_empty() {
            return Err(ReadError::InvalidArgument(
                "La búsqueda necesita un texto: escribí qué querés encontrar.".to_string(),
            ));
        }
        let conn = self.connection()?;
        let scope = params![collection_id, item_id];
        let failed = |error: rusqlite::Error| unavailable("buscar pasajes", error);
        let items_in_scope: i64 = conn
            .query_row(
                "SELECT count(*) FROM items i
                 WHERE (?1 IS NULL OR i.collection_id = ?1) AND (?2 IS NULL OR i.id = ?2)",
                scope,
                |row| row.get(0),
            )
            .map_err(failed)?;
        let items_with_passages: i64 = conn
            .query_row(
                "SELECT count(DISTINCT c.item_id) FROM rag_chunks c JOIN items i ON i.id = c.item_id
                 WHERE (?1 IS NULL OR i.collection_id = ?1) AND (?2 IS NULL OR i.id = ?2)",
                scope,
                |row| row.get(0),
            )
            .map_err(failed)?;
        let coverage = PassageCoverage {
            items_in_scope: items_in_scope.max(0) as usize,
            items_with_passages: items_with_passages.max(0) as usize,
        };
        let expression = crate::nlp::fts::sanitize_fts5_query_with_mode(
            query,
            crate::nlp::fts::FtsMatchMode::Any,
        );
        if expression.is_empty() {
            return Ok(PassageResults {
                records: Vec::new(),
                matches: 0,
                coverage,
            });
        }
        let from = "FROM rag_chunks_fts f
             JOIN rag_chunks c ON c.id = f.chunk_id
             JOIN items i ON i.id = c.item_id
             JOIN assets a ON a.id = c.asset_id
             WHERE rag_chunks_fts MATCH ?1
               AND (?2 IS NULL OR i.collection_id = ?2) AND (?3 IS NULL OR c.item_id = ?3)";
        let matches: i64 = conn
            .query_row(
                &format!("SELECT count(*) {from}"),
                params![expression, collection_id, item_id],
                |row| row.get(0),
            )
            .map_err(failed)?;
        let mut statement = conn
            .prepare(&format!(
                "SELECT c.item_id, i.title, i.collection_id, c.asset_id, a.page_number,
                        c.source_kind, c.start_char, c.end_char, c.source_text_hash,
                        c.text_content, bm25(rag_chunks_fts) AS rank
                 {from} ORDER BY rank LIMIT {limit}"
            ))
            .map_err(failed)?;
        let hits = collect(
            statement
                .query_map(params![expression, collection_id, item_id], |row| {
                    Ok(PassageRecord {
                        item_id: row.get(0)?,
                        title: row.get(1)?,
                        collection_id: row.get(2)?,
                        asset_id: row.get(3)?,
                        page_number: row.get(4)?,
                        source: row.get(5)?,
                        start_char: row.get(6)?,
                        end_char: row.get(7)?,
                        text_hash: row.get(8)?,
                        current: false,
                        text: row.get(9)?,
                        rank: row.get(10)?,
                    })
                })
                .map_err(failed)?,
            "los pasajes",
        )?;
        drop(statement);
        // A passage is current when the page text it indexes is still the one
        // documents_get_text reads; one hash per page, however many passages.
        let mut current_hash: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        let mut records = Vec::with_capacity(hits.len());
        for mut record in hits {
            let hash = match current_hash.get(&record.asset_id) {
                Some(hash) => hash.clone(),
                None => {
                    let hash =
                        page_source(&conn, &record.asset_id)?.map(|page| sha256_hex(&page.text));
                    current_hash.insert(record.asset_id.clone(), hash.clone());
                    hash
                }
            };
            record.current = hash.as_deref() == Some(record.text_hash.as_str());
            records.push(record);
        }
        Ok(PassageResults {
            records,
            matches: matches.max(0) as usize,
            coverage,
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
            .map_err(|error| unavailable("leer el ítem", error))?;
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
            .map_err(|error| unavailable("leer los archivos del ítem", error))?;
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
            .map_err(|error| unavailable("leer los archivos del ítem", error))?;
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
        // Same tokenization as the app's own keyword search; an expression with
        // nothing left after sanitizing matches nothing.
        let expression = crate::nlp::fts::sanitize_fts5_query_with_mode(
            query,
            crate::nlp::fts::FtsMatchMode::Any,
        );
        if expression.is_empty() {
            return Ok(SearchResults {
                records: Vec::new(),
                matches: 0,
            });
        }
        let conn = self.connection()?;
        let (hits, matches) = ranked_hits(&conn, &expression, collection_id, limit)?;
        let mut records = Vec::new();
        for (item_id, rank) in hits {
            let Some(record) = lookup_item(&conn, &item_id, rank)? else {
                continue; // the index outlived a deleted item; skip it
            };
            records.push(record);
        }
        Ok(SearchResults { records, matches })
    }
}

/// The best `limit` hits of an FTS5 `expression`, by BM25, and how many items
/// match in all. The limit is applied in SQL and the total is a count, so one
/// common term never makes a call buffer and rank every matching row.
fn ranked_hits(
    conn: &Connection,
    expression: &str,
    collection_id: Option<&str>,
    limit: usize,
) -> Result<(Vec<(String, f64)>, usize), ReadError> {
    let scope = if collection_id.is_some() {
        " AND i.collection_id = ?2"
    } else {
        ""
    };
    let from = format!(
        "FROM fts_items f JOIN items i ON i.rowid = f.rowid WHERE fts_items MATCH ?1{scope}"
    );
    let search = |error: rusqlite::Error| unavailable("buscar en el archivo", error);
    let count_sql = format!("SELECT count(*) {from}");
    let page_sql =
        format!("SELECT i.id, bm25(fts_items) AS rank {from} ORDER BY rank LIMIT {limit}");
    let (matches, hits): (i64, Vec<(String, f64)>) = match collection_id {
        Some(cid) => (
            conn.query_row(&count_sql, params![expression, cid], |row| row.get(0))
                .map_err(search)?,
            conn.prepare(&page_sql)
                .and_then(|mut stmt| {
                    stmt.query_map(params![expression, cid], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?
                    .collect()
                })
                .map_err(search)?,
        ),
        None => (
            conn.query_row(&count_sql, params![expression], |row| row.get(0))
                .map_err(search)?,
            conn.prepare(&page_sql)
                .and_then(|mut stmt| {
                    stmt.query_map(params![expression], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect()
                })
                .map_err(search)?,
        ),
    };
    Ok((hits, matches.max(0) as usize))
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
    .map_err(|error| unavailable("leer un resultado de la búsqueda", error))
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
        .map_err(|error| unavailable(&format!("leer {what}"), error))
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

/// Turn one failed archive read into the sentence the assistant shows.
///
/// The detail stays in the app's log: a `rusqlite` (or `open_archive_connection`)
/// error names the archive's absolute path, and the answer travels to the
/// assistant and its provider (S7). The message therefore says what could not be
/// done and what the person can do about it, and nothing about where the file is.
fn unavailable(action: &str, error: impl std::fmt::Display) -> ReadError {
    log::warn!(target: "automation", "the archive read failed while trying to {action}: {error}");
    ReadError::Unavailable(format!(
        "No se pudo {action} en este momento. Reintentá en unos segundos; si el problema sigue, revisá el archivo desde EntropIA."
    ))
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
    fn an_unavailable_archive_is_reported_without_naming_its_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory that does not exist: opening fails with the detail that
        // names the absolute path, which must stay in the app's log (S7).
        let reader = ArchiveReader::new(dir.path().join("no-existe").join("entropia.sqlite"));

        let error = reader
            .counts()
            .expect_err("a missing directory cannot be opened");

        assert_eq!(error.code(), "unavailable");
        let message = error.message();
        let path = dir.path().to_string_lossy();
        assert!(
            !message.contains(path.as_ref()),
            "the answer must not carry the archive path: {message:?}"
        );
        assert!(
            !message.contains('/') && !message.contains('\\'),
            "the answer must not look like a path: {message:?}"
        );
        assert!(
            !message.to_lowercase().contains("failed to open"),
            "the rusqlite detail stays in the log: {message:?}"
        );
        assert!(
            message.contains("EntropIA"),
            "the sentence still says what to do: {message:?}"
        );
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
