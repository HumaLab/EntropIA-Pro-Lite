//! Lite dependency command surface.
//!
//! EntropIA Lite uses remote providers only. These commands are kept so existing
//! Tauri/frontend contracts continue to compile, but they never probe, install,
//! or delete legacy local runtime dependencies.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::Mutex;

pub type DependencyId = String;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DependencyStatus {
    Unknown,
    Checking,
    Installed { version: Option<String> },
    Missing,
    Installing { percent: u8 },
    Failed { message: String },
}

#[derive(Debug, Default)]
pub struct DepsStateData {
    pub statuses: HashMap<DependencyId, DependencyStatus>,
    pub cached_probe_results: Option<HashMap<DependencyId, DependencyStatus>>,
    pub probe_in_flight: bool,
    pub probe_generation: u64,
}

#[derive(Clone, Debug)]
pub struct DepsState(pub Arc<Mutex<DepsStateData>>);

impl DepsState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(DepsStateData::default())))
    }
}

impl Default for DepsState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DepCheckResult {
    pub id: DependencyId,
    pub status: DependencyStatus,
    pub version: Option<String>,
}

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

#[derive(Clone, Serialize)]
pub struct DepsCompletePayload {
    pub results: Vec<DepCheckResult>,
    pub all_critical_installed: bool,
}

fn lite_dependency_results() -> Vec<DepCheckResult> {
    Vec::new()
}

fn lite_dependency_statuses() -> HashMap<DependencyId, DependencyStatus> {
    HashMap::new()
}

fn lite_uv_status() -> UvStatusResult {
    UvStatusResult {
        uv_ready: false,
        uv_path: None,
        uv_version: None,
        uv_source: Some("lite-remote".to_string()),
        uv_compatible_for_dev: false,
        venv_exists: false,
        venv_path: None,
        uv_warning: Some(
            "EntropIA Lite no usa gestor ni entorno local de dependencias.".to_string(),
        ),
        release_runtime_ready: true,
        release_runtime_state: Some("healthy".to_string()),
        dev_fallback_available: false,
        dev_fallback_reason: Some("No requerido en EntropIA Lite.".to_string()),
    }
}

pub fn should_invalidate_cache_for_setting(_key: &str) -> bool {
    false
}

pub async fn invalidate_probe_cache(state: &DepsState) {
    let mut data = state.0.lock().await;
    data.cached_probe_results = None;
    data.probe_in_flight = false;
    data.probe_generation = data.probe_generation.saturating_add(1);
}

pub async fn probe_all_once(
    state: &DepsState,
    _db: &crate::db::state::AppDbState,
) -> Result<HashMap<DependencyId, DependencyStatus>, String> {
    let results = lite_dependency_statuses();
    let mut data = state.0.lock().await;
    data.statuses = results.clone();
    data.cached_probe_results = Some(results.clone());
    data.probe_in_flight = false;
    Ok(results)
}

pub fn emit_probe_complete(
    app: &tauri::AppHandle,
    results: &HashMap<DependencyId, DependencyStatus>,
) {
    crate::app_logs::info(
        app,
        "deps",
        "EntropIA Lite: sin dependencias locales para verificar".to_string(),
    );

    let payload = DepsCompletePayload {
        results: results
            .iter()
            .map(|(id, status)| DepCheckResult {
                id: id.clone(),
                status: status.clone(),
                version: match status {
                    DependencyStatus::Installed { version } => version.clone(),
                    _ => None,
                },
            })
            .collect(),
        all_critical_installed: true,
    };

    if let Err(error) = app.emit("deps://complete", payload) {
        eprintln!("[deps] Failed to emit dependency completion event: {error}");
    }
}

#[tauri::command]
pub async fn deps_check_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, DepsState>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<Vec<DepCheckResult>, String> {
    crate::app_logs::info(
        &app,
        "deps",
        "EntropIA Lite: dependencias de IA resueltas mediante proveedores remotos",
    );
    let results_map = probe_all_once(state.inner(), db.inner()).await?;
    emit_probe_complete(&app, &results_map);
    Ok(lite_dependency_results())
}

#[tauri::command]
pub async fn deps_get_cached_statuses(
    state: tauri::State<'_, DepsState>,
) -> Result<Vec<DepCheckResult>, String> {
    let data = state.0.lock().await;
    Ok(data
        .statuses
        .iter()
        .map(|(id, status)| DepCheckResult {
            id: id.clone(),
            status: status.clone(),
            version: match status {
                DependencyStatus::Installed { version } => version.clone(),
                _ => None,
            },
        })
        .collect())
}

#[tauri::command]
pub async fn deps_install_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, DepsState>,
    _db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<(), String> {
    let results = lite_dependency_statuses();
    {
        let mut data = state.0.lock().await;
        data.statuses = results.clone();
        data.cached_probe_results = Some(results.clone());
    }
    emit_probe_complete(&app, &results);
    crate::app_logs::info(
        &app,
        "deps",
        "EntropIA Lite: no hay dependencias locales para instalar",
    );
    Ok(())
}

#[tauri::command]
pub async fn deps_install_one(
    id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, DepsState>,
    _db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<DepCheckResult, String> {
    let result = DepCheckResult {
        id,
        status: DependencyStatus::Installed {
            version: Some("lite-no-local-runtime".to_string()),
        },
        version: Some("lite-no-local-runtime".to_string()),
    };
    {
        let mut data = state.0.lock().await;
        data.statuses = lite_dependency_statuses();
        data.cached_probe_results = Some(data.statuses.clone());
    }
    emit_probe_complete(&app, &lite_dependency_statuses());
    crate::app_logs::info(
        &app,
        "deps",
        "EntropIA Lite: instalación individual omitida",
    );
    Ok(result)
}

#[tauri::command]
pub async fn deps_get_uv_status(_app: tauri::AppHandle) -> Result<UvStatusResult, String> {
    Ok(lite_uv_status())
}

#[tauri::command]
pub async fn deps_reset(
    state: tauri::State<'_, DepsState>,
    _db: tauri::State<'_, crate::db::state::AppDbState>,
    _app: tauri::AppHandle,
) -> Result<(), String> {
    let mut data = state.0.lock().await;
    data.statuses = lite_dependency_statuses();
    data.cached_probe_results = Some(data.statuses.clone());
    data.probe_in_flight = false;
    data.probe_generation = data.probe_generation.saturating_add(1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lite_has_no_local_dependency_results() {
        assert!(lite_dependency_results().is_empty());
        assert!(lite_dependency_statuses().is_empty());
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
        let payload = DepsCompletePayload {
            results: vec![DepCheckResult {
                id: "legacy-ner".to_string(),
                status: DependencyStatus::Failed {
                    message: "probe failed".to_string(),
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
                "message": "probe failed"
            })
        );
        assert_eq!(json["all_critical_installed"], serde_json::json!(false));
    }
}
