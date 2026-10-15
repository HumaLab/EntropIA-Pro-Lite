//! The file-system scope the renderer gets through `@tauri-apps/plugin-fs`.
//!
//! The capability grants no directory statically: setup grants exactly the
//! archive directories the frontend works in, resolved by `path_utils` like
//! the asset protocol scope, so dev profiles and a moved data directory are
//! covered by the same definition. Files the user picks in a dialog or drops
//! on the window are added at runtime by the dialog and fs plugins. The
//! archive database is never reachable (S-01).

use std::path::Path;

use tauri_plugin_fs::FsExt;

/// Archive (data directory) subdirectories the frontend reads, writes or
/// removes: `assets` (`lib/file-import.ts`, `lib/collection-import.ts`, the
/// collection and item deletions), `writing-images` (`lib/writing-images.ts`),
/// `writing-crops` (`lib/writing-crops.ts`), `temp` (dictation recordings,
/// `lib/transcription.ts`) and `sample-staging` (`lib/sample-collection.ts`).
pub const FRONTEND_ARCHIVE_DIRS: [&str; 5] = [
    "assets",
    "writing-images",
    "writing-crops",
    "temp",
    "sample-staging",
];

/// Cache subdirectories the frontend reads: the WAV previews
/// `prepare_audio_preview` writes (`lib/file-import.ts`).
pub const FRONTEND_CACHE_DIRS: [&str; 1] = ["audio-previews"];

/// Grants the renderer the archive and cache directories it works in and
/// nothing else.
pub fn grant_frontend_fs_scope<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    app_dir: &Path,
    cache_dir: &Path,
) -> Result<(), String> {
    let scope = app.fs_scope();
    let granted = FRONTEND_ARCHIVE_DIRS
        .iter()
        .map(|dir| app_dir.join(dir))
        .chain(FRONTEND_CACHE_DIRS.iter().map(|dir| cache_dir.join(dir)));
    for dir in granted {
        scope
            .allow_directory(&dir, true)
            .map_err(|e| format!("Could not grant {}: {e}", dir.display()))?;
    }
    // A forbidden path wins over every allowed one, including directories a
    // dialog or a drop adds later (dropping the archive folder itself grants
    // it recursively): the database files stay out of reach. Tauri escapes
    // these paths, so they are exact files; the capability's `deny` globs
    // cover any other `*.sqlite*` under the shared roots.
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let db_file = app_dir.join(format!("{}{suffix}", crate::SQLITE_BASENAME));
        scope
            .forbid_file(&db_file)
            .map_err(|e| format!("Could not forbid {}: {e}", db_file.display()))?;
    }
    Ok(())
}
