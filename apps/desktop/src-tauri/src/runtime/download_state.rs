//! Durable per-part resume state for the multi-part runtime bootstrap download.
//!
//! The downloader ([`crate::runtime::download`]) streams archive parts into a
//! single staging tmp file. After every part, and also periodically mid-part
//! and on any network/stall failure, it persists this state atomically.
//!
//! Atomic here means write to `*.tmp`, then flush, `fsync`, atomic rename,
//! then a best-effort parent-dir `fsync`, all under the existing `.downloads`
//! staging dir.
//!
//! The next attempt reloads the state, checks the release identity and the
//! tmp-file length, re-hashes the bytes already on disk, skips fully
//! completed parts, and resumes the part in progress with `Range: bytes=N-`.
//! A missing, corrupt, inconsistent, or stale file is discarded and the
//! download restarts from zero.

use crate::runtime::manifest::BootstrapReleaseManifest;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// File name (inside the per-pack `.downloads` dir) holding the in-progress
/// resume state. Separate from `resume.json`, which records a *completed*
/// download and is only written after the whole-archive hash verifies.
pub(crate) const RESUME_STATE_FILE_NAME: &str = "resume-state.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PartResumeState {
    /// Part URL (`archive_url` for part 1, `additional_part_urls` for 2..N).
    pub url: String,
    /// Bytes of this part already staged in the tmp file.
    pub bytes_done: u64,
    /// True once the part stream reached EOF and was durably recorded.
    pub completed: bool,
    /// Bytes observed when the part completed (`None` while in progress).
    #[serde(default)]
    pub expected_size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PersistedResumeState {
    /// Release identity: any mismatch discards the state and restarts.
    pub pack_version: String,
    pub archive_sha256: String,
    pub archive_size: u64,
    pub archive_url: String,
    #[serde(default)]
    pub additional_part_urls: Vec<String>,
    /// One entry per part URL, in fetch order; `total_downloaded` must equal
    /// the sum of `bytes_done` and the tmp-file length on disk.
    #[serde(default)]
    pub parts: Vec<PartResumeState>,
    #[serde(default)]
    pub total_downloaded: u64,
}

/// Part URLs in fetch order: `archive_url` is part 1, then 2..N. All sizes
/// stay `u64` — the archive is ~2.44 GB, past `i32`/`u32` range.
pub(crate) fn release_part_urls(release: &BootstrapReleaseManifest) -> Vec<String> {
    std::iter::once(release.archive_url.clone())
        .chain(release.additional_part_urls.iter().cloned())
        .collect()
}

pub(crate) fn resume_state_path(app_data_dir: &Path, pack_version: &str) -> PathBuf {
    crate::runtime::download::bootstrap_download_dir(app_data_dir, pack_version)
        .join(RESUME_STATE_FILE_NAME)
}

pub(crate) fn fresh_state(release: &BootstrapReleaseManifest) -> PersistedResumeState {
    PersistedResumeState {
        pack_version: release.pack_version.clone(),
        archive_sha256: release.archive_sha256.clone(),
        archive_size: release.archive_size,
        archive_url: release.archive_url.clone(),
        additional_part_urls: release.additional_part_urls.clone(),
        parts: release_part_urls(release)
            .into_iter()
            .map(|url| PartResumeState {
                url,
                bytes_done: 0,
                completed: false,
                expected_size: None,
            })
            .collect(),
        total_downloaded: 0,
    }
}

pub(crate) fn matches_release(
    state: &PersistedResumeState,
    release: &BootstrapReleaseManifest,
) -> bool {
    state.pack_version == release.pack_version
        && state.archive_sha256 == release.archive_sha256
        && state.archive_size == release.archive_size
        && state.archive_url == release.archive_url
        && state.additional_part_urls == release.additional_part_urls
}

/// Loads persisted state when it matches `release` *and* is internally
/// consistent (part URLs line up in order, completed parts carry their
/// observed size, offsets sum to the total). Returns `None` for a missing,
/// corrupt, stale, or inconsistent file — the caller discards the staged
/// bytes and restarts from zero in all those cases.
pub(crate) fn load(
    path: &Path,
    release: &BootstrapReleaseManifest,
) -> Option<PersistedResumeState> {
    let bytes = fs::read(path).ok()?;
    let state: PersistedResumeState = serde_json::from_slice(&bytes).ok()?;
    if !matches_release(&state, release) {
        return None;
    }
    let urls = release_part_urls(release);
    if state.parts.len() != urls.len() {
        return None;
    }
    if state
        .parts
        .iter()
        .map(|part| part.url.as_str())
        .ne(urls.iter().map(String::as_str))
    {
        return None;
    }
    for part in &state.parts {
        if part.completed && part.expected_size != Some(part.bytes_done) {
            return None;
        }
    }
    let sum: u64 = state
        .parts
        .iter()
        .map(|part| part.bytes_done)
        .fold(0u64, |total, done| total.saturating_add(done));
    if sum != state.total_downloaded {
        return None;
    }
    Some(state)
}

/// Durable persist: write + flush + `fsync`, then atomic rename; best-effort
/// `fsync` of the parent dir afterwards so the rename itself survives a
/// crash. All sizes are `u64`.
pub(crate) fn save(path: &Path, state: &PersistedResumeState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create resume state dir {}: {error}",
                parent.display()
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Failed to serialize bootstrap resume state: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut file = fs::File::create(&tmp).map_err(|error| {
            format!(
                "Failed to create resume state tmp {}: {error}",
                tmp.display()
            )
        })?;
        file.write_all(&bytes).map_err(|error| {
            format!(
                "Failed to write resume state tmp {}: {error}",
                tmp.display()
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "Failed to flush resume state tmp {}: {error}",
                tmp.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "Failed to fsync resume state tmp {}: {error}",
                tmp.display()
            )
        })?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        format!(
            "Failed to finalize resume state from {} to {}: {error}",
            tmp.display(),
            path.display()
        )
    })?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

pub(crate) fn discard(path: &Path) {
    let _ = fs::remove_file(path);
}
