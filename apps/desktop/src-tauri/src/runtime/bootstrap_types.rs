//! Bootstrap CONTRACT types, kept in an ALWAYS-COMPILED module so the API-only
//! ("lite", `--no-default-features`) build can construct and return them without
//! pulling in the gated `bootstrap` impl (reqwest/manifest/download). The full
//! `local-ml` build re-exports these from `runtime::bootstrap` so existing
//! `crate::runtime::bootstrap::Bootstrap*` paths keep resolving unchanged.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapPlanSource {
    ManagedReady,
    BundledRelease,
    TrustedRemote,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapRemoteSource {
    pub manifest_url: String,
    pub public_key_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapDownloadPlan {
    pub archive_url: String,
    /// Extra archive part URLs (parts 2..N) for multi-part hosting. Empty = single archive.
    #[serde(default)]
    pub additional_part_urls: Vec<String>,
    pub archive_sha256: String,
    pub archive_size: u64,
    pub signature: String,
    pub archive_path: String,
    pub staging_path: String,
    pub resume_metadata_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapPlan {
    pub eligible: bool,
    pub required: bool,
    pub source: Option<BootstrapPlanSource>,
    pub pack_version: Option<String>,
    pub summary: String,
    pub reason: Option<String>,
    pub remote_source: Option<BootstrapRemoteSource>,
    pub download: Option<BootstrapDownloadPlan>,
}
