use std::path::{Path, PathBuf};

/// Normalize Windows extended-length paths (`\\?\`) into plain filesystem paths.
///
/// Tauri resource resolution may return extended-length paths on Windows. Those
/// work for many Rust APIs, but they are noisy in logs and can confuse some
/// subprocesses/native libraries. On non-Windows platforms this is a no-op.
pub fn normalize_windows_path(path: impl AsRef<Path>) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.as_ref().to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }

    path.as_ref().to_path_buf()
}

pub fn normalize_windows_path_string(path: impl AsRef<Path>) -> String {
    normalize_windows_path(path).to_string_lossy().into_owned()
}

/// Canonicalize a path that must already exist and be a regular file.
///
/// Rejects empty paths, paths that cannot be canonicalized (broken or
/// nonexistent), and paths that resolve to something other than a file.
/// Returns the canonicalized path (on Windows this carries the `\\?\`
/// verbatim prefix — compare it only against other canonicalized paths).
pub fn validate_existing_file(path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("Path must not be empty".to_string());
    }

    let canonical =
        std::fs::canonicalize(path).map_err(|e| format!("Failed to resolve path '{path}': {e}"))?;

    if !canonical.is_file() {
        return Err(format!("Path is not a file: {path}"));
    }

    Ok(canonical)
}

/// Canonicalize a path whose deepest components may not exist yet (e.g. an
/// output directory that will be created later).
///
/// The deepest existing ancestor is canonicalized with `std::fs::canonicalize`
/// and the remaining (missing) components are appended verbatim. Any `..` or
/// `.` remnant in the missing tail is refused — it cannot be resolved against
/// the filesystem, so it could only be a traversal attempt.
pub fn canonicalize_allowing_missing_tail(path: impl AsRef<Path>) -> Result<PathBuf, String> {
    let path = path.as_ref();
    if path.as_os_str().is_empty() {
        return Err("Path must not be empty".to_string());
    }

    let mut existing = path.to_path_buf();
    let mut missing_tail: Vec<std::ffi::OsString> = Vec::new();

    loop {
        match std::fs::canonicalize(&existing) {
            Ok(canonical) => {
                let mut resolved = canonical;
                for component in missing_tail.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(_) => {
                let Some(file_name) = existing.file_name() else {
                    return Err(format!(
                        "Failed to resolve path '{}': no existing ancestor",
                        path.display()
                    ));
                };
                missing_tail.push(file_name.to_os_string());
                if !existing.pop() {
                    return Err(format!(
                        "Failed to resolve path '{}': no existing ancestor",
                        path.display()
                    ));
                }
            }
        }
    }
}

/// Ensure `path` resolves inside `root` (which must exist).
///
/// Both sides are canonicalized before comparison so Windows `\\?\` verbatim
/// prefixes, symlinks, and `..` traversal are all resolved consistently.
/// `path` itself may have a missing tail (see
/// [`canonicalize_allowing_missing_tail`]), but missing components must not
/// contain `..` or `.` remnants. Returns the canonicalized path on success.
pub fn ensure_within_dir(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
) -> Result<PathBuf, String> {
    let path = path.as_ref();
    let root = root.as_ref();

    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| format!("Failed to resolve directory '{}': {e}", root.display()))?;

    let canonical_path = canonicalize_allowing_missing_tail(path)?;

    if has_traversal_remnants(&canonical_path) {
        return Err(format!(
            "Path '{}' contains '..' traversal segments",
            path.display()
        ));
    }

    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "Path '{}' is outside the allowed directory '{}'",
            path.display(),
            root.display()
        ));
    }

    Ok(canonical_path)
}

/// True when a resolved path still carries `..`/`.` components. Canonicalized
/// prefixes never do; this only triggers for remnants in a missing tail.
fn has_traversal_remnants(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    })
}

// ---------------------------------------------------------------------------
// Asset path representation
//
// The relative form of an asset path is storage vocabulary, not a sync
// concern. It moved here from `sync::blobs`, which re-exports it.
// ---------------------------------------------------------------------------

/// Why a `rel_path` derivation failed. The caller maps these to a skipped row
/// plus a journaled `apply_error` (DESIGN §7).
#[derive(Debug, PartialEq, Eq)]
pub enum RelPathError {
    /// The local path is not inside the app-data dir (e.g. an external import
    /// that was never copied in). The row must be skipped, not pushed.
    OutsideAppData,
    /// After stripping the prefix the remainder did not begin with `assets/`.
    NotUnderAssets,
    /// The path was empty.
    Empty,
}

impl std::fmt::Display for RelPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelPathError::OutsideAppData => write!(f, "asset path is outside the app-data dir"),
            RelPathError::NotUnderAssets => write!(f, "asset path is not under assets/"),
            RelPathError::Empty => write!(f, "asset path is empty"),
        }
    }
}

/// Derives the wire `rel_path` from a local absolute `assets.path` (PROTOCOL
/// "Transformación de assets"):
///
/// 1. Strip the `data_dir` prefix.
/// 2. Normalize separators to `/`.
/// 3. Require the remainder to start with `assets/`.
///
/// Comparison is done on the string form normalized to `/` so a Windows
/// backslash path matches a forward-slash app-data dir. Rows whose path is
/// outside the app-data dir return [`RelPathError::OutsideAppData`] so the
/// caller skips + journals them (DESIGN §7).
pub fn derive_rel_path(abs_path: &str, data_dir: &Path) -> Result<String, RelPathError> {
    if abs_path.trim().is_empty() {
        return Err(RelPathError::Empty);
    }

    let normalize = |s: &str| s.replace('\\', "/");
    let path_norm = normalize(abs_path);
    let mut prefix_norm = normalize(&data_dir.to_string_lossy());
    if !prefix_norm.ends_with('/') {
        prefix_norm.push('/');
    }

    // Case-insensitive prefix match on Windows (drive letters/paths are
    // case-insensitive there); exact elsewhere.
    let starts_with_prefix = if cfg!(windows) {
        path_norm
            .to_ascii_lowercase()
            .starts_with(&prefix_norm.to_ascii_lowercase())
    } else {
        path_norm.starts_with(&prefix_norm)
    };
    if !starts_with_prefix {
        return Err(RelPathError::OutsideAppData);
    }

    // Slice off the matched prefix length from the ORIGINAL-normalized path so
    // the casing of the remainder (the assets/ subtree) is preserved verbatim.
    let rel = &path_norm[prefix_norm.len()..];
    let rel = rel.trim_start_matches('/');

    if !rel.starts_with("assets/") {
        return Err(RelPathError::NotUnderAssets);
    }

    Ok(rel.to_string())
}

/// Resolves a stored `assets.path` to a local filesystem path.
///
/// A relative key — the storage format this application is adopting — is joined
/// under `data_dir` component by component, after normalizing `\` to `/` so a
/// value written on Windows resolves the same way everywhere. An absolute path
/// is returned unchanged: that covers both a row written before the migration
/// and an external file that was never copied in.
///
/// This helper does not validate. Untrusted input keeps going through
/// `crate::sync::apply::validate_inbound_rel_path`, which refuses traversal,
/// drive letters, and UNC paths before resolving.
pub fn resolve_asset_path(stored: &str, data_dir: &Path) -> PathBuf {
    let candidate = Path::new(stored);
    if candidate.is_absolute() {
        return candidate.to_path_buf();
    }

    let mut resolved = data_dir.to_path_buf();
    for component in stored.replace('\\', "/").split('/') {
        if component.is_empty() {
            continue;
        }
        resolved.push(component);
    }
    resolved
}

/// The directory name every EntropIA variant shares.
///
/// Deliberately not any variant's Tauri identifier. Lite and Pro must stay
/// separately installable, and the identifier is what keeps their installers,
/// uninstall entries, and install directories apart — so neither can change,
/// and neither one's `app_data_dir()` can be the shared root. A fixed sibling
/// directory is the only option that leaves both identifiers alone.
pub const SHARED_DIR_NAME: &str = "com.entropia.shared";

/// The directory that holds the database and the assets.
///
/// Shared by Lite, Pro, and the dev build: whichever variant opens the app
/// reads and writes the same archive.
///
/// Every consumer resolves through this one function, so moving the directory
/// is a single edit rather than one per call site.
pub fn data_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    app_handle
        .path()
        .data_dir()
        .map(|dir| dir.join(SHARED_DIR_NAME))
        .map_err(|e| format!("Failed to resolve the data directory: {e}"))
}

/// The directory that holds regenerable weight.
///
/// Models, dependency runtimes, thumbnails, audio previews, logs and scratch
/// space live here rather than beside the database. `Roaming` is designed for
/// content that follows a user between machines in a domain, and a roaming
/// profile tries to copy it at every login — around 11 GB of redownloadable
/// runtime does not belong there. Keeping it out also means a backup of the
/// data directory is only what cannot be recovered from anywhere else.
///
/// Everything here can be deleted: the app redownloads or regenerates it.
pub fn cache_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    app_handle
        .path()
        .local_data_dir()
        .map(|dir| dir.join(SHARED_DIR_NAME))
        .map_err(|e| format!("Failed to resolve the cache directory: {e}"))
}

/// The cache directory, remembered for code too deep to hold an `AppHandle`.
///
/// The embedding configuration is built from a bare SQLite connection, several
/// layers below any Tauri handle. It used to derive its root from the database
/// file's parent, which was correct only while data and cache shared one
/// directory. Rather than thread a handle through half a dozen signatures, the
/// value is recorded once at startup.
static REMEMBERED_CACHE_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Record the cache directory. Called once, during setup.
pub fn remember_cache_dir(dir: PathBuf) {
    let _ = REMEMBERED_CACHE_DIR.set(dir);
}

/// The recorded cache directory, when the process has one.
///
/// `None` in unit tests and anywhere setup has not run; callers fall back to
/// their previous behaviour so a test does not depend on process-wide state.
pub fn remembered_cache_dir() -> Option<PathBuf> {
    REMEMBERED_CACHE_DIR.get().cloned()
}

/// Resolve a stored `assets.path` at a command boundary.
///
/// Commands receive the stored value from the frontend and hand it on to
/// workers, file reads, and sibling-path derivations. Resolving once on entry
/// means everything downstream keeps operating on an absolute path and needs no
/// knowledge of how the value is stored.
pub fn resolve_asset_path_at_boundary(
    stored: &str,
    app_handle: &tauri::AppHandle,
) -> Result<String, String> {
    let dir = data_dir(app_handle)?;
    Ok(resolve_asset_path(stored, &dir)
        .to_string_lossy()
        .into_owned())
}

/// The inverse of [`resolve_asset_path_at_boundary`]: turn a local absolute
/// path into the value to store in `assets.path`.
///
/// A command that resolves on the way in must relativize on the way out, or it
/// hands the frontend an absolute path that goes straight back into the column.
///
/// A path that cannot be expressed relative to the data directory — an external
/// file that was never copied in — is returned unchanged. Storing an absolute
/// path is still correct for such a row; refusing it would lose the asset.
pub fn store_asset_path_at_boundary(absolute: &str, app_handle: &tauri::AppHandle) -> String {
    let Ok(dir) = data_dir(app_handle) else {
        return absolute.to_string();
    };
    derive_rel_path(absolute, &dir).unwrap_or_else(|_| absolute.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_plain_paths_unchanged() {
        let path = PathBuf::from(r"C:\tmp\file.txt");
        assert_eq!(normalize_windows_path(&path), path);
    }

    #[test]
    fn strips_windows_extended_prefix() {
        #[cfg(windows)]
        {
            let path = PathBuf::from(r"\\?\C:\tmp\file.txt");
            assert_eq!(
                normalize_windows_path(path),
                PathBuf::from(r"C:\tmp\file.txt")
            );
        }
    }

    #[test]
    fn validate_existing_file_accepts_real_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("asset.png");
        std::fs::write(&file_path, b"data").expect("write file");

        let canonical = validate_existing_file(&file_path.to_string_lossy())
            .expect("existing file should validate");
        assert!(canonical.is_file());
    }

    #[test]
    fn validate_existing_file_rejects_empty_missing_and_directories() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(validate_existing_file("").is_err());
        assert!(validate_existing_file("   ").is_err());
        assert!(validate_existing_file(&dir.path().join("missing.png").to_string_lossy()).is_err());
        assert!(validate_existing_file(&dir.path().to_string_lossy()).is_err());
    }

    #[test]
    fn canonicalize_allowing_missing_tail_appends_missing_components() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("assets").join("col-1").join("item-1");

        let resolved =
            canonicalize_allowing_missing_tail(&target).expect("missing tail should resolve");
        let canonical_root = std::fs::canonicalize(dir.path()).expect("canonicalize root");
        assert!(resolved.starts_with(&canonical_root));
        assert!(resolved.ends_with(Path::new("assets/col-1/item-1")));
    }

    #[test]
    fn ensure_within_dir_accepts_nested_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("assets").join("col-1");
        std::fs::create_dir_all(&nested).expect("create nested dirs");
        let file_path = nested.join("asset.png");
        std::fs::write(&file_path, b"data").expect("write file");

        assert!(ensure_within_dir(&file_path, dir.path()).is_ok());
        // A directory that does not exist yet is still in scope.
        assert!(ensure_within_dir(nested.join("item-9"), dir.path()).is_ok());
    }

    #[test]
    fn ensure_within_dir_rejects_outside_paths() {
        let root = tempfile::tempdir().expect("tempdir root");
        let other = tempfile::tempdir().expect("tempdir other");
        let outside_file = other.path().join("outside.png");
        std::fs::write(&outside_file, b"data").expect("write file");

        assert!(ensure_within_dir(&outside_file, root.path()).is_err());
    }

    #[test]
    fn ensure_within_dir_rejects_traversal() {
        let root = tempfile::tempdir().expect("tempdir root");
        let nested = root.path().join("assets");
        std::fs::create_dir_all(&nested).expect("create nested dir");

        // Resolvable `..` components escape the root — caught by the scope check.
        let escape = nested.join("..").join("..");
        assert!(ensure_within_dir(&escape, root.path()).is_err());

        // `..` after a missing component cannot be resolved against the
        // filesystem — refused outright as a traversal remnant.
        let missing_escape = nested.join("ghost").join("phantom").join("..").join("evil");
        assert!(ensure_within_dir(&missing_escape, root.path()).is_err());
    }

    // ----------------------------------------------------------------------
    // resolve_asset_path
    // ----------------------------------------------------------------------

    fn data_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn resolve_asset_path_joins_a_relative_key_under_the_data_dir() {
        let dir = data_dir();
        let resolved = resolve_asset_path("assets/col-1/item-1/photo.jpg", dir.path());
        assert_eq!(
            resolved,
            dir.path()
                .join("assets")
                .join("col-1")
                .join("item-1")
                .join("photo.jpg")
        );
    }

    #[test]
    fn resolve_asset_path_normalizes_backslashes_in_a_relative_key() {
        let dir = data_dir();
        let resolved = resolve_asset_path(r"assets\col-1\item-1\photo.jpg", dir.path());
        assert_eq!(
            resolved,
            dir.path()
                .join("assets")
                .join("col-1")
                .join("item-1")
                .join("photo.jpg")
        );
    }

    #[test]
    fn resolve_asset_path_returns_an_absolute_path_unchanged() {
        let dir = data_dir();
        let absolute = dir.path().join("assets").join("photo.jpg");
        let resolved = resolve_asset_path(&absolute.to_string_lossy(), dir.path());
        assert_eq!(resolved, absolute);
    }

    #[test]
    fn resolve_asset_path_returns_a_foreign_absolute_path_unchanged() {
        let dir = data_dir();
        let foreign = if cfg!(windows) {
            r"D:\elsewhere\photo.jpg"
        } else {
            "/elsewhere/photo.jpg"
        };
        let resolved = resolve_asset_path(foreign, dir.path());
        assert_eq!(resolved, PathBuf::from(foreign));
    }

    #[test]
    fn derive_rel_path_and_resolve_asset_path_round_trip() {
        let dir = data_dir();
        let absolute = dir.path().join("assets").join("col-1").join("photo.jpg");
        let relative = derive_rel_path(&absolute.to_string_lossy(), dir.path()).expect("derive");
        assert_eq!(relative, "assets/col-1/photo.jpg");
        assert_eq!(resolve_asset_path(&relative, dir.path()), absolute);
    }
}
