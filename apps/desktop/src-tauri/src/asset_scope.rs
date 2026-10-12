//! The subdirectories of the archive the `asset:` protocol serves to the webview.
//!
//! The three `tauri*.conf.json` files declare the same list statically
//! (`app.security.assetProtocol.scope.allow`, with the `deny` globs that keep
//! the database and the saved capture HTML out); setup grants it again from the
//! resolved data and cache directories, so a dev profile or a moved archive
//! resolves to the same subdirectories instead of a second, hand-spelled copy
//! of the roots. Keep both halves in step in one change.
//!
//! Only what the renderer loads through `convertFileSrc` belongs here. A path
//! the frontend never turns into an `asset:` URL — dictation scratch files,
//! sample staging, audio previews — stays out of the asset protocol even when
//! the fs plugin scope covers it.

use std::path::Path;

/// Archive (data directory) subdirectories the webview loads through `asset:`:
/// imported assets (`lib/file-import.ts` `getAssetUrl`), manuscript images and
/// citation crops (`lib/writing-images.ts`, `lib/writing-crops.ts`) and the
/// saved web PDFs the Navegador viewer opens (`navegador_pdf_file`).
pub const ASSET_DATA_DIRS: [&str; 4] =
    ["assets", "writing-images", "writing-crops", "web-captures"];

/// Cache (local data directory) subdirectories the webview loads through
/// `asset:`: the thumbnails `generate_pdf_thumbnail` and
/// `generate_image_thumbnail` write and `lib/file-import.ts` converts.
pub const ASSET_CACHE_DIRS: [&str; 1] = ["thumbnails"];

/// Grants the asset protocol the subdirectories above under the resolved
/// archive and cache directories, and forbids the database files.
pub fn grant_asset_protocol_scope<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    app_dir: &Path,
    cache_dir: &Path,
) -> Result<(), String> {
    use tauri::Manager;
    let scope = app.asset_protocol_scope();
    let granted = ASSET_DATA_DIRS
        .iter()
        .map(|dir| app_dir.join(dir))
        .chain(ASSET_CACHE_DIRS.iter().map(|dir| cache_dir.join(dir)));
    for dir in granted {
        scope
            .allow_directory(&dir, true)
            .map_err(|e| format!("Could not grant {}: {e}", dir.display()))?;
    }
    // `forbid_file` escapes globs, so a runtime deny can only name exact files:
    // the database files are forbidden one by one (like `fs_scope`), while the
    // configs' `**/*.sqlite*` deny covers every other database name under the
    // shared roots.
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let db_file = app_dir.join(format!("{}{suffix}", crate::SQLITE_BASENAME));
        scope
            .forbid_file(&db_file)
            .map_err(|e| format!("Could not forbid {}: {e}", db_file.display()))?;
    }
    Ok(())
}
