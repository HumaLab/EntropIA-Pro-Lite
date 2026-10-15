//! PURE import-job model (Phase 2 task 13).
//!
//! No filesystem, no network, no clock: callers pass `now` as Unix seconds
//! and own staging-file deletion via the returned effects.
//!
//! State machines (plan §4.2):
//! - file: `Queued -> Downloading -> Verifying -> Verified -> Importing ->
//!   Imported`, plus `SkippedDuplicate`, `Unavailable`, `Failed`, `Cancelled`.
//! - job: `Draft -> Ready -> Running -> Completed | Partial | Failed |
//!   Cancelled`, with `Running -> Cancelling -> Cancelled`.
//!
//! Notes on forced moves (documented, not silent):
//! - `normalize_after_restart` from `Running` forces `Verifying -> Queued`
//!   (clearing staging); the normal table does not allow `Verifying ->
//!   Queued`, but a restart must discard partial verification output.
//! - `normalize_after_restart` from `Cancelling` forces `Importing ->
//!   Cancelled` and reports it in [`RestartEffects::needs_reconcile`]
//!   (never in `staging_to_delete`): the frontend import may have been
//!   interrupted midway, leaving a partial Corpus item, so the caller must
//!   reconcile it before deleting staging.
//! - `request_cancel` never touches `Downloading`/`Verifying` (left to the
//!   worker) nor `Importing` (finishes and confirms normally).
//! - `set_job_state` exists so callers can drive `Draft -> Ready -> Running`
//!   (and `Running -> Ready` restarts); without it a job could never leave
//!   `Draft` except via cancellation.
//! - Staging fields are cleared only where the spec says so (`Running`
//!   restart for `Downloading`/`Verifying`); `request_cancel` and the
//!   `Cancelling` restart only *report* staging for the caller to delete.

use serde::{Deserialize, Serialize};

use crate::item::{Availability, FileSource, PdfFile};
use crate::provenance::ItemSnapshot;

/// Schema version stored in every [`Job`]; bump on incompatible change.
pub const JOB_SCHEMA_VERSION: u32 = 1;

/// Where verified files will be imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportTarget {
    ExistingCollection { collection_id: String },
    NewCollection { name: String },
}

/// Job-level state machine (no pause states in the MVP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Draft,
    Ready,
    Running,
    Cancelling,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

/// Per-file state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Queued,
    Downloading,
    Verifying,
    Verified,
    Importing,
    Imported,
    SkippedDuplicate,
    Unavailable,
    Failed,
    Cancelled,
}

/// One remote PDF tracked by a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobFile {
    pub identifier: String,
    pub file_name: String,
    pub format: Option<String>,
    pub source: FileSource,
    pub expected_size: Option<u64>,
    pub mtime: Option<u64>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub state: FileState,
    pub attempts: u32,
    pub error: Option<String>,
    pub staging_name: Option<String>,
    pub page_count: Option<u32>,
    pub sha256: Option<String>,
    pub item_id: Option<String>,
}

/// An import job: target, state and tracked files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub schema_version: u32,
    pub id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub target: ImportTarget,
    pub state: JobState,
    pub files: Vec<JobFile>,
    #[serde(default)]
    pub items: Vec<ItemSnapshot>,
}

/// Invalid per-file transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid file transition from {from:?} to {to:?}")]
pub struct FileTransitionError {
    pub from: FileState,
    pub to: FileState,
}

/// Invalid job-level transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid job transition from {from:?} to {to:?}")]
pub struct JobTransitionError {
    pub from: JobState,
    pub to: JobState,
}

/// [`Job::new`] validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NewJobError {
    #[error("job must contain at least one file")]
    Empty,
    #[error("duplicate file {identifier}/{file_name}")]
    Duplicate {
        identifier: String,
        file_name: String,
    },
}

/// [`JobFile::from_pdf`] rejection: only `Available` PDFs are importable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("pdf not available: {identifier}/{file_name}")]
pub struct UnavailablePdfError {
    pub identifier: String,
    pub file_name: String,
}

/// [`Job::set_file_state`] failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetFileStateError {
    #[error("file index {index} out of bounds (len {len})")]
    OutOfBounds { index: usize, len: usize },
    #[error(transparent)]
    Transition(#[from] FileTransitionError),
}

/// Staging files the caller must delete after [`Job::request_cancel`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CancelEffects {
    pub staging_to_delete: Vec<String>,
}

/// One file that was `Importing` when a `Cancelling` job was restarted.
/// The file is already `Cancelled` in the model; the entry tells the caller
/// what still needs provenance reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileEntry {
    pub index: usize,
    pub identifier: String,
    pub file_name: String,
    pub staging_name: Option<String>,
}

/// Staging files the caller must delete after [`Job::normalize_after_restart`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestartEffects {
    pub staging_to_delete: Vec<String>,
    /// Files that were `Importing` when a `Cancelling` job restarted.
    /// The caller must resolve each one by provenance (keep a complete
    /// item, discard a partial one with the Corpus cleanup routine) and
    /// only then delete its staging file. Never auto-deleted: the frontend
    /// import may have been interrupted after writing provenance but before
    /// the page assets (plan §4), so blind deletion would leak a partial
    /// item and blind keeping would leak staging.
    pub needs_reconcile: Vec<ReconcileEntry>,
}

impl FileState {
    /// Terminal files never leave except via idempotent self-transition.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Imported
                | Self::SkippedDuplicate
                | Self::Unavailable
                | Self::Failed
                | Self::Cancelled
        )
    }
}

impl JobState {
    /// Terminal jobs never leave except via idempotent self-transition.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

/// True when `from -> to` is an allowed per-file transition, including the
/// idempotent terminal self-transition.
#[must_use]
pub fn is_file_transition_allowed(from: FileState, to: FileState) -> bool {
    if from == to {
        return from.is_terminal();
    }
    matches!(
        (from, to),
        (FileState::Queued, FileState::Downloading)
            | (FileState::Queued, FileState::SkippedDuplicate)
            | (FileState::Queued, FileState::Unavailable)
            | (FileState::Queued, FileState::Cancelled)
            | (FileState::Downloading, FileState::Verifying)
            | (FileState::Downloading, FileState::Queued)
            | (FileState::Downloading, FileState::Unavailable)
            | (FileState::Downloading, FileState::Failed)
            | (FileState::Downloading, FileState::Cancelled)
            | (FileState::Verifying, FileState::Verified)
            | (FileState::Verifying, FileState::Failed)
            | (FileState::Verifying, FileState::Cancelled)
            | (FileState::Verified, FileState::Importing)
            | (FileState::Verified, FileState::Cancelled)
            | (FileState::Importing, FileState::Imported)
            | (FileState::Importing, FileState::Verified)
            | (FileState::Importing, FileState::Failed)
    )
}

/// True when `from -> to` is an allowed job transition, including the
/// idempotent terminal self-transition.
#[must_use]
pub fn is_job_transition_allowed(from: JobState, to: JobState) -> bool {
    if from == to {
        return from.is_terminal();
    }
    matches!(
        (from, to),
        (JobState::Draft, JobState::Ready)
            | (JobState::Draft, JobState::Cancelled)
            | (JobState::Ready, JobState::Running)
            | (JobState::Ready, JobState::Cancelled)
            | (JobState::Running, JobState::Cancelling)
            | (JobState::Running, JobState::Completed)
            | (JobState::Running, JobState::Partial)
            | (JobState::Running, JobState::Failed)
            | (JobState::Running, JobState::Ready)
            | (JobState::Cancelling, JobState::Cancelled)
    )
}

impl JobFile {
    /// Build a `Queued` file from an `Available` PDF; anything else is
    /// rejected so restricted/unavailable files never enter a job.
    pub fn from_pdf(
        identifier: impl Into<String>,
        pdf: &PdfFile,
    ) -> Result<Self, UnavailablePdfError> {
        let identifier = identifier.into();
        if pdf.availability != Availability::Available {
            return Err(UnavailablePdfError {
                identifier,
                file_name: pdf.name.clone(),
            });
        }
        Ok(Self {
            identifier,
            file_name: pdf.name.clone(),
            format: pdf.format.clone(),
            source: pdf.source.clone(),
            expected_size: pdf.size,
            mtime: pdf.mtime,
            md5: pdf.md5.clone(),
            sha1: pdf.sha1.clone(),
            state: FileState::Queued,
            attempts: 0,
            error: None,
            staging_name: None,
            page_count: None,
            sha256: None,
            item_id: None,
        })
    }
}

impl Job {
    /// New job in `Draft`; rejects empty lists and duplicate
    /// `(identifier, file_name)` pairs.
    pub fn new(
        id: impl Into<String>,
        now: u64,
        target: ImportTarget,
        files: Vec<JobFile>,
    ) -> Result<Self, NewJobError> {
        if files.is_empty() {
            return Err(NewJobError::Empty);
        }
        let mut seen = std::collections::HashSet::new();
        for f in &files {
            let key = (f.identifier.as_str(), f.file_name.as_str());
            if !seen.insert(key) {
                return Err(NewJobError::Duplicate {
                    identifier: f.identifier.clone(),
                    file_name: f.file_name.clone(),
                });
            }
        }
        Ok(Self {
            schema_version: JOB_SCHEMA_VERSION,
            id: id.into(),
            created_at: now,
            updated_at: now,
            target,
            state: JobState::Draft,
            files,
            items: Vec::new(),
        })
    }

    /// Snapshot for `identifier`, if the job carries item metadata.
    #[must_use]
    pub fn item(&self, identifier: &str) -> Option<&ItemSnapshot> {
        self.items.iter().find(|s| s.identifier == identifier)
    }

    /// Validated per-file transition; updates `updated_at` on success,
    /// including idempotent terminal self-transitions.
    pub fn set_file_state(
        &mut self,
        index: usize,
        to: FileState,
        now: u64,
    ) -> Result<(), SetFileStateError> {
        let Some(file) = self.files.get(index) else {
            return Err(SetFileStateError::OutOfBounds {
                index,
                len: self.files.len(),
            });
        };
        let from = file.state;
        if !is_file_transition_allowed(from, to) {
            return Err(SetFileStateError::Transition(FileTransitionError {
                from,
                to,
            }));
        }
        self.files[index].state = to;
        self.updated_at = now;
        Ok(())
    }

    /// Validated job transition (drives `Draft -> Ready -> Running` and
    /// `Running -> Ready` restarts); updates `updated_at` on success.
    pub fn set_job_state(&mut self, to: JobState, now: u64) -> Result<(), JobTransitionError> {
        let from = self.state;
        if !is_job_transition_allowed(from, to) {
            return Err(JobTransitionError { from, to });
        }
        self.state = to;
        self.updated_at = now;
        Ok(())
    }

    /// When `Running` and every file is terminal, move to `Completed` (all
    /// `Imported`/`SkippedDuplicate`), `Failed` (none of them) or `Partial`
    /// (otherwise). Returns the new state, or `None` when nothing changed
    /// (leaves `updated_at` untouched).
    pub fn settle(&mut self, now: u64) -> Option<JobState> {
        if self.state != JobState::Running || self.files.is_empty() {
            return None;
        }
        if !self.files.iter().all(|f| f.state.is_terminal()) {
            return None;
        }
        let any_success = self
            .files
            .iter()
            .any(|f| matches!(f.state, FileState::Imported | FileState::SkippedDuplicate));
        let all_success = self
            .files
            .iter()
            .all(|f| matches!(f.state, FileState::Imported | FileState::SkippedDuplicate));
        let next = if all_success {
            JobState::Completed
        } else if any_success {
            JobState::Partial
        } else {
            JobState::Failed
        };
        self.state = next;
        self.updated_at = now;
        Some(next)
    }

    /// Cancellation request. `Draft`/`Ready` go straight to `Cancelled`;
    /// `Running` goes to `Cancelling` with every `Queued`/`Verified` file
    /// moved to `Cancelled` (verified staging reported for the caller to
    /// delete). `Downloading`/`Verifying` are left to the worker and an
    /// `Importing` file is never touched. Any other job state is a no-op
    /// (leaves `updated_at` untouched).
    pub fn request_cancel(&mut self, now: u64) -> CancelEffects {
        match self.state {
            JobState::Draft | JobState::Ready => {
                let mut effects = CancelEffects::default();
                for file in &mut self.files {
                    match file.state {
                        FileState::Queued => file.state = FileState::Cancelled,
                        FileState::Verified => {
                            file.state = FileState::Cancelled;
                            if let Some(name) = file.staging_name.clone() {
                                effects.staging_to_delete.push(name);
                            }
                        }
                        _ => {}
                    }
                }
                self.state = JobState::Cancelled;
                self.updated_at = now;
                effects
            }
            JobState::Running => {
                let mut effects = CancelEffects::default();
                for file in &mut self.files {
                    match file.state {
                        FileState::Queued => file.state = FileState::Cancelled,
                        FileState::Verified => {
                            file.state = FileState::Cancelled;
                            if let Some(name) = file.staging_name.clone() {
                                effects.staging_to_delete.push(name);
                            }
                        }
                        _ => {}
                    }
                }
                self.state = JobState::Cancelling;
                self.updated_at = now;
                effects
            }
            _ => CancelEffects::default(),
        }
    }

    /// When `Cancelling` with no `Downloading`/`Verifying`/`Importing` file
    /// left, move to `Cancelled`. Returns `true` on change, `false` on
    /// no-op (leaves `updated_at` untouched). Files are not touched: the
    /// worker must first move active files to terminal states via the
    /// allowed transitions.
    pub fn finish_cancel_if_idle(&mut self, now: u64) -> bool {
        if self.state != JobState::Cancelling {
            return false;
        }
        let busy = self.files.iter().any(|f| {
            matches!(
                f.state,
                FileState::Downloading | FileState::Verifying | FileState::Importing
            )
        });
        if busy {
            return false;
        }
        self.state = JobState::Cancelled;
        self.updated_at = now;
        true
    }

    /// Restart recovery (idempotent: a second call is a no-op returning an
    /// empty effect and leaving `updated_at` untouched).
    ///
    /// - `Running -> Ready`; `Downloading`/`Verifying` files go back to
    ///   `Queued` with staging cleared and reported; `Importing -> Verified`
    ///   with staging kept.
    /// - `Cancelling -> Cancelled`; every non-terminal file goes to
    ///   `Cancelled` (forcing `Importing`). Staging of every other
    ///   non-terminal file with a name is reported in `staging_to_delete`
    ///   without clearing; each forced `Importing` file is reported in
    ///   `needs_reconcile` (never in `staging_to_delete`) for provenance
    ///   reconciliation before its staging may be deleted.
    /// - Any other job state is a no-op.
    pub fn normalize_after_restart(&mut self, now: u64) -> RestartEffects {
        match self.state {
            JobState::Running => {
                let mut effects = RestartEffects::default();
                for file in &mut self.files {
                    match file.state {
                        FileState::Downloading | FileState::Verifying => {
                            file.state = FileState::Queued;
                            if let Some(name) = file.staging_name.take() {
                                effects.staging_to_delete.push(name);
                            }
                        }
                        FileState::Importing => {
                            file.state = FileState::Verified;
                        }
                        _ => {}
                    }
                }
                self.state = JobState::Ready;
                self.updated_at = now;
                effects
            }
            JobState::Cancelling => {
                let mut effects = RestartEffects::default();
                for (index, file) in self.files.iter_mut().enumerate() {
                    if file.state.is_terminal() {
                        continue;
                    }
                    if file.state == FileState::Importing {
                        file.state = FileState::Cancelled;
                        effects.needs_reconcile.push(ReconcileEntry {
                            index,
                            identifier: file.identifier.clone(),
                            file_name: file.file_name.clone(),
                            staging_name: file.staging_name.clone(),
                        });
                        continue;
                    }
                    file.state = FileState::Cancelled;
                    if let Some(name) = file.staging_name.clone() {
                        effects.staging_to_delete.push(name);
                    }
                }
                self.state = JobState::Cancelled;
                self.updated_at = now;
                effects
            }
            _ => RestartEffects::default(),
        }
    }
}

#[cfg(test)]
mod spec_tests {
    use super::*;
    use crate::item::{Availability, RestrictedReason};

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

    fn restricted_pdf(name: &str) -> PdfFile {
        PdfFile {
            name: name.to_string(),
            format: Some("Text PDF".to_string()),
            source: FileSource::Original,
            size: None,
            mtime: None,
            md5: None,
            sha1: None,
            availability: Availability::Restricted(RestrictedReason::Private),
        }
    }

    fn mk_file(identifier: &str, file_name: &str, state: FileState) -> JobFile {
        let mut f =
            JobFile::from_pdf(identifier, &available_pdf(file_name)).expect("available builds");
        f.state = state;
        f
    }

    fn mk_job(file_states: Vec<FileState>, job_state: JobState) -> Job {
        let files = file_states
            .into_iter()
            .enumerate()
            .map(|(i, s)| mk_file("id", &format!("f{i}.pdf"), s))
            .collect::<Vec<_>>();
        Job {
            schema_version: JOB_SCHEMA_VERSION,
            id: "job".to_string(),
            created_at: 1,
            updated_at: 1,
            target: ImportTarget::ExistingCollection {
                collection_id: "c".to_string(),
            },
            state: job_state,
            files,
            items: Vec::new(),
        }
    }

    #[test]
    fn file_allowed_transitions_table() {
        let allowed: &[(FileState, FileState)] = &[
            (FileState::Queued, FileState::Downloading),
            (FileState::Queued, FileState::SkippedDuplicate),
            (FileState::Queued, FileState::Unavailable),
            (FileState::Queued, FileState::Cancelled),
            (FileState::Downloading, FileState::Verifying),
            (FileState::Downloading, FileState::Queued),
            (FileState::Downloading, FileState::Unavailable),
            (FileState::Downloading, FileState::Failed),
            (FileState::Downloading, FileState::Cancelled),
            (FileState::Verifying, FileState::Verified),
            (FileState::Verifying, FileState::Failed),
            (FileState::Verifying, FileState::Cancelled),
            (FileState::Verified, FileState::Importing),
            (FileState::Verified, FileState::Cancelled),
            (FileState::Importing, FileState::Imported),
            (FileState::Importing, FileState::Verified),
            (FileState::Importing, FileState::Failed),
        ];
        assert_eq!(allowed.len(), 17);
        for (from, to) in allowed {
            let mut job = mk_job(vec![*from], JobState::Running);
            job.set_file_state(0, *to, 99).unwrap_or_else(|e| {
                panic!("allowed {from:?}->{to:?} rejected: {e:?}");
            });
            assert_eq!(job.files[0].state, *to);
            assert_eq!(job.updated_at, 99);
        }
    }

    #[test]
    fn file_forbidden_transitions_rejected() {
        let forbidden: &[(FileState, FileState)] = &[
            (FileState::Queued, FileState::Queued),
            (FileState::Queued, FileState::Verified),
            (FileState::Queued, FileState::Failed),
            (FileState::Downloading, FileState::Downloading),
            (FileState::Downloading, FileState::Imported),
            (FileState::Verifying, FileState::Verifying),
            (FileState::Verifying, FileState::Unavailable),
            (FileState::Verified, FileState::Verified),
            (FileState::Verified, FileState::Failed),
            (FileState::Verified, FileState::Imported),
            (FileState::Importing, FileState::Importing),
            (FileState::Importing, FileState::Cancelled),
            (FileState::Importing, FileState::Queued),
            (FileState::Imported, FileState::Verified),
            (FileState::Failed, FileState::Queued),
            (FileState::Cancelled, FileState::Queued),
        ];
        for (from, to) in forbidden {
            let mut job = mk_job(vec![*from], JobState::Running);
            match job.set_file_state(0, *to, 99) {
                Err(SetFileStateError::Transition(e)) => {
                    assert_eq!(e.from, *from);
                    assert_eq!(e.to, *to);
                }
                other => panic!("forbidden {from:?}->{to:?} gave {other:?}"),
            }
            assert_eq!(job.files[0].state, *from);
        }
    }

    #[test]
    fn file_terminal_self_transition_idempotent() {
        for s in [
            FileState::Imported,
            FileState::SkippedDuplicate,
            FileState::Unavailable,
            FileState::Failed,
            FileState::Cancelled,
        ] {
            let mut job = mk_job(vec![s], JobState::Running);
            job.set_file_state(0, s, 77).expect("terminal self Ok");
            assert_eq!(job.files[0].state, s);
        }
    }

    #[test]
    fn job_allowed_transitions_table() {
        let allowed: &[(JobState, JobState)] = &[
            (JobState::Draft, JobState::Ready),
            (JobState::Draft, JobState::Cancelled),
            (JobState::Ready, JobState::Running),
            (JobState::Ready, JobState::Cancelled),
            (JobState::Running, JobState::Cancelling),
            (JobState::Running, JobState::Completed),
            (JobState::Running, JobState::Partial),
            (JobState::Running, JobState::Failed),
            (JobState::Running, JobState::Ready),
            (JobState::Cancelling, JobState::Cancelled),
        ];
        assert_eq!(allowed.len(), 10);
        for (from, to) in allowed {
            let mut job = mk_job(vec![FileState::Queued], *from);
            job.set_job_state(*to, 55).unwrap_or_else(|e| {
                panic!("allowed job {from:?}->{to:?} rejected: {e:?}");
            });
            assert_eq!(job.state, *to);
        }
    }

    #[test]
    fn job_forbidden_and_terminal_idempotence() {
        for (from, to) in [
            (JobState::Draft, JobState::Draft),
            (JobState::Draft, JobState::Running),
            (JobState::Ready, JobState::Ready),
            (JobState::Running, JobState::Running),
            (JobState::Running, JobState::Draft),
            (JobState::Cancelling, JobState::Running),
            (JobState::Completed, JobState::Running),
        ] {
            let mut job = mk_job(vec![FileState::Queued], from);
            match job.set_job_state(to, 9) {
                Err(e) => {
                    assert_eq!(e.from, from);
                    assert_eq!(e.to, to);
                }
                Ok(()) => panic!("forbidden job {from:?}->{to:?} accepted"),
            }
        }
        for s in [
            JobState::Completed,
            JobState::Partial,
            JobState::Failed,
            JobState::Cancelled,
        ] {
            let mut job = mk_job(vec![FileState::Queued], s);
            job.set_job_state(s, 9).expect("terminal self Ok");
            assert_eq!(job.state, s);
        }
    }

    #[test]
    fn job_new_validation() {
        let err = Job::new(
            "j",
            1,
            ImportTarget::ExistingCollection {
                collection_id: "c".to_string(),
            },
            vec![],
        )
        .unwrap_err();
        assert_eq!(err, NewJobError::Empty);
        let f1 = mk_file("a", "x.pdf", FileState::Queued);
        let f2 = mk_file("a", "x.pdf", FileState::Queued);
        let err = Job::new(
            "j",
            1,
            ImportTarget::NewCollection {
                name: "n".to_string(),
            },
            vec![f1, f2],
        )
        .unwrap_err();
        assert_eq!(
            err,
            NewJobError::Duplicate {
                identifier: "a".to_string(),
                file_name: "x.pdf".to_string(),
            }
        );
        let f1 = mk_file("a", "x.pdf", FileState::Queued);
        let f2 = mk_file("b", "x.pdf", FileState::Queued);
        let job = Job::new(
            "j",
            7,
            ImportTarget::NewCollection { name: "n".into() },
            vec![f1, f2],
        )
        .unwrap();
        assert_eq!(job.state, JobState::Draft);
        assert_eq!(job.schema_version, JOB_SCHEMA_VERSION);
        let err = JobFile::from_pdf("a", &restricted_pdf("x.pdf")).unwrap_err();
        assert_eq!(
            err,
            UnavailablePdfError {
                identifier: "a".to_string(),
                file_name: "x.pdf".to_string(),
            }
        );
        let f = JobFile::from_pdf("ident", &available_pdf("doc.pdf")).unwrap();
        assert_eq!(f.identifier, "ident");
        assert_eq!(f.file_name, "doc.pdf");
        assert_eq!(f.state, FileState::Queued);
        assert_eq!(f.attempts, 0);
    }

    #[test]
    fn settle_outcomes_and_noop() {
        let mut job = mk_job(
            vec![FileState::Imported, FileState::SkippedDuplicate],
            JobState::Running,
        );
        assert_eq!(job.settle(10), Some(JobState::Completed));
        assert_eq!(job.state, JobState::Completed);

        let mut job = mk_job(
            vec![FileState::Failed, FileState::Unavailable],
            JobState::Running,
        );
        assert_eq!(job.settle(10), Some(JobState::Failed));
        assert_eq!(job.state, JobState::Failed);

        let mut job = mk_job(
            vec![FileState::Imported, FileState::Failed],
            JobState::Running,
        );
        assert_eq!(job.settle(10), Some(JobState::Partial));
        assert_eq!(job.state, JobState::Partial);

        let mut job = mk_job(
            vec![FileState::Imported, FileState::Queued],
            JobState::Running,
        );
        assert_eq!(job.settle(10), None);
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.updated_at, 1);

        let mut job = mk_job(vec![FileState::Imported], JobState::Draft);
        assert_eq!(job.settle(10), None);
        assert_eq!(job.state, JobState::Draft);
    }

    #[test]
    fn request_cancel_from_each_state() {
        let mut job = mk_job(vec![FileState::Queued], JobState::Draft);
        let fx = job.request_cancel(20);
        assert_eq!(job.state, JobState::Cancelled);
        assert_eq!(job.files[0].state, FileState::Cancelled);
        assert!(fx.staging_to_delete.is_empty());

        let mut job = mk_job(vec![FileState::Queued], JobState::Ready);
        job.request_cancel(20);
        assert_eq!(job.state, JobState::Cancelled);

        let mut job = mk_job(
            vec![
                FileState::Queued,
                FileState::Verified,
                FileState::Downloading,
                FileState::Verifying,
                FileState::Importing,
                FileState::Imported,
            ],
            JobState::Running,
        );
        job.files[1].staging_name = Some("v.pdf".to_string());
        job.files[4].staging_name = Some("i.pdf".to_string());
        let fx = job.request_cancel(30);
        assert_eq!(job.state, JobState::Cancelling);
        assert_eq!(job.files[0].state, FileState::Cancelled);
        assert_eq!(job.files[1].state, FileState::Cancelled);
        assert_eq!(job.files[2].state, FileState::Downloading);
        assert_eq!(job.files[3].state, FileState::Verifying);
        assert_eq!(job.files[4].state, FileState::Importing);
        assert_eq!(job.files[5].state, FileState::Imported);
        assert_eq!(fx.staging_to_delete, vec!["v.pdf".to_string()]);
        assert!(!fx.staging_to_delete.contains(&"i.pdf".to_string()));

        let mut job = mk_job(vec![FileState::Queued], JobState::Cancelling);
        let fx = job.request_cancel(40);
        assert_eq!(job.state, JobState::Cancelling);
        assert!(fx.staging_to_delete.is_empty());
        for s in [JobState::Completed, JobState::Failed, JobState::Cancelled] {
            let mut job = mk_job(vec![FileState::Queued], s);
            job.request_cancel(40);
            assert_eq!(job.state, s);
        }
    }

    #[test]
    fn finish_cancel_if_idle_rules() {
        let mut job = mk_job(
            vec![FileState::Cancelled, FileState::Imported],
            JobState::Cancelling,
        );
        assert!(job.finish_cancel_if_idle(50));
        assert_eq!(job.state, JobState::Cancelled);

        for active in [
            FileState::Downloading,
            FileState::Verifying,
            FileState::Importing,
        ] {
            let mut job = mk_job(vec![active], JobState::Cancelling);
            assert!(!job.finish_cancel_if_idle(50));
            assert_eq!(job.state, JobState::Cancelling);
        }
        let mut job = mk_job(vec![FileState::Cancelled], JobState::Running);
        assert!(!job.finish_cancel_if_idle(50));
    }

    #[test]
    fn normalize_after_restart_rules_and_idempotence() {
        let mut job = mk_job(
            vec![
                FileState::Downloading,
                FileState::Verifying,
                FileState::Importing,
                FileState::Verified,
                FileState::Queued,
            ],
            JobState::Running,
        );
        job.files[0].staging_name = Some("d.pdf".to_string());
        job.files[1].staging_name = Some("v.pdf".to_string());
        job.files[2].staging_name = Some("i.pdf".to_string());
        let fx = job.normalize_after_restart(60);
        assert_eq!(job.state, JobState::Ready);
        assert_eq!(job.files[0].state, FileState::Queued);
        assert_eq!(job.files[0].staging_name, None);
        assert_eq!(job.files[1].state, FileState::Queued);
        assert_eq!(job.files[1].staging_name, None);
        assert_eq!(job.files[2].state, FileState::Verified);
        assert_eq!(job.files[2].staging_name, Some("i.pdf".to_string()));
        assert!(fx.staging_to_delete.contains(&"d.pdf".to_string()));
        assert!(fx.staging_to_delete.contains(&"v.pdf".to_string()));
        assert!(!fx.staging_to_delete.contains(&"i.pdf".to_string()));
        let updated = job.updated_at;
        let fx2 = job.normalize_after_restart(70);
        assert!(fx2.staging_to_delete.is_empty());
        assert_eq!(job.updated_at, updated);
        assert_eq!(job.state, JobState::Ready);

        let mut job = mk_job(
            vec![FileState::Queued, FileState::Verified, FileState::Importing],
            JobState::Cancelling,
        );
        job.files[1].staging_name = Some("vv.pdf".to_string());
        job.files[2].staging_name = Some("imp.pdf".to_string());
        let fx = job.normalize_after_restart(80);
        assert_eq!(job.state, JobState::Cancelled);
        assert!(job.files.iter().all(|f| f.state == FileState::Cancelled));
        assert!(fx.staging_to_delete.contains(&"vv.pdf".to_string()));
        assert!(!fx.staging_to_delete.contains(&"imp.pdf".to_string()));
        assert_eq!(fx.needs_reconcile.len(), 1);
        let entry = &fx.needs_reconcile[0];
        assert_eq!(entry.index, 2);
        assert_eq!(entry.identifier, "id");
        assert_eq!(entry.file_name, "f2.pdf");
        assert_eq!(entry.staging_name.as_deref(), Some("imp.pdf"));
        let updated = job.updated_at;
        let fx2 = job.normalize_after_restart(90);
        assert!(fx2.staging_to_delete.is_empty());
        assert!(fx2.needs_reconcile.is_empty());
        assert_eq!(job.updated_at, updated);
    }

    #[test]
    fn json_round_trip_and_enum_spelling() {
        let mut f = mk_file("ident", "a.pdf", FileState::SkippedDuplicate);
        f.staging_name = Some("s.pdf".to_string());
        f.page_count = Some(12);
        f.sha256 = Some("abc".to_string());
        f.item_id = Some("item1".to_string());
        let job = Job {
            schema_version: JOB_SCHEMA_VERSION,
            id: "j1".to_string(),
            created_at: 5,
            updated_at: 6,
            target: ImportTarget::ExistingCollection {
                collection_id: "c1".to_string(),
            },
            state: JobState::Running,
            files: vec![f],
            items: Vec::new(),
        };
        let json = serde_json::to_string(&job).unwrap();
        assert!(json.contains("\"skipped_duplicate\""), "json: {json}");
        assert!(json.contains("\"existing_collection\""), "json: {json}");
        let back: Job = serde_json::from_str(&json).unwrap();
        assert_eq!(back, job);

        let job2 = Job {
            target: ImportTarget::NewCollection {
                name: "Nueva".to_string(),
            },
            ..job.clone()
        };
        let json2 = serde_json::to_string(&job2).unwrap();
        assert!(json2.contains("\"new_collection\""), "json: {json2}");
        let back2: Job = serde_json::from_str(&json2).unwrap();
        assert_eq!(back2, job2);
    }
}
