use crate::runtime::bootstrap::{
    bootstrap_operation_from_plan, fetch_remote_catalog, BootstrapController, BootstrapPlan,
    BootstrapRemoteCatalog,
};
use crate::runtime::download::download_and_activate_remote_runtime;
use crate::runtime::manifest::RuntimeManifest;
use crate::runtime::paths::{
    current_runtime_platform, ensure_executable_bit, managed_pack_dir, runtime_root,
    stage_marker_path, staging_pack_dir,
};
use crate::runtime::status::{
    RuntimeCapability, RuntimeOperation, RuntimeOperationKind, RuntimeOperationStage, RuntimeState,
    RuntimeStatus,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Debug, Default)]
pub struct RuntimeManager;

const RUNTIME_PACK_ROOT_ENV: &str = "ENTROPIA_RUNTIME_PACK_ROOT";

impl RuntimeManager {
    pub fn new() -> Self {
        Self
    }

    #[cfg(test)]
    pub fn ensure_ready_for_tests(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
    ) -> Result<RuntimeStatus, String> {
        let manifest = self.load_manifest(bundle_root)?;
        if let Some(status) = compatibility_status(&manifest) {
            return Ok(status);
        }

        hydrate_runtime(bundle_root, app_data_dir, &manifest)?;
        Ok(healthy_status(&manifest.pack_version))
    }

    #[cfg(test)]
    pub fn repair_for_tests(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
    ) -> Result<RuntimeStatus, String> {
        let manifest = self.load_manifest(bundle_root)?;
        if let Some(status) = compatibility_status(&manifest) {
            return Ok(status);
        }

        hydrate_runtime(bundle_root, app_data_dir, &manifest)?;
        Ok(healthy_status(&manifest.pack_version))
    }

    pub fn status_for_tests(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
    ) -> Result<RuntimeStatus, String> {
        let manifest = match self.load_manifest(bundle_root) {
            Ok(manifest) => manifest,
            Err(error) => {
                return Ok(RuntimeStatus {
                    state: RuntimeState::Incompatible,
                    pack_version: None,
                    repair_needed: false,
                    repair_available: false,
                    summary: "Runtime pack incompatible o incompleto".to_string(),
                    blocked_capabilities: blocked_capabilities(),
                    details: vec![error],
                    guidance: vec![
                        "Verificá que exista resources/runtime-pack/<platform>/manifest.json para esta plataforma.".to_string(),
                    ],
                    bootstrap_eligible: false,
                    bootstrap_required: false,
                    active_operation: None,
                })
            }
        };
        if let Some(status) = compatibility_status(&manifest) {
            if status.state == RuntimeState::Fixture {
                if let Some(existing_status) =
                    self.discover_hydrated_runtime_status_for_tests(app_data_dir)
                {
                    return Ok(existing_status);
                }
            }
            return Ok(status);
        }
        inspect_runtime(bundle_root, app_data_dir, &manifest)
    }

    pub fn status(&self, app_handle: &AppHandle) -> Result<RuntimeStatus, String> {
        let bundle_root = resolve_bundle_root(app_handle)?;
        let app_data_dir = crate::path_utils::cache_dir(app_handle)
            .map_err(|error| format!("Failed to get app data dir: {error}"))?;
        self.status_for_tests(&bundle_root, &app_data_dir)
    }

    pub fn bootstrap_plan(&self, app_handle: &AppHandle) -> Result<BootstrapPlan, String> {
        let bundle_root = resolve_bundle_root(app_handle)?;
        let app_data_dir = crate::path_utils::cache_dir(app_handle)
            .map_err(|error| format!("Failed to get app data dir: {error}"))?;
        let remote_catalog = configured_bootstrap_catalog(app_handle)?;

        self.plan_bootstrap_for_tests(&bundle_root, &app_data_dir, remote_catalog)
    }

    pub fn ensure_ready_or_bootstrap(
        &self,
        app_handle: &AppHandle,
    ) -> Result<RuntimeStatus, String> {
        let _guard = crate::runtime::ops_lock::try_acquire("runtime_bootstrap")?;
        self.ensure_ready_or_bootstrap_unlocked(app_handle)
    }

    pub(crate) fn ensure_ready_or_bootstrap_unlocked(
        &self,
        app_handle: &AppHandle,
    ) -> Result<RuntimeStatus, String> {
        let bundle_root = resolve_bundle_root(app_handle)?;
        let app_data_dir = crate::path_utils::cache_dir(app_handle)
            .map_err(|error| format!("Failed to get app data dir: {error}"))?;

        let mut emit_error: Option<String> = None;
        let status = self.ensure_ready_or_bootstrap_with_remote_support(
            &bundle_root,
            &app_data_dir,
            configured_bootstrap_catalog(app_handle)?,
            |public_key_id| configured_bootstrap_public_key(app_handle, public_key_id),
            |source_manifest_url, release, app_data_dir, public_key_base64, on_progress| {
                download_and_activate_remote_runtime(
                    source_manifest_url,
                    release,
                    app_data_dir,
                    public_key_base64,
                    on_progress,
                )
            },
            |operation| {
                if emit_error.is_none() {
                    emit_error = self.emit_progress(app_handle, &operation).err();
                }
            },
        )?;

        if let Some(error) = emit_error {
            return Err(error);
        }

        self.emit_status(app_handle, &status)?;
        Ok(status)
    }

    pub fn repair(&self, app_handle: &AppHandle) -> Result<RuntimeStatus, String> {
        let bundle_root = resolve_bundle_root(app_handle)?;
        let app_data_dir = crate::path_utils::cache_dir(app_handle)
            .map_err(|error| format!("Failed to get app data dir: {error}"))?;
        let mut emit_error: Option<String> = None;
        let status = self.ensure_ready_or_bootstrap_with_remote_support(
            &bundle_root,
            &app_data_dir,
            configured_bootstrap_catalog(app_handle)?,
            |public_key_id| configured_bootstrap_public_key(app_handle, public_key_id),
            |source_manifest_url, release, app_data_dir, public_key_base64, on_progress| {
                download_and_activate_remote_runtime(
                    source_manifest_url,
                    release,
                    app_data_dir,
                    public_key_base64,
                    on_progress,
                )
            },
            |operation| {
                if emit_error.is_none() {
                    emit_error = self.emit_progress(app_handle, &operation).err();
                }
            },
        )?;
        if let Some(error) = emit_error {
            return Err(error);
        }
        self.emit_status(app_handle, &status)?;
        Ok(status)
    }

    pub fn validate_startup(&self, app_handle: &AppHandle) -> Result<RuntimeStatus, String> {
        let status = self.status(app_handle)?;
        self.emit_status(app_handle, &status)?;
        Ok(status)
    }

    pub fn hydrated_runtime_root(
        &self,
        app_handle: &AppHandle,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let bundle_root = resolve_bundle_root(app_handle)?;
        let manifest = self.load_manifest(&bundle_root)?;
        let app_data_dir = crate::path_utils::cache_dir(app_handle)
            .map_err(|error| format!("Failed to get app data dir: {error}"))?;

        if matches!(compatibility_status(&manifest), Some(status) if status.state == RuntimeState::Fixture)
        {
            if let Some(root) = self.discover_hydrated_runtime_root_for_tests(&app_data_dir) {
                return Ok(Some(root));
            }
        }

        Ok(self.hydrated_runtime_root_for_tests(&bundle_root, &app_data_dir, &manifest))
    }

    pub fn discover_hydrated_runtime_root_for_tests(
        &self,
        app_data_dir: &Path,
    ) -> Option<std::path::PathBuf> {
        self.discover_hydrated_runtime_for_tests(app_data_dir)
            .map(|(path, _status)| path)
    }

    fn discover_hydrated_runtime_status_for_tests(
        &self,
        app_data_dir: &Path,
    ) -> Option<RuntimeStatus> {
        self.discover_hydrated_runtime_for_tests(app_data_dir)
            .map(|(_path, status)| status)
    }

    fn discover_hydrated_runtime_for_tests(
        &self,
        app_data_dir: &Path,
    ) -> Option<(std::path::PathBuf, RuntimeStatus)> {
        let runtime_dir = runtime_root(app_data_dir);
        let entries = fs::read_dir(&runtime_dir).ok()?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Ok(manifest) = RuntimeManifest::load_from_path(&path.join("manifest.json")) else {
                continue;
            };
            if manifest.platform != current_runtime_platform() {
                continue;
            }
            let Some(status) =
                self.inspect_hydrated_runtime_for_tests(app_data_dir, &path, &manifest)
            else {
                continue;
            };
            if status.state == RuntimeState::Healthy {
                return Some((path, status));
            }
        }

        None
    }

    pub fn hydrated_runtime_root_for_tests(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
        manifest: &RuntimeManifest,
    ) -> Option<std::path::PathBuf> {
        let status = inspect_runtime(bundle_root, app_data_dir, manifest).ok()?;
        if status.state != RuntimeState::Healthy {
            return None;
        }

        let managed_root = managed_pack_dir(app_data_dir, &manifest.pack_version);
        managed_root.exists().then_some(managed_root)
    }

    pub fn inspect_hydrated_runtime_for_tests(
        &self,
        app_data_dir: &Path,
        managed_root: &Path,
        manifest: &RuntimeManifest,
    ) -> Option<RuntimeStatus> {
        let status = inspect_runtime(managed_root, app_data_dir, manifest).ok()?;
        if status.pack_version.as_deref() != Some(manifest.pack_version.as_str()) {
            return None;
        }
        Some(status)
    }

    pub fn emit_status(
        &self,
        app_handle: &AppHandle,
        status: &RuntimeStatus,
    ) -> Result<(), String> {
        crate::app_logs::info(
            app_handle,
            "runtime",
            format!("Estado runtime: {:?} · {}", status.state, status.summary),
        );
        for detail in &status.details {
            crate::app_logs::info(app_handle, "runtime", format!("Detalle runtime: {detail}"));
        }
        app_handle
            .emit("runtime://status", status)
            .map_err(|error| format!("Failed to emit runtime status event: {error}"))
    }

    #[allow(dead_code)]
    pub fn emit_progress(
        &self,
        app_handle: &AppHandle,
        operation: &RuntimeOperation,
    ) -> Result<(), String> {
        crate::app_logs::info(
            app_handle,
            "runtime",
            format!(
                "Operación {:?}/{:?}: {}{}",
                operation.kind,
                operation.stage,
                operation.summary,
                operation
                    .progress_percent
                    .map(|pct| format!(" ({pct}%)"))
                    .unwrap_or_default()
            ),
        );
        app_handle
            .emit("runtime://progress", operation)
            .map_err(|error| format!("Failed to emit runtime progress event: {error}"))
    }

    fn load_manifest(&self, bundle_root: &Path) -> Result<RuntimeManifest, String> {
        RuntimeManifest::load_from_path(&bundle_root.join("manifest.json"))
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn load_manifest_for_tests(&self, bundle_root: &Path) -> Result<RuntimeManifest, String> {
        self.load_manifest(bundle_root)
    }

    pub fn plan_bootstrap_for_tests(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
        remote_catalog: BootstrapRemoteCatalog,
    ) -> Result<BootstrapPlan, String> {
        let manifest = self.load_manifest(bundle_root)?;
        let bundle_status = inspect_runtime(bundle_root, app_data_dir, &manifest)?;

        Ok(
            BootstrapController::new().plan(
                &bundle_status,
                &manifest,
                app_data_dir,
                remote_catalog,
            ),
        )
    }

    #[cfg(test)]
    pub fn ensure_ready_or_bootstrap_for_tests<F>(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
        remote_catalog: BootstrapRemoteCatalog,
        on_progress: F,
    ) -> Result<RuntimeStatus, String>
    where
        F: FnMut(RuntimeOperation),
    {
        self.ensure_ready_or_bootstrap_with_remote_support(
            bundle_root,
            app_data_dir,
            remote_catalog,
            |public_key_id| {
                Err(format!(
                    "Trusted bootstrap public key '{public_key_id}' is not configured in tests"
                ))
            },
            |source_manifest_url, release, app_data_dir, public_key_base64, on_progress| {
                download_and_activate_remote_runtime(
                    source_manifest_url,
                    release,
                    app_data_dir,
                    public_key_base64,
                    on_progress,
                )
            },
            on_progress,
        )
    }

    fn ensure_ready_or_bootstrap_with_remote_support<F, K, D>(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
        remote_catalog: BootstrapRemoteCatalog,
        key_provider: K,
        remote_downloader: D,
        mut on_progress: F,
    ) -> Result<RuntimeStatus, String>
    where
        F: FnMut(RuntimeOperation),
        K: Fn(&str) -> Result<String, String>,
        D: Fn(
            &str,
            &crate::runtime::manifest::BootstrapReleaseManifest,
            &Path,
            &str,
            &mut dyn FnMut(RuntimeOperation),
        ) -> Result<crate::runtime::download::BootstrapDownloadOutcome, String>,
    {
        emit_bootstrap_progress(
            &mut on_progress,
            RuntimeOperationStage::Checking,
            "Evaluando readiness del runtime",
            None,
            None,
            None,
            true,
        );

        let manifest = match self.load_manifest(bundle_root) {
            Ok(manifest) => manifest,
            Err(error) => {
                return Ok(incompatible_missing_manifest_status(error));
            }
        };

        invalidate_stale_managed_runtime(app_data_dir, &manifest)?;
        invalidate_stale_managed_runtime_dirs(app_data_dir, &manifest)?;

        let current_status = inspect_runtime(bundle_root, app_data_dir, &manifest)?;
        if current_status.state == RuntimeState::Healthy {
            return Ok(current_status);
        }
        if let Some(existing_status) = self.discover_hydrated_runtime_status_for_tests(app_data_dir)
        {
            return Ok(existing_status);
        }

        let plan = BootstrapController::new().plan(
            &current_status,
            &manifest,
            app_data_dir,
            remote_catalog,
        );
        match plan.source {
            Some(crate::runtime::bootstrap::BootstrapPlanSource::BundledRelease) => {
                match hydrate_runtime_with_progress(
                    bundle_root,
                    app_data_dir,
                    &manifest,
                    &mut on_progress,
                ) {
                    Ok(()) => Ok(healthy_status(&manifest.pack_version)),
                    Err(error) => Ok(classify_bootstrap_failure(&manifest, error)),
                }
            }
            Some(crate::runtime::bootstrap::BootstrapPlanSource::ManagedReady) => {
                Ok(current_status)
            }
            Some(crate::runtime::bootstrap::BootstrapPlanSource::TrustedRemote) => {
                let source = plan.remote_source.clone().ok_or_else(|| {
                    "Trusted remote bootstrap plan is missing source metadata".to_string()
                })?;
                let source_manifest_url = source.manifest_url.clone();
                let public_key = match key_provider(&source.public_key_id) {
                    Ok(public_key) => public_key,
                    Err(error) => {
                        return Ok(blocked_bootstrap_status(
                            &manifest,
                            RuntimeState::BlockedSourceUnavailable,
                            plan,
                            error,
                            true,
                        ))
                    }
                };
                let release = crate::runtime::manifest::BootstrapReleaseManifest {
                    app_version: manifest.app_version.clone(),
                    platform: manifest.platform.clone(),
                    pack_version: plan.pack_version.clone().ok_or_else(|| {
                        "Trusted remote bootstrap plan is missing pack version".to_string()
                    })?,
                    archive_url: plan
                        .download
                        .as_ref()
                        .map(|download| download.archive_url.clone())
                        .ok_or_else(|| {
                            "Trusted remote bootstrap plan is missing download metadata".to_string()
                        })?,
                    additional_part_urls: plan
                        .download
                        .as_ref()
                        .map(|download| download.additional_part_urls.clone())
                        .unwrap_or_default(),
                    archive_sha256: plan
                        .download
                        .as_ref()
                        .map(|download| download.archive_sha256.clone())
                        .ok_or_else(|| {
                            "Trusted remote bootstrap plan is missing checksum".to_string()
                        })?,
                    archive_size: plan
                        .download
                        .as_ref()
                        .map(|download| download.archive_size)
                        .ok_or_else(|| {
                            "Trusted remote bootstrap plan is missing archive size".to_string()
                        })?,
                    signature: plan
                        .download
                        .as_ref()
                        .map(|download| download.signature.clone())
                        .ok_or_else(|| {
                            "Trusted remote bootstrap plan is missing signature".to_string()
                        })?,
                };

                match remote_downloader(
                    &source_manifest_url,
                    &release,
                    app_data_dir,
                    &public_key,
                    &mut on_progress,
                ) {
                    Ok(_) => Ok(healthy_status(&release.pack_version)),
                    Err(error) => Ok(classify_bootstrap_failure(&manifest, error)),
                }
            }
            None => Ok(classify_unavailable_bootstrap_plan(
                &manifest,
                plan,
                &mut on_progress,
            )),
        }
    }

    #[cfg(test)]
    pub fn ensure_ready_or_bootstrap_for_tests_with_remote_support<F, K, D>(
        &self,
        bundle_root: &Path,
        app_data_dir: &Path,
        remote_catalog: BootstrapRemoteCatalog,
        key_provider: K,
        remote_downloader: D,
        on_progress: F,
    ) -> Result<RuntimeStatus, String>
    where
        F: FnMut(RuntimeOperation),
        K: Fn(&str) -> Result<String, String>,
        D: Fn(
            &str,
            &crate::runtime::manifest::BootstrapReleaseManifest,
            &Path,
            &str,
            &mut dyn FnMut(RuntimeOperation),
        ) -> Result<crate::runtime::download::BootstrapDownloadOutcome, String>,
    {
        self.ensure_ready_or_bootstrap_with_remote_support(
            bundle_root,
            app_data_dir,
            remote_catalog,
            key_provider,
            remote_downloader,
            on_progress,
        )
    }
}

fn resolve_bundle_root(app_handle: &AppHandle) -> Result<std::path::PathBuf, String> {
    let platform = current_runtime_platform();
    if let Some(override_root) = resolve_env_bundle_root(&platform)? {
        return Ok(override_root);
    }

    if let Some(generated_dev_root) = resolve_generated_dev_bundle_root(&platform) {
        return Ok(generated_dev_root);
    }

    let resource_candidates = bundled_runtime_pack_resource_candidates(&platform);
    for resource_candidate in &resource_candidates {
        if let Ok(resource_root) = app_handle
            .path()
            .resolve(resource_candidate, tauri::path::BaseDirectory::Resource)
        {
            if resource_root.join("manifest.json").is_file() {
                return Ok(crate::path_utils::normalize_windows_path(resource_root));
            }
        }
    }

    #[cfg(debug_assertions)]
    {
        let dev_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("runtime-pack")
            .join(&platform);
        if dev_root.join("manifest.json").is_file() {
            return Ok(dev_root);
        }

        Err(missing_bundle_root_error(
            &platform,
            &resource_candidates,
            Some(&dev_root),
        ))
    }

    #[cfg(not(debug_assertions))]
    {
        Err(missing_bundle_root_error(
            &platform,
            &resource_candidates,
            None,
        ))
    }
}

fn bundled_runtime_pack_resource_candidates(platform: &str) -> Vec<String> {
    vec![
        format!("resources/runtime-pack/{platform}"),
        format!("runtime-pack/{platform}"),
    ]
}

fn missing_bundle_root_error(
    platform: &str,
    resource_candidates: &[String],
    dev_root: Option<&Path>,
) -> String {
    let mut tried = resource_candidates
        .iter()
        .map(|candidate| format!("Tauri Resource::{candidate}"))
        .collect::<Vec<_>>();

    if let Some(dev_root) = dev_root {
        tried.push(format!("dev fallback {}", dev_root.display()));
    }

    format!(
        "Bundled runtime-pack not found for platform {platform}. Tried: {}",
        tried.join(", ")
    )
}

fn resolve_generated_dev_bundle_root(platform: &str) -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("runtime-pack")
            .join(platform);
        if runtime_root_matches_current_app(&root, platform) {
            return Some(root);
        }
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = platform;
    }

    None
}

#[cfg(any(debug_assertions, test))]
fn runtime_root_matches_current_app(root: &Path, platform: &str) -> bool {
    let Ok(manifest) = RuntimeManifest::load_from_path(&root.join("manifest.json")) else {
        return false;
    };

    manifest.app_version == running_app_version() && manifest.platform == platform
}

fn resolve_env_bundle_root(platform: &str) -> Result<Option<PathBuf>, String> {
    resolve_env_bundle_root_from_value(platform, std::env::var_os(RUNTIME_PACK_ROOT_ENV))
}

fn resolve_env_bundle_root_from_value(
    platform: &str,
    value: Option<std::ffi::OsString>,
) -> Result<Option<PathBuf>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    };

    let root = crate::path_utils::normalize_windows_path(PathBuf::from(value));
    if root.join("manifest.json").is_file() {
        return Ok(Some(root));
    }

    let platform_root = root.join(platform);
    if platform_root.join("manifest.json").is_file() {
        return Ok(Some(platform_root));
    }

    Err(format!(
        "{RUNTIME_PACK_ROOT_ENV} apunta a {}, pero no se encontró manifest.json ni layout {platform}/manifest.json",
        root.display()
    ))
}

fn configured_bootstrap_catalog(app_handle: &AppHandle) -> Result<BootstrapRemoteCatalog, String> {
    let source = {
        let db = app_handle.state::<crate::db::state::AppDbState>();
        let conn = db
            .ui_conn
            .lock()
            .map_err(|error| format!("DB lock error while reading bootstrap source: {error}"))?;
        crate::settings::get_runtime_bootstrap_remote_source(&conn)?
    };

    let Some(source) = source else {
        return Ok(BootstrapRemoteCatalog::SourceUnavailable {
            source: None,
            reason: "Trusted remote bootstrap source is not configured in this environment"
                .to_string(),
        });
    };

    Ok(fetch_remote_catalog(source))
}

fn configured_bootstrap_public_key(
    app_handle: &AppHandle,
    public_key_id: &str,
) -> Result<String, String> {
    let db = app_handle.state::<crate::db::state::AppDbState>();
    let conn = db
        .ui_conn
        .lock()
        .map_err(|error| format!("DB lock error while reading bootstrap public key: {error}"))?;
    crate::settings::get_runtime_bootstrap_public_key(&conn, public_key_id)
}

fn inspect_runtime(
    bundle_root: &Path,
    app_data_dir: &Path,
    manifest: &RuntimeManifest,
) -> Result<RuntimeStatus, String> {
    if let Some(status) = compatibility_status(manifest) {
        return Ok(status);
    }

    let managed_root = managed_pack_dir(app_data_dir, &manifest.pack_version);
    if !managed_root.exists() {
        return Ok(RuntimeStatus {
            state: RuntimeState::Damaged,
            pack_version: Some(manifest.pack_version.clone()),
            repair_needed: true,
            repair_available: bundle_root.exists(),
            summary: "Runtime no hidratado".to_string(),
            blocked_capabilities: blocked_capabilities(),
            details: vec!["El runtime administrado todavía no existe en app-data".to_string()],
            guidance: vec![
                "Esto no es un crash: hidratá o repará el runtime desde Ajustes > Dependencias para habilitar OCR, NLP y transcripción.".to_string(),
            ],
            bootstrap_eligible: true,
            bootstrap_required: true,
            active_operation: Some(RuntimeOperation {
                kind: RuntimeOperationKind::Bootstrap,
                stage: RuntimeOperationStage::Checking,
                summary: "Evaluando bootstrap del runtime".to_string(),
                progress_percent: None,
                downloaded_bytes: None,
                total_bytes: None,
                retryable: true,
            }),
        });
    }

    let mut invalid_entries = Vec::new();
    for entry in manifest.all_entries() {
        let target = managed_root.join(&entry.path);
        let metadata = match fs::metadata(&target) {
            Ok(metadata) => metadata,
            Err(_) => {
                invalid_entries.push(format!("Falta {}", entry.path));
                continue;
            }
        };
        // STEP 13: presence alone is not enough to call a model/cache "ready".
        // A truncated or empty download keeps the path but changes its length, so
        // requiring the declared size to match rejects those corrupt files. (A
        // declared-non-empty file that is now zero bytes is covered here too,
        // since 0 != size.)
        if metadata.len() != entry.size {
            invalid_entries.push(format!(
                "Tamaño inválido en {} (esperado {}, detectado {})",
                entry.path,
                entry.size,
                metadata.len()
            ));
        }
    }

    // STEP 12 / STEP 13: for the few CRITICAL entries (python/uv launchers +
    // native model libs) recompute the SHA-256 and compare it to the manifest,
    // so a file that is present with the right size but corrupt content is
    // detected. This catches a same-size corrupt launcher or a same-size corrupt
    // native model lib that the size check above cannot. The multi-GB wheelhouse
    // and model caches are deliberately NOT re-hashed to keep inspection bounded.
    invalid_entries.extend(critical_entry_integrity_failures(&managed_root, manifest));

    if invalid_entries.is_empty() {
        return Ok(healthy_status(&manifest.pack_version));
    }

    Ok(RuntimeStatus {
        state: RuntimeState::Damaged,
        pack_version: Some(manifest.pack_version.clone()),
        repair_needed: true,
        repair_available: bundle_root.exists(),
        summary: "Runtime dañado".to_string(),
        blocked_capabilities: blocked_capabilities(),
        details: invalid_entries,
        guidance: vec![
            "Podés intentar 'Reparar runtime' para rehidratar los archivos desde el runtime-pack disponible.".to_string(),
        ],
        bootstrap_eligible: true,
        bootstrap_required: true,
        active_operation: Some(RuntimeOperation {
            kind: RuntimeOperationKind::Bootstrap,
            stage: RuntimeOperationStage::Checking,
            summary: "El runtime necesita validación y posible bootstrap".to_string(),
            progress_percent: None,
            downloaded_bytes: None,
            total_bytes: None,
            retryable: true,
        }),
    })
}

fn invalidate_stale_managed_runtime(
    app_data_dir: &Path,
    manifest: &RuntimeManifest,
) -> Result<(), String> {
    let managed_root = managed_pack_dir(app_data_dir, &manifest.pack_version);
    if !managed_root.exists() {
        return Ok(());
    }

    let managed_manifest_path = managed_root.join("manifest.json");
    let Ok(managed_manifest) = RuntimeManifest::load_from_path(&managed_manifest_path) else {
        return Ok(());
    };

    if managed_manifest.app_version == manifest.app_version
        && managed_manifest.platform == manifest.platform
    {
        return Ok(());
    }

    fs::remove_dir_all(&managed_root).map_err(|error| {
        format!(
            "Failed to invalidate stale runtime {}: {error}",
            managed_root.display()
        )
    })
}

fn invalidate_stale_managed_runtime_dirs(
    app_data_dir: &Path,
    current_manifest: &RuntimeManifest,
) -> Result<(), String> {
    let runtime_dir = runtime_root(app_data_dir);
    let Ok(entries) = fs::read_dir(&runtime_dir) else {
        return Ok(());
    };

    let current_root = managed_pack_dir(app_data_dir, &current_manifest.pack_version);
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path == current_root {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') || name == "downloads" {
            continue;
        }
        let Ok(manifest) = RuntimeManifest::load_from_path(&path.join("manifest.json")) else {
            continue;
        };
        if manifest.platform != current_manifest.platform {
            continue;
        }
        if manifest.app_version == running_app_version() {
            continue;
        }

        fs::remove_dir_all(&path).map_err(|error| {
            format!(
                "Failed to invalidate stale runtime {}: {error}",
                path.display()
            )
        })?;
    }

    Ok(())
}

fn hydrate_runtime_with_progress<F>(
    bundle_root: &Path,
    app_data_dir: &Path,
    manifest: &RuntimeManifest,
    on_progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(RuntimeOperation),
{
    emit_bootstrap_progress(
        on_progress,
        RuntimeOperationStage::Hydrating,
        "Hidratando runtime desde el bundle local",
        Some(25),
        None,
        None,
        true,
    );
    hydrate_runtime_progress(bundle_root, app_data_dir, manifest, Some(on_progress))?;
    emit_bootstrap_progress(
        on_progress,
        RuntimeOperationStage::Verifying,
        "Verificando integridad del runtime hidratado",
        Some(75),
        None,
        None,
        true,
    );

    let verified_status = inspect_runtime(bundle_root, app_data_dir, manifest)?;
    if verified_status.state != RuntimeState::Healthy {
        return Err(format!(
            "Hydrated runtime verification failed with state {:?}",
            verified_status.state
        ));
    }

    emit_bootstrap_progress(
        on_progress,
        RuntimeOperationStage::Activating,
        "Activando runtime hidratado",
        Some(100),
        None,
        None,
        true,
    );

    Ok(())
}

#[cfg(test)]
fn hydrate_runtime(
    bundle_root: &Path,
    app_data_dir: &Path,
    manifest: &RuntimeManifest,
) -> Result<(), String> {
    hydrate_runtime_progress::<fn(RuntimeOperation)>(bundle_root, app_data_dir, manifest, None)
}

fn hydrate_runtime_progress<F>(
    bundle_root: &Path,
    app_data_dir: &Path,
    manifest: &RuntimeManifest,
    mut on_progress: Option<&mut F>,
) -> Result<(), String>
where
    F: FnMut(RuntimeOperation),
{
    fs::create_dir_all(runtime_root(app_data_dir)).map_err(|error| {
        format!(
            "Failed to create runtime root {}: {error}",
            runtime_root(app_data_dir).display()
        )
    })?;

    let staging_root = staging_pack_dir(app_data_dir, &manifest.pack_version);
    if staging_root.exists() {
        fs::remove_dir_all(&staging_root).map_err(|error| {
            format!(
                "Failed to clean staging runtime {}: {error}",
                staging_root.display()
            )
        })?;
    }
    fs::create_dir_all(&staging_root).map_err(|error| {
        format!(
            "Failed to create staging runtime {}: {error}",
            staging_root.display()
        )
    })?;

    write_stage_marker(app_data_dir, &manifest.pack_version, "copying")?;
    let entries = manifest.all_entries();
    let total_bytes = entries.iter().map(|entry| entry.size).sum::<u64>();
    let mut copied_bytes = 0u64;
    let mut last_progress = 25u8;
    for (index, entry) in entries.iter().enumerate() {
        let source = bundle_root.join(&entry.path);
        if !source.exists() {
            return Err(format!(
                "Bundled runtime entry missing: {}",
                source.display()
            ));
        }
        let actual_sha256 = file_sha256(&source)?;
        if actual_sha256 != entry.sha256 {
            return Err(format!(
                "Bundled runtime checksum mismatch for {} (entry {}, expected {}, got {}, expected size {}, actual size {})",
                source.display(),
                entry.path,
                entry.sha256,
                actual_sha256,
                entry.size,
                fs::metadata(&source).map(|metadata| metadata.len()).unwrap_or(0)
            ));
        }

        let target = staging_root.join(&entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create target directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        fs::copy(&source, &target).map_err(|error| {
            format!(
                "Failed to copy runtime entry from {} to {}: {error}",
                source.display(),
                target.display()
            )
        })?;
        ensure_executable_bit(&target, entry.executable)?;

        copied_bytes = copied_bytes.saturating_add(entry.size);
        let progress = if total_bytes > 0 {
            25u8.saturating_add(((copied_bytes.saturating_mul(45)) / total_bytes).min(45) as u8)
        } else {
            25u8.saturating_add((((index + 1) as u64 * 45) / entries.len().max(1) as u64) as u8)
        };
        if progress >= last_progress.saturating_add(5) || index + 1 == entries.len() {
            last_progress = progress;
            if let Some(callback) = on_progress.as_deref_mut() {
                emit_bootstrap_progress(
                    callback,
                    RuntimeOperationStage::Hydrating,
                    "Copiando archivos del runtime local",
                    Some(progress.min(70)),
                    Some(copied_bytes),
                    Some(total_bytes),
                    true,
                );
            }
        }
    }

    fs::write(
        staging_root.join("manifest.json"),
        serde_json::to_vec_pretty(manifest)
            .map_err(|error| format!("Failed to serialize hydrated runtime manifest: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "Failed to persist hydrated runtime manifest at {}: {error}",
            staging_root.join("manifest.json").display()
        )
    })?;

    write_stage_marker(app_data_dir, &manifest.pack_version, "promoting")?;
    if let Some(callback) = on_progress {
        emit_bootstrap_progress(
            callback,
            RuntimeOperationStage::Activating,
            "Promoviendo runtime hidratado",
            Some(72),
            Some(copied_bytes),
            Some(total_bytes),
            true,
        );
    }
    if let Err(error) = promote_local_staged_runtime(app_data_dir, manifest, &staging_root) {
        cleanup_local_incomplete_activation(app_data_dir, &manifest.pack_version, &staging_root);
        return Err(error);
    }

    let stage_marker = stage_marker_path(app_data_dir, &manifest.pack_version);
    if stage_marker.exists() {
        fs::remove_file(&stage_marker).map_err(|error| {
            format!(
                "Failed to remove stage marker {}: {error}",
                stage_marker.display()
            )
        })?;
    }

    Ok(())
}

fn cleanup_local_incomplete_activation(
    app_data_dir: &Path,
    pack_version: &str,
    staging_root: &Path,
) {
    let stage_marker = stage_marker_path(app_data_dir, pack_version);
    if stage_marker.exists() {
        let _ = fs::remove_file(stage_marker);
    }
    if staging_root.exists() {
        let _ = fs::remove_dir_all(staging_root);
    }
}

fn promote_local_staged_runtime(
    app_data_dir: &Path,
    manifest: &RuntimeManifest,
    staging_root: &Path,
) -> Result<PathBuf, String> {
    let managed_root = managed_pack_dir(app_data_dir, &manifest.pack_version);
    if existing_managed_runtime_matches_manifest(&managed_root, manifest) {
        fs::remove_dir_all(staging_root).map_err(|error| {
            format!(
                "Failed to remove redundant staging runtime {}: {error}",
                staging_root.display()
            )
        })?;
        return Ok(managed_root);
    }

    let backup_root = managed_replacement_backup_path(app_data_dir, &manifest.pack_version);
    let had_existing = managed_root.exists();
    if had_existing {
        fs::rename(&managed_root, &backup_root).map_err(|error| {
            format!(
                "Failed to move previous managed runtime {} to backup {} before activation: {error}",
                managed_root.display(),
                backup_root.display()
            )
        })?;
    }

    if let Err(error) = fs::rename(staging_root, &managed_root) {
        if had_existing && backup_root.exists() {
            let _ = fs::rename(&backup_root, &managed_root);
        }
        return Err(format!(
            "Failed to promote runtime from {} to {}: {error}",
            staging_root.display(),
            managed_root.display()
        ));
    }

    let promoted_status = inspect_runtime(&managed_root, app_data_dir, manifest)?;
    if promoted_status.state != RuntimeState::Healthy {
        if had_existing && backup_root.exists() {
            let _ = fs::remove_dir_all(&managed_root);
            let _ = fs::rename(&backup_root, &managed_root);
        }
        return Err(format!(
            "Promoted runtime verification failed with state {:?}",
            promoted_status.state
        ));
    }

    if had_existing && backup_root.exists() {
        let _ = fs::remove_dir_all(&backup_root);
    }

    Ok(managed_root)
}

fn existing_managed_runtime_matches_manifest(
    managed_root: &Path,
    expected: &RuntimeManifest,
) -> bool {
    let Ok(manifest) = RuntimeManifest::load_from_path(&managed_root.join("manifest.json")) else {
        return false;
    };
    if manifest.pack_version != expected.pack_version
        || manifest.app_version != expected.app_version
        || manifest.platform != expected.platform
    {
        return false;
    }

    inspect_runtime(
        managed_root,
        managed_root
            .parent()
            .and_then(Path::parent)
            .unwrap_or(managed_root),
        &manifest,
    )
    .map(|status| status.state == RuntimeState::Healthy)
    .unwrap_or(false)
}

fn managed_replacement_backup_path(app_data_dir: &Path, pack_version: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    runtime_root(app_data_dir).join(format!(".{pack_version}.replacing-{millis}"))
}

fn write_stage_marker(app_data_dir: &Path, pack_version: &str, stage: &str) -> Result<(), String> {
    let marker_path = stage_marker_path(app_data_dir, pack_version);
    fs::write(
        &marker_path,
        serde_json::to_vec_pretty(&json!({ "pack_version": pack_version, "stage": stage }))
            .map_err(|error| format!("Failed to serialize stage marker: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "Failed to write stage marker {}: {error}",
            marker_path.display()
        )
    })
}

fn emit_bootstrap_progress<F>(
    on_progress: &mut F,
    stage: RuntimeOperationStage,
    summary: &str,
    progress_percent: Option<u8>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    retryable: bool,
) where
    F: FnMut(RuntimeOperation),
{
    on_progress(RuntimeOperation {
        kind: RuntimeOperationKind::Bootstrap,
        stage,
        summary: summary.to_string(),
        progress_percent,
        downloaded_bytes,
        total_bytes,
        retryable,
    });
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {} for checksum: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Recompute the SHA-256 of the manifest's CRITICAL entries (python/uv
/// launchers + native assets) under `managed_root` and return a Spanish-language
/// description for each one whose content does not match the manifest digest.
///
/// Entries that are missing or already size-mismatched are skipped here: the
/// caller's presence/size loop already reports those, so we avoid duplicate
/// noise and avoid re-reading a file that is obviously wrong. Read failures are
/// surfaced as integrity failures rather than aborting the whole inspection.
fn critical_entry_integrity_failures(
    managed_root: &Path,
    manifest: &RuntimeManifest,
) -> Vec<String> {
    let mut failures = Vec::new();
    for entry in manifest.critical_entries() {
        let target = managed_root.join(&entry.path);
        match fs::metadata(&target) {
            // Missing or wrong-sized: already reported by the presence/size loop.
            Err(_) => continue,
            Ok(metadata) if metadata.len() != entry.size => continue,
            Ok(_) => {}
        }
        match file_sha256(&target) {
            Ok(actual_sha256) if actual_sha256 == entry.sha256 => {}
            Ok(actual_sha256) => failures.push(format!(
                "Checksum inválido en {} (esperado {}, detectado {})",
                entry.path, entry.sha256, actual_sha256
            )),
            Err(error) => failures.push(format!(
                "No se pudo verificar el checksum de {}: {error}",
                entry.path
            )),
        }
    }
    failures
}

fn healthy_status(pack_version: &str) -> RuntimeStatus {
    RuntimeStatus {
        state: RuntimeState::Healthy,
        pack_version: Some(pack_version.to_string()),
        repair_needed: false,
        repair_available: true,
        summary: "Runtime listo".to_string(),
        blocked_capabilities: vec![],
        details: vec![],
        guidance: vec![],
        bootstrap_eligible: false,
        bootstrap_required: false,
        active_operation: None,
    }
}

fn classify_unavailable_bootstrap_plan<F>(
    manifest: &RuntimeManifest,
    plan: BootstrapPlan,
    on_progress: &mut F,
) -> RuntimeStatus
where
    F: FnMut(RuntimeOperation),
{
    let reason = plan
        .reason
        .clone()
        .unwrap_or_else(|| "Bootstrap source unavailable".to_string());
    let offline = reason.to_ascii_lowercase().contains("offline");
    let state = if offline {
        RuntimeState::BlockedOffline
    } else {
        RuntimeState::BlockedSourceUnavailable
    };
    let message = if offline {
        "Bootstrap bloqueado por falta de conectividad"
    } else {
        "Bootstrap bloqueado por falta de una fuente confiable"
    };
    emit_bootstrap_progress(
        on_progress,
        RuntimeOperationStage::Blocked,
        message,
        None,
        None,
        plan.download.as_ref().map(|download| download.archive_size),
        true,
    );
    blocked_bootstrap_status(manifest, state, plan, reason, true)
}

fn classify_bootstrap_failure(manifest: &RuntimeManifest, error: String) -> RuntimeStatus {
    let lower = error.to_ascii_lowercase();
    let retryable =
        !(lower.contains("checksum") || lower.contains("signature") || lower.contains("integrity"));
    RuntimeStatus {
        state: RuntimeState::Damaged,
        pack_version: Some(manifest.pack_version.clone()),
        repair_needed: true,
        repair_available: true,
        summary: "Runtime bloqueado por fallo de integridad durante bootstrap".to_string(),
        blocked_capabilities: blocked_capabilities(),
        details: vec![error],
        guidance: vec![if retryable {
            "Reintentá el bootstrap o la reparación cuando la fuente vuelva a estar disponible."
                .to_string()
        } else {
            "Se detectó un problema de integridad/trust. No se activó ningún runtime parcial."
                .to_string()
        }],
        bootstrap_eligible: retryable,
        bootstrap_required: true,
        active_operation: Some(RuntimeOperation {
            kind: RuntimeOperationKind::Bootstrap,
            stage: RuntimeOperationStage::Blocked,
            summary: "Bootstrap detenido por fallo de integridad".to_string(),
            progress_percent: None,
            downloaded_bytes: None,
            total_bytes: None,
            retryable,
        }),
    }
}

fn blocked_bootstrap_status(
    manifest: &RuntimeManifest,
    state: RuntimeState,
    plan: BootstrapPlan,
    detail: String,
    retryable: bool,
) -> RuntimeStatus {
    let summary = match state {
        RuntimeState::BlockedOffline => {
            "No se pudo continuar el bootstrap porque la fuente confiable está offline"
        }
        RuntimeState::BlockedSourceUnavailable => {
            "No hay una fuente confiable disponible para bootstrap"
        }
        _ => "Bootstrap bloqueado",
    };

    RuntimeStatus {
        state,
        pack_version: Some(manifest.pack_version.clone()),
        repair_needed: false,
        repair_available: false,
        summary: summary.to_string(),
        blocked_capabilities: blocked_capabilities(),
        details: vec![detail],
        guidance: vec![
            "Reintentá cuando cambien las condiciones o se publique una fuente válida.".to_string(),
        ],
        bootstrap_eligible: plan.eligible,
        bootstrap_required: true,
        active_operation: Some(RuntimeOperation {
            kind: RuntimeOperationKind::Bootstrap,
            stage: RuntimeOperationStage::Blocked,
            summary: plan.summary,
            progress_percent: None,
            downloaded_bytes: None,
            total_bytes: plan.download.as_ref().map(|download| download.archive_size),
            retryable,
        }),
    }
}

fn incompatible_missing_manifest_status(error: String) -> RuntimeStatus {
    RuntimeStatus {
        state: RuntimeState::Incompatible,
        pack_version: None,
        repair_needed: false,
        repair_available: false,
        summary: "Runtime pack incompatible o incompleto".to_string(),
        blocked_capabilities: blocked_capabilities(),
        details: vec![error],
        guidance: vec![
            "Verificá que exista resources/runtime-pack/<platform>/manifest.json para esta plataforma.".to_string(),
        ],
        bootstrap_eligible: false,
        bootstrap_required: false,
        active_operation: None,
    }
}

fn compatibility_status(manifest: &RuntimeManifest) -> Option<RuntimeStatus> {
    if manifest.app_version != running_app_version() {
        return Some(app_version_incompatible_status(
            &manifest.app_version,
            &manifest.pack_version,
        ));
    }

    if manifest.platform != current_runtime_platform() {
        return Some(platform_incompatible_status(
            &manifest.platform,
            &manifest.pack_version,
        ));
    }

    if manifest.payload_profile == "release"
        && !manifest.release_injection_required
        && manifest.external_artifacts_required.is_empty()
    {
        return None;
    }

    let bootstrap_plan = BootstrapController::new().plan(
        &RuntimeStatus {
            state: RuntimeState::Fixture,
            pack_version: Some(manifest.pack_version.clone()),
            repair_needed: false,
            repair_available: false,
            summary: "Runtime fixture".to_string(),
            blocked_capabilities: blocked_capabilities(),
            details: vec![],
            guidance: vec![],
            bootstrap_eligible: false,
            bootstrap_required: true,
            active_operation: None,
        },
        manifest,
        Path::new("."),
        BootstrapRemoteCatalog::SourceUnavailable {
            source: None,
            reason: "Trusted remote bootstrap source wiring is not implemented yet".to_string(),
        },
    );

    Some(RuntimeStatus {
        state: RuntimeState::Fixture,
        pack_version: Some(manifest.pack_version.clone()),
        repair_needed: false,
        repair_available: false,
        summary: format!(
            "Runtime de release pendiente para {} (modo desarrollo activo)",
            manifest.platform
        ),
        blocked_capabilities: blocked_capabilities(),
        details: std::iter::once(format!(
            "La app {} arrancó correctamente, pero este runtime-pack todavía está en modo fixture/dev (app_version declarada: {}).",
            running_app_version(),
            manifest.app_version
        ))
        .chain(
            manifest
                .external_artifacts_required
                .iter()
                .map(|artifact| format!("Payload externo pendiente antes de hidratar: {artifact}")),
        )
        .collect(),
        guidance: fixture_guidance(manifest),
        bootstrap_eligible: bootstrap_plan.eligible,
        bootstrap_required: bootstrap_plan.required,
        active_operation: bootstrap_operation_from_plan(&bootstrap_plan),
    })
}

fn platform_incompatible_status(platform: &str, pack_version: &str) -> RuntimeStatus {
    RuntimeStatus {
        state: RuntimeState::Incompatible,
        pack_version: Some(pack_version.to_string()),
        repair_needed: false,
        repair_available: false,
        summary: format!("Runtime incompatible para {platform}"),
        blocked_capabilities: blocked_capabilities(),
        details: vec![format!(
            "El pack declara {platform} pero la app corre en {}",
            current_runtime_platform()
        )],
        guidance: vec![
            "Usá un runtime-pack generado para la misma plataforma/arquitectura que la app en ejecución.".to_string(),
        ],
        bootstrap_eligible: false,
        bootstrap_required: false,
        active_operation: None,
    }
}

fn app_version_incompatible_status(app_version: &str, pack_version: &str) -> RuntimeStatus {
    RuntimeStatus {
        state: RuntimeState::Incompatible,
        pack_version: Some(pack_version.to_string()),
        repair_needed: false,
        repair_available: false,
        summary: format!(
            "Runtime incompatible con EntropIA {}",
            running_app_version()
        ),
        blocked_capabilities: blocked_capabilities(),
        details: vec![format!(
            "El runtime-pack {} declara app_version {} pero la app en ejecución usa {}",
            pack_version,
            app_version,
            running_app_version()
        )],
        guidance: vec![
            "Regenerá o seleccioná un runtime-pack compatible con la versión actual de EntropIA."
                .to_string(),
        ],
        bootstrap_eligible: false,
        bootstrap_required: false,
        active_operation: None,
    }
}

fn fixture_guidance(manifest: &RuntimeManifest) -> Vec<String> {
    let mut guidance = vec![
        "Esto no indica una caída: en dev podés seguir con dependencias locales/fallback mientras el runtime-pack de release queda pendiente.".to_string(),
    ];

    if manifest.release_injection_required {
        guidance.push(
            "Detalle técnico de release: el runtime-pack final todavía requiere artefactos externos antes de distribuirse.".to_string(),
        );
    }

    if !manifest.external_artifacts_required.is_empty() {
        guidance.push(format!(
            "Payloads pendientes declarados por el manifest: {}.",
            manifest.external_artifacts_required.join(", ")
        ));
    }

    guidance
}

fn running_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn blocked_capabilities() -> Vec<RuntimeCapability> {
    vec![
        RuntimeCapability::Ocr,
        RuntimeCapability::Transcription,
        RuntimeCapability::Nlp,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bootstrap::{
        BootstrapPlanSource, BootstrapRemoteCatalog, BootstrapRemoteSource,
    };
    use crate::runtime::download::test_support::{build_signed_release, runtime_archive_bytes};
    use crate::runtime::manifest::ManifestEntry;
    use base64::Engine;
    use tempfile::tempdir;

    #[test]
    fn configured_bootstrap_catalog_returns_source_unavailable_when_not_configured() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("create app_settings");

        let source = crate::settings::get_runtime_bootstrap_remote_source(&conn)
            .expect("lookup should succeed");

        let catalog = match source {
            Some(source) => BootstrapRemoteCatalog::SourceUnavailable {
                source: Some(source),
                reason: "Trusted remote bootstrap source is configured, but remote manifest fetch is not implemented yet"
                    .to_string(),
            },
            None => BootstrapRemoteCatalog::SourceUnavailable {
                source: None,
                reason: "Trusted remote bootstrap source is not configured in this environment"
                    .to_string(),
            },
        };

        assert_eq!(
            catalog,
            BootstrapRemoteCatalog::SourceUnavailable {
                source: None,
                reason: "Trusted remote bootstrap source is not configured in this environment"
                    .to_string(),
            }
        );
    }

    #[test]
    fn configured_bootstrap_catalog_preserves_configured_remote_source_details() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("create app_settings");
        crate::settings::set_setting(
            &conn,
            crate::settings::RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY,
            "https://example.com/runtime/bootstrap.json",
        )
        .expect("save manifest url");
        crate::settings::set_setting(
            &conn,
            crate::settings::RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY,
            "entropia-root",
        )
        .expect("save public key id");

        let source = crate::settings::get_runtime_bootstrap_remote_source(&conn)
            .expect("lookup should succeed")
            .expect("source should be configured");

        let catalog = BootstrapRemoteCatalog::SourceUnavailable {
            source: Some(source),
            reason:
                "Trusted remote bootstrap source is configured, but remote manifest fetch is not implemented yet"
                    .to_string(),
        };

        assert_eq!(
            catalog,
            BootstrapRemoteCatalog::SourceUnavailable {
                source: Some(BootstrapRemoteSource {
                    manifest_url: "https://example.com/runtime/bootstrap.json".to_string(),
                    public_key_id: "entropia-root".to_string(),
                }),
                reason:
                    "Trusted remote bootstrap source is configured, but remote manifest fetch is not implemented yet"
                        .to_string(),
            }
        );
    }

    fn write_file(root: &Path, relpath: &str, bytes: &[u8]) -> String {
        let path = root.join(relpath);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, bytes).expect("write file");
        format!("{:x}", Sha256::digest(bytes))
    }

    fn sample_manifest(platform: &str, python_sha: &str, uv_sha: &str) -> RuntimeManifest {
        RuntimeManifest {
            pack_version: "2026.05.0".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform.to_string(),
            payload_profile: "release".to_string(),
            release_injection_required: false,
            external_artifacts_required: vec![],
            python_relpath: if cfg!(windows) {
                "python/python.exe".to_string()
            } else {
                "python/bin/python3".to_string()
            },
            uv_relpath: if cfg!(windows) {
                "uv/uv.exe".to_string()
            } else {
                "uv/bin/uv".to_string()
            },
            python_files: vec![ManifestEntry {
                path: if cfg!(windows) {
                    "python/python.exe".to_string()
                } else {
                    "python/bin/python3".to_string()
                },
                sha256: python_sha.to_string(),
                size: 6,
                executable: !cfg!(windows),
            }],
            uv_files: vec![ManifestEntry {
                path: if cfg!(windows) {
                    "uv/uv.exe".to_string()
                } else {
                    "uv/bin/uv".to_string()
                },
                sha256: uv_sha.to_string(),
                size: 2,
                executable: true,
            }],
            script_files: vec![],
            wheelhouse: vec![],
            caches: vec![],
            native_assets: vec![],
        }
    }

    fn write_manifest(root: &Path, manifest: &RuntimeManifest) {
        fs::create_dir_all(root).expect("create manifest root");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
    }

    #[test]
    fn hydrates_missing_runtime_from_bundled_pack() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        write_manifest(
            bundle_dir.path(),
            &sample_manifest(
                &crate::runtime::paths::current_runtime_platform(),
                &python_sha,
                &uv_sha,
            ),
        );

        let manager = RuntimeManager::new();
        let status = manager
            .ensure_ready_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("runtime should hydrate");

        assert_eq!(status.state, RuntimeState::Healthy);
        assert_eq!(status.pack_version.as_deref(), Some("2026.05.0"));
        assert_eq!(status.blocked_capabilities, Vec::<RuntimeCapability>::new());
        let managed_python = app_data_dir
            .path()
            .join("runtime")
            .join("2026.05.0")
            .join(python_relpath);
        assert!(
            managed_python.is_file(),
            "expected hydrated python at {}",
            managed_python.display()
        );
    }

    #[test]
    fn plans_trusted_remote_bootstrap_when_fixture_bundle_cannot_hydrate() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let plan = manager
            .plan_bootstrap_for_tests(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::Available {
                    source: BootstrapRemoteSource {
                        manifest_url: "https://example.com/bootstrap.json".to_string(),
                        public_key_id: "entropia-root".to_string(),
                    },
                    index: crate::runtime::manifest::BootstrapManifestIndex {
                        channel: "stable".to_string(),
                        generated_at: "2026-05-06T00:00:00Z".to_string(),
                        releases: vec![crate::runtime::manifest::BootstrapReleaseManifest {
                            app_version: env!("CARGO_PKG_VERSION").to_string(),
                            platform: crate::runtime::paths::current_runtime_platform(),
                            pack_version: "2026.05.1".to_string(),
                            archive_url: "https://example.com/runtime-pack.archive".to_string(),
                            additional_part_urls: Vec::new(),
                            archive_sha256: "remote-sha".to_string(),
                            archive_size: 1024,
                            signature: "signed".to_string(),
                        }],
                    },
                },
            )
            .expect("remote bootstrap plan should be created");

        assert_eq!(plan.source, Some(BootstrapPlanSource::TrustedRemote));
        assert!(plan.eligible);
        assert!(plan.required);
        assert_eq!(plan.pack_version.as_deref(), Some("2026.05.1"));
    }

    #[test]
    fn blocks_bootstrap_when_remote_source_is_unavailable() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let plan = manager
            .plan_bootstrap_for_tests(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::SourceUnavailable {
                    source: Some(BootstrapRemoteSource {
                        manifest_url: "https://example.com/bootstrap.json".to_string(),
                        public_key_id: "entropia-root".to_string(),
                    }),
                    reason: "manifest fetch pending implementation".to_string(),
                },
            )
            .expect("blocked bootstrap plan should still be returned");

        assert_eq!(plan.source, None);
        assert!(plan.required);
        assert!(!plan.eligible);
        assert!(plan.reason.unwrap_or_default().contains("pending"));
    }

    #[test]
    fn ensure_ready_or_bootstrap_hydrates_release_bundle_and_reports_progress() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        write_manifest(
            bundle_dir.path(),
            &sample_manifest(
                &crate::runtime::paths::current_runtime_platform(),
                &python_sha,
                &uv_sha,
            ),
        );

        let manager = RuntimeManager::new();
        let mut progress_events = Vec::new();
        let status = manager
            .ensure_ready_or_bootstrap_for_tests(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::NotConfigured,
                |operation| progress_events.push(operation),
            )
            .expect("release bundle should hydrate successfully");

        assert_eq!(status.state, RuntimeState::Healthy);
        assert!(progress_events
            .iter()
            .any(|operation| operation.stage == RuntimeOperationStage::Checking));
        assert!(progress_events
            .iter()
            .any(|operation| operation.stage == RuntimeOperationStage::Hydrating));
        assert!(progress_events
            .iter()
            .any(|operation| operation.stage == RuntimeOperationStage::Verifying));
        assert!(progress_events
            .iter()
            .any(|operation| operation.stage == RuntimeOperationStage::Activating));
    }

    #[test]
    fn ensure_ready_or_bootstrap_blocks_fixture_runtime_when_no_source_exists() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .ensure_ready_or_bootstrap_for_tests(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::SourceUnavailable {
                    source: None,
                    reason: "manifest not published".to_string(),
                },
                |_| {},
            )
            .expect("blocked status should be returned");

        assert_eq!(status.state, RuntimeState::BlockedSourceUnavailable);
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("manifest not published")));
        assert!(status.bootstrap_required);
    }

    #[test]
    fn ensure_ready_or_bootstrap_activates_remote_runtime_when_trusted_release_is_available() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let manager = RuntimeManager::new();
        let mut progress_events = Vec::new();

        let status = manager
            .ensure_ready_or_bootstrap_for_tests_with_remote_support(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::Available {
                    source: BootstrapRemoteSource {
                        manifest_url: "https://example.com/bootstrap.json".to_string(),
                        public_key_id: "entropia-root".to_string(),
                    },
                    index: crate::runtime::manifest::BootstrapManifestIndex {
                        channel: "stable".to_string(),
                        generated_at: "2026-05-06T00:00:00Z".to_string(),
                        releases: vec![release],
                    },
                },
                |_| Ok(public_key.clone()),
                |source_manifest_url, release, app_data_dir, public_key_base64, on_progress| {
                    crate::runtime::download::download_and_activate_remote_runtime_with_fetch(
                        source_manifest_url,
                        release,
                        app_data_dir,
                        public_key_base64,
                        |_| Ok(std::io::Cursor::new(archive_bytes.clone())),
                        on_progress,
                    )
                },
                |operation| progress_events.push(operation),
            )
            .expect("trusted remote runtime should activate");

        assert_eq!(status.state, RuntimeState::Healthy);
        assert!(app_data_dir
            .path()
            .join("runtime")
            .join("2026.05.1")
            .join("manifest.json")
            .is_file());
        assert!(progress_events
            .iter()
            .any(|operation| operation.stage == RuntimeOperationStage::Downloading));
        assert!(progress_events
            .iter()
            .any(|operation| operation.stage == RuntimeOperationStage::Activating));
    }

    #[test]
    fn ensure_ready_or_bootstrap_reuses_existing_hydrated_runtime_before_remote_download() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let manager = RuntimeManager::new();
        manager
            .ensure_ready_or_bootstrap_for_tests_with_remote_support(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::Available {
                    source: BootstrapRemoteSource {
                        manifest_url: "https://example.com/bootstrap.json".to_string(),
                        public_key_id: "entropia-root".to_string(),
                    },
                    index: crate::runtime::manifest::BootstrapManifestIndex {
                        channel: "stable".to_string(),
                        generated_at: "2026-05-06T00:00:00Z".to_string(),
                        releases: vec![release.clone()],
                    },
                },
                |_| Ok(public_key.clone()),
                |source_manifest_url, release, app_data_dir, public_key_base64, on_progress| {
                    crate::runtime::download::download_and_activate_remote_runtime_with_fetch(
                        source_manifest_url,
                        release,
                        app_data_dir,
                        public_key_base64,
                        |_| Ok(std::io::Cursor::new(archive_bytes.clone())),
                        on_progress,
                    )
                },
                |_| {},
            )
            .expect("initial remote bootstrap should activate");

        let downloads = std::sync::atomic::AtomicUsize::new(0);
        let status = manager
            .ensure_ready_or_bootstrap_for_tests_with_remote_support(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::Available {
                    source: BootstrapRemoteSource {
                        manifest_url: "https://example.com/bootstrap.json".to_string(),
                        public_key_id: "entropia-root".to_string(),
                    },
                    index: crate::runtime::manifest::BootstrapManifestIndex {
                        channel: "stable".to_string(),
                        generated_at: "2026-05-06T00:00:00Z".to_string(),
                        releases: vec![release],
                    },
                },
                |_| Ok(public_key.clone()),
                |_source_manifest_url,
                 _release,
                 _app_data_dir,
                 _public_key_base64,
                 _on_progress| {
                    downloads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err("download should not be called".to_string())
                },
                |_| {},
            )
            .expect("existing hydrated runtime should be reused");

        assert_eq!(status.state, RuntimeState::Healthy);
        assert_eq!(downloads.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn ensure_ready_or_bootstrap_reports_remote_integrity_failures_honestly() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let archive_bytes = runtime_archive_bytes();
        let (mut release, public_key) = build_signed_release(&archive_bytes);
        release.archive_sha256 = "bad-sha".to_string();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[11u8; 32]);
        let signature =
            ed25519_dalek::Signer::sign(&signing_key, release.signature_payload().as_bytes());
        release.signature = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
        let manager = RuntimeManager::new();

        let status = manager
            .ensure_ready_or_bootstrap_for_tests_with_remote_support(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::Available {
                    source: BootstrapRemoteSource {
                        manifest_url: "https://example.com/bootstrap.json".to_string(),
                        public_key_id: "entropia-root".to_string(),
                    },
                    index: crate::runtime::manifest::BootstrapManifestIndex {
                        channel: "stable".to_string(),
                        generated_at: "2026-05-06T00:00:00Z".to_string(),
                        releases: vec![release],
                    },
                },
                |_| Ok(public_key.clone()),
                |source_manifest_url, release, app_data_dir, public_key_base64, on_progress| {
                    crate::runtime::download::download_and_activate_remote_runtime_with_fetch(
                        source_manifest_url,
                        release,
                        app_data_dir,
                        public_key_base64,
                        |_| Ok(std::io::Cursor::new(archive_bytes.clone())),
                        on_progress,
                    )
                },
                |_| {},
            )
            .expect("integrity failure should return status");

        assert_eq!(status.state, RuntimeState::Damaged);
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("checksum mismatch")));
        assert_eq!(
            status
                .active_operation
                .as_ref()
                .map(|operation| operation.retryable),
            Some(false)
        );
    }

    #[test]
    fn ensure_ready_or_bootstrap_classifies_offline_blockers_honestly() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .ensure_ready_or_bootstrap_for_tests(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::SourceUnavailable {
                    source: None,
                    reason: "offline: trusted bootstrap source unreachable".to_string(),
                },
                |_| {},
            )
            .expect("offline blocker should be returned");

        assert_eq!(status.state, RuntimeState::BlockedOffline);
        assert!(status
            .guidance
            .iter()
            .any(|item| item.contains("Reintentá")));
    }

    #[test]
    fn ensure_ready_or_bootstrap_rejects_release_bundle_with_checksum_mismatch() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        write_file(bundle_dir.path(), python_relpath, b"python");
        write_file(bundle_dir.path(), uv_relpath, b"uv");
        write_manifest(
            bundle_dir.path(),
            &sample_manifest(
                &crate::runtime::paths::current_runtime_platform(),
                "bad-sha",
                "also-bad",
            ),
        );

        let manager = RuntimeManager::new();
        let status = manager
            .ensure_ready_or_bootstrap_for_tests(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::NotConfigured,
                |_| {},
            )
            .expect("integrity failure should be returned as status");

        assert_eq!(status.state, RuntimeState::Damaged);
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("checksum")));
        assert_eq!(
            status
                .active_operation
                .as_ref()
                .map(|operation| operation.retryable),
            Some(false)
        );
    }

    #[test]
    fn repairs_corrupt_managed_runtime_using_local_payload() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        write_manifest(bundle_dir.path(), &manifest);

        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        write_manifest(&managed_root, &manifest);
        write_file(&managed_root, python_relpath, b"broken-runtime");
        write_file(&managed_root, uv_relpath, b"uv");

        let manager = RuntimeManager::new();
        let status = manager
            .repair_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("runtime should repair");

        assert_eq!(status.state, RuntimeState::Healthy);
        assert_eq!(
            fs::read(
                app_data_dir
                    .path()
                    .join("runtime")
                    .join("2026.05.0")
                    .join(python_relpath)
            )
            .expect("read repaired python"),
            b"python"
        );
    }

    #[test]
    fn ensure_ready_invalidates_stale_managed_runtime_before_rehydrating() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let current_manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        write_manifest(bundle_dir.path(), &current_manifest);

        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        let mut stale_manifest = current_manifest.clone();
        stale_manifest.app_version = "0.0.11".to_string();
        write_manifest(&managed_root, &stale_manifest);
        write_file(&managed_root, python_relpath, b"stale-python");
        write_file(&managed_root, uv_relpath, b"uv");

        let manager = RuntimeManager::new();
        let status = manager
            .ensure_ready_or_bootstrap_for_tests(
                bundle_dir.path(),
                app_data_dir.path(),
                BootstrapRemoteCatalog::NotConfigured,
                |_| {},
            )
            .expect("stale runtime should be invalidated and rehydrated");

        assert_eq!(status.state, RuntimeState::Healthy);
        let repaired_manifest =
            RuntimeManifest::load_from_path(&managed_root.join("manifest.json"))
                .expect("rehydrated manifest");
        assert_eq!(repaired_manifest.app_version, running_app_version());
        assert_eq!(
            fs::read(managed_root.join(python_relpath)).expect("read rehydrated python"),
            b"python"
        );
    }

    #[test]
    fn generated_dev_runtime_root_must_match_running_app_version() {
        let bundle_dir = tempdir().expect("bundle dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut stale_manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        stale_manifest.app_version = "0.0.11".to_string();
        write_manifest(bundle_dir.path(), &stale_manifest);

        assert!(!runtime_root_matches_current_app(
            bundle_dir.path(),
            &crate::runtime::paths::current_runtime_platform()
        ));

        stale_manifest.app_version = running_app_version().to_string();
        write_manifest(bundle_dir.path(), &stale_manifest);

        assert!(runtime_root_matches_current_app(
            bundle_dir.path(),
            &crate::runtime::paths::current_runtime_platform()
        ));
    }

    #[test]
    fn hydrates_runtime_scripts_listed_in_manifest() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let script_relpath = "scripts/transcribe.py";
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let script_sha = write_file(bundle_dir.path(), script_relpath, b"print('fixture')\n");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.script_files = vec![ManifestEntry {
            path: script_relpath.to_string(),
            sha256: script_sha,
            size: 17,
            executable: false,
        }];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .ensure_ready_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("runtime should hydrate");

        assert_eq!(status.state, RuntimeState::Healthy);
        assert!(app_data_dir
            .path()
            .join("runtime")
            .join("2026.05.0")
            .join(script_relpath)
            .is_file());
    }

    #[test]
    fn marks_runtime_incompatible_when_bundle_platform_mismatches() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let wrong_platform =
            if crate::runtime::paths::current_runtime_platform() == "windows-x86_64" {
                "linux-x86_64"
            } else {
                "windows-x86_64"
            };
        write_manifest(
            bundle_dir.path(),
            &sample_manifest(wrong_platform, &python_sha, &uv_sha),
        );

        let manager = RuntimeManager::new();
        let status = manager
            .ensure_ready_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("status should be returned");

        assert_eq!(status.state, RuntimeState::Incompatible);
        assert!(!status.repair_available);
        assert_eq!(
            status.blocked_capabilities,
            vec![
                RuntimeCapability::Ocr,
                RuntimeCapability::Transcription,
                RuntimeCapability::Nlp,
            ]
        );
    }

    #[test]
    fn reports_fixture_pack_as_incompatible_until_release_artifacts_are_injected() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .status_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("fixture packs should produce status");

        assert_eq!(status.state, RuntimeState::Fixture);
        assert!(status.summary.contains("desarrollo") || status.summary.contains("fixture"));
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("relocatable-python")));
        assert!(status
            .guidance
            .iter()
            .any(|item| item.contains("no indica una caída")));
    }

    #[test]
    fn status_prefers_existing_hydrated_release_over_fixture_bundle() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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

        let fixture_python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let fixture_uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut fixture_manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &fixture_python_sha,
            &fixture_uv_sha,
        );
        fixture_manifest.payload_profile = "fixture".to_string();
        fixture_manifest.release_injection_required = true;
        fixture_manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &fixture_manifest);

        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        let release_python_sha = write_file(&managed_root, python_relpath, b"python");
        let release_uv_sha = write_file(&managed_root, uv_relpath, b"uv");
        let release_manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &release_python_sha,
            &release_uv_sha,
        );
        write_manifest(&managed_root, &release_manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .status_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("status should prefer existing hydrated release runtime");

        assert_eq!(status.state, RuntimeState::Healthy);
        assert!(status.blocked_capabilities.is_empty());
    }

    #[test]
    fn repair_refuses_fixture_pack_and_does_not_hydrate_runtime() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .repair_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("fixture packs should return incompatible status on repair");

        assert_eq!(status.state, RuntimeState::Fixture);
        assert!(manager
            .discover_hydrated_runtime_root_for_tests(app_data_dir.path())
            .is_none());
    }

    #[test]
    fn hydrated_runtime_inspection_marks_fixture_payload_as_incompatible() {
        let app_data_dir = tempdir().expect("app data dir");
        let manager = RuntimeManager::new();
        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
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
        let python_sha = write_file(&managed_root, python_relpath, b"python");
        let uv_sha = write_file(&managed_root, uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(&managed_root, &manifest);

        let status = manager
            .inspect_hydrated_runtime_for_tests(app_data_dir.path(), &managed_root, &manifest)
            .expect("fixture managed root should still report a status");

        assert_eq!(status.state, RuntimeState::Fixture);
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("relocatable-python")));
    }

    #[test]
    fn reports_incompatible_status_when_manifest_targets_different_app_version() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.app_version = "9.9.9".to_string();
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .status_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("status should inspect app-version compatibility");

        assert_eq!(status.state, RuntimeState::Incompatible);
        assert!(status.summary.contains(running_app_version()));
        assert!(status.details.iter().any(|detail| detail.contains("9.9.9")));
    }

    #[test]
    fn rediscovery_ignores_fixture_runtime_roots_even_if_files_are_valid() {
        let app_data_dir = tempdir().expect("app data dir");
        let manager = RuntimeManager::new();
        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
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
        let python_sha = write_file(&managed_root, python_relpath, b"python");
        let uv_sha = write_file(&managed_root, uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "fixture".to_string();
        manifest.release_injection_required = true;
        manifest.external_artifacts_required = vec!["relocatable-python".to_string()];
        write_manifest(&managed_root, &manifest);

        let discovered = manager.discover_hydrated_runtime_root_for_tests(app_data_dir.path());

        assert_eq!(discovered, None);
    }

    #[test]
    fn release_pack_with_pending_external_artifacts_stays_blocked() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        manifest.payload_profile = "release".to_string();
        manifest.release_injection_required = false;
        manifest.external_artifacts_required = vec!["seeded-model-caches".to_string()];
        write_manifest(bundle_dir.path(), &manifest);

        let manager = RuntimeManager::new();
        let status = manager
            .status_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("release packs with pending artifacts should still be blocked");

        assert_eq!(status.state, RuntimeState::Fixture);
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("seeded-model-caches")));
    }

    #[test]
    fn reports_damaged_status_when_managed_runtime_is_corrupt() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        write_manifest(bundle_dir.path(), &manifest);

        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        write_manifest(&managed_root, &manifest);
        write_file(&managed_root, python_relpath, b"broken-runtime");
        write_file(&managed_root, uv_relpath, b"uv");

        let manager = RuntimeManager::new();
        let status = manager
            .status_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("status should inspect corruption");

        assert_eq!(status.state, RuntimeState::Damaged);
        assert!(status.repair_needed);
        assert!(status.repair_available);
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("Tamaño inválido")));
    }

    #[test]
    fn reports_damaged_status_when_critical_executable_content_is_corrupt_but_same_size() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
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
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        let manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        write_manifest(bundle_dir.path(), &manifest);

        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        write_manifest(&managed_root, &manifest);
        // Same length as b"python" (6 bytes) so the size check passes, but the
        // bytes differ so only a recomputed SHA-256 can catch the corruption.
        write_file(&managed_root, python_relpath, b"PYTHON");
        write_file(&managed_root, uv_relpath, b"uv");

        let manager = RuntimeManager::new();
        let status = manager
            .status_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("status should inspect critical-entry integrity");

        assert_eq!(status.state, RuntimeState::Damaged);
        assert!(status.repair_needed);
        assert!(status
            .details
            .iter()
            .any(|detail| detail.contains("Checksum inválido") && detail.contains(python_relpath)));
    }

    #[test]
    fn critical_entry_integrity_failures_skip_size_mismatches_and_flag_corruption() {
        let managed_root = tempdir().expect("managed root");
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
        let python_sha = write_file(managed_root.path(), python_relpath, b"python");
        let uv_sha = write_file(managed_root.path(), uv_relpath, b"uv");
        let mut manifest = sample_manifest(
            &crate::runtime::paths::current_runtime_platform(),
            &python_sha,
            &uv_sha,
        );
        // Force python's expected digest to disagree with the on-disk content
        // while keeping the declared size, so the size loop stays happy and only
        // the SHA recomputation rejects it.
        manifest.python_files[0].sha256 = "deadbeef".to_string();

        let failures = critical_entry_integrity_failures(managed_root.path(), &manifest);

        assert_eq!(failures.len(), 1, "only python should fail: {failures:?}");
        assert!(failures[0].contains("Checksum inválido"));
        assert!(failures[0].contains(python_relpath));
    }

    #[test]
    fn reports_incompatible_status_when_runtime_pack_manifest_is_missing() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");

        let manager = RuntimeManager::new();
        let status = manager
            .status_for_tests(bundle_dir.path(), app_data_dir.path())
            .expect("missing manifest should return status");

        assert_eq!(status.state, RuntimeState::Incompatible);
        assert!(!status.repair_available);
        assert!(status.summary.contains("pack"));
    }

    #[test]
    fn discovers_hydrated_runtime_root_from_app_data_without_bundle_resources() {
        let app_data_dir = tempdir().expect("app data dir");
        let manager = RuntimeManager::new();
        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
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
        let python_sha = write_file(&managed_root, python_relpath, b"python");
        let uv_sha = write_file(&managed_root, uv_relpath, b"uv");
        write_manifest(
            &managed_root,
            &sample_manifest(
                &crate::runtime::paths::current_runtime_platform(),
                &python_sha,
                &uv_sha,
            ),
        );

        let discovered = manager.discover_hydrated_runtime_root_for_tests(app_data_dir.path());

        assert_eq!(discovered, Some(managed_root));
    }

    #[test]
    fn ignores_incompatible_hydrated_runtime_roots_when_scanning_app_data() {
        let app_data_dir = tempdir().expect("app data dir");
        let manager = RuntimeManager::new();
        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
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
        let python_sha = write_file(&managed_root, python_relpath, b"python");
        let uv_sha = write_file(&managed_root, uv_relpath, b"uv");
        write_manifest(
            &managed_root,
            &sample_manifest("windows-x86_64", &python_sha, &uv_sha),
        );

        let discovered = manager.discover_hydrated_runtime_root_for_tests(app_data_dir.path());

        if crate::runtime::paths::current_runtime_platform() == "windows-x86_64" {
            assert_eq!(discovered, Some(managed_root));
        } else {
            assert_eq!(discovered, None);
        }
    }

    #[test]
    fn resolves_env_runtime_pack_override_as_platform_parent() {
        let root = tempdir().expect("override root");
        let platform = crate::runtime::paths::current_runtime_platform();
        let platform_root = root.path().join(&platform);
        fs::create_dir_all(&platform_root).expect("platform root");
        fs::write(platform_root.join("manifest.json"), "{}").expect("manifest");

        let resolved = resolve_env_bundle_root_from_value(
            &platform,
            Some(root.path().as_os_str().to_os_string()),
        )
        .expect("override should be valid");

        assert_eq!(resolved, Some(platform_root));
    }

    #[test]
    fn resolves_env_runtime_pack_override_as_direct_pack() {
        let root = tempdir().expect("override root");
        let platform = crate::runtime::paths::current_runtime_platform();
        fs::write(root.path().join("manifest.json"), "{}").expect("manifest");

        let resolved = resolve_env_bundle_root_from_value(
            &platform,
            Some(root.path().as_os_str().to_os_string()),
        )
        .expect("override should be valid");

        assert_eq!(
            resolved,
            Some(crate::path_utils::normalize_windows_path(root.path()))
        );
    }

    #[test]
    fn rejects_invalid_env_runtime_pack_override() {
        let root = tempdir().expect("override root");

        let error = resolve_env_bundle_root_from_value(
            "windows-x86_64",
            Some(root.path().as_os_str().to_os_string()),
        )
        .expect_err("invalid override should fail fast");

        assert!(error.contains(RUNTIME_PACK_ROOT_ENV));
        assert!(error.contains("windows-x86_64/manifest.json"));
    }

    #[test]
    fn bundled_runtime_pack_candidates_match_tauri_resource_layout() {
        let candidates = bundled_runtime_pack_resource_candidates("windows-x86_64");

        assert_eq!(
            candidates.first().map(String::as_str),
            Some("resources/runtime-pack/windows-x86_64")
        );
        assert!(candidates
            .iter()
            .any(|candidate| candidate == "runtime-pack/windows-x86_64"));
    }

    #[test]
    fn release_missing_bundle_error_does_not_leak_dev_manifest_dir() {
        let candidates = bundled_runtime_pack_resource_candidates("windows-x86_64");
        let error = missing_bundle_root_error("windows-x86_64", &candidates, None);

        assert!(error.contains("resources/runtime-pack/windows-x86_64"));
        assert!(!error.contains(env!("CARGO_MANIFEST_DIR")));
    }
}
