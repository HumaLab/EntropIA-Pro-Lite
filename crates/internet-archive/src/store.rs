//! Local `job.json` store (Phase 2 task 14).
//!
//! Each job lives in `<root>/<job_id>/job.json` with atomic writes
//! (temp file + rename). Local only, no migration, no sync.
//!
//! Identifiers are lowercase UUID v4 (`new_job_id`) and staging files are
//! opaque `<uuid>.pdf` names (`new_staging_name`). Every method taking a job
//! id or staging name validates its exact shape first, so traversal (`..`,
//! separators, absolute paths) is impossible by construction: no validated
//! path is ever built from anything else.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::job::{Job, JOB_SCHEMA_VERSION};

/// Typed store errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid id: {0}")]
    InvalidId(String),
    #[error("unsupported schema version {found} (max is {JOB_SCHEMA_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("not found: {0}")]
    NotFound(String),
}

/// One `Importing` file found while recovering a `Cancelling` job, with the
/// owning job attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReconcileEntry {
    pub job_id: String,
    pub index: usize,
    pub identifier: String,
    pub file_name: String,
    pub staging_name: Option<String>,
}

/// Outcome of [`JobStore::recover_all`].
#[derive(Debug, Clone, Default)]
pub struct RecoveryReport {
    pub jobs: Vec<Job>,
    pub needs_reconcile: Vec<RecoveryReconcileEntry>,
    pub skipped: Vec<String>,
    pub quarantined: Vec<String>,
    pub unsupported: Vec<String>,
}

/// Lowercase hyphenated UUID v4 for a new job id.
#[must_use]
pub fn new_job_id() -> String {
    uuid::Uuid::new_v4().hyphenated().to_string()
}

/// Opaque staging file name (`<uuid>.pdf`).
#[must_use]
pub fn new_staging_name() -> String {
    format!("{}.pdf", uuid::Uuid::new_v4().hyphenated())
}

/// Accept only `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`.
#[must_use]
pub fn is_valid_job_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    if bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-' {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            continue;
        }
        if !matches!(byte, b'0'..=b'9' | b'a'..=b'f') {
            return false;
        }
    }
    true
}

/// Accept only a valid job id plus `.pdf`.
#[must_use]
pub fn is_valid_staging_name(name: &str) -> bool {
    if name.len() != 40 || !name.ends_with(".pdf") {
        return false;
    }
    is_valid_job_id(&name[..36])
}

fn check_job_id(id: &str) -> Result<(), StoreError> {
    if is_valid_job_id(id) {
        Ok(())
    } else {
        Err(StoreError::InvalidId(id.to_string()))
    }
}

fn check_staging_name(name: &str) -> Result<(), StoreError> {
    if is_valid_staging_name(name) {
        Ok(())
    } else {
        Err(StoreError::InvalidId(name.to_string()))
    }
}

/// Write `job.json` atomically: `job.json.tmp` + flush + `sync_all` +
/// rename, then a best-effort folder sync on Unix. The caller must have
/// validated the job id and ensured `dir` is a real directory.
fn write_job_atomic(dir: &Path, job: &Job) -> Result<(), StoreError> {
    let json = serde_json::to_string_pretty(job)?;
    let tmp = dir.join("job.json.tmp");
    let dst = dir.join("job.json");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(json.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, &dst)?;
    #[cfg(unix)]
    {
        if let Ok(dir_file) = fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
    }
    Ok(())
}

/// A single path component that cannot escape its parent directory.
fn is_confined_file_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name)
}

/// Filesystem store rooted at `<data_dir>/staging/internet-archive`.
#[derive(Debug, Clone)]
pub struct JobStore {
    root: PathBuf,
}

impl JobStore {
    /// The caller passes `<data_dir>/staging/internet-archive`.
    /// `root` is created lazily by [`JobStore::create`]/[`JobStore::save`]
    /// (and tolerated as missing by [`JobStore::recover_all`]).
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Store root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Persist a new job; fails when the job folder already exists.
    pub fn create(&self, job: &Job) -> Result<(), StoreError> {
        check_job_id(&job.id)?;
        let dir = self.root.join(&job.id);
        match fs::symlink_metadata(&dir) {
            Ok(_) => return Err(StoreError::AlreadyExists(job.id.clone())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(StoreError::Io(e)),
        }
        fs::create_dir_all(&dir)?;
        write_job_atomic(&dir, job)
    }

    /// Overwrite a job atomically (no `job.json.tmp` left behind).
    pub fn save(&self, job: &Job) -> Result<(), StoreError> {
        check_job_id(&job.id)?;
        let dir = self.root.join(&job.id);
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(StoreError::Io(std::io::Error::other(format!(
                    "refusing to write through symlink: {}",
                    job.id
                ))));
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(StoreError::Io(std::io::Error::other(format!(
                    "job path is not a directory: {}",
                    job.id
                ))));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&dir)?;
            }
            Err(e) => return Err(StoreError::Io(e)),
        }
        write_job_atomic(&dir, job)
    }

    /// Load a job; rejects a newer `schema_version` without rewriting it.
    pub fn load(&self, job_id: &str) -> Result<Job, StoreError> {
        check_job_id(job_id)?;
        let path = self.root.join(job_id).join("job.json");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::NotFound(job_id.to_string()));
            }
            Err(e) => return Err(StoreError::Io(e)),
        };
        let job: Job = serde_json::from_slice(&bytes)?;
        if job.schema_version > JOB_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedVersion {
                found: job.schema_version,
            });
        }
        Ok(job)
    }

    /// Validated `<root>/<job_id>/<staging_name>` path, strictly inside root.
    pub fn staging_path(&self, job_id: &str, staging_name: &str) -> Result<PathBuf, StoreError> {
        check_job_id(job_id)?;
        check_staging_name(staging_name)?;
        Ok(self.root.join(job_id).join(staging_name))
    }

    /// Delete one staging file; missing files are `Ok`.
    pub fn delete_staging(&self, job_id: &str, staging_name: &str) -> Result<(), StoreError> {
        let path = self.staging_path(job_id, staging_name)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::Io(e)),
        }
    }

    /// Remove a whole job folder; refuses symlinked folders.
    pub fn remove_job(&self, job_id: &str) -> Result<(), StoreError> {
        check_job_id(job_id)?;
        let dir = self.root.join(job_id);
        let meta = match fs::symlink_metadata(&dir) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::NotFound(job_id.to_string()));
            }
            Err(e) => return Err(StoreError::Io(e)),
        };
        if meta.file_type().is_symlink() {
            return Err(StoreError::Io(std::io::Error::other(format!(
                "refusing to remove symlinked job folder: {job_id}"
            ))));
        }
        if !meta.is_dir() {
            return Err(StoreError::Io(std::io::Error::other(format!(
                "job path is not a directory: {job_id}"
            ))));
        }
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Restart recovery over every entry in `root`, in sorted order.
    ///
    /// For each real directory with a valid job id name (symlinks are
    /// skipped, never followed): remove a leftover `job.json.tmp`;
    /// quarantine an unparsable `job.json` as `job.json.corrupt-<now>`
    /// without touching PDFs; leave unsupported versions untouched;
    /// otherwise run `normalize_after_restart(now)`, save the job when
    /// anything changed, delete every `staging_to_delete` file, and delete
    /// orphan staging files (valid `<uuid>.pdf` names referenced by no
    /// `JobFile.staging_name`). Staging listed in `needs_reconcile` is
    /// never deleted. Never panics; per-file deletions are best-effort.
    #[must_use]
    pub fn recover_all(&self, now: u64) -> RecoveryReport {
        let mut report = RecoveryReport::default();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(_) => return report,
        };
        let mut names: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            names.push((name, entry.path()));
        }
        names.sort();
        for (name, path) in names {
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => {
                    report.skipped.push(name);
                    continue;
                }
            };
            if meta.file_type().is_symlink() || !meta.is_dir() || !is_valid_job_id(&name) {
                report.skipped.push(name);
                continue;
            }
            // Leftover from an interrupted atomic write; safe to drop.
            match fs::symlink_metadata(path.join("job.json.tmp")) {
                Ok(tmp_meta) if tmp_meta.is_dir() => {}
                Ok(_) => {
                    let _ = fs::remove_file(path.join("job.json.tmp"));
                }
                Err(_) => {}
            }
            let job_path = path.join("job.json");
            let bytes = match fs::read(&job_path) {
                Ok(bytes) => bytes,
                Err(_) => {
                    // Valid folder but no manifest: leave it alone.
                    report.skipped.push(name);
                    continue;
                }
            };
            let mut job: Job = match serde_json::from_slice(&bytes) {
                Ok(job) => job,
                Err(_) => {
                    let dst = path.join(format!("job.json.corrupt-{now}"));
                    let _ = fs::rename(&job_path, &dst);
                    #[cfg(unix)]
                    {
                        if let Ok(dir_file) = fs::File::open(&path) {
                            let _ = dir_file.sync_all();
                        }
                    }
                    report.quarantined.push(name);
                    continue;
                }
            };
            if job.schema_version > JOB_SCHEMA_VERSION {
                report.unsupported.push(name);
                continue;
            }
            let before = job.clone();
            let effects = job.normalize_after_restart(now);
            let reconcile_names: HashSet<&str> = effects
                .needs_reconcile
                .iter()
                .filter_map(|entry| entry.staging_name.as_deref())
                .collect();
            for entry in &effects.needs_reconcile {
                report.needs_reconcile.push(RecoveryReconcileEntry {
                    job_id: name.clone(),
                    index: entry.index,
                    identifier: entry.identifier.clone(),
                    file_name: entry.file_name.clone(),
                    staging_name: entry.staging_name.clone(),
                });
            }
            for staging in &effects.staging_to_delete {
                if is_confined_file_name(staging) {
                    let _ = fs::remove_file(path.join(staging));
                }
            }
            let referenced: HashSet<&str> = job
                .files
                .iter()
                .filter_map(|file| file.staging_name.as_deref())
                .collect();
            if let Ok(folder) = fs::read_dir(&path) {
                for entry in folder.flatten() {
                    let file_name = entry.file_name().to_string_lossy().into_owned();
                    if !is_valid_staging_name(&file_name) {
                        continue;
                    }
                    if referenced.contains(file_name.as_str()) {
                        continue;
                    }
                    if reconcile_names.contains(file_name.as_str()) {
                        continue;
                    }
                    let _ = fs::remove_file(path.join(&file_name));
                }
            }
            if job != before && write_job_atomic(&path, &job).is_err() {
                report.skipped.push(name);
                continue;
            }
            report.jobs.push(job);
        }
        report.jobs.sort_by(|a, b| a.id.cmp(&b.id));
        report
            .needs_reconcile
            .sort_by(|a, b| a.job_id.cmp(&b.job_id).then(a.index.cmp(&b.index)));
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Availability, FileSource, PdfFile};
    use crate::job::{FileState, ImportTarget, JobFile, JobState};
    use std::fs;

    const VALID_ID: &str = "12345678-1234-1234-1234-1234567890ab";
    const VALID_ID_2: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    const VALID_STAGING: &str = "12345678-1234-1234-1234-1234567890ab.pdf";

    fn available_pdf(name: &str) -> PdfFile {
        PdfFile {
            name: name.to_string(),
            format: Some("Text PDF".to_string()),
            source: FileSource::Original,
            size: Some(100),
            mtime: Some(200),
            md5: Some("md5".to_string()),
            sha1: Some("sha1".to_string()),
            availability: Availability::Available,
        }
    }

    fn sample_job(id: &str) -> Job {
        let file = JobFile::from_pdf("ident", &available_pdf("doc.pdf")).unwrap();
        Job::new(
            id,
            1,
            ImportTarget::NewCollection {
                name: "Nueva".to_string(),
            },
            vec![file],
        )
        .unwrap()
    }

    fn sample_job_with_state(id: &str, job_state: JobState, file_state: FileState) -> Job {
        let mut file = JobFile::from_pdf("ident", &available_pdf("doc.pdf")).unwrap();
        file.state = file_state;
        Job {
            schema_version: JOB_SCHEMA_VERSION,
            id: id.to_string(),
            created_at: 1,
            updated_at: 1,
            target: ImportTarget::NewCollection {
                name: "Nueva".to_string(),
            },
            state: job_state,
            files: vec![file],
            items: Vec::new(),
        }
    }

    fn write_job_file(dir: &Path, job: &Job) {
        fs::create_dir_all(dir).unwrap();
        let json = serde_json::to_string_pretty(job).unwrap();
        fs::write(dir.join("job.json"), json).unwrap();
    }

    #[test]
    fn create_load_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("internet-archive");
        let store = JobStore::new(root);
        let job = sample_job(VALID_ID);
        store.create(&job).unwrap();
        let back = store.load(VALID_ID).unwrap();
        assert_eq!(back, job);
    }

    #[test]
    fn create_twice_is_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let job = sample_job(VALID_ID);
        store.create(&job).unwrap();
        let err = store.create(&job).unwrap_err();
        assert!(matches!(err, StoreError::AlreadyExists(_)), "got {err:?}");
    }

    #[test]
    fn save_overwrites_atomically_without_tmp_left() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let job = sample_job(VALID_ID);
        store.create(&job).unwrap();
        let mut updated = job.clone();
        updated.updated_at = 99;
        store.save(&updated).unwrap();
        let back = store.load(VALID_ID).unwrap();
        assert_eq!(back.updated_at, 99);
        assert!(
            !store.root().join(VALID_ID).join("job.json.tmp").exists(),
            "no tmp left behind"
        );
    }

    #[test]
    fn validators_accept_valid_shapes() {
        assert!(is_valid_job_id(VALID_ID));
        assert!(is_valid_job_id(&new_job_id()));
        assert!(is_valid_staging_name(VALID_STAGING));
        assert!(is_valid_staging_name(&new_staging_name()));
    }

    #[test]
    fn validators_reject_bad_shapes() {
        for bad in [
            "..",
            "a/b",
            "C:\\x",
            "",
            "12345678-1234-1234-1234-1234567890AB",
            "12345678_1234_1234_1234_1234567890ab",
            "not-a-uuid",
            "x.pdf",
            "12345678-1234-1234-1234-1234567890ab.PDF",
            "../12345678-1234-1234-1234-1234567890ab",
        ] {
            assert!(!is_valid_job_id(bad), "job id accepted: {bad:?}");
            assert!(!is_valid_staging_name(bad), "staging accepted: {bad:?}");
        }
        assert!(!is_valid_staging_name(VALID_ID));
    }

    #[test]
    fn invalid_ids_rejected_on_every_method_without_fs_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().join("root"));
        let bad_ids = ["..", "a/b", "C:\\x", "", "NOT-A-UUID", "x.pdf"];
        for bad in bad_ids {
            assert!(
                matches!(store.load(bad), Err(StoreError::InvalidId(_))),
                "load({bad:?})"
            );
            assert!(
                matches!(
                    store.staging_path(bad, VALID_STAGING),
                    Err(StoreError::InvalidId(_))
                ),
                "staging_path job {bad:?}"
            );
            assert!(
                matches!(
                    store.staging_path(VALID_ID, bad),
                    Err(StoreError::InvalidId(_))
                ),
                "staging_path staging {bad:?}"
            );
            assert!(
                matches!(
                    store.delete_staging(bad, VALID_STAGING),
                    Err(StoreError::InvalidId(_))
                ),
                "delete_staging job {bad:?}"
            );
            assert!(
                matches!(
                    store.delete_staging(VALID_ID, bad),
                    Err(StoreError::InvalidId(_))
                ),
                "delete_staging staging {bad:?}"
            );
            assert!(
                matches!(store.remove_job(bad), Err(StoreError::InvalidId(_))),
                "remove_job({bad:?})"
            );
            let mut job = sample_job(VALID_ID);
            job.id = bad.to_string();
            assert!(
                matches!(store.create(&job), Err(StoreError::InvalidId(_))),
                "create({bad:?})"
            );
            assert!(
                matches!(store.save(&job), Err(StoreError::InvalidId(_))),
                "save({bad:?})"
            );
        }
        // No file system effect: root either missing or empty.
        let root = tmp.path().join("root");
        if root.exists() {
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        }
    }

    #[test]
    fn staging_path_stays_inside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let store = JobStore::new(root.clone());
        let path = store.staging_path(VALID_ID, VALID_STAGING).unwrap();
        assert!(path.starts_with(&root), "escaped root: {path:?}");
        assert_eq!(path, root.join(VALID_ID).join(VALID_STAGING));
    }

    #[test]
    fn recover_removes_leftover_tmp() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let job = sample_job(VALID_ID);
        store.create(&job).unwrap();
        fs::write(store.root().join(VALID_ID).join("job.json.tmp"), b"stale").unwrap();
        let report = store.recover_all(100);
        assert!(!store.root().join(VALID_ID).join("job.json.tmp").exists());
        assert_eq!(report.jobs.len(), 1);
    }

    #[test]
    fn recover_quarantines_corrupt_json_and_keeps_pdfs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let dir = store.root().join(VALID_ID);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("job.json"), b"{ not json").unwrap();
        fs::write(dir.join("job.json.tmp"), b"stale").unwrap();
        fs::write(dir.join(VALID_STAGING), b"%PDF-1.4 fake").unwrap();
        let report = store.recover_all(4242);
        assert_eq!(report.quarantined, vec![VALID_ID.to_string()]);
        assert!(report.jobs.is_empty());
        assert!(!dir.join("job.json").exists());
        assert!(dir.join("job.json.corrupt-4242").exists());
        assert!(dir.join(VALID_STAGING).exists(), "PDFs must be kept");
        assert!(!dir.join("job.json.tmp").exists());
    }

    #[test]
    fn recover_leaves_unsupported_version_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let mut job = sample_job(VALID_ID);
        job.schema_version = JOB_SCHEMA_VERSION + 1;
        let dir = store.root().join(VALID_ID);
        write_job_file(&dir, &job);
        let before = fs::read(dir.join("job.json")).unwrap();
        let report = store.recover_all(7);
        assert_eq!(report.unsupported, vec![VALID_ID.to_string()]);
        assert!(report.jobs.is_empty());
        let after = fs::read(dir.join("job.json")).unwrap();
        assert_eq!(before, after, "unsupported file must be untouched");
        assert!(store.load(VALID_ID).is_err());
    }

    #[test]
    fn recover_running_downloading_back_to_ready_queued() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let mut job = sample_job_with_state(VALID_ID, JobState::Running, FileState::Downloading);
        let staging = new_staging_name();
        assert!(is_valid_staging_name(&staging));
        job.files[0].staging_name = Some(staging.clone());
        let dir = store.root().join(VALID_ID);
        write_job_file(&dir, &job);
        fs::write(dir.join(&staging), b"partial-bytes").unwrap();
        let report = store.recover_all(999);
        assert_eq!(report.jobs.len(), 1);
        let recovered = &report.jobs[0];
        assert_eq!(recovered.state, JobState::Ready);
        assert_eq!(recovered.files[0].state, FileState::Queued);
        assert_eq!(recovered.files[0].staging_name, None);
        assert!(!dir.join(&staging).exists(), "staging must be deleted");
        let saved = store.load(VALID_ID).unwrap();
        assert_eq!(saved, *recovered);
        assert!(report.needs_reconcile.is_empty());
    }

    #[test]
    fn recover_cancelling_importing_reports_reconcile_and_keeps_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let mut job = sample_job_with_state(VALID_ID, JobState::Cancelling, FileState::Importing);
        let staging = new_staging_name();
        job.files[0].staging_name = Some(staging.clone());
        let dir = store.root().join(VALID_ID);
        write_job_file(&dir, &job);
        fs::write(dir.join(&staging), b"imported-partial").unwrap();
        let report = store.recover_all(1000);
        assert_eq!(report.jobs.len(), 1);
        assert_eq!(report.jobs[0].state, JobState::Cancelled);
        assert_eq!(report.needs_reconcile.len(), 1);
        let entry = &report.needs_reconcile[0];
        assert_eq!(entry.job_id, VALID_ID);
        assert_eq!(entry.index, 0);
        assert_eq!(entry.staging_name.as_deref(), Some(staging.as_str()));
        assert!(dir.join(&staging).exists(), "reconcile staging kept");
        let saved = store.load(VALID_ID).unwrap();
        assert_eq!(saved.files[0].state, FileState::Cancelled);
    }

    #[test]
    fn recover_deletes_orphan_pdf_but_keeps_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let mut job = sample_job_with_state(VALID_ID, JobState::Ready, FileState::Verified);
        let kept = new_staging_name();
        job.files[0].staging_name = Some(kept.clone());
        let dir = store.root().join(VALID_ID);
        write_job_file(&dir, &job);
        let orphan = new_staging_name();
        assert_ne!(orphan, kept);
        fs::write(dir.join(&kept), b"referenced").unwrap();
        fs::write(dir.join(&orphan), b"orphan").unwrap();
        fs::write(dir.join("notes.txt"), b"leave me").unwrap();
        let report = store.recover_all(55);
        assert_eq!(report.jobs.len(), 1);
        assert!(dir.join(&kept).exists(), "referenced staging kept");
        assert!(!dir.join(&orphan).exists(), "orphan staging deleted");
        assert!(dir.join("notes.txt").exists(), "other names left alone");
    }

    #[cfg(unix)]
    #[test]
    fn recover_skips_symlinked_job_folder_and_leaves_target_untouched() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().join("root"));
        fs::create_dir_all(store.root()).unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let job = sample_job(VALID_ID);
        write_job_file(&target, &job);
        let before = fs::read(target.join("job.json")).unwrap();
        symlink(&target, store.root().join(VALID_ID)).unwrap();
        let report = store.recover_all(5);
        assert!(report.jobs.is_empty());
        assert_eq!(report.skipped, vec![VALID_ID.to_string()]);
        let after = fs::read(target.join("job.json")).unwrap();
        assert_eq!(before, after, "target untouched");
    }

    #[test]
    fn remove_job_removes_folder_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().to_path_buf());
        let job = sample_job(VALID_ID);
        store.create(&job).unwrap();
        fs::write(store.staging_path(VALID_ID, VALID_STAGING).unwrap(), b"x").unwrap();
        store.remove_job(VALID_ID).unwrap();
        assert!(!store.root().join(VALID_ID).exists());
        assert!(matches!(store.load(VALID_ID), Err(StoreError::NotFound(_))));
    }

    #[cfg(unix)]
    #[test]
    fn remove_job_refuses_symlink() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let store = JobStore::new(tmp.path().join("root"));
        fs::create_dir_all(store.root()).unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let job = sample_job(VALID_ID_2);
        write_job_file(&target, &job);
        symlink(&target, store.root().join(VALID_ID_2)).unwrap();
        let err = store.remove_job(VALID_ID_2).unwrap_err();
        assert!(
            !matches!(err, StoreError::InvalidId(_)),
            "must be refused, got {err:?}"
        );
        // Both link and target survive.
        assert!(fs::symlink_metadata(store.root().join(VALID_ID_2))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(target.join("job.json").exists());
    }
}
