#[cfg(feature = "local-ml")]
pub mod bootstrap;
pub mod bootstrap_types;
#[cfg(feature = "local-ml")]
pub mod download;
#[cfg(feature = "local-ml")]
pub mod manager;
#[cfg(feature = "local-ml")]
pub mod manifest;
#[cfg(feature = "local-ml")]
pub(crate) mod ops_lock;
#[cfg(feature = "local-ml")]
pub mod paths;
pub mod status;

#[cfg(feature = "local-ml")]
pub use manager::RuntimeManager;
#[cfg(feature = "local-ml")]
pub use paths::{
    managed_entry_path, managed_hf_cache_dir, managed_paddlex_cache_dir, managed_resource_path,
    managed_script_path, managed_venv_dir, managed_venv_python_path, managed_wheelhouse_dir,
};
// Contract types are always compiled (shared by both the local-ml and the lite arm).
// In the lean build no local consumer references them, but they are part of the
// public bootstrap contract — allow the re-export to be unused rather than gating
// it (gating a pub re-export risks breaking external/default-build consumers).
#[allow(unused_imports)]
pub use bootstrap_types::{
    BootstrapDownloadPlan, BootstrapPlan, BootstrapPlanSource, BootstrapRemoteSource,
};

// The three runtime commands keep an IDENTICAL signature + registered name in both
// build variants — only the body branches. The lite arm returns a Healthy/no-op
// status built from the always-compiled contract types (mirrors EntropIA Lite).

#[cfg(feature = "local-ml")]
#[tauri::command]
pub fn runtime_get_status(app_handle: tauri::AppHandle) -> Result<status::RuntimeStatus, String> {
    RuntimeManager::new().status(&app_handle)
}

#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub fn runtime_get_status(_app_handle: tauri::AppHandle) -> Result<status::RuntimeStatus, String> {
    Ok(lite_runtime_status())
}

#[cfg(feature = "local-ml")]
#[tauri::command]
pub fn runtime_get_bootstrap_plan(
    app_handle: tauri::AppHandle,
) -> Result<bootstrap_types::BootstrapPlan, String> {
    RuntimeManager::new().bootstrap_plan(&app_handle)
}

#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub fn runtime_get_bootstrap_plan(
    _app_handle: tauri::AppHandle,
) -> Result<bootstrap_types::BootstrapPlan, String> {
    Ok(lite_bootstrap_plan())
}

#[cfg(feature = "local-ml")]
#[tauri::command]
pub fn runtime_repair(app_handle: tauri::AppHandle) -> Result<status::RuntimeStatus, String> {
    let _guard = ops_lock::try_acquire("runtime_repair")?;
    RuntimeManager::new().repair(&app_handle)
}

#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub fn runtime_repair(_app_handle: tauri::AppHandle) -> Result<status::RuntimeStatus, String> {
    Ok(lite_runtime_status())
}

// API-only (lite) constructors for the always-compiled status/bootstrap contract types.
// All typed-optional fields are None, so building Pro's richer types is straightforward.
#[cfg(not(feature = "local-ml"))]
fn lite_runtime_status() -> status::RuntimeStatus {
    status::RuntimeStatus {
        state: status::RuntimeState::Healthy,
        pack_version: Some("lite-remote".to_string()),
        repair_needed: false,
        repair_available: false,
        summary: "EntropIA usa proveedores remotos de IA en este perfil".to_string(),
        blocked_capabilities: vec![],
        details: vec![
            "No se requiere instalación adicional de runtime de IA en este perfil.".to_string(),
        ],
        guidance: vec!["Configurá las claves remotas en Ajustes para usar IA.".to_string()],
        bootstrap_eligible: false,
        bootstrap_required: false,
        active_operation: None,
    }
}

#[cfg(not(feature = "local-ml"))]
fn lite_bootstrap_plan() -> bootstrap_types::BootstrapPlan {
    bootstrap_types::BootstrapPlan {
        eligible: false,
        required: false,
        source: Some(bootstrap_types::BootstrapPlanSource::ManagedReady),
        pack_version: Some("lite-remote".to_string()),
        summary: "EntropIA no necesita bootstrap de runtime de IA en este perfil".to_string(),
        reason: None,
        remote_source: None,
        download: None,
    }
}
