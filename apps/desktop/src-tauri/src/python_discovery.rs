//! Shared Python interpreter discovery and module probing.
//!
//! All subsystems (PaddleVL, transcription) that need a
//! Python interpreter follow the same pattern: discover candidate interpreters,
//! then probe each for the required module. This module consolidates the
//! discovery step so it runs ONCE and the results are shared, reducing log noise
//! and redundant filesystem scans.
//!
//! Each subsystem still probes for its specific module, but results are cached
//! per (tag) so repeated calls for the same module skip redundant subprocess spawns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::Connection;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PYTHON_FINDER_TIMEOUT: Duration = Duration::from_secs(2);
const PYTHON_MODULE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Apply the Windows `CREATE_NO_WINDOW` flag to prevent console popups.
pub fn apply_windows_no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        _cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

fn command_output_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    label: &str,
) -> Result<std::process::Output, String> {
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("{label} failed to start: {error}"))?;
    let started_at = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("{label} failed to collect output: {error}"));
            }
            Ok(None) if started_at.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} failed while waiting: {error}"));
            }
        }
    }
}

// ── Shared candidate discovery ────────────────────────────────────────────────

/// Global cache of discovered Python candidates.
/// Discovered once on first access, shared across all subsystems.
static PYTHON_CANDIDATES: OnceLock<Vec<PathBuf>> = OnceLock::new();

fn is_verbose_python_logging_enabled() -> bool {
    std::env::var("ENTROPIA_VERBOSE_PYTHON_DISCOVERY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Discover Python interpreter candidates on the system.
///
/// Returns a list of candidate Python interpreter paths, discovered once
/// and cached for all subsequent calls. Discovery strategy:
/// 1. CONDA_PREFIX if set
/// 2. System PATH (where/which python, incl. python3 variants on Unix)
/// 3. Common Conda/Python install locations on Windows
///
/// Each candidate is verified to be an existing file. Duplicates are removed.
/// Logs a single summary line instead of per-subsystem noise.
pub fn discover_python_candidates() -> &'static Vec<PathBuf> {
    PYTHON_CANDIDATES.get_or_init(|| {
        let mut candidates = Vec::new();

        // 1. Conda environment — if CONDA_PREFIX is set, that Python is authoritative
        if let Ok(conda_prefix) = std::env::var("CONDA_PREFIX") {
            let conda_python = if cfg!(windows) {
                PathBuf::from(&conda_prefix).join("python.exe")
            } else {
                PathBuf::from(&conda_prefix).join("bin").join("python")
            };
            candidates.push(conda_python);
        }

        // 2. Discover Python executables on PATH via `where` (Windows) / `which` (Unix)
        let finder_cmd = if cfg!(windows) { "where" } else { "which" };
        let candidate_names: &[&str] = if cfg!(windows) {
            &["python"]
        } else {
            &["python", "python3", "python3.12", "python3.11"]
        };

        for candidate_name in candidate_names {
            let mut find_cmd = Command::new(finder_cmd);
            apply_windows_no_window(&mut find_cmd);
            find_cmd
                .arg(candidate_name)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if let Ok(output) = command_output_with_timeout(
                find_cmd,
                PYTHON_FINDER_TIMEOUT,
                &format!("{finder_cmd} {candidate_name}"),
            ) {
                if output.status.success() {
                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                        let path = PathBuf::from(line.trim());
                        if path.is_file() && !candidates.contains(&path) {
                            candidates.push(path);
                        }
                    }
                }
            }
        }

        // 3. Scan common Conda/Python install locations not on PATH (Windows)
        if cfg!(windows) {
            if let Ok(user_profile) = std::env::var("USERPROFILE") {
                let home = PathBuf::from(&user_profile);
                if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                    let lad = PathBuf::from(&local_app_data);
                    for dir in [
                        lad.join("r-miniconda"),
                        lad.join("miniconda3"),
                        lad.join("anaconda3"),
                        home.join("miniconda3"),
                        home.join("anaconda3"),
                        home.join(".conda"),
                    ] {
                        let python_exe = dir.join("python.exe");
                        if python_exe.is_file() && !candidates.contains(&python_exe) {
                            candidates.push(python_exe);
                        }
                        // Also check envs/ subdirectories
                        let envs_dir = dir.join("envs");
                        if envs_dir.is_dir() {
                            if let Ok(entries) = std::fs::read_dir(&envs_dir) {
                                for entry in entries.flatten() {
                                    let env_python = entry.path().join("python.exe");
                                    if env_python.is_file() && !candidates.contains(&env_python) {
                                        candidates.push(env_python);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if is_verbose_python_logging_enabled() {
            eprintln!(
                "[python] Discovered {} Python candidate(s)",
                candidates.len()
            );
            for (i, c) in candidates.iter().enumerate() {
                eprintln!("[python]   [{}] {}", i + 1, c.display());
            }
        }

        candidates
    })
}

// ── Module probe result cache ────────────────────────────────────────────────

/// Cache of module probe results: tag → winning interpreter path.
/// Prevents redundant subprocess spawns if the same module is queried multiple times.
/// `None` values are also cached (module not found) to avoid re-probing.
static MODULE_PROBE_CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();

fn get_probe_cache() -> &'static Mutex<HashMap<String, Option<PathBuf>>> {
    MODULE_PROBE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn python_setting_key(cache_key: &str) -> String {
    format!("python.{cache_key}.path")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PythonRuntimeSelection {
    Managed,
    System,
}

const PYTHON_RUNTIME_SELECTION_KEY: &str = "python.runtime_selection";

fn load_python_runtime_selection(conn: &Connection) -> PythonRuntimeSelection {
    match crate::settings::get_setting(conn, PYTHON_RUNTIME_SELECTION_KEY)
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("system") => PythonRuntimeSelection::System,
        _ => PythonRuntimeSelection::Managed,
    }
}

fn resolve_managed_python_candidate(
    selection: PythonRuntimeSelection,
    hydrated_runtime_python: Option<&Path>,
    persisted_managed_python: Option<&Path>,
) -> Option<PathBuf> {
    if selection == PythonRuntimeSelection::System {
        return None;
    }

    hydrated_runtime_python
        .filter(|path| path.is_file())
        .map(Path::to_path_buf)
        .or_else(|| {
            persisted_managed_python
                .filter(|path| path.is_file())
                .map(Path::to_path_buf)
        })
}

fn hydrated_runtime_python_path(settings_db_path: Option<&Path>) -> Option<PathBuf> {
    let db_path = settings_db_path?;
    let app_data_dir = db_path.parent()?;
    let manager = crate::runtime::RuntimeManager::new();
    let managed_root = manager.discover_hydrated_runtime_root_for_tests(app_data_dir)?;
    let manifest = crate::runtime::manifest::RuntimeManifest::load_from_path(
        &managed_root.join("manifest.json"),
    )
    .ok()?;
    let status =
        manager.inspect_hydrated_runtime_for_tests(app_data_dir, &managed_root, &manifest)?;

    if status.state != crate::runtime::status::RuntimeState::Healthy {
        return None;
    }

    Some(crate::runtime::managed_venv_python_path(&managed_root))
}

fn load_managed_venv_python(probe_code: &str, settings_db_path: Option<&Path>) -> Option<PathBuf> {
    let db_path = settings_db_path?;
    let conn = Connection::open(db_path).ok()?;
    let selection = load_python_runtime_selection(&conn);
    let persisted_managed =
        crate::settings::get_setting(&conn, "deps_venv_python_path").map(PathBuf::from);
    let path = resolve_managed_python_candidate(
        selection,
        hydrated_runtime_python_path(settings_db_path).as_deref(),
        persisted_managed.as_deref(),
    )?;

    if path.is_file() && probe_python_module(&path, probe_code) {
        return Some(path);
    }

    None
}

fn load_persisted_python(
    cache_key: &str,
    probe_code: &str,
    settings_db_path: Option<&Path>,
) -> Option<PathBuf> {
    let db_path = settings_db_path?;
    let conn = Connection::open(db_path).ok()?;
    if load_python_runtime_selection(&conn) != PythonRuntimeSelection::System {
        return None;
    }
    let setting_key = python_setting_key(cache_key);
    let persisted = crate::settings::get_setting(&conn, &setting_key)?;
    let path = PathBuf::from(&persisted);

    if path.is_file() && probe_python_module(&path, probe_code) {
        return Some(path);
    }

    let _ = crate::settings::delete_setting(&conn, &setting_key);
    None
}

fn persist_python_hit(cache_key: &str, path: &Path, settings_db_path: Option<&Path>) {
    let Some(db_path) = settings_db_path else {
        return;
    };
    let Ok(conn) = Connection::open(db_path) else {
        return;
    };

    let setting_key = python_setting_key(cache_key);
    let value = path.to_string_lossy();
    let _ = crate::settings::set_setting(&conn, &setting_key, &value);
}

// ── Module probing ────────────────────────────────────────────────────────────

/// Probe a single Python interpreter for a specific import check.
///
/// Spawns `python -c "<probe_code>"` and checks if stdout contains "ok".
/// Used by subsystem-specific `which_python_for_module` functions.
pub fn probe_python_module(python_path: &Path, probe_code: &str) -> bool {
    let mut cmd = Command::new(python_path);
    apply_windows_no_window(&mut cmd);
    cmd.args(["-c", probe_code])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match command_output_with_timeout(
        cmd,
        PYTHON_MODULE_PROBE_TIMEOUT,
        &format!("{} -c <probe>", python_path.display()),
    ) {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim() == "ok"
        }
        _ => false,
    }
}

/// Find a Python interpreter that can import a specific module.
///
/// Uses the shared candidate cache (discovered once) and probes each candidate
/// until one succeeds. Logs one summary line per subsystem.
///
/// Arguments:
/// - `tag`: Subsystem tag for logging (e.g., "transcription", "embeddings")
/// - `module_name`: Display name of the module for logging
/// - `probe_code`: Python code to verify import (e.g., `"import faster_whisper; print('ok')"`)
pub fn which_python_for_module(
    tag: &str,
    cache_key: &str,
    module_name: &str,
    probe_code: &str,
    settings_db_path: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = load_managed_venv_python(probe_code, settings_db_path) {
        eprintln!(
            "[{tag}] Python resolver hit ({cache_key}, source=managed_venv): {}",
            path.display()
        );
        if let Ok(mut cache) = get_probe_cache().lock() {
            cache.insert(cache_key.to_string(), Some(path.clone()));
        }
        persist_python_hit(cache_key, &path, settings_db_path);
        return Some(path);
    }

    // Check probe cache — if we already resolved this capability, return the cached result.
    if let Ok(cache) = get_probe_cache().lock() {
        if let Some(cached) = cache.get(cache_key) {
            if let Some(path) = cached {
                eprintln!(
                    "[{tag}] Python resolver hit ({cache_key}, source=memory_cache): {}",
                    path.display()
                );
                return Some(path.clone());
            }
            eprintln!(
                "[{tag}] Python resolver hit ({cache_key}, source=memory_cache): not available"
            );
            return None;
        }
    }

    if let Some(path) = load_persisted_python(cache_key, probe_code, settings_db_path) {
        eprintln!(
            "[{tag}] Python resolver hit ({cache_key}, source=persisted_cache): {}",
            path.display()
        );
        if let Ok(mut cache) = get_probe_cache().lock() {
            cache.insert(cache_key.to_string(), Some(path.clone()));
        }
        return Some(path);
    }

    let known_good = collect_known_good_pythons();
    if !known_good.is_empty() {
        eprintln!(
            "[{tag}] Fast-path: trying {} previously-validated Python(s) before full scan for {module_name}",
            known_good.len()
        );
        for python in &known_good {
            let probe_start = std::time::Instant::now();
            if probe_python_module(python, probe_code) {
                eprintln!(
                    "[{tag}] ✅ Found Python with {module_name} via fast-path (source=known_good, {}ms): {}",
                    probe_start.elapsed().as_millis(),
                    python.display()
                );
                if let Ok(mut cache) = get_probe_cache().lock() {
                    cache.insert(cache_key.to_string(), Some(python.clone()));
                }
                persist_python_hit(cache_key, python, settings_db_path);
                return Some(python.clone());
            }
        }
        eprintln!("[{tag}] Fast-path: no previously-validated Python had {module_name}, falling back to full scan");
    }

    let known_good_keys: std::collections::HashSet<String> = known_good
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();

    let candidates: Vec<PathBuf> = discover_python_candidates()
        .iter()
        .filter(|candidate| !known_good_keys.contains(&candidate.to_string_lossy().into_owned()))
        .cloned()
        .collect();

    let candidate_count = candidates.len();
    eprintln!(
        "[{tag}] Probing {n} candidate(s) for {module_name}",
        n = candidate_count
    );
    let mut failed_probes = 0usize;

    for candidate in candidates {
        let probe_start = std::time::Instant::now();
        let import_ok = probe_python_module(&candidate, probe_code);

        if import_ok {
            eprintln!(
                "[{tag}] ✅ Found Python with {module_name} after {} failed probe(s) (source=full_scan, {}ms): {}",
                failed_probes,
                probe_start.elapsed().as_millis(),
                candidate.display()
            );
            // Cache the hit
            if let Ok(mut cache) = get_probe_cache().lock() {
                cache.insert(cache_key.to_string(), Some(candidate.clone()));
            }
            persist_python_hit(cache_key, &candidate, settings_db_path);
            return Some(candidate.clone());
        }

        failed_probes += 1;
        if is_verbose_python_logging_enabled() {
            eprintln!(
                "[{tag}]   ❌ {} ({}ms): {module_name} not importable",
                candidate.display(),
                probe_start.elapsed().as_millis()
            );
        }
    }

    eprintln!(
        "[{tag}] WARNING: No Python with {module_name} found among {} candidate(s)",
        candidate_count
    );
    // Cache the miss
    if let Ok(mut cache) = get_probe_cache().lock() {
        cache.insert(cache_key.to_string(), None);
    }
    None
}

/// Clear the module probe cache so the next `which_python_for_module` call re-probes.
///
/// Called by `deps_reset` after the managed venv is deleted, so subsystems
/// (OCR, embeddings, transcription, NER) don't keep using a now-gone interpreter.
pub fn invalidate_probe_cache() {
    if let Ok(mut cache) = get_probe_cache().lock() {
        cache.clear();
        eprintln!("[python_discovery] Probe cache invalidated");
    }
}

/// Clear a single module probe cache entry so the next call for that key re-probes.
///
/// Used by runtime retry logic: when a previously-failed init is retried after
/// dependencies are installed, we
/// must drop the cached `None` miss so the probe actually runs again.
pub fn invalidate_probe_cache_entry(cache_key: &str) {
    if let Ok(mut cache) = get_probe_cache().lock() {
        if cache.remove(cache_key).is_some() {
            eprintln!("[python_discovery] Probe cache entry '{cache_key}' invalidated");
        }
    }
}

/// Collect Python interpreters that were previously validated for ANY module.
///
/// Returns deduplicated paths from the probe cache, ordered by the normal
/// discovery preference when possible. Used as a fast-path so that modules
/// probed later can try known-good Pythons before re-scanning everything.
fn collect_known_good_pythons() -> Vec<PathBuf> {
    let cache = match get_probe_cache().lock() {
        Ok(cache) => cache,
        Err(_) => return Vec::new(),
    };
    let mut cached = std::collections::HashSet::new();
    for value in cache.values() {
        if let Some(path) = value {
            cached.insert(path.to_string_lossy().into_owned());
        }
    }

    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for candidate in discover_python_candidates() {
        let key = candidate.to_string_lossy().into_owned();
        if cached.contains(&key) && seen.insert(key) {
            paths.push(candidate.clone());
        }
    }

    for value in cache.values() {
        if let Some(path) = value {
            let key = path.to_string_lossy().into_owned();
            if seen.insert(key) {
                paths.push(path.clone());
            }
        }
    }

    paths
}

/// Find a Python interpreter that can import a specific module, with candidate scoring.
///
/// Like [`which_python_for_module`], but sorts candidates by a scoring function
/// before probing. Higher scores are probed first. This is useful for subsystems
/// that prefer dedicated environments (e.g., PaddleVL prefers `ppocrvl-py312` envs).
///
/// **Fast-path**: before scanning all candidates, this function tries Python
/// interpreters that were already validated for OTHER modules (from the probe cache).
/// On typical setups where the same Conda Python has ALL required packages, this
/// avoids 10+ seconds of redundant probing.
///
/// Arguments:
/// - `tag`: Subsystem tag for logging
/// - `module_name`: Display name of the module for logging
/// - `probe_code`: Python code to verify import
/// - `scorer`: Function that assigns a score to each candidate path (higher = better)
pub fn which_python_for_module_scored(
    tag: &str,
    cache_key: &str,
    module_name: &str,
    probe_code: &str,
    settings_db_path: Option<&Path>,
    scorer: &dyn Fn(&std::path::Path) -> i32,
) -> Option<PathBuf> {
    if let Some(path) = load_managed_venv_python(probe_code, settings_db_path) {
        eprintln!(
            "[{tag}] Python resolver hit ({cache_key}, source=managed_venv): {}",
            path.display()
        );
        if let Ok(mut cache) = get_probe_cache().lock() {
            cache.insert(cache_key.to_string(), Some(path.clone()));
        }
        persist_python_hit(cache_key, &path, settings_db_path);
        return Some(path);
    }

    // Check probe cache — if we already resolved this capability, return the cached result.
    if let Ok(cache) = get_probe_cache().lock() {
        if let Some(cached) = cache.get(cache_key) {
            if let Some(path) = cached {
                eprintln!(
                    "[{tag}] Python resolver hit ({cache_key}, source=memory_cache): {}",
                    path.display()
                );
                return Some(path.clone());
            }
            eprintln!(
                "[{tag}] Python resolver hit ({cache_key}, source=memory_cache): not available"
            );
            return None;
        }
    }

    if let Some(path) = load_persisted_python(cache_key, probe_code, settings_db_path) {
        eprintln!(
            "[{tag}] Python resolver hit ({cache_key}, source=persisted_cache): {}",
            path.display()
        );
        if let Ok(mut cache) = get_probe_cache().lock() {
            cache.insert(cache_key.to_string(), Some(path.clone()));
        }
        return Some(path);
    }

    // Fast-path: try Python interpreters that were already validated for other
    // modules before scanning all candidates. On a typical setup where one
    // Conda Python has all packages, this avoids ~10s of redundant probing.
    let known_good = collect_known_good_pythons();
    if !known_good.is_empty() {
        eprintln!(
            "[{tag}] Fast-path: trying {} previously-validated Python(s) before full scan for {module_name}",
            known_good.len()
        );
        for python in &known_good {
            let probe_start = std::time::Instant::now();
            if probe_python_module(python, probe_code) {
                eprintln!(
                    "[{tag}] ✅ Found Python with {module_name} via fast-path (source=known_good, {}ms): {}",
                    probe_start.elapsed().as_millis(),
                    python.display()
                );
                // Cache the hit
                if let Ok(mut cache) = get_probe_cache().lock() {
                    cache.insert(cache_key.to_string(), Some(python.clone()));
                }
                persist_python_hit(cache_key, python, settings_db_path);
                return Some(python.clone());
            }
        }
        eprintln!("[{tag}] Fast-path: no previously-validated Python had {module_name}, falling back to full scan");
    }

    let known_good_keys: std::collections::HashSet<String> = known_good
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();

    let mut candidates: Vec<PathBuf> = discover_python_candidates()
        .iter()
        .filter(|candidate| !known_good_keys.contains(&candidate.to_string_lossy().into_owned()))
        .cloned()
        .collect();

    // Sort candidates by score (descending) — dedicated envs first
    candidates.sort_by_key(|c| -scorer(c));

    eprintln!(
        "[{tag}] Probing {} candidate(s) for {module_name} (scored, dedicated envs first)",
        candidates.len()
    );
    let mut failed_probes = 0usize;

    for candidate in &candidates {
        let probe_start = std::time::Instant::now();
        let import_ok = probe_python_module(candidate, probe_code);

        if import_ok {
            eprintln!(
                "[{tag}] ✅ Found Python with {module_name} after {} failed probe(s) (source=full_scan, {}ms): {}",
                failed_probes,
                probe_start.elapsed().as_millis(),
                candidate.display()
            );
            // Cache the hit
            if let Ok(mut cache) = get_probe_cache().lock() {
                cache.insert(cache_key.to_string(), Some(candidate.clone()));
            }
            persist_python_hit(cache_key, candidate, settings_db_path);
            return Some(candidate.clone());
        }

        failed_probes += 1;
        if is_verbose_python_logging_enabled() {
            eprintln!(
                "[{tag}]   ❌ {} ({}ms): {module_name} not importable",
                candidate.display(),
                probe_start.elapsed().as_millis()
            );
        }
    }

    eprintln!(
        "[{tag}] WARNING: No Python with {module_name} found among {} candidate(s)",
        candidates.len()
    );
    // Cache the miss
    if let Ok(mut cache) = get_probe_cache().lock() {
        cache.insert(cache_key.to_string(), None);
    }
    None
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use tempfile::tempdir;

    fn write_setting_db(path: &Path, entries: &[(&str, &str)]) {
        let conn = rusqlite::Connection::open(path).expect("open db");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("create app_settings");
        for (key, value) in entries {
            conn.execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .expect("insert setting");
        }
    }

    #[test]
    fn managed_runtime_selection_prefers_hydrated_runtime_path() {
        let dir = tempdir().expect("temp dir");
        let hydrated = dir.path().join("runtime-python");
        let persisted = dir.path().join("persisted-python");
        std::fs::write(&hydrated, b"runtime").expect("write hydrated python");
        std::fs::write(&persisted, b"persisted").expect("write persisted python");

        let resolved = resolve_managed_python_candidate(
            PythonRuntimeSelection::Managed,
            Some(&hydrated),
            Some(&persisted),
        );

        assert_eq!(resolved, Some(hydrated));
    }

    #[test]
    fn system_runtime_selection_disables_managed_python_preference() {
        let dir = tempdir().expect("temp dir");
        let hydrated = dir.path().join("runtime-python");
        std::fs::write(&hydrated, b"runtime").expect("write hydrated python");

        let resolved =
            resolve_managed_python_candidate(PythonRuntimeSelection::System, Some(&hydrated), None);

        assert_eq!(resolved, None);
    }

    #[test]
    fn hydrated_runtime_python_path_discovers_managed_venv_from_app_data() {
        let dir = tempdir().expect("temp dir");
        let db_path = dir.path().join("entropia.sqlite");
        write_setting_db(&db_path, &[]);
        let managed_root = dir.path().join("runtime").join("2026.05.0");
        let venv_python = crate::runtime::managed_venv_python_path(&managed_root);
        let python_relpath = if cfg!(windows) {
            "python/python.exe"
        } else {
            "python/bin/python3"
        };
        let uv_relpath = if cfg!(windows) {
            "uv/uv.exe"
        } else {
            "uv/bin/uv"
        };
        let python_path = managed_root.join(python_relpath);
        let uv_path = managed_root.join(uv_relpath);
        if let Some(parent) = python_path.parent() {
            std::fs::create_dir_all(parent).expect("create python parent");
        }
        if let Some(parent) = uv_path.parent() {
            std::fs::create_dir_all(parent).expect("create uv parent");
        }
        if let Some(parent) = venv_python.parent() {
            std::fs::create_dir_all(parent).expect("create venv parent");
        }
        std::fs::write(&python_path, b"python").expect("write python");
        std::fs::write(&uv_path, b"uv").expect("write uv");
        std::fs::write(&venv_python, b"venv-python").expect("write venv python");
        let python_sha = format!("{:x}", sha2::Sha256::digest(b"python"));
        let uv_sha = format!("{:x}", sha2::Sha256::digest(b"uv"));
        std::fs::write(
            managed_root.join("manifest.json"),
            serde_json::to_vec_pretty(&crate::runtime::manifest::RuntimeManifest {
                pack_version: "2026.05.0".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                platform: crate::runtime::paths::current_runtime_platform(),
                payload_profile: "release".to_string(),
                release_injection_required: false,
                external_artifacts_required: vec![],
                python_relpath: python_relpath.to_string(),
                uv_relpath: uv_relpath.to_string(),
                python_files: vec![crate::runtime::manifest::ManifestEntry {
                    path: python_relpath.to_string(),
                    sha256: python_sha,
                    size: 6,
                    executable: !cfg!(windows),
                }],
                uv_files: vec![crate::runtime::manifest::ManifestEntry {
                    path: uv_relpath.to_string(),
                    sha256: uv_sha,
                    size: 2,
                    executable: true,
                }],
                script_files: vec![],
                wheelhouse: vec![],
                caches: vec![],
                native_assets: vec![],
            })
            .expect("serialize manifest"),
        )
        .expect("write manifest");

        let discovered = hydrated_runtime_python_path(Some(&db_path));

        assert_eq!(discovered, Some(venv_python));
    }

    #[test]
    fn discover_candidates_returns_non_empty_on_any_system() {
        // On any dev system with Python installed, at least one candidate should exist.
        // On CI without Python, this might return empty — that's OK for a smoke test.
        let candidates = discover_python_candidates();
        // Just verify it doesn't panic and returns a valid Vec
        assert!(
            candidates.len() <= 50,
            "Should not have more than 50 candidates"
        );
    }

    #[test]
    fn probe_python_module_returns_false_for_nonsense() {
        // Probing a nonsense module should return false without panicking
        let candidates = discover_python_candidates();
        if let Some(first) = candidates.first() {
            let result =
                probe_python_module(first, "import __nonexistent_module_xyz__; print('ok')");
            assert!(!result, "Nonsense module should not be importable");
        }
        // If no candidates, the test is a no-op
    }

    #[test]
    fn invalidate_probe_cache_entry_clears_specific_key() {
        let key = "test_invalidate_entry";
        // Prime cache with a miss
        {
            let mut cache = get_probe_cache().lock().unwrap();
            cache.insert(key.to_string(), None);
            assert!(cache.contains_key(key));
        }

        invalidate_probe_cache_entry(key);

        {
            let cache = get_probe_cache().lock().unwrap();
            assert!(!cache.contains_key(key));
        }
    }

    #[test]
    fn unix_candidate_names_include_versioned_python_binaries() {
        if cfg!(windows) {
            return;
        }

        let candidate_names = ["python", "python3", "python3.12", "python3.11"];
        assert!(candidate_names.contains(&"python3.11"));
        assert!(candidate_names.contains(&"python3.12"));
    }
}
