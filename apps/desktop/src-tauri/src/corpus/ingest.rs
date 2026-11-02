//! Importing files from the owner's enabled folders into a collection
//! (`odd/tasks/agent-corpus-tools.md` D9, T5a-2).
//!
//! The same result as the app's own import (`lib/collection-import.ts`): one
//! item per file titled by its name without the extension, the file copied to
//! `assets/{collection}/{item}/{uuid}_{name}`, its provenance under the
//! reserved `__entropia_file_metadata` key, one asset row, and for a PDF the
//! original kept as a parent with one single-page PDF asset per page.
//!
//! What only this path does: every source is checked by its real path (links
//! and `..` resolved) to be a regular file directly inside an enabled folder
//! and outside the archive's own data, and a duplicate is detected inside the
//! same transaction that creates the item, so two imports of one file cannot
//! both pass the check.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Largest file one import may copy.
pub const MAX_SOURCE_BYTES: u64 = 1024 * 1024 * 1024;

/// What a file is, by its extension: the app's own list (`lib/file-import.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Image,
    Pdf,
    Audio,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Pdf => "pdf",
            Self::Audio => "audio",
        }
    }
}

/// The kind of a file by its extension, case-insensitive. The list is the
/// app's own (`lib/file-import.ts` `SUPPORTED_*`), so a file the app classified
/// is never refused here.
pub fn classify(path: &Path) -> Option<FileKind> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "tiff" | "tif" => Some(FileKind::Image),
        "pdf" => Some(FileKind::Pdf),
        "wav" | "mp3" | "flac" | "m4a" | "aac" | "ogg" => Some(FileKind::Audio),
        _ => None,
    }
}

/// Bounds a source must respect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLimits {
    pub max_bytes: u64,
}

impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_SOURCE_BYTES,
        }
    }
}

/// A file that passed every check, as it was when it was checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    /// The real path, links and `..` resolved.
    #[schemars(with = "String")]
    pub path: PathBuf,
    pub name: String,
    pub kind: FileKind,
    pub size_bytes: u64,
    /// Unix milliseconds, when the system reports them.
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub accessed_at: Option<i64>,
    pub readonly: bool,
}

fn millis(time: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    time.ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_millis() as i64)
}

/// Check one candidate. The error starts with a stable reason
/// (`missing`, `not_a_file`, `in_archive`, `not_in_folder`, `unsupported`,
/// `too_large`) followed by `: ` and a detail.
pub fn check_source(
    path: &str,
    folders: &[PathBuf],
    data_dir: &Path,
    limits: &SourceLimits,
) -> Result<SourceFile, String> {
    let real =
        std::fs::canonicalize(path).map_err(|_| format!("missing: {path} does not exist"))?;
    let data_dir = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    if real.starts_with(&data_dir) {
        return Err(format!("in_archive: {path} is part of EntropIA's own data"));
    }
    let metadata =
        std::fs::metadata(&real).map_err(|_| format!("missing: {path} cannot be read"))?;
    if !metadata.is_file() {
        return Err(format!("not_a_file: {path} is not a file"));
    }
    // Enabled folders are stored in their plain form (no `\\?\` on Windows).
    let inside = real.parent().is_some_and(|parent| {
        let parent = crate::path_utils::normalize_windows_path(parent);
        folders.iter().any(|folder| folder == &parent)
    });
    if !inside {
        return Err(format!(
            "not_in_folder: {path} is not directly inside an enabled folder"
        ));
    }
    describe(path, real, &metadata, limits)
}

/// Check one candidate the owner chose for the app's own import: a file from
/// the picker or a drop, or one the app staged for the Internet Archive, the
/// Navegador copy or the sample collection.
///
/// The owner chose the path, so there is no enabled-folder rule (D10(a)). The
/// one place the import never reads is the archive's own `assets`: a file there
/// is already imported, and importing it again would copy a document into
/// itself. The app does stage inside the data directory, so `in_archive` here
/// means exactly that folder. The other reasons are the same as
/// [`check_source`]: `missing`, `not_a_file`, `unsupported`, `too_large`.
pub fn check_app_source(
    path: &str,
    data_dir: &Path,
    limits: &SourceLimits,
) -> Result<SourceFile, String> {
    let real =
        std::fs::canonicalize(path).map_err(|_| format!("missing: {path} does not exist"))?;
    let data_dir = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    if real.starts_with(data_dir.join("assets")) {
        return Err(format!("in_archive: {path} is part of EntropIA's own data"));
    }
    let metadata =
        std::fs::metadata(&real).map_err(|_| format!("missing: {path} cannot be read"))?;
    if !metadata.is_file() {
        return Err(format!("not_a_file: {path} is not a file"));
    }
    describe(path, real, &metadata, limits)
}

/// The file a check accepted, by its kind and size.
fn describe(
    path: &str,
    real: PathBuf,
    metadata: &std::fs::Metadata,
    limits: &SourceLimits,
) -> Result<SourceFile, String> {
    let kind = classify(&real)
        .ok_or_else(|| format!("unsupported: {path} is not a PDF, an image or an audio file"))?;
    if metadata.len() > limits.max_bytes {
        return Err(format!(
            "too_large: {path} is larger than {} bytes",
            limits.max_bytes
        ));
    }
    Ok(SourceFile {
        name: real
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
        path: real,
        kind,
        size_bytes: metadata.len(),
        modified_at: millis(metadata.modified()),
        created_at: millis(metadata.created()),
        accessed_at: millis(metadata.accessed()),
        readonly: metadata.permissions().readonly(),
    })
}

/// The supported files directly inside one enabled folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub files: Vec<SourceFile>,
    /// More files exist than were listed.
    pub truncated: bool,
}

/// List the supported files directly inside `folder` (never its subfolders),
/// by name, at most `max_files`. `folder` must be one of the enabled ones.
pub fn list_folder(
    folder: &Path,
    folders: &[PathBuf],
    data_dir: &Path,
    max_files: usize,
) -> Result<FolderListing, String> {
    let real = std::fs::canonicalize(folder)
        .map_err(|_| format!("not_enabled: {} is not an enabled folder", folder.display()))?;
    let plain = crate::path_utils::normalize_windows_path(&real);
    if !folders.iter().any(|enabled| enabled == &plain) {
        return Err(format!(
            "not_enabled: {} is not an enabled folder",
            folder.display()
        ));
    }
    let mut names: Vec<PathBuf> = std::fs::read_dir(&real)
        .map_err(|error| format!("unreadable: {}: {error}", folder.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| classify(path).is_some())
        .collect();
    names.sort();
    let limits = SourceLimits::default();
    let mut files = Vec::new();
    let mut truncated = false;
    for path in names {
        let Ok(file) = check_source(&path.to_string_lossy(), folders, data_dir, &limits) else {
            continue;
        };
        if files.len() == max_files {
            truncated = true;
            break;
        }
        files.push(file);
    }
    Ok(FolderListing { files, truncated })
}

/// What importing one file did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ImportOutcome {
    /// A new item with `assets` asset rows (a PDF counts its pages too).
    Imported {
        item_id: String,
        title: String,
        assets: usize,
    },
    /// The same unchanged file is already an item of the collection.
    Duplicate { item_id: String },
}

/// The item of `collection_id` that already holds this unchanged file: the
/// app's rule (`findImportedFromSource`), same path ignoring case, same size,
/// same modification time.
/// The path provenance records and duplicates are matched by: the real path in
/// its plain form, as the renderer always wrote it (never `\\?\` on Windows),
/// so a file imported before keeps matching.
pub(crate) fn original_path(file: &SourceFile) -> String {
    crate::path_utils::normalize_windows_path_string(&file.path)
}

pub fn find_duplicate(
    conn: &Connection,
    collection_id: &str,
    file: &SourceFile,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM items
         WHERE collection_id = ?1
           AND lower(json_extract(metadata, '$.__entropia_file_metadata.originalPath')) = lower(?2)
           AND json_extract(metadata, '$.__entropia_file_metadata.sizeBytes') = ?3
           AND json_extract(metadata, '$.__entropia_file_metadata.modifiedAt') IS ?4
         LIMIT 1",
        params![
            collection_id,
            original_path(file),
            file.size_bytes as i64,
            file.modified_at
        ],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| format!("unavailable: could not check for an earlier import: {error}"))
}

fn now_ms() -> i64 {
    crate::processing::repository::now_ms()
}

/// `ms` since the epoch, UTC, with milliseconds: the shape the app has always
/// written (`new Date().toISOString()`, e.g. `2026-11-02T22:03:48.123Z`).
fn rfc3339_utc_ms(ms: i64) -> String {
    let ms = ms.max(0);
    let stamp = crate::navegador::capture::rfc3339_utc((ms / 1000) as u64);
    let millis = ms % 1000;
    format!("{}.{millis:03}Z", stamp.trim_end_matches('Z'))
}

/// The reserved key an import's own provenance lives under
/// (`lib/item-metadata.ts`). Nothing a caller sends may replace it.
const FILE_METADATA_KEY: &str = "__entropia_file_metadata";

/// Longest stored file name one import may write, extension included.
const STORED_NAME_MAX_CHARS: usize = 255;

/// What the caller may override about one import (D10(b)).
///
/// The agent's own import passes none of it: its title comes from the file
/// name, it stores the file under its own name and it never adds metadata.
/// The app's import names the item (a saved web capture has the page's title),
/// stores the copy under a name it chose, adds the provenance of its own flow
/// beside the file's, and decides the duplicate question itself when it has
/// already asked the person about it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportOverrides {
    /// The item's title; by default the file name without its extension.
    pub title: Option<String>,
    /// The name the stored copy and its recorded original name carry.
    pub file_name: Option<String>,
    /// Written beside `__entropia_file_metadata`, never replacing that key.
    pub extra_metadata: Option<serde_json::Map<String, serde_json::Value>>,
    /// Import even when this unchanged source is already in the collection.
    pub allow_duplicate: bool,
}

fn provenance(
    file: &SourceFile,
    stored_name: &str,
    extra: Option<&serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Value {
    let mut metadata = extra.cloned().unwrap_or_default();
    let imported_at = rfc3339_utc_ms(now_ms());
    metadata.insert(
        FILE_METADATA_KEY.to_string(),
        serde_json::json!({
            "originalName": stored_name,
            "originalPath": original_path(file),
            "importedAt": imported_at,
            "sizeBytes": file.size_bytes,
            "readonly": file.readonly,
            "isFile": true,
            "isDirectory": false,
            "createdAt": file.created_at,
            "modifiedAt": file.modified_at,
            "accessedAt": file.accessed_at,
        }),
    );
    serde_json::Value::Object(metadata)
}

/// The item title for a file: its name without the last extension, bounded
/// like any item title.
fn title_for(name: &str) -> String {
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    };
    let title: String = stem
        .trim()
        .chars()
        .take(super::commands::ITEM_TITLE_MAX_CHARS)
        .collect();
    if title.is_empty() {
        "Documento".to_string()
    } else {
        title
    }
}

fn stored_key(absolute: &Path, data_dir: &Path) -> Result<String, String> {
    crate::path_utils::derive_rel_path(&absolute.to_string_lossy(), data_dir).map_err(|error| {
        format!(
            "unavailable: could not store {}: {error:?}",
            absolute.display()
        )
    })
}

/// One asset row to record for an imported item.
struct NewAsset<'a> {
    item_id: &'a str,
    path: &'a str,
    kind: FileKind,
    size: u64,
    sort_index: i64,
    parent: Option<&'a str>,
    page_number: Option<i64>,
}

fn insert_asset(conn: &Connection, asset: &NewAsset<'_>) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO assets (id, item_id, path, type, sort_index, size, parent_asset_id, page_number, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            asset.item_id,
            asset.path,
            asset.kind.as_str(),
            asset.sort_index,
            asset.size as i64,
            asset.parent,
            asset.page_number,
            now_ms()
        ],
    )
    .map_err(|error| format!("unavailable: could not record the file: {error}"))?;
    Ok(id)
}

/// Remove what a failed import created: its asset rows, its item and its folder.
fn discard(conn: &Connection, item_id: &str, item_dir: &Path) {
    let _ = conn.execute("DELETE FROM assets WHERE item_id = ?1", [item_id]);
    let _ = conn.execute("DELETE FROM items WHERE id = ?1", [item_id]);
    let _ = std::fs::remove_dir_all(item_dir);
}

/// The name the stored copy carries: the caller's, or the source's own. It
/// must stay a plain file name, since it names a file inside the item folder.
fn stored_copy_name<'a>(
    file: &'a SourceFile,
    overrides: &'a ImportOverrides,
) -> Result<&'a str, String> {
    let name = overrides.file_name.as_deref().unwrap_or(&file.name).trim();
    let plain = !name.is_empty()
        && name.chars().count() <= STORED_NAME_MAX_CHARS
        && !name.contains(['/', '\\', ':', '\0'])
        && name != "."
        && name != "..";
    if !plain {
        return Err(format!(
            "invalid_argument: {name:?} is not a usable file name"
        ));
    }
    Ok(name)
}

/// The item's title: the caller's, or the file name without its extension.
fn title_with_override(file: &SourceFile, overrides: &ImportOverrides) -> String {
    match overrides
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        Some(title) => title
            .chars()
            .take(super::commands::ITEM_TITLE_MAX_CHARS)
            .collect(),
        None => title_for(&file.name),
    }
}

/// Import one checked file into `collection_id`.
///
/// The duplicate check and the item row share one transaction, so a second
/// import of the same file sees the first one's item. The copy and the PDF
/// split run outside any transaction (they can take a while), then the asset
/// rows and the search entry are written in a second one. Any failure removes
/// the item, its rows and its folder.
///
/// `overrides` is what the app may add to an import (D10(b)); the agent path
/// passes [`ImportOverrides::default`] and gets the same result as before.
pub fn import_file(
    conn: &Connection,
    data_dir: &Path,
    collection_id: &str,
    file: &SourceFile,
    overrides: &ImportOverrides,
) -> Result<ImportOutcome, String> {
    let stored_name = stored_copy_name(file, overrides)?;
    let collection: Option<String> = conn
        .query_row(
            "SELECT id FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("unavailable: could not read the collection: {error}"))?;
    if collection.is_none() {
        return Err(format!("not_found: no collection {collection_id}"));
    }
    let item_id = uuid::Uuid::new_v4().to_string();
    let title = title_with_override(file, overrides);
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| format!("unavailable: could not begin the import: {error}"))?;
    let admitted = (|| {
        if !overrides.allow_duplicate {
            if let Some(existing) = find_duplicate(conn, collection_id, file)? {
                return Ok(Some(existing));
            }
        }
        let now = now_ms();
        conn.execute(
            "INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                item_id,
                title,
                collection_id,
                provenance(file, stored_name, overrides.extra_metadata.as_ref()).to_string(),
                now
            ],
        )
        .map_err(|error| format!("unavailable: could not create the item: {error}"))?;
        Ok::<_, String>(None)
    })();
    match admitted {
        Ok(Some(existing)) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Ok(ImportOutcome::Duplicate { item_id: existing });
        }
        Ok(None) => conn
            .execute_batch("COMMIT")
            .map_err(|error| format!("unavailable: could not commit the item: {error}"))?,
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(error);
        }
    }

    let item_dir = data_dir.join("assets").join(collection_id).join(&item_id);
    let copied = (|| {
        std::fs::create_dir_all(&item_dir)
            .map_err(|error| format!("unavailable: could not create the item folder: {error}"))?;
        let destination = item_dir.join(format!("{}_{stored_name}", uuid::Uuid::new_v4()));
        std::fs::copy(&file.path, &destination)
            .map_err(|error| format!("unavailable: could not copy {stored_name}: {error}"))?;
        let mut pages = Vec::new();
        if file.kind == FileKind::Pdf {
            let bytes = std::fs::read(&destination)
                .map_err(|error| format!("unavailable: could not read the copy: {error}"))?;
            // The page assets are named after the stored copy, as the app has
            // always named them (`splitPdfPages` with the copy's base name).
            let base = title_for(stored_name);
            let limits = crate::ocr::commands::split_limits_for(&item_dir);
            pages = crate::ocr::pdf::split_pdf_to_files(&bytes, &item_dir, &base, &limits)
                .map_err(|error| {
                    format!("invalid_file: {stored_name} could not be split into pages: {error}")
                })?;
            if pages.is_empty() {
                return Err(format!("invalid_file: {stored_name} has no pages"));
            }
        }
        Ok((destination, pages))
    })();
    let (destination, pages) = match copied {
        Ok(copied) => copied,
        Err(error) => {
            discard(conn, &item_id, &item_dir);
            return Err(error);
        }
    };

    let recorded = (|| {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|error| {
            format!("unavailable: could not begin recording the files: {error}")
        })?;
        let rows = (|| {
            let original_key = stored_key(&destination, data_dir)?;
            let parent = insert_asset(
                conn,
                &NewAsset {
                    item_id: &item_id,
                    path: &original_key,
                    kind: file.kind,
                    size: file.size_bytes,
                    sort_index: 0,
                    parent: None,
                    page_number: None,
                },
            )?;
            for (page_number, page_path) in &pages {
                let size = std::fs::metadata(page_path)
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                let page_key = stored_key(page_path, data_dir)?;
                insert_asset(
                    conn,
                    &NewAsset {
                        item_id: &item_id,
                        path: &page_key,
                        kind: FileKind::Pdf,
                        size,
                        sort_index: i64::from(*page_number) - 1,
                        parent: Some(&parent),
                        page_number: Some(i64::from(*page_number)),
                    },
                )?;
            }
            crate::nlp::fts::index_item_from_db(conn, &item_id)
                .map_err(|error| format!("unavailable: could not index the item: {error}"))?;
            Ok::<_, String>(1 + pages.len())
        })();
        match rows {
            Ok(count) => {
                conn.execute_batch("COMMIT")
                    .map_err(|error| format!("unavailable: could not commit the files: {error}"))?;
                Ok(count)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    })();
    match recorded {
        Ok(assets) => Ok(ImportOutcome::Imported {
            item_id,
            title,
            assets,
        }),
        Err(error) => {
            discard(conn, &item_id, &item_dir);
            Err(error)
        }
    }
}
