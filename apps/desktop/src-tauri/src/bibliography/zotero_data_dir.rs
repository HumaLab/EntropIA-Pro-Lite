//! The Zotero data directory the backend reads stored attachment copies from.
//!
//! It is a root for backend file reads, so the renderer never names it: the
//! `zotero_data_dir_grant` command opens the native folder picker itself and
//! hands the choice to [`grant_zotero_data_dir`] (S-01).

use std::path::Path;

use crate::bibliography::processing::ZOTERO_DATA_DIR_SETTING_KEY;

/// Validates a folder the user picked and stores it as the Zotero data
/// directory. The folder must exist and look like a Zotero data directory (a
/// `storage` folder or a `zotero.sqlite` file); it is stored canonicalized.
/// Returns the stored path.
pub fn grant_zotero_data_dir(conn: &rusqlite::Connection, picked: &Path) -> Result<String, String> {
    let dir = std::fs::canonicalize(picked)
        .map_err(|e| format!("The folder {} cannot be read: {e}", picked.display()))?;
    if !dir.join("storage").is_dir() && !dir.join("zotero.sqlite").is_file() {
        return Err(format!(
            "{} is not a Zotero data directory: it has no storage folder or zotero.sqlite",
            dir.display()
        ));
    }
    let value = dir.to_string_lossy().into_owned();
    crate::settings::persist_setting(conn, ZOTERO_DATA_DIR_SETTING_KEY, &value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("create app_settings");
        conn
    }

    #[test]
    fn a_zotero_folder_with_storage_is_granted_by_its_canonical_path() {
        let conn = settings_db();
        let zotero = tempfile::tempdir().unwrap();
        std::fs::create_dir(zotero.path().join("storage")).unwrap();
        // The picker may hand back a non-canonical spelling of the folder.
        let picked = zotero.path().join("storage").join("..");

        let granted = grant_zotero_data_dir(&conn, &picked).expect("granted");

        let canonical = std::fs::canonicalize(zotero.path()).unwrap();
        assert_eq!(granted, canonical.to_string_lossy());
        assert_eq!(
            crate::settings::get_setting(&conn, ZOTERO_DATA_DIR_SETTING_KEY).as_deref(),
            Some(granted.as_str())
        );
    }

    #[test]
    fn a_zotero_folder_with_only_its_database_is_granted() {
        let conn = settings_db();
        let zotero = tempfile::tempdir().unwrap();
        std::fs::write(zotero.path().join("zotero.sqlite"), b"").unwrap();

        grant_zotero_data_dir(&conn, zotero.path()).expect("granted");
    }

    #[test]
    fn a_folder_that_is_not_a_zotero_data_directory_is_refused() {
        let conn = settings_db();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("tesis.pdf"), b"%PDF").unwrap();

        grant_zotero_data_dir(&conn, elsewhere.path()).expect_err("not a Zotero folder");
        grant_zotero_data_dir(&conn, &elsewhere.path().join("missing")).expect_err("missing");
        assert_eq!(
            crate::settings::get_setting(&conn, ZOTERO_DATA_DIR_SETTING_KEY),
            None
        );
    }

    #[test]
    fn a_legacy_renderer_written_value_is_not_a_granted_root() {
        let conn = settings_db();
        let zotero = tempfile::tempdir().unwrap();
        std::fs::create_dir(zotero.path().join("storage")).unwrap();
        crate::settings::set_setting(&conn, "zotero_data_dir", &zotero.path().to_string_lossy())
            .unwrap();

        assert_eq!(
            crate::settings::get_setting(&conn, ZOTERO_DATA_DIR_SETTING_KEY),
            None
        );
    }
}
