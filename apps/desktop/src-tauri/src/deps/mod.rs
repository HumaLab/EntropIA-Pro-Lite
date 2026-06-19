//! Dependency manager for EntropIA.
//!
//! Tracks the status of Python and Python-package dependencies required by the
//! AI pipeline (OCR, embeddings, transcription, NER). Provides probe/check,
//! install, and uv-binary management sub-modules.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::runtime::status::RuntimeState;
use crate::runtime::RuntimeManager;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

const UV_STATUS_CACHE_TTL: Duration = Duration::from_secs(30);
static UV_STATUS_CACHE: OnceLock<Mutex<Option<(Instant, UvStatusResult)>>> = OnceLock::new();

fn uv_status_cache() -> &'static Mutex<Option<(Instant, UvStatusResult)>> {
    UV_STATUS_CACHE.get_or_init(|| Mutex::new(None))
}

async fn invalidate_uv_status_cache() {
    let mut cached = uv_status_cache().lock().await;
    *cached = None;
}

pub mod checks;
pub mod install;
pub mod registry;
pub mod uv;

// Re-export checks so lib.rs can access them directly via `deps::checks`.
pub use checks::resolve_probe_python;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Identifies a single managed dependency.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DependencyId {
    Python,
    PaddlePaddle,
    PaddleOcr,
    FasterWhisper,
    Spacy,
}

/// The runtime status of a single dependency.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DependencyStatus {
    /// Status has never been checked.
    Unknown,
    /// A probe is currently running.
    Checking,
    /// Dependency is present and (optionally) at a known version.
    Installed { version: Option<String> },
    /// Dependency was probed and was not found.
    Missing,
    /// An installation is in progress.
    Installing { percent: u8 },
    /// The last install attempt failed with this message.
    Failed { message: String },
}

/// Shared, async-safe map of dependency statuses.
///
/// Wrapped in `Arc<Mutex<…>>` so it can be cloned cheaply and shared between
/// the Tauri command layer and background workers.
#[derive(Debug, Default)]
pub struct DepsStateData {
    pub statuses: HashMap<DependencyId, DependencyStatus>,
    pub cached_probe_python: Option<PathBuf>,
    pub cached_probe_results: Option<HashMap<DependencyId, DependencyStatus>>,
    pub probe_in_flight: bool,
    pub probe_generation: u64,
}

#[derive(Clone, Debug)]
pub struct DepsState(pub Arc<Mutex<DepsStateData>>);

fn default_dependency_statuses() -> HashMap<DependencyId, DependencyStatus> {
    use DependencyId::*;

    let mut map = HashMap::new();
    for id in [Python, PaddlePaddle, PaddleOcr, FasterWhisper, Spacy] {
        map.insert(id, DependencyStatus::Unknown);
    }
    map
}

fn missing_dependency_statuses() -> HashMap<DependencyId, DependencyStatus> {
    registry::all_deps()
        .into_iter()
        .map(|dep| (dep.id.clone(), DependencyStatus::Missing))
        .collect()
}

fn dep_results_from_map(
    results_map: HashMap<DependencyId, DependencyStatus>,
) -> Vec<DepCheckResult> {
    registry::all_deps()
        .iter()
        .filter_map(|dep| {
            results_map.get(&dep.id).cloned().map(|status| {
                let version = match &status {
                    DependencyStatus::Installed { version } => version.clone(),
                    _ => None,
                };
                DepCheckResult {
                    id: dep.id.clone(),
                    status,
                    version,
                }
            })
        })
        .collect()
}

/// Remove a directory tree, retrying transient Windows file locks
/// (ERROR_SHARING_VIOLATION / ACCESS_DENIED held by a closing ML child or an AV
/// scan) with backoff before giving up. `NotFound` counts as success.
async fn remove_dir_all_with_retry(path: &std::path::Path) -> Result<(), String> {
    const ATTEMPTS: u32 = 5;
    let mut last_err = String::new();
    for attempt in 0..ATTEMPTS {
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                tokio::time::sleep(std::time::Duration::from_millis(
                    200 * u64::from(attempt + 1),
                ))
                .await;
            }
        }
    }
    Err(last_err)
}

fn reset_candidate_paths(app_data_dir: &std::path::Path) -> Vec<PathBuf> {
    let mut reset_paths: Vec<PathBuf> = Vec::new();
    if let Some(managed_root) =
        RuntimeManager::new().discover_hydrated_runtime_root_for_tests(app_data_dir)
    {
        reset_paths.push(install::venv_path(&managed_root));
    }
    let runtime_dir = app_data_dir.join("runtime");
    if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                reset_paths.push(install::venv_path(&path));
            }
        }
    }
    reset_paths.push(install::dev_fallback_root(app_data_dir));
    reset_paths.push(app_data_dir.join("cache"));
    reset_paths.sort();
    reset_paths.dedup();
    reset_paths
}

impl DepsState {
    /// Create a new state map with all dependencies initialised to `Unknown`.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(DepsStateData {
            statuses: default_dependency_statuses(),
            cached_probe_python: None,
            cached_probe_results: None,
            probe_in_flight: false,
            probe_generation: 0,
        })))
    }
}

impl Default for DepsState {
    fn default() -> Self {
        Self::new()
    }
}

/// The outcome of probing a single dependency.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DepCheckResult {
    pub id: DependencyId,
    pub status: DependencyStatus,
    pub version: Option<String>,
}

/// Result returned by `deps_get_uv_status`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UvStatusResult {
    pub uv_ready: bool,
    pub uv_path: Option<String>,
    pub uv_version: Option<String>,
    pub uv_source: Option<String>,
    pub uv_compatible_for_dev: bool,
    pub venv_exists: bool,
    pub venv_path: Option<String>,
    pub uv_warning: Option<String>,
    pub release_runtime_ready: bool,
    pub release_runtime_state: Option<String>,
    pub dev_fallback_available: bool,
    pub dev_fallback_reason: Option<String>,
}

pub fn should_invalidate_cache_for_setting(key: &str) -> bool {
    matches!(
        key,
        "deps_venv_python_path"
            | "python.runtime_selection"
            | "python.paddle_vl.path"
            | "python.faster_whisper.path"
    )
}

pub async fn invalidate_probe_cache(state: &DepsState) {
    let mut data = state.0.lock().await;
    data.cached_probe_python = None;
    data.cached_probe_results = None;
    data.probe_in_flight = false;
    data.probe_generation = data.probe_generation.saturating_add(1);
    drop(data);
    checks::invalidate_resolved_probe_python_log();
}

pub async fn cache_current_statuses(state: &DepsState, probe_python: Option<PathBuf>) {
    let mut data = state.0.lock().await;
    data.cached_probe_python = probe_python;
    data.cached_probe_results = Some(data.statuses.clone());
    data.probe_in_flight = false;
}

async fn finish_probe_attempt(
    state: &DepsState,
    probe_generation: u64,
    probe_python: Option<PathBuf>,
    results: Option<HashMap<DependencyId, DependencyStatus>>,
) {
    let mut data = state.0.lock().await;
    if data.probe_generation != probe_generation {
        return;
    }
    data.cached_probe_python = probe_python;
    data.cached_probe_results = results.clone();
    data.probe_in_flight = false;
    if let Some(results_map) = results {
        for (id, status) in results_map {
            data.statuses.insert(id, status);
        }
    }
}

fn managed_runtime_probe_context(
    app_data_dir: &std::path::Path,
) -> (
    Option<PathBuf>,
    Option<crate::runtime::status::RuntimeStatus>,
) {
    let manager = crate::runtime::RuntimeManager::new();
    let Some(managed_root) = manager.discover_hydrated_runtime_root_for_tests(app_data_dir) else {
        return (None, None);
    };
    let Ok(manifest) = crate::runtime::manifest::RuntimeManifest::load_from_path(
        &managed_root.join("manifest.json"),
    ) else {
        return (None, None);
    };
    let Some(status) =
        manager.inspect_hydrated_runtime_for_tests(app_data_dir, &managed_root, &manifest)
    else {
        return (None, None);
    };

    if status.state != RuntimeState::Healthy {
        return (None, Some(status));
    }

    (
        Some(crate::runtime::managed_venv_python_path(&managed_root)),
        Some(status),
    )
}

pub async fn probe_all_once(
    state: &DepsState,
    db: &crate::db::state::AppDbState,
) -> Result<HashMap<DependencyId, DependencyStatus>, String> {
    loop {
        let probe_generation = {
            let mut data = state.0.lock().await;

            if let Some(results) = &data.cached_probe_results {
                return Ok(results.clone());
            }

            if data.probe_in_flight {
                None
            } else {
                data.probe_in_flight = true;
                for dep in registry::all_deps() {
                    data.statuses
                        .insert(dep.id.clone(), DependencyStatus::Checking);
                }
                Some(data.probe_generation)
            }
        };

        if let Some(probe_generation) = probe_generation {
            let probe_settings = {
                let conn = db
                    .ui_conn
                    .lock()
                    .map_err(|err| format!("DB lock error: {err}"));

                conn.map(|guard| checks::load_probe_python_settings(&guard))
            };

            let probe_settings = if let Ok(settings) = probe_settings {
                settings
            } else {
                finish_probe_attempt(state, probe_generation, None, None).await;
                return Err(probe_settings
                    .err()
                    .unwrap_or_else(|| "DB lock error".to_string()));
            };

            let (managed_runtime_python, managed_runtime_status) = managed_runtime_probe_context(
                db.db_path.parent().unwrap_or(std::path::Path::new(".")),
            );

            let python_path = checks::resolve_probe_python_with_runtime(
                probe_settings,
                checks::ProbePythonMode::DependencyManager,
                managed_runtime_python.as_deref(),
                managed_runtime_status.as_ref(),
            );

            let results_map = match python_path.clone() {
                Some(python) => checks::probe_all(&python).await,
                None => missing_dependency_statuses(),
            };

            finish_probe_attempt(
                state,
                probe_generation,
                python_path,
                Some(results_map.clone()),
            )
            .await;

            return Ok(results_map);
        }

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

fn all_critical_installed(results: &HashMap<DependencyId, DependencyStatus>) -> bool {
    registry::all_deps()
        .iter()
        .filter(|dep| dep.critical)
        .all(|dep| {
            matches!(
                results.get(&dep.id),
                Some(DependencyStatus::Installed { .. })
            )
        })
}

pub fn emit_probe_complete(
    app: &tauri::AppHandle,
    results: &HashMap<DependencyId, DependencyStatus>,
) {
    let failed = results
        .values()
        .filter(|status| matches!(status, DependencyStatus::Failed { .. }))
        .count();
    let missing = results
        .values()
        .filter(|status| matches!(status, DependencyStatus::Missing))
        .count();
    crate::app_logs::info(
        app,
        "deps",
        format!(
            "Verificación completada: {} dependencias, {missing} faltantes, {failed} fallidas",
            results.len()
        ),
    );

    let payload = install::DepsCompletePayload {
        results: dep_results_from_map(results.clone()),
        all_critical_installed: all_critical_installed(results),
    };

    if let Err(error) = app.emit("deps://complete", payload) {
        eprintln!("[deps] Failed to emit dependency completion event: {error}");
    }
}

/// Probe all registered dependencies and update the shared DepsState.
///
/// - Reads the venv Python path from app_settings via the UI DB connection.
/// - If no Python is available, returns all deps as `Missing`.
/// - Otherwise runs all probes concurrently and updates `DepsState`.
#[tauri::command]
pub async fn deps_check_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, DepsState>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<Vec<DepCheckResult>, String> {
    crate::app_logs::info(&app, "deps", "Verificando dependencias de IA");
    let results_map = probe_all_once(state.inner(), db.inner()).await?;
    emit_probe_complete(&app, &results_map);
    Ok(dep_results_from_map(results_map))
}

#[tauri::command]
pub async fn deps_get_cached_statuses(
    state: tauri::State<'_, DepsState>,
) -> Result<Vec<DepCheckResult>, String> {
    let data = state.0.lock().await;
    Ok(dep_results_from_map(data.statuses.clone()))
}

/// Install all registered dependencies into the managed venv.
///
/// - Ensures the uv binary (bundled/dev/system fallback, downloads only if needed).
/// - Creates the venv (idempotent).
/// - Persists venv Python paths in app_settings.
/// - Emits `deps://progress` events per dep, `deps://complete` when done.
#[tauri::command]
pub async fn deps_install_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, DepsState>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<(), String> {
    let _guard = crate::runtime::ops_lock::acquire_with_timeout("deps_install_all").await?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Error obteniendo directorio de datos de la app: {e}"))?;
    let db_path = db.db_path.clone();
    crate::app_logs::info(
        &app,
        "deps",
        "Inicio de instalación completa de dependencias",
    );
    invalidate_uv_status_cache().await;
    let result = install::install_all(&app, &state, &db_path, &app_data_dir).await;
    invalidate_uv_status_cache().await;
    match &result {
        Ok(()) => crate::app_logs::info(
            &app,
            "deps",
            "Instalación completa de dependencias finalizada",
        ),
        Err(error) => crate::app_logs::error(
            &app,
            "deps",
            format!("Instalación completa de dependencias falló: {error}"),
        ),
    }
    result
}

/// Install a single dependency by id string.
///
/// - The `id` must match a `DependencyId` variant in snake_case (e.g. `"paddle_ocr"`).
/// - Pre-flight: uv and venv must already exist.
/// - Emits `deps://progress` Installing → Installed/Failed.
#[tauri::command]
pub async fn deps_install_one(
    id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, DepsState>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<DepCheckResult, String> {
    let _guard = crate::runtime::ops_lock::acquire_with_timeout("deps_install_one").await?;
    // Parse the id string into a DependencyId using serde_json round-trip.
    let dep_id: DependencyId = serde_json::from_value(serde_json::Value::String(id.clone()))
        .map_err(|_| format!("ID de dependencia desconocido: '{id}'"))?;

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Error obteniendo directorio de datos de la app: {e}"))?;
    let db_path = db.db_path.clone();
    crate::app_logs::info(
        &app,
        "deps",
        format!("Inicio de instalación individual: {dep_id:?}"),
    );
    invalidate_uv_status_cache().await;
    let result = install::install_one(&dep_id, &app, &state, &db_path, &app_data_dir).await;
    invalidate_uv_status_cache().await;
    match &result {
        Ok(_) => crate::app_logs::info(
            &app,
            "deps",
            format!("Instalación individual finalizada: {dep_id:?}"),
        ),
        Err(error) => crate::app_logs::error(
            &app,
            "deps",
            format!("Instalación individual falló ({dep_id:?}): {error}"),
        ),
    }
    result
}

/// Return the current status of the managed uv binary and venv.
#[tauri::command]
pub async fn deps_get_uv_status(app: tauri::AppHandle) -> Result<UvStatusResult, String> {
    let mut cached = uv_status_cache().lock().await;
    if let Some((cached_at, result)) = cached.as_ref() {
        if cached_at.elapsed() < UV_STATUS_CACHE_TTL {
            return Ok(result.clone());
        }
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Error obteniendo directorio de datos de la app: {e}"))?;

    let runtime_status = RuntimeManager::new().status(&app).ok();
    let managed_runtime = install::load_managed_runtime_context(&app).ok().flatten();

    let uv_inspection = uv::UvBinary::inspect_with_runtime(
        Some(&app),
        &app_data_dir,
        managed_runtime
            .as_ref()
            .map(|runtime| runtime.managed_uv())
            .as_deref(),
        managed_runtime.as_ref().map(|runtime| &runtime.status),
    );
    let uv_ready = uv_inspection.ready.is_some();
    let uv_compatible_for_dev =
        uv_ready || uv::dev_system_uv_relaxed_allowed() && uv_inspection.detected_path.is_some();
    let uv_path = uv_inspection
        .ready
        .as_ref()
        .map(|b| b.path.to_string_lossy().into_owned())
        .or_else(|| {
            uv_inspection
                .detected_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
        });
    let uv_version = uv_inspection
        .ready
        .as_ref()
        .map(|b| b.version.clone())
        .or_else(|| uv_inspection.detected_version.clone());
    let release_runtime_ready = runtime_status
        .as_ref()
        .map(|status| status.state == RuntimeState::Healthy)
        .unwrap_or(false);
    let release_runtime_state = runtime_status
        .as_ref()
        .map(|status| format!("{:?}", status.state).to_ascii_lowercase());
    let dev_prerequisites = install::inspect_dev_fallback_prerequisites(&app_data_dir);
    let dev_fallback_available = install::dev_fallback_allowed()
        && uv_compatible_for_dev
        && dev_prerequisites.python.is_some();
    let dev_fallback_reason = if dev_fallback_available {
        Some(install::dev_fallback_available_reason().to_string())
    } else if install::dev_fallback_allowed() {
        match (dev_prerequisites.python.is_some(), uv_compatible_for_dev) {
            (false, false) => Some(
                "Fallback de desarrollo no disponible: falta Python 3.11+ y también falta un uv del sistema utilizable."
                    .to_string(),
            ),
            (false, true) => Some(
                "Fallback de desarrollo no disponible: detectamos uv, pero falta Python 3.11+ en el sistema."
                    .to_string(),
            ),
            (true, false) => Some(
                "Fallback de desarrollo no disponible: detectamos Python 3.11+, pero falta un uv del sistema utilizable."
                    .to_string(),
            ),
            (true, true) => None,
        }
    } else if !release_runtime_ready {
        Some(install::dev_fallback_platform_hint().to_string())
    } else {
        None
    };
    let uv_source = match (release_runtime_ready, uv_ready, uv_path.as_ref()) {
        (true, true, Some(_)) => Some("managed-runtime".to_string()),
        (_, true, Some(_)) => Some("strict-compatible".to_string()),
        (_, false, Some(_)) if dev_fallback_available => Some("system-dev-fallback".to_string()),
        _ => None,
    };

    let venv_python = managed_runtime
        .as_ref()
        .map(|runtime| runtime.venv_python())
        .unwrap_or_else(|| {
            install::dev_fallback_python_path(&install::dev_fallback_root(&app_data_dir))
        });
    let venv_exists = venv_python.is_file();
    let venv_path = if venv_exists {
        Some(
            venv_python
                .parent()
                .and_then(|parent| parent.parent())
                .unwrap_or_else(|| std::path::Path::new(""))
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };

    let result = UvStatusResult {
        uv_ready,
        uv_path,
        uv_version,
        uv_source,
        uv_compatible_for_dev,
        venv_exists,
        venv_path,
        uv_warning: uv_inspection.warning,
        release_runtime_ready,
        release_runtime_state,
        dev_fallback_available,
        dev_fallback_reason,
    };

    *cached = Some((Instant::now(), result.clone()));
    Ok(result)
}

/// Reset the dependency manager: delete the venv, clear settings, invalidate caches.
///
/// After this, `deps_install_all` must be run again to restore Python functionality.
#[tauri::command]
pub async fn deps_reset(
    state: tauri::State<'_, DepsState>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let _guard = crate::runtime::ops_lock::try_acquire("deps_reset")?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Error obteniendo directorio de datos de la app: {e}"))?;

    // ── 1. Delete actual managed/dev venvs and transient dependency caches ───
    // Continue past a locked path instead of aborting the whole reset on the
    // first one (a live OCR/transcription child or AV can hold a venv DLL). Each
    // path is retried with backoff; persistent failures are collected and
    // reported at the end so the settings/state are still cleared.
    let mut locked_paths: Vec<String> = Vec::new();
    for path in reset_candidate_paths(&app_data_dir) {
        if !path.exists() {
            continue;
        }
        match remove_dir_all_with_retry(&path).await {
            Ok(()) => {
                eprintln!("[deps] Reset deleted: {}", path.display());
                crate::app_logs::warn(
                    &app,
                    "deps",
                    format!("Reset eliminó entorno/caché: {}", path.display()),
                );
            }
            Err(e) => {
                eprintln!("[deps] Reset could not delete {}: {e}", path.display());
                crate::app_logs::warn(
                    &app,
                    "deps",
                    format!("Reset no pudo eliminar {}: {e}", path.display()),
                );
                locked_paths.push(format!("{} ({e})", path.display()));
            }
        }
    }

    // ── 2. Delete Python-path settings from app_settings ─────────────────────
    {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        let keys = [
            "deps_venv_python_path",
            "python.runtime_selection",
            "python.paddle_vl.path",
            "python.faster_whisper.path",
        ];
        for key in keys {
            crate::settings::delete_setting(&conn, key)
                .map_err(|e| format!("Error eliminando configuración '{key}': {e}"))?;
        }
    }

    // ── 3. Invalidate the Python discovery probe cache ────────────────────────
    crate::python_discovery::invalidate_probe_cache();
    invalidate_uv_status_cache().await;
    invalidate_probe_cache(state.inner()).await;

    // ── 4. Reset DepsState without caching synthetic Missing ─────────────────
    {
        let mut map = state.0.lock().await;
        map.statuses = default_dependency_statuses();
        map.cached_probe_python = None;
        map.cached_probe_results = None;
        map.probe_in_flight = false;
        map.probe_generation = map.probe_generation.saturating_add(1);
    }

    if !locked_paths.is_empty() {
        let detail = locked_paths.join("; ");
        crate::app_logs::warn(&app, "deps", format!("Reset: rutas bloqueadas: {detail}"));
        return Err(format!(
            "El reset limpió la configuración pero no pudo eliminar {} ruta(s) bloqueada(s) \
             (posible OCR/transcripción en curso o antivirus): {detail}. Cerrá los trabajos en \
             curso y volvé a intentar.",
            locked_paths.len()
        ));
    }

    eprintln!("[deps] Reset complete — dependency state invalidated");
    crate::app_logs::warn(&app, "deps", "Reset de dependencias completado");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    fn write_file(root: &std::path::Path, relpath: &str, bytes: &[u8]) -> String {
        let path = root.join(relpath);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, bytes).expect("write file");
        format!("{:x}", Sha256::digest(bytes))
    }

    #[test]
    fn reset_candidate_paths_include_real_managed_venv_and_dev_state() {
        let app_data_dir = tempdir().expect("app data dir");
        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        std::fs::create_dir_all(install::venv_path(&managed_root)).expect("create venv");
        std::fs::create_dir_all(install::dev_fallback_root(app_data_dir.path()))
            .expect("create dev fallback");
        std::fs::create_dir_all(app_data_dir.path().join("cache")).expect("create cache");

        let paths = reset_candidate_paths(app_data_dir.path());

        assert!(paths.contains(&install::venv_path(&managed_root)));
        assert!(paths.contains(&install::dev_fallback_root(app_data_dir.path())));
        assert!(paths.contains(&app_data_dir.path().join("cache")));
        assert!(!paths.contains(&install::venv_path(app_data_dir.path())));
    }

    #[test]
    fn managed_runtime_probe_context_prefers_hydrated_runtime_python() {
        let app_data_dir = tempdir().expect("app data dir");
        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        let python_relpath = if cfg!(windows) {
            "python/python.exe"
        } else {
            "python/bin/python3"
        };
        let venv_python_relpath = if cfg!(windows) {
            "venv/entropia-env/Scripts/python.exe"
        } else {
            "venv/entropia-env/bin/python"
        };
        let uv_relpath = if cfg!(windows) {
            "uv/uv.exe"
        } else {
            "uv/bin/uv"
        };
        let python_sha = write_file(&managed_root, python_relpath, b"python");
        let uv_sha = write_file(&managed_root, uv_relpath, b"uv");
        write_file(&managed_root, venv_python_relpath, b"venv-python");
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

        let (python, status) = managed_runtime_probe_context(app_data_dir.path());

        assert_eq!(
            status.map(|status| status.state),
            Some(RuntimeState::Healthy)
        );
        assert_eq!(
            python,
            Some(crate::runtime::managed_venv_python_path(&managed_root))
        );
    }

    #[test]
    fn default_dependency_statuses_includes_paddlepaddle() {
        let statuses = default_dependency_statuses();
        assert!(
            statuses.contains_key(&DependencyId::PaddlePaddle),
            "default statuses must include PaddlePaddle"
        );
        assert_eq!(
            statuses[&DependencyId::PaddlePaddle],
            DependencyStatus::Unknown
        );
    }

    #[test]
    fn should_invalidate_cache_for_runtime_selection_escape_hatch() {
        assert!(should_invalidate_cache_for_setting(
            "python.runtime_selection"
        ));
    }

    #[test]
    fn uv_status_result_serializes_new_dev_fallback_fields() {
        let status = UvStatusResult {
            uv_ready: false,
            uv_path: Some("/usr/bin/uv".to_string()),
            uv_version: Some("0.10.3".to_string()),
            uv_source: Some("system-dev-fallback".to_string()),
            uv_compatible_for_dev: true,
            venv_exists: false,
            venv_path: None,
            uv_warning: Some("warning".to_string()),
            release_runtime_ready: false,
            release_runtime_state: Some("fixture".to_string()),
            dev_fallback_available: true,
            dev_fallback_reason: Some("reason".to_string()),
        };

        let json = serde_json::to_value(&status).expect("serialize uv status");

        assert_eq!(
            json.get("uv_source").and_then(|value| value.as_str()),
            Some("system-dev-fallback")
        );
        assert_eq!(
            json.get("dev_fallback_available")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn dependency_failed_status_serializes_as_stable_tagged_object() {
        let status = DependencyStatus::Failed {
            message: "probe failed".to_string(),
        };

        let json = serde_json::to_value(&status).expect("serialize failed status");

        assert_eq!(
            json,
            serde_json::json!({
                "type": "failed",
                "message": "probe failed"
            })
        );
    }

    #[test]
    fn deps_complete_payload_serializes_failed_status_without_error() {
        let payload = install::DepsCompletePayload {
            results: vec![DepCheckResult {
                id: DependencyId::PaddleOcr,
                status: DependencyStatus::Failed {
                    message: "blocked because PaddlePaddle failed".to_string(),
                },
                version: None,
            }],
            all_critical_installed: false,
        };

        let json = serde_json::to_value(&payload).expect("serialize complete payload");

        assert_eq!(
            json["results"][0]["status"],
            serde_json::json!({
                "type": "failed",
                "message": "blocked because PaddlePaddle failed"
            })
        );
        assert_eq!(json["all_critical_installed"], serde_json::json!(false));
    }
}
