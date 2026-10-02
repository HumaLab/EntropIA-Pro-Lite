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

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use rusqlite::{params, Connection};

use super::apply::validate_inbound_rel_path;
use super::session::meta_delete;

/// Literal prefix of the file-removal queue (matched with `substr`, never
/// `LIKE`: the underscores would be wildcards).
pub(crate) const FILE_REMOVAL_PREFIX: &str = "asset_file_remove:";

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
    Done(bool),
    /// The file could not be removed now; try again next cycle.
    Retry,
}

/// Removes the files queued by [`queue_removal`], after the delete committed.
/// Returns how many files were deleted. A file that is still referenced, not a
/// plain file, or outside `assets/` is left alone and its entry dropped; a file
/// that fails to delete stays queued and is logged, never an error.
pub(crate) fn drain_asset_file_removals(
    conn: &Connection,
    app_data_dir: &Path,
) -> Result<usize, String> {
    let mut stmt = conn
        .prepare("SELECT key FROM sync_meta WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key")
        .map_err(|e| format!("[sync] failed to read asset file removal queue: {e}"))?;
    let keys: Vec<String> = stmt
        .query_map([FILE_REMOVAL_PREFIX], |row| row.get(0))
        .map_err(|e| format!("[sync] failed to read asset file removal queue: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("[sync] failed to read asset file removal queue: {e}"))?;
    drop(stmt);

    let mut removed = 0usize;
    for key in keys {
        let stored = &key[FILE_REMOVAL_PREFIX.len()..];
        match decide(conn, app_data_dir, stored)? {
            Verdict::Done(was_removed) => {
                meta_delete(conn, &key)?;
                if was_removed {
                    removed += 1;
                }
            }
            Verdict::Retry => {}
        }
    }
    Ok(removed)
}

fn decide(conn: &Connection, data_dir: &Path, stored: &str) -> Result<Verdict, String> {
    // A row written before the relative-path migration stores an absolute path.
    let rel = if Path::new(stored).is_absolute() {
        crate::path_utils::derive_rel_path(stored, data_dir).unwrap_or_else(|_| stored.to_string())
    } else {
        stored.to_string()
    };

    // Another asset may still use this file. The suffix comparison also catches
    // a legacy absolute spelling of the same key; keeping too much is safe.
    let referenced: bool = conn
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM assets
                WHERE length(path) >= length(?1)
                  AND substr(replace(path, char(92), '/'), -length(?1)) = ?1)",
            [&rel],
            |row| row.get(0),
        )
        .map_err(|e| format!("[sync] failed to check asset references: {e}"))?;
    if referenced {
        return Ok(Verdict::Done(false));
    }

    let Ok(candidate) = validate_inbound_rel_path(&rel, data_dir) else {
        return Ok(Verdict::Done(false));
    };
    let assets_root = data_dir.join("assets");
    // Within the canonical assets root (resolves junctions and links).
    if crate::path_utils::ensure_within_dir(&candidate, &assets_root).is_err() {
        return Ok(Verdict::Done(false));
    }
    match fs::symlink_metadata(&candidate) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Verdict::Done(false)),
        Err(_) => return Ok(Verdict::Done(false)),
        // Never remove a link, a folder or anything that is not a plain file.
        Ok(meta) if !meta.is_file() => return Ok(Verdict::Done(false)),
        Ok(_) => {}
    }
    if let Err(e) = fs::remove_file(&candidate) {
        eprintln!(
            "[sync] could not remove the file of a deleted asset ({}): {e}",
            candidate.display()
        );
        return Ok(Verdict::Retry);
    }
    // Folders the file leaves empty go too (a PDF's `.pages`, an item folder),
    // never the `assets` root itself and never a folder that still holds files.
    let mut dir = candidate.parent();
    while let Some(d) = dir {
        if d == assets_root || !d.starts_with(&assets_root) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(Verdict::Done(true))
}
