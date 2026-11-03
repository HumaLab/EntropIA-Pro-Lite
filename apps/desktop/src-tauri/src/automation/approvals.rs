//! The owner's approval of paid work an assistant asked for (D3, S4, S5).
//!
//! An assistant can never start, resume or retry paid processing by itself. Its
//! request becomes a pending approval here, bound to who asked (client id),
//! what they asked (the action) and exactly which state of which job they
//! looked at (job id and revision). The app shows it; only the person's answer
//! in the app executes it, through [`ApprovalStore::decide`] and the batch
//! services, with that same revision as the fence: if the job changed in the
//! meantime the start refuses instead of running something else.
//!
//! Approvals are single use and expire ten minutes after they were asked for.
//! They live in memory only, so an app restart leaves nothing approved. Nothing
//! an assistant sends can mark an approval approved: the endpoint can only
//! create them and read the caller's own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::processing::commands::{self as batches, ControlRequest};

/// How long a request waits for the person (D13).
pub const APPROVAL_TTL_MS: i64 = 10 * 60 * 1000;

/// The event the app listens to: re-read the pending approvals.
pub const APPROVALS_CHANGED_EVENT: &str = "agent-approvals:changed";

/// What changed, so the app knows when to come to the front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChange {
    /// An assistant asked: show the request and bring the window forward.
    Requested,
    /// The person answered.
    Decided,
    /// A request ran out of time.
    Expired,
}

/// What the assistant asked to do with a job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ApprovalAction {
    /// Start a prepared draft.
    Start,
    /// Resume a paused job.
    Resume,
    /// Reopen failed work: one task, or every failed task.
    Retry {
        task_id: Option<String>,
        failed_only: bool,
    },
    /// Replace the recognized text of one page. Bound to the page text the
    /// assistant read (`text_hash`) and to the text it proposes.
    ReplaceText {
        item_id: String,
        asset_id: String,
        text_hash: String,
        new_text_hash: String,
    },
    /// Import files from the owner's enabled folders into a collection. Bound
    /// to the exact files (path, size, modification time) the plan listed.
    ImportFiles {
        collection_id: String,
        files: usize,
        plan_hash: String,
    },
    /// Delete documents. Bound to the exact set the assistant listed: its ids
    /// sorted, and `set_hash` over each one's revision and asset count.
    DeleteItems {
        item_ids: Vec<String>,
        set_hash: String,
    },
    /// Delete one collection and everything it holds. `set_hash` covers every
    /// document the collection had, so one gained or lost changes it.
    DeleteCollection {
        collection_id: String,
        set_hash: String,
    },
}

/// One collection a delete touches, as the app refreshes it afterwards: the
/// collection and one of the documents that go with it. Kept in memory only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    pub collection_id: String,
    pub item_id: String,
}

/// What a delete would remove, as the dialog and the assistant see it: the
/// impact counts, the first document titles and the collections to refresh,
/// never a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePlan {
    pub impact: crate::corpus::delete::DeleteImpact,
    pub titles: Vec<String>,
    pub targets: Vec<DeleteTarget>,
}

/// One file of an import request, as the dialog and the assistant see it:
/// its name, kind and size, never its path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportFileView {
    pub name: String,
    pub kind: crate::corpus::ingest::FileKind,
    pub size_bytes: u64,
}

/// One imported file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFileReport {
    pub name: String,
    pub item_id: String,
    pub title: String,
}

/// One file that was not imported, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFileReport {
    pub name: String,
    /// A stable reason first (`changed`, `duplicate`, `not_in_folder`,
    /// `missing`, `failed`, …), then a sentence.
    pub reason: String,
}

/// What an approved import did, file by file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub collection_id: String,
    pub imported: Vec<ImportedFileReport>,
    pub skipped: Vec<SkippedFileReport>,
}

/// What executing an approved action needs from the app beyond a connection:
/// the archive's data directory, its cache directory (thumbnails live there)
/// and the folders imports may read right now.
#[derive(Debug, Clone, Default)]
pub struct ExecContext {
    pub data_dir: Option<std::path::PathBuf>,
    pub cache_dir: Option<std::path::PathBuf>,
    pub folders: Vec<std::path::PathBuf>,
}

/// Where an approval stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    /// Waiting for the person.
    Pending,
    /// The person approved and the action ran.
    Approved,
    /// The person approved but the action could not run (the job changed).
    Failed,
    /// The person said no.
    Rejected,
    /// Nobody answered in time.
    Expired,
}

/// One approval as both the app and the assistant see it. It carries ids,
/// names and counts only: never a token, never document content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub action: ApprovalAction,
    /// The job the action runs on; empty for an action that is not about a
    /// job (`replace_text` names its page in `action`).
    pub job_id: String,
    /// The job revision the assistant looked at; the action runs only at it.
    pub revision: i64,
    pub operations: Vec<String>,
    /// Collection names in the job's scope, for the dialog.
    pub collections: Vec<String>,
    /// Files in the job's scope.
    pub files: i64,
    pub status: ApprovalStatus,
    /// Unix milliseconds.
    pub created_at: i64,
    pub expires_at: i64,
    pub decided_at: Option<i64>,
    /// Why an approved action failed, in one sentence.
    pub failure: Option<String>,
    /// The start of the proposed text, for the dialog (`replace_text`).
    pub preview: Option<String>,
    /// The whole proposed text. Kept here until it runs; never serialized,
    /// so neither the app's list nor `approvals_status` carries it.
    #[serde(skip)]
    #[schemars(skip)]
    pub text: Option<String>,
    /// The files an import would bring in (`import_files`), without paths.
    pub import_files: Option<Vec<ImportFileView>>,
    /// What deleting the request's scope would remove (`delete_items`,
    /// `delete_collection`).
    pub delete_impact: Option<crate::corpus::delete::DeleteImpact>,
    /// The documents the delete would remove, first titles only; `files` says
    /// how many there are.
    pub document_titles: Option<Vec<String>>,
    /// The collections a delete touches, so the app refreshes them after it.
    /// Kept here until it runs; never serialized.
    #[serde(skip)]
    #[schemars(skip)]
    pub delete_targets: Option<Vec<DeleteTarget>>,
    /// The checked files an import would copy. Kept here until it runs;
    /// never serialized, so paths stay in the app.
    #[serde(skip)]
    #[schemars(skip)]
    pub import_plan: Option<Vec<crate::corpus::ingest::SourceFile>>,
    /// What an approved import did, file by file.
    pub import_result: Option<ImportResult>,
}

/// What the endpoint knows when an assistant asks.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub client_id: String,
    pub client_name: String,
    pub action: ApprovalAction,
    pub job_id: String,
    pub revision: i64,
    pub operations: Vec<String>,
    pub collections: Vec<String>,
    pub files: i64,
    pub preview: Option<String>,
    pub text: Option<String>,
    pub import_plan: Option<Vec<crate::corpus::ingest::SourceFile>>,
    /// What a delete would remove, for the dialog; absent for every other
    /// action.
    pub delete: Option<DeletePlan>,
}

/// Called after every change, so the app can show the request and come to the
/// front. Never called with the store's lock held.
pub type ApprovalNotifier = Arc<dyn Fn(ApprovalChange) + Send + Sync>;

/// The pending and recent approvals of this app run.
pub struct ApprovalStore {
    approvals: Mutex<HashMap<String, Approval>>,
    notifier: Mutex<Option<ApprovalNotifier>>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::with_clock(Arc::new(crate::processing::repository::now_ms))
    }
}

impl ApprovalStore {
    /// A store reading time from `now_ms`, so tests can move the clock.
    pub fn with_clock(now_ms: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self {
            approvals: Mutex::new(HashMap::new()),
            notifier: Mutex::new(None),
            now_ms,
        }
    }

    /// Install what runs after every change (the app's event and window focus).
    pub fn set_notifier(&self, notifier: ApprovalNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    fn notify(&self, change: ApprovalChange) {
        let notifier = self.notifier.lock().ok().and_then(|slot| slot.clone());
        if let Some(notifier) = notifier {
            notifier(change);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Approval>> {
        // A poisoned lock only means a panic elsewhere; the map is still valid.
        self.approvals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Mark every pending approval past its deadline as expired.
    fn expire(&self, approvals: &mut HashMap<String, Approval>, now: i64) -> bool {
        let mut changed = false;
        for approval in approvals.values_mut() {
            if approval.status == ApprovalStatus::Pending && now >= approval.expires_at {
                approval.status = ApprovalStatus::Expired;
                changed = true;
            }
        }
        changed
    }

    /// Ask the person. The same client asking the same action on the same job
    /// at the same revision while a request is pending gets that request back,
    /// so a retrying assistant opens one dialog, not many.
    pub fn request(&self, request: ApprovalRequest) -> Approval {
        let now = (self.now_ms)();
        let approval = {
            let mut approvals = self.lock();
            self.expire(&mut approvals, now);
            let existing = approvals.values().find(|approval| {
                approval.status == ApprovalStatus::Pending
                    && approval.client_id == request.client_id
                    && approval.job_id == request.job_id
                    && approval.revision == request.revision
                    && approval.action == request.action
            });
            if let Some(existing) = existing {
                return existing.clone();
            }
            let approval = Approval {
                id: format!("apr_{}", uuid::Uuid::new_v4().simple()),
                client_id: request.client_id,
                client_name: request.client_name,
                action: request.action,
                job_id: request.job_id,
                revision: request.revision,
                operations: request.operations,
                collections: request.collections,
                files: request.files,
                status: ApprovalStatus::Pending,
                created_at: now,
                expires_at: now + APPROVAL_TTL_MS,
                decided_at: None,
                failure: None,
                preview: request.preview,
                text: request.text,
                import_files: request.import_plan.as_ref().map(|plan| {
                    plan.iter()
                        .map(|file| ImportFileView {
                            name: file.name.clone(),
                            kind: file.kind,
                            size_bytes: file.size_bytes,
                        })
                        .collect()
                }),
                import_plan: request.import_plan,
                import_result: None,
                delete_impact: request.delete.as_ref().map(|plan| plan.impact),
                document_titles: request.delete.as_ref().map(|plan| plan.titles.clone()),
                delete_targets: request.delete.map(|plan| plan.targets),
            };
            approvals.insert(approval.id.clone(), approval.clone());
            approval
        };
        self.notify(ApprovalChange::Requested);
        approval
    }

    /// One approval, for the client that asked for it only: another client's
    /// id reads as unknown.
    pub fn get_for_client(&self, client_id: &str, approval_id: &str) -> Option<Approval> {
        let now = (self.now_ms)();
        let mut approvals = self.lock();
        self.expire(&mut approvals, now);
        approvals
            .get(approval_id)
            .filter(|approval| approval.client_id == client_id)
            .cloned()
    }

    /// What the app's dialog lists: the requests still waiting, oldest first.
    pub fn pending(&self) -> Vec<Approval> {
        let now = (self.now_ms)();
        let changed;
        let mut pending: Vec<Approval> = {
            let mut approvals = self.lock();
            changed = self.expire(&mut approvals, now);
            approvals
                .values()
                .filter(|approval| approval.status == ApprovalStatus::Pending)
                .cloned()
                .collect()
        };
        if changed {
            self.notify(ApprovalChange::Expired);
        }
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        pending
    }

    /// The person's answer. Approving runs the action once, fenced by the
    /// revision the assistant saw; rejecting runs nothing. An approval that is
    /// unknown, already answered or expired is refused and nothing runs.
    pub fn decide(
        &self,
        conn: &Connection,
        approval_id: &str,
        approve: bool,
    ) -> Result<Approval, String> {
        self.decide_with(conn, approval_id, approve, &ExecContext::default())
    }

    /// [`Self::decide`] with what an import needs: the data directory and the
    /// folders enabled at the moment of the answer.
    pub fn decide_with(
        &self,
        conn: &Connection,
        approval_id: &str,
        approve: bool,
        context: &ExecContext,
    ) -> Result<Approval, String> {
        let now = (self.now_ms)();
        // Claim it under the lock so two answers can never both execute.
        let claimed = {
            let mut approvals = self.lock();
            self.expire(&mut approvals, now);
            let approval = approvals
                .get_mut(approval_id)
                .ok_or_else(|| format!("unknown_approval: no request {approval_id}"))?;
            match approval.status {
                ApprovalStatus::Pending => {}
                ApprovalStatus::Expired => {
                    return Err(format!(
                        "approval_expired: request {approval_id} expired before it was answered"
                    ))
                }
                _ => {
                    return Err(format!(
                        "approval_used: request {approval_id} was already answered"
                    ))
                }
            }
            approval.decided_at = Some(now);
            if !approve {
                approval.status = ApprovalStatus::Rejected;
                None
            } else {
                approval.status = ApprovalStatus::Approved;
                Some(approval.clone())
            }
        };
        if let Some(approval) = claimed {
            let outcome = match &approval.action {
                ApprovalAction::ImportFiles { collection_id, .. } => {
                    run_import(conn, &approval, collection_id, context).map(Some)
                }
                ApprovalAction::DeleteItems { .. } | ApprovalAction::DeleteCollection { .. } => {
                    run_delete(conn, &approval, context).map(|()| None)
                }
                _ => execute(conn, &approval).map(|()| None),
            };
            let mut approvals = self.lock();
            if let Some(stored) = approvals.get_mut(approval_id) {
                // The proposed text and file list have done their job either way.
                stored.text = None;
                stored.import_plan = None;
                match outcome {
                    Ok(result) => stored.import_result = result,
                    Err(error) => {
                        stored.status = ApprovalStatus::Failed;
                        stored.failure = Some(failure_sentence(&error));
                    }
                }
            }
        }
        self.notify(ApprovalChange::Decided);
        self.lock()
            .get(approval_id)
            .cloned()
            .ok_or_else(|| format!("unknown_approval: no request {approval_id}"))
    }
}

/// Run an approved action with the revision it was approved at.
fn execute(conn: &Connection, approval: &Approval) -> Result<(), String> {
    match &approval.action {
        ApprovalAction::Start => {
            batches::start_batch(conn, &approval.job_id, Some(approval.revision)).map(|_| ())
        }
        ApprovalAction::Resume => batches::control_batches(
            conn,
            &ControlRequest {
                batch_id: Some(approval.job_id.clone()),
                action: "resume".to_string(),
                expected_revision: Some(approval.revision),
            },
        )
        .map(|_| ()),
        ApprovalAction::Retry {
            task_id,
            failed_only,
        } => {
            fence(conn, &approval.job_id, approval.revision)?;
            batches::retry_batch(
                conn,
                // The approval id is the idempotency key: one approval, one cycle.
                &format!("agent:{}:{}", approval.client_id, approval.id),
                &approval.job_id,
                task_id.as_deref(),
                *failed_only,
            )
            .map(|_| ())
        }
        ApprovalAction::ReplaceText {
            asset_id,
            text_hash,
            ..
        } => {
            let text = approval
                .text
                .as_deref()
                .ok_or_else(|| "invalid_transition: the proposed text is gone".to_string())?;
            crate::corpus::replace_page_text(conn, asset_id, text_hash, text)
                .map(|_| ())
                .map_err(|error| match error {
                    crate::corpus::CorpusError::RevisionConflict(detail) => {
                        format!("revision_conflict: {detail}")
                    }
                    other => other.to_string(),
                })
        }
        // Imports run through `run_import`, which needs the app's context.
        ApprovalAction::ImportFiles { .. } => {
            Err("invalid_transition: an import needs the app's context".to_string())
        }
        // Deletes run through `run_delete`: they need the archive's folders.
        ApprovalAction::DeleteItems { .. } | ApprovalAction::DeleteCollection { .. } => {
            Err("invalid_transition: a delete needs the app's context".to_string())
        }
    }
}

/// Delete what the owner approved, only while it is still exactly what the
/// dialog showed (D11 e). The fence is checked and the rows removed in one
/// transaction, so a document that changed or a collection that gained or lost
/// one fails the approval and nothing is deleted.
fn run_delete(conn: &Connection, approval: &Approval, context: &ExecContext) -> Result<(), String> {
    let data_dir = context
        .data_dir
        .as_deref()
        .ok_or_else(|| "invalid_transition: the archive folder is unknown".to_string())?;
    let cache_dir = context
        .cache_dir
        .as_deref()
        .ok_or_else(|| "invalid_transition: the cache folder is unknown".to_string())?;
    match &approval.action {
        ApprovalAction::DeleteItems { item_ids, set_hash } => {
            crate::corpus::delete_items_if_unchanged(conn, data_dir, cache_dir, item_ids, set_hash)
        }
        ApprovalAction::DeleteCollection {
            collection_id,
            set_hash,
        } => crate::corpus::delete_collection_if_unchanged(
            conn,
            data_dir,
            cache_dir,
            collection_id,
            set_hash,
        ),
        _ => Ok(()),
    }
    .map_err(delete_error)
}

/// The stable prefix a failed delete carries, so its sentence never borrows
/// the wording of a changed job.
fn delete_error(error: crate::corpus::CorpusError) -> String {
    match error {
        crate::corpus::CorpusError::RevisionConflict(detail) => {
            format!("delete_changed: {detail}")
        }
        crate::corpus::CorpusError::NotFound(detail) => format!("delete_missing: {detail}"),
        other => other.to_string(),
    }
}

/// Import the files an approved request listed. Each file is checked again
/// against the folders enabled now: a file that left them, changed size or
/// modification time, or is already in the collection is skipped with its
/// reason, so an approval never imports something other than what it showed.
fn run_import(
    conn: &Connection,
    approval: &Approval,
    collection_id: &str,
    context: &ExecContext,
) -> Result<ImportResult, String> {
    use crate::corpus::ingest::{
        check_source, import_file, ImportOutcome, ImportOverrides, SourceLimits,
    };
    let data_dir = context
        .data_dir
        .as_deref()
        .ok_or_else(|| "invalid_transition: the archive folder is unknown".to_string())?;
    let plan = approval
        .import_plan
        .as_ref()
        .ok_or_else(|| "invalid_transition: the file list is gone".to_string())?;
    let mut result = ImportResult {
        collection_id: collection_id.to_string(),
        ..ImportResult::default()
    };
    let limits = SourceLimits::default();
    for planned in plan {
        let skip = |reason: String| SkippedFileReport {
            name: planned.name.clone(),
            reason,
        };
        let current = match check_source(
            &planned.path.to_string_lossy(),
            &context.folders,
            data_dir,
            &limits,
        ) {
            Ok(current) => current,
            // Keep the stable code and name the file; the check's sentence
            // carries the path, which never leaves the app.
            Err(reason) => {
                let code = reason.split(':').next().unwrap_or("invalid");
                result
                    .skipped
                    .push(skip(format!("{code}: {} was not imported", planned.name)));
                continue;
            }
        };
        if current.size_bytes != planned.size_bytes || current.modified_at != planned.modified_at {
            result.skipped.push(skip(format!(
                "changed: {} changed after the request; it was not imported",
                planned.name
            )));
            continue;
        }
        match import_file(
            conn,
            data_dir,
            collection_id,
            &current,
            &ImportOverrides::default(),
        ) {
            Ok(ImportOutcome::Imported { item_id, title, .. }) => {
                result.imported.push(ImportedFileReport {
                    name: planned.name.clone(),
                    item_id,
                    title,
                })
            }
            Ok(ImportOutcome::Duplicate { item_id }) => result.skipped.push(skip(format!(
                "duplicate: {} is already in the collection as item {item_id}",
                planned.name
            ))),
            Err(error) => {
                log::warn!(target: "automation", "an approved import of one file failed: {error}");
                let reason = error.split(':').next().unwrap_or("failed");
                result.skipped.push(skip(format!(
                    "{reason}: {} could not be imported",
                    planned.name
                )));
            }
        }
    }
    Ok(result)
}

/// Retry has no revision fence of its own; check the approved one first.
fn fence(conn: &Connection, job_id: &str, revision: i64) -> Result<(), String> {
    let current = batches::batch_snapshot(conn, job_id)?.revision;
    if current != revision {
        return Err(format!(
            "revision_conflict: batch {job_id} is at revision {current}, not {revision}"
        ));
    }
    Ok(())
}

/// The sentence the dialog and the assistant show for a failed approved action.
fn failure_sentence(error: &str) -> String {
    if error.starts_with("delete_changed:") {
        "Lo que se iba a borrar cambió después del pedido; no se borró nada. El asistente tiene que volver a pedirlo.".to_string()
    } else if error.starts_with("delete_missing:") {
        "Lo que se iba a borrar ya no existe en este archivo; no se borró nada.".to_string()
    } else if error.starts_with("revision_conflict:") {
        "El trabajo cambió después del pedido; no se ejecutó nada. El asistente tiene que volver a pedirlo.".to_string()
    } else if error.starts_with("invalid_transition:") {
        "El trabajo ya no está en un estado que admita esta acción; no se ejecutó nada.".to_string()
    } else {
        log::warn!(target: "automation", "an approved agent action failed: {error}");
        "No se pudo ejecutar la acción aprobada; no se ejecutó nada.".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::test_archive::{seeded_archive, ALFA};
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    struct Fixture {
        _dir: tempfile::TempDir,
        db_path: std::path::PathBuf,
        conn: Connection,
        clock: Arc<AtomicI64>,
        store: ApprovalStore,
        notified: Arc<AtomicUsize>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_archive(dir.path());
        let conn = batches::open_ready(&db_path).unwrap();
        let clock = Arc::new(AtomicI64::new(1_000_000));
        let reader = clock.clone();
        let store = ApprovalStore::with_clock(Arc::new(move || reader.load(Ordering::SeqCst)));
        let notified = Arc::new(AtomicUsize::new(0));
        let counter = notified.clone();
        store.set_notifier(Arc::new(move |change| {
            if change == ApprovalChange::Requested {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }));
        Fixture {
            _dir: dir,
            db_path,
            conn,
            clock,
            store,
            notified,
        }
    }

    /// A draft planning has finished, the state an assistant asks to start.
    fn ready_job(conn: &Connection) -> (String, i64) {
        let prepared = batches::prepare_batch(
            conn,
            "req-1",
            &[ALFA.to_string()],
            &[],
            &["ocr".to_string()],
        )
        .unwrap();
        conn.execute(
            "UPDATE processing_batches SET state = 'ready', planning_done = 1 WHERE id = ?1",
            [&prepared.batch_id],
        )
        .unwrap();
        let revision = batches::batch_snapshot(conn, &prepared.batch_id)
            .unwrap()
            .revision;
        (prepared.batch_id, revision)
    }

    fn start_request(client: &str, job: &str, revision: i64) -> ApprovalRequest {
        ApprovalRequest {
            client_id: client.to_string(),
            client_name: "Claude Code".to_string(),
            action: ApprovalAction::Start,
            job_id: job.to_string(),
            revision,
            operations: vec!["ocr".to_string()],
            collections: vec!["Alfa".to_string()],
            files: 2,
            preview: None,
            text: None,
            import_plan: None,
            delete: None,
        }
    }

    fn sha(text: &str) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(text.as_bytes()))
    }

    /// A page with recognized text and a request to replace it.
    fn replace_request(conn: &Connection, current: &str, new_text: &str) -> ApprovalRequest {
        conn.execute(
            "INSERT OR REPLACE INTO extractions (id, asset_id, text_content, method, confidence, created_at)
             VALUES ('ext_a', 'ast_alfa_1', ?1, 'ocr', 0.8, 1000)",
            [current],
        )
        .unwrap();
        ApprovalRequest {
            client_id: "cl_a".to_string(),
            client_name: "Claude Code".to_string(),
            action: ApprovalAction::ReplaceText {
                item_id: crate::automation::test_archive::item_id(ALFA, 1),
                asset_id: "ast_alfa_1".to_string(),
                text_hash: sha(current),
                new_text_hash: sha(new_text),
            },
            job_id: String::new(),
            revision: 0,
            operations: vec!["replace_text".to_string()],
            collections: vec!["Alfa".to_string()],
            files: 1,
            preview: Some(new_text.chars().take(10).collect()),
            text: Some(new_text.to_string()),
            import_plan: None,
            delete: None,
        }
    }

    fn page_text(conn: &Connection) -> String {
        conn.query_row(
            "SELECT text_content FROM extractions WHERE asset_id = 'ast_alfa_1'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn an_approved_text_replacement_runs_once_and_never_shows_the_whole_text() {
        let f = fixture();
        let approval = f.store.request(replace_request(
            &f.conn,
            "Texto viejo",
            "Texto nuevo y largo",
        ));
        let json = serde_json::to_value(&approval).unwrap();
        assert_eq!(json["preview"], "Texto nuev");
        assert!(
            json.get("text").is_none(),
            "the full text is never serialized"
        );
        assert_eq!(
            page_text(&f.conn),
            "Texto viejo",
            "nothing changes before the answer"
        );

        let decided = f.store.decide(&f.conn, &approval.id, true).unwrap();
        assert_eq!(decided.status, ApprovalStatus::Approved);
        assert_eq!(page_text(&f.conn), "Texto nuevo y largo");
        let again = f.store.decide(&f.conn, &approval.id, true).unwrap_err();
        assert!(again.starts_with("approval_used:"), "{again}");
    }

    #[test]
    fn a_text_replacement_is_not_applied_when_the_page_changed_meanwhile() {
        let f = fixture();
        let approval = f
            .store
            .request(replace_request(&f.conn, "Texto viejo", "Propuesta"));
        // The person corrects the page in the app before answering.
        f.conn
            .execute(
                "UPDATE extractions SET text_content = 'Corrección manual' WHERE asset_id = 'ast_alfa_1'",
                [],
            )
            .unwrap();
        let decided = f.store.decide(&f.conn, &approval.id, true).unwrap();
        assert_eq!(decided.status, ApprovalStatus::Failed);
        assert!(decided.failure.unwrap().contains("cambió"));
        assert_eq!(
            page_text(&f.conn),
            "Corrección manual",
            "the person's text stays"
        );
    }

    #[test]
    fn two_different_texts_for_one_page_are_two_requests() {
        let f = fixture();
        let first = f.store.request(replace_request(&f.conn, "Viejo", "Uno"));
        let second = f.store.request(replace_request(&f.conn, "Viejo", "Dos"));
        assert_ne!(
            first.id, second.id,
            "a request is bound to the text it proposes"
        );
    }

    /// Files waiting in an enabled folder outside the archive.
    struct Inbox {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }

    fn inbox(files: &[(&str, &[u8])]) -> Inbox {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::bibliography::attachment::plain_canonical(dir.path()).unwrap();
        for (name, bytes) in files {
            std::fs::write(path.join(name), bytes).unwrap();
        }
        Inbox { _dir: dir, path }
    }

    fn import_request(f: &Fixture, inbox: &Inbox, names: &[&str]) -> ApprovalRequest {
        let folders = vec![inbox.path.clone()];
        let plan: Vec<crate::corpus::ingest::SourceFile> = names
            .iter()
            .map(|name| {
                crate::corpus::ingest::check_source(
                    &inbox.path.join(name).to_string_lossy(),
                    &folders,
                    f._dir.path(),
                    &Default::default(),
                )
                .unwrap()
            })
            .collect();
        ApprovalRequest {
            client_id: "cl_a".to_string(),
            client_name: "Claude Code".to_string(),
            action: ApprovalAction::ImportFiles {
                collection_id: ALFA.to_string(),
                files: plan.len(),
                plan_hash: format!("hash-{}", names.join(",")),
            },
            job_id: String::new(),
            revision: 0,
            operations: vec!["import_files".to_string()],
            collections: vec!["Alfa".to_string()],
            files: plan.len() as i64,
            preview: None,
            text: None,
            import_plan: Some(plan),
            delete: None,
        }
    }

    fn context(f: &Fixture, inbox: &Inbox) -> ExecContext {
        ExecContext {
            data_dir: Some(f._dir.path().to_path_buf()),
            cache_dir: Some(f._dir.path().join("cache")),
            folders: vec![inbox.path.clone()],
        }
    }

    fn items_titled(conn: &Connection, title: &str) -> i64 {
        conn.query_row(
            "SELECT count(*) FROM items WHERE title = ?1",
            [title],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn an_approved_import_imports_the_files_it_listed_and_reports_them() {
        let f = fixture();
        let inbox = inbox(&[("Carta.jpg", b"jpg"), ("Retrato.png", b"png")]);
        let approval = f
            .store
            .request(import_request(&f, &inbox, &["Carta.jpg", "Retrato.png"]));
        let json = serde_json::to_value(&approval).unwrap();
        assert_eq!(
            json["importFiles"],
            serde_json::json!([
                { "name": "Carta.jpg", "kind": "image", "sizeBytes": 3 },
                { "name": "Retrato.png", "kind": "image", "sizeBytes": 3 }
            ])
        );
        assert!(json.get("importPlan").is_none(), "paths stay in the app");
        assert_eq!(
            items_titled(&f.conn, "Carta"),
            0,
            "nothing before the answer"
        );

        let decided = f
            .store
            .decide_with(&f.conn, &approval.id, true, &context(&f, &inbox))
            .unwrap();
        assert_eq!(decided.status, ApprovalStatus::Approved);
        let result = decided.import_result.expect("a result");
        assert_eq!(result.imported.len(), 2);
        assert!(result.skipped.is_empty());
        assert_eq!(result.imported[0].name, "Carta.jpg");
        assert_eq!(items_titled(&f.conn, "Carta"), 1);
        assert_eq!(items_titled(&f.conn, "Retrato"), 1);
    }

    #[test]
    fn a_file_that_changed_or_left_the_enabled_folders_is_skipped() {
        let f = fixture();
        let inbox = inbox(&[("Carta.jpg", b"jpg"), ("Retrato.png", b"png")]);
        let approval = f
            .store
            .request(import_request(&f, &inbox, &["Carta.jpg", "Retrato.png"]));
        std::fs::write(inbox.path.join("Carta.jpg"), b"a longer jpg").unwrap();
        let decided = f
            .store
            .decide_with(&f.conn, &approval.id, true, &context(&f, &inbox))
            .unwrap();
        let result = decided.import_result.unwrap();
        assert_eq!(result.imported.len(), 1);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "Carta.jpg");
        assert!(
            result.skipped[0].reason.starts_with("changed"),
            "{:?}",
            result.skipped
        );
        assert_eq!(items_titled(&f.conn, "Carta"), 0);

        // The owner removed the folder before answering.
        let second = inbox_request_after_disable(&f);
        assert_eq!(second.0, 0);
        assert!(second.1.starts_with("not_in_folder"), "{}", second.1);
        assert!(
            !second.1.contains(std::path::MAIN_SEPARATOR),
            "a reason names the file, never its path: {}",
            second.1
        );
    }

    /// Ask for one file, then answer with no enabled folder left.
    fn inbox_request_after_disable(f: &Fixture) -> (usize, String) {
        let inbox = inbox(&[("Nota.jpg", b"jpg")]);
        let approval = f.store.request(import_request(f, &inbox, &["Nota.jpg"]));
        let decided = f
            .store
            .decide_with(
                &f.conn,
                &approval.id,
                true,
                &ExecContext {
                    data_dir: Some(f._dir.path().to_path_buf()),
                    cache_dir: Some(f._dir.path().join("cache")),
                    folders: Vec::new(),
                },
            )
            .unwrap();
        let result = decided.import_result.unwrap();
        (result.imported.len(), result.skipped[0].reason.clone())
    }

    #[test]
    fn a_rejected_import_imports_nothing() {
        let f = fixture();
        let inbox = inbox(&[("Carta.jpg", b"jpg")]);
        let approval = f.store.request(import_request(&f, &inbox, &["Carta.jpg"]));
        let decided = f
            .store
            .decide_with(&f.conn, &approval.id, false, &context(&f, &inbox))
            .unwrap();
        assert_eq!(decided.status, ApprovalStatus::Rejected);
        assert!(decided.import_result.is_none());
        assert_eq!(items_titled(&f.conn, "Carta"), 0);
    }

    fn desired_state(conn: &Connection, job: &str) -> String {
        batches::batch_snapshot(conn, job).unwrap().desired_state
    }

    #[test]
    fn a_request_waits_and_runs_nothing_until_the_person_approves() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let approval = f.store.request(start_request("cl_a", &job, revision));
        assert_eq!(approval.status, ApprovalStatus::Pending);
        assert_eq!(approval.expires_at, approval.created_at + APPROVAL_TTL_MS);
        assert_eq!(f.store.pending(), vec![approval.clone()]);
        assert_eq!(desired_state(&f.conn, &job), "pause", "nothing started yet");
        assert_eq!(
            f.notified.load(Ordering::SeqCst),
            1,
            "the app was told once"
        );

        let decided = f.store.decide(&f.conn, &approval.id, true).unwrap();
        assert_eq!(decided.status, ApprovalStatus::Approved);
        assert_eq!(desired_state(&f.conn, &job), "run");
        assert!(f.store.pending().is_empty());
    }

    #[test]
    fn an_approval_runs_once() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let approval = f.store.request(start_request("cl_a", &job, revision));
        f.store.decide(&f.conn, &approval.id, true).unwrap();
        let again = f.store.decide(&f.conn, &approval.id, true).unwrap_err();
        assert!(again.starts_with("approval_used:"), "{again}");
    }

    #[test]
    fn a_rejection_runs_nothing() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let approval = f.store.request(start_request("cl_a", &job, revision));
        let decided = f.store.decide(&f.conn, &approval.id, false).unwrap();
        assert_eq!(decided.status, ApprovalStatus::Rejected);
        assert_eq!(desired_state(&f.conn, &job), "pause");
    }

    #[test]
    fn an_unanswered_request_expires_after_ten_minutes_and_cannot_be_approved() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let approval = f.store.request(start_request("cl_a", &job, revision));
        f.clock.fetch_add(APPROVAL_TTL_MS - 1, Ordering::SeqCst);
        assert_eq!(f.store.pending().len(), 1, "still waiting one ms before");
        f.clock.fetch_add(1, Ordering::SeqCst);
        assert!(f.store.pending().is_empty());
        let late = f.store.decide(&f.conn, &approval.id, true).unwrap_err();
        assert!(late.starts_with("approval_expired:"), "{late}");
        assert_eq!(desired_state(&f.conn, &job), "pause");
        assert_eq!(
            f.store.get_for_client("cl_a", &approval.id).unwrap().status,
            ApprovalStatus::Expired
        );
    }

    #[test]
    fn a_job_that_changed_after_the_request_is_not_started() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let approval = f.store.request(start_request("cl_a", &job, revision));
        // The person changes the job's priority in the tab: a new revision.
        batches::set_priority(
            &f.conn,
            &batches::SetPriorityRequest {
                batch_id: job.clone(),
                priority: 1,
                expected_revision: None,
            },
        )
        .unwrap();
        let decided = f.store.decide(&f.conn, &approval.id, true).unwrap();
        assert_eq!(decided.status, ApprovalStatus::Failed);
        assert!(decided.failure.unwrap().contains("cambió"));
        assert_eq!(desired_state(&f.conn, &job), "pause");
    }

    #[test]
    fn the_same_request_returns_the_pending_approval_instead_of_a_second_one() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let first = f.store.request(start_request("cl_a", &job, revision));
        let again = f.store.request(start_request("cl_a", &job, revision));
        assert_eq!(first.id, again.id);
        assert_eq!(f.store.pending().len(), 1);
        assert_eq!(
            f.notified.load(Ordering::SeqCst),
            1,
            "a repeated request does not bring the window up again"
        );
        let other_client = f.store.request(start_request("cl_b", &job, revision));
        assert_ne!(
            other_client.id, first.id,
            "another assistant asks on its own"
        );
    }

    #[test]
    fn a_client_only_reads_its_own_approvals() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let approval = f.store.request(start_request("cl_a", &job, revision));
        assert!(f.store.get_for_client("cl_a", &approval.id).is_some());
        assert!(f.store.get_for_client("cl_b", &approval.id).is_none());
    }

    #[test]
    fn racing_answers_run_the_approval_once() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let approval = f.store.request(start_request("cl_a", &job, revision));
        let outcomes: Vec<Result<Approval, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let conn = batches::open_ready(&f.db_path).unwrap();
                        f.store.decide(&conn, &approval.id, true)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        let snapshot = batches::batch_snapshot(&f.conn, &job).unwrap();
        assert_eq!(snapshot.revision, revision + 1, "started exactly once");
        assert_eq!(snapshot.desired_state, "run");
    }

    #[test]
    fn two_assistants_asking_the_same_start_run_it_once() {
        let f = fixture();
        let (job, revision) = ready_job(&f.conn);
        let first = f.store.request(start_request("cl_a", &job, revision));
        let second = f.store.request(start_request("cl_b", &job, revision));
        assert_eq!(
            f.store.decide(&f.conn, &first.id, true).unwrap().status,
            ApprovalStatus::Approved
        );
        assert_eq!(
            f.store.decide(&f.conn, &second.id, true).unwrap().status,
            ApprovalStatus::Failed,
            "the job moved on: the second approval runs nothing"
        );
        assert_eq!(
            batches::batch_snapshot(&f.conn, &job).unwrap().revision,
            revision + 1
        );
    }

    #[test]
    fn an_unknown_approval_is_refused() {
        let f = fixture();
        let error = f.store.decide(&f.conn, "apr_missing", true).unwrap_err();
        assert_eq!(error, "unknown_approval: no request apr_missing");
    }
}
