use crate::runtime::download::{bootstrap_download_plan_paths, DownloadPlanPaths};
use crate::runtime::manifest::{BootstrapManifestIndex, BootstrapReleaseManifest, RuntimeManifest};
use crate::runtime::status::{
    RuntimeOperation, RuntimeOperationKind, RuntimeOperationStage, RuntimeStatus,
};
use reqwest::blocking::Client;
use std::path::Path;
use std::time::Duration;

// Bootstrap CONTRACT types live in the always-compiled `bootstrap_types` module so the
// API-only (lite) build can construct/return them without this gated impl. Re-exported
// here so existing `crate::runtime::bootstrap::Bootstrap*` paths keep resolving.
pub use crate::runtime::bootstrap_types::{
    BootstrapDownloadPlan, BootstrapPlan, BootstrapPlanSource, BootstrapRemoteSource,
};

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BootstrapRemoteCatalog {
    NotConfigured,
    SourceUnavailable {
        source: Option<BootstrapRemoteSource>,
        reason: String,
    },
    Available {
        source: BootstrapRemoteSource,
        index: BootstrapManifestIndex,
    },
}

pub struct BootstrapController;

impl BootstrapController {
    pub fn new() -> Self {
        Self
    }

    pub fn plan(
        &self,
        bundle_status: &RuntimeStatus,
        bundle_manifest: &RuntimeManifest,
        app_data_dir: &Path,
        remote_catalog: BootstrapRemoteCatalog,
    ) -> BootstrapPlan {
        if bundle_status.state == crate::runtime::status::RuntimeState::Healthy {
            return BootstrapPlan {
                eligible: false,
                required: false,
                source: Some(BootstrapPlanSource::ManagedReady),
                pack_version: bundle_status.pack_version.clone(),
                summary: "El runtime ya está listo".to_string(),
                reason: None,
                remote_source: None,
                download: None,
            };
        }

        if bundle_manifest.payload_profile == "release"
            && !bundle_manifest.release_injection_required
            && bundle_manifest.external_artifacts_required.is_empty()
        {
            return BootstrapPlan {
                eligible: true,
                required: true,
                source: Some(BootstrapPlanSource::BundledRelease),
                pack_version: Some(bundle_manifest.pack_version.clone()),
                summary: "El runtime puede hidratarse desde el bundle local".to_string(),
                reason: None,
                remote_source: None,
                download: None,
            };
        }

        match remote_catalog {
            BootstrapRemoteCatalog::NotConfigured => BootstrapPlan {
                eligible: false,
                required: true,
                source: None,
                pack_version: bundle_status.pack_version.clone(),
                summary: "No hay canal remoto confiable configurado".to_string(),
                reason: Some("Trusted bootstrap source is not configured yet".to_string()),
                remote_source: None,
                download: None,
            },
            BootstrapRemoteCatalog::SourceUnavailable { source, reason } => BootstrapPlan {
                eligible: false,
                required: true,
                source: None,
                pack_version: bundle_status.pack_version.clone(),
                summary: "La fuente remota confiable no está disponible".to_string(),
                reason: Some(reason),
                remote_source: source,
                download: None,
            },
            BootstrapRemoteCatalog::Available { source, index } => {
                let Some(release) =
                    index.select_release(&bundle_manifest.app_version, &bundle_manifest.platform)
                else {
                    return BootstrapPlan {
                        eligible: false,
                        required: true,
                        source: None,
                        pack_version: bundle_status.pack_version.clone(),
                        summary: "No existe un artifact remoto compatible".to_string(),
                        reason: Some(format!(
                            "No bootstrap release matches app_version={} platform={}",
                            bundle_manifest.app_version, bundle_manifest.platform
                        )),
                        remote_source: Some(source),
                        download: None,
                    };
                };

                let paths = bootstrap_download_plan_paths(
                    app_data_dir,
                    &release.pack_version,
                    archive_file_name(release),
                );

                BootstrapPlan {
                    eligible: true,
                    required: true,
                    source: Some(BootstrapPlanSource::TrustedRemote),
                    pack_version: Some(release.pack_version.clone()),
                    summary: "EntropIA puede bootstrapear un runtime confiable".to_string(),
                    reason: None,
                    remote_source: Some(source),
                    download: Some(download_plan_from_release(release, &paths)),
                }
            }
        }
    }
}

pub fn fetch_remote_catalog(source: BootstrapRemoteSource) -> BootstrapRemoteCatalog {
    load_remote_catalog(source, |source| {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| format!("Failed to create bootstrap HTTP client: {error}"))?;
        let response = client
            .get(&source.manifest_url)
            .send()
            .map_err(|error| format!("Failed to fetch bootstrap manifest: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "Bootstrap manifest request failed with HTTP status {}",
                status
            ));
        }

        response
            .text()
            .map_err(|error| format!("Failed to read bootstrap manifest body: {error}"))
    })
}

pub fn load_remote_catalog<F>(
    source: BootstrapRemoteSource,
    fetch_manifest: F,
) -> BootstrapRemoteCatalog
where
    F: FnOnce(&BootstrapRemoteSource) -> Result<String, String>,
{
    match fetch_manifest(&source) {
        Ok(body) => match serde_json::from_str::<BootstrapManifestIndex>(&body) {
            Ok(index) => BootstrapRemoteCatalog::Available { source, index },
            Err(error) => BootstrapRemoteCatalog::SourceUnavailable {
                source: Some(source),
                reason: format!("Failed to parse bootstrap manifest index: {error}"),
            },
        },
        Err(reason) => BootstrapRemoteCatalog::SourceUnavailable {
            source: Some(source),
            reason,
        },
    }
}

pub fn bootstrap_operation_from_plan(plan: &BootstrapPlan) -> Option<RuntimeOperation> {
    if !plan.required {
        return None;
    }

    let (stage, retryable) = if plan.eligible {
        (RuntimeOperationStage::PlanningDownload, true)
    } else {
        (RuntimeOperationStage::Blocked, true)
    };

    Some(RuntimeOperation {
        kind: RuntimeOperationKind::Bootstrap,
        stage,
        summary: plan.summary.clone(),
        progress_percent: None,
        downloaded_bytes: None,
        total_bytes: plan.download.as_ref().map(|download| download.archive_size),
        retryable,
    })
}

fn download_plan_from_release(
    release: &BootstrapReleaseManifest,
    paths: &DownloadPlanPaths,
) -> BootstrapDownloadPlan {
    BootstrapDownloadPlan {
        archive_url: release.archive_url.clone(),
        additional_part_urls: release.additional_part_urls.clone(),
        archive_sha256: release.archive_sha256.clone(),
        archive_size: release.archive_size,
        signature: release.signature.clone(),
        archive_path: paths.archive_path.display().to_string(),
        staging_path: paths.staging_path.display().to_string(),
        resume_metadata_path: paths.resume_metadata_path.display().to_string(),
    }
}

fn archive_file_name(release: &BootstrapReleaseManifest) -> &str {
    release
        .archive_url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("runtime-pack.archive")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::status::{RuntimeCapability, RuntimeState};
    use std::path::PathBuf;

    fn fixture_status() -> RuntimeStatus {
        RuntimeStatus {
            state: RuntimeState::Fixture,
            pack_version: Some("2026.05.0".to_string()),
            repair_needed: false,
            repair_available: false,
            summary: "Runtime fixture".to_string(),
            blocked_capabilities: vec![RuntimeCapability::Ocr],
            details: vec![],
            guidance: vec![],
            bootstrap_eligible: false,
            bootstrap_required: false,
            active_operation: None,
        }
    }

    fn fixture_manifest() -> RuntimeManifest {
        RuntimeManifest {
            pack_version: "2026.05.0".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: crate::runtime::paths::current_runtime_platform(),
            payload_profile: "fixture".to_string(),
            release_injection_required: true,
            external_artifacts_required: vec!["relocatable-python".to_string()],
            python_relpath: "python/bin/python3".to_string(),
            uv_relpath: "uv/bin/uv".to_string(),
            python_files: vec![],
            uv_files: vec![],
            script_files: vec![],
            wheelhouse: vec![],
            caches: vec![],
            native_assets: vec![],
        }
    }

    #[test]
    fn remote_catalog_available_yields_download_plan() {
        let controller = BootstrapController::new();
        let plan = controller.plan(
            &fixture_status(),
            &fixture_manifest(),
            &PathBuf::from("/tmp/entropia-data"),
            BootstrapRemoteCatalog::Available {
                source: BootstrapRemoteSource {
                    manifest_url: "https://example.com/bootstrap.json".to_string(),
                    public_key_id: "entropia-root".to_string(),
                },
                index: BootstrapManifestIndex {
                    channel: "stable".to_string(),
                    generated_at: "2026-05-06T00:00:00Z".to_string(),
                    releases: vec![BootstrapReleaseManifest {
                        app_version: env!("CARGO_PKG_VERSION").to_string(),
                        platform: crate::runtime::paths::current_runtime_platform(),
                        pack_version: "2026.05.1".to_string(),
                        archive_url: "https://example.com/runtime-pack.zip".to_string(),
                        additional_part_urls: Vec::new(),
                        archive_sha256: "sha".to_string(),
                        archive_size: 99,
                        signature: "sig".to_string(),
                    }],
                },
            },
        );

        assert_eq!(plan.source, Some(BootstrapPlanSource::TrustedRemote));
        assert!(plan.eligible);
        assert_eq!(
            plan.download.as_ref().map(|download| download.archive_size),
            Some(99)
        );
    }

    #[test]
    fn source_unavailable_yields_blocked_operation() {
        let controller = BootstrapController::new();
        let plan = controller.plan(
            &fixture_status(),
            &fixture_manifest(),
            &PathBuf::from("/tmp/entropia-data"),
            BootstrapRemoteCatalog::SourceUnavailable {
                source: None,
                reason: "offline".to_string(),
            },
        );

        let operation = bootstrap_operation_from_plan(&plan).expect("operation should exist");

        assert!(!plan.eligible);
        assert_eq!(operation.stage, RuntimeOperationStage::Blocked);
    }

    #[test]
    fn load_remote_catalog_returns_available_when_manifest_is_valid() {
        let catalog = load_remote_catalog(
            BootstrapRemoteSource {
                manifest_url: "https://example.com/bootstrap.json".to_string(),
                public_key_id: "entropia-root".to_string(),
            },
            |_| {
                Ok(r#"{
                        "channel": "stable",
                        "generated_at": "2026-05-06T00:00:00Z",
                        "releases": [{
                            "app_version": "0.0.10",
                            "platform": "linux-x86_64",
                            "pack_version": "2026.05.1",
                            "archive_url": "https://example.com/runtime-pack.zip",
                            "archive_sha256": "abc",
                            "archive_size": 42,
                            "signature": "sig"
                        }]
                    }"#
                .to_string())
            },
        );

        match catalog {
            BootstrapRemoteCatalog::Available { source, index } => {
                assert_eq!(source.public_key_id, "entropia-root");
                assert_eq!(index.releases.len(), 1);
            }
            other => panic!("expected available catalog, got {other:?}"),
        }
    }

    #[test]
    fn load_remote_catalog_surfaces_fetch_failures_as_source_unavailable() {
        let catalog = load_remote_catalog(
            BootstrapRemoteSource {
                manifest_url: "https://example.com/bootstrap.json".to_string(),
                public_key_id: "entropia-root".to_string(),
            },
            |_| Err("offline: manifest fetch failed".to_string()),
        );

        assert_eq!(
            catalog,
            BootstrapRemoteCatalog::SourceUnavailable {
                source: Some(BootstrapRemoteSource {
                    manifest_url: "https://example.com/bootstrap.json".to_string(),
                    public_key_id: "entropia-root".to_string(),
                }),
                reason: "offline: manifest fetch failed".to_string(),
            }
        );
    }

    #[test]
    fn load_remote_catalog_rejects_invalid_manifest_json() {
        let catalog = load_remote_catalog(
            BootstrapRemoteSource {
                manifest_url: "https://example.com/bootstrap.json".to_string(),
                public_key_id: "entropia-root".to_string(),
            },
            |_| Ok("not-json".to_string()),
        );

        match catalog {
            BootstrapRemoteCatalog::SourceUnavailable { source, reason } => {
                assert_eq!(
                    source,
                    Some(BootstrapRemoteSource {
                        manifest_url: "https://example.com/bootstrap.json".to_string(),
                        public_key_id: "entropia-root".to_string(),
                    })
                );
                assert!(reason.contains("Failed to parse bootstrap manifest index"));
            }
            other => panic!("expected unavailable catalog, got {other:?}"),
        }
    }
}
