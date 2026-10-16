//! Backups of the archive (T-47): the database plus every file the archive
//! keeps next to it, copied into a folder the user picked, by hand or on a
//! schedule (daily or weekly), keeping the last N copies.
//!
//! Each copy is a plain folder `entropia-backup-<UTC stamp>` that holds the
//! archive as it is on disk, so restoring is copying it back. It is written
//! under a `.partial` name and renamed only once complete, so a cut copy never
//! looks like a good one and pruning never counts it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::State;

use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;
use crate::settings::{get_setting, persist_setting};

/// The folder copies go to. Backend-only: the backup writes there and prunes
/// old copies in it, so only the native folder picker sets it (as S-01).
pub const BACKUP_DIR_KEY: &str = "backend_grant.backup_dir";
/// Epoch milliseconds of the last complete copy, written by the backend only.
pub const BACKUP_LAST_AT_KEY: &str = "backend_grant.backup_last_at";
/// `never` (default), `daily` or `weekly`.
pub const BACKUP_FREQUENCY_KEY: &str = "backup_frequency";
/// How many copies to keep, 1..=100; 5 by default.
pub const BACKUP_KEEP_KEY: &str = "backup_keep";
pub const DEFAULT_BACKUP_KEEP: usize = 5;

const COPY_PREFIX: &str = "entropia-backup-";
const PARTIAL_SUFFIX: &str = ".partial";
const DB_FILE: &str = "entropia.sqlite";
/// What the copy leaves out of the archive folder: the live database files
/// (the copy takes a consistent snapshot instead), lock files, other archives
/// (dev profiles) and models, which are downloaded again on demand.
const SKIPPED: [&str; 5] = [
    "entropia.sqlite",
    "entropia.sqlite-wal",
    "entropia.sqlite-shm",
    "dev-profiles",
    "models",
];

static RUNNING: AtomicBool = AtomicBool::new(false);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `2026-10-12_153005` in UTC: sortable, so name order is age order.
fn stamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn interval_ms(frequency: &str) -> Option<i64> {
    match frequency {
        "daily" => Some(86_400_000),
        "weekly" => Some(7 * 86_400_000),
        _ => None,
    }
}

/// Whether a scheduled copy is due now.
fn is_due(frequency: &str, last_at: Option<i64>, now: i64) -> bool {
    match interval_ms(frequency) {
        Some(interval) => last_at.is_none_or(|last| now - last >= interval),
        None => false,
    }
}

fn keep_setting(conn: &rusqlite::Connection) -> usize {
    get_setting(conn, BACKUP_KEEP_KEY)
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, 100))
        .unwrap_or(DEFAULT_BACKUP_KEEP)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copies of this app in `dir`, oldest first; partial ones are not copies.
fn complete_copies(dir: &Path) -> Vec<PathBuf> {
    let mut copies: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.starts_with(COPY_PREFIX) && !name.ends_with(PARTIAL_SUFFIX)).then(|| e.path())
        })
        .collect();
    copies.sort();
    copies
}

fn prune(dir: &Path, keep: usize) {
    let copies = complete_copies(dir);
    let excess = copies.len().saturating_sub(keep);
    for old in &copies[..excess] {
        if let Err(error) = std::fs::remove_dir_all(old) {
            log::warn!(target: "backup", "could not delete old copy {}: {error}", old.display());
        }
    }
}

/// Makes one copy of the archive at `db_path` into the chosen folder and
/// prunes the old ones. Returns the new copy's folder.
pub fn run_backup(db_path: &Path) -> Result<PathBuf, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Ya hay una copia de seguridad en curso.".into());
    }
    let result = run_backup_unguarded(db_path);
    RUNNING.store(false, Ordering::SeqCst);
    result
}

fn run_backup_unguarded(db_path: &Path) -> Result<PathBuf, String> {
    let conn = open_archive_connection(db_path)?;
    let dir = get_setting(&conn, BACKUP_DIR_KEY)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .ok_or("Todavía no elegiste la carpeta para las copias.")?;
    if !dir.is_dir() {
        return Err(format!(
            "No se encuentra la carpeta de copias {} (¿un disco desconectado?).",
            dir.display()
        ));
    }
    let archive = db_path.parent().ok_or("The archive has no folder")?;

    // A copy cut short by a crash or a closed app is garbage, never a copy.
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(COPY_PREFIX) && name.ends_with(PARTIAL_SUFFIX) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }

    let now = now_ms();
    let name = format!("{COPY_PREFIX}{}", stamp(now));
    let partial = dir.join(format!("{name}{PARTIAL_SUFFIX}"));
    let done = dir.join(&name);
    let fail = |what: &str, error: String| {
        let _ = std::fs::remove_dir_all(&partial);
        format!("{what}: {error}")
    };

    std::fs::create_dir_all(&partial)
        .map_err(|e| fail("No se pudo crear la carpeta de la copia", e.to_string()))?;
    // One read transaction: a consistent snapshot while the app keeps writing.
    conn.execute(
        "VACUUM INTO ?1",
        [partial.join(DB_FILE).to_string_lossy().as_ref()],
    )
    .map_err(|e| fail("No se pudo copiar la base de datos", e.to_string()))?;
    for entry in std::fs::read_dir(archive)
        .map_err(|e| fail("No se pudo leer la carpeta del archivo", e.to_string()))?
        .flatten()
    {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if SKIPPED.contains(&name.as_ref()) || name.ends_with(".lock") {
            continue;
        }
        let target = partial.join(&file_name);
        let copied = match entry.file_type() {
            Ok(kind) if kind.is_dir() => copy_tree(&entry.path(), &target),
            Ok(kind) if kind.is_file() => std::fs::copy(entry.path(), &target).map(|_| ()),
            _ => Ok(()),
        };
        copied.map_err(|e| fail(&format!("No se pudo copiar {name}"), e.to_string()))?;
    }
    std::fs::rename(&partial, &done)
        .map_err(|e| fail("No se pudo cerrar la copia", e.to_string()))?;

    persist_setting(&conn, BACKUP_LAST_AT_KEY, &now.to_string())?;
    prune(&dir, keep_setting(&conn));
    log::info!(target: "backup", "copy written to {}", done.display());
    Ok(done)
}

/// Checks once an hour whether a scheduled copy is due, from a little after
/// startup on. A failed copy is logged and retried at the next check.
pub fn spawn_scheduler(db_path: PathBuf) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(120));
        loop {
            let due = open_archive_connection(&db_path).map(|conn| {
                let frequency = get_setting(&conn, BACKUP_FREQUENCY_KEY).unwrap_or_default();
                let last_at = get_setting(&conn, BACKUP_LAST_AT_KEY).and_then(|v| v.parse().ok());
                let has_dir = get_setting(&conn, BACKUP_DIR_KEY).is_some_and(|d| !d.is_empty());
                has_dir && is_due(&frequency, last_at, now_ms())
            });
            if due.unwrap_or(false) {
                if let Err(error) = run_backup(&db_path) {
                    log::warn!(target: "backup", "scheduled copy failed: {error}");
                }
            }
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

/// Validates a folder the user picked for the copies and stores it. It cannot
/// be inside the archive, or each copy would copy the earlier ones.
fn grant_backup_dir(
    conn: &rusqlite::Connection,
    archive: &Path,
    picked: &Path,
) -> Result<String, String> {
    let dir = std::fs::canonicalize(picked)
        .map_err(|e| format!("No se puede leer la carpeta {}: {e}", picked.display()))?;
    let archive = std::fs::canonicalize(archive).map_err(|e| e.to_string())?;
    if dir.starts_with(&archive) {
        return Err("La carpeta de copias no puede estar dentro de la carpeta de EntropIA.".into());
    }
    let value = dir.to_string_lossy().into_owned();
    persist_setting(conn, BACKUP_DIR_KEY, &value)?;
    Ok(value)
}

/// Opens the native folder picker for the backup folder. Returns the stored
/// path, or `None` when the user cancels.
#[tauri::command]
pub async fn backup_dir_grant(
    app: tauri::AppHandle,
    db: State<'_, AppDbState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = tokio::task::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| format!("The folder picker failed: {e}"))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let picked = picked
        .into_path()
        .map_err(|e| format!("The chosen folder is not a local path: {e}"))?;
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let archive = db_path.parent().ok_or("The archive has no folder")?;
        grant_backup_dir(&conn, archive, &picked).map(Some)
    })
    .await
    .map_err(|e| format!("backup task failed: {e}"))?
}

/// Makes a copy now. Returns the new copy's folder.
#[tauri::command]
pub async fn backup_run_now(db: State<'_, AppDbState>) -> Result<String, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || run_backup(&db_path))
        .await
        .map_err(|e| format!("backup task failed: {e}"))?
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive_with_settings(settings: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let archive = tempfile::tempdir().unwrap();
        let db_path = archive.path().join(DB_FILE);
        let conn = open_archive_connection(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('hola');",
        )
        .unwrap();
        for (key, value) in settings {
            persist_setting(&conn, key, value).unwrap();
        }
        (archive, db_path)
    }

    #[test]
    fn the_stamp_is_a_sortable_utc_date() {
        assert_eq!(stamp(0), "1970-01-01_000000");
        assert_eq!(stamp(1_791_821_820_590), "2026-10-12_161700");
        assert_eq!(stamp(951_782_400_000), "2000-02-29_000000");
    }

    #[test]
    fn a_copy_is_due_only_when_its_interval_has_passed() {
        let day = 86_400_000;
        assert!(!is_due("never", None, 10 * day));
        assert!(is_due("daily", None, 10 * day));
        assert!(!is_due("daily", Some(10 * day), 10 * day + day - 1));
        assert!(is_due("daily", Some(10 * day), 11 * day));
        assert!(!is_due("weekly", Some(10 * day), 16 * day));
        assert!(is_due("weekly", Some(10 * day), 17 * day));
    }

    #[test]
    fn a_copy_holds_the_database_and_the_files_and_old_ones_are_pruned() {
        let target = tempfile::tempdir().unwrap();
        let dir = target.path().to_string_lossy().into_owned();
        let (archive, db_path) =
            archive_with_settings(&[(BACKUP_DIR_KEY, &dir), (BACKUP_KEEP_KEY, "2")]);
        std::fs::create_dir_all(archive.path().join("assets/a")).unwrap();
        std::fs::write(archive.path().join("assets/a/doc.pdf"), b"%PDF").unwrap();
        std::fs::create_dir_all(archive.path().join("dev-profiles/x")).unwrap();
        std::fs::write(archive.path().join("entropia.processing.lock"), b"").unwrap();
        // Three older copies and a cut one.
        for old in [
            "2001-01-01_000000",
            "2002-01-01_000000",
            "2003-01-01_000000",
        ] {
            std::fs::create_dir(target.path().join(format!("{COPY_PREFIX}{old}"))).unwrap();
        }
        let cut = target
            .path()
            .join(format!("{COPY_PREFIX}2004-01-01_000000{PARTIAL_SUFFIX}"));
        std::fs::create_dir(&cut).unwrap();
        // Something else of the user's in the same folder.
        std::fs::create_dir(target.path().join("fotos")).unwrap();

        let copy = run_backup(&db_path).expect("backup");

        let restored = rusqlite::Connection::open(copy.join(DB_FILE)).unwrap();
        let body: String = restored
            .query_row("SELECT body FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "hola");
        assert_eq!(
            std::fs::read(copy.join("assets/a/doc.pdf")).unwrap(),
            b"%PDF"
        );
        assert!(!copy.join("dev-profiles").exists());
        assert!(!copy.join("entropia.processing.lock").exists());

        let left = complete_copies(target.path());
        assert_eq!(left.len(), 2);
        assert_eq!(
            left[0].file_name().unwrap(),
            "entropia-backup-2003-01-01_000000"
        );
        assert_eq!(left[1], copy);
        assert!(!cut.exists());
        assert!(target.path().join("fotos").exists());

        let conn = open_archive_connection(&db_path).unwrap();
        assert!(get_setting(&conn, BACKUP_LAST_AT_KEY).is_some());
    }

    #[test]
    fn without_a_folder_there_is_no_copy() {
        let (_archive, db_path) = archive_with_settings(&[]);
        assert!(run_backup_unguarded(&db_path).is_err());
    }

    #[test]
    fn the_backup_folder_cannot_be_inside_the_archive() {
        let (archive, db_path) = archive_with_settings(&[]);
        let conn = open_archive_connection(&db_path).unwrap();
        let inside = archive.path().join("copias");
        std::fs::create_dir(&inside).unwrap();
        assert!(grant_backup_dir(&conn, archive.path(), &inside).is_err());

        let outside = tempfile::tempdir().unwrap();
        grant_backup_dir(&conn, archive.path(), outside.path()).expect("granted");
        assert!(get_setting(&conn, BACKUP_DIR_KEY).is_some());
    }
}
