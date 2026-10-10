//! Internet Archive connector over the shared `internet-archive` crate:
//! read-only commands (identifier resolution, full-text search, collection
//! browsing, item metadata with PDF availability) plus import preparation
//! over an existing collection id.
//!
//! The Tauri state lives here; the commands live in [`commands`]. The job
//! store persists prepared import jobs for the later download/import tasks.

pub mod commands;
pub mod provenance_db;

use std::path::Path;

use internet_archive::store::JobStore;
use internet_archive::{Endpoints, FetchPolicy, IaClient};

/// Managed state for the Internet Archive connector: a strict read client,
/// the production endpoints, and the local job store.
pub struct IaState {
    /// Strict read client (production fetch policy).
    pub client: IaClient,
    /// API endpoints (production).
    pub endpoints: Endpoints,
    /// Local job store, rooted under the data dir so it follows dev
    /// profiles. Written by `ia_prepare_import`, consumed by the later
    /// download/import tasks.
    pub store: JobStore,
}

impl IaState {
    /// Build the state once the data dir is known. The store root hangs off
    /// `data_dir` so it follows dev profiles; the directory itself is
    /// created lazily by [`JobStore`] on first write, never here.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            client: IaClient::new(FetchPolicy::production()),
            endpoints: Endpoints::production(),
            store: JobStore::new(data_dir.join("staging").join("internet-archive")),
        }
    }
}
