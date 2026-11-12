//! The versions of an image asset that are on disk.
//!
//! Every image edit writes a new `<stem>_vN.<ext>` next to the file it started
//! from and points `assets.path` at it ([`crate::image_edit`]). The files stay
//! where they are, so the family on disk is the one durable record of what the
//! image has been: the undo history of the viewer lives in memory and is gone
//! when the document is closed. This module lists that family, so the person
//! can go back to any member of it.
//!
//! Nothing here writes: choosing a version is the viewer's own path change.

use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use crate::derived_source::{file_facts, mtime_ms, AssetFileFacts};
use crate::image_edit::{file_belongs_to_asset_family, strip_version_suffix};
use crate::path_utils::{derive_rel_path, ensure_within_dir, resolve_asset_path};
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, State};

/// One file of an image asset's version family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageVersion {
    /// The value to store in `assets.path` to show this file. For the file the
    /// asset shows now it is the stored value itself, character for character.
    pub path: String,
    /// `N` of `<stem>_vN`. `0` for a file with no version suffix: the original.
    pub version: u64,
    /// EXIF orientation, upright pixel size and, when asked for, the hash: the
    /// same facts the version check reads of the current file.
    #[serde(flatten)]
    pub facts: AssetFileFacts,
    /// Modification time, milliseconds since the Unix epoch.
    pub modified_ms: i64,
    /// Size in bytes.
    pub size: u64,
    /// Whether `assets.path` points at this file.
    pub current: bool,
}

/// `assets.type` of the only assets that have versions.
const IMAGE_ASSET_TYPE: &str = "image";

/// What `assets` says about the asset whose versions are asked for.
struct AssetRow {
    path: String,
    asset_type: String,
    parent_asset_id: Option<String>,
}

/// Only an image that is its own document has a version family. A PDF is
/// edited through another path, and a page of a PDF belongs to its container.
fn require_image_asset(asset: &AssetRow) -> Result<(), String> {
    if asset.asset_type != IMAGE_ASSET_TYPE || asset.parent_asset_id.is_some() {
        return Err("invalid_argument: only image assets have versions".to_string());
    }
    Ok(())
}

/// `N` of a `<base>_vN` stem, `0` for the stem with no suffix.
fn version_number(stem: &str) -> u64 {
    let base = strip_version_suffix(stem);
    if base.len() == stem.len() {
        return 0;
    }
    // `strip_version_suffix` only strips `_v` followed by digits, so the parse
    // fails for a number too long to hold and for nothing else.
    stem[base.len() + 2..].parse().unwrap_or(u64::MAX)
}

/// The version family of the asset stored as `stored_path`: the original and
/// every `<stem>_vN.<ext>` beside it, the original first and the versions by
/// `N` as a number.
///
/// Only files in the directory of `stored_path`, which must be inside
/// `data_dir`, are ever listed. The file `stored_path` names may be gone: its
/// siblings are listed all the same and none is the current one.
pub(crate) fn list_family(
    stored_path: &str,
    data_dir: &Path,
    with_sha: bool,
) -> Result<Vec<ImageVersion>, String> {
    if stored_path.trim().is_empty() {
        return Err("Asset path must not be empty".to_string());
    }
    let current = resolve_asset_path(stored_path, data_dir);
    ensure_within_dir(&current, data_dir)?;

    let (Some(dir), Some(current_name), Some(stem)) = (
        current.parent(),
        current.file_name(),
        current.file_stem().and_then(|stem| stem.to_str()),
    ) else {
        return Err(format!("Asset path has no file name: {stored_path}"));
    };
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let base = strip_version_suffix(stem);

    let mut versions = Vec::new();
    for entry in
        std::fs::read_dir(dir).map_err(|e| format!("Failed to read asset directory: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Failed to read asset directory entry: {e}"))?;
        // The entry's own type: a link is not followed out of the directory.
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !file_belongs_to_asset_family(name_str, base) {
            continue;
        }
        // A file that went away or cannot be read while listing is left out;
        // the others are still worth showing.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        let facts = file_facts(&path, with_sha).unwrap_or(AssetFileFacts {
            orientation: 1,
            width: None,
            height: None,
            sha256: None,
        });
        let is_current = name == current_name;
        let absolute = path.to_string_lossy();
        versions.push(ImageVersion {
            path: if is_current {
                stored_path.to_string()
            } else {
                derive_rel_path(&absolute, data_dir).unwrap_or_else(|_| absolute.into_owned())
            },
            version: Path::new(name_str)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map_or(0, version_number),
            facts,
            modified_ms: mtime_ms(&meta),
            size: meta.len(),
            current: is_current,
        });
    }

    // By number, so `v11` comes after `v4`; the name settles two files of the
    // same number (`photo.jpg` and `photo.webp`).
    versions.sort_by(|a, b| a.version.cmp(&b.version).then_with(|| a.path.cmp(&b.path)));
    Ok(versions)
}

fn asset_row(db: &AppDbState, asset_id: &str) -> Result<AssetRow, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock poisoned: {e}"))?;
    conn.query_row(
        "SELECT path, type, parent_asset_id FROM assets WHERE id = ?1",
        [asset_id],
        |row| {
            Ok(AssetRow {
                path: row.get(0)?,
                asset_type: row.get(1)?,
                parent_asset_id: row.get(2)?,
            })
        },
    )
    .map_err(|e| format!("not_found: asset {asset_id}: {e}"))
}

/// The files of an image asset's version family, the original first.
///
/// The family is the one of the file `assets.path` names for `asset_id`; the
/// frontend never passes a path. `with_sha` hashes every file, so it is asked
/// for only when there is a `source_sha256` to compare the versions with.
#[tauri::command]
pub async fn list_image_versions(
    asset_id: String,
    with_sha: Option<bool>,
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<Vec<ImageVersion>, String> {
    let db = db.inner().clone();
    let asset = run_blocking_db_task(move || asset_row(&db, &asset_id)).await?;
    require_image_asset(&asset)?;
    let data_dir = crate::path_utils::data_dir(&app)?;
    // The database is released by now: reading the files blocks no query.
    tokio::task::spawn_blocking(move || {
        list_family(&asset.path, &data_dir, with_sha.unwrap_or(false))
    })
    .await
    .map_err(|e| format!("Image versions task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_orient::test_support::{camera_jpeg, encode_jpeg, upright_quadrants};
    use crate::sync::blobs::hash_file;
    use std::path::PathBuf;

    /// An item directory inside a data directory, as the import flow lays it.
    fn item_dir(data: &tempfile::TempDir) -> PathBuf {
        let dir = data.path().join("assets").join("col-1").join("item-1");
        std::fs::create_dir_all(&dir).expect("create item dir");
        dir
    }

    fn write_jpeg(dir: &Path, name: &str, width: u32, height: u32) {
        std::fs::write(
            dir.join(name),
            encode_jpeg(upright_quadrants(width, height)),
        )
        .expect("write fixture");
    }

    fn paths(versions: &[ImageVersion]) -> Vec<&str> {
        versions.iter().map(|v| v.path.as_str()).collect()
    }

    #[test]
    fn versions_come_back_by_number_with_the_original_first_and_the_current_one_flagged() {
        let data = tempfile::tempdir().expect("tempdir");
        let dir = item_dir(&data);
        // Written out of order: neither the directory order nor the names decide.
        for name in ["photo_v11.jpg", "photo_v2.jpg", "photo.jpg", "photo_v4.png"] {
            write_jpeg(&dir, name, 64, 32);
        }

        let versions =
            list_family("assets/col-1/item-1/photo_v4.png", data.path(), false).expect("versions");

        assert_eq!(
            paths(&versions),
            [
                "assets/col-1/item-1/photo.jpg",
                "assets/col-1/item-1/photo_v2.jpg",
                "assets/col-1/item-1/photo_v4.png",
                "assets/col-1/item-1/photo_v11.jpg",
            ]
        );
        assert_eq!(
            versions.iter().map(|v| v.version).collect::<Vec<_>>(),
            [0, 2, 4, 11]
        );
        assert_eq!(
            versions.iter().map(|v| v.current).collect::<Vec<_>>(),
            [false, false, true, false]
        );
        let original = &versions[0];
        assert_eq!(
            original.size,
            std::fs::metadata(dir.join("photo.jpg"))
                .expect("meta")
                .len()
        );
        assert!(original.modified_ms > 0);
        assert_eq!(original.facts.sha256, None, "nothing is hashed unasked");
    }

    #[test]
    fn the_files_of_a_sibling_asset_with_a_similar_stem_are_not_listed() {
        let data = tempfile::tempdir().expect("tempdir");
        let dir = item_dir(&data);
        for name in ["photo.jpg", "photo_v2.jpg"] {
            write_jpeg(&dir, name, 64, 32);
        }
        for name in [
            "photo_final.jpg",
            "photo_final_v2.jpg",
            "photography.jpg",
            "photo_v2x.jpg",
            "photo2_v3.jpg",
        ] {
            write_jpeg(&dir, name, 64, 32);
        }
        std::fs::write(dir.join("photo.pdf"), b"%PDF-1.7\n%%EOF\n").expect("write pdf");
        // A directory named like a version is not a version.
        std::fs::create_dir(dir.join("photo_v3.jpg")).expect("create dir");

        let versions =
            list_family("assets/col-1/item-1/photo.jpg", data.path(), false).expect("versions");

        assert_eq!(
            paths(&versions),
            [
                "assets/col-1/item-1/photo.jpg",
                "assets/col-1/item-1/photo_v2.jpg",
            ]
        );
    }

    #[test]
    fn a_camera_photo_reports_its_upright_size_and_its_tag() {
        let data = tempfile::tempdir().expect("tempdir");
        let dir = item_dir(&data);
        // What the person sees is 64×32; the file holds it sideways.
        std::fs::write(
            dir.join("photo.jpg"),
            camera_jpeg(&upright_quadrants(64, 32), 6),
        )
        .expect("write original");
        // An edit saves the upright picture with no tag.
        write_jpeg(&dir, "photo_v2.jpg", 40, 20);

        let versions =
            list_family("assets/col-1/item-1/photo_v2.jpg", data.path(), false).expect("versions");

        let facts: Vec<_> = versions
            .iter()
            .map(|v| (v.facts.orientation, v.facts.width, v.facts.height))
            .collect();
        assert_eq!(facts, [(6, Some(64), Some(32)), (1, Some(40), Some(20))]);
    }

    #[test]
    fn a_missing_current_file_still_lists_the_ones_that_are_there() {
        let data = tempfile::tempdir().expect("tempdir");
        let dir = item_dir(&data);
        for name in ["photo.jpg", "photo_v2.jpg"] {
            write_jpeg(&dir, name, 64, 32);
        }

        // `assets.path` names a version that is no longer on disk.
        let versions =
            list_family("assets/col-1/item-1/photo_v3.jpg", data.path(), false).expect("versions");

        assert_eq!(
            paths(&versions),
            [
                "assets/col-1/item-1/photo.jpg",
                "assets/col-1/item-1/photo_v2.jpg",
            ]
        );
        assert!(versions.iter().all(|v| !v.current));

        // Not even the directory is there.
        let none =
            list_family("assets/col-1/ghost/photo.jpg", data.path(), false).expect("no directory");
        assert!(none.is_empty());
    }

    #[test]
    fn every_version_is_hashed_when_the_hash_is_asked_for() {
        let data = tempfile::tempdir().expect("tempdir");
        let dir = item_dir(&data);
        write_jpeg(&dir, "photo.jpg", 64, 32);
        write_jpeg(&dir, "photo_v2.jpg", 40, 20);

        let versions =
            list_family("assets/col-1/item-1/photo.jpg", data.path(), true).expect("versions");

        let hashes: Vec<_> = versions.iter().map(|v| v.facts.sha256.clone()).collect();
        assert_eq!(
            hashes,
            [
                Some(hash_file(&dir.join("photo.jpg")).expect("hash").0),
                Some(hash_file(&dir.join("photo_v2.jpg")).expect("hash").0),
            ]
        );
    }

    #[test]
    fn the_current_file_keeps_the_stored_path_as_it_is_written() {
        let data = tempfile::tempdir().expect("tempdir");
        let dir = item_dir(&data);
        write_jpeg(&dir, "photo.jpg", 64, 32);
        write_jpeg(&dir, "photo_v2.jpg", 40, 20);

        // A row written before the relative-path migration holds an absolute path.
        let stored = dir.join("photo_v2.jpg").to_string_lossy().into_owned();
        let versions = list_family(&stored, data.path(), false).expect("versions");

        assert_eq!(
            paths(&versions),
            ["assets/col-1/item-1/photo.jpg", stored.as_str()]
        );
        assert!(versions[1].current);
    }

    #[test]
    fn a_path_outside_the_data_directory_is_refused() {
        let data = tempfile::tempdir().expect("tempdir");
        item_dir(&data);
        let outside = tempfile::tempdir().expect("tempdir outside");
        write_jpeg(outside.path(), "photo.jpg", 64, 32);

        let stored = outside
            .path()
            .join("photo.jpg")
            .to_string_lossy()
            .into_owned();
        assert!(list_family(&stored, data.path(), false).is_err());
        assert!(list_family("assets/../../photo.jpg", data.path(), false).is_err());
        assert!(list_family("", data.path(), false).is_err());
    }

    #[test]
    fn only_an_image_that_is_its_own_document_has_versions() {
        let asset = |asset_type: &str, parent: Option<&str>| AssetRow {
            path: "assets/col-1/item-1/photo.jpg".to_string(),
            asset_type: asset_type.to_string(),
            parent_asset_id: parent.map(str::to_string),
        };

        assert!(require_image_asset(&asset("image", None)).is_ok());
        assert!(require_image_asset(&asset("pdf", None)).is_err());
        assert!(require_image_asset(&asset("audio", None)).is_err());
        // A page of a PDF belongs to its container.
        assert!(require_image_asset(&asset("image", Some("asset-pdf"))).is_err());
    }

    #[test]
    fn the_version_number_is_read_as_a_number() {
        assert_eq!(version_number("photo"), 0);
        assert_eq!(version_number("photo_v2"), 2);
        assert_eq!(version_number("photo_v011"), 11);
        assert_eq!(version_number("photo_vX"), 0);
        assert_eq!(version_number("photo_v99999999999999999999999"), u64::MAX);
    }
}
