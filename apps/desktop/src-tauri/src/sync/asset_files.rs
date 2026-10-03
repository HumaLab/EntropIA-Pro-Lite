//! Local files of assets that a pulled delete removed.
//!
//! A pulled tombstone (directly, or through the `assets.parent_asset_id`
//! cascade) deletes the `assets` row. Without this module its file stayed under
//! `assets/` and its `sync_pending_blobs` / `sync_blob_index` rows lingered.
//!
//! [`queue_removal`] runs inside the apply transaction right before the delete,
//! so the queue entry and the delete commit (or roll back) together. A
//! tombstone that is deferred (dirty child) never reaches it.
//! [`drain_asset_file_removals`] runs after commit and only removes a file that
//! no remaining `assets` row references and that is a plain file strictly
//! inside `<data>/assets/`. It never fails the sync cycle over a file problem.
//!
//! Two more things belong to a deleted asset and go with it, under the same
//! rules (after commit, never outside their own folder, never a link):
//! * its image edit versions (`<name>_v2.<ext>`, ...) next to the file, unless
//!   some live asset still uses a file of that family (then the whole family
//!   stays: its versions cannot be told apart from that asset's history);
//! * its cached thumbnails under `<cache>/thumbnails/`, keyed by asset id.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use rusqlite::{params, Connection};

use super::apply::validate_inbound_rel_path;
use super::session::meta_delete;

/// Literal prefix of the file-removal queue (matched with `substr`, never
/// `LIKE`: the underscores would be wildcards).
pub(crate) const FILE_REMOVAL_PREFIX: &str = "asset_file_remove:";

/// Literal prefix of the thumbnail-removal queue; the rest of the key is the
/// asset id.
pub(crate) const THUMB_REMOVAL_PREFIX: &str = "asset_thumb_remove:";

/// Called by the apply path right before an `assets` row is deleted. Queues the
/// file of that asset and of every page asset the delete cascades to, and drops
/// their download/upload bookkeeping (no point fetching a deleted asset).
pub(crate) fn queue_removal(conn: &Connection, asset_id: &str) -> Result<(), String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack = vec![asset_id.to_string()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let path: Option<String> = conn
            .query_row("SELECT path FROM assets WHERE id = ?1", [&id], |row| {
                row.get(0)
            })
            .ok();
        conn.execute(
            "INSERT INTO sync_meta(key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO NOTHING",
            params![format!("{THUMB_REMOVAL_PREFIX}{id}")],
        )
        .map_err(|e| format!("[sync] failed to queue asset thumbnail removal: {e}"))?;
        if let Some(path) = path {
            let norm = path.trim().replace('\\', "/");
            if !norm.is_empty() {
                conn.execute(
                    "INSERT INTO sync_meta(key, value) VALUES (?1, '1')
                     ON CONFLICT(key) DO NOTHING",
                    params![format!("{FILE_REMOVAL_PREFIX}{norm}")],
                )
                .map_err(|e| format!("[sync] failed to queue asset file removal: {e}"))?;
            }
        }
        conn.execute("DELETE FROM sync_pending_blobs WHERE asset_id = ?1", [&id])
            .map_err(|e| format!("[sync] failed to drop pending blob of {id}: {e}"))?;
        conn.execute("DELETE FROM sync_blob_index WHERE asset_id = ?1", [&id])
            .map_err(|e| format!("[sync] failed to drop blob index of {id}: {e}"))?;

        let mut stmt = conn
            .prepare("SELECT id FROM assets WHERE parent_asset_id = ?1")
            .map_err(|e| format!("[sync] failed to read page assets: {e}"))?;
        let children: Vec<String> = stmt
            .query_map([&id], |row| row.get(0))
            .map_err(|e| format!("[sync] failed to read page assets: {e}"))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("[sync] failed to read page assets: {e}"))?;
        stack.extend(children);
    }
    Ok(())
}

/// What the drain decides for one queued path.
enum Verdict {
    /// Nothing more to do: removed, already gone, still referenced or refused.
    /// Carries how many files were deleted.
    Done(usize),
    /// A file could not be removed now; try again next cycle. Carries how many
    /// files were deleted meanwhile.
    Retry(usize),
}

/// Removes the files queued by [`queue_removal`], after the delete committed.
/// Returns how many files were deleted. A file that is still referenced, not a
/// plain file, or outside `assets/` is left alone and its entry dropped; a file
/// that fails to delete stays queued and is logged, never an error. Thumbnails
/// go from the process's remembered cache directory.
pub(crate) fn drain_asset_file_removals(
    conn: &Connection,
    app_data_dir: &Path,
) -> Result<usize, String> {
    let cache = crate::path_utils::remembered_cache_dir();
    drain_with_cache(conn, app_data_dir, cache.as_deref())
}

/// [`drain_asset_file_removals`] with the cache directory injected. A `None`
/// cache leaves the thumbnail queue untouched for a later cycle.
pub(crate) fn drain_with_cache(
    conn: &Connection,
    app_data_dir: &Path,
    cache_dir: Option<&Path>,
) -> Result<usize, String> {
    let mut removed = 0usize;
    for key in queued_keys(conn, FILE_REMOVAL_PREFIX)? {
        let stored = &key[FILE_REMOVAL_PREFIX.len()..];
        match decide(conn, app_data_dir, stored)? {
            Verdict::Done(count) => {
                meta_delete(conn, &key)?;
                removed += count;
            }
            Verdict::Retry(count) => removed += count,
        }
    }
    if let Some(cache_dir) = cache_dir {
        for key in queued_keys(conn, THUMB_REMOVAL_PREFIX)? {
            let id = &key[THUMB_REMOVAL_PREFIX.len()..];
            if remove_thumbnails(conn, cache_dir, id)? {
                meta_delete(conn, &key)?;
            }
        }
    }
    Ok(removed)
}

fn queued_keys(conn: &Connection, prefix: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT key FROM sync_meta WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key")
        .map_err(|e| format!("[sync] failed to read asset file removal queue: {e}"))?;
    let keys = stmt
        .query_map([prefix], |row| row.get(0))
        .map_err(|e| format!("[sync] failed to read asset file removal queue: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("[sync] failed to read asset file removal queue: {e}"))?;
    Ok(keys)
}

/// True when some `assets` row uses `rel` (exact key, or a legacy absolute
/// spelling of it; keeping too much is safe).
fn is_referenced(conn: &Connection, rel: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM assets
            WHERE length(path) >= length(?1)
              AND substr(replace(path, char(92), '/'), -length(?1)) = ?1)",
        [rel],
        |row| row.get(0),
    )
    .map_err(|e| format!("[sync] failed to check asset references: {e}"))
}

fn decide(conn: &Connection, data_dir: &Path, stored: &str) -> Result<Verdict, String> {
    // A row written before the relative-path migration stores an absolute path.
    let rel = if Path::new(stored).is_absolute() {
        crate::path_utils::derive_rel_path(stored, data_dir).unwrap_or_else(|_| stored.to_string())
    } else {
        stored.to_string()
    };
    let rel = rel.trim().replace('\\', "/");

    // Another asset may still use this file.
    if is_referenced(conn, &rel)? {
        return Ok(Verdict::Done(0));
    }

    let Ok(candidate) = validate_inbound_rel_path(&rel, data_dir) else {
        return Ok(Verdict::Done(0));
    };
    let assets_root = data_dir.join("assets");
    // Within the canonical assets root (resolves junctions and links).
    if crate::path_utils::ensure_within_dir(&candidate, &assets_root).is_err() {
        return Ok(Verdict::Done(0));
    }
    let mut removed = 0usize;
    match fs::symlink_metadata(&candidate) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Ok(Verdict::Done(0)),
        // Never remove a link, a folder or anything that is not a plain file.
        Ok(meta) if !meta.is_file() => return Ok(Verdict::Done(0)),
        Ok(_) => match fs::remove_file(&candidate) {
            Ok(()) => removed += 1,
            Err(e) => {
                eprintln!(
                    "[sync] could not remove the file of a deleted asset ({}): {e}",
                    candidate.display()
                );
                return Ok(Verdict::Retry(0));
            }
        },
    }

    let (versions_removed, versions_failed) = remove_versions(conn, &candidate, &rel)?;
    removed += versions_removed;

    // Folders the file leaves empty go too (a PDF's `.pages`, an item folder),
    // never the `assets` root itself and never a folder that still holds files.
    let mut dir = candidate.parent();
    while let Some(d) = dir {
        if d == assets_root || !d.starts_with(&assets_root) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(if versions_failed {
        Verdict::Retry(removed)
    } else {
        Verdict::Done(removed)
    })
}

/// True for a name with one of the raster extensions image edits produce.
fn is_image_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            crate::image_edit::file_belongs_to_asset_family(&format!("x.{ext}"), "x")
        })
}

/// Removes the image edit versions that sit next to a deleted asset's file.
/// Returns how many went and whether one failed (retry next cycle).
fn remove_versions(
    conn: &Connection,
    candidate: &Path,
    rel: &str,
) -> Result<(usize, bool), String> {
    let (Some(parent), Some((rel_dir, file_name))) = (candidate.parent(), rel.rsplit_once('/'))
    else {
        return Ok((0, false));
    };
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if stem.is_empty() || !is_image_name(file_name) {
        return Ok((0, false));
    }
    let base = crate::image_edit::strip_version_suffix(stem);

    // A live asset in this folder with a file of the same family: its history
    // is indistinguishable from the deleted one, so nothing of it is swept.
    let mut stmt = conn
        .prepare("SELECT path FROM assets WHERE instr(replace(path, char(92), '/'), ?1) > 0")
        .map_err(|e| format!("[sync] failed to read sibling assets: {e}"))?;
    let live: Vec<String> = stmt
        .query_map([format!("{rel_dir}/")], |row| row.get(0))
        .map_err(|e| format!("[sync] failed to read sibling assets: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("[sync] failed to read sibling assets: {e}"))?;
    drop(stmt);
    for path in live {
        let norm = path.replace('\\', "/");
        if let Some((dir, name)) = norm.rsplit_once('/') {
            if dir.ends_with(rel_dir) && crate::image_edit::file_belongs_to_asset_family(name, base)
            {
                return Ok((0, false));
            }
        }
    }

    let Ok(entries) = fs::read_dir(parent) else {
        return Ok((0, false));
    };
    let mut removed = 0usize;
    let mut failed = false;
    for entry in entries.flatten() {
        // `DirEntry::file_type` does not follow links: only plain files count.
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !crate::image_edit::file_belongs_to_asset_family(name, base)
            || is_referenced(conn, &format!("{rel_dir}/{name}"))?
        {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(e) => {
                eprintln!(
                    "[sync] could not remove an edit version of a deleted asset ({}): {e}",
                    entry.path().display()
                );
                failed = true;
            }
        }
    }
    Ok((removed, failed))
}

/// True for the file names the app caches for `asset_id`: `{id}.png` (PDF) and
/// `image-{id}-{sha256 hex}.png` (image, one per file version).
fn is_thumbnail_of(name: &str, asset_id: &str) -> bool {
    if name == format!("{asset_id}.png") {
        return true;
    }
    name.strip_prefix(&format!("image-{asset_id}-"))
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Removes the cached thumbnails of a deleted asset. Returns `true` when the
/// queue entry can be dropped (done, or nothing to do), `false` to retry.
fn remove_thumbnails(conn: &Connection, cache_dir: &Path, asset_id: &str) -> Result<bool, String> {
    let safe = !asset_id.is_empty()
        && asset_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !safe {
        return Ok(true);
    }
    // The id came back (a later pull re-created the asset): its cache is live.
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM assets WHERE id = ?1)",
            [asset_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("[sync] failed to check asset existence: {e}"))?;
    if exists {
        return Ok(true);
    }
    let dir = cache_dir.join("thumbnails");
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        // Missing, a link or a file: nothing of ours in there.
        _ => return Ok(true),
    }
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(false);
    };
    let mut done = true;
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if is_thumbnail_of(name, asset_id) {
            if let Err(e) = fs::remove_file(entry.path()) {
                eprintln!(
                    "[sync] could not remove the thumbnail of a deleted asset ({}): {e}",
                    entry.path().display()
                );
                done = false;
            }
        }
    }
    Ok(done)
}
