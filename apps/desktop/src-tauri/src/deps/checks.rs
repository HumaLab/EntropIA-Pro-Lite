//! Probe/check system for managed Python dependencies.
//!
//! Each dependency has a short Python one-liner (`probe_code`) that prints
//! `"ok"` when the dependency is importable. This module runs those probes
//! asynchronously and maps the results to `DependencyStatus` values.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::task;
use tokio::time::timeout;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use super::{DependencyId, DependencyStatus};
use crate::deps::registry::all_deps;
use crate::runtime::status::{RuntimeState, RuntimeStatus};

const PROBE_TIMEOUT_SECS: u64 = 45;
const GLOBAL_PROBE_TIMEOUT_SECS: u64 = 90;
const GPU_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

const PROBE_PADDLEPADDLE_CUDA: &str = "import paddle; assert paddle.device.is_compiled_with_cuda(), 'PaddlePaddle CPU wheel installed on NVIDIA hardware'; print('ok')";
const PROBE_PADDLE_VL: &str = "from paddleocr import PaddleOCRVL; print('ok')";
const PROBE_FASTER_WHISPER: &str = "import faster_whisper; print('ok')";
const RUNTIME_PYTHON_KEYS: &[&str] = &["python.paddle_vl.path", "python.faster_whisper.path"];

static LOGGED_RUNTIME_FALLBACK: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn runtime_fallback_log_cache() -> &'static Mutex<Option<PathBuf>> {
    LOGGED_RUNTIME_FALLBACK.get_or_init(|| Mutex::new(None))
}

pub fn invalidate_resolved_probe_python_log() {
    if let Ok(mut cache) = runtime_fallback_log_cache().lock() {
        *cache = None;
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProbePythonSettings {
    managed_path: Option<PathBuf>,
    runtime_candidates: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbePythonMode {
    DependencyManager,
    RuntimeFallback,
}

// ---------------------------------------------------------------------------
// Per-dependency probe
// ---------------------------------------------------------------------------

/// Probe a single dependency by running its `probe_code` with `python_path`.
///
/// - Spawns `python_path -c "<probe_code>"` with a 10 s per-probe timeout.
/// - stdout contains `"ok"` → `Installed { version: None }`
/// - Non-zero exit, timeout, or spawn error → `Missing`
pub async fn probe_one(
    dep: &crate::deps::registry::DependencySpec,
    python_path: &Path,
) -> DependencyStatus {
    let mut cmd = Command::new(python_path);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.args(["-c", probe_code_for(dep)])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let probe_result = timeout(Duration::from_secs(PROBE_TIMEOUT_SECS), cmd.output()).await;

    match probe_result {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim().contains("ok") {
                DependencyStatus::Installed { version: None }
            } else {
                DependencyStatus::Missing
            }
        }
        // Non-zero exit or I/O error from spawn
        Ok(_) => DependencyStatus::Missing,
        // Timeout
        Err(_) => {
            eprintln!(
                "[deps/checks] probe timed out for '{}' using {}",
                dep.display_name,
                python_path.display()
            );
            DependencyStatus::Missing
        }
    }
}

// ---------------------------------------------------------------------------
// Probe all dependencies
// ---------------------------------------------------------------------------

/// Probe all registered dependencies concurrently and return a status map.
///
/// - Runs all probes in parallel using `tokio::task::JoinSet`.
/// - Applies a 15 s global timeout over the entire set.
/// - Dependencies that haven't finished when the global timeout fires are
///   marked `Unknown` (not yet checked).
pub async fn probe_all(python_path: &Path) -> HashMap<DependencyId, DependencyStatus> {
    let deps = all_deps();
    let python_path = python_path.to_path_buf();

    // Spawn one task per dependency.
    let mut join_set: tokio::task::JoinSet<(DependencyId, DependencyStatus)> =
        tokio::task::JoinSet::new();

    for dep in deps {
        // SAFETY: DependencySpec is &'static so borrowing id/probe_code is fine.
        let id = dep.id.clone();
        let probe_code = probe_code_for(dep);
        let display_name = dep.display_name;
        let python = python_path.clone();

        join_set.spawn(async move {
            let mut cmd = Command::new(&python);
            #[cfg(windows)]
            {
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.args(["-c", probe_code])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            cmd.kill_on_drop(true);

            let result = timeout(Duration::from_secs(PROBE_TIMEOUT_SECS), cmd.output()).await;
            let status = match result {
                Ok(Ok(output)) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.trim().contains("ok") {
                        DependencyStatus::Installed { version: None }
                    } else {
                        DependencyStatus::Missing
                    }
                }
                Ok(_) => DependencyStatus::Missing,
                Err(_) => {
                    eprintln!("[deps/checks] probe timed out for '{display_name}'");
                    DependencyStatus::Missing
                }
            };
            (id, status)
        });
    }

    // Collect results with a 15 s global timeout.
    let mut results: HashMap<DependencyId, DependencyStatus> = HashMap::new();

    let collect_all = async {
        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((id, status)) => {
                    results.insert(id, status);
                }
                Err(e) => {
                    eprintln!("[deps/checks] probe task panicked: {e}");
                }
            }
        }
    };

    match timeout(Duration::from_secs(GLOBAL_PROBE_TIMEOUT_SECS), collect_all).await {
        Ok(()) => {}
        Err(_) => {
            eprintln!(
                "[deps/checks] global probe timeout ({GLOBAL_PROBE_TIMEOUT_SECS} s) — marking remaining deps Unknown"
            );
            // Abort any tasks still running.
            join_set.abort_all();
            // Any dep not yet inserted stays Unknown (default for missing keys).
        }
    }

    // Ensure every registered dep has an entry — default to Unknown if we
    // didn't get a result (e.g. was still in flight when timeout hit).
    for dep in all_deps() {
        results
            .entry(dep.id.clone())
            .or_insert(DependencyStatus::Unknown);
    }

    results
}

fn probe_code_for(dep: &crate::deps::registry::DependencySpec) -> &'static str {
    if dep.id == DependencyId::PaddlePaddle && detect_nvidia_gpu_hardware() {
        return PROBE_PADDLEPADDLE_CUDA;
    }

    dep.probe_code
}

fn detect_nvidia_gpu_hardware() -> bool {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.arg("-L");
    match std_command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi -L") {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.trim().is_empty() && stdout.contains("GPU") {
                return true;
            }
        }
        _ => {}
    }

    detect_nvidia_gpu_from_system_inventory()
}

fn std_command_output_with_timeout(
    mut cmd: std::process::Command,
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

#[cfg(target_os = "linux")]
fn detect_nvidia_gpu_from_system_inventory() -> bool {
    if std::fs::read_dir("/proc/driver/nvidia/gpus")
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
    {
        return true;
    }

    let Ok(entries) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return false;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let vendor = std::fs::read_to_string(path.join("vendor")).unwrap_or_default();
        if vendor.trim() != "0x10de" {
            continue;
        }

        let class = std::fs::read_to_string(path.join("class")).unwrap_or_default();
        if class.trim().starts_with("0x03") {
            return true;
        }
    }

    false
}

#[cfg(windows)]
fn detect_nvidia_gpu_from_system_inventory() -> bool {
    let candidates = [
        std::env::var_os("ProgramFiles").map(PathBuf::from),
        std::env::var_os("ProgramW6432").map(PathBuf::from),
    ];

    for base in candidates.into_iter().flatten() {
        let smi = base
            .join("NVIDIA Corporation")
            .join("NVSMI")
            .join("nvidia-smi.exe");
        if !smi.exists() {
            continue;
        }
        let mut cmd = std::process::Command::new(&smi);
        cmd.arg("-L");
        if let Ok(output) =
            std_command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi.exe -L")
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if output.status.success() && stdout.contains("GPU") {
                return true;
            }
        }
    }

    false
}

#[cfg(not(any(target_os = "linux", windows)))]
fn detect_nvidia_gpu_from_system_inventory() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Python path resolution
// ---------------------------------------------------------------------------

/// Resolve the Python interpreter path to use for probing.
///
/// Reads `deps_venv_python_path` from `app_settings` via an open rusqlite
/// connection. Runtime fallback resolution is handled separately so dependency
/// manager probes can require the managed venv while runtime features still
/// retain their system/runtime fallback behavior.
pub fn load_probe_python_settings(conn: &rusqlite::Connection) -> ProbePythonSettings {
    let managed_path = crate::settings::get_setting(conn, "deps_venv_python_path")
        .map(PathBuf::from)
        .filter(|path| path.is_file());

    ProbePythonSettings {
        managed_path,
        runtime_candidates: persisted_runtime_candidates(conn),
    }
}

pub fn resolve_probe_python(conn: &rusqlite::Connection) -> Option<PathBuf> {
    resolve_probe_python_from_settings(
        load_probe_python_settings(conn),
        ProbePythonMode::RuntimeFallback,
    )
}

pub async fn resolve_probe_python_async(
    settings: ProbePythonSettings,
    mode: ProbePythonMode,
) -> Result<Option<PathBuf>, String> {
    task::spawn_blocking(move || resolve_probe_python_from_settings(settings, mode))
        .await
        .map_err(|error| format!("Dependency Python resolution task failed: {error}"))
}

pub fn resolve_probe_python_from_settings(
    settings: ProbePythonSettings,
    mode: ProbePythonMode,
) -> Option<PathBuf> {
    if let Some(path) = settings.managed_path.filter(|path| path.is_file()) {
        return Some(path);
    }

    match mode {
        ProbePythonMode::DependencyManager => None,
        ProbePythonMode::RuntimeFallback => {
            resolve_runtime_python_candidates(settings.runtime_candidates)
        }
    }
}

fn resolve_runtime_python_choice(
    managed_path: Option<&Path>,
    runtime_candidates: Vec<PathBuf>,
    managed_runtime_python: Option<&Path>,
    managed_runtime_status: Option<&RuntimeStatus>,
) -> Option<PathBuf> {
    // Prefer the runtime pack's Python only when the pack is healthy. When the pack
    // is unhealthy/damaged we must NOT use its Python, but the persisted managed
    // venv (managed_path) and system candidates are independent artifacts — a damaged
    // pack does not invalidate them, so they remain valid fallbacks below.
    if runtime_status_is_healthy(managed_runtime_status) {
        if let Some(path) = managed_runtime_python.filter(|path| path.is_file()) {
            return Some(path.to_path_buf());
        }
    }

    managed_path
        .filter(|path| path.is_file())
        .map(Path::to_path_buf)
        .or_else(|| {
            if runtime_status_is_healthy(managed_runtime_status) {
                managed_runtime_python
                    .filter(|path| path.is_file())
                    .map(Path::to_path_buf)
            } else {
                None
            }
        })
        .or_else(|| resolve_runtime_python_candidates(runtime_candidates))
}

fn runtime_status_is_healthy(status: Option<&RuntimeStatus>) -> bool {
    matches!(
        status.map(|status| &status.state),
        Some(RuntimeState::Healthy)
    )
}

pub fn resolve_probe_python_with_runtime(
    settings: ProbePythonSettings,
    mode: ProbePythonMode,
    managed_runtime_python: Option<&Path>,
    managed_runtime_status: Option<&RuntimeStatus>,
) -> Option<PathBuf> {
    if let Some(path) = resolve_runtime_python_choice(
        settings.managed_path.as_deref(),
        settings.runtime_candidates.clone(),
        managed_runtime_python,
        managed_runtime_status,
    ) {
        return Some(path);
    }

    match mode {
        ProbePythonMode::DependencyManager => None,
        ProbePythonMode::RuntimeFallback => {
            resolve_runtime_python_candidates(settings.runtime_candidates)
        }
    }
}

fn resolve_runtime_python_candidates(mut candidates: Vec<PathBuf>) -> Option<PathBuf> {
    for candidate in crate::python_discovery::discover_python_candidates() {
        if candidate.is_file() && !candidates.contains(candidate) {
            candidates.push(candidate.clone());
        }
    }

    let mut best_match: Option<(PathBuf, usize)> = None;

    for candidate in candidates {
        let capabilities = probe_runtime_capabilities(&candidate);
        if !capabilities.has_paddle_vl {
            continue;
        }

        let optional_score = usize::from(capabilities.has_faster_whisper);

        match &best_match {
            Some((_, best_score)) if *best_score >= optional_score => {}
            _ => {
                best_match = Some((candidate, optional_score));
            }
        }
    }

    if let Some((path, optional_score)) = best_match {
        let should_log = runtime_fallback_log_cache()
            .lock()
            .map(|mut cache| {
                let already_logged = cache.as_ref() == Some(&path);
                if !already_logged {
                    *cache = Some(path.clone());
                }
                !already_logged
            })
            .unwrap_or(true);

        if should_log {
            eprintln!(
                "[deps/checks] Using runtime Python fallback (critical deps OK, optional score={}): {}",
                optional_score,
                path.display()
            );
        }
        return Some(path);
    }

    None
}

fn persisted_runtime_candidates(conn: &rusqlite::Connection) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for key in RUNTIME_PYTHON_KEYS {
        let Some(raw) = crate::settings::get_setting(conn, key) else {
            continue;
        };

        let path = PathBuf::from(raw);
        if path.is_file() && !candidates.contains(&path) {
            candidates.push(path);
        }
    }

    candidates
}

#[derive(Debug, Clone, Copy, Default)]
struct RuntimeCapabilities {
    has_paddle_vl: bool,
    has_faster_whisper: bool,
}

fn probe_runtime_capabilities(path: &Path) -> RuntimeCapabilities {
    RuntimeCapabilities {
        has_paddle_vl: crate::python_discovery::probe_python_module(path, PROBE_PADDLE_VL),
        has_faster_whisper: crate::python_discovery::probe_python_module(
            path,
            PROBE_FASTER_WHISPER,
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn in_memory_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory SQLite");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("create app_settings");
        conn
    }

    #[test]
    fn test_resolve_probe_python_prefers_existing_managed_path() {
        let conn = in_memory_conn();
        let current_exe = std::env::current_exe().expect("current exe path");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                "deps_venv_python_path",
                current_exe.to_string_lossy().as_ref()
            ],
        )
        .expect("insert managed python path");

        let result = resolve_probe_python(&conn);
        assert!(
            result.as_ref() == Some(&current_exe),
            "Expected managed venv path to be preferred when present"
        );
    }

    #[test]
    fn test_resolve_probe_python_with_stale_managed_path_does_not_panic() {
        let conn = in_memory_conn();
        // Insert a path that does not exist on disk
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('deps_venv_python_path', '/nonexistent/path/python.exe')",
            [],
        )
        .expect("insert setting");

        let result = resolve_probe_python(&conn);
        assert!(
            result.as_ref().map(|path| path.is_file()).unwrap_or(true),
            "A stale managed path should either fall back to a valid runtime or return None"
        );
    }

    #[test]
    fn test_persisted_runtime_candidates_ignore_missing_and_duplicate_paths() {
        let conn = in_memory_conn();
        let current_exe = std::env::current_exe().expect("current exe path");

        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                "python.faster_whisper.path",
                current_exe.to_string_lossy().as_ref()
            ],
        )
        .expect("insert paddle path");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                "python.paddle_vl.path",
                current_exe.to_string_lossy().as_ref()
            ],
        )
        .expect("insert duplicate path");

        let candidates = persisted_runtime_candidates(&conn);
        assert_eq!(candidates, vec![current_exe]);
    }

    #[test]
    fn test_load_probe_python_settings_prefers_existing_managed_path_without_runtime_candidates() {
        let conn = in_memory_conn();
        let current_exe = std::env::current_exe().expect("current exe path");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                "deps_venv_python_path",
                current_exe.to_string_lossy().as_ref()
            ],
        )
        .expect("insert managed python path");

        let settings = load_probe_python_settings(&conn);
        let result = resolve_probe_python_from_settings(settings, ProbePythonMode::RuntimeFallback);

        assert_eq!(result, Some(current_exe));
    }

    #[test]
    fn test_dependency_manager_mode_requires_managed_venv() {
        let conn = in_memory_conn();
        let current_exe = std::env::current_exe().expect("current exe path");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                "python.paddle_vl.path",
                current_exe.to_string_lossy().as_ref()
            ],
        )
        .expect("insert runtime python path");

        let settings = load_probe_python_settings(&conn);
        let result =
            resolve_probe_python_from_settings(settings, ProbePythonMode::DependencyManager);

        assert_eq!(
            result, None,
            "dependency checks must not fall back to runtime python when managed venv is missing"
        );
    }

    #[test]
    fn test_runtime_fallback_mode_can_use_runtime_candidates_without_managed_venv() {
        let conn = in_memory_conn();
        let current_exe = std::env::current_exe().expect("current exe path");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                "python.paddle_vl.path",
                current_exe.to_string_lossy().as_ref()
            ],
        )
        .expect("insert runtime python path");

        let settings = load_probe_python_settings(&conn);
        let result = resolve_probe_python_from_settings(settings, ProbePythonMode::RuntimeFallback);

        assert!(
            result.as_ref().map(|path| path.is_file()).unwrap_or(true),
            "runtime fallback mode may use runtime candidates when no managed venv exists"
        );
    }

    #[test]
    fn test_runtime_python_choice_prefers_healthy_managed_runtime_python() {
        let dir = tempdir().expect("temp dir");
        let runtime_python = dir.path().join("python");
        std::fs::write(&runtime_python, b"python").expect("write runtime python");

        let resolution = resolve_runtime_python_choice(
            None,
            vec![],
            Some(&runtime_python),
            Some(&crate::runtime::status::RuntimeStatus {
                state: crate::runtime::status::RuntimeState::Healthy,
                pack_version: Some("2026.05.0".to_string()),
                repair_needed: false,
                repair_available: true,
                summary: "Runtime listo".to_string(),
                blocked_capabilities: vec![],
                details: vec![],
                guidance: vec![],
                bootstrap_eligible: false,
                bootstrap_required: false,
                active_operation: None,
            }),
        );

        assert_eq!(resolution, Some(runtime_python));
    }

    #[test]
    fn test_runtime_python_choice_keeps_persisted_managed_path_when_runtime_unhealthy() {
        let dir = tempdir().expect("temp dir");
        let managed_python = dir.path().join("managed-python");
        let runtime_python = dir.path().join("runtime-python");
        std::fs::write(&managed_python, b"managed").expect("write managed python");
        std::fs::write(&runtime_python, b"runtime").expect("write runtime python");

        let resolution = resolve_runtime_python_choice(
            Some(managed_python.as_path()),
            vec![],
            Some(&runtime_python),
            Some(&crate::runtime::status::RuntimeStatus {
                state: crate::runtime::status::RuntimeState::Damaged,
                pack_version: Some("2026.05.0".to_string()),
                repair_needed: true,
                repair_available: true,
                summary: "Runtime dañado".to_string(),
                blocked_capabilities: vec![],
                details: vec!["checksum".to_string()],
                guidance: vec!["reparar".to_string()],
                bootstrap_eligible: false,
                bootstrap_required: false,
                active_operation: None,
            }),
        );

        assert_eq!(resolution, Some(managed_python));
    }
}
