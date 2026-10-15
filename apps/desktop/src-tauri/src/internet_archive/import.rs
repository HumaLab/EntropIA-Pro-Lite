//! Task 23: hand-off of verified files to the frontend importer and
//! confirmation.
//!
//! The runner (task 22) leaves verified PDFs staged on disk; the actual
//! import into the Corpus (items + parent/child page assets) happens in the
//! frontend importer (`collection-import.ts`), which writes `items.metadata`
//! BEFORE `finalizeImportedItem` splits the pages. These commands bridge the
//! two sides:
//!
//! - [`ia_begin_file_import`]: the file must be `Verified` with its staging
//!   present. SQLite decides whether the import already exists: `Complete`
//!   (parent + exact pages `1..=page_count`, every file on disk) hands the
//!   frontend the existing item id, `Partial` tells it to discard the remnant
//!   and begin again, `None` hands over the staging payload for a fresh
//!   import. Either way the file moves to `Importing`, except `Partial`.
//! - [`ia_confirm_file_imported`]: validates the item the frontend reports
//!   (exists, in the job's target collection, provenance matches, actually
//!   `Complete`) and settles the file to `Imported`, or sends it back to
//!   `Verified` with a machine-coded error.
//! - [`ia_fail_file_import`]: the frontend reports a failed import; the file
//!   goes to `Failed` and its staging is dropped.
//! - [`ia_list_reconcile`] / [`ia_finish_reconcile`]: durable recovery
//!   leftovers derived on demand from `job.json` (a `Cancelled` file that
//!   still has a `staging_name`), classified the same way.
//!
//! Locking mirrors the runner: every job write is load → modify → save under
//! the single `jobs` writer lock via [`update`]; the SQLite `ui_conn` mutex
//! is taken only for short synchronous reads, never across an `.await`
//! (these commands are sync). Error strings are `"<code>: <message>"` with
//! machine codes and no paths.

use std::path::Path;
use std::sync::Mutex;

use internet_archive::job::{FileState, Job, JobFile};
use internet_archive::prepare::ProvenanceRecord;
use internet_archive::store::{is_valid_job_id, JobStore};
use internet_archive::PROVENANCE_KEY;
use serde::Serialize;
use tauri::State;

use super::commands::{
    emit_job_updated, job_load_error, now_secs, target_collection_id, verified_file_entry,
    VerifiedFileDto,
};
use super::runner::update;
use super::IaState;
use crate::db::state::AppDbState;
use crate::path_utils::resolve_asset_path;

/// What the frontend already has in the target collection for one
/// `(identifier, file_name, sha256)` triple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Existing {
    /// Nothing imported for this triple (a different `sha256_local` is
    /// another version and does not count).
    None,
    /// Exactly one parent asset (`type = 'pdf'`, no parent) whose children
    /// carry page numbers exactly `1..=page_count`, every file on disk.
    Complete { item_id: String },
    /// A candidate exists but is not whole (missing/extra/duplicate pages,
    /// missing parent, or a file gone from disk).
    Partial { item_id: String },
}

/// Classify what the target collection already holds for one verified file.
///
/// Candidates are items in `collection_id` whose provenance (via
/// [`ProvenanceRecord::from_row`]) carries the same `identifier`,
/// `file_name` AND `sha256_local`. Any `Complete` wins (lowest id first for
/// determinism), else the lowest-id `Partial`, else [`Existing::None`].
pub(crate) fn existing_import(
    conn: &rusqlite::Connection,
    data_dir: &Path,
    collection_id: &str,
    identifier: &str,
    file_name: &str,
    sha256: &str,
    page_count: u32,
) -> rusqlite::Result<Existing> {
    // Candidates: items in the target collection whose provenance carries
    // this identifier. `json_valid` guards the extract so one malformed row
    // cannot fail the load (see provenance_db.rs); `from_row` drops whatever
    // is left unusable, and the file_name/sha256 filter runs in memory.
    let sql = format!(
        "SELECT id, metadata FROM items \
         WHERE collection_id = ?1 \
         AND CASE WHEN json_valid(metadata) \
         THEN json_extract(metadata, '$.{PROVENANCE_KEY}.identifier') \
         ELSE NULL END = ?2 \
         ORDER BY id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![collection_id, identifier], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut partial: Option<String> = None;
    for row in rows {
        let (item_id, metadata) = row?;
        let Some(metadata) = metadata.as_deref() else {
            continue;
        };
        let Some(record) = ProvenanceRecord::from_row(&item_id, collection_id, metadata) else {
            continue;
        };
        // A different `sha256_local` is another version, not this import.
        if record.file_name != file_name || record.sha256_local.as_deref() != Some(sha256) {
            continue;
        }
        // Lowest id first: the first `Complete` wins outright.
        if is_complete(conn, data_dir, &item_id, page_count)? {
            return Ok(Existing::Complete { item_id });
        }
        if partial.is_none() {
            partial = Some(item_id);
        }
    }
    Ok(match partial {
        Some(item_id) => Existing::Partial { item_id },
        None => Existing::None,
    })
}

/// True when `item_id` holds exactly one parent PDF asset (no parent of its
/// own) whose children carry page numbers exactly `1..=page_count` — no
/// duplicates, no extras — with every one of those files present on disk
/// after [`resolve_asset_path`]. Anything else is a remnant, not an import.
fn is_complete(
    conn: &rusqlite::Connection,
    data_dir: &Path,
    item_id: &str,
    page_count: u32,
) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT id, path FROM assets \
         WHERE item_id = ?1 AND type = 'pdf' AND parent_asset_id IS NULL",
    )?;
    let parents: Vec<(String, String)> = stmt
        .query_map([item_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    if parents.len() != 1 {
        return Ok(false);
    }
    let (parent_id, parent_path) = &parents[0];
    if !resolve_asset_path(parent_path, data_dir).is_file() {
        return Ok(false);
    }
    let mut stmt =
        conn.prepare("SELECT page_number, path FROM assets WHERE parent_asset_id = ?1")?;
    let children: Vec<(Option<i64>, String)> = stmt
        .query_map([parent_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    if children.len() != page_count as usize {
        return Ok(false);
    }
    let mut pages = Vec::with_capacity(children.len());
    for (page, path) in &children {
        let Some(page) = page else {
            return Ok(false);
        };
        pages.push(*page);
        if !resolve_asset_path(path, data_dir).is_file() {
            return Ok(false);
        }
    }
    pages.sort_unstable();
    let expected: Vec<i64> = (1..=i64::from(page_count)).collect();
    Ok(pages == expected)
}

/// What the frontend must do with a verified file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BeginAction {
    /// Fresh import: `file` carries the staging payload.
    Import,
    /// The document is already whole: `item_id` is the existing item.
    ConfirmExisting,
    /// A remnant sits in the collection: `item_id` is the partial item the
    /// frontend removes before beginning again. The file stays `Verified`.
    DiscardPartial,
}

/// The begin handoff, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeginDto {
    pub action: BeginAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<VerifiedFileDto>,
}

/// True when the file's staging name resolves to a file on disk.
fn staging_present(store: &JobStore, job_id: &str, file: &JobFile) -> bool {
    match &file.staging_name {
        Some(name) => store
            .staging_path(job_id, name)
            .is_ok_and(|path| path.is_file()),
        None => false,
    }
}

fn unknown_index(index: usize) -> String {
    format!("invalid_request: unknown file index {index}")
}

fn not_verified(index: usize) -> String {
    format!("invalid_state: file {index} is not verified")
}

fn not_importing(index: usize) -> String {
    format!("invalid_state: file {index} is not being imported")
}

/// Begin the frontend import of one file: `Verified` (+ staging present) →
/// `Importing`, with the action SQLite dictates. `Partial` leaves the file
/// `Verified` and neither saves nor reports. Every other path saves and
/// calls `on_update` (the production wrapper emits `ia://job-updated`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn begin_file_import(
    store: &JobStore,
    jobs: &Mutex<()>,
    ui: &Mutex<rusqlite::Connection>,
    data_dir: &Path,
    job_id: &str,
    index: usize,
    now: u64,
    on_update: &(dyn Fn(&Job) + Send + Sync),
) -> Result<BeginDto, String> {
    let job = store.load(job_id).map_err(job_load_error)?;
    let file = job.files.get(index).ok_or_else(|| unknown_index(index))?;
    if file.state != FileState::Verified {
        return Err(not_verified(index));
    }
    if !staging_present(store, job_id, file) {
        return Err(format!(
            "staging_missing: staging file for file {index} is gone"
        ));
    }
    let collection_id = target_collection_id(&job)
        .ok_or_else(|| "invalid_state: import job has no collection target".to_string())?;
    let sha256 = file
        .sha256
        .clone()
        .filter(|hash| !hash.trim().is_empty())
        .ok_or_else(|| "io: verified file has no hash".to_string())?;
    let page_count = file
        .page_count
        .ok_or_else(|| "io: verified file has no page count".to_string())?;
    let identifier = file.identifier.clone();
    let file_name = file.file_name.clone();
    // Short synchronous read under the SQLite lock, dropped before the job
    // write below; never across an `.await` (this command is sync).
    let existing = {
        let conn = ui
            .lock()
            .map_err(|_| "io: local storage error".to_string())?;
        existing_import(
            &conn,
            data_dir,
            &collection_id,
            &identifier,
            &file_name,
            &sha256,
            page_count,
        )
        .map_err(|error| format!("db: {error}"))?
    };
    // The file may have moved since the classification read; the closure
    // re-checks `Verified` under the writer lock.
    let move_to_importing = |job: &mut Job| -> Result<(), String> {
        match job.files.get(index) {
            Some(file) if file.state == FileState::Verified => {}
            _ => return Err(not_verified(index)),
        }
        job.set_file_state(index, FileState::Importing, now)
            .map_err(|_| not_verified(index))?;
        Ok(())
    };
    match existing {
        Existing::Complete { item_id } => {
            let (_, fresh) = update(store, jobs, job_id, move_to_importing)?;
            on_update(&fresh);
            Ok(BeginDto {
                action: BeginAction::ConfirmExisting,
                item_id: Some(item_id),
                file: None,
            })
        }
        // The frontend removes the remnant and begins again; nothing to
        // save, nothing to report.
        Existing::Partial { item_id } => Ok(BeginDto {
            action: BeginAction::DiscardPartial,
            item_id: Some(item_id),
            file: None,
        }),
        Existing::None => {
            // Same payload an `ia_list_verified_files` entry carries: one
            // builder for both, so the handoff cannot drift from the list.
            let entry = verified_file_entry(store, &job, index, file, now)?;
            let (_, fresh) = update(store, jobs, job_id, move_to_importing)?;
            on_update(&fresh);
            Ok(BeginDto {
                action: BeginAction::Import,
                item_id: None,
                file: Some(entry),
            })
        }
    }
}

/// A confirm validation failure (the file goes back to `Verified`) versus
/// an infrastructure failure (the job is left untouched).
enum ConfirmError {
    Validation(String),
    Infra(String),
}

/// Validate the item the frontend reports for one `Importing` file: it
/// exists, sits in the job's target collection, carries provenance matching
/// identifier/file_name/sha256, and is complete on its own ([`is_complete`]).
#[allow(clippy::too_many_arguments)]
fn validate_confirm(
    conn: &rusqlite::Connection,
    data_dir: &Path,
    collection_id: &str,
    identifier: &str,
    file_name: &str,
    sha256: Option<&str>,
    page_count: Option<u32>,
    item_id: &str,
) -> Result<(), ConfirmError> {
    use rusqlite::OptionalExtension as _;
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT collection_id, metadata FROM items WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| ConfirmError::Infra(format!("db: {error}")))?;
    let Some((item_collection, metadata)) = row else {
        return Err(ConfirmError::Validation(
            "item_not_found: imported item not found".to_string(),
        ));
    };
    if item_collection != collection_id {
        return Err(ConfirmError::Validation(
            "wrong_collection: imported item is in another collection".to_string(),
        ));
    }
    // Without the recorded fingerprint completeness cannot be established;
    // back to `Verified` like any other validation failure.
    let (Some(sha256), Some(page_count)) = (sha256, page_count) else {
        return Err(ConfirmError::Validation(
            "incomplete_import: verified file lost its fingerprint".to_string(),
        ));
    };
    let provenance_ok = metadata
        .as_deref()
        .and_then(|metadata| ProvenanceRecord::from_row(item_id, &item_collection, metadata))
        .is_some_and(|record| {
            record.identifier == identifier
                && record.file_name == file_name
                && record.sha256_local.as_deref() == Some(sha256)
        });
    if !provenance_ok {
        return Err(ConfirmError::Validation(
            "provenance_mismatch: imported item carries different provenance".to_string(),
        ));
    }
    // Completeness of this exact item: another complete copy with a lower
    // id must not make a correct confirmation fail.
    if is_complete(conn, data_dir, item_id, page_count)
        .map_err(|error| ConfirmError::Infra(format!("db: {error}")))?
    {
        Ok(())
    } else {
        Err(ConfirmError::Validation(
            "incomplete_import: imported item is not complete".to_string(),
        ))
    }
}

/// Confirm the item the frontend imported for one `Importing` file.
/// Success settles the file to `Imported` (staging deleted, job settled);
/// any validation failure sends the file back to `Verified` (staging kept),
/// saves, reports, and returns the machine-coded error.
#[allow(clippy::too_many_arguments)]
pub(crate) fn confirm_file_imported(
    store: &JobStore,
    jobs: &Mutex<()>,
    ui: &Mutex<rusqlite::Connection>,
    data_dir: &Path,
    job_id: &str,
    index: usize,
    item_id: &str,
    now: u64,
    on_update: &(dyn Fn(&Job) + Send + Sync),
) -> Result<(), String> {
    let job = store.load(job_id).map_err(job_load_error)?;
    let file = job.files.get(index).ok_or_else(|| unknown_index(index))?;
    if file.state != FileState::Importing {
        return Err(not_importing(index));
    }
    let collection_id = target_collection_id(&job)
        .ok_or_else(|| "invalid_state: import job has no collection target".to_string())?;
    let identifier = file.identifier.clone();
    let file_name = file.file_name.clone();
    let sha256 = file.sha256.clone();
    let page_count = file.page_count;
    let outcome = {
        let conn = ui
            .lock()
            .map_err(|_| "io: local storage error".to_string())?;
        validate_confirm(
            &conn,
            data_dir,
            &collection_id,
            &identifier,
            &file_name,
            sha256.as_deref().filter(|hash| !hash.trim().is_empty()),
            page_count,
            item_id,
        )
    };
    match outcome {
        Ok(()) => {
            let (_, fresh) = update(store, jobs, job_id, |job| {
                match job.files.get(index) {
                    Some(file) if file.state == FileState::Importing => {}
                    _ => return Err(not_importing(index)),
                }
                let staging = job.files[index].staging_name.take();
                job.files[index].item_id = Some(item_id.to_string());
                job.files[index].error = None;
                job.set_file_state(index, FileState::Imported, now)
                    .map_err(|_| not_importing(index))?;
                job.settle(now);
                // Best effort like the runner's staging cleanup: the import
                // already landed, a leftover must not fail the confirm.
                if let Some(name) = staging {
                    let _ = store.delete_staging(job_id, &name);
                }
                Ok(())
            })?;
            on_update(&fresh);
            Ok(())
        }
        Err(ConfirmError::Validation(code)) => {
            // Staging is kept; the frontend begins again from `Verified`.
            // `update` only saves when the closure succeeds, so the revert
            // saves first and the code returns after.
            let (_, fresh) = update(store, jobs, job_id, |job| {
                if matches!(job.files.get(index), Some(file) if file.state == FileState::Importing)
                {
                    job.set_file_state(index, FileState::Verified, now)
                        .map_err(|_| not_importing(index))?;
                }
                Ok(())
            })?;
            on_update(&fresh);
            Err(code)
        }
        Err(ConfirmError::Infra(error)) => Err(error),
    }
}

/// Report a failed frontend import: the `Importing` file goes to `Failed`
/// with error `import_failed`, its staging is dropped, the job settles.
pub(crate) fn fail_file_import(
    store: &JobStore,
    jobs: &Mutex<()>,
    job_id: &str,
    index: usize,
    now: u64,
    on_update: &(dyn Fn(&Job) + Send + Sync),
) -> Result<(), String> {
    let (_, fresh) = update(store, jobs, job_id, |job| {
        match job.files.get(index) {
            Some(file) if file.state == FileState::Importing => {}
            Some(_) => return Err(not_importing(index)),
            None => return Err(unknown_index(index)),
        }
        let staging = job.files[index].staging_name.take();
        job.files[index].error = Some("import_failed".to_string());
        job.set_file_state(index, FileState::Failed, now)
            .map_err(|_| not_importing(index))?;
        job.settle(now);
        if let Some(name) = staging {
            let _ = store.delete_staging(job_id, &name);
        }
        Ok(())
    })?;
    on_update(&fresh);
    Ok(())
}

/// What the frontend must do with a recovery leftover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReconcileAction {
    /// The import actually completed: keep the item, staging may go.
    Keep,
    /// A remnant sits in the collection: `item_id` is the partial item the
    /// frontend removes before the entry is released.
    Discard,
    /// Nothing imported: the entry may be released straight away.
    None,
}

/// One recovery leftover with its classification, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReconcileDto {
    pub job_id: String,
    pub index: usize,
    pub identifier: String,
    pub file_name: String,
    pub action: ReconcileAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

/// One durable recovery leftover: a `Cancelled` file that still has a
/// `staging_name`. Only the restart path for an `Importing` file of a
/// `Cancelling` job keeps it; every live cancel or failure path that
/// deletes staging also clears the name, so anything listed here still has
/// bytes to reconcile.
#[derive(Debug, Clone)]
struct DurableEntry {
    job_id: String,
    index: usize,
    identifier: String,
    file_name: String,
    collection_id: Option<String>,
    sha256: Option<String>,
    page_count: Option<u32>,
    staging_name: String,
}

/// Scan the store for durable entries, under the `jobs` lock with only
/// synchronous file I/O. Missing roots are empty; unloadable jobs are
/// skipped; ids sort for a stable list.
fn durable_entries(store: &JobStore, jobs: &Mutex<()>) -> Result<Vec<DurableEntry>, String> {
    let _guard = jobs
        .lock()
        .map_err(|_| "io: local storage error".to_string())?;
    let entries = match std::fs::read_dir(store.root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("io: local storage error".to_string()),
    };
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_valid_job_id(&name) {
            ids.push(name);
        }
    }
    ids.sort();
    let mut out = Vec::new();
    for id in ids {
        let job = match store.load(&id) {
            Ok(job) => job,
            Err(_) => continue,
        };
        let collection_id = target_collection_id(&job);
        for (index, file) in job.files.iter().enumerate() {
            if file.state != FileState::Cancelled {
                continue;
            }
            let Some(staging_name) = file.staging_name.clone() else {
                continue;
            };
            out.push(DurableEntry {
                job_id: job.id.clone(),
                index,
                identifier: file.identifier.clone(),
                file_name: file.file_name.clone(),
                collection_id: collection_id.clone(),
                sha256: file.sha256.clone().filter(|hash| !hash.trim().is_empty()),
                page_count: file.page_count,
                staging_name,
            });
        }
    }
    Ok(out)
}

/// Classify one durable entry with [`existing_import`] over its owning
/// job's target collection and the file's recorded sha256/page_count.
/// Anything unloadable (missing target or fingerprint) is `None`: there is
/// nothing the classification can vouch for.
fn classify_durable(
    conn: &rusqlite::Connection,
    data_dir: &Path,
    entry: &DurableEntry,
) -> Result<Existing, String> {
    let (Some(collection_id), Some(sha256), Some(page_count)) = (
        entry.collection_id.as_deref(),
        entry.sha256.as_deref(),
        entry.page_count,
    ) else {
        return Ok(Existing::None);
    };
    existing_import(
        conn,
        data_dir,
        collection_id,
        &entry.identifier,
        &entry.file_name,
        sha256,
        page_count,
    )
    .map_err(|error| format!("db: {error}"))
}

/// Classify every durable leftover with [`existing_import`]. The store scan
/// runs under the `jobs` lock (sync I/O only); classification runs under
/// the SQLite lock after, never holding both at once.
pub(crate) fn list_reconcile(
    store: &JobStore,
    jobs: &Mutex<()>,
    ui: &Mutex<rusqlite::Connection>,
    data_dir: &Path,
) -> Result<Vec<ReconcileDto>, String> {
    let entries = durable_entries(store, jobs)?;
    let conn = ui
        .lock()
        .map_err(|_| "io: local storage error".to_string())?;
    let mut out = Vec::with_capacity(entries.len());
    for entry in &entries {
        let (action, item_id) = match classify_durable(&conn, data_dir, entry)? {
            Existing::Complete { item_id } => (ReconcileAction::Keep, Some(item_id)),
            Existing::Partial { item_id } => (ReconcileAction::Discard, Some(item_id)),
            Existing::None => (ReconcileAction::None, None),
        };
        out.push(ReconcileDto {
            job_id: entry.job_id.clone(),
            index: entry.index,
            identifier: entry.identifier.clone(),
            file_name: entry.file_name.clone(),
            action,
            item_id,
        });
    }
    Ok(out)
}

/// Best-effort staging deletion for a finished leftover: the file plus its
/// `.partial`, like the runner's cleanup.
fn delete_reconcile_staging(store: &JobStore, job_id: &str, staging_name: &str) {
    let _ = store.delete_staging(job_id, staging_name);
    if let Ok(dest) = store.staging_path(job_id, staging_name) {
        let _ = std::fs::remove_file(internet_archive::partial_path(&dest));
        let _ = std::fs::remove_file(&dest);
    }
}

/// Release one durable leftover: re-classify the live job, refuse with
/// `partial_remains` while the remnant is still there (persisted state
/// untouched), otherwise delete the staging file and clear `staging_name`
/// via the [`update`] load-modify-save helper so the clearing persists and
/// a second list no longer returns it.
pub(crate) fn finish_reconcile(
    store: &JobStore,
    jobs: &Mutex<()>,
    ui: &Mutex<rusqlite::Connection>,
    data_dir: &Path,
    job_id: &str,
    index: usize,
) -> Result<(), String> {
    // Live snapshot under the writer lock; anything missing or not a
    // durable entry is `not_found` (no job shape is leaked).
    let snapshot: DurableEntry = {
        let _guard = jobs
            .lock()
            .map_err(|_| "io: local storage error".to_string())?;
        let job = match store.load(job_id) {
            Ok(job) => job,
            Err(_) => return Err("not_found: reconcile entry not found".to_string()),
        };
        let file = match job.files.get(index) {
            Some(file) => file,
            None => return Err("not_found: reconcile entry not found".to_string()),
        };
        if file.state != FileState::Cancelled {
            return Err("not_found: reconcile entry not found".to_string());
        }
        let Some(staging_name) = file.staging_name.clone() else {
            return Err("not_found: reconcile entry not found".to_string());
        };
        DurableEntry {
            job_id: job.id.clone(),
            index,
            identifier: file.identifier.clone(),
            file_name: file.file_name.clone(),
            collection_id: target_collection_id(&job),
            sha256: file.sha256.clone().filter(|hash| !hash.trim().is_empty()),
            page_count: file.page_count,
            staging_name,
        }
    };
    // Re-classify against the live collection: only a remnant that is still
    // there keeps the entry (and its staging) around.
    let existing = {
        let conn = ui
            .lock()
            .map_err(|_| "io: local storage error".to_string())?;
        classify_durable(&conn, data_dir, &snapshot)?
    };
    if matches!(existing, Existing::Partial { .. }) {
        return Err("partial_remains: the partial import is still in the collection".to_string());
    }
    let staging_name = snapshot.staging_name.clone();
    update(store, jobs, job_id, |job| {
        let file = match job.files.get(index) {
            Some(file) => file,
            None => return Err("not_found: reconcile entry not found".to_string()),
        };
        if file.state != FileState::Cancelled
            || file.staging_name.as_deref() != Some(staging_name.as_str())
        {
            return Err("not_found: reconcile entry not found".to_string());
        }
        job.files[index].staging_name = None;
        Ok(())
    })?;
    delete_reconcile_staging(store, job_id, &staging_name);
    Ok(())
}

/// Begin the frontend import of one verified file (see [`begin_file_import`]).
#[tauri::command]
pub fn ia_begin_file_import(
    job_id: String,
    index: usize,
    ia: State<'_, IaState>,
    db: State<'_, AppDbState>,
    app: tauri::AppHandle,
) -> Result<BeginDto, String> {
    begin_file_import(
        &ia.store,
        &ia.jobs,
        &db.ui_conn,
        &ia.data_dir,
        &job_id,
        index,
        now_secs(),
        &|job| emit_job_updated(&app, &job.id),
    )
}

/// Confirm the item the frontend imported (see [`confirm_file_imported`]).
#[tauri::command]
pub fn ia_confirm_file_imported(
    job_id: String,
    index: usize,
    item_id: String,
    ia: State<'_, IaState>,
    db: State<'_, AppDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    confirm_file_imported(
        &ia.store,
        &ia.jobs,
        &db.ui_conn,
        &ia.data_dir,
        &job_id,
        index,
        &item_id,
        now_secs(),
        &|job| emit_job_updated(&app, &job.id),
    )
}

/// Report a failed frontend import (see [`fail_file_import`]).
#[tauri::command]
pub fn ia_fail_file_import(
    job_id: String,
    index: usize,
    ia: State<'_, IaState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    fail_file_import(&ia.store, &ia.jobs, &job_id, index, now_secs(), &|job| {
        emit_job_updated(&app, &job.id)
    })
}

/// Classified recovery leftovers for the reconcile screen, derived on
/// demand from `job.json`: every `Cancelled` file that still has a
/// `staging_name`. No startup timing can lose them: the frontend may call
/// any time after `recover_all`.
#[tauri::command]
pub fn ia_list_reconcile(
    ia: State<'_, IaState>,
    db: State<'_, AppDbState>,
) -> Result<Vec<ReconcileDto>, String> {
    list_reconcile(&ia.store, &ia.jobs, &db.ui_conn, &ia.data_dir)
}

/// Release one recovery leftover (see [`finish_reconcile`]).
#[tauri::command]
pub fn ia_finish_reconcile(
    job_id: String,
    index: usize,
    ia: State<'_, IaState>,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    finish_reconcile(
        &ia.store,
        &ia.jobs,
        &db.ui_conn,
        &ia.data_dir,
        &job_id,
        index,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use internet_archive::job::{ImportTarget, JobFile, JobState};
    use internet_archive::store::new_staging_name;
    use internet_archive::{Availability, FileSource, PdfFile, PROVENANCE_KEY};
    use std::path::PathBuf;

    const IDENT: &str = "some-id";
    const FILE: &str = "doc.pdf";
    const SHA: &str = "abc123";
    const PAGES: u32 = 3;

    fn ia_pdf(name: &str) -> PdfFile {
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

    /// In-memory SQLite with the minimal `items`/`assets` surface task 23
    /// reads: `items` (id, collection_id, metadata), `assets` (id, item_id,
    /// path, type, parent_asset_id, page_number).
    fn mem_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory database");
        conn.execute_batch(
            "CREATE TABLE items (
               id TEXT PRIMARY KEY,
               collection_id TEXT NOT NULL,
               metadata TEXT
             );
             CREATE TABLE assets (
               id TEXT PRIMARY KEY,
               item_id TEXT NOT NULL,
               path TEXT NOT NULL,
               type TEXT NOT NULL,
               parent_asset_id TEXT,
               page_number INTEGER
             );",
        )
        .expect("minimal import schema");
        conn
    }

    fn prov_meta(identifier: &str, file_name: &str, sha256: &str) -> String {
        serde_json::json!({ PROVENANCE_KEY: {
            "identifier": identifier,
            "file_name": file_name,
            "sha256_local": sha256,
            "page_count": PAGES,
        }})
        .to_string()
    }

    fn insert_item(
        conn: &rusqlite::Connection,
        id: &str,
        collection: &str,
        metadata: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO items (id, collection_id, metadata) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, collection, metadata],
        )
        .expect("insert item");
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_asset(
        conn: &rusqlite::Connection,
        id: &str,
        item_id: &str,
        path: &str,
        asset_type: &str,
        parent: Option<&str>,
        page: Option<i64>,
    ) {
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, parent_asset_id, page_number)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, item_id, path, asset_type, parent, page],
        )
        .expect("insert asset");
    }

    /// Create the file behind a stored asset path (relative keys resolve
    /// under `data_dir`, absolute paths are used unchanged).
    fn touch(data_dir: &Path, stored: &str) {
        let path = resolve_asset_path(stored, data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("asset parent dirs");
        }
        std::fs::write(&path, b"asset-bytes").expect("write asset file");
    }

    /// A whole imported document: parent asset plus page children `1..=pages`
    /// with every file on disk.
    #[allow(clippy::too_many_arguments)]
    fn insert_complete_item(
        conn: &rusqlite::Connection,
        data_dir: &Path,
        item_id: &str,
        collection: &str,
        identifier: &str,
        file_name: &str,
        sha256: &str,
        pages: u32,
    ) {
        insert_item(
            conn,
            item_id,
            collection,
            Some(&prov_meta(identifier, file_name, sha256)),
        );
        let parent_id = format!("{item_id}-parent");
        let parent_path = format!("assets/{collection}/{item_id}/parent.pdf");
        insert_asset(conn, &parent_id, item_id, &parent_path, "pdf", None, None);
        touch(data_dir, &parent_path);
        for page in 1..=pages {
            let child_path = format!("assets/{collection}/{item_id}/p{page}.pdf");
            insert_asset(
                conn,
                &format!("{item_id}-p{page}"),
                item_id,
                &child_path,
                "pdf",
                Some(&parent_id),
                Some(i64::from(page)),
            );
            touch(data_dir, &child_path);
        }
    }

    struct Ctx {
        _root: tempfile::TempDir,
        data_dir: PathBuf,
        store: JobStore,
        ui: Mutex<rusqlite::Connection>,
    }

    fn ctx() -> Ctx {
        let root = tempfile::tempdir().expect("tempdir");
        let data_dir = root.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        Ctx {
            store: JobStore::new(root.path().join("staging")),
            data_dir,
            ui: Mutex::new(mem_db()),
            _root: root,
        }
    }

    fn verified_file(
        identifier: &str,
        file_name: &str,
        staging_name: &str,
        sha256: &str,
        page_count: u32,
    ) -> JobFile {
        let mut file = JobFile::from_pdf(identifier, &ia_pdf(file_name)).expect("available");
        file.state = FileState::Verified;
        file.staging_name = Some(staging_name.to_string());
        file.sha256 = Some(sha256.to_string());
        file.page_count = Some(page_count);
        file
    }

    fn running_job(files: Vec<JobFile>) -> Job {
        let mut job = Job::new(
            uuid::Uuid::new_v4().hyphenated().to_string(),
            1_700_000_000,
            ImportTarget::ExistingCollection {
                collection_id: "col-1".to_string(),
            },
            files,
        )
        .expect("job");
        job.set_job_state(JobState::Ready, 1_700_000_001)
            .expect("ready");
        job.set_job_state(JobState::Running, 1_700_000_002)
            .expect("running");
        job
    }

    /// Persist `job` and create every recorded staging file on disk so
    /// presence checks pass (recovery leftovers keep theirs too).
    fn persist(ctx: &Ctx, job: &Job) {
        ctx.store.create(job).expect("job persisted");
        for file in &job.files {
            if file.staging_name.is_some() {
                let name = file.staging_name.as_deref().expect("staging name");
                let dest = ctx.store.staging_path(&job.id, name).expect("staging path");
                std::fs::create_dir_all(dest.parent().expect("staging parent"))
                    .expect("staging parent");
                std::fs::write(&dest, b"%PDF-1.4 staged").expect("write staging");
            }
        }
    }

    fn staging_exists(ctx: &Ctx, job_id: &str, staging_name: &str) -> bool {
        ctx.store
            .staging_path(job_id, staging_name)
            .map(|path| path.is_file())
            .unwrap_or(false)
    }

    fn load(ctx: &Ctx, job_id: &str) -> Job {
        ctx.store.load(job_id).expect("job loads")
    }

    // ------------------------------------------------------------------
    // existing_import
    // ------------------------------------------------------------------

    #[test]
    fn existing_none_when_no_candidate() {
        let ctx = ctx();
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::None,
        );
    }

    #[test]
    fn existing_complete_parent_and_pages_present() {
        let ctx = ctx();
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::Complete {
                item_id: "item-1".to_string(),
            },
        );
    }

    #[test]
    fn existing_partial_when_a_page_is_missing() {
        let ctx = ctx();
        {
            let conn = ctx.ui.lock().unwrap();
            insert_item(&conn, "item-1", "col-1", Some(&prov_meta(IDENT, FILE, SHA)));
            insert_asset(
                &conn,
                "p0",
                "item-1",
                "assets/col-1/item-1/parent.pdf",
                "pdf",
                None,
                None,
            );
            touch(&ctx.data_dir, "assets/col-1/item-1/parent.pdf");
            // Pages 1 and 2 of 3: page 3 was never imported.
            for page in [1, 2] {
                let path = format!("assets/col-1/item-1/p{page}.pdf");
                insert_asset(
                    &conn,
                    &format!("c{page}"),
                    "item-1",
                    &path,
                    "pdf",
                    Some("p0"),
                    Some(page),
                );
                touch(&ctx.data_dir, &path);
            }
        }
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::Partial {
                item_id: "item-1".to_string(),
            },
        );
    }

    #[test]
    fn existing_partial_when_a_page_number_repeats() {
        let ctx = ctx();
        {
            let conn = ctx.ui.lock().unwrap();
            insert_item(&conn, "item-1", "col-1", Some(&prov_meta(IDENT, FILE, SHA)));
            insert_asset(
                &conn,
                "p0",
                "item-1",
                "assets/col-1/item-1/parent.pdf",
                "pdf",
                None,
                None,
            );
            touch(&ctx.data_dir, "assets/col-1/item-1/parent.pdf");
            // 1, 2, 2, 3: page 2 twice, so the set is not exactly 1..=3.
            for (n, page) in [1, 2, 2, 3].iter().enumerate() {
                let path = format!("assets/col-1/item-1/dup{n}.pdf");
                insert_asset(
                    &conn,
                    &format!("c{n}"),
                    "item-1",
                    &path,
                    "pdf",
                    Some("p0"),
                    Some(*page),
                );
                touch(&ctx.data_dir, &path);
            }
        }
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::Partial {
                item_id: "item-1".to_string(),
            },
        );
    }

    #[test]
    fn existing_partial_when_an_extra_page_exists() {
        let ctx = ctx();
        {
            let conn = ctx.ui.lock().unwrap();
            insert_item(&conn, "item-1", "col-1", Some(&prov_meta(IDENT, FILE, SHA)));
            insert_asset(
                &conn,
                "p0",
                "item-1",
                "assets/col-1/item-1/parent.pdf",
                "pdf",
                None,
                None,
            );
            touch(&ctx.data_dir, "assets/col-1/item-1/parent.pdf");
            for page in [1, 2, 3, 4] {
                let path = format!("assets/col-1/item-1/p{page}.pdf");
                insert_asset(
                    &conn,
                    &format!("c{page}"),
                    "item-1",
                    &path,
                    "pdf",
                    Some("p0"),
                    Some(page),
                );
                touch(&ctx.data_dir, &path);
            }
        }
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::Partial {
                item_id: "item-1".to_string(),
            },
        );
    }

    #[test]
    fn existing_partial_when_a_file_is_missing_on_disk() {
        let ctx = ctx();
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        // Page 2's file vanished from disk: the rows are whole, the bytes
        // are not.
        std::fs::remove_file(resolve_asset_path(
            "assets/col-1/item-1/p2.pdf",
            &ctx.data_dir,
        ))
        .expect("remove page file");
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::Partial {
                item_id: "item-1".to_string(),
            },
        );
    }

    #[test]
    fn existing_partial_when_there_is_no_parent() {
        let ctx = ctx();
        {
            let conn = ctx.ui.lock().unwrap();
            insert_item(&conn, "item-1", "col-1", Some(&prov_meta(IDENT, FILE, SHA)));
            // Page rows without any parent asset row at all.
            for page in [1, 2, 3] {
                let path = format!("assets/col-1/item-1/p{page}.pdf");
                insert_asset(
                    &conn,
                    &format!("c{page}"),
                    "item-1",
                    &path,
                    "pdf",
                    Some("ghost"),
                    Some(page),
                );
                touch(&ctx.data_dir, &path);
            }
        }
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::Partial {
                item_id: "item-1".to_string(),
            },
        );
    }

    #[test]
    fn existing_ignores_a_different_sha256_version() {
        let ctx = ctx();
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            "other-sha",
            PAGES,
        );
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::None,
        );
    }

    #[test]
    fn existing_complete_wins_over_partial_with_lowest_ids() {
        let ctx = ctx();
        {
            let conn = ctx.ui.lock().unwrap();
            // item-1: partial (only 2 of 3 pages). item-2: complete.
            insert_item(&conn, "item-1", "col-1", Some(&prov_meta(IDENT, FILE, SHA)));
            insert_asset(
                &conn,
                "p1",
                "item-1",
                "assets/col-1/item-1/parent.pdf",
                "pdf",
                None,
                None,
            );
            touch(&ctx.data_dir, "assets/col-1/item-1/parent.pdf");
            for page in [1, 2] {
                let path = format!("assets/col-1/item-1/p{page}.pdf");
                insert_asset(
                    &conn,
                    &format!("c{page}"),
                    "item-1",
                    &path,
                    "pdf",
                    Some("p1"),
                    Some(page),
                );
                touch(&ctx.data_dir, &path);
            }
        }
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-2",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        let conn = ctx.ui.lock().unwrap();
        assert_eq!(
            existing_import(&conn, &ctx.data_dir, "col-1", IDENT, FILE, SHA, PAGES)
                .expect("classifies"),
            Existing::Complete {
                item_id: "item-2".to_string(),
            },
        );
    }

    // ------------------------------------------------------------------
    // begin
    // ------------------------------------------------------------------

    fn begin_ctx() -> (Ctx, Job, String) {
        let ctx = ctx();
        let staging = new_staging_name();
        let job = running_job(vec![verified_file(IDENT, FILE, &staging, SHA, PAGES)]);
        persist(&ctx, &job);
        (ctx, job, staging)
    }

    #[test]
    fn begin_import_moves_verified_to_importing_with_the_payload() {
        let (ctx, job, _staging) = begin_ctx();
        let dto = begin_file_import(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job.id,
            0,
            1_700_000_010,
            &|_| {},
        )
        .expect("begin succeeds");
        assert_eq!(dto.action, BeginAction::Import);
        assert_eq!(dto.item_id, None);
        let entry = dto.file.expect("import carries the payload");
        assert_eq!(entry.identifier, IDENT);
        assert_eq!(entry.file_name, FILE);
        assert_eq!(entry.sha256, SHA);
        assert_eq!(entry.page_count, PAGES);
        // Same payload shape as an `ia_list_verified_files` entry.
        let listed = crate::internet_archive::commands::verified_files_logic(
            &ctx.store,
            &job,
            1_700_000_010,
        )
        .expect("list");
        assert_eq!(
            serde_json::to_value(&entry).expect("serializes"),
            serde_json::to_value(&listed[0]).expect("serializes"),
        );
        assert_eq!(load(&ctx, &job.id).files[0].state, FileState::Importing);
    }

    #[test]
    fn begin_confirm_existing_hands_over_the_item_id() {
        let (ctx, job, _staging) = begin_ctx();
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        let dto = begin_file_import(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job.id,
            0,
            1_700_000_010,
            &|_| {},
        )
        .expect("begin succeeds");
        assert_eq!(dto.action, BeginAction::ConfirmExisting);
        assert_eq!(dto.item_id.as_deref(), Some("item-1"));
        assert!(dto.file.is_none());
        // The file goes `Importing` so a following confirm completes it.
        assert_eq!(load(&ctx, &job.id).files[0].state, FileState::Importing);
    }

    #[test]
    fn begin_discard_partial_keeps_the_file_verified() {
        let (ctx, job, _staging) = begin_ctx();
        {
            let conn = ctx.ui.lock().unwrap();
            insert_item(&conn, "item-9", "col-1", Some(&prov_meta(IDENT, FILE, SHA)));
            insert_asset(
                &conn,
                "p9",
                "item-9",
                "assets/col-1/item-9/parent.pdf",
                "pdf",
                None,
                None,
            );
            touch(&ctx.data_dir, "assets/col-1/item-9/parent.pdf");
        }
        let dto = begin_file_import(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job.id,
            0,
            1_700_000_010,
            &|_| {},
        )
        .expect("begin succeeds");
        assert_eq!(dto.action, BeginAction::DiscardPartial);
        assert_eq!(dto.item_id.as_deref(), Some("item-9"));
        assert!(dto.file.is_none());
        assert_eq!(load(&ctx, &job.id).files[0].state, FileState::Verified);
    }

    #[test]
    fn begin_rejects_a_non_verified_file() {
        let ctx = ctx();
        let file = JobFile::from_pdf(IDENT, &ia_pdf(FILE)).expect("available");
        assert_eq!(file.state, FileState::Queued);
        let job = running_job(vec![file]);
        let job_id = job.id.clone();
        persist(&ctx, &job);
        let error = begin_file_import(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job_id,
            0,
            1_700_000_010,
            &|_| {},
        )
        .unwrap_err();
        assert!(error.starts_with("invalid_state"), "got {error:?}");
        assert!(!error.contains(&ctx.data_dir.display().to_string()));
    }

    #[test]
    fn begin_rejects_a_missing_staging_file() {
        let ctx = ctx();
        let staging = new_staging_name();
        let job = running_job(vec![verified_file(IDENT, FILE, &staging, SHA, PAGES)]);
        let job_id = job.id.clone();
        // Persisted without creating the staging bytes.
        ctx.store.create(&job).expect("job persisted");
        let error = begin_file_import(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job_id,
            0,
            1_700_000_010,
            &|_| {},
        )
        .unwrap_err();
        assert!(error.starts_with("staging_missing"), "got {error:?}");
    }

    // ------------------------------------------------------------------
    // confirm
    // ------------------------------------------------------------------

    fn confirm_ctx() -> (Ctx, Job, String) {
        let ctx = ctx();
        let staging = new_staging_name();
        let mut file = verified_file(IDENT, FILE, &staging, SHA, PAGES);
        file.state = FileState::Importing;
        let job = running_job(vec![file]);
        persist(&ctx, &job);
        (ctx, job, staging)
    }

    #[test]
    fn confirm_success_imports_and_settles_the_last_file() {
        let (ctx, job, staging) = confirm_ctx();
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        confirm_file_imported(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job.id,
            0,
            "item-1",
            1_700_000_010,
            &|_| {},
        )
        .expect("confirm succeeds");
        let saved = load(&ctx, &job.id);
        assert_eq!(saved.files[0].state, FileState::Imported);
        assert_eq!(saved.files[0].item_id.as_deref(), Some("item-1"));
        assert!(!staging_exists(&ctx, &job.id, &staging), "staging gone");
        assert_eq!(saved.state, JobState::Completed);
    }

    #[test]
    fn confirm_accepts_a_complete_item_when_an_older_complete_copy_exists() {
        let (ctx, job, _staging) = confirm_ctx();
        {
            let conn = ctx.ui.lock().unwrap();
            for item in ["item-a", "item-b"] {
                insert_complete_item(&conn, &ctx.data_dir, item, "col-1", IDENT, FILE, SHA, PAGES);
            }
        }
        confirm_file_imported(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job.id,
            0,
            "item-b",
            1_700_000_010,
            &|_| {},
        )
        .expect("the confirmed item is complete on its own");
        let saved = load(&ctx, &job.id);
        assert_eq!(saved.files[0].item_id.as_deref(), Some("item-b"));
    }

    fn assert_confirm_failure(
        setup: impl FnOnce(&rusqlite::Connection, &Path),
        item_id: &str,
        code: &str,
    ) {
        let (ctx, job, staging) = confirm_ctx();
        setup(&ctx.ui.lock().unwrap(), &ctx.data_dir);
        let error = confirm_file_imported(
            &ctx.store,
            &Mutex::new(()),
            &ctx.ui,
            &ctx.data_dir,
            &job.id,
            0,
            item_id,
            1_700_000_010,
            &|_| {},
        )
        .unwrap_err();
        assert!(error.starts_with(code), "got {error:?} want {code}");
        let saved = load(&ctx, &job.id);
        assert_eq!(saved.files[0].state, FileState::Verified);
        assert!(staging_exists(&ctx, &job.id, &staging), "staging kept");
    }

    #[test]
    fn confirm_item_not_found_returns_to_verified() {
        assert_confirm_failure(|_, _| {}, "item-gone", "item_not_found");
    }

    #[test]
    fn confirm_wrong_collection_returns_to_verified() {
        assert_confirm_failure(
            |conn, data_dir| {
                insert_complete_item(
                    conn,
                    data_dir,
                    "item-1",
                    "col-other",
                    IDENT,
                    FILE,
                    SHA,
                    PAGES,
                );
            },
            "item-1",
            "wrong_collection",
        );
    }

    #[test]
    fn confirm_provenance_mismatch_returns_to_verified() {
        assert_confirm_failure(
            |conn, data_dir| {
                insert_complete_item(
                    conn, data_dir, "item-1", "col-1", "other-id", FILE, SHA, PAGES,
                );
            },
            "item-1",
            "provenance_mismatch",
        );
    }

    #[test]
    fn confirm_incomplete_import_returns_to_verified() {
        assert_confirm_failure(
            |conn, data_dir| {
                // Provenance matches but only 2 of 3 pages made it.
                conn.execute(
                    "INSERT INTO items (id, collection_id, metadata) VALUES ('item-1', 'col-1', ?1)",
                    rusqlite::params![prov_meta(IDENT, FILE, SHA)],
                )
                .expect("insert item");
                insert_asset(
                    conn,
                    "p1",
                    "item-1",
                    "assets/col-1/item-1/parent.pdf",
                    "pdf",
                    None,
                    None,
                );
                touch(data_dir, "assets/col-1/item-1/parent.pdf");
                for page in [1, 2] {
                    let path = format!("assets/col-1/item-1/p{page}.pdf");
                    insert_asset(
                        conn,
                        &format!("c{page}"),
                        "item-1",
                        &path,
                        "pdf",
                        Some("p1"),
                        Some(page),
                    );
                    touch(data_dir, &path);
                }
            },
            "item-1",
            "incomplete_import",
        );
    }

    // ------------------------------------------------------------------
    // fail
    // ------------------------------------------------------------------

    #[test]
    fn fail_moves_importing_to_failed_without_staging() {
        let (ctx, job, staging) = confirm_ctx();
        fail_file_import(
            &ctx.store,
            &Mutex::new(()),
            &job.id,
            0,
            1_700_000_010,
            &|_| {},
        )
        .expect("fail succeeds");
        let saved = load(&ctx, &job.id);
        assert_eq!(saved.files[0].state, FileState::Failed);
        assert_eq!(saved.files[0].error.as_deref(), Some("import_failed"));
        assert!(!staging_exists(&ctx, &job.id, &staging), "staging gone");
        assert_eq!(saved.state, JobState::Failed);
    }

    // ------------------------------------------------------------------
    // reconcile (durable, derived on demand: `Cancelled` + `staging_name`)
    // ------------------------------------------------------------------

    fn reconcile_ctx(pages_setup: impl FnOnce(&rusqlite::Connection, &Path)) -> (Ctx, Job, String) {
        let ctx = ctx();
        let staging = new_staging_name();
        // A leftover of an interrupted frontend import: `Cancelled` with the
        // recorded sha256/page_count and the staging still on disk.
        let mut file = verified_file(IDENT, FILE, &staging, SHA, PAGES);
        file.state = FileState::Cancelled;
        let job = running_job(vec![file]);
        persist(&ctx, &job);
        pages_setup(&ctx.ui.lock().unwrap(), &ctx.data_dir);
        (ctx, job, staging)
    }

    /// A `Cancelling` job with an `Importing` file, as a crash would leave
    /// it: `recover_all` normalizes it to `Cancelled` and the durable scan
    /// must find it with no in-memory list.
    fn cancelling_importing_ctx() -> (Ctx, Job, String) {
        let ctx = ctx();
        let staging = new_staging_name();
        let mut file = verified_file(IDENT, FILE, &staging, SHA, PAGES);
        file.state = FileState::Importing;
        let mut job = Job::new(
            uuid::Uuid::new_v4().hyphenated().to_string(),
            1_700_000_000,
            ImportTarget::ExistingCollection {
                collection_id: "col-1".to_string(),
            },
            vec![file],
        )
        .expect("job");
        job.set_job_state(JobState::Ready, 1_700_000_001)
            .expect("ready");
        job.set_job_state(JobState::Running, 1_700_000_002)
            .expect("running");
        job.set_job_state(JobState::Cancelling, 1_700_000_003)
            .expect("cancelling");
        persist(&ctx, &job);
        (ctx, job, staging)
    }

    #[test]
    fn durable_entry_exists_after_restart_without_in_memory_state() {
        let (ctx, job, staging) = cancelling_importing_ctx();
        assert!(staging_exists(&ctx, &job.id, &staging));
        // Simulate the restart: `recover_all` persists the `Cancelled` job.
        let report = ctx.store.recover_all(1_700_000_010);
        assert!(report.jobs.iter().any(|recovered| recovered.id == job.id));
        let saved = load(&ctx, &job.id);
        assert_eq!(saved.state, JobState::Cancelled);
        assert_eq!(saved.files[0].state, FileState::Cancelled);
        assert_eq!(
            saved.files[0].staging_name.as_deref(),
            Some(staging.as_str())
        );
        // Derived on demand: no in-memory list, just the store scan.
        let jobs = Mutex::new(());
        let out = list_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir).expect("list succeeds");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].job_id, job.id);
        assert_eq!(out[0].index, 0);
        assert_eq!(out[0].identifier, IDENT);
        assert_eq!(out[0].file_name, FILE);
    }

    #[test]
    fn reconcile_classifies_keep_discard_and_none() {
        let jobs = Mutex::new(());
        let (ctx, job, _staging) = reconcile_ctx(|_, _| {});
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        let out = list_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir).expect("list succeeds");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].job_id, job.id);
        assert_eq!(out[0].action, ReconcileAction::Keep);
        assert_eq!(out[0].item_id.as_deref(), Some("item-1"));

        // Partial remnant: discard with the item id.
        let (ctx, job, _staging) = reconcile_ctx(|conn, data_dir| {
            conn.execute(
                "INSERT INTO items (id, collection_id, metadata) VALUES ('item-9', 'col-1', ?1)",
                rusqlite::params![prov_meta(IDENT, FILE, SHA)],
            )
            .expect("insert item");
            insert_asset(
                conn,
                "p9",
                "item-9",
                "assets/col-1/item-9/parent.pdf",
                "pdf",
                None,
                None,
            );
            touch(data_dir, "assets/col-1/item-9/parent.pdf");
        });
        let out = list_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir).expect("list succeeds");
        // Only this ctx's job lives in its own store, so exactly one entry.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].job_id, job.id);
        assert_eq!(out[0].action, ReconcileAction::Discard);
        assert_eq!(out[0].item_id.as_deref(), Some("item-9"));

        // Nothing imported: none.
        let (ctx, job, _staging) = reconcile_ctx(|_, _| {});
        let out = list_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir).expect("list succeeds");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].job_id, job.id);
        assert_eq!(out[0].action, ReconcileAction::None);
        assert_eq!(out[0].item_id, None);
    }

    #[test]
    fn finish_persists_cleared_staging_and_second_list_is_empty() {
        let (ctx, job, staging) = reconcile_ctx(|_, _| {});
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        let jobs = Mutex::new(());
        finish_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir, &job.id, 0)
            .expect("finish succeeds");
        // Persisted: the name is cleared in `job.json`, not just memory.
        let saved = load(&ctx, &job.id);
        assert_eq!(saved.files[0].staging_name, None);
        assert!(!staging_exists(&ctx, &job.id, &staging), "staging gone");
        let out = list_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir).expect("list succeeds");
        assert!(out.is_empty(), "second list no longer returns it");
        // A second finish finds nothing to release.
        let error =
            finish_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir, &job.id, 0).unwrap_err();
        assert!(error.starts_with("not_found"), "got {error:?}");
    }

    #[test]
    fn finish_partial_remains_keeps_entry_and_staging() {
        let (ctx, job, staging) = reconcile_ctx(|conn, data_dir| {
            conn.execute(
                "INSERT INTO items (id, collection_id, metadata) VALUES ('item-9', 'col-1', ?1)",
                rusqlite::params![prov_meta(IDENT, FILE, SHA)],
            )
            .expect("insert item");
            insert_asset(
                conn,
                "p9",
                "item-9",
                "assets/col-1/item-9/parent.pdf",
                "pdf",
                None,
                None,
            );
            touch(data_dir, "assets/col-1/item-9/parent.pdf");
        });
        let jobs = Mutex::new(());
        let before = load(&ctx, &job.id);
        let error =
            finish_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir, &job.id, 0).unwrap_err();
        assert!(error.starts_with("partial_remains"), "got {error:?}");
        // Persisted state unchanged: the name and the file stay.
        let saved = load(&ctx, &job.id);
        assert_eq!(saved, before);
        assert_eq!(
            saved.files[0].staging_name.as_deref(),
            Some(staging.as_str())
        );
        assert!(staging_exists(&ctx, &job.id, &staging), "staging kept");
        let out = list_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir).expect("list succeeds");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].action, ReconcileAction::Discard);
    }

    #[test]
    fn finish_releases_keep_and_none() {
        let jobs = Mutex::new(());
        // Complete: staging deleted, entry gone.
        let (ctx, job, staging) = reconcile_ctx(|_, _| {});
        insert_complete_item(
            &ctx.ui.lock().unwrap(),
            &ctx.data_dir,
            "item-1",
            "col-1",
            IDENT,
            FILE,
            SHA,
            PAGES,
        );
        finish_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir, &job.id, 0)
            .expect("finish succeeds");
        assert_eq!(load(&ctx, &job.id).files[0].staging_name, None);
        assert!(!staging_exists(&ctx, &job.id, &staging), "staging gone");

        // None: same release.
        let (ctx, job, staging) = reconcile_ctx(|_, _| {});
        finish_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir, &job.id, 0)
            .expect("finish succeeds");
        assert_eq!(load(&ctx, &job.id).files[0].staging_name, None);
        assert!(!staging_exists(&ctx, &job.id, &staging), "staging gone");
    }

    #[test]
    fn live_cancel_leaves_no_durable_entries() {
        use crate::internet_archive::runner::apply_end_of_run_cancel;

        let ctx = ctx();
        let staging = new_staging_name();
        let file = verified_file(IDENT, FILE, &staging, SHA, PAGES);
        let job = running_job(vec![file]);
        persist(&ctx, &job);
        let jobs = Mutex::new(());
        let (applied, _) = apply_end_of_run_cancel(&ctx.store, &jobs, &job.id, true, 1_700_000_030)
            .expect("cancel applies");
        assert!(applied);
        let out = list_reconcile(&ctx.store, &jobs, &ctx.ui, &ctx.data_dir).expect("list succeeds");
        assert!(out.is_empty(), "live cancel must leave no entries");
    }

    #[test]
    fn begin_dto_uses_camel_case_keys() {
        let dto = BeginDto {
            action: BeginAction::ConfirmExisting,
            item_id: Some("item-1".to_string()),
            file: None,
        };
        let value = serde_json::to_value(&dto).expect("dto serializes");
        assert_eq!(value["action"], "confirm_existing");
        assert_eq!(value["itemId"], "item-1");
        assert!(value.get("item_id").is_none());
        assert!(value.get("file").is_none());

        let import = BeginDto {
            action: BeginAction::Import,
            item_id: None,
            file: None,
        };
        let value = serde_json::to_value(&import).expect("dto serializes");
        assert_eq!(value["action"], "import");

        let discard = BeginDto {
            action: BeginAction::DiscardPartial,
            item_id: Some("item-9".to_string()),
            file: None,
        };
        let value = serde_json::to_value(&discard).expect("dto serializes");
        assert_eq!(value["action"], "discard_partial");
    }

    #[test]
    fn reconcile_dto_uses_camel_case_keys() {
        let dto = ReconcileDto {
            job_id: "job-1".to_string(),
            index: 0,
            identifier: IDENT.to_string(),
            file_name: FILE.to_string(),
            action: ReconcileAction::Discard,
            item_id: Some("item-9".to_string()),
        };
        let value = serde_json::to_value(&dto).expect("dto serializes");
        assert_eq!(value["jobId"], "job-1");
        assert_eq!(value["fileName"], FILE);
        assert_eq!(value["action"], "discard");
        assert_eq!(value["itemId"], "item-9");
        assert!(value.get("job_id").is_none());
    }
}
