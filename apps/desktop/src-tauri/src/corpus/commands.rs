//! The corpus services and the Tauri commands the app's views call.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::corpus::delete::{delete_collection, delete_items};
use crate::corpus::ingest::{
    check_app_source, import_file, ImportOutcome, ImportOverrides, SourceLimits,
};
use crate::db::state::AppDbState;

/// Longest collection name.
pub const COLLECTION_NAME_MAX_CHARS: usize = 200;
/// Longest collection description.
pub const COLLECTION_DESCRIPTION_MAX_CHARS: usize = 2000;
/// Longest item title.
pub const ITEM_TITLE_MAX_CHARS: usize = 500;
/// Most metadata fields one item may hold.
pub const ITEM_METADATA_MAX_FIELDS: usize = 100;
/// Longest page text an edit may write.
pub const MAX_PAGE_TEXT_CHARS: usize = 200_000;
/// Metadata keys the app keeps for itself (import and web-capture provenance,
/// `lib/item-metadata.ts`). An edit never writes them and always keeps them.
pub const RESERVED_METADATA_PREFIX: &str = "__entropia_";

/// Why a corpus write was refused, in the terms both callers need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusError {
    InvalidArgument(String),
    NotFound(String),
    /// The record is no longer at the revision the caller read.
    RevisionConflict(String),
    /// The archive could not be written; the detail may name a path.
    Unavailable(String),
}

impl std::fmt::Display for CorpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(detail) => write!(f, "invalid_argument: {detail}"),
            Self::NotFound(detail) => write!(f, "not_found: {detail}"),
            Self::RevisionConflict(detail) => write!(f, "revision_conflict: {detail}"),
            Self::Unavailable(detail) => write!(f, "unavailable: {detail}"),
        }
    }
}

fn unavailable(action: &str, error: impl std::fmt::Display) -> CorpusError {
    CorpusError::Unavailable(format!("could not {action}: {error}"))
}

/// One collection as both callers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: i64,
    /// The revision the next update names.
    pub updated_at: i64,
}

/// One item's editable fields after a write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemDto {
    pub id: String,
    pub title: String,
    pub collection_id: String,
    /// The stored JSON object, reserved keys included.
    pub metadata: Option<String>,
    pub created_at: i64,
    /// The revision the next update names.
    pub updated_at: i64,
}

/// What a collection update changes. `description: Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionPatch {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
}

/// What an item update changes.
///
/// `metadata` (the agent's form) replaces every non-reserved field and keeps the
/// reserved ones. `metadata_json` (the app's form) stores the JSON object the
/// app built itself, reserved keys included, exactly as the views always did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPatch {
    pub title: Option<String>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub metadata_json: Option<String>,
}

/// Tells an absent field (`None`) from an explicit `null` (`Some(None)`).
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

fn now_ms() -> i64 {
    crate::processing::repository::now_ms()
}

/// The next revision: now, but always later than the current one, so two
/// writes in the same millisecond still read as two revisions.
fn next_revision(current: i64) -> i64 {
    now_ms().max(current + 1)
}

fn clean_text(
    what: &str,
    value: &str,
    max_chars: usize,
    allow_newlines: bool,
) -> Result<String, CorpusError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(CorpusError::InvalidArgument(format!(
            "the {what} must be 1 to {max_chars} characters"
        )));
    }
    let bad = |c: char| c.is_control() && !(allow_newlines && (c == '\n' || c == '\t'));
    if value.chars().any(bad) {
        return Err(CorpusError::InvalidArgument(format!(
            "the {what} has control characters"
        )));
    }
    Ok(value.to_string())
}

fn clean_description(value: Option<&str>) -> Result<Option<String>, CorpusError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => {
            clean_text("description", value, COLLECTION_DESCRIPTION_MAX_CHARS, true).map(Some)
        }
    }
}

fn read_collection(conn: &Connection, id: &str) -> Result<Option<CollectionDto>, CorpusError> {
    conn.query_row(
        "SELECT id, name, description, created_at, updated_at FROM collections WHERE id = ?1",
        params![id],
        |row| {
            Ok(CollectionDto {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|error| unavailable("read the collection", error))
}

fn read_item(conn: &Connection, id: &str) -> Result<Option<ItemDto>, CorpusError> {
    conn.query_row(
        "SELECT id, title, collection_id, metadata, created_at, updated_at FROM items WHERE id = ?1",
        params![id],
        |row| {
            Ok(ItemDto {
                id: row.get(0)?,
                title: row.get(1)?,
                collection_id: row.get(2)?,
                metadata: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|error| unavailable("read the item", error))
}

fn check_revision(
    what: &str,
    id: &str,
    current: i64,
    expected: Option<i64>,
) -> Result<(), CorpusError> {
    match expected {
        Some(expected) if expected != current => Err(CorpusError::RevisionConflict(format!(
            "{what} {id} is at revision {current}, not {expected}"
        ))),
        _ => Ok(()),
    }
}

/// Create a collection. `id` is minted here unless the app passes its own
/// (the sample collection keeps a fixed one).
pub fn create_collection(
    conn: &Connection,
    name: &str,
    description: Option<&str>,
    id: Option<&str>,
) -> Result<CollectionDto, CorpusError> {
    let name = clean_text("collection name", name, COLLECTION_NAME_MAX_CHARS, false)?;
    let description = clean_description(description)?;
    let id = match id.map(str::trim) {
        Some("") => {
            return Err(CorpusError::InvalidArgument(
                "a collection id cannot be empty".into(),
            ))
        }
        Some(id) => {
            if read_collection(conn, id)?.is_some() {
                return Err(CorpusError::InvalidArgument(format!(
                    "a collection with id {id} already exists"
                )));
            }
            id.to_string()
        }
        None => uuid::Uuid::new_v4().to_string(),
    };
    let now = now_ms();
    conn.execute(
        "INSERT INTO collections (id, name, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![id, name, description, now],
    )
    .map_err(|error| unavailable("create the collection", error))?;
    Ok(CollectionDto {
        id,
        name,
        description,
        created_at: now,
        updated_at: now,
    })
}

/// Rename or re-describe a collection, fenced by `expected` when given.
pub fn update_collection(
    conn: &Connection,
    id: &str,
    patch: &CollectionPatch,
    expected: Option<i64>,
) -> Result<CollectionDto, CorpusError> {
    if patch.name.is_none() && patch.description.is_none() {
        return Err(CorpusError::InvalidArgument(
            "nothing to change: name or description is required".into(),
        ));
    }
    let name = patch
        .name
        .as_deref()
        .map(|name| clean_text("collection name", name, COLLECTION_NAME_MAX_CHARS, false))
        .transpose()?;
    let description = patch
        .description
        .as_ref()
        .map(|value| clean_description(value.as_deref()))
        .transpose()?;
    let current = read_collection(conn, id)?
        .ok_or_else(|| CorpusError::NotFound(format!("no collection {id}")))?;
    check_revision("collection", id, current.updated_at, expected)?;
    let next = CollectionDto {
        name: name.unwrap_or(current.name),
        description: description.unwrap_or(current.description),
        updated_at: next_revision(current.updated_at),
        ..current
    };
    let changed = conn
        .execute(
            "UPDATE collections SET name = ?1, description = ?2, updated_at = ?3
             WHERE id = ?4 AND updated_at = ?5",
            params![
                next.name,
                next.description,
                next.updated_at,
                id,
                current.updated_at
            ],
        )
        .map_err(|error| unavailable("update the collection", error))?;
    if changed == 0 {
        return Err(CorpusError::RevisionConflict(format!(
            "collection {id} changed while it was being updated"
        )));
    }
    Ok(next)
}

/// The stored metadata with `fields` as its non-reserved content and every
/// reserved key of `current` kept.
fn merged_metadata(
    current: Option<&str>,
    fields: &BTreeMap<String, String>,
) -> Result<String, CorpusError> {
    if fields.len() > ITEM_METADATA_MAX_FIELDS {
        return Err(CorpusError::InvalidArgument(format!(
            "an item holds at most {ITEM_METADATA_MAX_FIELDS} metadata fields"
        )));
    }
    let mut merged = serde_json::Map::new();
    for (key, value) in fields {
        let key = key.trim();
        if key.is_empty() || key.chars().any(char::is_control) {
            return Err(CorpusError::InvalidArgument(
                "a metadata field name must be visible text".into(),
            ));
        }
        if key.starts_with(RESERVED_METADATA_PREFIX) {
            return Err(CorpusError::InvalidArgument(format!(
                "metadata fields starting with {RESERVED_METADATA_PREFIX} belong to the app"
            )));
        }
        merged.insert(key.to_string(), serde_json::Value::String(value.clone()));
    }
    let stored = current
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for (key, value) in stored {
        if key.starts_with(RESERVED_METADATA_PREFIX) && value.is_object() {
            merged.insert(key, value);
        }
    }
    serde_json::to_string(&merged).map_err(|error| unavailable("encode the metadata", error))
}

/// Retitle an item or replace its metadata (reserved keys kept), fenced by
/// `expected` when given, and refresh its search entry in the same
/// transaction so the new text is searchable at once.
pub fn update_item(
    conn: &Connection,
    id: &str,
    patch: &ItemPatch,
    expected: Option<i64>,
) -> Result<ItemDto, CorpusError> {
    if patch.title.is_none() && patch.metadata.is_none() && patch.metadata_json.is_none() {
        return Err(CorpusError::InvalidArgument(
            "nothing to change: title or metadata is required".into(),
        ));
    }
    if patch.metadata.is_some() && patch.metadata_json.is_some() {
        return Err(CorpusError::InvalidArgument(
            "send metadata as fields or as JSON, not both".into(),
        ));
    }
    if let Some(json) = &patch.metadata_json {
        let is_object = serde_json::from_str::<serde_json::Value>(json)
            .map(|value| value.is_object())
            .unwrap_or(false);
        if !is_object {
            return Err(CorpusError::InvalidArgument(
                "item metadata must be a JSON object".into(),
            ));
        }
    }
    let title = patch
        .title
        .as_deref()
        .map(|title| clean_text("item title", title, ITEM_TITLE_MAX_CHARS, false))
        .transpose()?;
    let current =
        read_item(conn, id)?.ok_or_else(|| CorpusError::NotFound(format!("no item {id}")))?;
    check_revision("item", id, current.updated_at, expected)?;
    let metadata = match (&patch.metadata, &patch.metadata_json) {
        (Some(fields), _) => Some(merged_metadata(current.metadata.as_deref(), fields)?),
        (None, Some(json)) => Some(json.clone()),
        (None, None) => current.metadata.clone(),
    };
    let next = ItemDto {
        title: title.unwrap_or(current.title),
        metadata,
        updated_at: next_revision(current.updated_at),
        ..current
    };
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| unavailable("begin the item update", error))?;
    let written = (|| {
        let changed = conn
            .execute(
                "UPDATE items SET title = ?1, metadata = ?2, updated_at = ?3
                 WHERE id = ?4 AND updated_at = ?5",
                params![
                    next.title,
                    next.metadata,
                    next.updated_at,
                    id,
                    current.updated_at
                ],
            )
            .map_err(|error| unavailable("update the item", error))?;
        if changed == 0 {
            return Err(CorpusError::RevisionConflict(format!(
                "item {id} changed while it was being updated"
            )));
        }
        crate::nlp::fts::index_item_from_db(conn, id)
            .map_err(|error| unavailable("refresh the search index", error))
    })();
    match written {
        Ok(()) => {
            conn.execute_batch("COMMIT")
                .map_err(|error| unavailable("commit the item update", error))?;
            Ok(next)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Where a replaced page text lives now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplacedText {
    pub item_id: String,
    pub collection_id: String,
    pub asset_id: String,
    /// sha256 of the new page text, the hash `documents_get_text` now reports.
    pub text_hash: String,
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Replace the recognized text of one page (its latest extraction) when that
/// text still has `expected_hash`, keeping the text it had before the first
/// correction in `ocr_correction_backups`, so the app's «Restaurar original»
/// undoes it, and refreshing the item's search entry, all in one transaction.
/// A page whose text is a transcription is refused: audio has no such undo.
pub fn replace_page_text(
    conn: &Connection,
    asset_id: &str,
    expected_hash: &str,
    text: &str,
) -> Result<ReplacedText, CorpusError> {
    if text.trim().is_empty() || text.chars().count() > MAX_PAGE_TEXT_CHARS {
        return Err(CorpusError::InvalidArgument(format!(
            "the page text must be 1 to {MAX_PAGE_TEXT_CHARS} characters"
        )));
    }
    let owner: Option<(String, String)> = conn
        .query_row(
            "SELECT a.item_id, i.collection_id FROM assets a JOIN items i ON i.id = a.item_id
             WHERE a.id = ?1",
            params![asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| unavailable("read the page", error))?;
    let (item_id, collection_id) =
        owner.ok_or_else(|| CorpusError::NotFound(format!("no file {asset_id}")))?;
    crate::llm::ocr_correction::ensure_schema(conn)
        .map_err(|error| unavailable("prepare the text backup", error))?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| unavailable("begin the text replacement", error))?;
    let replaced = (|| {
        let current: Option<(String, String)> = conn
            .query_row(
                "SELECT id, text_content FROM extractions WHERE asset_id = ?1
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                params![asset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| unavailable("read the page text", error))?;
        let Some((extraction_id, current)) = current.filter(|(_, text)| !text.trim().is_empty())
        else {
            return Err(CorpusError::InvalidArgument(format!(
                "file {asset_id} has no recognized text to replace (an audio transcription cannot be replaced this way)"
            )));
        };
        let current_hash = sha256_hex(&current);
        if current_hash != expected_hash {
            return Err(CorpusError::RevisionConflict(format!(
                "the text of file {asset_id} changed since it was read"
            )));
        }
        if current == text {
            return Err(CorpusError::InvalidArgument(
                "the new text is the same as the current one".into(),
            ));
        }
        conn.execute(
            "INSERT OR IGNORE INTO ocr_correction_backups (asset_id, original_text_content, created_at)
             VALUES (?1, ?2, ?3)",
            params![asset_id, current, now_ms()],
        )
        .map_err(|error| unavailable("keep the original text", error))?;
        conn.execute(
            "UPDATE extractions SET text_content = ?1 WHERE id = ?2",
            params![text, extraction_id],
        )
        .map_err(|error| unavailable("replace the page text", error))?;
        crate::asset_history::record_best_effort(
            conn,
            &crate::asset_history::ProcessEvent::new(
                asset_id,
                crate::asset_history::PROCESS_TEXT_REPLACE,
                "Asistente conectado (conector EntropIA Agent)",
            )
            .params(serde_json::json!({
                "previousCharacters": current.chars().count(),
                "characters": text.chars().count(),
            }))
            .input_sha256(Some(current_hash.clone()))
            .output_text(text),
        );
        crate::nlp::fts::index_item_from_db(conn, &item_id)
            .map_err(|error| unavailable("refresh the search index", error))?;
        Ok(ReplacedText {
            item_id: item_id.clone(),
            collection_id: collection_id.clone(),
            asset_id: asset_id.to_string(),
            text_hash: sha256_hex(text),
        })
    })();
    match replaced {
        Ok(replaced) => {
            conn.execute_batch("COMMIT")
                .map_err(|error| unavailable("commit the text replacement", error))?;
            Ok(replaced)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Refresh one item's search entry from its current title, metadata and text.
pub fn reindex_item(conn: &Connection, item_id: &str) -> Result<(), CorpusError> {
    read_item(conn, item_id)?.ok_or_else(|| CorpusError::NotFound(format!("no item {item_id}")))?;
    crate::nlp::fts::index_item_from_db(conn, item_id)
        .map_err(|error| unavailable("refresh the search index", error))
}

fn open(db: &AppDbState) -> Result<Connection, String> {
    crate::db::open::open_archive_connection(&db.db_path)
}

async fn blocking<F, T>(task: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| format!("corpus task failed: {error}"))?
}

#[tauri::command]
pub async fn corpus_collection_create(
    name: String,
    description: Option<String>,
    id: Option<String>,
    db: State<'_, AppDbState>,
) -> Result<CollectionDto, String> {
    let conn = open(&db)?;
    blocking(move || {
        create_collection(&conn, &name, description.as_deref(), id.as_deref())
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn corpus_collection_update(
    id: String,
    patch: CollectionPatch,
    expected_updated_at: Option<i64>,
    db: State<'_, AppDbState>,
) -> Result<CollectionDto, String> {
    let conn = open(&db)?;
    blocking(move || {
        update_collection(&conn, &id, &patch, expected_updated_at)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn corpus_item_update(
    id: String,
    patch: ItemPatch,
    expected_updated_at: Option<i64>,
    db: State<'_, AppDbState>,
) -> Result<ItemDto, String> {
    let conn = open(&db)?;
    blocking(move || {
        update_item(&conn, &id, &patch, expected_updated_at).map_err(|error| error.to_string())
    })
    .await
}

/// What importing one chosen file did, as the app reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum ImportedPath {
    /// A new item with `assets` asset rows (a PDF counts its pages too).
    Imported {
        #[serde(rename = "itemId")]
        item_id: String,
        title: String,
        assets: usize,
    },
    /// The same unchanged file is already an item of the collection.
    Duplicate {
        #[serde(rename = "itemId")]
        item_id: String,
    },
}

impl From<ImportOutcome> for ImportedPath {
    fn from(outcome: ImportOutcome) -> Self {
        match outcome {
            ImportOutcome::Imported {
                item_id,
                title,
                assets,
            } => Self::Imported {
                item_id,
                title,
                assets,
            },
            ImportOutcome::Duplicate { item_id } => Self::Duplicate { item_id },
        }
    }
}

/// Import one file the owner chose (a picker, a drop, or one of the app's own
/// staging folders) into a collection, on the service the agent path uses too
/// (`odd/tasks/agent-corpus-tools.md` D10).
///
/// The owner chose the path, so no enabled folder is required; the archive's
/// own `assets` folder is refused, and so is a missing, non-file, unsupported
/// or oversized source. The duplicate check and the item run inside the
/// import's own transaction, so two imports of one file cannot both pass it.
pub fn import_path(
    conn: &Connection,
    data_dir: &std::path::Path,
    collection_id: &str,
    source_path: &str,
    overrides: &ImportOverrides,
) -> Result<ImportedPath, String> {
    let source = check_app_source(source_path, data_dir, &SourceLimits::default())?;
    import_file(conn, data_dir, collection_id, &source, overrides).map(ImportedPath::from)
}

#[tauri::command]
pub async fn corpus_import_path(
    collection_id: String,
    source_path: String,
    overrides: Option<ImportOverrides>,
    db: State<'_, AppDbState>,
) -> Result<ImportedPath, String> {
    let conn = open(&db)?;
    // The archive's data directory holds the database and `assets/`: the same
    // derivation the approval path uses (`automation::commands`).
    let data_dir = db
        .db_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "unavailable: the archive folder is unknown".to_string())?;
    let overrides = overrides.unwrap_or_default();
    blocking(move || import_path(&conn, &data_dir, &collection_id, &source_path, &overrides)).await
}

/// Delete items and everything they own, through the same service the
/// assistant endpoint will use (`odd/tasks/agent-corpus-tools.md` D11).
#[tauri::command]
pub async fn corpus_items_delete(
    item_ids: Vec<String>,
    db: State<'_, AppDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let conn = open(&db)?;
    let data_dir = db
        .db_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "unavailable: the archive folder is unknown".to_string())?;
    // The same cache the thumbnail commands write to.
    let cache_dir = crate::path_utils::cache_dir(&app)?;
    blocking(move || {
        delete_items(&conn, &data_dir, &cache_dir, &item_ids).map_err(|error| error.to_string())
    })
    .await
}

/// Delete a collection and everything it owns, through the same service the
/// assistant endpoint will use (`odd/tasks/agent-corpus-tools.md` D11).
#[tauri::command]
pub async fn corpus_collection_delete(
    collection_id: String,
    db: State<'_, AppDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let conn = open(&db)?;
    let data_dir = db
        .db_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "unavailable: the archive folder is unknown".to_string())?;
    let cache_dir = crate::path_utils::cache_dir(&app)?;
    blocking(move || {
        delete_collection(&conn, &data_dir, &cache_dir, &collection_id)
            .map_err(|error| error.to_string())
    })
    .await
}
