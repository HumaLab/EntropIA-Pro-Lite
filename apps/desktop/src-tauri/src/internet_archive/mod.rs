//! Internet Archive connector over the shared `internet-archive` crate:
//! read-only commands (identifier resolution, full-text search, collection
//! browsing, item metadata with PDF availability) plus import preparation
//! over an existing collection id.
//!
//! The Tauri state lives here; the commands live in [`commands`]. The job
//! store persists prepared import jobs for the later download/import tasks.

pub mod commands;
pub mod import;
pub mod provenance_db;
pub mod runner;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use internet_archive::store::{JobStore, RecoveryReconcileEntry};
use internet_archive::{Endpoints, FetchPolicy, IaClient};

/// One active download runner: the job it works on plus the cancel signal.
/// Only one job runs at a time; the frontend cancels through the command
/// that flips this sender.
#[derive(Debug)]
pub struct ActiveRun {
    /// Job id the runner works on.
    pub job_id: String,
    /// `true` means cancel; the runner races the download against it.
    pub cancel: tokio::sync::watch::Sender<bool>,
}

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
    /// The currently running download, if any. One job at a time: a second
    /// `ia_start_import` is rejected with `job_running` while this is set.
    /// Lock order is fixed: `active` first, then `jobs`. Nothing takes
    /// `jobs` and then `active`. Never held across an `.await`: locked only
    /// to inspect, insert, or clear.
    pub active: Mutex<Option<ActiveRun>>,
    /// Single writer lock for `job.json`. Every write is load → modify →
    /// save while holding it (runner transitions, start, direct cancel,
    /// end-of-run cancel, recovery, and later task 23). Only synchronous
    /// file I/O happens under it; it is never held across an `.await`.
    pub jobs: Mutex<()>,
    /// `needs_reconcile` entries from the startup `recover_all`: files that
    /// were `Importing` when a `Cancelling` job restarted. The frontend
    /// import may have been interrupted midway, so these need provenance
    /// reconciliation before their staging may be deleted.
    pub reconcile: Mutex<Vec<RecoveryReconcileEntry>>,
    /// Resolved data dir (remembered at setup): asset-file existence checks
    /// resolve stored `assets.path` values against it.
    pub data_dir: PathBuf,
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
            active: Mutex::new(None),
            jobs: Mutex::new(()),
            reconcile: Mutex::new(Vec::new()),
            data_dir: data_dir.to_path_buf(),
        }
    }
}
