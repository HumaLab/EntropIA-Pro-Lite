//! Which file a layout or an extraction was computed from, and what the file
//! of an asset is right now.
//!
//! The two halves meet in the version check: a writer stamps its result with
//! the file it read ([`SourceStamp`]), and a reader compares that stamp with
//! the facts of the asset's current file ([`file_facts`]). Both hashes are
//! taken over the file exactly as it is on disk, before any orientation is
//! applied, so they are equal when, and only when, it is the same file.

use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, State};

/// The file a layout or an extraction was computed from: the `source_path` and
/// `source_sha256` columns of `layouts` and `extractions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStamp {
    /// The path as `assets.path` held it when the file was read.
    pub path: String,
    /// SHA-256 of the file's bytes, lowercase hex.
    pub sha256: String,
}

impl SourceStamp {
    /// Stamp of the file read from `stored_path`. `raw` is that file exactly as
    /// it came off the disk: hashing the upright re-encoding of a tagged photo
    /// would never match the file again.
    pub(crate) fn of(stored_path: &str, raw: &[u8]) -> Self {
        Self {
            path: stored_path.to_string(),
            sha256: crate::asset_history::sha256_hex(raw),
        }
    }
}

/// The two column values of an optional stamp, `NULL` twice when the origin is
/// unknown.
pub(crate) fn stamp_columns(source: Option<&SourceStamp>) -> (Option<&str>, Option<&str>) {
    (
        source.map(|stamp| stamp.path.as_str()),
        source.map(|stamp| stamp.sha256.as_str()),
    )
}

/// What the current file of an asset is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFileFacts {
    /// EXIF orientation, `1..=8`. `1` for a file with no tag and for anything
    /// that is not an image.
    pub orientation: u8,
    /// Pixel size as a person sees the image, with the orientation applied.
    /// `None` when the file is not an image.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// SHA-256 of the file, lowercase hex. `None` unless it was asked for.
    pub sha256: Option<String>,
}

struct CachedSha {
    size: u64,
    mtime_ms: i64,
    sha256: String,
}

/// File hashes remembered for as long as the process lives, each one valid
/// while the file keeps its size and its modification time.
#[derive(Default)]
struct ShaCache {
    entries: Mutex<HashMap<PathBuf, CachedSha>>,
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

impl ShaCache {
    /// The hash of `path` and whether it came from the cache.
    fn sha256_of(&self, path: &Path) -> Result<(String, bool), String> {
        let meta = std::fs::metadata(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let (size, mtime_ms) = (meta.len(), mtime_ms(&meta));

        if let Some(hit) = self
            .entries
            .lock()
            .map_err(|e| format!("File hash cache lock poisoned: {e}"))?
            .get(path)
            .filter(|entry| entry.size == size && entry.mtime_ms == mtime_ms)
        {
            return Ok((hit.sha256.clone(), true));
        }

        // Hashed with the lock released: a large file must not hold up the
        // hashes of the others.
        let (sha256, hashed_size) = crate::sync::blobs::hash_file(path)?;
        // A file that grew or shrank while it was hashed is not remembered
        // under a size it no longer has.
        if u64::try_from(hashed_size).ok() == Some(size) {
            self.entries
                .lock()
                .map_err(|e| format!("File hash cache lock poisoned: {e}"))?
                .insert(
                    path.to_path_buf(),
                    CachedSha {
                        size,
                        mtime_ms,
                        sha256: sha256.clone(),
                    },
                );
        }
        Ok((sha256, false))
    }
}

static SHA_CACHE: LazyLock<ShaCache> = LazyLock::new(ShaCache::default);

fn file_facts_with(
    cache: &ShaCache,
    path: &Path,
    with_sha: bool,
) -> Result<AssetFileFacts, String> {
    // The header probe answers "not an image" for a missing file too; a file
    // that is not there is an error, not a file without a tag.
    std::fs::metadata(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let header = crate::image_orient::upright_header(path);
    let sha256 = if with_sha {
        Some(cache.sha256_of(path)?.0)
    } else {
        None
    };
    Ok(AssetFileFacts {
        orientation: header.map_or(1, |(orientation, _, _)| orientation),
        width: header.map(|(_, width, _)| width),
        height: header.map(|(_, _, height)| height),
        sha256,
    })
}

/// The facts of the file at `path`, an absolute path.
///
/// The orientation and the size come from the file's header. The hash reads
/// the whole file, so it is computed only when `with_sha` asks for it, and
/// remembered by path, size and modification time: checking a collection
/// hashes each file once, not once per check.
pub(crate) fn file_facts(path: &Path, with_sha: bool) -> Result<AssetFileFacts, String> {
    file_facts_with(&SHA_CACHE, path, with_sha)
}

fn stored_asset_path(db: &AppDbState, asset_id: &str) -> Result<String, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock poisoned: {e}"))?;
    conn.query_row("SELECT path FROM assets WHERE id = ?1", [asset_id], |row| {
        row.get(0)
    })
    .map_err(|e| format!("No encontré el asset {asset_id}: {e}"))
}

/// The facts of an asset's current file: EXIF orientation, upright pixel size
/// and, when `with_sha` is true, its SHA-256.
///
/// The file is the one `assets.path` names for `asset_id`; the frontend never
/// passes a path.
#[tauri::command]
pub async fn asset_file_facts(
    asset_id: String,
    with_sha: Option<bool>,
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<AssetFileFacts, String> {
    let db = db.inner().clone();
    let stored_path = run_blocking_db_task(move || stored_asset_path(&db, &asset_id)).await?;
    let path = crate::path_utils::resolve_asset_path_at_boundary(&stored_path, &app)?;
    // The database is released by now: hashing a large file blocks no query.
    tokio::task::spawn_blocking(move || file_facts(Path::new(&path), with_sha.unwrap_or(false)))
        .await
        .map_err(|e| format!("Asset file facts task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_orient::test_support::{camera_jpeg, encode_jpeg, upright_quadrants};
    use crate::sync::blobs::hash_file;

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    /// Give `path` a modification time `seconds` after the one it has, so the
    /// test does not depend on the clock resolution of the file system.
    fn push_mtime(path: &Path, seconds: u64) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for mtime");
        let current = file
            .metadata()
            .expect("metadata")
            .modified()
            .expect("mtime");
        file.set_modified(current + std::time::Duration::from_secs(seconds))
            .expect("set mtime");
    }

    #[test]
    fn a_stamp_carries_the_hash_of_the_file_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let photo = camera_jpeg(&upright_quadrants(64, 32), 6);
        let path = write(&dir, "photo_v3.jpg", &photo);

        let stamp = SourceStamp::of("assets/c1/photo_v3.jpg", &photo);

        assert_eq!(stamp.path, "assets/c1/photo_v3.jpg");
        assert_eq!(stamp.sha256, hash_file(&path).expect("hash").0);
    }

    #[test]
    fn a_camera_photo_reports_its_tag_and_its_upright_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        // What the person sees is 64×32; the file holds it sideways.
        let path = write(
            &dir,
            "photo.jpg",
            &camera_jpeg(&upright_quadrants(64, 32), 6),
        );

        let facts = file_facts_with(&ShaCache::default(), &path, false).expect("facts");

        assert_eq!(
            facts,
            AssetFileFacts {
                orientation: 6,
                width: Some(64),
                height: Some(32),
                sha256: None,
            }
        );
    }

    #[test]
    fn the_hash_is_computed_only_when_asked_for() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(&dir, "scan.jpg", &encode_jpeg(upright_quadrants(64, 32)));
        let cache = ShaCache::default();

        let without = file_facts_with(&cache, &path, false).expect("facts");
        assert_eq!(without.sha256, None);
        assert!(
            cache.entries.lock().expect("cache").is_empty(),
            "nothing was hashed"
        );

        let with = file_facts_with(&cache, &path, true).expect("facts");
        assert_eq!(with.sha256, Some(hash_file(&path).expect("hash").0));
        assert_eq!(
            (with.orientation, with.width, with.height),
            (1, Some(64), Some(32))
        );
    }

    #[test]
    fn a_file_that_is_not_an_image_has_no_size_and_a_missing_one_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write(&dir, "doc.pdf", b"%PDF-1.7\n%%EOF\n");
        let cache = ShaCache::default();

        let facts = file_facts_with(&cache, &pdf, true).expect("facts");
        assert_eq!(
            (facts.orientation, facts.width, facts.height),
            (1, None, None)
        );
        assert_eq!(facts.sha256, Some(hash_file(&pdf).expect("hash").0));

        let error =
            file_facts_with(&cache, &dir.path().join("gone.jpg"), false).expect_err("missing file");
        assert!(error.starts_with("Failed to read"), "{error}");
    }

    #[test]
    fn an_unchanged_file_is_hashed_once_and_a_changed_one_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(&dir, "photo.jpg", b"first content");
        let cache = ShaCache::default();

        let (first, from_cache) = cache.sha256_of(&path).expect("first");
        assert!(!from_cache);
        assert_eq!(
            cache.sha256_of(&path).expect("second"),
            (first.clone(), true),
            "same path, size and mtime: not hashed again"
        );

        // Same size, other content, later mtime.
        std::fs::write(&path, b"other content").expect("rewrite");
        push_mtime(&path, 10);
        let (second, from_cache) = cache.sha256_of(&path).expect("after mtime change");
        assert!(!from_cache, "a new mtime invalidates the entry");
        assert_ne!(second, first);
        assert_eq!(second, hash_file(&path).expect("hash").0);

        // Other size.
        std::fs::write(&path, b"a longer third content").expect("rewrite");
        let (third, from_cache) = cache.sha256_of(&path).expect("after size change");
        assert!(!from_cache, "a new size invalidates the entry");
        assert_eq!(third, hash_file(&path).expect("hash").0);
        assert_eq!(cache.sha256_of(&path).expect("again"), (third, true));
    }
}
