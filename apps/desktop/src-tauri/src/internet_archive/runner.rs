//! Job runner: downloads every `Queued` file of a job one at a time,
//! verifies the staged PDF, and records `sha256` / `page_count`.
//!
//! The testable core is [`run_job`]: it never touches Tauri, never takes a
//! path or URL from the caller (only a job id validated by the store plus
//! file indexes in order), and reports progress through `on_update` so the
//! production wrapper can persist + emit without holding a lock across an
//! `.await`.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use internet_archive::download::DownloadLimits;
use internet_archive::job::{FileState, Job, JobState};
use internet_archive::store::JobStore;
use internet_archive::verify::{ExpectedRemote, PageLimits};
use internet_archive::{Endpoints, IaClient, IaError};

/// One file download, abstracted so tests inject fakes without the network.
pub(crate) trait Downloader {
    async fn download(
        &self,
        identifier: &str,
        remote_name: &str,
        dest: &Path,
        expected_size: Option<u64>,
    ) -> Result<u64, IaError>;
}

/// Production downloader over [`IaClient::download_file_with_retry`].
pub(crate) struct LiveDownloader<'a> {
    pub client: &'a IaClient,
    pub endpoints: &'a Endpoints,
    pub limits: DownloadLimits,
    pub retry: internet_archive::RetryPolicy,
}

impl Downloader for LiveDownloader<'_> {
    async fn download(
        &self,
        identifier: &str,
        remote_name: &str,
        dest: &Path,
        expected_size: Option<u64>,
    ) -> Result<u64, IaError> {
        self.client
            .download_file_with_retry(
                self.endpoints,
                identifier,
                remote_name,
                dest,
                &self.limits,
                expected_size,
                &self.retry,
            )
            .await
    }
}

/// Whether a job state may start (or resume) downloading: `Draft`, `Ready`
/// and `Running` are accepted, everything else is `invalid_state`.
pub(crate) fn decide_start(state: JobState) -> Result<(), String> {
    match state {
        JobState::Draft | JobState::Ready | JobState::Running => Ok(()),
        _ => Err(format!(
            "invalid_state: cannot start a job in state {state:?}"
        )),
    }
}

fn store_load_error(error: internet_archive::store::StoreError) -> String {
    use internet_archive::store::StoreError;
    match error {
        StoreError::NotFound(_) => "not_found: import job not found".to_string(),
        StoreError::InvalidId(_) => "invalid_request: invalid job id".to_string(),
        StoreError::UnsupportedVersion { .. } => "io: unsupported job version".to_string(),
        StoreError::AlreadyExists(_) => "io: job already exists".to_string(),
        StoreError::Io(_) | StoreError::Json(_) => "io: local storage error".to_string(),
    }
}

/// Load → modify → save while holding the single writer lock. Only
/// synchronous file I/O happens under it; never held across an `.await`.
/// Returns the closure result plus the fresh job for `on_update`.
/// Lock order elsewhere is `active` first, then `jobs`; this helper only
/// ever takes `jobs`.
pub(crate) fn update<R>(
    store: &JobStore,
    jobs: &Mutex<()>,
    job_id: &str,
    f: impl FnOnce(&mut Job) -> Result<R, String>,
) -> Result<(R, Job), String> {
    let _guard = jobs
        .lock()
        .map_err(|_| "io: local storage error".to_string())?;
    let mut job = store.load(job_id).map_err(store_load_error)?;
    let result = f(&mut job)?;
    store
        .save(&job)
        .map_err(|_| "io: cannot persist the import job".to_string())?;
    Ok((result, job))
}

/// `request_cancel` + staging deletion + `finish_cancel_if_idle` on an
/// in-memory job already owned under the `jobs` lock.
fn apply_cancel_locked(store: &JobStore, job: &mut Job, now: u64) {
    let job_id = job.id.clone();
    let effects = job.request_cancel(now);
    for name in &effects.staging_to_delete {
        delete_dest_and_partial(store, &job_id, name);
    }
    job.finish_cancel_if_idle(now);
}

/// End-of-run / direct-path cancel: if `cancelled` and the job is not
/// terminal, apply the same cancel effects the direct path applies (files
/// `Cancelled`, staging removed) and save. Returns whether cancel was
/// applied plus the fresh job.
pub(crate) fn apply_end_of_run_cancel(
    store: &JobStore,
    jobs: &Mutex<()>,
    job_id: &str,
    cancelled: bool,
    now: u64,
) -> Result<(bool, Job), String> {
    if !cancelled {
        let _guard = jobs
            .lock()
            .map_err(|_| "io: local storage error".to_string())?;
        let job = store.load(job_id).map_err(store_load_error)?;
        return Ok((false, job));
    }
    update(store, jobs, job_id, |job| {
        if job.state.is_terminal() {
            return Ok(false);
        }
        apply_cancel_locked(store, job, now);
        Ok(true)
    })
}

/// Delete both `<dest>` and its `<dest>.partial` best-effort. Dropping a
/// download future can leave the partial behind, so both go.
fn delete_dest_and_partial(store: &JobStore, job_id: &str, staging_name: &str) {
    let _ = store.delete_staging(job_id, staging_name);
    if let Ok(dest) = store.staging_path(job_id, staging_name) {
        let _ = std::fs::remove_file(internet_archive::partial_path(&dest));
        // `delete_staging` already removed `dest`; this second remove is a
        // no-op when it worked, and covers a path the store refused.
        let _ = std::fs::remove_file(&dest);
    }
}

/// Map a download error to `(next file state, machine code)`. Codes only,
/// never paths.
fn map_download_error(error: &IaError) -> (FileState, String) {
    match error {
        IaError::Http {
            status: 401 | 403, ..
        } => (FileState::Unavailable, "unavailable".to_string()),
        IaError::Http { status: 404, .. } => (FileState::Failed, "not_found".to_string()),
        IaError::FileTooLarge { .. } => (FileState::Failed, "file_too_large".to_string()),
        IaError::Http { status: 429, .. } => (FileState::Failed, "rate_limited".to_string()),
        IaError::Http { status, .. } => (FileState::Failed, format!("http_{status}")),
        IaError::VerifyFailed { .. } => (FileState::Failed, "verify_failed".to_string()),
        IaError::TooManyPages { .. } => (FileState::Failed, "too_many_pages".to_string()),
        _ => (FileState::Failed, "network".to_string()),
    }
}

fn map_verify_error(error: &IaError) -> String {
    match error {
        IaError::TooManyPages { .. } => "too_many_pages".to_string(),
        IaError::VerifyFailed { .. } => "verify_failed".to_string(),
        IaError::Http { status, .. } => format!("http_{status}"),
        _ => "network".to_string(),
    }
}

/// Cancel with no in-flight file (between files) via the writer lock:
/// `request_cancel`, delete the reported staging, `finish_cancel_if_idle`,
/// save + notify. Returns the fresh job.
fn cancel_between_files_via_update(
    store: &JobStore,
    jobs: &Mutex<()>,
    job_id: &str,
    clock: &(dyn Fn() -> u64 + Send + Sync),
    on_update: &(dyn Fn(&Job) + Send + Sync),
) -> Result<Job, String> {
    let (_, fresh) = update(store, jobs, job_id, |job| {
        apply_cancel_locked(store, job, clock());
        Ok(())
    })?;
    on_update(&fresh);
    Ok(fresh)
}

/// Cancel with file `index` in flight (`Downloading`/`Verifying`) via the
/// writer lock: like above, plus the interrupted file goes to `Cancelled`
/// with `cancelled` and both its staging and partial deleted. Returns the
/// fresh job.
fn cancel_with_interrupted_via_update(
    store: &JobStore,
    jobs: &Mutex<()>,
    job_id: &str,
    index: usize,
    clock: &(dyn Fn() -> u64 + Send + Sync),
    on_update: &(dyn Fn(&Job) + Send + Sync),
) -> Result<Job, String> {
    let (_, fresh) = update(store, jobs, job_id, |job| {
        let job_id_owned = job.id.clone();
        let effects = job.request_cancel(clock());
        for name in &effects.staging_to_delete {
            delete_dest_and_partial(store, &job_id_owned, name);
        }
        // The worker owns `Downloading`/`Verifying`: `request_cancel` leaves
        // it alone, so the runner moves it itself before finishing.
        // Allowed transitions include `Downloading -> Cancelled` and
        // `Verifying -> Cancelled`.
        if let Some(file) = job.files.get(index) {
            if matches!(file.state, FileState::Downloading | FileState::Verifying) {
                let staging = file.staging_name.clone();
                if let Some(name) = staging {
                    delete_dest_and_partial(store, &job_id_owned, &name);
                    job.files[index].staging_name = None;
                }
                job.files[index].error = Some("cancelled".to_string());
                let now = clock();
                let _ = job.set_file_state(index, FileState::Cancelled, now);
            }
        }
        job.finish_cancel_if_idle(clock());
        Ok(())
    })?;
    on_update(&fresh);
    Ok(fresh)
}

fn is_cancelled(cancel: &tokio::sync::watch::Receiver<bool>) -> bool {
    *cancel.borrow()
}

/// Immutable remote description of the file the runner works on. Only this
/// (plus the index) travels across an `.await`; never a job copy.
type SelectedFile = (
    usize,
    String,
    String,
    Option<u64>,
    Option<String>,
    Option<String>,
);

/// Download every `Queued` file of `job_id` one at a time, verifying each
/// staged PDF and recording `sha256` / `page_count`.
///
/// No job copy is ever kept across an `.await`: every write goes through
/// [`update`] (fresh load → touch only the current file → save) under the
/// single writer lock, so a concurrent task-23 `Verified -> Importing` move
/// of another file survives. Only the file index plus the immutable remote
/// description travel across the download.
///
/// - 429 waits `retry_after` (default 10 s, capped at 60 s), at most 3 waits
///   per file and cancel-aware; the 4th 429 fails with `rate_limited`.
///   Network/5xx retries stay inside the crate's [`RetryPolicy`].
/// - `attempts` increments for every download attempt (including 429
///   retries).
/// - After every state change the job is saved with [`JobStore::save`] and
///   `on_update` runs (the production wrapper emits `ia://job-updated`).
/// - The job stays `Running` while any file is `Verified`/`Importing`; only
///   when every file is terminal does it [`settle`](Job::settle).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_job<D: Downloader + Sync>(
    store: &JobStore,
    jobs: &Mutex<()>,
    job_id: &str,
    downloader: &D,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    limits: (DownloadLimits, PageLimits),
    clock: &(dyn Fn() -> u64 + Send + Sync),
    on_update: &(dyn Fn(&Job) + Send + Sync),
) -> Result<Job, String> {
    let (_, page_limits) = limits;

    loop {
        // Fresh selection: never a job copy across the `.await` below.
        let selected: Option<SelectedFile> = {
            let _guard = jobs
                .lock()
                .map_err(|_| "io: local storage error".to_string())?;
            let job = store.load(job_id).map_err(store_load_error)?;
            if job.state.is_terminal() {
                return Ok(job);
            }
            job.files
                .iter()
                .enumerate()
                .find(|(_, file)| file.state == FileState::Queued)
                .map(|(index, file)| {
                    (
                        index,
                        file.identifier.clone(),
                        file.file_name.clone(),
                        file.expected_size,
                        file.md5.clone(),
                        file.sha1.clone(),
                    )
                })
        };
        let Some((index, identifier, remote_name, expected_size, md5, sha1)) = selected else {
            break;
        };
        if is_cancelled(&cancel) {
            cancel_between_files_via_update(store, jobs, job_id, clock, on_update)?;
            break;
        }
        // Fresh staging name plus `Downloading`, touching only this file.
        let staging = internet_archive::store::new_staging_name();
        let dest = store
            .staging_path(job_id, &staging)
            .map_err(|_| "io: local storage error".to_string())?;
        match update(store, jobs, job_id, |job| {
            match job.files.get(index) {
                Some(file) if file.state == FileState::Queued => {}
                _ => return Err("invalid_state: file no longer queued".to_string()),
            }
            job.files[index].staging_name = Some(staging.clone());
            job.files[index].error = None;
            job.set_file_state(index, FileState::Downloading, clock())
                .map_err(|e| format!("invalid_state: {e}"))?;
            Ok(())
        }) {
            Ok(((), fresh)) => on_update(&fresh),
            Err(error) if error.starts_with("invalid_state: file no longer") => continue,
            Err(error) => return Err(error),
        }

        // Download with 429 retries. Each pass increments `attempts` through
        // the lock; the download future races the cancel signal.
        enum Outcome {
            Downloaded,
            Cancelled,
            Failed(IaError),
        }
        let mut waits_used: u32 = 0;
        let outcome: Outcome = loop {
            let attempt = update(store, jobs, job_id, |job| {
                match job.files.get(index) {
                    Some(file) if file.state == FileState::Downloading => {}
                    _ => return Err("invalid_state: download interrupted".to_string()),
                }
                job.files[index].attempts = job.files[index].attempts.saturating_add(1);
                Ok(())
            });
            match attempt {
                Ok(((), fresh)) => on_update(&fresh),
                Err(error) if error.starts_with("invalid_state: download interrupted") => {
                    break Outcome::Cancelled;
                }
                Err(error) => return Err(error),
            }
            let download_fut = downloader.download(&identifier, &remote_name, &dest, expected_size);
            tokio::select! {
                biased;
                _ = cancel.changed() => {
                    break Outcome::Cancelled;
                }
                result = download_fut => {
                    match result {
                        Ok(_) => break Outcome::Downloaded,
                        Err(IaError::Http { status: 429, retry_after }) => {
                            if waits_used >= 3 {
                                break Outcome::Failed(IaError::Http {
                                    status: 429,
                                    retry_after,
                                });
                            }
                            waits_used += 1;
                            let wait = retry_after
                                .unwrap_or(Duration::from_secs(10))
                                .min(Duration::from_secs(60));
                            tokio::select! {
                                biased;
                                _ = cancel.changed() => {
                                    break Outcome::Cancelled;
                                }
                                _ = tokio::time::sleep(wait) => {}
                            }
                            if is_cancelled(&cancel) {
                                break Outcome::Cancelled;
                            }
                        }
                        Err(other) => break Outcome::Failed(other),
                    }
                }
            }
        };

        match outcome {
            Outcome::Cancelled => {
                cancel_with_interrupted_via_update(store, jobs, job_id, index, clock, on_update)?;
                break;
            }
            Outcome::Failed(error) => {
                let (next, code) = map_download_error(&error);
                delete_dest_and_partial(store, job_id, &staging);
                let (_, fresh) = update(store, jobs, job_id, |job| {
                    // The file may have been cancelled meanwhile; only move
                    // it when still `Downloading`.
                    match job.files.get(index) {
                        Some(file) if file.state == FileState::Downloading => {}
                        _ => return Ok(()),
                    }
                    job.files[index].staging_name = None;
                    job.files[index].error = Some(code.clone());
                    job.set_file_state(index, next, clock())
                        .map_err(|e| format!("invalid_state: {e}"))?;
                    Ok(())
                })?;
                on_update(&fresh);
                if is_cancelled(&cancel) {
                    cancel_between_files_via_update(store, jobs, job_id, clock, on_update)?;
                    break;
                }
                continue;
            }
            Outcome::Downloaded => {}
        }

        if is_cancelled(&cancel) {
            cancel_with_interrupted_via_update(store, jobs, job_id, index, clock, on_update)?;
            break;
        }
        // Verify the staged bytes: first `Downloading -> Verifying` through
        // the lock, then the synchronous verify, then the result.
        if let Err(error) = update(store, jobs, job_id, |job| {
            match job.files.get(index) {
                Some(file) if file.state == FileState::Downloading => {}
                _ => return Err("invalid_state: download interrupted".to_string()),
            }
            job.set_file_state(index, FileState::Verifying, clock())
                .map_err(|e| format!("invalid_state: {e}"))?;
            Ok(())
        })
        .map(|(_, fresh)| on_update(&fresh))
        {
            if error.starts_with("invalid_state") {
                if is_cancelled(&cancel) {
                    cancel_with_interrupted_via_update(
                        store, jobs, job_id, index, clock, on_update,
                    )?;
                    break;
                }
                continue;
            }
            return Err(error);
        }
        let expected = ExpectedRemote {
            md5,
            sha1,
            content_length: expected_size,
        };
        match internet_archive::verify_staged_pdf(&dest, &expected, &page_limits) {
            Ok(verified) => {
                let sha256_hex = verified.sha256_hex;
                let page_count = verified.page_count;
                let (_, fresh) = update(store, jobs, job_id, |job| {
                    match job.files.get(index) {
                        Some(file) if file.state == FileState::Verifying => {}
                        _ => return Err("invalid_state: verification interrupted".to_string()),
                    }
                    job.files[index].sha256 = Some(sha256_hex.clone());
                    job.files[index].page_count = Some(page_count);
                    job.files[index].error = None;
                    job.set_file_state(index, FileState::Verified, clock())
                        .map_err(|e| format!("invalid_state: {e}"))?;
                    Ok(())
                })?;
                on_update(&fresh);
            }
            Err(error) => {
                let code = map_verify_error(&error);
                // `verify_staged_pdf` deletes the staged file on rejection;
                // the partial cannot exist here, but drop both anyway.
                delete_dest_and_partial(store, job_id, &staging);
                let (_, fresh) = update(store, jobs, job_id, |job| {
                    match job.files.get(index) {
                        Some(file) if file.state == FileState::Verifying => {}
                        _ => return Ok(()),
                    }
                    job.files[index].staging_name = None;
                    job.files[index].sha256 = None;
                    job.files[index].page_count = None;
                    job.files[index].error = Some(code.clone());
                    job.set_file_state(index, FileState::Failed, clock())
                        .map_err(|e| format!("invalid_state: {e}"))?;
                    Ok(())
                })?;
                on_update(&fresh);
            }
        }
        if is_cancelled(&cancel) {
            cancel_between_files_via_update(store, jobs, job_id, clock, on_update)?;
            break;
        }
    }

    // A job with `Verified`/`Importing` files stays `Running` for the
    // frontend import (task 23); only all-terminal jobs settle, and only
    // then is there a state change to persist and report.
    let (settled, final_job) = {
        let _guard = jobs
            .lock()
            .map_err(|_| "io: local storage error".to_string())?;
        let mut job = store.load(job_id).map_err(store_load_error)?;
        let settled = job.settle(clock()).is_some();
        if settled {
            store
                .save(&job)
                .map_err(|_| "io: cannot persist the import job".to_string())?;
        }
        (settled, job)
    };
    if settled {
        on_update(&final_job);
    }
    Ok(final_job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use internet_archive::job::{ImportTarget, JobFile};
    use internet_archive::{Availability, FileSource, PdfFile};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn available_pdf(name: &str) -> PdfFile {
        PdfFile {
            name: name.to_string(),
            format: Some("Text PDF".to_string()),
            source: FileSource::Original,
            size: None,
            mtime: None,
            md5: None,
            sha1: None,
            availability: Availability::Available,
        }
    }

    pub(crate) fn tiny_pdf_bytes(pages: u32) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut document = Document::with_version("1.5");
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let pages_id = document.new_object_id();
        let page_ids = (0..pages)
            .map(|index| {
                let content = document.add_object(Stream::new(
                    dictionary! {},
                    format!("BT /F1 12 Tf 20 100 Td (Page {index}) Tj ET").into_bytes(),
                ));
                document.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                    "Resources" => resources_id,
                    "Contents" => content,
                })
            })
            .collect::<Vec<_>>();
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Count" => i64::from(pages),
                "Kids" => page_ids.iter().map(|id| (*id).into()).collect::<Vec<Object>>(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("serialize test PDF");
        bytes
    }

    fn test_store() -> (tempfile::TempDir, JobStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobStore::new(dir.path().join("internet-archive"));
        (dir, store)
    }

    fn make_job(files: Vec<(&str, &str)>) -> Job {
        let job_files = files
            .into_iter()
            .map(|(identifier, name)| {
                JobFile::from_pdf(identifier, &available_pdf(name)).expect("available")
            })
            .collect::<Vec<_>>();
        Job::new(
            uuid::Uuid::new_v4().hyphenated().to_string(),
            1_700_000_000,
            ImportTarget::ExistingCollection {
                collection_id: "col-1".to_string(),
            },
            job_files,
        )
        .expect("job")
    }

    fn running_job(job: Job) -> Job {
        let mut job = job;
        job.set_job_state(JobState::Ready, 1_700_000_001)
            .expect("to ready");
        job.set_job_state(JobState::Running, 1_700_000_002)
            .expect("to running");
        job
    }

    fn default_limits() -> (DownloadLimits, PageLimits) {
        (DownloadLimits::clamped(None), PageLimits::clamped(None))
    }

    struct SuccessDownloader {
        pages: u32,
    }

    impl Downloader for SuccessDownloader {
        async fn download(
            &self,
            _identifier: &str,
            _remote_name: &str,
            dest: &Path,
            _expected_size: Option<u64>,
        ) -> Result<u64, IaError> {
            let bytes = tiny_pdf_bytes(self.pages);
            std::fs::write(dest, &bytes).map_err(|e| IaError::Io(e.to_string()))?;
            Ok(bytes.len() as u64)
        }
    }

    struct StatusDownloader {
        status: u16,
    }

    impl Downloader for StatusDownloader {
        async fn download(
            &self,
            _identifier: &str,
            _remote_name: &str,
            _dest: &Path,
            _expected_size: Option<u64>,
        ) -> Result<u64, IaError> {
            Err(IaError::Http {
                status: self.status,
                retry_after: None,
            })
        }
    }

    struct TooLargeDownloader;

    impl Downloader for TooLargeDownloader {
        async fn download(
            &self,
            _identifier: &str,
            _remote_name: &str,
            _dest: &Path,
            _expected_size: Option<u64>,
        ) -> Result<u64, IaError> {
            Err(IaError::FileTooLarge { limit: 10 })
        }
    }

    struct NonPdfDownloader;

    impl Downloader for NonPdfDownloader {
        async fn download(
            &self,
            _identifier: &str,
            _remote_name: &str,
            dest: &Path,
            _expected_size: Option<u64>,
        ) -> Result<u64, IaError> {
            std::fs::write(dest, b"not a pdf").map_err(|e| IaError::Io(e.to_string()))?;
            Ok(9)
        }
    }

    struct Flaky429Downloader {
        failures_left: Mutex<u32>,
    }

    impl Downloader for Flaky429Downloader {
        async fn download(
            &self,
            _identifier: &str,
            _remote_name: &str,
            dest: &Path,
            _expected_size: Option<u64>,
        ) -> Result<u64, IaError> {
            let mut left = self.failures_left.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                return Err(IaError::Http {
                    status: 429,
                    retry_after: Some(Duration::ZERO),
                });
            }
            drop(left);
            let bytes = tiny_pdf_bytes(1);
            std::fs::write(dest, &bytes).map_err(|e| IaError::Io(e.to_string()))?;
            Ok(bytes.len() as u64)
        }
    }

    struct PendingDownloader;

    impl Downloader for PendingDownloader {
        async fn download(
            &self,
            _identifier: &str,
            _remote_name: &str,
            dest: &Path,
            _expected_size: Option<u64>,
        ) -> Result<u64, IaError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let bytes = tiny_pdf_bytes(1);
            std::fs::write(dest, &bytes).map_err(|e| IaError::Io(e.to_string()))?;
            Ok(bytes.len() as u64)
        }
    }

    #[tokio::test]
    async fn two_files_verify_and_job_stays_running() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf"), ("id-b", "b.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let downloader = SuccessDownloader { pages: 1 };
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .expect("run succeeds");
        assert_eq!(out.files.len(), 2);
        for file in &out.files {
            assert_eq!(file.state, FileState::Verified);
            assert!(file.sha256.is_some());
            assert_eq!(file.page_count, Some(1));
            assert!(file.staging_name.is_some());
            let path = store
                .staging_path(&job_id, file.staging_name.as_deref().unwrap())
                .unwrap();
            assert!(path.exists(), "staging present");
            assert!(!internet_archive::partial_path(&path).exists());
        }
        assert_eq!(out.state, JobState::Running);
        assert!(calls.load(Ordering::SeqCst) > 0, "on_update called");
        // Persisted too.
        let saved = store.load(&job_id).unwrap();
        assert_eq!(saved, out);
    }

    #[tokio::test]
    async fn forbidden_maps_to_unavailable_and_deletes_staging() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let downloader = StatusDownloader { status: 403 };
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run succeeds");
        assert_eq!(out.files[0].state, FileState::Unavailable);
        assert_eq!(out.files[0].error.as_deref(), Some("unavailable"));
        assert!(!out.files[0].error.as_deref().unwrap().contains('/'));
        // Staging gone: no name left and nothing on disk.
        assert_eq!(out.files[0].staging_name, None);
        let entries: Vec<_> = std::fs::read_dir(store.root().join(&job_id))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            entries.iter().all(|n| n == "job.json"),
            "only job.json left, saw {entries:?}"
        );
    }

    #[tokio::test]
    async fn too_large_maps_to_failed() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let downloader = TooLargeDownloader;
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run succeeds");
        assert_eq!(out.files[0].state, FileState::Failed);
        assert_eq!(out.files[0].error.as_deref(), Some("file_too_large"));
    }

    #[tokio::test]
    async fn non_pdf_maps_to_verify_failed() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let downloader = NonPdfDownloader;
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run succeeds");
        assert_eq!(out.files[0].state, FileState::Failed);
        assert_eq!(out.files[0].error.as_deref(), Some("verify_failed"));
        assert_eq!(out.files[0].staging_name, None);
    }

    #[tokio::test]
    async fn all_failed_settles_to_failed() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf"), ("id-b", "b.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let downloader = NonPdfDownloader;
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run succeeds");
        assert!(out.files.iter().all(|f| f.state == FileState::Failed));
        assert_eq!(out.state, JobState::Failed);
    }

    #[tokio::test]
    async fn two_429_then_success_gives_three_attempts() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let downloader = Flaky429Downloader {
            failures_left: Mutex::new(2),
        };
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run succeeds");
        assert_eq!(out.files[0].state, FileState::Verified);
        assert_eq!(out.files[0].attempts, 3);
    }

    #[tokio::test]
    async fn four_429s_fail_rate_limited() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let downloader = Flaky429Downloader {
            failures_left: Mutex::new(4),
        };
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run succeeds");
        assert_eq!(out.files[0].state, FileState::Failed);
        assert_eq!(out.files[0].error.as_deref(), Some("rate_limited"));
        assert_eq!(out.state, JobState::Failed);
    }

    #[tokio::test]
    async fn cancel_pending_download_marks_cancelled() {
        let (_dir, store) = test_store();
        let job = running_job(make_job(vec![("id-a", "a.pdf"), ("id-b", "b.pdf")]));
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        // Send cancel while the fake sleeps: the sender lives in another task,
        // the runner runs here so no borrowed `Send + 'static` juggling.
        let (tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx.send(true);
        });
        let out = run_job(
            &store,
            &Mutex::new(()),
            &job_id,
            &PendingDownloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run ok");
        assert_eq!(out.files[0].state, FileState::Cancelled);
        assert_eq!(out.files[0].error.as_deref(), Some("cancelled"));
        assert_eq!(out.files[1].state, FileState::Cancelled);
        assert_eq!(out.state, JobState::Cancelled);
        // No staging or partial left for either file.
        let dir = store.root().join(&out.id);
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            entries.iter().all(|n| n == "job.json"),
            "only job.json left, saw {entries:?}"
        );
    }

    #[test]
    fn start_states_accept_draft_ready_running() {
        for state in [JobState::Draft, JobState::Ready, JobState::Running] {
            assert!(decide_start(state).is_ok(), "{state:?} accepted");
        }
        for state in [
            JobState::Cancelling,
            JobState::Completed,
            JobState::Partial,
            JobState::Failed,
            JobState::Cancelled,
        ] {
            let err = decide_start(state).unwrap_err();
            assert!(err.starts_with("invalid_state"), "got {err:?}");
        }
    }

    struct DelayedSuccessDownloader {
        delay: Duration,
        pages: u32,
    }

    impl Downloader for DelayedSuccessDownloader {
        async fn download(
            &self,
            _identifier: &str,
            _remote_name: &str,
            dest: &Path,
            _expected_size: Option<u64>,
        ) -> Result<u64, IaError> {
            tokio::time::sleep(self.delay).await;
            let bytes = tiny_pdf_bytes(self.pages);
            std::fs::write(dest, &bytes).map_err(|e| IaError::Io(e.to_string()))?;
            Ok(bytes.len() as u64)
        }
    }

    #[tokio::test]
    async fn concurrent_importing_change_survives_runner() {
        // File 0 already `Verified` (task 23 moves it to `Importing` while
        // the runner downloads file 1). The runner must not overwrite that
        // change with its stale in-memory copy.
        let (_dir, store) = test_store();
        let mut job = make_job(vec![("id-a", "a.pdf"), ("id-b", "b.pdf")]);
        job.files[0].state = FileState::Verified;
        job.files[0].staging_name = Some(internet_archive::store::new_staging_name());
        job.files[0].sha256 = Some("abc123".to_string());
        job.files[0].page_count = Some(1);
        let staging0 = job.files[0].staging_name.clone().unwrap();
        let job = running_job(job);
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let dest0 = store.staging_path(&job_id, &staging0).unwrap();
        std::fs::create_dir_all(dest0.parent().unwrap()).unwrap();
        std::fs::write(&dest0, tiny_pdf_bytes(1)).unwrap();
        let jobs = std::sync::Arc::new(Mutex::new(()));
        let (_tx, rx) = tokio::sync::watch::channel(false);
        // Concurrent writer (task 23): same lock, same helper.
        let store_clone = store.clone();
        let jobs_clone = std::sync::Arc::clone(&jobs);
        let job_id_clone = job_id.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let (_, fresh) = update(&store_clone, &jobs_clone, &job_id_clone, |job| {
                job.set_file_state(0, FileState::Importing, 1_700_000_020)
                    .map_err(|e| format!("invalid_state: {e}"))?;
                Ok(())
            })
            .expect("concurrent move to Importing");
            assert_eq!(fresh.files[0].state, FileState::Importing);
        });
        let downloader = DelayedSuccessDownloader {
            delay: Duration::from_millis(200),
            pages: 1,
        };
        let out = run_job(
            &store,
            &jobs,
            &job_id,
            &downloader,
            rx,
            default_limits(),
            &|| 1_700_000_010,
            &|_| {},
        )
        .await
        .expect("run succeeds");
        writer.await.expect("writer done");
        let saved = store.load(&job_id).unwrap();
        assert_eq!(
            saved.files[0].state,
            FileState::Importing,
            "concurrent Verified->Importing must survive (runner returned {out:?})"
        );
        assert_eq!(saved.files[1].state, FileState::Verified);
    }

    #[test]
    fn end_of_run_cancel_marks_verified_cancelled_and_removes_staging() {
        let (_dir, store) = test_store();
        let mut job = make_job(vec![("id-a", "a.pdf")]);
        job.files[0].state = FileState::Verified;
        job.files[0].staging_name = Some(internet_archive::store::new_staging_name());
        job.files[0].sha256 = Some("abc123".to_string());
        job.files[0].page_count = Some(1);
        let staging = job.files[0].staging_name.clone().unwrap();
        let job = running_job(job);
        let job_id = job.id.clone();
        store.create(&job).unwrap();
        let dest = store.staging_path(&job_id, &staging).unwrap();
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"staged").unwrap();
        let jobs = Mutex::new(());
        let (applied, out) = apply_end_of_run_cancel(&store, &jobs, &job_id, true, 1_700_000_030)
            .expect("cancel applies");
        assert!(applied);
        assert_eq!(out.files[0].state, FileState::Cancelled);
        assert!(!dest.exists(), "staging removed");
        assert!(!internet_archive::partial_path(&dest).exists());
        assert_eq!(out.state, JobState::Cancelled);
    }
}
