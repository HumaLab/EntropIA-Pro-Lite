use std::path::{Path, PathBuf};

pub fn current_runtime_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Where the managed Python runtimes live.
///
/// Normally `<cache>/runtime`. Inside an MSIX package Windows redirects every
/// write under AppData to `...\Packages\<family>\LocalCache\Local\...`, which
/// adds about 55 characters. Python then cannot import the deepest Paddle
/// modules: the full path passes 260 characters, the limit while long paths
/// are disabled (the Windows default). There, setup remembers a short root
/// outside AppData, which Windows does not redirect.
pub fn runtime_root(app_data_dir: &Path) -> PathBuf {
    remembered_runtime_root().unwrap_or_else(|| app_data_dir.join("runtime"))
}

/// The short runtime root to use when the cache directory is redirected
/// (`real` differs from `nominal`), or `None` to keep `<cache>/runtime`.
/// An isolated dev profile keeps its own folder under the short root, so it
/// never shares a runtime with the real app.
pub fn short_runtime_root_for(
    nominal_cache: &Path,
    real_cache: &Path,
    home: Option<&Path>,
    dev_profile: Option<&str>,
) -> Option<PathBuf> {
    if nominal_cache == real_cache {
        return None;
    }
    let root = home?.join(".entropia");
    Some(match dev_profile {
        Some(name) => root
            .join(crate::dev_profile::PROFILES_DIR)
            .join(name)
            .join("runtime"),
        None => root.join("runtime"),
    })
}

static REMEMBERED_RUNTIME_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Record the short runtime root. Called once, during setup, and only when the
/// cache directory is redirected.
pub fn remember_runtime_root(root: PathBuf) {
    let _ = REMEMBERED_RUNTIME_ROOT.set(root);
}

fn remembered_runtime_root() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(root) = TEST_RUNTIME_ROOT.with(|root| root.borrow().clone()) {
        return Some(root);
    }
    REMEMBERED_RUNTIME_ROOT.get().cloned()
}

#[cfg(test)]
thread_local! {
    static TEST_RUNTIME_ROOT: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Give the calling test thread a runtime root without touching the
/// process-wide value.
#[cfg(test)]
pub fn set_test_runtime_root(root: Option<PathBuf>) {
    TEST_RUNTIME_ROOT.with(|cell| *cell.borrow_mut() = root);
}

pub fn managed_pack_dir(app_data_dir: &Path, pack_version: &str) -> PathBuf {
    runtime_root(app_data_dir).join(pack_version)
}

pub fn managed_venv_dir(managed_root: &Path) -> PathBuf {
    managed_root.join("venv").join("entropia-env")
}

pub fn managed_venv_python_path(managed_root: &Path) -> PathBuf {
    let venv_dir = managed_venv_dir(managed_root);
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

pub fn managed_wheelhouse_dir(managed_root: &Path) -> PathBuf {
    managed_root.join("wheelhouse")
}

pub fn managed_scripts_dir(managed_root: &Path) -> PathBuf {
    managed_root.join("scripts")
}

pub fn managed_script_path(managed_root: &Path, script_name: &str) -> PathBuf {
    managed_scripts_dir(managed_root).join(script_name)
}

pub fn managed_hf_cache_dir(managed_root: &Path) -> PathBuf {
    managed_root.join("caches").join("hf")
}

pub fn managed_paddlex_cache_dir(managed_root: &Path) -> PathBuf {
    managed_root.join("caches").join("paddlex")
}

pub fn managed_resource_path(managed_root: &Path, relative_path: &str) -> PathBuf {
    managed_root.join("resources").join(relative_path)
}

pub fn managed_entry_path(managed_root: &Path, relpath: &str) -> PathBuf {
    managed_root.join(relpath)
}

pub fn staging_pack_dir(app_data_dir: &Path, pack_version: &str) -> PathBuf {
    runtime_root(app_data_dir).join(format!(".{pack_version}.staging"))
}

pub fn stage_marker_path(app_data_dir: &Path, pack_version: &str) -> PathBuf {
    runtime_root(app_data_dir).join(format!(".{pack_version}.stage.json"))
}

pub fn ensure_executable_bit(_path: &Path, _executable: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        if _executable {
            use std::os::unix::fs::PermissionsExt;

            let metadata = std::fs::metadata(_path).map_err(|error| {
                format!("Failed to read metadata for {}: {error}", _path.display())
            })?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(permissions.mode() | 0o755);
            std::fs::set_permissions(_path, permissions).map_err(|error| {
                format!(
                    "Failed to set executable bit for {}: {error}",
                    _path.display()
                )
            })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use tempfile::tempdir;

    #[test]
    fn computes_stable_runtime_layout_paths() {
        let app_data = PathBuf::from("/tmp/entropia-data");
        let managed_root = managed_pack_dir(&app_data, "2026.05.0");

        assert_eq!(runtime_root(&app_data), app_data.join("runtime"));
        assert_eq!(managed_root, app_data.join("runtime").join("2026.05.0"));
        assert_eq!(
            stage_marker_path(&app_data, "2026.05.0"),
            app_data.join("runtime").join(".2026.05.0.stage.json")
        );
        assert_eq!(
            managed_venv_dir(&managed_root),
            managed_root.join("venv").join("entropia-env")
        );
        assert_eq!(
            managed_wheelhouse_dir(&managed_root),
            managed_root.join("wheelhouse")
        );
    }

    #[test]
    fn a_redirected_cache_puts_the_runtime_in_the_short_home_folder() {
        let home = PathBuf::from(r"C:\Users\someone");
        let nominal = home
            .join("AppData")
            .join("Local")
            .join("com.entropia.shared");
        let real = home
            .join("AppData")
            .join("Local")
            .join("Packages")
            .join("CONICET.EntropIAPro_b16na7gwepwme")
            .join("LocalCache")
            .join("Local")
            .join("com.entropia.shared");

        assert_eq!(
            short_runtime_root_for(&nominal, &real, Some(&home), None),
            Some(home.join(".entropia").join("runtime"))
        );
    }

    #[test]
    fn a_dev_profile_gets_its_own_short_runtime_root() {
        let home = PathBuf::from(r"C:\Users\someone");
        let nominal = PathBuf::from(r"C:\nominal\com.entropia.shared\dev-profiles\navegador");
        let real = PathBuf::from(r"C:\real\com.entropia.shared\dev-profiles\navegador");

        let profile = short_runtime_root_for(&nominal, &real, Some(&home), Some("navegador"));
        let shared = short_runtime_root_for(&nominal, &real, Some(&home), None);

        assert_eq!(
            profile,
            Some(
                home.join(".entropia")
                    .join("dev-profiles")
                    .join("navegador")
                    .join("runtime")
            )
        );
        assert_ne!(profile, shared);
    }

    #[test]
    fn a_direct_install_keeps_the_runtime_beside_the_cache() {
        let home = PathBuf::from(r"C:\Users\someone");
        let cache = home
            .join("AppData")
            .join("Local")
            .join("com.entropia.shared");

        assert_eq!(
            short_runtime_root_for(&cache, &cache, Some(&home), None),
            None
        );
    }

    #[test]
    fn without_a_home_folder_the_runtime_stays_beside_the_cache() {
        let nominal = PathBuf::from(r"C:\nominal\com.entropia.shared");
        let real = PathBuf::from(r"C:\real\com.entropia.shared");

        assert_eq!(short_runtime_root_for(&nominal, &real, None, None), None);
    }

    #[test]
    fn a_remembered_short_root_moves_every_runtime_path() {
        let app_data = PathBuf::from("/tmp/entropia-data");
        let short = PathBuf::from("/home/someone/.entropia/runtime");

        set_test_runtime_root(Some(short.clone()));
        let root = runtime_root(&app_data);
        let pack = managed_pack_dir(&app_data, "2026.11.24-cuda129");
        let staging = staging_pack_dir(&app_data, "2026.11.24-cuda129");
        let marker = stage_marker_path(&app_data, "2026.11.24-cuda129");
        set_test_runtime_root(None);

        assert_eq!(root, short);
        assert_eq!(pack, short.join("2026.11.24-cuda129"));
        assert_eq!(staging, short.join(".2026.11.24-cuda129.staging"));
        assert_eq!(marker, short.join(".2026.11.24-cuda129.stage.json"));
        assert_eq!(runtime_root(&app_data), app_data.join("runtime"));
    }

    #[test]
    fn managed_runtime_binary_paths_follow_platform_conventions() {
        let managed_root = PathBuf::from("/tmp/entropia-data/runtime/2026.05.0");

        let venv_python = managed_venv_python_path(&managed_root);
        if cfg!(windows) {
            assert_eq!(
                venv_python.file_name().and_then(|name| name.to_str()),
                Some("python.exe")
            );
            assert_eq!(
                venv_python
                    .parent()
                    .and_then(|path| path.file_name())
                    .and_then(|name| name.to_str()),
                Some("Scripts")
            );
        } else {
            assert!(venv_python.to_string_lossy().ends_with("bin/python"));
        }
    }

    #[test]
    fn managed_runtime_cache_and_script_paths_follow_conventions() {
        let managed_root = PathBuf::from("/tmp/entropia-data/runtime/2026.05.0");

        assert_eq!(
            managed_scripts_dir(&managed_root),
            managed_root.join("scripts")
        );
        assert_eq!(
            managed_script_path(&managed_root, "transcribe.py"),
            managed_root.join("scripts").join("transcribe.py")
        );
        assert_eq!(
            managed_hf_cache_dir(&managed_root),
            managed_root.join("caches").join("hf")
        );
        assert_eq!(
            managed_paddlex_cache_dir(&managed_root),
            managed_root.join("caches").join("paddlex")
        );
        assert_eq!(
            managed_resource_path(&managed_root, "models/ocr"),
            managed_root.join("resources").join("models").join("ocr")
        );
    }

    #[test]
    fn current_platform_contains_os_and_arch() {
        let platform = current_runtime_platform();

        assert!(platform.contains(std::env::consts::OS));
        assert!(platform.contains(std::env::consts::ARCH));
    }

    #[cfg(unix)]
    #[test]
    fn ensures_executable_bit_for_marked_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("temp dir");
        let binary = dir.path().join("uv");
        fs::write(&binary, "uv").expect("write file");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o644)).expect("set perms");

        ensure_executable_bit(&binary, true).expect("set exec bit");

        let mode = fs::metadata(&binary)
            .expect("metadata")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "expected executable bit to be set");
    }
}
