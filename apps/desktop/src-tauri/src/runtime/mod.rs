#[cfg(feature = "local-ml")]
pub mod bootstrap;
pub mod bootstrap_types;
#[cfg(feature = "local-ml")]
pub mod download;
#[cfg(feature = "local-ml")]
pub(crate) mod download_state;
#[cfg(feature = "local-ml")]
pub(crate) mod hash;
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
    BootstrapUnavailableReason,
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

// T2 (S2-background, S5): non-blocking bootstrap entry. The blocking core
// runs on a background worker thread — never the main thread — so the app
// stays usable for everything but OCR/NLP/transcription while downloading.
// Both commands are `async` (pinned by `start_command_is_async`); the
// start returns the current status promptly instead of awaiting the
// transfer, and a second start while one is in flight returns the current
// status without opening a second fetch (single-flight `ops_lock`).
// Progress keeps flowing through the existing `runtime://progress` emission
// the download core drives, so the DependenciasTab listener is unaffected.
#[cfg(feature = "local-ml")]
#[tauri::command]
pub async fn runtime_bootstrap_start(
    app_handle: tauri::AppHandle,
) -> Result<status::RuntimeStatus, String> {
    use crate::runtime::download::BootstrapSpawn;

    let worker_app = app_handle.clone();
    match crate::runtime::download::spawn_bootstrap_download("runtime_bootstrap_start", move || {
        // Cleared on launch and only here: an idempotent busy-start must
        // never disarm another download's cancel. A cancel arriving in
        // the microseconds between spawn and this line is swallowed —
        // negligible against disarming a live transfer instead.
        crate::runtime::download::clear_bootstrap_cancel();
        let manager = RuntimeManager::new();
        // A panicking worker stays observable: the ops-lock guard and
        // the running flag still release via `Drop` during unwinding,
        // and the panic is caught here so the transfer dies logged.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            manager.ensure_ready_or_bootstrap_unlocked(&worker_app)
        }));
        match outcome {
            Ok(result) => {
                if let Err(error) = &result {
                    eprintln!("[runtime] background bootstrap ended: {error}");
                }
                if crate::runtime::download::is_bootstrap_cancel_requested() {
                    // User cancel wins over any latched failure: re-emit
                    // the current status quietly instead of the Damaged
                    // classification the manager mapped the sentinel into
                    // (T3 owns the status copy; T2 only guarantees no
                    // error latch survives a cancel).
                    if let Ok(clean) = manager.status(&worker_app) {
                        let _ = manager.emit_status(&worker_app, &clean);
                    }
                }
            }
            Err(_) => {
                eprintln!("[runtime] background bootstrap worker panicked");
            }
        }
    }) {
        // Fire-and-forget by design: the detached worker owns the ops-lock
        // guard until the transfer ends (progress flows through the existing
        // `runtime://progress` emissions, completion through status events).
        // Binding the handle counts as the field read and documents the
        // detach — dropping it lets the worker run on.
        BootstrapSpawn::Started(_worker) => {}
        BootstrapSpawn::AlreadyRunning => {}
    }
    // Prompt either way: while a download is in flight this is its current
    // status instead of a second transfer; on a fresh start it is the
    // pre-transfer snapshot.
    RuntimeManager::new().status(&app_handle)
}

#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub async fn runtime_bootstrap_start(
    _app_handle: tauri::AppHandle,
) -> Result<status::RuntimeStatus, String> {
    Ok(lite_runtime_status())
}

/// T2 (S2): arms the shared cancel token the download read loop checks per
/// chunk; returns whether a bootstrap download was actually running.
#[cfg(feature = "local-ml")]
#[tauri::command]
pub async fn runtime_cancel_download() -> bool {
    crate::runtime::download::request_bootstrap_cancel()
}

#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub async fn runtime_cancel_download() -> bool {
    false
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
        blocked_reason: None,
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
        unavailable_reason: None,
        remote_source: None,
        download: None,
    }
}
