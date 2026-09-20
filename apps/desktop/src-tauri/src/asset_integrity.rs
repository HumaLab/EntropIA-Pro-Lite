//! Reconciliation between `assets` rows and the files they claim to point at.
//!
//! Nothing in the migration or startup path ever checked that `assets.path`
//! resolves to a real file. A scan of the real 1.0.13 archive found one row
//! out of 4133 whose file was simply gone — an 8.7 MB `.wav` — and nothing
//! noticed. This module answers one question, cheaply and repeatedly: how
//! many rows point at nothing.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How many missing rows travel back over IPC (or into a log line) at once.
/// The archive this was written for has 4133 rows; the command must never
/// hand the frontend all of them just to say "one is missing".
pub const MISSING_SAMPLE_CAP: usize = 20;

/// One row whose file could not be found where `assets.path` says it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissingAsset {
    pub id: String,
    pub item_id: String,
    pub path: String,
}

/// The result of walking `assets` and checking each row's file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetIntegrityReport {
    /// Rows read from `assets`, including ones whose file was found.
    pub checked: usize,
    /// Rows whose file could not be found (or resolved outside the archive).
    pub missing: usize,
    /// A bounded, non-exhaustive sample of the missing rows — see
    /// [`MISSING_SAMPLE_CAP`].
    pub missing_sample: Vec<MissingAsset>,
}

/// Walk every row in `assets` and check whether its file exists on disk.
///
/// `data_dir` is the archive root `assets.path` is stored relative to — the
/// same directory [`crate::path_utils::resolve_asset_path`] resolves
/// against. A missing `assets` table (never migrated yet, or a fresh
/// archive) reads as zero rows rather than an error: there is nothing to
/// reconcile, not a failure.
pub fn reconcile_assets(
    conn: &Connection,
    data_dir: &Path,
    sample_cap: usize,
) -> Result<AssetIntegrityReport, String> {
    let has_assets_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='assets' LIMIT 1",
            [],
            |_row| Ok(true),
        )
        .unwrap_or(false);

    if !has_assets_table {
        return Ok(AssetIntegrityReport {
            checked: 0,
            missing: 0,
            missing_sample: Vec::new(),
        });
    }

    let mut stmt = conn
        .prepare("SELECT id, item_id, path FROM assets")
        .map_err(|error| format!("Failed to read assets for the integrity check: {error}"))?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|error| format!("Failed to read assets for the integrity check: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read assets for the integrity check: {error}"))?;

    let mut checked = 0usize;
    let mut missing = 0usize;
    let mut missing_sample = Vec::new();

    for (id, item_id, path) in rows {
        checked += 1;
        if asset_file_present(&path, data_dir) {
            continue;
        }
        missing += 1;
        if missing_sample.len() < sample_cap {
            missing_sample.push(MissingAsset { id, item_id, path });
        }
    }

    Ok(AssetIntegrityReport {
        checked,
        missing,
        missing_sample,
    })
}

/// True when `stored` resolves to a real file actually inside the archive.
///
/// An absolute stored value is trusted as-is — `resolve_asset_path` returns
/// it unchanged, and that is by design: it is how an external file that was
/// never copied in gets recorded. A relative key is different: it must
/// resolve *inside* `data_dir`. One that escapes it (a corrupt or crafted
/// `..` segment) is never reported present, even when a file genuinely
/// exists at the escaped location — reporting that as "found" would make a
/// row look intact while pointing outside the archive it belongs to.
fn asset_file_present(stored: &str, data_dir: &Path) -> bool {
    let candidate = Path::new(stored);
    if candidate.is_absolute() {
        return candidate.is_file();
    }

    let resolved = crate::path_utils::resolve_asset_path(stored, data_dir);
    match crate::path_utils::ensure_within_dir(&resolved, data_dir) {
        Ok(canonical) => canonical.is_file(),
        Err(_) => false,
    }
}

/// The single `[setup]`-style line a startup scan logs, in the style of the
/// other `eprintln!("[setup] …")` lines this module runs alongside.
fn format_integrity_log_line(report: &AssetIntegrityReport) -> String {
    format!(
        "[setup] asset integrity: checked {}, missing {}",
        report.checked, report.missing
    )
}

/// Resolve the data directory and open a database command, walking `assets`
/// and reporting the reconciliation. The frontend can call this on demand;
/// [`spawn_startup_scan`] calls the same underlying logic automatically.
#[tauri::command]
pub async fn assets_check_integrity(
    app_handle: tauri::AppHandle,
) -> Result<AssetIntegrityReport, String> {
    tokio::task::spawn_blocking(move || {
        let data_dir = crate::path_utils::data_dir(&app_handle)
            .map_err(|error| format!("Failed to resolve the data dir for the asset integrity check: {error}"))?;
        let db_path = data_dir.join(crate::SQLITE_BASENAME);
        let conn = crate::db::open::open_archive_connection(&db_path)
            .map_err(|error| format!("Failed to open the database for the asset integrity check: {error}"))?;
        reconcile_assets(&conn, &data_dir, MISSING_SAMPLE_CAP)
    })
    .await
    .map_err(|error| format!("Asset integrity check task panicked: {error}"))?
}

/// Run the same reconciliation at startup, off the critical path.
///
/// A background OS thread, not the async runtime: startup must never wait on
/// this, and it must never be able to fail startup — a missing asset file is
/// worth knowing about, not worth blocking the window over. Opens its own
/// connection so it never contends with the UI connection's lock.
pub fn spawn_startup_scan(db_path: std::path::PathBuf, data_dir: std::path::PathBuf) {
    std::thread::spawn(move || match crate::db::open::open_archive_connection(&db_path) {
        Ok(conn) => match reconcile_assets(&conn, &data_dir, MISSING_SAMPLE_CAP) {
            Ok(report) => eprintln!("{}", format_integrity_log_line(&report)),
            Err(error) => eprintln!("[setup] asset integrity scan failed: {error}"),
        },
        Err(error) => eprintln!("[setup] asset integrity scan skipped: {error}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_assets(db_path: &Path, rows: &[(&str, &str, &str)]) {
        let conn = Connection::open(db_path).expect("open db");
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL);",
        )
        .expect("create assets table");
        for (id, item_id, path) in rows {
            conn.execute(
                "INSERT INTO assets(id, item_id, path) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, item_id, path],
            )
            .expect("insert asset row");
        }
    }

    #[test]
    fn a_row_whose_file_exists_is_not_reported_missing() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(data_dir.path().join("assets").join("col").join("item"))
            .expect("asset dir");
        std::fs::write(
            data_dir
                .path()
                .join("assets")
                .join("col")
                .join("item")
                .join("photo.jpg"),
            b"bytes",
        )
        .expect("write asset file");

        let db_path = data_dir.path().join("entropia.sqlite");
        seed_assets(
            &db_path,
            &[("asset-1", "item-1", "assets/col/item/photo.jpg")],
        );
        let conn = Connection::open(&db_path).expect("open db");

        let report =
            reconcile_assets(&conn, data_dir.path(), MISSING_SAMPLE_CAP).expect("reconcile");

        assert_eq!(report.checked, 1);
        assert_eq!(report.missing, 0);
        assert!(report.missing_sample.is_empty());
    }

    #[test]
    fn a_row_whose_file_is_missing_is_reported_and_sampled() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let db_path = data_dir.path().join("entropia.sqlite");
        seed_assets(
            &db_path,
            &[("asset-1", "item-1", "assets/col/item/gone.wav")],
        );
        let conn = Connection::open(&db_path).expect("open db");

        let report =
            reconcile_assets(&conn, data_dir.path(), MISSING_SAMPLE_CAP).expect("reconcile");

        assert_eq!(report.checked, 1);
        assert_eq!(report.missing, 1);
        assert_eq!(
            report.missing_sample,
            vec![MissingAsset {
                id: "asset-1".to_string(),
                item_id: "item-1".to_string(),
                path: "assets/col/item/gone.wav".to_string(),
            }]
        );
    }

    #[test]
    fn an_archive_with_no_assets_at_all_reports_zero() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let db_path = data_dir.path().join("entropia.sqlite");
        seed_assets(&db_path, &[]);
        let conn = Connection::open(&db_path).expect("open db");

        let report =
            reconcile_assets(&conn, data_dir.path(), MISSING_SAMPLE_CAP).expect("reconcile");

        assert_eq!(report.checked, 0);
        assert_eq!(report.missing, 0);
        assert!(report.missing_sample.is_empty());
    }

    #[test]
    fn a_path_that_escapes_the_archive_root_is_never_reported_present() {
        let parent = tempfile::tempdir().expect("tempdir");
        let data_dir = parent.path().join("archive");
        std::fs::create_dir_all(&data_dir).expect("data dir");

        // The file genuinely exists — but outside the archive root, reached
        // only by a stored `..` segment. Existing on disk must not be enough
        // to count as "found": the row still points outside the archive.
        let outside_dir = parent.path().join("outside");
        std::fs::create_dir_all(&outside_dir).expect("outside dir");
        std::fs::write(outside_dir.join("secret.wav"), b"bytes").expect("write outside file");

        let db_path = data_dir.join(crate::SQLITE_BASENAME);
        seed_assets(&db_path, &[("asset-1", "item-1", "../outside/secret.wav")]);
        let conn = Connection::open(&db_path).expect("open db");

        let report = reconcile_assets(&conn, &data_dir, MISSING_SAMPLE_CAP).expect("reconcile");

        assert_eq!(report.checked, 1);
        assert_eq!(
            report.missing, 1,
            "a file outside the archive root must never be reported present"
        );
    }

    #[test]
    fn the_startup_log_line_names_checked_and_missing_counts() {
        let report = AssetIntegrityReport {
            checked: 4133,
            missing: 1,
            missing_sample: Vec::new(),
        };

        assert_eq!(
            format_integrity_log_line(&report),
            "[setup] asset integrity: checked 4133, missing 1"
        );
    }
}
