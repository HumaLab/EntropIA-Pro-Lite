use crate::runtime::download_state;
use crate::runtime::manifest::{BootstrapReleaseManifest, RuntimeManifest};
use crate::runtime::paths::{
    ensure_executable_bit, managed_pack_dir, runtime_root, stage_marker_path,
};
use crate::runtime::status::{RuntimeOperation, RuntimeOperationKind, RuntimeOperationStage};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;

/// Re-persist the in-progress resume state after this many newly downloaded
/// bytes, so an interruption loses at most ~1 MB of bookkeeping (the staged
/// bytes themselves are always on disk; the state is re-saved unconditionally
/// after every part and on every failure). All sizes stay `u64`.
const RESUME_PERSIST_INTERVAL_BYTES: u64 = 1024 * 1024;

/// Connect timeout for the bootstrap HTTP client: bounding only connection
/// setup, never the transfer itself.
pub(crate) const BOOTSTRAP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-read stall timeout for the bootstrap HTTP client: a read that yields
/// no bytes for this long aborts, while a steadily flowing ~2.44 GB transfer
/// runs as long as it needs (total duration unbounded).
pub(crate) const BOOTSTRAP_READ_STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout policy for the bootstrap HTTP client, and the single source of
/// truth the builder below consumes: a connect timeout plus a per-read stall
/// timeout, and explicitly NO total deadline (a ~2.44 GB transfer must run as
/// long as it keeps flowing). `stall_timeout_not_total_timeout` pins this:
/// reintroducing a total deadline must flow through this struct and fails
/// that test — the builder takes no other timeout input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BootstrapTimeoutConfig {
    pub connect_timeout: Duration,
    pub read_stall_timeout: Duration,
    /// Must stay `None`.
    pub total_timeout: Option<Duration>,
}

pub(crate) fn bootstrap_timeout_config() -> BootstrapTimeoutConfig {
    BootstrapTimeoutConfig {
        connect_timeout: BOOTSTRAP_CONNECT_TIMEOUT,
        read_stall_timeout: BOOTSTRAP_READ_STALL_TIMEOUT,
        total_timeout: None,
    }
}

/// Builds the bootstrap HTTP client: connect timeout plus per-read stall
/// timeout, no total deadline.
///
/// NOTE on `reqwest::blocking` semantics: `ClientBuilder::timeout` is
/// enforced per blocking operation — the header wait *and each*
/// `Response::read` — not as a total transfer deadline (every successful
/// read resets it). Combined with an explicit `connect_timeout` and no other
/// deadline, a healthy multi-GB transfer is never cut off early, while a
/// connection that stalls (no bytes for `BOOTSTRAP_READ_STALL_TIMEOUT`) is.
pub(crate) fn build_bootstrap_client() -> Result<Client, String> {
    let config = bootstrap_timeout_config();
    Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.read_stall_timeout)
        .build()
        .map_err(|error| format!("Failed to create bootstrap HTTP client: {error}"))
}

/// T2 (S2, S5) cancellation token: a shared flag the download read loop
/// checks at least once per chunk, including the T1 resume path (resumed
/// transfers flow through the same loop). The core takes an explicit
/// `Option<&AtomicBool>` so unit tests stay hermetic with per-test tokens;
/// production passes the command-layer flag below (`runtime_cancel_download`
/// sets it, the `runtime_bootstrap_start` worker clears it on launch).
///
/// On cancel the core persists the resume state and keeps the partial (T1
/// semantics — a later start resumes, never restarts), leaves the runtime
/// in a clean non-downloading state, and returns the quiet sentinel below
/// instead of the network/verification error shape.
pub(crate) const BOOTSTRAP_DOWNLOAD_CANCELLED_MESSAGE: &str =
    "Bootstrap download cancelled by user";

/// True when `error` is the user-cancel sentinel rather than a transfer or
/// verification failure: cancel stays quiet with the partial kept, while
/// every other error keeps the existing failure path unchanged. Test-only
/// for now: production distinguishes cancel through the flag, not the
/// message (T3 owns the status copy and may promote this).
#[cfg(test)]
pub(crate) fn is_bootstrap_cancelled_error(error: &str) -> bool {
    error == BOOTSTRAP_DOWNLOAD_CANCELLED_MESSAGE
}

/// Command-layer cancel flag observed by the in-flight download loop.
static BOOTSTRAP_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
/// True while a bootstrap worker launched by `spawn_bootstrap_download` is
/// in flight, so `request_bootstrap_cancel` can report whether a download
/// was actually running (as opposed to merely arming the flag).
static BOOTSTRAP_DOWNLOAD_RUNNING: AtomicBool = AtomicBool::new(false);

/// Sets the cancel token; returns whether a bootstrap download was actually
/// running. The in-flight read loop observes the flag on its next per-chunk
/// check and exits promptly with the quiet sentinel.
pub(crate) fn request_bootstrap_cancel() -> bool {
    BOOTSTRAP_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
    BOOTSTRAP_DOWNLOAD_RUNNING.load(Ordering::SeqCst)
}

/// Disarms the cancel token. Called by the start worker on launch so a
/// previous cancel never leaks into the next transfer (and only there: an
/// idempotent busy-start must never disarm another download's cancel).
pub(crate) fn clear_bootstrap_cancel() {
    BOOTSTRAP_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
}

pub(crate) fn is_bootstrap_cancel_requested() -> bool {
    BOOTSTRAP_CANCEL_REQUESTED.load(Ordering::SeqCst)
}

fn bootstrap_cancel_requested(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::SeqCst))
}

/// Outcome of attempting to launch a bootstrap download worker: either the
/// worker was spawned (holding the single-flight `ops_lock` guard for the
/// whole transfer) or a download is already running (no second fetch opened).
pub(crate) enum BootstrapSpawn<T> {
    Started(std::thread::JoinHandle<T>),
    AlreadyRunning,
}

/// Launches `run` on a background thread under the single-flight `ops_lock`
/// — the blocking core executes off the main thread, so the app stays
/// usable while downloading (S5). A busy gate returns `AlreadyRunning`
/// WITHOUT invoking `run`, which is what makes `runtime_bootstrap_start`
/// idempotent. `std::thread::spawn` (rather than `spawn_blocking`) keeps
/// this unit-testable without a Tokio context; the async Tauri command is a
/// thin wrapper that returns promptly while the worker runs on.
pub(crate) fn spawn_bootstrap_download<T>(
    operation: &str,
    run: impl FnOnce() -> T + Send + 'static,
) -> BootstrapSpawn<T>
where
    T: Send + 'static,
{
    let guard = match crate::runtime::ops_lock::try_acquire(operation) {
        Ok(guard) => guard,
        Err(_) => return BootstrapSpawn::AlreadyRunning,
    };
    BootstrapSpawn::Started(std::thread::spawn(move || {
        // Held for the whole transfer: `Drop` releases the lock on return
        // AND on worker panic (unwinding), so a crashing worker can never
        // wedge the single-flight gate; the running flag resets the same way.
        let _guard = guard;
        BOOTSTRAP_DOWNLOAD_RUNNING.store(true, Ordering::SeqCst);
        let _reset_running = BootstrapRunningReset;
        run()
    }))
}

struct BootstrapRunningReset;

impl Drop for BootstrapRunningReset {
    fn drop(&mut self) {
        BOOTSTRAP_DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// One archive-part fetch: the byte stream plus whether the server honored a
/// requested `Range` (HTTP 206, `resumed: true`) or answered with the full
/// part from byte zero (HTTP 200, `resumed: false` — the staged prefix of
/// that part is then truncated and rewritten).
pub struct FetchResponse<R> {
    pub reader: R,
    pub resumed: bool,
    /// For a 206: the start offset from `Content-Range`, when the server sent
    /// a parseable one. A mismatch with the requested offset restarts the
    /// part (the body is unusable mid-stream).
    pub content_range_start: Option<u64>,
    /// True when the server answered 416 (range not satisfiable): the
    /// recorded offset is invalid, so the part restarts from zero with a
    /// clean refetch (the 416 itself carries no body).
    pub range_not_satisfiable: bool,
}

/// Bytes that must be free on the runtime volume before starting a remote bootstrap:
/// the compressed archive is written to disk AND extracted next to it, so the peak
/// footprint is roughly 2x the archive plus headroom for the materialized env. We
/// require ~3x so a budget/corporate PC with little free space fails fast with a
/// clear message instead of dying mid-extract with Windows error 112 (disk full).
fn required_free_bytes(archive_size: u64) -> u64 {
    archive_size.saturating_mul(3)
}

/// Best-effort free space on the volume containing `path`. Returns `None` when it
/// cannot be determined (then the precheck is skipped rather than blocking install).
#[cfg(windows)]
pub(crate) fn available_disk_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_available: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free_available)
}

#[cfg(not(windows))]
pub(crate) fn available_disk_space(_path: &Path) -> Option<u64> {
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapDownloadOutcome {
    pub archive_path: PathBuf,
    pub staging_path: PathBuf,
    pub managed_root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResumeMetadata {
    pub manifest_url: String,
    pub archive_url: String,
    pub archive_sha256: String,
    pub downloaded_bytes: u64,
    pub archive_size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPlanPaths {
    pub archive_path: PathBuf,
    pub staging_path: PathBuf,
    pub resume_metadata_path: PathBuf,
}

pub fn bootstrap_download_dir(app_data_dir: &Path, pack_version: &str) -> PathBuf {
    runtime_root(app_data_dir)
        .join(".downloads")
        .join(pack_version)
}

pub fn bootstrap_download_plan_paths(
    app_data_dir: &Path,
    pack_version: &str,
    archive_name: &str,
) -> DownloadPlanPaths {
    let download_dir = bootstrap_download_dir(app_data_dir, pack_version);
    DownloadPlanPaths {
        archive_path: download_dir.join(archive_name),
        staging_path: runtime_root(app_data_dir).join(format!(".{pack_version}.staging")),
        resume_metadata_path: download_dir.join("resume.json"),
    }
}

/// Cleanup policy for staged download bytes on failure. A network/stall
/// failure keeps the staged tmp bytes plus the durably persisted resume
/// state so the next attempt continues where it stopped; a verification
/// failure (size/hash mismatch) deletes everything so a corrupt prefix can
/// never be mistaken for a resumable one. Success disarms the guard.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GuardAction {
    DeleteAll,
    KeepPartial,
    Committed,
}

struct PartialDownloadGuard {
    tmp: PathBuf,
    archive: PathBuf,
    resume_state: PathBuf,
    action: GuardAction,
}

impl PartialDownloadGuard {
    fn keep_partial(&mut self) {
        self.action = GuardAction::KeepPartial;
    }

    fn commit(&mut self) {
        self.action = GuardAction::Committed;
    }
}

impl Drop for PartialDownloadGuard {
    fn drop(&mut self) {
        if self.action == GuardAction::DeleteAll {
            let _ = fs::remove_file(&self.tmp);
            let _ = fs::remove_file(&self.archive);
            let _ = fs::remove_file(&self.resume_state);
        }
    }
}

pub fn download_and_activate_remote_runtime<F>(
    source_manifest_url: &str,
    release: &BootstrapReleaseManifest,
    app_data_dir: &Path,
    public_key_base64: &str,
    mut on_progress: F,
) -> Result<BootstrapDownloadOutcome, String>
where
    F: FnMut(RuntimeOperation),
{
    let client = build_bootstrap_client()?;

    download_and_activate_remote_runtime_with_ranged_fetch(
        source_manifest_url,
        release,
        app_data_dir,
        public_key_base64,
        |url, range_from| {
            let mut request = client.get(url);
            if let Some(from) = range_from {
                request = request.header("Range", format!("bytes={from}-"));
            }
            let response = request
                .send()
                .map_err(|error| format!("Failed to fetch remote runtime archive: {error}"))?;
            let status = response.status();
            if range_from.is_some() {
                if status == reqwest::StatusCode::PARTIAL_CONTENT {
                    let content_range_start = response
                        .headers()
                        .get(reqwest::header::CONTENT_RANGE)
                        .and_then(|value| value.to_str().ok())
                        .and_then(parse_content_range_start);
                    return Ok(FetchResponse {
                        reader: Box::new(response) as Box<dyn Read>,
                        resumed: true,
                        content_range_start,
                        range_not_satisfiable: false,
                    });
                }
                if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                    // The recorded offset is invalid (past the part end): no
                    // body follows, so the caller restarts the part from zero
                    // with a clean refetch instead of failing.
                    return Ok(FetchResponse {
                        reader: Box::new(std::io::empty()) as Box<dyn Read>,
                        resumed: false,
                        content_range_start: None,
                        range_not_satisfiable: true,
                    });
                }
                if status.is_success() {
                    // Server ignored `Range` (200 instead of 206): the caller
                    // truncates this part's staged prefix and rewrites it.
                    return Ok(FetchResponse {
                        reader: Box::new(response) as Box<dyn Read>,
                        resumed: false,
                        content_range_start: None,
                        range_not_satisfiable: false,
                    });
                }
            } else if status.is_success() {
                return Ok(FetchResponse {
                    reader: Box::new(response) as Box<dyn Read>,
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                });
            }
            Err(format!(
                "Remote runtime archive request failed with HTTP status {status}"
            ))
        },
        &mut on_progress,
    )
}

/// Single-attempt fetch adapter over the ranged core below. Test-only:
/// production entry points use the ranged core directly, and the only other
/// caller (manager bootstrap tests) is test code too. One-shot fetchers
/// never observe a `Range` offset, so every response is a full body
/// (`resumed: false`). If a persisted resume offset ever meets such a full
/// body, the core takes the server-ignored-Range path (truncate that part's
/// prefix, rewrite from zero) — still correct.
#[cfg(test)]
pub fn download_and_activate_remote_runtime_with_fetch<F, R>(
    source_manifest_url: &str,
    release: &BootstrapReleaseManifest,
    app_data_dir: &Path,
    public_key_base64: &str,
    mut fetch_archive: F,
    on_progress: &mut (impl FnMut(RuntimeOperation) + ?Sized),
) -> Result<BootstrapDownloadOutcome, String>
where
    F: FnMut(&str) -> Result<R, String>,
    R: Read,
{
    download_and_activate_remote_runtime_with_ranged_fetch(
        source_manifest_url,
        release,
        app_data_dir,
        public_key_base64,
        |url, _range_from| {
            fetch_archive(url).map(|reader| FetchResponse {
                reader,
                resumed: false,
                content_range_start: None,
                range_not_satisfiable: false,
            })
        },
        on_progress,
    )
}

/// Command-layer entry of the resumable core below: identical behavior with
/// the command-layer cancel flag armed, so `runtime_cancel_download` stops
/// the manager-driven transfer without touching `manager.rs`.
pub fn download_and_activate_remote_runtime_with_ranged_fetch<F, R>(
    source_manifest_url: &str,
    release: &BootstrapReleaseManifest,
    app_data_dir: &Path,
    public_key_base64: &str,
    fetch_archive: F,
    on_progress: &mut (impl FnMut(RuntimeOperation) + ?Sized),
) -> Result<BootstrapDownloadOutcome, String>
where
    F: FnMut(&str, Option<u64>) -> Result<FetchResponse<R>, String>,
    R: Read,
{
    download_and_activate_remote_runtime_with_ranged_fetch_and_cancel(
        source_manifest_url,
        release,
        app_data_dir,
        public_key_base64,
        fetch_archive,
        Some(&BOOTSTRAP_CANCEL_REQUESTED),
        on_progress,
    )
}

/// Cancel exit shared by the part-fetch and chunk-read loops: durably
/// persists the resume state (a later start resumes, never restarts),
/// disarms the cleanup guard so the partial survives, and returns the quiet
/// cancel sentinel — never the network/verification error shape, so cancel
/// stays distinguishable from failure (no error latch).
fn finish_cancelled_download(
    resume_state_file: &Path,
    resume_state: &mut download_state::PersistedResumeState,
    downloaded_bytes: u64,
    partial_guard: &mut PartialDownloadGuard,
) -> String {
    resume_state.total_downloaded = downloaded_bytes;
    let _ = download_state::save(resume_state_file, resume_state);
    partial_guard.keep_partial();
    BOOTSTRAP_DOWNLOAD_CANCELLED_MESSAGE.to_string()
}

/// Resumable multi-part download core. `fetch_archive` receives each part URL
/// plus the already-staged byte offset of that part (`None` for a fresh part
/// — no `Range` header is sent then) and reports whether the server honored
/// the range (HTTP 206) or restarted the part (HTTP 200).
///
/// `cancel` is the T2 shared cancel flag, checked at least once per chunk
/// (see the read loop below): on cancel the partial is persisted and kept
/// (T1 resume semantics) and the quiet sentinel is returned. Pass `None`
/// for an uncancellable run. Production callers pass the command-layer
/// flag; use `download_and_activate_remote_runtime_with_ranged_fetch` for
/// the command-layer default.
pub fn download_and_activate_remote_runtime_with_ranged_fetch_and_cancel<F, R>(
    source_manifest_url: &str,
    release: &BootstrapReleaseManifest,
    app_data_dir: &Path,
    public_key_base64: &str,
    mut fetch_archive: F,
    cancel: Option<&AtomicBool>,
    on_progress: &mut (impl FnMut(RuntimeOperation) + ?Sized),
) -> Result<BootstrapDownloadOutcome, String>
where
    F: FnMut(&str, Option<u64>) -> Result<FetchResponse<R>, String>,
    R: Read,
{
    release.verify_signature(public_key_base64)?;

    // Fail fast on a low-disk machine before committing to a multi-GB download +
    // extract (otherwise it dies mid-way with os error 112). Best-effort: if free
    // space can't be read, proceed rather than block.
    if let Some(free) = available_disk_space(app_data_dir) {
        let required = required_free_bytes(release.archive_size);
        if free < required {
            return Err(format!(
                "Espacio en disco insuficiente para instalar el runtime de IA: se requieren ~{:.1} GB libres y hay {:.1} GB en el disco de la aplicación. Liberá espacio y volvé a intentar.",
                required as f64 / 1_000_000_000.0,
                free as f64 / 1_000_000_000.0
            ));
        }
    }

    let paths = bootstrap_download_plan_paths(
        app_data_dir,
        &release.pack_version,
        archive_file_name(release),
    );
    if let Some(parent) = paths.archive_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create download dir {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::create_dir_all(runtime_root(app_data_dir)).map_err(|error| {
        format!(
            "Failed to create runtime root {}: {error}",
            runtime_root(app_data_dir).display()
        )
    })?;

    emit_progress(
        on_progress,
        RuntimeOperationStage::Downloading,
        "Descargando runtime remoto confiable",
        Some(0),
        Some(0),
        Some(release.archive_size),
        true,
    );

    let tmp_archive_path = paths.archive_path.with_extension("download.tmp");
    let resume_state_file = download_state::resume_state_path(app_data_dir, &release.pack_version);

    // Resume: keep the staged bytes when the persisted per-part state matches
    // this release (pack_version + archive sha256 + size + part URLs).
    // - Staged length equals the recorded offset: resume directly.
    // - Staged file is LONGER (hard kill between periodic persists): the tail
    //   past the recorded offset never made it into the bookkeeping, so
    //   truncate back to the recorded offset and resume from there — losing
    //   at most one persist interval of re-download instead of everything.
    // - Staged file is SHORTER (or the recorded total exceeds the release
    //   size): the state is untrustworthy — discard everything, restart zero.
    // A missing/corrupt/stale state also discards any orphan staged bytes.
    let mut resume = download_state::load(&resume_state_file, release);
    if let Some(ref state) = resume {
        let staged_len = fs::metadata(&tmp_archive_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        // Short-circuit order matters: an oversized recorded total never
        // reaches the truncate attempt below.
        let restart_from_zero = state.total_downloaded > release.archive_size
            || staged_len < state.total_downloaded
            || (staged_len > state.total_downloaded
                && truncate_file_to(&tmp_archive_path, state.total_downloaded).is_err());
        if restart_from_zero {
            resume = None;
        }
    }
    if resume.is_none() {
        let _ = fs::remove_file(&tmp_archive_path);
        download_state::discard(&resume_state_file);
    }
    let mut resume_state = resume.unwrap_or_else(|| download_state::fresh_state(release));

    // Re-hash the bytes already on disk so the final whole-archive SHA-256 is
    // computed over the complete data including resumed bytes. Design choice:
    // one extra streaming pass over the partial file instead of persisting
    // hasher internals — no fragile serialized hash state, and correctness
    // holds no matter how the process was interrupted. An unreadable partial
    // is not resumable, so that error deletes everything via the guard.
    let mut downloaded_bytes = resume_state.total_downloaded;
    let mut hasher = Sha256::new();
    if downloaded_bytes > 0 {
        hasher = sha256_of_file(&tmp_archive_path)?;
    }

    // Network/stall failures keep the staged bytes plus the durably persisted
    // resume state so the next attempt continues; verification failures
    // (size/hash mismatch) delete everything (guard default).
    let mut partial_guard = PartialDownloadGuard {
        tmp: tmp_archive_path.clone(),
        archive: paths.archive_path.clone(),
        resume_state: resume_state_file.clone(),
        action: GuardAction::DeleteAll,
    };
    // Opened with `write` (NOT `append`): an append-only handle cannot
    // `set_len` on Windows (no FILE_WRITE_DATA), which the truncate paths
    // below need. The cursor is seeked to the end explicitly, so resumed
    // attempts keep appending; a fresh attempt recreates the empty file.
    // `truncate(false)` is explicit: resuming must never truncate the file
    // on open (truncation happens only via the repair paths below).
    let mut archive_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&tmp_archive_path)
        .map_err(|error| {
            format!(
                "Failed to create temporary archive {}: {error}",
                tmp_archive_path.display()
            )
        })?;
    if let Err(error) = archive_file.seek(std::io::SeekFrom::End(0)) {
        partial_guard.keep_partial();
        return Err(format!(
            "Failed while seeking in temporary archive {}: {error}",
            tmp_archive_path.display()
        ));
    }
    let mut buffer = vec![0u8; DEFAULT_DOWNLOAD_CHUNK_SIZE];
    let mut last_reported_progress = 0u8;
    let mut since_last_persist = 0u64;

    // Part 1 is `archive_url`; any `additional_part_urls` are parts 2..N. They are
    // fetched and concatenated in order, so the running hash/size cover the whole
    // archive regardless of how many parts the host had to split it into (e.g. to
    // stay under GitHub's 2 GiB per-asset limit). A single part behaves exactly as
    // the original single-archive download. Fully completed parts are skipped
    // outright; the part in progress resumes with `Range: bytes=N-`.
    let part_urls = download_state::release_part_urls(release);
    let start_part = resume_state
        .parts
        .iter()
        .position(|part| !part.completed)
        .unwrap_or(resume_state.parts.len());

    for (part_index, part_url) in part_urls.iter().enumerate().skip(start_part) {
        if bootstrap_cancel_requested(cancel) {
            return Err(finish_cancelled_download(
                &resume_state_file,
                &mut resume_state,
                downloaded_bytes,
                &mut partial_guard,
            ));
        }
        let part_offset = resume_state.parts[part_index].bytes_done;
        let range_from = if part_offset > 0 {
            Some(part_offset)
        } else {
            None
        };

        // Fetch the part body. A 416 (recorded offset invalid) or a 206
        // whose Content-Range start mismatches the request restarts the part
        // from zero and refetches it cleanly — at most once, since a refetch
        // carries no Range and therefore cannot 416; the loop cap below is
        // only a backstop against pathological servers.
        let mut attempt_range = range_from;
        let mut clean_refetch_done = false;
        let mut archive_reader = loop {
            let fetched = match fetch_archive(part_url, attempt_range) {
                Ok(fetched) => fetched,
                Err(error) => {
                    resume_state.total_downloaded = downloaded_bytes;
                    let _ = download_state::save(&resume_state_file, &resume_state);
                    partial_guard.keep_partial();
                    return Err(error);
                }
            };
            if fetched.range_not_satisfiable {
                if clean_refetch_done {
                    resume_state.total_downloaded = downloaded_bytes;
                    let _ = download_state::save(&resume_state_file, &resume_state);
                    partial_guard.keep_partial();
                    return Err("Remote server repeatedly rejected the part request".to_string());
                }
                if let Err(error) = restart_part_and_persist(
                    &mut archive_file,
                    &tmp_archive_path,
                    &mut hasher,
                    &mut downloaded_bytes,
                    &mut resume_state,
                    part_index,
                    &resume_state_file,
                ) {
                    partial_guard.keep_partial();
                    return Err(error);
                }
                attempt_range = None;
                clean_refetch_done = true;
                continue;
            }
            match (attempt_range, fetched.resumed, fetched.content_range_start) {
                (Some(_), false, _) => {
                    // 200 to a Range request: the body is the full part, so
                    // truncate this part's staged prefix and rewrite it.
                    if let Err(error) = restart_part_and_persist(
                        &mut archive_file,
                        &tmp_archive_path,
                        &mut hasher,
                        &mut downloaded_bytes,
                        &mut resume_state,
                        part_index,
                        &resume_state_file,
                    ) {
                        partial_guard.keep_partial();
                        return Err(error);
                    }
                    break fetched.reader;
                }
                (Some(offset), true, Some(start)) if start != offset => {
                    if start == 0 {
                        // Mislabeled 206: the body is actually the full part.
                        if let Err(error) = restart_part_and_persist(
                            &mut archive_file,
                            &tmp_archive_path,
                            &mut hasher,
                            &mut downloaded_bytes,
                            &mut resume_state,
                            part_index,
                            &resume_state_file,
                        ) {
                            partial_guard.keep_partial();
                            return Err(error);
                        }
                        break fetched.reader;
                    }
                    // Unusable mid-stream body: restart and refetch cleanly.
                    if clean_refetch_done {
                        resume_state.total_downloaded = downloaded_bytes;
                        let _ = download_state::save(&resume_state_file, &resume_state);
                        partial_guard.keep_partial();
                        return Err("Remote server sent an inconsistent Content-Range".to_string());
                    }
                    if let Err(error) = restart_part_and_persist(
                        &mut archive_file,
                        &tmp_archive_path,
                        &mut hasher,
                        &mut downloaded_bytes,
                        &mut resume_state,
                        part_index,
                        &resume_state_file,
                    ) {
                        partial_guard.keep_partial();
                        return Err(error);
                    }
                    attempt_range = None;
                    clean_refetch_done = true;
                    continue;
                }
                _ => break fetched.reader,
            }
        };

        loop {
            if bootstrap_cancel_requested(cancel) {
                return Err(finish_cancelled_download(
                    &resume_state_file,
                    &mut resume_state,
                    downloaded_bytes,
                    &mut partial_guard,
                ));
            }
            let read = match archive_reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) => {
                    resume_state.total_downloaded = downloaded_bytes;
                    let _ = download_state::save(&resume_state_file, &resume_state);
                    partial_guard.keep_partial();
                    return Err(format!(
                        "Failed while reading remote runtime archive: {error}"
                    ));
                }
            };
            if let Err(error) = archive_file.write_all(&buffer[..read]) {
                resume_state.total_downloaded = downloaded_bytes;
                let _ = download_state::save(&resume_state_file, &resume_state);
                partial_guard.keep_partial();
                return Err(format!("Failed while writing runtime archive: {error}"));
            }
            hasher.update(&buffer[..read]);
            downloaded_bytes += read as u64;
            resume_state.parts[part_index].bytes_done += read as u64;
            resume_state.total_downloaded = downloaded_bytes;
            since_last_persist += read as u64;
            if since_last_persist >= RESUME_PERSIST_INTERVAL_BYTES {
                let _ = download_state::save(&resume_state_file, &resume_state);
                since_last_persist = 0;
            }

            let progress = progress_percent(downloaded_bytes, release.archive_size).unwrap_or(99);
            if progress >= last_reported_progress.saturating_add(5)
                || downloaded_bytes == release.archive_size
            {
                last_reported_progress = progress;
                emit_progress(
                    on_progress,
                    RuntimeOperationStage::Downloading,
                    "Descargando runtime remoto confiable",
                    Some(progress.min(99)),
                    Some(downloaded_bytes),
                    Some(release.archive_size),
                    true,
                );
            }
        }

        resume_state.parts[part_index].expected_size =
            Some(resume_state.parts[part_index].bytes_done);
        resume_state.parts[part_index].completed = true;
        resume_state.total_downloaded = downloaded_bytes;
        let _ = download_state::save(&resume_state_file, &resume_state);
        since_last_persist = 0;
    }

    // Best-effort durability of the staged bytes before the atomic rename.
    let _ = archive_file.sync_all();
    drop(archive_file);

    if let Err(error) = fs::rename(&tmp_archive_path, &paths.archive_path) {
        resume_state.total_downloaded = downloaded_bytes;
        let _ = download_state::save(&resume_state_file, &resume_state);
        partial_guard.keep_partial();
        return Err(format!(
            "Failed to finalize downloaded archive from {} to {}: {error}",
            tmp_archive_path.display(),
            paths.archive_path.display()
        ));
    }

    if downloaded_bytes != release.archive_size {
        return Err(format!(
            "Downloaded archive size mismatch: expected {} bytes, got {}",
            release.archive_size, downloaded_bytes
        ));
    }

    let archive_sha256 = hex::encode(hasher.finalize());
    if archive_sha256 != release.archive_sha256 {
        return Err(format!(
            "Downloaded runtime archive checksum mismatch: expected {}, got {}",
            release.archive_sha256, archive_sha256
        ));
    }

    // Verified: the staged archive is complete and trustworthy, so it is never
    // deleted from here on. The completed `resume.json` record is written only
    // after verification, and the in-progress state is removed.
    partial_guard.keep_partial();
    write_resume_metadata(
        &paths.resume_metadata_path,
        &BootstrapResumeMetadata {
            manifest_url: source_manifest_url.to_string(),
            archive_url: release.archive_url.clone(),
            archive_sha256: release.archive_sha256.clone(),
            downloaded_bytes,
            archive_size: release.archive_size,
        },
    )?;
    let _ = fs::remove_file(&resume_state_file);

    emit_progress(
        on_progress,
        RuntimeOperationStage::Verifying,
        "Verificando integridad del runtime remoto descargado",
        Some(100),
        Some(downloaded_bytes),
        Some(release.archive_size),
        false,
    );

    let staging_path = paths.staging_path.clone();
    if staging_path.exists() {
        fs::remove_dir_all(&staging_path).map_err(|error| {
            format!(
                "Failed to clean staging dir {}: {error}",
                staging_path.display()
            )
        })?;
    }
    fs::create_dir_all(&staging_path).map_err(|error| {
        format!(
            "Failed to create staging dir {}: {error}",
            staging_path.display()
        )
    })?;
    write_stage_marker(app_data_dir, &release.pack_version, "extracting")?;

    emit_progress(
        on_progress,
        RuntimeOperationStage::Hydrating,
        "Extrayendo runtime remoto a staging",
        Some(100),
        Some(downloaded_bytes),
        Some(release.archive_size),
        false,
    );
    let activation_result = (|| {
        extract_archive_to_staging(&paths.archive_path, &staging_path)?;

        let manifest_path = staging_path.join("manifest.json");
        let runtime_manifest = RuntimeManifest::load_from_path(&manifest_path)?;
        verify_extracted_runtime(&staging_path, &runtime_manifest, &release.pack_version)?;

        Ok(runtime_manifest)
    })();

    let runtime_manifest = match activation_result {
        Ok(manifest) => manifest,
        Err(error) => {
            cleanup_incomplete_activation(app_data_dir, &release.pack_version, &staging_path);
            return Err(error);
        }
    };

    emit_progress(
        on_progress,
        RuntimeOperationStage::Activating,
        "Activando runtime remoto verificado",
        Some(100),
        Some(downloaded_bytes),
        Some(release.archive_size),
        false,
    );
    let managed_root =
        match promote_staged_runtime(app_data_dir, release, &runtime_manifest, &staging_path) {
            Ok(managed_root) => managed_root,
            Err(error) => {
                cleanup_incomplete_activation(app_data_dir, &release.pack_version, &staging_path);
                return Err(error);
            }
        };

    let stage_marker = stage_marker_path(app_data_dir, &release.pack_version);
    if stage_marker.exists() {
        fs::remove_file(&stage_marker).map_err(|error| {
            format!(
                "Failed to remove stage marker {}: {error}",
                stage_marker.display()
            )
        })?;
    }

    // Verified, extracted, and promoted — keep the archive and disarm the cleanup.
    partial_guard.commit();
    Ok(BootstrapDownloadOutcome {
        archive_path: paths.archive_path,
        staging_path: paths.staging_path,
        managed_root,
    })
}

fn cleanup_incomplete_activation(app_data_dir: &Path, pack_version: &str, staging_path: &Path) {
    let stage_marker = stage_marker_path(app_data_dir, pack_version);
    if stage_marker.exists() {
        let _ = fs::remove_file(stage_marker);
    }
    if staging_path.exists() {
        let _ = fs::remove_dir_all(staging_path);
    }
}

fn promote_staged_runtime(
    app_data_dir: &Path,
    release: &BootstrapReleaseManifest,
    runtime_manifest: &RuntimeManifest,
    staging_path: &Path,
) -> Result<PathBuf, String> {
    let managed_root = managed_pack_dir(app_data_dir, &release.pack_version);

    if existing_managed_runtime_matches_release(&managed_root, release) {
        fs::remove_dir_all(staging_path).map_err(|error| {
            format!(
                "Failed to remove redundant staging runtime {}: {error}",
                staging_path.display()
            )
        })?;
        return Ok(managed_root);
    }

    let backup_root = managed_replacement_backup_path(app_data_dir, &release.pack_version);
    let had_existing = managed_root.exists();
    if had_existing {
        fs::rename(&managed_root, &backup_root).map_err(|error| {
            format!(
                "Failed to move existing managed runtime {} to backup {} before activation: {error}",
                managed_root.display(),
                backup_root.display()
            )
        })?;
    }

    if let Err(error) = fs::rename(staging_path, &managed_root) {
        if had_existing && backup_root.exists() {
            let _ = fs::rename(&backup_root, &managed_root);
        }
        return Err(format!(
            "Failed to promote runtime from {} to {}: {error}",
            staging_path.display(),
            managed_root.display()
        ));
    }

    if let Err(error) =
        verify_extracted_runtime(&managed_root, runtime_manifest, &release.pack_version)
    {
        if had_existing && backup_root.exists() {
            let _ = fs::remove_dir_all(&managed_root);
            let _ = fs::rename(&backup_root, &managed_root);
        }
        return Err(error);
    }

    if had_existing && backup_root.exists() {
        let _ = fs::remove_dir_all(&backup_root);
    }

    Ok(managed_root)
}

fn existing_managed_runtime_matches_release(
    managed_root: &Path,
    release: &BootstrapReleaseManifest,
) -> bool {
    let Ok(manifest) = RuntimeManifest::load_from_path(&managed_root.join("manifest.json")) else {
        return false;
    };
    if manifest.pack_version != release.pack_version
        || manifest.app_version != release.app_version
        || manifest.platform != release.platform
    {
        return false;
    }

    verify_extracted_runtime(managed_root, &manifest, &release.pack_version).is_ok()
}

fn managed_replacement_backup_path(app_data_dir: &Path, pack_version: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    runtime_root(app_data_dir).join(format!(".{pack_version}.replacing-{millis}"))
}

fn archive_file_name(release: &BootstrapReleaseManifest) -> &str {
    release
        .archive_url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("runtime-pack.archive")
}

fn extract_archive_to_staging(archive_path: &Path, staging_path: &Path) -> Result<(), String> {
    let archive_file = fs::File::open(archive_path).map_err(|error| {
        format!(
            "Failed to open runtime archive {}: {error}",
            archive_path.display()
        )
    })?;
    let mut archive = zip::ZipArchive::new(archive_file).map_err(|error| {
        format!(
            "Failed to read runtime archive {}: {error}",
            archive_path.display()
        )
    })?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read archive entry #{index}: {error}"))?;
        let entry_name = entry
            .name()
            .map_err(|error| format!("Archive entry #{index} has an invalid name: {error}"))?
            .into_owned();
        let Some(safe_name) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            return Err(format!("Archive entry {entry_name} has an unsafe path"));
        };
        let output_path = staging_path.join(safe_name);
        if entry_name.ends_with('/') {
            fs::create_dir_all(&output_path).map_err(|error| {
                format!(
                    "Failed to create extracted directory {}: {error}",
                    output_path.display()
                )
            })?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create extracted parent {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut output_file = fs::File::create(&output_path).map_err(|error| {
            format!(
                "Failed to create extracted file {}: {error}",
                output_path.display()
            )
        })?;
        std::io::copy(&mut entry, &mut output_file)
            .map_err(|error| format!("Failed to extract archive entry {entry_name}: {error}"))?;
    }

    Ok(())
}

fn verify_extracted_runtime(
    staging_path: &Path,
    manifest: &RuntimeManifest,
    expected_pack_version: &str,
) -> Result<(), String> {
    if manifest.pack_version != expected_pack_version {
        return Err(format!(
            "Extracted runtime pack version mismatch: expected {}, got {}",
            expected_pack_version, manifest.pack_version
        ));
    }

    for entry in manifest.all_entries() {
        let target = staging_path.join(&entry.path);
        if !target.exists() {
            return Err(format!("Extracted runtime entry missing: {}", entry.path));
        }
        let sha256 = super::hash::sha256_file_streaming(&target).map_err(|error| {
            format!(
                "Failed to read extracted entry {}: {error}",
                target.display()
            )
        })?;
        if sha256 != entry.sha256 {
            return Err(format!(
                "Extracted runtime checksum mismatch for {}",
                entry.path
            ));
        }
        ensure_executable_bit(&target, entry.executable)?;
    }

    Ok(())
}

/// Streams a staged file back through SHA-256. Used to re-seed the running
/// hash from bytes kept from an interrupted attempt (and after truncating a
/// part whose server ignored `Range`), so the final whole-archive digest
/// always covers exactly the bytes on disk.
/// Truncates a staged file to `len` through a write (not append) handle —
/// an append-only handle cannot `set_len` on Windows (no FILE_WRITE_DATA).
fn truncate_file_to(path: &Path, len: u64) -> Result<(), String> {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_len(len))
        .map_err(|error| {
            format!(
                "Failed to truncate partial archive {}: {error}",
                path.display()
            )
        })
}

/// Restarts the in-progress part from byte zero: truncates this part's staged
/// prefix and positions the cursor for rewriting, then re-hashes the
/// truncated prefix so the running digest still covers exactly the bytes on
/// disk. On error the numbers are left untouched while the disk already
/// shrank, so the caller must save state, keep the partial, and return — the
/// next attempt then sees staged < recorded and discards everything cleanly
/// (see the resume block above).
/// Restarts the in-progress part from byte zero and durably persists the
/// rewound state: truncates this part's staged prefix (see
/// `restart_part_from_zero`), syncs the in-memory totals, and saves. On
/// error the partial is left saved-but-inconsistent on purpose — the caller
/// keeps it and returns, and the next attempt sees staged < recorded and
/// discards everything cleanly (see the resume block above).
fn restart_part_and_persist(
    archive_file: &mut fs::File,
    tmp_archive_path: &Path,
    hasher: &mut Sha256,
    downloaded_bytes: &mut u64,
    resume_state: &mut download_state::PersistedResumeState,
    part_index: usize,
    resume_state_file: &Path,
) -> Result<(), String> {
    if let Err(error) = restart_part_from_zero(
        archive_file,
        tmp_archive_path,
        hasher,
        downloaded_bytes,
        &mut resume_state.parts[part_index].bytes_done,
    ) {
        resume_state.total_downloaded = *downloaded_bytes;
        let _ = download_state::save(resume_state_file, resume_state);
        return Err(error);
    }
    resume_state.total_downloaded = *downloaded_bytes;
    let _ = download_state::save(resume_state_file, resume_state);
    Ok(())
}

fn restart_part_from_zero(
    archive_file: &mut fs::File,
    tmp_archive_path: &Path,
    hasher: &mut Sha256,
    downloaded_bytes: &mut u64,
    part_bytes_done: &mut u64,
) -> Result<(), String> {
    let truncate_to = (*downloaded_bytes).saturating_sub(*part_bytes_done);
    archive_file
        .set_len(truncate_to)
        .and_then(|()| {
            archive_file
                .seek(std::io::SeekFrom::Start(truncate_to))
                .map(|_| ())
        })
        .map_err(|error| format!("Failed while restarting part from zero: {error}"))?;
    *hasher = sha256_of_file(tmp_archive_path)?;
    *downloaded_bytes = truncate_to;
    *part_bytes_done = 0;
    Ok(())
}

/// Parses the start offset of a `Content-Range` header
/// (`bytes <start>-<end>/<total>` or `bytes <start>-<end>/*`).
fn parse_content_range_start(header: &str) -> Option<u64> {
    header
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .parse::<u64>()
        .ok()
}

fn sha256_of_file(path: &Path) -> Result<Sha256, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Failed to open partial archive {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; DEFAULT_DOWNLOAD_CHUNK_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed while re-hashing partial archive: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher)
}

fn write_resume_metadata(path: &Path, metadata: &BootstrapResumeMetadata) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create resume metadata dir {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(metadata)
            .map_err(|error| format!("Failed to serialize bootstrap resume metadata: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "Failed to write resume metadata {}: {error}",
            path.display()
        )
    })
}

fn write_stage_marker(app_data_dir: &Path, pack_version: &str, stage: &str) -> Result<(), String> {
    let marker_path = stage_marker_path(app_data_dir, pack_version);
    fs::write(
        &marker_path,
        serde_json::to_vec_pretty(
            &serde_json::json!({ "pack_version": pack_version, "stage": stage }),
        )
        .map_err(|error| format!("Failed to serialize stage marker: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "Failed to write stage marker {}: {error}",
            marker_path.display()
        )
    })
}

fn progress_percent(downloaded_bytes: u64, total_bytes: u64) -> Option<u8> {
    (total_bytes > 0).then(|| ((downloaded_bytes.saturating_mul(100)) / total_bytes).min(100) as u8)
}

fn emit_progress(
    on_progress: &mut (impl FnMut(RuntimeOperation) + ?Sized),
    stage: RuntimeOperationStage,
    summary: &str,
    progress_percent: Option<u8>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    retryable: bool,
) {
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

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;

    pub(crate) fn sample_release(
        signature: String,
        archive_sha256: String,
        archive_size: u64,
    ) -> BootstrapReleaseManifest {
        BootstrapReleaseManifest {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: crate::runtime::paths::current_runtime_platform(),
            pack_version: "2026.05.1".to_string(),
            archive_url: "https://example.com/runtime-pack.zip".to_string(),
            additional_part_urls: Vec::new(),
            archive_sha256,
            archive_size,
            signature,
        }
    }

    pub(crate) fn build_signed_release(archive_bytes: &[u8]) -> (BootstrapReleaseManifest, String) {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let unsigned = sample_release(
            String::new(),
            hex::encode(Sha256::digest(archive_bytes)),
            archive_bytes.len() as u64,
        );
        let signature = signing_key.sign(unsigned.signature_payload().as_bytes());
        let release = BootstrapReleaseManifest {
            signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
            ..unsigned
        };
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());
        (release, public_key)
    }

    pub(crate) fn runtime_archive_bytes() -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = SimpleFileOptions::default();
            zip.add_directory("python/", options).expect("python dir");
            zip.add_directory("python/bin/", options)
                .expect("python bin dir");
            zip.add_directory("uv/", options).expect("uv dir");
            zip.add_directory("uv/bin/", options).expect("uv bin dir");
            zip.start_file("python/bin/python3", options)
                .expect("python entry");
            zip.write_all(b"python").expect("python bytes");
            zip.start_file("uv/bin/uv", options).expect("uv entry");
            zip.write_all(b"uv").expect("uv bytes");
            let manifest = RuntimeManifest {
                pack_version: "2026.05.1".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                platform: crate::runtime::paths::current_runtime_platform(),
                payload_profile: "release".to_string(),
                release_injection_required: false,
                external_artifacts_required: vec![],
                python_relpath: "python/bin/python3".to_string(),
                uv_relpath: "uv/bin/uv".to_string(),
                python_files: vec![crate::runtime::manifest::ManifestEntry {
                    path: "python/bin/python3".to_string(),
                    sha256: hex::encode(Sha256::digest(b"python")),
                    size: 6,
                    executable: !cfg!(windows),
                }],
                uv_files: vec![crate::runtime::manifest::ManifestEntry {
                    path: "uv/bin/uv".to_string(),
                    sha256: hex::encode(Sha256::digest(b"uv")),
                    size: 2,
                    executable: true,
                }],
                script_files: vec![],
                wheelhouse: vec![],
                caches: vec![],
                native_assets: vec![],
            };
            zip.start_file("manifest.json", options)
                .expect("manifest entry");
            zip.write_all(&serde_json::to_vec_pretty(&manifest).expect("manifest json"))
                .expect("manifest bytes");
            zip.finish().expect("finish zip");
        }
        buffer.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::download::test_support::{build_signed_release, runtime_archive_bytes};
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    #[test]
    fn computes_bootstrap_download_paths_under_runtime_downloads_dir() {
        let app_data_dir = PathBuf::from("/tmp/entropia-data");

        let paths =
            bootstrap_download_plan_paths(&app_data_dir, "2026.05.1", "runtime-pack.archive");

        assert_eq!(
            paths.archive_path,
            app_data_dir
                .join("runtime")
                .join(".downloads")
                .join("2026.05.1")
                .join("runtime-pack.archive")
        );
        assert_eq!(
            paths.resume_metadata_path,
            app_data_dir
                .join("runtime")
                .join(".downloads")
                .join("2026.05.1")
                .join("resume.json")
        );
    }

    #[test]
    fn keeps_staging_path_sibling_to_managed_runtime_dirs() {
        let app_data_dir = PathBuf::from("/tmp/entropia-data");

        let paths = bootstrap_download_plan_paths(&app_data_dir, "2026.05.1", "runtime-pack.zip");

        assert_eq!(
            paths.staging_path,
            app_data_dir.join("runtime").join(".2026.05.1.staging")
        );
    }

    #[test]
    fn downloads_verifies_extracts_and_activates_remote_runtime_archive() {
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let mut progress = Vec::new();

        let outcome = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_| Ok(Cursor::new(archive_bytes.clone())),
            &mut |operation| progress.push(operation),
        )
        .expect("remote runtime bootstrap should succeed");

        assert!(outcome.archive_path.is_file());
        assert!(outcome.managed_root.join("manifest.json").is_file());
        assert!(outcome.managed_root.join("python/bin/python3").is_file());
        assert!(progress
            .iter()
            .any(|item| item.stage == RuntimeOperationStage::Downloading));
        assert!(progress
            .iter()
            .any(|item| item.stage == RuntimeOperationStage::Hydrating));
        assert!(progress
            .iter()
            .any(|item| item.stage == RuntimeOperationStage::Activating));
        assert!(!outcome.staging_path.exists());
        assert!(!stage_marker_path(app_data_dir.path(), &release.pack_version).exists());
    }

    #[test]
    fn downloads_and_concatenates_multi_part_remote_runtime_archive() {
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let mid = archive_bytes.len() / 2;
        let part1 = archive_bytes[..mid].to_vec();
        let part2 = archive_bytes[mid..].to_vec();

        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let unsigned = BootstrapReleaseManifest {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: crate::runtime::paths::current_runtime_platform(),
            pack_version: "2026.05.1".to_string(),
            archive_url: "https://example.com/runtime-pack.zip.part1".to_string(),
            additional_part_urls: vec!["https://example.com/runtime-pack.zip.part2".to_string()],
            archive_sha256: hex::encode(Sha256::digest(&archive_bytes)),
            archive_size: archive_bytes.len() as u64,
            signature: String::new(),
        };
        let signature = signing_key.sign(unsigned.signature_payload().as_bytes());
        let release = BootstrapReleaseManifest {
            signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
            ..unsigned
        };
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());

        let mut fetched_urls = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |url| {
                fetched_urls.push(url.to_string());
                if url.ends_with("part2") {
                    Ok(Cursor::new(part2.clone()))
                } else {
                    Ok(Cursor::new(part1.clone()))
                }
            },
            &mut |_| {},
        )
        .expect("multi-part remote runtime bootstrap should succeed");

        // Parts are fetched in order, concatenated, then verified against the
        // whole-archive sha256 before extraction.
        assert_eq!(
            fetched_urls,
            vec![
                "https://example.com/runtime-pack.zip.part1".to_string(),
                "https://example.com/runtime-pack.zip.part2".to_string(),
            ]
        );
        assert!(outcome.managed_root.join("manifest.json").is_file());
        assert!(outcome.managed_root.join("python/bin/python3").is_file());
    }

    #[test]
    fn remote_activation_reuses_existing_healthy_runtime_without_replacing_it() {
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);

        let first_outcome = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_| Ok(Cursor::new(archive_bytes.clone())),
            &mut |_| {},
        )
        .expect("first bootstrap should succeed");
        let sentinel = first_outcome
            .managed_root
            .join("venv")
            .join("created-by-deps.txt");
        fs::create_dir_all(sentinel.parent().expect("sentinel parent")).expect("venv dir");
        fs::write(&sentinel, b"keep me").expect("sentinel write");

        let second_outcome = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_| Ok(Cursor::new(archive_bytes.clone())),
            &mut |_| {},
        )
        .expect("second bootstrap should reuse existing healthy runtime");

        assert_eq!(second_outcome.managed_root, first_outcome.managed_root);
        assert!(second_outcome.managed_root.join("manifest.json").is_file());
        assert!(sentinel.is_file());
        assert!(!second_outcome.staging_path.exists());
        assert!(!stage_marker_path(app_data_dir.path(), &release.pack_version).exists());
    }

    #[test]
    fn rejects_remote_archive_when_checksum_does_not_match() {
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (mut release, public_key) = build_signed_release(&archive_bytes);
        release.archive_sha256 = "bad-sha".to_string();
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let signature = signing_key.sign(release.signature_payload().as_bytes());
        release.signature = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

        let error = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_| Ok(Cursor::new(archive_bytes.clone())),
            &mut |_| {},
        )
        .expect_err("checksum mismatch must fail");

        assert!(error.contains("checksum mismatch"));
    }

    #[test]
    fn rejects_remote_archive_when_signature_is_invalid() {
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (mut release, public_key) = build_signed_release(&archive_bytes);
        release.signature = base64::engine::general_purpose::STANDARD.encode([1u8; 64]);

        let error = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_| Ok(Cursor::new(archive_bytes.clone())),
            &mut |_| {},
        )
        .expect_err("signature mismatch must fail");

        assert!(error.contains("signature"));
    }

    #[test]
    fn rejects_remote_archive_with_missing_manifest_entry_after_extract() {
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = SimpleFileOptions::default();
            zip.start_file("manifest.json", options)
                .expect("manifest entry");
            let manifest = RuntimeManifest {
                pack_version: "2026.05.1".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                platform: crate::runtime::paths::current_runtime_platform(),
                payload_profile: "release".to_string(),
                release_injection_required: false,
                external_artifacts_required: vec![],
                python_relpath: "python/bin/python3".to_string(),
                uv_relpath: "uv/bin/uv".to_string(),
                python_files: vec![crate::runtime::manifest::ManifestEntry {
                    path: "python/bin/python3".to_string(),
                    sha256: hex::encode(Sha256::digest(b"python")),
                    size: 6,
                    executable: !cfg!(windows),
                }],
                uv_files: vec![],
                script_files: vec![],
                wheelhouse: vec![],
                caches: vec![],
                native_assets: vec![],
            };
            zip.write_all(&serde_json::to_vec_pretty(&manifest).expect("manifest json"))
                .expect("manifest bytes");
            zip.finish().expect("finish zip");
        }
        let archive_bytes = buffer.into_inner();
        let (release, public_key) = build_signed_release(&archive_bytes);

        let error = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_| Ok(Cursor::new(archive_bytes.clone())),
            &mut |_| {},
        )
        .expect_err("missing extracted file must fail");

        assert!(error.contains("missing") || error.contains("Missing"));
        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        assert!(!paths.staging_path.exists());
        assert!(!stage_marker_path(app_data_dir.path(), &release.pack_version).exists());
    }

    #[test]
    fn writes_resume_metadata_after_download() {
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);

        let outcome = download_and_activate_remote_runtime_with_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_| Ok(Cursor::new(archive_bytes.clone())),
            &mut |_| {},
        )
        .expect("download should succeed");
        let metadata_path = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        )
        .resume_metadata_path;

        let metadata: BootstrapResumeMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).expect("read metadata"))
                .expect("parse metadata");

        assert_eq!(metadata.archive_url, release.archive_url);
        assert_eq!(metadata.downloaded_bytes, release.archive_size);
        assert_eq!(
            outcome.managed_root,
            app_data_dir.path().join("runtime").join("2026.05.1")
        );
    }

    struct FailAfterReader {
        data: Vec<u8>,
        pos: usize,
        fail_after: usize,
    }

    impl FailAfterReader {
        fn new(data: Vec<u8>, fail_after: usize) -> Self {
            Self {
                data,
                pos: 0,
                fail_after,
            }
        }
    }

    impl std::io::Read for FailAfterReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.fail_after {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "simulated network failure",
                ));
            }
            let remaining =
                (self.fail_after - self.pos).min(self.data.len().saturating_sub(self.pos));
            if remaining == 0 {
                return Ok(0);
            }
            let n = buf.len().min(remaining);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    fn build_signed_two_part_release(
        archive_bytes: &[u8],
        part_urls: (&str, &str),
    ) -> (BootstrapReleaseManifest, String) {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let unsigned = BootstrapReleaseManifest {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: crate::runtime::paths::current_runtime_platform(),
            pack_version: "2026.05.1".to_string(),
            archive_url: part_urls.0.to_string(),
            additional_part_urls: vec![part_urls.1.to_string()],
            archive_sha256: hex::encode(Sha256::digest(archive_bytes)),
            archive_size: archive_bytes.len() as u64,
            signature: String::new(),
        };
        let signature = signing_key.sign(unsigned.signature_payload().as_bytes());
        let release = BootstrapReleaseManifest {
            signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
            ..unsigned
        };
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());
        (release, public_key)
    }

    #[test]
    fn resumes_from_partial_bytes_with_range_request() {
        use super::FetchResponse;
        use crate::runtime::download_state::resume_state_path;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let mid = archive_bytes.len() / 2;
        let part1 = archive_bytes[..mid].to_vec();
        let part2 = archive_bytes[mid..].to_vec();
        let (release, public_key) = build_signed_two_part_release(
            &archive_bytes,
            ("https://example.com/p1", "https://example.com/p2"),
        );

        // First attempt: part 1 completes, part 2 is interrupted after 10 bytes.
        let first = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |url, _range| {
                if url.ends_with("p2") {
                    Ok(FetchResponse {
                        reader: Box::new(FailAfterReader::new(part2.clone(), 10))
                            as Box<dyn std::io::Read>,
                        resumed: false,
                        content_range_start: None,
                        range_not_satisfiable: false,
                    })
                } else {
                    Ok(FetchResponse {
                        reader: Box::new(Cursor::new(part1.clone())) as Box<dyn std::io::Read>,
                        resumed: false,
                        content_range_start: None,
                        range_not_satisfiable: false,
                    })
                }
            },
            &mut |_| {},
        );
        assert!(first.is_err(), "first attempt must be interrupted");

        // Second attempt: completed part 1 is skipped, part 2 resumes via Range.
        let mut observed: Vec<(String, Option<u64>)> = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |url, range| {
                observed.push((url.to_string(), range));
                if url.ends_with("p2") {
                    let from = range.expect("part 2 must resume with a Range offset") as usize;
                    assert_eq!(from, 10);
                    Ok(FetchResponse {
                        reader: Cursor::new(part2[from..].to_vec()),
                        resumed: true,
                        content_range_start: Some(from as u64),
                        range_not_satisfiable: false,
                    })
                } else {
                    panic!("completed part 1 must not be re-requested, got {url}");
                }
            },
            &mut |_| {},
        )
        .expect("resumed bootstrap should succeed");

        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].0, "https://example.com/p2");
        let final_bytes = fs::read(&outcome.archive_path).expect("read final archive");
        assert_eq!(final_bytes, archive_bytes);
        assert!(outcome.managed_root.join("manifest.json").is_file());
        assert!(!resume_state_path(app_data_dir.path(), &release.pack_version).exists());
    }

    #[test]
    fn keeps_partial_bytes_after_network_failure() {
        use super::FetchResponse;
        use crate::runtime::download_state::resume_state_path;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let cut = 100usize.min(archive_bytes.len() / 2).max(1);

        let error = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, _range| {
                Ok(FetchResponse {
                    reader: FailAfterReader::new(archive_bytes.clone(), cut),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect_err("network failure must error");
        assert!(
            error.contains("Failed while reading"),
            "unexpected error: {error}"
        );

        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        let tmp = paths.archive_path.with_extension("download.tmp");
        assert!(tmp.is_file(), "partial tmp must be kept");
        assert_eq!(fs::metadata(&tmp).expect("tmp metadata").len(), cut as u64);
        let state_path = resume_state_path(app_data_dir.path(), &release.pack_version);
        assert!(state_path.is_file(), "resume state must be kept");
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(&state_path).expect("read state"))
                .expect("parse state");
        assert_eq!(state["archive_sha256"], release.archive_sha256);
        assert!(!paths.archive_path.exists());
    }

    #[test]
    fn discards_stale_state_for_a_different_release() {
        use super::FetchResponse;
        use crate::runtime::download::bootstrap_download_dir;
        use crate::runtime::download_state::resume_state_path;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);

        // Seed stale state: same staging dir, but a different release identity.
        let dir = bootstrap_download_dir(app_data_dir.path(), &release.pack_version);
        fs::create_dir_all(&dir).expect("download dir");
        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        let tmp = paths.archive_path.with_extension("download.tmp");
        fs::write(&tmp, b"stale-garbage").expect("seed tmp");
        let stale = serde_json::json!({
            "pack_version": release.pack_version,
            "archive_sha256": "stale-sha",
            "archive_size": release.archive_size + 999,
            "archive_url": release.archive_url,
            "additional_part_urls": [],
            "parts": [],
            "total_downloaded": 13u64,
        });
        let state_path = resume_state_path(app_data_dir.path(), &release.pack_version);
        fs::write(
            &state_path,
            serde_json::to_vec_pretty(&stale).expect("stale json"),
        )
        .expect("seed state");

        let mut observed: Vec<(String, Option<u64>)> = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |url, range| {
                observed.push((url.to_string(), range));
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes.clone()),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect("download after stale state should succeed");

        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].1, None, "stale state must restart from byte 0");
        assert_eq!(
            fs::read(&outcome.archive_path).expect("final archive"),
            archive_bytes
        );
    }

    #[test]
    fn restarts_part_when_server_ignores_range() {
        use super::FetchResponse;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let cut = 50usize.min(archive_bytes.len() / 2).max(1);

        let first = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, _range| {
                Ok(FetchResponse {
                    reader: FailAfterReader::new(archive_bytes.clone(), cut),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        );
        assert!(first.is_err(), "first attempt must be interrupted");

        // Server ignores the Range request: full 200 body despite the offset.
        let mut observed: Vec<Option<u64>> = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, range| {
                observed.push(range);
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes.clone()),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect("restart after ignored Range should succeed");

        assert_eq!(observed, vec![Some(cut as u64)]);
        assert_eq!(
            fs::read(&outcome.archive_path).expect("final archive"),
            archive_bytes
        );
        assert!(outcome.managed_root.join("manifest.json").is_file());
    }

    #[test]
    fn verification_failure_still_removes_partial_data() {
        use super::FetchResponse;
        use crate::runtime::download_state::resume_state_path;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (mut release, public_key) = build_signed_release(&archive_bytes);
        release.archive_sha256 = "bad-sha".to_string();
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let signature = signing_key.sign(release.signature_payload().as_bytes());
        release.signature = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

        let error = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, _range| {
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes.clone()),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect_err("checksum mismatch must fail");
        assert!(error.contains("checksum mismatch"));

        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        let tmp = paths.archive_path.with_extension("download.tmp");
        assert!(!tmp.exists(), "tmp must be removed on verification failure");
        assert!(
            !paths.archive_path.exists(),
            "archive must be removed on verification failure"
        );
        assert!(
            !resume_state_path(app_data_dir.path(), &release.pack_version).exists(),
            "resume state must be removed on verification failure"
        );
        assert!(
            !paths.resume_metadata_path.exists(),
            "completed resume record must not remain on verification failure"
        );
    }

    #[test]
    fn stall_timeout_not_total_timeout() {
        use super::{bootstrap_timeout_config, build_bootstrap_client};
        use std::time::Duration;

        // Single source of truth the client builder consumes: a ~2.44 GB
        // transfer must never face a total deadline — only a connect timeout
        // plus a per-read stall timeout (no bytes for N seconds).
        let config = bootstrap_timeout_config();
        assert!(config.connect_timeout >= Duration::from_secs(1));
        assert!(config.read_stall_timeout >= Duration::from_secs(30));
        assert_eq!(config.total_timeout, None);
        // Reintroducing a total deadline must flow through this struct and
        // fails the assertion above: the builder takes no other timeout
        // input (reqwest's blocking `timeout` is per-read, never total).
        build_bootstrap_client().expect("bootstrap client builds");
    }

    /// 3 MB activatable archive: same manifest/entries as
    /// `runtime_archive_bytes` plus an inert ~3 MB stored blob so the 1 MB
    /// periodic resume persist triggers mid-download. The blob is stored
    /// uncompressed — a constant-filled deflated entry would shrink to
    /// nothing and defeat the purpose.
    fn large_runtime_archive_bytes() -> Vec<u8> {
        use std::io::Write;
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = SimpleFileOptions::default();
            zip.add_directory("python/", options).expect("python dir");
            zip.add_directory("python/bin/", options)
                .expect("python bin dir");
            zip.add_directory("uv/", options).expect("uv dir");
            zip.add_directory("uv/bin/", options).expect("uv bin dir");
            zip.add_directory("payload/", options).expect("payload dir");
            zip.start_file("python/bin/python3", options)
                .expect("python entry");
            zip.write_all(b"python").expect("python bytes");
            zip.start_file("uv/bin/uv", options).expect("uv entry");
            zip.write_all(b"uv").expect("uv bytes");
            zip.start_file(
                "payload/blob.bin",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .expect("blob entry");
            let chunk = [0xABu8; 64 * 1024];
            for _ in 0..48 {
                zip.write_all(&chunk).expect("blob bytes");
            }
            let manifest = crate::runtime::manifest::RuntimeManifest {
                pack_version: "2026.05.1".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                platform: crate::runtime::paths::current_runtime_platform(),
                payload_profile: "release".to_string(),
                release_injection_required: false,
                external_artifacts_required: vec![],
                python_relpath: "python/bin/python3".to_string(),
                uv_relpath: "uv/bin/uv".to_string(),
                python_files: vec![crate::runtime::manifest::ManifestEntry {
                    path: "python/bin/python3".to_string(),
                    sha256: hex::encode(Sha256::digest(b"python")),
                    size: 6,
                    executable: !cfg!(windows),
                }],
                uv_files: vec![crate::runtime::manifest::ManifestEntry {
                    path: "uv/bin/uv".to_string(),
                    sha256: hex::encode(Sha256::digest(b"uv")),
                    size: 2,
                    executable: true,
                }],
                script_files: vec![],
                wheelhouse: vec![],
                caches: vec![],
                native_assets: vec![],
            };
            zip.start_file("manifest.json", options)
                .expect("manifest entry");
            zip.write_all(&serde_json::to_vec_pretty(&manifest).expect("manifest json"))
                .expect("manifest bytes");
            zip.finish().expect("finish zip");
        }
        buffer.into_inner()
    }

    /// Dies at `kill_after` bytes like a SIGKILL: snapshots whatever resume
    /// state made it to disk, then fails without running any error-path
    /// save. The test restores the snapshot afterwards, reproducing exactly
    /// the on-disk state a real kill leaves behind.
    struct KillSnapshotReader {
        data: Vec<u8>,
        pos: usize,
        kill_after: usize,
        state_path: PathBuf,
        snapshot: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    }

    impl std::io::Read for KillSnapshotReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.kill_after {
                let snapshot = fs::read(&self.state_path).ok();
                *self.snapshot.lock().expect("snapshot lock") = snapshot;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "simulated hard kill",
                ));
            }
            let remaining =
                (self.kill_after - self.pos).min(self.data.len().saturating_sub(self.pos));
            if remaining == 0 {
                return Ok(0);
            }
            let n = buf.len().min(remaining);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn resumes_after_hard_kill_between_periodic_persists() {
        use super::FetchResponse;
        use crate::runtime::download_state::resume_state_path;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = large_runtime_archive_bytes();
        assert!(
            archive_bytes.len() > 1_500_000,
            "fixture must exceed the kill point"
        );
        let (release, public_key) = build_signed_release(&archive_bytes);
        let state_path = resume_state_path(app_data_dir.path(), &release.pack_version);
        let kill_at = 1_500_000usize;

        let snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let first = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, range| {
                assert_eq!(range, None, "first attempt starts from zero");
                Ok(FetchResponse {
                    reader: KillSnapshotReader {
                        data: archive_bytes.clone(),
                        pos: 0,
                        kill_after: kill_at,
                        state_path: state_path.clone(),
                        snapshot: snapshot.clone(),
                    },
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        );
        assert!(first.is_err(), "killed attempt must fail");

        // Restore the kill-time snapshot over the error-path save: this is the
        // on-disk state a real SIGKILL leaves (persisted at 1 MB, tmp at kill).
        let killed_state = snapshot
            .lock()
            .expect("snapshot lock")
            .take()
            .expect("resume state must have been persisted at 1 MB");
        fs::write(&state_path, &killed_state).expect("restore kill snapshot");
        let killed: serde_json::Value = serde_json::from_slice(&killed_state).expect("parse state");
        assert_eq!(killed["total_downloaded"].as_u64(), Some(1_048_576));

        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        let tmp = paths.archive_path.with_extension("download.tmp");
        assert_eq!(
            fs::metadata(&tmp).expect("tmp metadata").len(),
            kill_at as u64
        );

        // The second attempt must resume from the recorded 1 MB offset even
        // though the tmp file ran ahead to the kill point.
        let mut observed: Vec<Option<u64>> = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, range| {
                observed.push(range);
                let from = range.expect("must resume with Range after hard kill") as usize;
                assert_eq!(from, 1_048_576);
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes[from..].to_vec()),
                    resumed: true,
                    content_range_start: Some(from as u64),
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect("resume after hard kill should succeed");

        assert_eq!(observed, vec![Some(1_048_576u64)]);
        assert_eq!(
            fs::read(&outcome.archive_path).expect("final archive"),
            archive_bytes
        );
        assert!(outcome.managed_root.join("manifest.json").is_file());
    }

    #[test]
    fn restarts_from_zero_when_state_missing_but_partial_present() {
        use super::FetchResponse;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        let tmp = paths.archive_path.with_extension("download.tmp");
        fs::create_dir_all(tmp.parent().expect("tmp parent")).expect("download dir");
        fs::write(&tmp, b"orphan-partial-bytes").expect("seed tmp");

        let mut observed: Vec<Option<u64>> = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, range| {
                observed.push(range);
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes.clone()),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect("download without state should succeed");

        assert_eq!(observed, vec![None], "no state means restart from zero");
        assert_eq!(
            fs::read(&outcome.archive_path).expect("final archive"),
            archive_bytes
        );
    }

    #[test]
    fn restarts_part_from_zero_on_416_range_not_satisfiable() {
        use super::FetchResponse;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let cut = 50usize.min(archive_bytes.len() / 2).max(1);

        let first = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, _range| {
                Ok(FetchResponse {
                    reader: FailAfterReader::new(archive_bytes.clone(), cut),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        );
        assert!(first.is_err(), "first attempt must be interrupted");

        // Server answers 416 to the Range resume: the recorded offset is
        // invalid, so the part restarts from zero with a clean refetch.
        let mut observed: Vec<Option<u64>> = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, range| {
                observed.push(range);
                if range.is_some() {
                    return Ok(FetchResponse {
                        reader: Cursor::new(Vec::new()),
                        resumed: false,
                        content_range_start: None,
                        range_not_satisfiable: true,
                    });
                }
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes.clone()),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect("restart after 416 should succeed");

        assert_eq!(observed, vec![Some(cut as u64), None]);
        assert_eq!(
            fs::read(&outcome.archive_path).expect("final archive"),
            archive_bytes
        );
        assert!(outcome.managed_root.join("manifest.json").is_file());
    }

    #[test]
    fn restarts_part_when_content_range_start_mismatches() {
        use super::FetchResponse;

        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);
        let cut = 50usize.min(archive_bytes.len() / 2).max(1);

        let first = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, _range| {
                Ok(FetchResponse {
                    reader: FailAfterReader::new(archive_bytes.clone(), cut),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        );
        assert!(first.is_err(), "first attempt must be interrupted");

        // 206 whose Content-Range start does not match the requested offset:
        // the body is unusable mid-stream, so restart and refetch cleanly.
        let mut observed: Vec<Option<u64>> = Vec::new();
        let outcome = download_and_activate_remote_runtime_with_ranged_fetch(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, range| {
                observed.push(range);
                if range.is_some() {
                    let wrong_from = 7u64;
                    assert_ne!(wrong_from, cut as u64);
                    return Ok(FetchResponse {
                        reader: Cursor::new(archive_bytes[wrong_from as usize..].to_vec()),
                        resumed: true,
                        content_range_start: Some(wrong_from),
                        range_not_satisfiable: false,
                    });
                }
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes.clone()),
                    resumed: false,
                    content_range_start: None,
                    range_not_satisfiable: false,
                })
            },
            &mut |_| {},
        )
        .expect("restart after Content-Range mismatch should succeed");

        assert_eq!(observed, vec![Some(cut as u64), None]);
        assert_eq!(
            fs::read(&outcome.archive_path).expect("final archive"),
            archive_bytes
        );
        assert!(outcome.managed_root.join("manifest.json").is_file());
    }

    // ------------------------------------------------------------------
    // T2 (S2, S5): cancel token + non-blocking single-flight entry. All
    // fetchers below are in-memory fakes: these tests never touch the
    // network (S8).
    // ------------------------------------------------------------------

    /// Serializes the T2 tests that touch the process-wide single-flight
    /// `ops_lock` (or the command-layer cancel flag). The 74 pre-existing
    /// runtime tests never touch either, so serializing only among
    /// ourselves keeps every test hermetic under parallel execution.
    static T2_OPS_LOCK_SERIALIZER: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_t2_serializer() -> std::sync::MutexGuard<'static, ()> {
        T2_OPS_LOCK_SERIALIZER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A reader that arms the shared cancel token after `cancel_after` reads
    /// and counts every non-empty `read`, so the test can bound how many
    /// chunks the download loop consumed after the cancel.
    struct CancelAfterNReads<R> {
        inner: R,
        cancel_after: usize,
        reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        token: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl<R: std::io::Read> std::io::Read for CancelAfterNReads<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            if n > 0 {
                let reads = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if reads >= self.cancel_after {
                    self.token.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            Ok(n)
        }
    }

    #[test]
    fn cancel_stops_within_one_chunk_and_releases_the_lock() {
        use super::FetchResponse;
        use crate::runtime::download_state::resume_state_path;

        let _serial = lock_t2_serializer();
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        // ~3 MB activatable fixture: dozens of 64 KiB chunks, so stopping
        // after a handful of chunks proves the loop exits promptly instead
        // of draining the whole archive.
        let archive_bytes = large_runtime_archive_bytes();
        let total_chunks = archive_bytes.len().div_ceil(64 * 1024);
        assert!(total_chunks > 20, "fixture must span many chunks");
        let (release, public_key) = build_signed_release(&archive_bytes);

        let cancel_after = 5usize;
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let data_dir = app_data_dir.path().to_path_buf();

        let first = super::spawn_bootstrap_download("t2-cancel-stops", {
            let reads = reads.clone();
            let token = token.clone();
            let bytes = archive_bytes.clone();
            let release = release.clone();
            let public_key = public_key.clone();
            move || {
                let mut progress = Vec::new();
                let result =
                    super::download_and_activate_remote_runtime_with_ranged_fetch_and_cancel(
                        "https://example.com/runtime/bootstrap.json",
                        &release,
                        &data_dir,
                        &public_key,
                        |_url, _range| {
                            Ok(FetchResponse {
                                reader: CancelAfterNReads {
                                    inner: Cursor::new(bytes.clone()),
                                    cancel_after,
                                    reads: reads.clone(),
                                    token: token.clone(),
                                },
                                resumed: false,
                                content_range_start: None,
                                range_not_satisfiable: false,
                            })
                        },
                        Some(token.as_ref()),
                        &mut |operation| progress.push(operation),
                    );
                (result, progress)
            }
        });
        let handle = match first {
            super::BootstrapSpawn::Started(handle) => handle,
            super::BootstrapSpawn::AlreadyRunning => {
                panic!("ops lock must be free for the first start")
            }
        };
        let (result, progress) = handle.join().expect("worker thread joins");

        // Cancel surfaces as the quiet sentinel, not a transfer error.
        let error = result.expect_err("cancelled download must fail");
        assert_eq!(error, super::BOOTSTRAP_DOWNLOAD_CANCELLED_MESSAGE);
        assert!(super::is_bootstrap_cancelled_error(&error));

        // The loop exits within one chunk of the cancel, not at the end of
        // the archive: the token arms on read #5, so at most one more chunk
        // can slip through before the per-chunk check fires.
        let chunks_read = reads.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            chunks_read <= cancel_after + 1,
            "cancel must stop promptly, read {chunks_read} chunks"
        );
        assert!(
            chunks_read < total_chunks,
            "must not drain the whole archive ({total_chunks} chunks)"
        );

        // Progress reporting keeps firing while cancellable: the
        // `runtime://progress` listener path observes Downloading stages.
        assert!(
            progress
                .iter()
                .any(|operation| operation.stage == RuntimeOperationStage::Downloading),
            "cancellable download must still emit Downloading progress"
        );

        // Resumable partial state is kept on disk with T1 semantics.
        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        let tmp = paths.archive_path.with_extension("download.tmp");
        assert!(tmp.is_file(), "partial tmp must be kept for resume");
        assert_eq!(
            fs::metadata(&tmp).expect("tmp metadata").len(),
            chunks_read as u64 * 64 * 1024,
            "staged bytes must match the chunks consumed before the cancel"
        );
        assert!(
            resume_state_path(app_data_dir.path(), &release.pack_version).is_file(),
            "resume state must be kept for resume"
        );
        assert!(!paths.archive_path.exists());

        // The ops lock is free afterwards: a probe acquires it, and a
        // second start proceeds — resuming from the kept offset, not
        // restarting from zero.
        drop(
            crate::runtime::ops_lock::try_acquire("t2-cancel-probe")
                .expect("ops lock must be released after cancel"),
        );
        let expected_offset = chunks_read as u64 * 64 * 1024;
        let observed_ranges = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let second = super::spawn_bootstrap_download("t2-cancel-resume", {
            let bytes = archive_bytes.clone();
            let release = release.clone();
            let public_key = public_key.clone();
            let data_dir = app_data_dir.path().to_path_buf();
            let observed_ranges = observed_ranges.clone();
            move || {
                let fresh_token = std::sync::atomic::AtomicBool::new(false);
                super::download_and_activate_remote_runtime_with_ranged_fetch_and_cancel(
                    "https://example.com/runtime/bootstrap.json",
                    &release,
                    &data_dir,
                    &public_key,
                    |_url, range| {
                        observed_ranges
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push(range);
                        let from = range.expect("resume must carry a Range offset") as usize;
                        Ok(FetchResponse {
                            reader: Cursor::new(bytes[from..].to_vec()),
                            resumed: true,
                            content_range_start: Some(from as u64),
                            range_not_satisfiable: false,
                        })
                    },
                    Some(&fresh_token),
                    &mut |_| {},
                )
            }
        });
        let outcome = match second {
            super::BootstrapSpawn::Started(handle) => handle.join().expect("worker thread joins"),
            super::BootstrapSpawn::AlreadyRunning => {
                panic!("released lock must let the second start proceed")
            }
        }
        .expect("resumed download after cancel must succeed");
        assert_eq!(
            *observed_ranges
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![Some(expected_offset)]
        );
        assert_eq!(
            fs::read(&outcome.archive_path).expect("final archive"),
            archive_bytes
        );
        assert!(outcome.managed_root.join("manifest.json").is_file());
    }

    #[test]
    fn cancel_is_quiet_not_an_error() {
        use super::FetchResponse;
        use crate::runtime::download_state::resume_state_path;

        let _serial = lock_t2_serializer();
        let app_data_dir = tempfile::tempdir().expect("app data dir");
        let archive_bytes = large_runtime_archive_bytes();
        let (release, public_key) = build_signed_release(&archive_bytes);

        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = {
            let bytes = archive_bytes.clone();
            super::download_and_activate_remote_runtime_with_ranged_fetch_and_cancel(
                "https://example.com/runtime/bootstrap.json",
                &release,
                app_data_dir.path(),
                &public_key,
                |_url, _range| {
                    Ok(FetchResponse {
                        reader: CancelAfterNReads {
                            inner: Cursor::new(bytes.clone()),
                            cancel_after: 3,
                            reads: reads.clone(),
                            token: token.clone(),
                        },
                        resumed: false,
                        content_range_start: None,
                        range_not_satisfiable: false,
                    })
                },
                Some(token.as_ref()),
                &mut |_| {},
            )
            .expect_err("cancelled download must fail")
        };

        // Quiet cancel: exactly the sentinel — never the network-failure
        // shape ("Failed while ...") nor the verification-failure shape
        // (checksum/signature mismatch, which deletes the partial). T3 owns
        // the status copy; the core-level contract is this distinguishable
        // sentinel plus kept resumable state, i.e. no error latch.
        assert_eq!(error, super::BOOTSTRAP_DOWNLOAD_CANCELLED_MESSAGE);
        assert!(super::is_bootstrap_cancelled_error(&error));
        assert!(
            !error.contains("Failed while"),
            "cancel must not look like a network failure: {error}"
        );
        assert!(
            !error.to_ascii_lowercase().contains("mismatch")
                && !error.to_ascii_lowercase().contains("checksum")
                && !error.to_ascii_lowercase().contains("signature"),
            "cancel must not look like a verification failure: {error}"
        );

        let paths = bootstrap_download_plan_paths(
            app_data_dir.path(),
            &release.pack_version,
            "runtime-pack.zip",
        );
        assert!(
            paths.archive_path.with_extension("download.tmp").is_file(),
            "cancel must keep the partial for resume"
        );
        assert!(
            resume_state_path(app_data_dir.path(), &release.pack_version).is_file(),
            "cancel must keep the resume state"
        );

        // ...and the retry resumes from the kept offset instead of failing.
        let staged = 3u64 * 64 * 1024;
        let mut observed_ranges = Vec::new();
        let fresh_token = std::sync::atomic::AtomicBool::new(false);
        super::download_and_activate_remote_runtime_with_ranged_fetch_and_cancel(
            "https://example.com/runtime/bootstrap.json",
            &release,
            app_data_dir.path(),
            &public_key,
            |_url, range| {
                observed_ranges.push(range);
                let from = range.expect("resume must carry a Range offset") as usize;
                Ok(FetchResponse {
                    reader: Cursor::new(archive_bytes[from..].to_vec()),
                    resumed: true,
                    content_range_start: Some(from as u64),
                    range_not_satisfiable: false,
                })
            },
            Some(&fresh_token),
            &mut |_| {},
        )
        .expect("retry after a quiet cancel must resume and succeed");
        assert_eq!(observed_ranges, vec![Some(staged)]);
    }

    #[test]
    fn start_is_idempotent_while_running() {
        // Simulate an in-flight download holding the single-flight lock: the
        // second start must report busy WITHOUT opening a second fetch.
        let _serial = lock_t2_serializer();
        let _in_flight = crate::runtime::ops_lock::try_acquire("t2-idempotent-first")
            .expect("ops lock must be free at test start");

        let fetches = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let second = super::spawn_bootstrap_download("t2-idempotent-second", {
            let fetches = fetches.clone();
            move || {
                fetches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        assert!(
            matches!(second, super::BootstrapSpawn::AlreadyRunning),
            "second start while one is in flight must be idempotent"
        );
        assert_eq!(
            fetches.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a busy start must not open a second fetch"
        );

        // Once the lock is released, a later start proceeds exactly once.
        drop(_in_flight);
        let third = super::spawn_bootstrap_download("t2-idempotent-third", {
            let fetches = fetches.clone();
            move || {
                fetches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        match third {
            super::BootstrapSpawn::Started(handle) => handle.join().expect("worker joins"),
            super::BootstrapSpawn::AlreadyRunning => {
                panic!("released lock must let the next start proceed")
            }
        }
        assert_eq!(fetches.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn start_does_not_block_until_the_fetch_completes() {
        let _serial = lock_t2_serializer();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        // The spawner runs on its own thread: if the implementation ever
        // executed the blocking core inline, no prompt return would arrive
        // and the bounded wait below fails instead of hanging the suite.
        let (returned_tx, returned_rx) = std::sync::mpsc::channel::<()>();
        let spawner = std::thread::spawn({
            let entered = entered.clone();
            let finished = finished.clone();
            move || {
                let outcome = super::spawn_bootstrap_download("t2-nonblocking", move || {
                    entered.store(true, std::sync::atomic::Ordering::SeqCst);
                    let _ = release_rx.recv();
                    finished.store(true, std::sync::atomic::Ordering::SeqCst);
                });
                returned_tx.send(()).expect("signal prompt return");
                outcome
            }
        });

        returned_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("start must return before the blocking fetch completes");
        // The worker may not be scheduled the instant spawn returns: wait
        // for it with a bound instead of asserting instantly (that races
        // the scheduler and flakes).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !entered.load(std::sync::atomic::Ordering::SeqCst) {
            if std::time::Instant::now() > deadline {
                panic!("the worker must have started the blocking work");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            !finished.load(std::sync::atomic::Ordering::SeqCst),
            "the blocking work must still be pending after start returned"
        );

        drop(release_tx);
        let outcome = spawner.join().expect("spawner joins");
        match outcome {
            super::BootstrapSpawn::Started(handle) => handle.join().expect("worker joins"),
            super::BootstrapSpawn::AlreadyRunning => panic!("lock was free"),
        }
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn start_command_is_async() {
        // Compile-time contract: both Tauri commands must stay `async` so
        // the bootstrap never runs on the main thread. De-asyncing either
        // command breaks this bound and fails the build.
        fn assert_async_command<F, Fut>(_: F)
        where
            F: FnOnce(tauri::AppHandle) -> Fut,
            Fut: std::future::Future,
        {
        }
        fn assert_async_command_without_args<F, Fut>(_: F)
        where
            F: FnOnce() -> Fut,
            Fut: std::future::Future,
        {
        }
        assert_async_command(crate::runtime::runtime_bootstrap_start);
        assert_async_command_without_args(crate::runtime::runtime_cancel_download);
    }

    #[test]
    fn command_layer_cancel_flag_round_trips() {
        // The flag the `runtime_cancel_download` command sets and the
        // download loop observes: set is observable, clear disarms. Kept
        // instantaneous on purpose — no download runs here, so parallel
        // download tests can never observe the set window.
        let _serial = lock_t2_serializer();
        assert!(!super::is_bootstrap_cancel_requested());
        super::request_bootstrap_cancel();
        assert!(super::is_bootstrap_cancel_requested());
        super::clear_bootstrap_cancel();
        assert!(!super::is_bootstrap_cancel_requested());
    }
}
