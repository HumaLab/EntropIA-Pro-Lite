//! Deleting corpus items and collections (`odd/tasks/agent-corpus-tools.md`
//! D11).
//!
//! The app used to build these cascades in TypeScript and delete their files
//! from the renderer (`item.repo.ts` `deleteWithCascade`,
//! `collection.repo.ts` `delete`). They live here now, on one connection and
//! in one `BEGIN IMMEDIATE`, so the app and the assistant endpoint share the
//! same rules: the same rows the store deleted explicitly, the same foreign-key
//! cascades, the same search rows, the same `D` tombstones through the sync
//! triggers, and the same empty-collection cleanup.
//!
//! Files are removed only after the transaction commits (D11 b): each asset's
//! own stored file (its image version family and a PDF's generated pages
//! directory included), the item folders under `<data>/assets`, the collection
//! folder when the collection is deleted or left empty, and the cached
//! thumbnails. Asset files and folders are touched only when their canonical
//! path resolves strictly inside `<data>/assets`; the thumbnail cache is only
//! addressed by its own exact names. A file that cannot be removed is logged
//! and left; the delete still succeeds and never restores rows.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params_from_iter, Connection};
use serde::Serialize;

use crate::corpus::commands::CorpusError;

/// What a delete applies to, as both the owner dialog and the service read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteScope {
    Items(Vec<String>),
    Collection(String),
}

/// What deleting the scope would remove (D11 c). `pages` counts asset rows,
/// including a PDF's child page assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteImpact {
    pub documents: i64,
    pub pages: i64,
    pub notes: i64,
    pub annotations: i64,
    pub documents_with_text: i64,
}

/// One deleted asset's file identity, read before the rows go.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AssetFile {
    id: String,
    path: String,
    kind: String,
    parent_asset_id: Option<String>,
}

/// The rows and paths one committed delete leaves to clean up.
struct DeleteOutcome {
    /// `(collection_id, item_id)` of every deleted item, for its folder.
    item_folders: Vec<(String, String)>,
    /// Collections the item delete emptied, whose folder only goes if empty.
    empty_collections: Vec<String>,
    /// Collections deleted whole, whose folder goes recursively.
    whole_collections: Vec<String>,
    /// Deleted assets, whose own files and cached thumbnails go after commit.
    asset_files: Vec<AssetFile>,
    /// Every asset path that still exists after the delete, resolved the way
    /// the app resolves stored paths. A file on this list is never removed.
    remaining_asset_paths: Vec<PathBuf>,
}

fn unavailable(action: &str, error: impl std::fmt::Display) -> CorpusError {
    CorpusError::Unavailable(format!("could not {action}: {error}"))
}

fn placeholders(count: usize) -> String {
    (1..=count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn normalize_item_ids(item_ids: &[String]) -> Result<Vec<String>, CorpusError> {
    if item_ids.is_empty() {
        return Err(CorpusError::InvalidArgument(
            "at least one item id is required".into(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut unique = Vec::with_capacity(item_ids.len());
    for id in item_ids {
        if id.trim().is_empty() {
            return Err(CorpusError::InvalidArgument(
                "an item id cannot be empty".into(),
            ));
        }
        if seen.insert(id.as_str()) {
            unique.push(id.clone());
        }
    }
    Ok(unique)
}

/// `(id, collection_id)` for every requested item, refusing the first unknown.
fn read_item_owners(
    conn: &Connection,
    item_ids: &[String],
) -> Result<Vec<(String, String)>, CorpusError> {
    let sql = format!(
        "SELECT id, collection_id FROM items WHERE id IN ({})",
        placeholders(item_ids.len())
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| unavailable("read the items", error))?;
    let rows = statement
        .query_map(params_from_iter(item_ids), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| unavailable("read the items", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unavailable("read the items", error))?;
    for id in item_ids {
        if !rows.iter().any(|(found, _)| found == id) {
            return Err(CorpusError::NotFound(format!("no item {id}")));
        }
    }
    Ok(rows)
}

/// The item ids of a collection, or `None` when the collection does not exist.
fn read_collection_item_ids(
    conn: &Connection,
    collection_id: &str,
) -> Result<Option<Vec<String>>, CorpusError> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?1)",
            [collection_id],
            |row| row.get(0),
        )
        .map_err(|error| unavailable("read the collection", error))?;
    if !exists {
        return Ok(None);
    }
    let mut statement = conn
        .prepare("SELECT id FROM items WHERE collection_id = ?1 ORDER BY id")
        .map_err(|error| unavailable("read the collection items", error))?;
    let ids = statement
        .query_map([collection_id], |row| row.get::<_, String>(0))
        .map_err(|error| unavailable("read the collection items", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unavailable("read the collection items", error))?;
    Ok(Some(ids))
}

fn read_asset_files(conn: &Connection, item_ids: &[String]) -> Result<Vec<AssetFile>, CorpusError> {
    if item_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT id, path, type, parent_asset_id FROM assets
         WHERE item_id IN ({}) ORDER BY id",
        placeholders(item_ids.len())
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| unavailable("read the item assets", error))?;
    let assets = statement
        .query_map(params_from_iter(item_ids), |row| {
            Ok(AssetFile {
                id: row.get(0)?,
                path: row.get(1)?,
                kind: row.get(2)?,
                parent_asset_id: row.get(3)?,
            })
        })
        .map_err(|error| unavailable("read the item assets", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unavailable("read the item assets", error))?;
    Ok(assets)
}

/// Resolve a stored asset path the way the app does, canonicalizing it when
/// the file exists so two spellings of one file compare equal.
fn resolved_asset_path(stored: &str, data_dir: &Path) -> PathBuf {
    let resolved = crate::path_utils::resolve_asset_path(stored, data_dir);
    std::fs::canonicalize(&resolved).unwrap_or(resolved)
}

/// Every asset path that still exists after the deletes, read inside the same
/// transaction so the post-commit file pass cannot race a newly committed row.
fn read_remaining_asset_paths(
    conn: &Connection,
    data_dir: &Path,
) -> Result<Vec<PathBuf>, CorpusError> {
    let mut statement = conn
        .prepare("SELECT path FROM assets")
        .map_err(|error| unavailable("read the surviving asset paths", error))?;
    let paths = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| unavailable("read the surviving asset paths", error))?
        .map(|path| path.map(|stored| resolved_asset_path(&stored, data_dir)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unavailable("read the surviving asset paths", error))?;
    Ok(paths)
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, CorpusError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )
    .map_err(|error| unavailable("read the schema", error))
}

fn read_scope_item_ids(conn: &Connection, scope: &DeleteScope) -> Result<Vec<String>, CorpusError> {
    match scope {
        DeleteScope::Items(item_ids) => {
            let item_ids = normalize_item_ids(item_ids)?;
            read_item_owners(conn, &item_ids)?;
            Ok(item_ids)
        }
        DeleteScope::Collection(collection_id) => read_collection_item_ids(conn, collection_id)?
            .ok_or_else(|| CorpusError::NotFound(format!("no collection {collection_id}"))),
    }
}

fn count_items(conn: &Connection, sql: &str, item_ids: &[String]) -> Result<i64, CorpusError> {
    if item_ids.is_empty() {
        return Ok(0);
    }
    conn.query_row(sql, params_from_iter(item_ids), |row| row.get(0))
        .map_err(|error| unavailable("count the delete impact", error))
}

/// Count what deleting the scope would remove (D11 c). Unknown ids are a
/// `not_found` refusal, like the delete itself.
pub fn delete_impact(conn: &Connection, scope: &DeleteScope) -> Result<DeleteImpact, CorpusError> {
    let item_ids = read_scope_item_ids(conn, scope)?;
    let placeholders = placeholders(item_ids.len());
    let pages = count_items(
        conn,
        &format!("SELECT count(*) FROM assets WHERE item_id IN ({placeholders})"),
        &item_ids,
    )?;
    let notes = count_items(
        conn,
        &format!("SELECT count(*) FROM notes WHERE item_id IN ({placeholders})"),
        &item_ids,
    )?;
    let annotations = count_items(
        conn,
        &format!(
            "SELECT count(*) FROM annotations WHERE asset_id IN
             (SELECT id FROM assets WHERE item_id IN ({placeholders}))"
        ),
        &item_ids,
    )?;
    let documents_with_text = count_items(
        conn,
        &format!(
            "SELECT count(*) FROM items WHERE id IN ({placeholders}) AND EXISTS (
               SELECT 1 FROM extractions e JOIN assets a ON a.id = e.asset_id
               WHERE a.item_id = items.id AND TRIM(COALESCE(e.text_content, '')) <> ''
             )"
        ),
        &item_ids,
    )?;
    Ok(DeleteImpact {
        documents: item_ids.len() as i64,
        pages,
        notes,
        annotations,
        documents_with_text,
    })
}

fn run_in_transaction<T>(
    conn: &Connection,
    work: impl FnOnce() -> Result<T, CorpusError>,
) -> Result<T, CorpusError> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| unavailable("begin the delete", error))?;
    match work() {
        Ok(value) => match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(value),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(unavailable("commit the delete", error))
            }
        },
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Delete the rows of `item_ids` in the store's order, including the collection
/// each one leaves empty, all in the caller's one transaction.
fn delete_items_rows(
    conn: &Connection,
    data_dir: &Path,
    item_ids: &[String],
) -> Result<DeleteOutcome, CorpusError> {
    let owners = read_item_owners(conn, item_ids)?;
    let asset_files = read_asset_files(conn, item_ids)?;
    let has_vec_assets = table_exists(conn, "vec_assets")?;
    let placeholders = placeholders(item_ids.len());

    for sql in [
        format!(
            "DELETE FROM extractions WHERE asset_id IN
             (SELECT id FROM assets WHERE item_id IN ({placeholders}))"
        ),
        format!(
            "DELETE FROM layouts WHERE asset_id IN
             (SELECT id FROM assets WHERE item_id IN ({placeholders}))"
        ),
        format!(
            "DELETE FROM llm_results WHERE (target_type = 'asset' OR target_type = 'unknown')
             AND target_id IN (SELECT id FROM assets WHERE item_id IN ({placeholders}))"
        ),
        format!(
            "DELETE FROM llm_results WHERE target_id IN ({placeholders})
             AND (target_type = 'item' OR target_type = 'unknown')"
        ),
        format!("DELETE FROM assets WHERE item_id IN ({placeholders})"),
        format!("DELETE FROM entities WHERE item_id IN ({placeholders})"),
        format!("DELETE FROM triples WHERE item_id IN ({placeholders})"),
        format!("DELETE FROM notes WHERE item_id IN ({placeholders})"),
        format!(
            "DELETE FROM fts_items WHERE rowid IN
             (SELECT rowid FROM items WHERE id IN ({placeholders}))"
        ),
        format!("DELETE FROM items WHERE id IN ({placeholders})"),
    ] {
        conn.execute(&sql, params_from_iter(item_ids))
            .map_err(|error| unavailable("delete the item rows", error))?;
    }

    let mut empty_collections = Vec::new();
    for collection_id in unique_collections(&owners) {
        let changed = conn
            .execute(
                "DELETE FROM collections WHERE id = ?1
                 AND id NOT IN (SELECT DISTINCT collection_id FROM items)",
                [collection_id.as_str()],
            )
            .map_err(|error| unavailable("delete the emptied collection", error))?;
        if changed > 0 {
            empty_collections.push(collection_id);
        }
    }

    if has_vec_assets {
        conn.execute(
            &format!("DELETE FROM vec_assets WHERE item_id IN ({placeholders})"),
            params_from_iter(item_ids),
        )
        .map_err(|error| unavailable("delete the item embeddings", error))?;
    }

    let remaining_asset_paths = read_remaining_asset_paths(conn, data_dir)?;

    Ok(DeleteOutcome {
        item_folders: owners
            .into_iter()
            .map(|(item_id, collection_id)| (collection_id, item_id))
            .collect(),
        empty_collections,
        whole_collections: Vec::new(),
        asset_files,
        remaining_asset_paths,
    })
}

fn unique_collections(owners: &[(String, String)]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut collections = Vec::new();
    for (_, collection_id) in owners {
        if seen.insert(collection_id.as_str()) {
            collections.push(collection_id.clone());
        }
    }
    collections
}

/// Delete one collection and everything it owns in the store's order, all in
/// the caller's one transaction.
fn delete_collection_rows(
    conn: &Connection,
    data_dir: &Path,
    collection_id: &str,
) -> Result<DeleteOutcome, CorpusError> {
    let item_ids = read_collection_item_ids(conn, collection_id)?
        .ok_or_else(|| CorpusError::NotFound(format!("no collection {collection_id}")))?;
    let asset_files = read_asset_files(conn, &item_ids)?;
    let has_vec_assets = table_exists(conn, "vec_assets")?;

    for sql in [
        "DELETE FROM extractions WHERE asset_id IN
         (SELECT id FROM assets WHERE item_id IN
            (SELECT id FROM items WHERE collection_id = ?1))",
        "DELETE FROM layouts WHERE asset_id IN
         (SELECT id FROM assets WHERE item_id IN
            (SELECT id FROM items WHERE collection_id = ?1))",
        "DELETE FROM llm_results WHERE (target_type = 'asset' OR target_type = 'unknown')
         AND target_id IN (SELECT id FROM assets WHERE item_id IN
            (SELECT id FROM items WHERE collection_id = ?1))",
        "DELETE FROM llm_results WHERE (target_type = 'item' OR target_type = 'unknown')
         AND target_id IN (SELECT id FROM items WHERE collection_id = ?1)",
        "DELETE FROM llm_results WHERE target_id = ?1
         AND (target_type = 'collection' OR target_type = 'unknown')",
        "DELETE FROM assets WHERE item_id IN
         (SELECT id FROM items WHERE collection_id = ?1)",
        "DELETE FROM entities WHERE item_id IN
         (SELECT id FROM items WHERE collection_id = ?1)",
        "DELETE FROM triples WHERE item_id IN
         (SELECT id FROM items WHERE collection_id = ?1)",
        "DELETE FROM notes WHERE item_id IN
         (SELECT id FROM items WHERE collection_id = ?1)",
        "DELETE FROM fts_items WHERE rowid IN
         (SELECT rowid FROM items WHERE collection_id = ?1)",
        "DELETE FROM items WHERE collection_id = ?1",
        "DELETE FROM collections WHERE id = ?1",
    ] {
        conn.execute(sql, [collection_id])
            .map_err(|error| unavailable("delete the collection rows", error))?;
    }

    if has_vec_assets && !item_ids.is_empty() {
        conn.execute(
            &format!(
                "DELETE FROM vec_assets WHERE item_id IN ({})",
                placeholders(item_ids.len())
            ),
            params_from_iter(&item_ids),
        )
        .map_err(|error| unavailable("delete the collection embeddings", error))?;
    }

    let remaining_asset_paths = read_remaining_asset_paths(conn, data_dir)?;

    Ok(DeleteOutcome {
        item_folders: item_ids
            .into_iter()
            .map(|item_id| (collection_id.to_string(), item_id))
            .collect(),
        empty_collections: Vec::new(),
        whole_collections: vec![collection_id.to_string()],
        asset_files,
        remaining_asset_paths,
    })
}

enum FolderRemoval {
    Recursive,
    EmptyOnly,
}

/// A database id is only usable as a folder name when it is exactly one
/// component: no separator, no `.`/`..`, no empty value.
fn valid_folder_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains(':')
        && !value.contains('\0')
}

fn folder_path(assets_root: &Path, collection_id: &str, item_id: Option<&str>) -> Option<PathBuf> {
    if !valid_folder_component(collection_id)
        || item_id.is_some_and(|item_id| !valid_folder_component(item_id))
    {
        return None;
    }
    let mut path = assets_root.join(collection_id);
    if let Some(item_id) = item_id {
        path.push(item_id);
    }
    Some(path)
}

/// A canonical path strictly below `canonical_root`. Symlinks are refused,
/// and the root itself is never a valid target.
fn canonical_strict_descendant(path: &Path, canonical_root: &Path) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    let canonical = std::fs::canonicalize(path).ok()?;
    (canonical != canonical_root && canonical.starts_with(canonical_root)).then_some(canonical)
}

/// A canonical directory at or below `canonical_root`; symlinks are refused.
fn canonical_directory(path: &Path, canonical_root: &Path) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    let canonical = std::fs::canonicalize(path).ok()?;
    (canonical == canonical_root || canonical.starts_with(canonical_root)).then_some(canonical)
}

/// A canonical folder exactly `depth` components below `canonical_root`.
fn canonical_folder(path: &Path, canonical_root: &Path, depth: usize) -> Option<PathBuf> {
    let canonical = canonical_directory(path, canonical_root)?;
    let relative = canonical.strip_prefix(canonical_root).ok()?;
    (relative.components().count() == depth).then_some(canonical)
}

fn remove_file_logged(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        log::warn!(
            "corpus delete: could not remove {}: {error}",
            path.display()
        );
    }
}

fn remove_dir_all_logged(path: &Path) {
    if let Err(error) = std::fs::remove_dir_all(path) {
        log::warn!(
            "corpus delete: could not remove {}: {error}",
            path.display()
        );
    }
}

fn remove_one_asset_file(canonical_root: &Path, remaining: &HashSet<PathBuf>, path: &Path) {
    let Some(canonical) = canonical_strict_descendant(path, canonical_root) else {
        return;
    };
    if !canonical.is_file() {
        return;
    }
    if remaining.contains(&canonical) {
        log::warn!(
            "corpus delete: keeping {} because another asset still references it",
            canonical.display()
        );
        return;
    }
    remove_file_logged(&canonical);
}

/// Remove the image and its edited versions, using the same family rule as
/// `image_edit::delete_asset_files`.
fn remove_image_family(canonical_root: &Path, remaining: &HashSet<PathBuf>, path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    let Some(parent) = canonical_directory(parent, canonical_root) else {
        return;
    };
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return;
    };
    let base = crate::image_edit::strip_version_suffix(stem);
    let Ok(entries) = std::fs::read_dir(&parent) else {
        return;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&entry_path) else {
            continue;
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !crate::image_edit::file_belongs_to_asset_family(&file_name, base) {
            continue;
        }
        let Ok(canonical) = std::fs::canonicalize(&entry_path) else {
            continue;
        };
        if canonical == canonical_root || !canonical.starts_with(canonical_root) {
            continue;
        }
        if remaining.contains(&canonical) {
            log::warn!(
                "corpus delete: keeping {} because another asset still references it",
                canonical.display()
            );
            continue;
        }
        remove_file_logged(&canonical);
    }
}

/// Remove the generated pages directory that belongs to a source PDF.
fn remove_pdf_pages(canonical_root: &Path, remaining: &HashSet<PathBuf>, path: &Path) {
    let pages = path.with_extension("pages");
    let Some(canonical) = canonical_strict_descendant(&pages, canonical_root) else {
        return;
    };
    if !canonical.is_dir() {
        return;
    }
    if remaining.iter().any(|path| path.starts_with(&canonical)) {
        log::warn!(
            "corpus delete: keeping {} because another asset still references a page",
            canonical.display()
        );
        return;
    }
    remove_dir_all_logged(&canonical);
}

fn remove_asset_file(
    canonical_root: &Path,
    remaining: &HashSet<PathBuf>,
    data_dir: &Path,
    asset: &AssetFile,
) {
    let path = crate::path_utils::resolve_asset_path(&asset.path, data_dir);
    if asset.kind == "image" {
        remove_image_family(canonical_root, remaining, &path);
    } else {
        remove_one_asset_file(canonical_root, remaining, &path);
    }
    if asset.kind == "pdf" && asset.parent_asset_id.is_none() {
        remove_pdf_pages(canonical_root, remaining, &path);
    }
}

fn remove_folder_inside(
    canonical_root: &Path,
    folder: &Path,
    mode: FolderRemoval,
    depth: usize,
    remaining: &HashSet<PathBuf>,
) {
    let Some(canonical) = canonical_folder(folder, canonical_root, depth) else {
        if folder.exists() {
            log::warn!(
                "corpus delete: refusing to remove {}: not exactly {depth} component(s) below assets",
                folder.display()
            );
        }
        return;
    };
    if remaining.iter().any(|path| path.starts_with(&canonical)) {
        log::warn!(
            "corpus delete: keeping {} because another asset still references a file inside it",
            canonical.display()
        );
        return;
    }
    let removed = match mode {
        FolderRemoval::Recursive => std::fs::remove_dir_all(&canonical),
        FolderRemoval::EmptyOnly => std::fs::remove_dir(&canonical),
    };
    if let Err(error) = removed {
        log::warn!(
            "corpus delete: could not remove {}: {error}",
            canonical.display()
        );
    }
}

/// Remove the files the committed delete left behind (D11 b). Asset files and
/// folders are touched only when they resolve strictly inside `<data>/assets`;
/// every failure is logged and skipped, and the delete already succeeded.
fn remove_files_after_commit(data_dir: &Path, cache_dir: &Path, outcome: &DeleteOutcome) {
    let assets_root = data_dir.join("assets");
    if let Ok(canonical_root) = std::fs::canonicalize(&assets_root) {
        let remaining: HashSet<PathBuf> = outcome.remaining_asset_paths.iter().cloned().collect();

        for asset in &outcome.asset_files {
            remove_asset_file(&canonical_root, &remaining, data_dir, asset);
        }
        for (collection_id, item_id) in &outcome.item_folders {
            match folder_path(&assets_root, collection_id, Some(item_id)) {
                Some(folder) => remove_folder_inside(
                    &canonical_root,
                    &folder,
                    FolderRemoval::Recursive,
                    2,
                    &remaining,
                ),
                None => log::warn!(
                    "corpus delete: refusing to build an item folder from ids {collection_id:?}/{item_id:?}"
                ),
            }
        }
        for collection_id in &outcome.empty_collections {
            match folder_path(&assets_root, collection_id, None) {
                Some(folder) => remove_folder_inside(
                    &canonical_root,
                    &folder,
                    FolderRemoval::EmptyOnly,
                    1,
                    &remaining,
                ),
                None => log::warn!(
                    "corpus delete: refusing to build a collection folder from id {collection_id:?}"
                ),
            }
        }
        for collection_id in &outcome.whole_collections {
            match folder_path(&assets_root, collection_id, None) {
                Some(folder) => remove_folder_inside(
                    &canonical_root,
                    &folder,
                    FolderRemoval::Recursive,
                    1,
                    &remaining,
                ),
                None => log::warn!(
                    "corpus delete: refusing to build a collection folder from id {collection_id:?}"
                ),
            }
        }
    }

    for asset in &outcome.asset_files {
        if let Err(error) = crate::ocr::commands::delete_pdf_thumbnail_file(cache_dir, &asset.id) {
            log::warn!("corpus delete: could not remove a PDF thumbnail: {error}");
        }
        if let Err(error) = crate::ocr::commands::delete_image_thumbnail_files(cache_dir, &asset.id)
        {
            log::warn!("corpus delete: could not remove an image thumbnail: {error}");
        }
    }
}

/// Delete items and everything they own, then their files. All ids are checked
/// before the first write: any unknown id refuses the whole delete with
/// `not_found` and nothing changed.
pub fn delete_items(
    conn: &Connection,
    data_dir: &Path,
    cache_dir: &Path,
    item_ids: &[String],
) -> Result<(), CorpusError> {
    let item_ids = normalize_item_ids(item_ids)?;
    let outcome = run_in_transaction(conn, || delete_items_rows(conn, data_dir, &item_ids))?;
    remove_files_after_commit(data_dir, cache_dir, &outcome);
    Ok(())
}

/// Delete a collection and everything it owns, then its files. An unknown
/// collection is a `not_found` refusal with nothing changed.
pub fn delete_collection(
    conn: &Connection,
    data_dir: &Path,
    cache_dir: &Path,
    collection_id: &str,
) -> Result<(), CorpusError> {
    let outcome = run_in_transaction(conn, || {
        delete_collection_rows(conn, data_dir, collection_id)
    })?;
    remove_files_after_commit(data_dir, cache_dir, &outcome);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_commit_rolls_back_instead_of_leaving_the_transaction_open() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open::open_archive_connection(
            &crate::automation::test_archive::seeded_archive(dir.path()),
        )
        .unwrap();

        let error = run_in_transaction(&conn, || {
            // A deferred foreign-key violation makes COMMIT fail after a
            // statement that itself succeeded.
            conn.execute_batch("PRAGMA defer_foreign_keys = ON")
                .unwrap();
            conn.execute(
                "INSERT INTO notes (id, item_id, content, created_at, updated_at)
                 VALUES ('bad-note', 'missing-item', 'x', 1, 1)",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("unavailable: could not commit the delete"),
            "{error}"
        );
        // A failed COMMIT leaves the transaction open until it is rolled back;
        // without the rollback this second BEGIN would fail.
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM notes WHERE id = 'bad-note'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
