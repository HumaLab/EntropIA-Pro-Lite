//! One sync run of a library: reconcile uncertain requests, pull, release
//! `waiting_parent` rows, push (plan sections 4 and 5).
//!
//! Pull and push of a library share one lock, so a row with a request in
//! flight is never merged. The first run of a library in this process marks
//! every request recorded as sent as uncertain: none can still be live.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use rusqlite::Connection;
use serde::Serialize;

use super::api::{AnnotationApi, ApiError, CreateResult, ServerItem, WriteResult};
use super::machine::{
    apply_outcome, begin_send, mark_outstanding_uncertain, merge_remote, next_request, reconcile,
    release_waiting_parent, Next, Observed, Outcome, ParentStatus, RemoteChange, Request, RowAction,
};
use super::model::{AnnotationRow, Sent, SyncState};
use super::repo::{self, LibrarySyncState, Scope};

/// Zotero takes at most 50 objects per write and per `itemKey` read.
pub const BATCH: usize = 50;

/// Pull runs that see the library change mid-download restart this often.
const PULL_ATTEMPTS: usize = 2;

/// Whether the parent attachment is still in the local catalog, which
/// mirrors Zotero Desktop.
pub trait ParentCatalog {
    fn has_attachment(&self, parent_key: &str) -> bool;
}

/// The local catalog of one library.
pub struct CatalogParents<'a> {
    pub conn: &'a Connection,
    pub library_row_id: &'a str,
}

impl ParentCatalog for CatalogParents<'_> {
    fn has_attachment(&self, parent_key: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM zotero_attachments a JOIN bibliographic_items i ON i.id = a.item_id \
                 WHERE i.library_id = ?1 AND a.attachment_key = ?2 LIMIT 1",
                rusqlite::params![self.library_row_id, parent_key],
                |_| Ok(()),
            )
            .is_ok()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub pulled: usize,
    pub removed: usize,
    pub sent: usize,
    /// `credentials`, `backoff`, `retry_after` or `server`, when the run
    /// stopped early.
    pub paused: Option<String>,
    pub uncertain: usize,
}

/// The lock of one library. Library rows are few and live as long as the
/// app, so the map is never pruned.
fn library_lock(library_row_id: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(library_row_id.to_string()).or_default().clone()
}

/// Libraries whose outstanding requests were already marked uncertain in this
/// process.
fn first_run_in_process(scope: &Scope) -> bool {
    static RECOVERED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mut seen = RECOVERED.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    seen.insert(format!("{}\u{1f}{}", scope.library_row_id, scope.web_user_id))
}

/// Runs one full sync of a library under its lock.
pub fn sync_library(
    conn: &Connection,
    scope: &Scope,
    api: &dyn AnnotationApi,
    parents: &dyn ParentCatalog,
    now_ms: i64,
) -> Result<SyncReport, String> {
    let lock = library_lock(&scope.library_row_id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if first_run_in_process(scope) {
        recover_after_start(conn, scope, now_ms)?;
    }
    let mut run = Run { conn, scope, api, parents, now_ms, report: SyncReport::default() };
    run.go()?;
    Ok(run.report)
}

/// Marks every request recorded as sent as uncertain (plan section 5,
/// Uncertain outcome). Called once per library per process.
pub fn recover_after_start(conn: &Connection, scope: &Scope, now_ms: i64) -> Result<usize, String> {
    let mut marked = 0;
    for mut row in repo::outstanding(conn, scope)? {
        if mark_outstanding_uncertain(&mut row) {
            repo::save(conn, scope, &row, now_ms)?;
            marked += 1;
        }
    }
    Ok(marked)
}

struct Run<'a> {
    conn: &'a Connection,
    scope: &'a Scope,
    api: &'a dyn AnnotationApi,
    parents: &'a dyn ParentCatalog,
    now_ms: i64,
    report: SyncReport,
}

/// Stops the run: the library pauses and nothing else is sent.
struct Stop;

impl Run<'_> {
    fn go(&mut self) -> Result<(), String> {
        let state = repo::sync_state(self.conn, self.scope)?;
        if state.paused_until.is_some_and(|until| until > self.now_ms) {
            self.report.paused = state.pause_reason.clone();
            return Ok(());
        }
        if state.paused_until.is_some() {
            self.set_pause(None, None)?;
        }
        if self.reconcile_uncertain()?.is_err() || self.pull()?.is_err() {
            return Ok(());
        }
        for mut row in repo::in_state(self.conn, self.scope, SyncState::WaitingParent)? {
            if release_waiting_parent(&mut row) {
                repo::save(self.conn, self.scope, &row, self.now_ms)?;
            }
        }
        let _ = self.push()?;
        self.report.uncertain = repo::outstanding(self.conn, self.scope)?.len();
        Ok(())
    }

    fn set_pause(&self, until: Option<i64>, reason: Option<&str>) -> Result<(), String> {
        let mut state = repo::sync_state(self.conn, self.scope)?;
        state.paused_until = until;
        state.pause_reason = reason.map(str::to_string);
        repo::set_sync_state(self.conn, self.scope, &state, self.now_ms)
    }

    /// Turns a failed call into a pause. Returns `Stop` so callers bail out.
    fn pause_for(&mut self, error: &ApiError) -> Result<Stop, String> {
        let (until, reason) = match error {
            ApiError::Credentials => (self.now_ms + 365 * 24 * 3_600_000, "credentials"),
            ApiError::RetryAfter { seconds } => (self.now_ms + *seconds as i64 * 1000, "retry_after"),
            ApiError::Server { .. } => (self.now_ms + 60_000, "server"),
            ApiError::Unreachable { .. } => (self.now_ms + 60_000, "unreachable"),
        };
        self.set_pause(Some(until), Some(reason))?;
        self.report.paused = Some(reason.to_string());
        Ok(Stop)
    }

    /// Honors a `Backoff` header seen on the last answer.
    fn check_backoff(&mut self) -> Result<Result<(), Stop>, String> {
        if let Some(seconds) = self.api.take_backoff() {
            self.set_pause(Some(self.now_ms + seconds as i64 * 1000), Some("backoff"))?;
            self.report.paused = Some("backoff".into());
            return Ok(Err(Stop));
        }
        Ok(Ok(()))
    }

    fn save(&mut self, key_before: &str, row: &AnnotationRow, action: &RowAction) -> Result<(), String> {
        if *action == RowAction::Remove {
            self.report.removed += 1;
        }
        repo::apply(self.conn, self.scope, key_before, row, action, self.now_ms)
    }

    // --- Reconciliation -----------------------------------------------------

    fn reconcile_uncertain(&mut self) -> Result<Result<(), Stop>, String> {
        for mut row in repo::outstanding(self.conn, self.scope)? {
            if !row.sent_uncertain {
                continue;
            }
            let observed = match self.api.item(&row.annotation_key) {
                Ok(Some(item)) => Observed::Found { version: item.version, data: item.data },
                Ok(None) => Observed::NotFound,
                Err(error) => return Ok(Err(self.pause_for(&error)?)),
            };
            let key_before = row.annotation_key.clone();
            let action = reconcile(&mut row, observed);
            self.save(&key_before, &row, &action)?;
        }
        Ok(Ok(()))
    }

    // --- Pull (section 4) ---------------------------------------------------

    fn pull(&mut self) -> Result<Result<(), Stop>, String> {
        let since = repo::sync_state(self.conn, self.scope)?.web_library_version;
        for attempt in 0..PULL_ATTEMPTS {
            match self.fetch_changes(since) {
                Ok(Some(changes)) => return self.commit_pull(changes).map(Ok),
                Ok(None) if attempt + 1 < PULL_ATTEMPTS => continue,
                Ok(None) => return Ok(Ok(())),
                Err(error) => return Ok(Err(self.pause_for(&error)?)),
            }
        }
        Ok(Ok(()))
    }

    /// Reads every change since `since`. `None` when the library moved while
    /// reading (a `Last-Modified-Version` mismatch).
    fn fetch_changes(&mut self, since: i64) -> Result<Option<PulledChanges>, ApiError> {
        let versions = self.api.versions_since(since)?;
        let library_version = versions.library_version;
        let keys: Vec<String> = versions.value.into_keys().collect();
        let mut items = Vec::new();
        for chunk in keys.chunks(BATCH) {
            let batch = self.api.items(chunk)?;
            if batch.library_version != library_version {
                return Ok(None);
            }
            items.extend(batch.value);
        }
        let deleted = self.api.deleted_since(since)?;
        if deleted.library_version != library_version {
            return Ok(None);
        }
        Ok(Some(PulledChanges { library_version, items, deleted: deleted.value }))
    }

    /// Applies a complete pull in one transaction, cursor included.
    fn commit_pull(&mut self, changes: PulledChanges) -> Result<(), String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|error| format!("annotation pull failed: {error}"))?;
        for item in changes.items {
            let Some(key) = item.data.get("key").and_then(|key| key.as_str()).map(str::to_string) else {
                continue;
            };
            if item.data.get("itemType").and_then(|kind| kind.as_str()) != Some("annotation") {
                continue;
            }
            self.merge_one(&key, RemoteChange::Item { version: item.version, data: item.data })?;
        }
        for key in changes.deleted {
            self.merge_one(&key, RemoteChange::PermanentlyDeleted)?;
        }
        let mut state: LibrarySyncState = repo::sync_state(self.conn, self.scope)?;
        state.web_library_version = changes.library_version;
        repo::set_sync_state(self.conn, self.scope, &state, self.now_ms)?;
        tx.commit().map_err(|error| format!("annotation pull failed: {error}"))
    }

    fn merge_one(&mut self, key: &str, change: RemoteChange) -> Result<(), String> {
        match repo::load(self.conn, self.scope, key)? {
            None => {
                if let RemoteChange::Item { version, data } = change {
                    let row = AnnotationRow::from_server(key, version, data);
                    self.report.pulled += 1;
                    repo::save(self.conn, self.scope, &row, self.now_ms)?;
                }
                Ok(())
            }
            Some(mut row) => {
                // Reconciled before the pull; anything still out stays as it is.
                if row.sent_gen.is_some() {
                    return Ok(());
                }
                let action = merge_remote(&mut row, change).map_err(|error| error.to_string())?;
                self.report.pulled += 1;
                self.save(key, &row, &action)
            }
        }
    }

    // --- Push (section 5) ---------------------------------------------------

    fn push(&mut self) -> Result<Result<(), Stop>, String> {
        let mut creates = Vec::new();
        for mut row in repo::in_state(self.conn, self.scope, SyncState::Pending)? {
            let key_before = row.annotation_key.clone();
            let Some(request) = next_request(&mut row) else {
                if row.sync_state != SyncState::Pending {
                    self.save(&key_before, &row, &RowAction::Save)?;
                }
                continue;
            };
            match request {
                Request::Create { .. } => creates.push((row, request)),
                other => {
                    if let Err(stop) = self.send_one(row, other)? {
                        return Ok(Err(stop));
                    }
                }
            }
        }
        for batch in creates.chunks(BATCH) {
            if let Err(stop) = self.send_creates(batch.to_vec())? {
                return Ok(Err(stop));
            }
        }
        Ok(Ok(()))
    }

    fn send_creates(&mut self, batch: Vec<(AnnotationRow, Request)>) -> Result<Result<(), Stop>, String> {
        let mut rows = Vec::new();
        let mut bodies = Vec::new();
        for (mut row, request) in batch {
            begin_send(&mut row, &request);
            repo::save(self.conn, self.scope, &row, self.now_ms)?;
            if let Request::Create { data, .. } = request {
                bodies.push(data);
            }
            rows.push(row);
        }
        let results = match self.api.create(&bodies) {
            Ok(results) => results,
            Err(error) => {
                for mut row in rows {
                    let key = row.annotation_key.clone();
                    let action = apply_outcome(&mut row, outcome_of_error(&error)).into_action();
                    self.save(&key, &row, &action)?;
                }
                return Ok(Err(self.pause_for(&error)?));
            }
        };
        for (mut row, result) in rows.into_iter().zip(results) {
            let key = row.annotation_key.clone();
            let outcome = match result {
                CreateResult::Created { version } => Outcome::Applied { version },
                CreateResult::Exists => match self.api.item(&key) {
                    Ok(Some(item)) => Outcome::CreateExists { version: item.version, data: item.data },
                    Ok(None) => Outcome::Uncertain,
                    Err(_) => Outcome::Uncertain,
                },
                CreateResult::Failed { message, parent_missing: true, .. } => {
                    Outcome::ParentMissing { parent: self.parent_status(&row), message }
                }
                CreateResult::Failed { message, .. } => Outcome::Rejected { message },
            };
            self.report.sent += 1;
            let action = apply_outcome(&mut row, outcome).into_action();
            self.save(&key, &row, &action)?;
        }
        self.check_backoff()
    }

    fn parent_status(&self, row: &AnnotationRow) -> ParentStatus {
        match self.api.item(&row.parent_key) {
            Ok(Some(_)) => ParentStatus::OnServer,
            _ if self.parents.has_attachment(&row.parent_key) => ParentStatus::PendingInZotero,
            Ok(None) => ParentStatus::Deleted,
            Err(_) => ParentStatus::PendingInZotero,
        }
    }

    fn send_one(&mut self, mut row: AnnotationRow, request: Request) -> Result<Result<(), Stop>, String> {
        let mut retries_left = 1;
        let mut request = request;
        loop {
            let key = row.annotation_key.clone();
            begin_send(&mut row, &request);
            repo::save(self.conn, self.scope, &row, self.now_ms)?;
            let answer = match &request {
                Request::Update { key, version, fields } => self.api.patch(key, *version, fields),
                Request::Delete { key, version } => self.api.delete(key, *version),
                Request::Create { .. } => unreachable!("creates go in batches"),
            };
            let outcome = match answer {
                Ok(WriteResult::Done { version }) => {
                    if matches!(row.sent_json, Some(Sent::Delete)) {
                        Outcome::Gone
                    } else {
                        Outcome::Applied { version }
                    }
                }
                Ok(WriteResult::NotFound) if matches!(row.sent_json, Some(Sent::Delete)) => Outcome::Gone,
                Ok(WriteResult::NotFound) => Outcome::Stale { remote: RemoteChange::PermanentlyDeleted },
                Ok(WriteResult::Stale) => match self.api.item(&key) {
                    Ok(Some(ServerItem { version, data })) => Outcome::Stale { remote: RemoteChange::Item { version, data } },
                    Ok(None) => Outcome::Stale { remote: RemoteChange::PermanentlyDeleted },
                    Err(error) => {
                        // The write was refused; only the follow-up read failed.
                        let action = apply_outcome(&mut row, Outcome::Refused).into_action();
                        self.save(&key, &row, &action)?;
                        return Ok(Err(self.pause_for(&error)?));
                    }
                },
                Ok(WriteResult::Rejected { message, .. }) => Outcome::Rejected { message },
                Err(error) => {
                    let action = apply_outcome(&mut row, outcome_of_error(&error)).into_action();
                    self.save(&key, &row, &action)?;
                    return Ok(Err(self.pause_for(&error)?));
                }
            };
            self.report.sent += 1;
            match apply_outcome(&mut row, outcome) {
                Next::Done(action) => {
                    self.save(&key, &row, &action)?;
                    break;
                }
                Next::RetryOnce(action) => {
                    self.save(&key, &row, &action)?;
                    if retries_left == 0 {
                        break;
                    }
                    retries_left -= 1;
                    match next_request(&mut row) {
                        Some(next) => request = next,
                        None => {
                            self.save(&key, &row, &RowAction::Save)?;
                            break;
                        }
                    }
                }
            }
        }
        self.check_backoff()
    }
}

/// The machine outcome of a call that produced no usable answer.
fn outcome_of_error(error: &ApiError) -> Outcome {
    match error {
        ApiError::Unreachable { after_send: true } | ApiError::Server { .. } => Outcome::Uncertain,
        ApiError::Unreachable { after_send: false } | ApiError::Credentials | ApiError::RetryAfter { .. } => {
            Outcome::Refused
        }
    }
}

struct PulledChanges {
    library_version: i64,
    items: Vec<ServerItem>,
    deleted: Vec<String>,
}

/// Rows of one parent attachment for the panel, newest state first by page.
pub fn rows_for_parent(conn: &Connection, scope: &Scope, parent_key: &str) -> Result<Vec<AnnotationRow>, String> {
    repo::by_parent(conn, scope, parent_key)
}
