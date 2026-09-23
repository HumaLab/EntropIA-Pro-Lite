//! Bibliographic sync execution and durable publication behind the batch queue (E2b-3).
//!
//! E2b-1 admitted `bibliography_sync` tasks; this module is the engine that
//! runs them: it resolves the claimed internal `zotero_libraries` row to the
//! local-Zotero library it mirrors, then walks that library page by page
//! through a narrow, injectable page source. The seam is async because the
//! production source is HTTP; the queue's [`Executor`] contract is
//! synchronous, so the executor blocks on each page from the scheduler
//! thread exactly like the OCR engine does — one page in flight at a time,
//! the cooperative stop observed between pages, never inside a request.
//!
//! Each answered page is one small transaction containing its item upserts,
//! run-scoped seen keys, reconciliation cursor and queue checkpoint. Trusted
//! completion is published later inside the scheduler success transaction,
//! where unseen items are tombstoned and the task receipt is committed with
//! the finalized reconciliation row.
//!
//! # Honest states
//!
//! Every verdict names what was observed, never a guess:
//!
//! - `library_missing` (fatal): the internal `zotero_libraries` row the
//!   task's subject names is gone from the catalog. Internal state, not an
//!   endpoint state — retrying cannot bring it back.
//! - `zotero_unreachable` / `zotero_timeout` (retryable): nothing answered,
//!   or it answered too slowly. The endpoint may come back.
//! - `zotero_api_disabled` (blocked): the local API refused the read; a
//!   human toggles the Zotero setting and resumes.
//! - `zotero_library_not_found` (fatal): the endpoint answered definitively
//!   that this library does not exist there anymore.
//! - `zotero_invalid_response` (retryable): the answer was malformed or
//!   self-inconsistent (zero items while the total says more remain). A
//!   malformed answer is never treated as an empty library.
//! - `zotero_snapshot_changed` (retryable task, retired reconciliation): a
//!   page crossed the run's version/identity fence; the next attempt starts
//!   from zero rather than mixing snapshots.
//!
//! No message claims Zotero is closed or not installed: nothing observable
//! here distinguishes those, and the writing workspace's diagnosis rules
//! (§11.3) forbid the assertion.

use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::reconciliation::{
    self, BeginReconciliationInput, ReconciliationEntityKind, ReconciliationErrorInput,
    ReconciliationPageInput, ReconciliationRun, ReconciliationRunRef, ReconciliationSeenInput,
};
use super::repository::{self as catalog_repository, BibliographicItemInput};
use crate::db::open::open_archive_connection;
use crate::processing::repository::{self as processing_repository, BIBLIOGRAPHY_SYNC_CONTRACT};
use crate::processing::scheduler::{
    ClaimedTask, EngineOutput, ExecCtx, ExecOutput, ExecResult, Executor, NewCheckpoint, StopFlag,
};
use crate::writing::zotero::connector::{self, Page as ConnectorPage};
use crate::writing::zotero::{Library, LibraryType, ZoteroState};

/// One bounded page request: `start` plus a `limit` clamped to the range the
/// local API accepts (1..=[`connector::MAX_LIMIT`]). Constructed through
/// [`BibliographyPageQuery::new`], so an unbounded request cannot be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BibliographyPageQuery {
    start: u32,
    limit: u32,
}

impl BibliographyPageQuery {
    pub fn new(start: u32, limit: u32) -> Self {
        Self {
            start,
            limit: limit.clamp(1, connector::MAX_LIMIT),
        }
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn limit(self) -> u32 {
        self.limit
    }
}

/// One work as a page carries it: the complete native Zotero row plus its
/// qualified key/version and normalized CSL-JSON text (the
/// `format=json&include=csljson` shape the writing connector already reads).
/// Keeping the full row is required for E2b-3's lossless native snapshot;
/// re-constructing it from CSL would discard Zotero fields. Owned data only —
/// no client, runtime or transport type crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibliographyPageItem {
    pub key: String,
    pub item_version: u64,
    pub native_json_snapshot: String,
    pub csl_json: String,
}

/// One answered page of a library's works. `total` and `library_version`
/// are the headers Zotero sends (`total-results`, `Last-Modified-Version`)
/// when it sends them; `None` means the walk must decide from the page
/// itself. Versions live in the page contract — every item carries its own —
/// so no separate version call is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibliographyPage {
    pub items: Vec<BibliographyPageItem>,
    pub total: Option<u64>,
    pub library_version: Option<u64>,
}

impl BibliographyPage {
    /// A consistent terminal answer: zero items, no totals to contradict it.
    pub fn terminal_empty() -> Self {
        Self {
            items: Vec::new(),
            total: Some(0),
            library_version: None,
        }
    }
}

/// Parses one page body strictly: every row must yield its native key,
/// version and CSL text, or the whole page is [`ZoteroState::InvalidResponse`].
///
/// Strict where the writing mirror is lenient: a sync that silently dropped
/// rows would reconcile (E2b-3) against a library it never fully read, and a
/// page whose rows do not parse is an answer about Zotero, not a smaller
/// library. An empty array is valid and distinct — it is a real answer that
/// says "no works here".
pub fn page_items_from_json(
    body: &serde_json::Value,
) -> Result<Vec<BibliographyPageItem>, ZoteroState> {
    let rows = body
        .as_array()
        .ok_or_else(|| ZoteroState::InvalidResponse {
            detail: "the library's answer was not a list of items".into(),
        })?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.is_object() {
            return Err(malformed_row());
        }
        let key = row
            .get("key")
            .and_then(|v| v.as_str())
            .filter(|key| !key.is_empty())
            .ok_or_else(malformed_row)?;
        let version = row
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or_else(malformed_row)?;
        let csl_text = row
            .get("csljson")
            .and_then(|v| v.as_str())
            .ok_or_else(malformed_row)?;
        // The CSL arrives as text holding a one-element array (S5); accept
        // exactly the shapes the writing parser accepts, so a citation made
        // from a synced page is indistinguishable from a live read.
        let parsed: serde_json::Value =
            serde_json::from_str(csl_text).map_err(|_| malformed_row())?;
        let csl = match parsed {
            serde_json::Value::Array(mut entries) if entries.len() == 1 => entries.remove(0),
            object @ serde_json::Value::Object(_) => object,
            _ => return Err(malformed_row()),
        };
        items.push(BibliographyPageItem {
            key: key.to_string(),
            item_version: version,
            native_json_snapshot: row.to_string(),
            csl_json: csl.to_string(),
        });
    }
    Ok(items)
}

fn malformed_row() -> ZoteroState {
    ZoteroState::InvalidResponse {
        detail: "an item of the page had no key, version or readable CSL".into(),
    }
}

/// The future a page source resolves to. `'static` + `Send` so the same seam
/// serves the scheduler thread today and any executor E2b-3 chooses; no
/// reqwest or tokio type appears in it.
pub type PageFuture =
    Pin<Box<dyn std::future::Future<Output = Result<BibliographyPage, ZoteroState>> + Send>>;

/// Narrow client seam for one library's pages: the executor asks for one
/// bounded page at a time and owns pacing and stopping. Anything wider
/// (collections, tags, attachments, deletions) is out of scope for the sync
/// contract by design; E2b-3 reconciles works only.
pub trait ZoteroPageSource: Send + Sync {
    fn fetch_page(&self, library: &Library, query: BibliographyPageQuery) -> PageFuture;
}

/// Production page source: Zotero's local API through the writing
/// connector's URL and request conventions (bounded `/items/top` pages with
/// `format=json&include=csljson`, the connector's timeout, honest status
/// classification).
pub struct LocalZoteroPageSource {
    client: reqwest::Client,
}

impl LocalZoteroPageSource {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for LocalZoteroPageSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ZoteroPageSource for LocalZoteroPageSource {
    fn fetch_page(&self, library: &Library, query: BibliographyPageQuery) -> PageFuture {
        let client = self.client.clone();
        let library = library.clone();
        Box::pin(async move {
            let answer = connector::read_entries_page(
                &client,
                &library,
                ConnectorPage::new(query.start(), query.limit()),
            )
            .await?;
            Ok(BibliographyPage {
                items: page_items_from_json(&answer.body)?,
                total: answer.total,
                library_version: answer.library_version,
            })
        })
    }
}

/// What the claimed subject resolved to inside the archive.
#[derive(Debug)]
enum LibrarySubject {
    /// The row exists and names an addressable local-API library under the
    /// current connection identity fence.
    Present {
        library: Library,
        connection_revision: i64,
    },
    /// No such row: the subject is gone from the migrated catalog.
    Missing,
    /// The row exists but its stored identity cannot address any library.
    Invalid(String),
}

/// Resolves one internal `zotero_libraries` row id to the local-API library
/// it mirrors, on the executor's own short-lived read connection (the
/// supervisor connection is never shared with engines).
fn resolve_library_subject(
    db_path: &std::path::Path,
    library_row_id: &str,
) -> Result<LibrarySubject, String> {
    let conn = open_archive_connection(db_path)?;
    let stored: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT l.library_type, l.library_id, c.revision
               FROM zotero_libraries l
               JOIN zotero_connections c ON c.id = l.connection_id
              WHERE l.id = ?1",
            [library_row_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| format!("Failed to read Zotero library {library_row_id}: {error}"))?;
    let Some((library_type, library_id, connection_revision)) = stored else {
        return Ok(LibrarySubject::Missing);
    };
    let library_type = match library_type.as_str() {
        "user" => LibraryType::User,
        "group" => LibraryType::Group,
        // Unreachable while the table CHECK holds; kept honest anyway.
        other => {
            return Ok(LibrarySubject::Invalid(format!(
                "library {library_row_id} stores unknown type {other:?}"
            )))
        }
    };
    match Library::new(library_type, library_id) {
        Ok(library) => Ok(LibrarySubject::Present {
            library,
            connection_revision,
        }),
        Err(detail) => Ok(LibrarySubject::Invalid(format!(
            "library {library_row_id} holds an unusable identity: {detail}"
        ))),
    }
}

/// The walk's advance rule, pure so every paging shape is testable without
/// a source: returns the next `start`, or `None` when the enumeration is
/// complete. The next start advances by what was actually read, so a row
/// vanishing mid-walk can never be skipped over.
fn pagination_next(start: u32, fetched: usize, limit: u32, total: Option<u64>) -> Option<u32> {
    if fetched == 0 {
        return None;
    }
    let next = start.saturating_add(fetched as u32);
    match total {
        Some(total) if u64::from(next) < total => Some(next),
        Some(_) => None,
        // Without a count there is nothing to plan with: a full page is the
        // only signal another page might exist.
        None if fetched >= limit as usize => Some(next),
        None => None,
    }
}

/// An answer that contradicts itself: zero items on a page the total says
/// still has works left. Treated as malformed — retry the endpoint — never
/// as "the library is empty", which would drop every remaining work from a
/// later reconciliation.
fn empty_while_total_remains(fetched: usize, start: u32, total: Option<u64>) -> bool {
    fetched == 0 && total.is_some_and(|total| u64::from(start) < total)
}

/// One complete page payload. It is checksummed and stored in the same
/// transaction as the page's catalog rows and reconciliation cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PageCheckpoint {
    start: u32,
    next_start: u32,
    total: Option<u64>,
    terminal: bool,
    items: Vec<BibliographyPageItem>,
    library_version: Option<u64>,
}

/// Explicit bibliography product routed by the scheduler. Page rows are
/// already durable; publication performs only trusted tombstone/finalize work
/// inside the task-success transaction.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyComputeOutput {
    pub library_row_id: String,
    pub run_id: String,
    pub connection_revision: i64,
    pub items_seen: i64,
    pub remote_total: Option<i64>,
    pub target_version: Option<i64>,
}

impl BibliographyComputeOutput {
    fn run_ref(&self) -> ReconciliationRunRef {
        ReconciliationRunRef {
            library_id: self.library_row_id.clone(),
            run_id: self.run_id.clone(),
            connection_revision: self.connection_revision,
        }
    }
}

fn run_ref(run: &ReconciliationRun) -> ReconciliationRunRef {
    ReconciliationRunRef {
        library_id: run.library_id.clone(),
        run_id: run.run_id.clone(),
        connection_revision: run.connection_revision,
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_or_first(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| match value {
        serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        _ => None,
    })
}

fn csl_date(csl: &serde_json::Value) -> Option<String> {
    let issued = csl.get("issued")?;
    if let Some(raw) = string_field(issued, "raw") {
        return Some(raw);
    }
    let parts = issued.get("date-parts")?.as_array()?.first()?.as_array()?;
    let parts: Vec<String> = parts
        .iter()
        .filter_map(serde_json::Value::as_i64)
        .map(|part| part.to_string())
        .collect();
    (!parts.is_empty()).then(|| parts.join("-"))
}

fn creators_snapshot(native_data: &serde_json::Value, csl: &serde_json::Value) -> Option<String> {
    if let Some(creators) = native_data.get("creators").filter(|value| value.is_array()) {
        return Some(creators.to_string());
    }
    for (field, creator_type) in [("author", "author"), ("editor", "editor")] {
        let Some(creators) = csl.get(field).and_then(serde_json::Value::as_array) else {
            continue;
        };
        let mapped: Vec<serde_json::Value> = creators
            .iter()
            .filter_map(serde_json::Value::as_object)
            .map(|creator| {
                let mut mapped = serde_json::Map::new();
                mapped.insert(
                    "creatorType".to_string(),
                    serde_json::Value::String(creator_type.to_string()),
                );
                if let Some(given) = creator.get("given").and_then(serde_json::Value::as_str) {
                    mapped.insert(
                        "firstName".to_string(),
                        serde_json::Value::String(given.to_string()),
                    );
                }
                if let Some(family) = creator.get("family").and_then(serde_json::Value::as_str) {
                    mapped.insert(
                        "lastName".to_string(),
                        serde_json::Value::String(family.to_string()),
                    );
                }
                if let Some(literal) = creator.get("literal").and_then(serde_json::Value::as_str) {
                    mapped.insert(
                        "name".to_string(),
                        serde_json::Value::String(literal.to_string()),
                    );
                }
                serde_json::Value::Object(mapped)
            })
            .collect();
        if !mapped.is_empty() {
            return Some(serde_json::Value::Array(mapped).to_string());
        }
    }
    None
}

fn item_input(item: &BibliographyPageItem) -> Result<BibliographicItemInput, String> {
    let native: serde_json::Value = serde_json::from_str(&item.native_json_snapshot)
        .map_err(|error| format!("native item {} is not JSON: {error}", item.key))?;
    let native = native
        .as_object()
        .ok_or_else(|| format!("native item {} is not an object", item.key))?;
    if native.get("key").and_then(serde_json::Value::as_str) != Some(item.key.as_str())
        || native.get("version").and_then(serde_json::Value::as_u64) != Some(item.item_version)
    {
        return Err(format!(
            "native identity of item {} does not match its page key/version",
            item.key
        ));
    }
    let csl: serde_json::Value = serde_json::from_str(&item.csl_json)
        .map_err(|error| format!("CSL item {} is not JSON: {error}", item.key))?;
    if !csl.is_object() {
        return Err(format!("CSL item {} is not an object", item.key));
    }
    let native = serde_json::Value::Object(native.clone());
    let native_data = native
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&native);
    let item_version = i64::try_from(item.item_version)
        .map_err(|_| format!("item {} version exceeds SQLite's integer range", item.key))?;

    Ok(BibliographicItemInput {
        item_key: item.key.clone(),
        item_version: Some(item_version),
        native_json_snapshot: item.native_json_snapshot.clone(),
        csl_json_snapshot: item.csl_json.clone(),
        item_type: string_field(native_data, "itemType").or_else(|| string_field(&csl, "type")),
        title: string_field(native_data, "title").or_else(|| string_field(&csl, "title")),
        creators_json: creators_snapshot(native_data, &csl),
        publication_title: string_field(native_data, "publicationTitle")
            .or_else(|| string_or_first(&csl, "container-title")),
        publisher: string_field(native_data, "publisher")
            .or_else(|| string_field(&csl, "publisher")),
        date: string_field(native_data, "date").or_else(|| csl_date(&csl)),
        doi: string_field(native_data, "DOI").or_else(|| string_field(&csl, "DOI")),
        isbn: string_field(native_data, "ISBN").or_else(|| string_field(&csl, "ISBN")),
        abstract_text: string_field(native_data, "abstractNote")
            .or_else(|| string_field(&csl, "abstract")),
        language: string_field(native_data, "language").or_else(|| string_field(&csl, "language")),
        url: string_field(native_data, "url").or_else(|| string_field(&csl, "URL")),
    })
}

#[derive(Debug)]
enum PagePersistError {
    InvalidResponse(String),
    SnapshotChanged(String),
    ConnectionChanged(String),
    LeaseLost(String),
    // E2c-WU4: demand withdrawn between claim and page commit (WU2
    // save_checkpoint fence). Like a lost lease, this parks the walk as
    // Stopped with staged checkpoints — never a storage Fatal.
    DemandLost(String),
    Storage(String),
}

fn persist_page(
    ctx: &ExecCtx,
    task: &ClaimedTask,
    run: &ReconciliationRun,
    checkpoint: &NewCheckpoint,
    start: u32,
    next_start: u32,
    page: &BibliographyPage,
) -> Result<ReconciliationRun, PagePersistError> {
    let mut page_keys = HashSet::with_capacity(page.items.len());
    for item in &page.items {
        if !page_keys.insert(item.key.as_str()) {
            return Err(PagePersistError::InvalidResponse(format!(
                "the library repeated item key {} within one page",
                item.key
            )));
        }
    }
    let inputs: Vec<BibliographicItemInput> = page
        .items
        .iter()
        .map(item_input)
        .collect::<Result<_, _>>()
        .map_err(PagePersistError::InvalidResponse)?;
    let remote_total = page.total.map(i64::try_from).transpose().map_err(|_| {
        PagePersistError::InvalidResponse("remote total exceeds SQLite's integer range".to_string())
    })?;
    let target_version = page
        .library_version
        .map(i64::try_from)
        .transpose()
        .map_err(|_| {
            PagePersistError::InvalidResponse(
                "library version exceeds SQLite's integer range".to_string(),
            )
        })?;
    let observed_at = processing_repository::now_ms();
    let seen = page
        .items
        .iter()
        .map(|item| {
            let remote_version = i64::try_from(item.item_version).map_err(|_| {
                PagePersistError::InvalidResponse(format!(
                    "item {} version exceeds SQLite's integer range",
                    item.key
                ))
            })?;
            Ok(ReconciliationSeenInput {
                entity_kind: ReconciliationEntityKind::Item,
                entity_key: item.key.clone(),
                parent_key: None,
                remote_version: Some(remote_version),
                observed_at,
            })
        })
        .collect::<Result<Vec<_>, PagePersistError>>()?;

    let conn = open_archive_connection(&ctx.db_path).map_err(PagePersistError::Storage)?;
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|error| {
        PagePersistError::Storage(format!("Failed to begin bibliography page: {error}"))
    })?;
    let persisted = (|| {
        for item in &page.items {
            let already_seen =
                conn.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM zotero_reconciliation_seen
                         WHERE library_id = ?1 AND run_id = ?2
                           AND entity_kind = 'item' AND entity_key = ?3
                    )",
                    rusqlite::params![&run.library_id, &run.run_id, &item.key],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| {
                    PagePersistError::Storage(format!(
                        "Failed to validate bibliography page identity: {error}"
                    ))
                })? != 0;
            if already_seen {
                return Err(PagePersistError::SnapshotChanged(format!(
                    "the library repeated item key {} across pages",
                    item.key
                )));
            }
        }
        for input in inputs {
            catalog_repository::upsert_item_in_transaction(&conn, &run.library_id, input).map_err(
                |error| match error.code.as_str() {
                    "invalid_input" | "invalid_json" => {
                        PagePersistError::InvalidResponse(error.message)
                    }
                    _ => PagePersistError::Storage(error.message),
                },
            )?;
        }
        let checkpointed = reconciliation::checkpoint_page_in_transaction(
            &conn,
            ReconciliationPageInput {
                run: run_ref(run),
                phase: run.phase,
                cursor_start: i64::from(start),
                next_cursor_start: i64::from(next_start),
                remote_total,
                seen,
            },
            target_version,
        )
        .map_err(|error| match error.code.as_str() {
            "stale_connection" => PagePersistError::ConnectionChanged(error.message),
            "stale_total" | "stale_version" => PagePersistError::SnapshotChanged(error.message),
            "invalid_input" => PagePersistError::InvalidResponse(error.message),
            _ => PagePersistError::Storage(error.message),
        })?;
        processing_repository::save_checkpoint(
            &conn,
            &task.task_id,
            task.lease_epoch,
            checkpoint,
            observed_at,
        )
        .map_err(|error| {
            if error.starts_with("lease_lost") {
                PagePersistError::LeaseLost(error)
            } else if error.starts_with("demand_lost") {
                PagePersistError::DemandLost(error)
            } else {
                PagePersistError::Storage(error)
            }
        })?;
        Ok(checkpointed)
    })();
    match persisted {
        Ok(checkpointed) => {
            conn.execute_batch("COMMIT").map_err(|error| {
                PagePersistError::Storage(format!("Failed to commit bibliography page: {error}"))
            })?;
            Ok(checkpointed)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Scheduler publisher for the explicit bibliography output. This function
/// never accepts a corpus subject and never opens or commits a transaction.
pub(crate) fn publish_bibliography_output(
    conn: &rusqlite::Connection,
    task: &ClaimedTask,
    output: &BibliographyComputeOutput,
) -> Result<(), String> {
    if task.domain != "bibliography"
        || task.subject_kind != "library"
        || task.kind != "bibliography_sync"
        || task.subject_id != output.library_row_id
    {
        return Err(format!(
            "unsupported_subject: task {} cannot publish bibliography output for {}",
            task.task_id, output.library_row_id
        ));
    }
    let completed = reconciliation::finalize_items_run_in_transaction(conn, output.run_ref())
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    // E3b-WU3: chained profile demand lands in the same success
    // transaction — a committed sync always carries its reindex follow-up.
    let _ = processing_repository::admit_stale_profile_demands(conn, &output.library_row_id)?;
    // E4a-WU3: chained extraction demand for readable files, same
    // transaction and same durability.
    let _ = processing_repository::admit_stale_extraction_demands(conn, &output.library_row_id)?;
    if completed.cursor_start != output.items_seen
        || completed.remote_total != output.remote_total
        || completed.target_version != output.target_version
    {
        return Err(
            "stale_output: bibliography output no longer matches its reconciliation run"
                .to_string(),
        );
    }
    Ok(())
}

/// The E2b-3 bibliographic sync engine: resolves the claimed subject, resumes
/// its fenced reconciliation and persists one complete page at a time.
pub struct BibliographySyncExecutor {
    source: Arc<dyn ZoteroPageSource>,
    page_limit: u32,
}

impl BibliographySyncExecutor {
    /// An executor over an injected page source (tests and E2b-3 wiring).
    pub fn new(source: Arc<dyn ZoteroPageSource>) -> Self {
        Self {
            source,
            page_limit: connector::MAX_LIMIT,
        }
    }

    /// Tunable page size (clamped to the API's range) so tests walk several
    /// pages cheaply; production keeps the connector default.
    pub fn with_page_limit(mut self, limit: u32) -> Self {
        self.page_limit = limit.clamp(1, connector::MAX_LIMIT);
        self
    }

    /// The production engine over the local Zotero API. E2b-3 registers this
    /// beside OCR and embeddings because page persistence and final publication
    /// are now both durable.
    pub fn production() -> Self {
        Self::new(Arc::new(LocalZoteroPageSource::new()))
    }

    fn fatal(&self, code: &str, message: String) -> ExecResult {
        ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: None,
            output: ExecOutput::Fatal {
                code: code.to_string(),
                message,
            },
        }
    }

    fn durable_verdict(
        &self,
        ctx: &ExecCtx,
        run: &ReconciliationRun,
        checkpoints: Vec<NewCheckpoint>,
        progress_total: Option<i64>,
        output: ExecOutput,
    ) -> ExecResult {
        self.persist_verdict(ctx, run, checkpoints, progress_total, output, false)
    }

    /// A consistency fence changed while walking pages. The processing task
    /// remains retryable, but this reconciliation is terminal so convergence
    /// starts the next attempt from zero instead of looping on a stale cursor.
    fn restart_verdict(
        &self,
        ctx: &ExecCtx,
        run: &ReconciliationRun,
        checkpoints: Vec<NewCheckpoint>,
        progress_total: Option<i64>,
        message: String,
    ) -> ExecResult {
        self.persist_verdict(
            ctx,
            run,
            checkpoints,
            progress_total,
            ExecOutput::Retryable {
                code: "zotero_snapshot_changed".to_string(),
                message,
            },
            true,
        )
    }

    fn persist_verdict(
        &self,
        ctx: &ExecCtx,
        run: &ReconciliationRun,
        checkpoints: Vec<NewCheckpoint>,
        progress_total: Option<i64>,
        output: ExecOutput,
        retire_retryable_run: bool,
    ) -> ExecResult {
        let mut conn = match open_archive_connection(&ctx.db_path) {
            Ok(conn) => conn,
            Err(error) => {
                return ExecResult {
                    checkpoints,
                    progress_total,
                    engine_output: None,
                    output: ExecOutput::Fatal {
                        code: "storage_unavailable".to_string(),
                        message: error,
                    },
                };
            }
        };
        let transition = match &output {
            ExecOutput::Retryable { code, message } => reconciliation::record_error(
                &mut conn,
                ReconciliationErrorInput {
                    run: run_ref(run),
                    expected_revision: run.revision,
                    phase: run.phase,
                    code: code.clone(),
                    message: message.clone(),
                    retryable: !retire_retryable_run,
                    next_retry_at: None,
                },
            ),
            ExecOutput::Fatal { code, message } => reconciliation::record_error(
                &mut conn,
                ReconciliationErrorInput {
                    run: run_ref(run),
                    expected_revision: run.revision,
                    phase: run.phase,
                    code: code.clone(),
                    message: message.clone(),
                    retryable: false,
                    next_retry_at: None,
                },
            ),
            ExecOutput::Blocked { .. } => reconciliation::mark_blocked(&mut conn, run_ref(run)),
            ExecOutput::Stopped => reconciliation::mark_interrupted(&mut conn, run_ref(run)),
            ExecOutput::Success { .. } => Ok(run.clone()),
        };
        match transition {
            Ok(_) => ExecResult {
                checkpoints,
                progress_total,
                engine_output: None,
                output,
            },
            Err(error) => ExecResult {
                checkpoints,
                progress_total,
                engine_output: None,
                output: ExecOutput::Fatal {
                    code: "storage_unavailable".to_string(),
                    message: format!(
                        "failed to persist reconciliation verdict: {}: {}",
                        error.code, error.message
                    ),
                },
            },
        }
    }

    fn converge_reconciliation(
        &self,
        ctx: &ExecCtx,
        task: &ClaimedTask,
        connection_revision: i64,
    ) -> Result<ReconciliationRun, String> {
        let mut conn = open_archive_connection(&ctx.db_path)?;
        reconciliation::converge_run(
            &mut conn,
            BeginReconciliationInput {
                library_id: task.subject_id.clone(),
                connection_revision,
                cursor_start: 0,
                cursor_limit: i64::from(self.page_limit),
                remote_total: None,
                target_version: None,
            },
        )
        .map_err(|error| format!("{}: {}", error.code, error.message))
    }
}

impl Executor for BibliographySyncExecutor {
    fn kinds(&self) -> &[&str] {
        &["bibliography_sync"]
    }

    fn run(&self, ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
        // Defensive depth: the claim gates this, but an engine handed a
        // foreign subject says so instead of interpreting it.
        if task.domain != "bibliography"
            || task.subject_kind != "library"
            || task.kind != "bibliography_sync"
        {
            return self.fatal(
                "unsupported_subject",
                format!(
                    "task {} domain='{}' subject_kind='{}' kind='{}' is not a bibliography library sync",
                    task.task_id, task.domain, task.subject_kind, task.kind
                ),
            );
        }
        if task.contract_hash != BIBLIOGRAPHY_SYNC_CONTRACT {
            return self.fatal(
                "configuration_changed",
                format!(
                    "task {} pins contract {} but this engine runs {}",
                    task.task_id, task.contract_hash, BIBLIOGRAPHY_SYNC_CONTRACT
                ),
            );
        }
        let (library, connection_revision) =
            match resolve_library_subject(&ctx.db_path, &task.subject_id) {
                Ok(LibrarySubject::Present {
                    library,
                    connection_revision,
                }) => (library, connection_revision),
                Ok(LibrarySubject::Missing) => {
                    return self.fatal(
                        "library_missing",
                        format!(
                            "library {} no longer exists in the bibliography catalog",
                            task.subject_id
                        ),
                    )
                }
                Ok(LibrarySubject::Invalid(detail)) => {
                    return self.fatal("library_identity_invalid", detail)
                }
                Err(error) => return self.fatal("storage_unavailable", error),
            };
        let mut run = match self.converge_reconciliation(ctx, task, connection_revision) {
            Ok(run) => run,
            Err(error) => return self.fatal("reconciliation_unavailable", error),
        };
        let page_limit = match u32::try_from(run.cursor_limit) {
            Ok(limit) if limit > 0 => limit.min(connector::MAX_LIMIT),
            _ => {
                return self.fatal(
                    "reconciliation_invalid",
                    format!("run {} stores an invalid page limit", run.run_id),
                )
            }
        };
        let mut start = match u32::try_from(run.cursor_start) {
            Ok(start) => start,
            Err(_) => {
                return self.fatal(
                    "reconciliation_invalid",
                    format!("run {} stores an out-of-range cursor", run.run_id),
                )
            }
        };
        let mut checkpoints: Vec<NewCheckpoint> = Vec::new();
        let mut progress_total: Option<i64> = run.remote_total;
        loop {
            // Observed before every request: once stopped, no next request.
            if stop.stopped() {
                return self.durable_verdict(
                    ctx,
                    &run,
                    checkpoints,
                    progress_total,
                    ExecOutput::Stopped,
                );
            }
            let query = BibliographyPageQuery::new(start, page_limit);
            // One page in flight, resolved on this thread: an in-flight
            // request is never preempted, only the next one is withheld.
            let page = match tauri::async_runtime::block_on(self.source.fetch_page(&library, query))
            {
                Ok(page) => page,
                Err(state) => {
                    return self.durable_verdict(
                        ctx,
                        &run,
                        checkpoints,
                        progress_total,
                        endpoint_verdict(&state),
                    );
                }
            };
            if empty_while_total_remains(page.items.len(), start, page.total) {
                return self.durable_verdict(
                    ctx,
                    &run,
                    checkpoints,
                    progress_total,
                    ExecOutput::Retryable {
                        code: "zotero_invalid_response".to_string(),
                        message: format!(
                            "the library's page at start {start} answered no items while its total says works remain"
                        ),
                    },
                );
            }
            if page.items.len() > page_limit as usize {
                return self.durable_verdict(
                    ctx,
                    &run,
                    checkpoints,
                    progress_total,
                    ExecOutput::Retryable {
                        code: "zotero_invalid_response".to_string(),
                        message: format!(
                            "the library's page at start {start} answered {} items, more than the {} asked for",
                            page.items.len(),
                            page_limit
                        ),
                    },
                );
            }
            let page_end = u64::from(start).saturating_add(page.items.len() as u64);
            if page.total.is_some_and(|total| page_end > total) {
                return self.durable_verdict(
                    ctx,
                    &run,
                    checkpoints,
                    progress_total,
                    ExecOutput::Retryable {
                        code: "zotero_invalid_response".to_string(),
                        message: format!(
                            "the library's page ending at {page_end} exceeds its reported total"
                        ),
                    },
                );
            }
            let next_page = pagination_next(start, page.items.len(), page_limit, page.total);
            let next_start = start.saturating_add(page.items.len() as u32);
            let terminal = next_page.is_none();
            let payload = match serde_json::to_string(&PageCheckpoint {
                start,
                next_start,
                total: page.total,
                terminal,
                items: page.items.clone(),
                library_version: page.library_version,
            }) {
                Ok(payload) => payload,
                Err(error) => {
                    return self.durable_verdict(
                        ctx,
                        &run,
                        checkpoints,
                        progress_total,
                        ExecOutput::Fatal {
                            code: "storage_unavailable".to_string(),
                            message: format!("failed to serialize bibliography page: {error}"),
                        },
                    )
                }
            };
            let checkpoint = NewCheckpoint {
                unit_key: format!("page:{start}"),
                input_fingerprint: task.input_fingerprint.clone(),
                contract_hash: task.contract_hash.clone(),
                payload_checksum: format!("{:x}", Sha256::digest(payload.as_bytes())),
                payload,
            };
            match persist_page(ctx, task, &run, &checkpoint, start, next_start, &page) {
                Ok(checkpointed) => {
                    run = checkpointed;
                    progress_total = run.remote_total;
                }
                Err(PagePersistError::InvalidResponse(message)) => {
                    return self.durable_verdict(
                        ctx,
                        &run,
                        checkpoints,
                        progress_total,
                        ExecOutput::Retryable {
                            code: "zotero_invalid_response".to_string(),
                            message,
                        },
                    )
                }
                Err(PagePersistError::SnapshotChanged(message)) => {
                    return self.restart_verdict(ctx, &run, checkpoints, progress_total, message)
                }
                Err(PagePersistError::ConnectionChanged(message)) => {
                    // The old run cannot be mutated through a fence that has
                    // already changed. Keep it as evidence and retry the task;
                    // convergence retires it under the new current fence.
                    return ExecResult {
                        checkpoints,
                        progress_total,
                        engine_output: None,
                        output: ExecOutput::Retryable {
                            code: "zotero_snapshot_changed".to_string(),
                            message,
                        },
                    };
                }
                Err(PagePersistError::LeaseLost(_message)) => {
                    return self.durable_verdict(
                        ctx,
                        &run,
                        checkpoints,
                        progress_total,
                        ExecOutput::Stopped,
                    )
                }
                Err(PagePersistError::DemandLost(_message)) => {
                    return self.durable_verdict(
                        ctx,
                        &run,
                        checkpoints,
                        progress_total,
                        ExecOutput::Stopped,
                    )
                }
                Err(PagePersistError::Storage(message)) => {
                    return self.durable_verdict(
                        ctx,
                        &run,
                        checkpoints,
                        progress_total,
                        ExecOutput::Fatal {
                            code: "storage_unavailable".to_string(),
                            message,
                        },
                    )
                }
            }
            checkpoints.push(checkpoint);
            if terminal {
                break;
            }
            start = next_page.expect("non-terminal page has a next cursor");
        }
        // Observed after the last page too: a run whose demand vanished at
        // the end reports Stopped, not a completion verdict.
        if stop.stopped() {
            return self.durable_verdict(
                ctx,
                &run,
                checkpoints,
                progress_total,
                ExecOutput::Stopped,
            );
        }
        let output = BibliographyComputeOutput {
            library_row_id: run.library_id.clone(),
            run_id: run.run_id.clone(),
            connection_revision: run.connection_revision,
            items_seen: run.cursor_start,
            remote_total: run.remote_total,
            target_version: run.target_version,
        };
        let receipt = match serde_json::to_string(&output) {
            Ok(receipt) => receipt,
            Err(error) => {
                return self.durable_verdict(
                    ctx,
                    &run,
                    checkpoints,
                    progress_total,
                    ExecOutput::Fatal {
                        code: "storage_unavailable".to_string(),
                        message: format!("failed to serialize bibliography receipt: {error}"),
                    },
                )
            }
        };
        ExecResult {
            checkpoints,
            progress_total,
            engine_output: Some(EngineOutput::Bibliography(output)),
            output: ExecOutput::Success {
                outcome: "bibliography_synced".to_string(),
                receipt,
            },
        }
    }
}

/// Maps one observed endpoint state to its honest verdict. Endpoint
/// unreachable and timeout are retryable; a disabled local API is blocked
/// (a human toggles a setting); a definitive 404 for this library is fatal;
/// anything unreadable is retryable, never an empty library.
fn endpoint_verdict(state: &ZoteroState) -> ExecOutput {
    match state {
        // Never an error in practice; if a source reported it on an error
        // path, that contradiction is itself unreadable — never a success.
        ZoteroState::Available => ExecOutput::Retryable {
            code: "zotero_invalid_response".to_string(),
            message: "the page source reported success on an error path".to_string(),
        },
        ZoteroState::EndpointUnavailable => ExecOutput::Retryable {
            code: "zotero_unreachable".to_string(),
            message: "nothing answered at Zotero's local endpoint".to_string(),
        },
        ZoteroState::Timeout => ExecOutput::Retryable {
            code: "zotero_timeout".to_string(),
            message: "Zotero's local endpoint answered too slowly".to_string(),
        },
        ZoteroState::ApiDisabled => ExecOutput::Blocked {
            code: "zotero_api_disabled".to_string(),
            message:
                "Zotero's local API refused the read; enable it in Zotero's settings and resume"
                    .to_string(),
        },
        ZoteroState::NotFound => ExecOutput::Fatal {
            code: "zotero_library_not_found".to_string(),
            message: "Zotero's local endpoint answered that this library does not exist there"
                .to_string(),
        },
        ZoteroState::InvalidResponse { detail } => ExecOutput::Retryable {
            code: "zotero_invalid_response".to_string(),
            message: format!("Zotero's local endpoint answered something unreadable: {detail}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, version: u64, csl: &str) -> serde_json::Value {
        serde_json::json!({ "key": key, "version": version, "csljson": csl })
    }

    /// A well-formed row carries key, version and one CSL object as text.
    #[test]
    fn a_page_parses_key_version_and_csl() {
        let body = serde_json::json!([
            row("AAAA1111", 12, r#"{"id":"AAAA1111","title":"Uno"}"#),
            row("BBBB2222", 40, r#"[{"id":"BBBB2222","title":"Dos"}]"#),
        ]);
        let items = page_items_from_json(&body).expect("strict parse");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "AAAA1111");
        assert_eq!(items[0].item_version, 12);
        assert_eq!(items[0].csl_json, r#"{"id":"AAAA1111","title":"Uno"}"#);
        assert!(items[0]
            .native_json_snapshot
            .contains("\"key\":\"AAAA1111\""));
        assert!(items[0].native_json_snapshot.contains("\"csljson\""));
        // The CSL text arrives as a one-element array and is stored as the
        // single object — the same text a live `format=csljson` read gives.
        assert_eq!(items[1].csl_json, r#"{"id":"BBBB2222","title":"Dos"}"#);
    }

    /// E2b-3's catalog upsert needs the lossless native Zotero JSON: every
    /// item carries the serialized ORIGINAL row of the
    /// `/items/top?format=json&include=csljson` answer — including fields
    /// the executor never interprets (`data`, `library`, `links`, `meta`) —
    /// not a reconstructed subset.
    #[test]
    fn a_page_row_keeps_its_lossless_native_snapshot() {
        let original = serde_json::json!({
            "key": "AAAA1111",
            "version": 12,
            "csljson": r#"{"id":"AAAA1111","title":"Uno"}"#,
            "data": { "itemType": "book", "title": "Uno", "creators": [] },
            "library": { "type": "user", "id": 0, "name": "My Library" },
            "links": { "self": { "href": "http://localhost:23119/api/users/0/items/AAAA1111" } },
            "meta": { "numChildren": 2 }
        });
        let items =
            page_items_from_json(&serde_json::json!([original.clone()])).expect("strict parse");
        assert_eq!(items.len(), 1);
        // The snapshot is a serialized JSON object...
        let snapshot: serde_json::Value =
            serde_json::from_str(&items[0].native_json_snapshot).expect("snapshot is JSON");
        assert!(snapshot.is_object(), "the snapshot must be an object");
        // ...that equals the original row value in full, interpreted fields
        // (key/version/csl) and uninterpreted ones (data/library/links/meta)
        // alike.
        assert_eq!(snapshot, original, "no field of the native row may be lost");
    }

    /// A row that is not an object cannot be snapshotted losslessly and is
    /// malformed, exactly like a row missing its key, version or CSL.
    #[test]
    fn a_non_object_row_is_malformed() {
        for body in [
            serde_json::json!([42]),
            serde_json::json!([["nested"]]),
            serde_json::json!(["plain string"]),
        ] {
            assert!(
                page_items_from_json(&body).is_err(),
                "a non-object row must refuse the page: {body}"
            );
        }
    }

    /// Anything else is a malformed page, not a smaller library: a
    /// non-list body, or any row missing its key, version or CSL, refuses
    /// the whole page.
    #[test]
    fn a_malformed_page_refuses_rather_than_shrinking_the_library() {
        let cases = [
            serde_json::json!({ "AAAA1111": 12 }),
            serde_json::json!([row("", 12, r#"{"id":"x"}"#)]),
            serde_json::json!([{ "key": "AAAA1111", "version": "twelve", "csljson": r#"{"id":"x"}"# }]),
            serde_json::json!([row("AAAA1111", 12, "not json")]),
            serde_json::json!([row("AAAA1111", 12, r#"[{},{}]"#)]),
            serde_json::json!([{ "version": 12, "csljson": r#"{"id":"x"}"# }]),
        ];
        for body in cases {
            assert!(
                page_items_from_json(&body).is_err(),
                "malformed page must refuse: {body}"
            );
        }
    }

    /// An empty array is a real answer ("no works"), distinct from every
    /// malformed shape above.
    #[test]
    fn an_empty_page_is_a_valid_answer() {
        let items = page_items_from_json(&serde_json::json!([])).expect("empty is valid");
        assert!(items.is_empty());
    }

    #[test]
    fn a_query_is_always_bounded() {
        assert_eq!(BibliographyPageQuery::new(0, 0).limit(), 1);
        assert_eq!(
            BibliographyPageQuery::new(0, connector::MAX_LIMIT * 10).limit(),
            connector::MAX_LIMIT
        );
        assert_eq!(BibliographyPageQuery::new(40, 5).start(), 40);
    }

    /// The walk advances by what was read and stops exactly when the
    /// library is exhausted, with or without a count to plan against.
    #[test]
    fn pagination_advances_by_what_was_read() {
        // Full page, total says more: continue at the next start.
        assert_eq!(pagination_next(0, 2, 2, Some(6)), Some(2));
        assert_eq!(pagination_next(2, 2, 2, Some(6)), Some(4));
        // Last page by count: stop.
        assert_eq!(pagination_next(4, 2, 2, Some(6)), None);
        // Short page without a count: the library is done.
        assert_eq!(pagination_next(4, 1, 2, None), None);
        // Full page without a count: one more page might exist.
        assert_eq!(pagination_next(0, 2, 2, None), Some(2));
        // A row vanished mid-walk (page came back short while the total
        // still counts it): the next start still moves past what was read,
        // never past what exists.
        assert_eq!(pagination_next(2, 1, 2, Some(6)), Some(3));
        // An empty page always ends the walk; its honesty is guarded
        // separately against contradicting totals.
        assert_eq!(pagination_next(2, 0, 2, Some(6)), None);
    }

    #[test]
    fn an_empty_page_contradicting_its_total_is_flagged() {
        assert!(empty_while_total_remains(0, 0, Some(5)));
        assert!(!empty_while_total_remains(0, 0, Some(0)));
        assert!(!empty_while_total_remains(0, 0, None));
        assert!(!empty_while_total_remains(2, 2, Some(5)));
    }

    /// The verdict table is total and honest: every endpoint state maps to
    /// its own stable code and severity, and no message invents a Zotero
    /// installation state it cannot know.
    #[test]
    fn every_endpoint_state_maps_to_its_own_verdict() {
        let table = [
            (
                ZoteroState::EndpointUnavailable,
                "retryable",
                "zotero_unreachable",
            ),
            (ZoteroState::Timeout, "retryable", "zotero_timeout"),
            (ZoteroState::ApiDisabled, "blocked", "zotero_api_disabled"),
            (ZoteroState::NotFound, "fatal", "zotero_library_not_found"),
            (
                ZoteroState::InvalidResponse {
                    detail: "unreadable".into(),
                },
                "retryable",
                "zotero_invalid_response",
            ),
        ];
        for (state, severity, code) in table {
            let verdict = endpoint_verdict(&state);
            let (actual_code, message) = match &verdict {
                ExecOutput::Retryable { code, message }
                | ExecOutput::Fatal { code, message }
                | ExecOutput::Blocked { code, message } => (code.as_str(), message.clone()),
                other => panic!("state verdict expected, got {other:?}"),
            };
            assert_eq!(actual_code, code, "{state:?}");
            match severity {
                "retryable" => assert!(matches!(verdict, ExecOutput::Retryable { .. })),
                "blocked" => assert!(matches!(verdict, ExecOutput::Blocked { .. })),
                "fatal" => assert!(matches!(verdict, ExecOutput::Fatal { .. })),
                _ => {}
            }
            let lower = message.to_lowercase();
            assert!(
                !lower.contains("closed") && !lower.contains("not installed"),
                "no invented installation state: {message}"
            );
        }
    }

    /// The production source addresses the local API through the writing
    /// connector's conventions — bounded `/items/top` entries pages — which
    /// is asserted here without any network: same builder, same shape.
    #[test]
    fn the_production_source_uses_the_connector_entries_convention() {
        let library = Library::user("0");
        let url = connector::entries_url(
            &library,
            ConnectorPage::new(BibliographyPageQuery::new(200, 100).start(), 100),
        );
        assert!(url.contains("/api/users/0/items/top?"), "{url}");
        assert!(url.contains("format=json&include=csljson"), "{url}");
        assert!(url.contains("limit=100"), "{url}");
        assert!(url.contains("start=200"), "{url}");
    }
}

// ── Per-work profile engine (E3b-WU2) ──────────────────────────────────────

use crate::bibliography::profile::BIBLIOGRAPHY_PROFILE_TEMPLATE_V1;
use crate::processing::eligibility::resolve_effective_embedding_contract;
use crate::processing::embedding::map_embedding_error;
use std::sync::Mutex;

/// Explicit profile output routed by the scheduler. Profile and vector are
/// published together inside the task-success transaction; the receipt only
/// describes identity, the rows are the product.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyProfileComputeOutput {
    pub item_id: String,
    /// The staging generation the vector was computed for. The publisher
    /// refuses the commit when this generation stopped being staging.
    pub generation_id: String,
    pub template_version: String,
    pub canonical_text: String,
    pub input_hash: String,
    pub field_provenance_json: String,
    pub profile_revision: i64,
    pub model: String,
    pub contract: String,
    pub dimensions: usize,
    /// Little-endian f32 bytes of the profile vector.
    pub embedding: Vec<u8>,
}

/// Embedding boundary for the profile engine. Production resolves the
/// settings-driven BGE-M3 engine; tests inject fakes so the durable path is
/// verifiable without a network or model files.
pub trait ProfileEmbedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
    /// `(model, contract, dimensions)` the vector is computed under — the
    /// effective contract resolved from settings, never hardcoded constants.
    fn identity(&self) -> Result<(String, String, usize), String>;
}

/// Production embedder over the shared, lazily-initialized engine.
pub struct EngineProfileEmbedder {
    db_path: std::path::PathBuf,
    cache: Mutex<crate::processing::embedding::EngineCache>,
}

impl EngineProfileEmbedder {
    pub fn new(db_path: std::path::PathBuf) -> Self {
        Self {
            db_path,
            cache: Mutex::new(crate::processing::embedding::EngineCache {
                cached: None,
                last_error: None,
            }),
        }
    }
}

impl ProfileEmbedder for EngineProfileEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let conn = open_archive_connection(&self.db_path)?;
        let mut guard = self
            .cache
            .lock()
            .map_err(|e| format!("Embedding engine lock poisoned: {e}"))?;
        let crate::processing::embedding::EngineCache { cached, last_error } = &mut *guard;
        crate::nlp::ensure_embed_engine_for_current_settings(&conn, cached, last_error)
            .ok_or_else(|| {
                crate::nlp::embeddings::embedding_engine_unavailable_reason(last_error.as_deref())
            })?
            .embed_text(text)
    }

    fn identity(&self) -> Result<(String, String, usize), String> {
        let conn = open_archive_connection(&self.db_path)?;
        let contract = resolve_effective_embedding_contract(&conn)?;
        Ok((contract.model, contract.contract, contract.dimensions))
    }
}

/// The E3b-WU2 profile engine: one canonical text per verified work, one
/// vector, published atomically with the profile row at commit. Works
/// without attachments profile identically — the catalog row is the only
/// input.
pub struct BibliographyProfileExecutor {
    embedder: Arc<dyn ProfileEmbedder>,
}

impl BibliographyProfileExecutor {
    pub fn new(embedder: Arc<dyn ProfileEmbedder>) -> Self {
        Self { embedder }
    }

    /// Stages one profile output or an honest verdict. Subject identity,
    /// contract currency, catalog presence, stop boundaries, and vector
    /// validation all gate before anything is staged.
    fn stage(
        &self,
        ctx: &crate::processing::scheduler::ExecCtx,
        task: &crate::processing::scheduler::ClaimedTask,
        stop: &crate::processing::scheduler::StopFlag,
    ) -> Result<crate::processing::scheduler::ExecResult, ExecOutput> {
        use crate::processing::scheduler::{EngineOutput, ExecResult};
        if task.domain != "bibliography"
            || task.subject_kind != "item"
            || task.kind != "bibliography_profile"
        {
            return Err(ExecOutput::Fatal {
                code: "unsupported_subject".to_string(),
                message: format!(
                    "task {} domain='{}' subject_kind='{}' kind='{}' is not a bibliography work profile",
                    task.task_id, task.domain, task.subject_kind, task.kind
                ),
            });
        }
        let conn = open_archive_connection(&ctx.db_path).map_err(|error| ExecOutput::Fatal {
            code: "storage_unavailable".to_string(),
            message: error,
        })?;
        let effective =
            resolve_effective_embedding_contract(&conn).map_err(|error| ExecOutput::Blocked {
                code: "configuration_required".to_string(),
                message: error,
            })?;
        if task.contract_hash != effective.hash {
            return Err(ExecOutput::Blocked {
                code: "configuration_required".to_string(),
                message:
                    "the effective embedding contract changed; resume with the current configuration to re-evaluate"
                        .to_string(),
            });
        }
        let input = crate::bibliography::profile::profile_input_for_item(&conn, &task.subject_id)
            .map_err(|error| ExecOutput::Fatal {
                code: "storage_unavailable".to_string(),
                message: format!("{}: {}", error.code, error.message),
            })?
            .ok_or_else(|| ExecOutput::Fatal {
                code: "item_missing".to_string(),
                message: format!(
                    "work {} no longer exists in the verified catalog",
                    task.subject_id
                ),
            })?;
        let generation = crate::bibliography::generation::ensure_staging_generation_for_contract(
            &conn,
            &crate::bibliography::generation::EmbeddingContractRow {
                contract_hash: effective.hash.clone(),
                provider: effective.provider.clone(),
                model: effective.model.clone(),
                dimensions: effective.dimensions as i64,
                chunking_contract: crate::nlp::embeddings::RAG_CHUNKING_CONTRACT_V1.to_string(),
            },
            crate::processing::repository::now_ms(),
        )
        .map_err(|error| ExecOutput::Fatal {
            code: "storage_unavailable".to_string(),
            message: format!("{}: {}", error.code, error.message),
        })?;
        let built = crate::bibliography::profile::build_profile(&input);
        drop(conn);
        if stop.stopped() {
            return Err(ExecOutput::Stopped);
        }
        let (model, contract, dimensions) =
            self.embedder
                .identity()
                .map_err(|error| match map_embedding_error(&error) {
                    ExecOutput::Fatal { code, message } => ExecOutput::Fatal { code, message },
                    other => other,
                })?;
        let vector = ctx
            .unit(task, "profile", || {
                let vector = self.embedder.embed(&built.canonical_text)?;
                if vector.len() != dimensions || vector.iter().any(|value| !value.is_finite()) {
                    return Err(format!(
                        "Profile embedding does not satisfy {dimensions} finite dimensions"
                    ));
                }
                Ok(vector)
            })
            .map_err(|error| match map_embedding_error(&error) {
                ExecOutput::Fatal { code, message } => ExecOutput::Fatal { code, message },
                other => other,
            })?;
        if stop.stopped() {
            return Err(ExecOutput::Stopped);
        }
        // The publisher assigns the durable revision inside the commit
        // transaction; the staged value is only advisory.
        let conn = open_archive_connection(&ctx.db_path).map_err(|error| ExecOutput::Fatal {
            code: "storage_unavailable".to_string(),
            message: error,
        })?;
        let profile_revision =
            crate::bibliography::repository::get_semantic_profile(&conn, &task.subject_id)
                .map(|profile| profile.map(|profile| profile.profile_revision).unwrap_or(0) + 1)
                .map_err(|error| ExecOutput::Fatal {
                    code: "storage_unavailable".to_string(),
                    message: format!("{}: {}", error.code, error.message),
                })?;
        let field_provenance_json = serde_json::to_string(
            &built
                .field_provenance
                .iter()
                .map(|(field, line)| serde_json::json!({ "field": field, "line": line }))
                .collect::<Vec<_>>(),
        )
        .map_err(|error| ExecOutput::Fatal {
            code: "storage_unavailable".to_string(),
            message: format!("failed to serialize profile provenance: {error}"),
        })?;
        let output = BibliographyProfileComputeOutput {
            item_id: task.subject_id.clone(),
            generation_id: generation.id.clone(),
            template_version: BIBLIOGRAPHY_PROFILE_TEMPLATE_V1.to_string(),
            canonical_text: built.canonical_text,
            input_hash: built.input_hash,
            field_provenance_json,
            profile_revision,
            model,
            contract,
            dimensions,
            embedding: crate::nlp::embeddings::floats_to_blob(&vector),
        };
        let receipt = serde_json::json!({
            "itemId": output.item_id,
            "inputHash": output.input_hash,
            "contract": output.contract,
            "model": output.model,
            "generationId": output.generation_id,
        })
        .to_string();
        Ok(ExecResult {
            checkpoints: Vec::new(),
            progress_total: Some(1),
            engine_output: Some(EngineOutput::BibliographyProfile(output)),
            output: ExecOutput::Success {
                outcome: "bibliography_profiled".to_string(),
                receipt,
            },
        })
    }
}

impl crate::processing::scheduler::Executor for BibliographyProfileExecutor {
    fn kinds(&self) -> &[&str] {
        &["bibliography_profile"]
    }

    fn run(
        &self,
        ctx: &crate::processing::scheduler::ExecCtx,
        task: &crate::processing::scheduler::ClaimedTask,
        stop: &crate::processing::scheduler::StopFlag,
    ) -> crate::processing::scheduler::ExecResult {
        match self.stage(ctx, task, stop) {
            Ok(result) => result,
            Err(output) => crate::processing::scheduler::ExecResult {
                checkpoints: Vec::new(),
                progress_total: None,
                engine_output: None,
                output,
            },
        }
    }
}

/// Scheduler publisher for one work profile. Runs inside the commit
/// transaction: profile row and its vector land together, or nothing does.
pub fn publish_bibliography_profile_output(
    conn: &rusqlite::Connection,
    task: &crate::processing::scheduler::ClaimedTask,
    output: &BibliographyProfileComputeOutput,
) -> Result<(), String> {
    if task.domain != "bibliography"
        || task.subject_kind != "item"
        || task.kind != "bibliography_profile"
        || task.subject_id != output.item_id
    {
        return Err(format!(
            "unsupported_subject: task {} cannot publish a profile for {}",
            task.task_id, output.item_id
        ));
    }
    // The generation must still be staging for this contract: a switch or
    // retire between run and commit requeues the work instead of writing
    // into a dead generation.
    let generation = crate::bibliography::generation::read_generation(conn, &output.generation_id)
        .map_err(|error| format!("{}: {}", error.code, error.message))?
        .ok_or_else(|| {
            format!(
                "configuration_changed: generation {} no longer exists",
                output.generation_id
            )
        })?;
    // The row must belong to this task pinned space: output stamps stay
    // the executor responsibility (production resolves them from the same
    // settings; doubles assert their own stamps on readback).
    if generation.status != "staging" || generation.contract_hash != task.contract_hash {
        return Err(format!(
            "configuration_changed: generation {} is {} and cannot take publishes for {} tasks",
            output.generation_id, generation.status, task.kind
        ));
    }
    let is_new = !crate::bibliography::generation::generation_has_item(
        conn,
        &output.generation_id,
        &output.item_id,
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let revision = crate::bibliography::repository::upsert_semantic_profile_in_transaction(
        conn,
        &output.item_id,
        &output.template_version,
        &output.canonical_text,
        &output.input_hash,
        &output.field_provenance_json,
        processing_repository::now_ms(),
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    crate::bibliography::repository::upsert_item_embedding_in_transaction(
        conn,
        &crate::bibliography::repository::ItemEmbeddingRow {
            item_id: output.item_id.clone(),
            generation_id: output.generation_id.clone(),
            embedding_contract: output.contract.clone(),
            embedding_model: output.model.clone(),
            dimensions: output.dimensions,
            embedding: output.embedding.clone(),
            input_hash: output.input_hash.clone(),
            profile_revision: revision,
        },
        processing_repository::now_ms(),
    )?;
    // Progress only grows when a new work lands: re-profiles update their
    // row in place and never inflate the manifest count.
    if is_new {
        crate::bibliography::generation::note_generation_progress(conn, &output.generation_id)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
    }
    // The manifest floors at the distinct published union: direct demand
    // outside a sync chain has no declared manifest, and monotonic MAX
    // keeps a chain-declared manifest from ever shrinking below reality.
    let distinct =
        crate::bibliography::generation::generation_distinct_published(conn, &output.generation_id)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
    crate::bibliography::generation::raise_generation_manifest(
        conn,
        &output.generation_id,
        distinct,
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    Ok(())
}

// ── Native extraction engine (E4a-WU2) ─────────────────────────────────────
//
// Reads one resolved attachment file and extracts its native PDF text layer
// through the `ocr::pdf` text primitive — never through the OCR executor,
// never minting corpus assets. Quality verdicts (rich/sparse/empty) tell
// E4b's selective OCR exactly where native text runs out.

use crate::processing::repository::BIBLIOGRAPHY_EXTRACT_CONTRACT;

/// Setting key for the user-configured Zotero profile directory. Stored
/// copies resolve under `<dir>/storage/<key>/<filename>`; no UI binds it
/// yet, so an unset key simply makes stored copies unavailable.
pub const ZOTERO_DATA_DIR_SETTING_KEY: &str = "zotero_data_dir";

/// Refusal size for attachment reads: a file above this is not a text
/// extraction job but a storage problem the user must solve first.
pub const BIBLIOGRAPHY_EXTRACT_MAX_BYTES: u64 = 200 * 1024 * 1024;

/// Per-page decompressed content bound for native page reads. A page above
/// it records Unreadable: the decoder refuses the bomb instead of
/// inflating memory without limit.
pub const BIBLIOGRAPHY_PAGE_CONTENT_LIMIT_BYTES: usize = 4 * 1024 * 1024;

/// One page's native text with its own hash and quality verdict.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractPageText {
    pub page_number: i64,
    pub text_content: String,
    pub text_hash: String,
    pub text_chars: i64,
    pub quality: String,
}

/// Explicit native extraction output routed by the scheduler. The rows are
/// the product; the receipt only describes identity.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyExtractComputeOutput {
    pub attachment_id: String,
    pub item_id: String,
    pub page_count: i64,
    pub text_content: String,
    pub text_hash: String,
    pub text_chars: i64,
    pub quality: String,
    pub source_mtime: Option<i64>,
    pub source_bytes: i64,
    /// One entry per document page, in page order. Pages no decoder
    /// could read carry `unreadable` with empty text.
    pub pages: Vec<ExtractPageText>,
}

fn extraction_text_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn extraction_quality(text: &str) -> &'static str {
    if text.trim().is_empty() {
        "empty"
    } else if crate::ocr::pdf::is_quality_text(text) {
        "rich"
    } else {
        "sparse"
    }
}

fn is_pdf_attachment(content_type: Option<&str>, filename: Option<&str>) -> bool {
    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.contains("pdf") {
        return true;
    }
    filename
        .unwrap_or_default()
        .to_ascii_lowercase()
        .ends_with(".pdf")
}

/// The E4a-WU2 native extraction engine: resolve, read, extract the text
/// layer, grade it. Every unresolvable state maps to an honest verdict —
/// blocked when the user can fix it (missing file, unconfigured data dir),
/// fatal when retrying could never help (unsupported MIME, corrupt file).
pub struct BibliographyExtractExecutor;

impl BibliographyExtractExecutor {
    pub fn new() -> Self {
        Self
    }

    fn stage(
        &self,
        ctx: &crate::processing::scheduler::ExecCtx,
        task: &crate::processing::scheduler::ClaimedTask,
        stop: &crate::processing::scheduler::StopFlag,
    ) -> Result<crate::processing::scheduler::ExecResult, crate::processing::scheduler::ExecOutput>
    {
        use crate::processing::scheduler::{EngineOutput, ExecOutput, ExecResult};
        if task.domain != "bibliography"
            || task.subject_kind != "attachment"
            || task.kind != "bibliography_extract"
        {
            return Err(ExecOutput::Fatal {
                code: "unsupported_subject".to_string(),
                message: format!(
                    "task {} domain='{}' subject_kind='{}' kind='{}' is not a bibliography attachment extraction",
                    task.task_id, task.domain, task.subject_kind, task.kind
                ),
            });
        }
        if task.contract_hash != BIBLIOGRAPHY_EXTRACT_CONTRACT {
            return Err(ExecOutput::Blocked {
                code: "configuration_required".to_string(),
                message:
                    "the bibliography extraction contract changed; resume with the current configuration to re-evaluate"
                        .to_string(),
            });
        }
        let conn = open_archive_connection(&ctx.db_path).map_err(|error| ExecOutput::Fatal {
            code: "storage_unavailable".to_string(),
            message: error,
        })?;
        let attachment =
            crate::bibliography::attachment::attachment_ref_for(&conn, &task.subject_id)
                .map_err(|error| ExecOutput::Fatal {
                    code: "storage_unavailable".to_string(),
                    message: error,
                })?
                .ok_or_else(|| ExecOutput::Fatal {
                    code: "attachment_missing".to_string(),
                    message: format!(
                        "attachment {} no longer exists in the verified catalog",
                        task.subject_id
                    ),
                })?;
        // Catalog reads first, on the short-lived connection: parent item,
        // configured data dir, then drop. Resolution comes next: for a web
        // link the primary truth is that no local file exists (blocked,
        // resumable), not its MIME type.
        let item_id: String = conn
            .query_row(
                "SELECT item_id FROM zotero_attachments WHERE id = ?1",
                [&task.subject_id],
                |row| row.get(0),
            )
            .map_err(|error| ExecOutput::Fatal {
                code: "storage_unavailable".to_string(),
                message: format!("Failed to read attachment parent: {error}"),
            })?;
        let data_dir = crate::settings::get_setting(&conn, ZOTERO_DATA_DIR_SETTING_KEY);
        drop(conn);
        if stop.stopped() {
            return Err(ExecOutput::Stopped);
        }
        let path = match crate::bibliography::attachment::resolve_attachment_file(
            &attachment,
            data_dir.as_deref(),
        ) {
            crate::bibliography::attachment::AttachmentResolution::File(path) => path,
            crate::bibliography::attachment::AttachmentResolution::Unavailable {
                reason,
                detail,
            } => {
                return Err(ExecOutput::Blocked {
                    code: "extraction_unavailable".to_string(),
                    message: format!("{reason}: {detail}"),
                });
            }
        };
        if !is_pdf_attachment(
            attachment.content_type.as_deref(),
            attachment.filename.as_deref(),
        ) {
            return Err(ExecOutput::Fatal {
                code: "extraction_unsupported".to_string(),
                message: format!(
                    "attachment {} is not a PDF ({}); native text extraction covers PDFs only",
                    task.subject_id,
                    attachment.content_type.as_deref().unwrap_or("unknown type"),
                ),
            });
        }
        // The size gate runs before any checkpoint: an oversized file is
        // a storage problem, never a retryable extraction.
        let metadata = std::fs::metadata(&path).map_err(|error| ExecOutput::Retryable {
            code: "extraction_io".to_string(),
            message: format!("Failed to stat attachment file {}: {error}", path.display()),
        })?;
        if metadata.len() > BIBLIOGRAPHY_EXTRACT_MAX_BYTES {
            return Err(ExecOutput::Fatal {
                code: "file_too_large".to_string(),
                message: format!(
                    "Attachment file {} is {} bytes, over the {}-byte extraction limit",
                    path.display(),
                    metadata.len(),
                    BIBLIOGRAPHY_EXTRACT_MAX_BYTES
                ),
            });
        }
        let bytes = ctx
            .unit(task, "extract", || {
                std::fs::read(&path).map_err(|error| {
                    format!("Failed to read attachment file {}: {error}", path.display())
                })
            })
            .map_err(|error| {
                if error.starts_with("lease_lost") || error.starts_with("demand_lost") {
                    ExecOutput::Stopped
                } else {
                    ExecOutput::Retryable {
                        code: "extraction_io".to_string(),
                        message: error,
                    }
                }
            })?;
        if stop.stopped() {
            return Err(ExecOutput::Stopped);
        }
        let page_count =
            crate::ocr::pdf::pdf_page_count(&bytes).map_err(|error| ExecOutput::Fatal {
                code: "extraction_failed".to_string(),
                message: error,
            })? as i64;
        let text =
            crate::ocr::pdf::extract_pdf_text(&bytes).map_err(|error| ExecOutput::Fatal {
                code: "extraction_failed".to_string(),
                message: error,
            })?;
        let quality = extraction_quality(&text);
        let text_chars = text.chars().count() as i64;
        let pages = read_native_page_texts(&bytes, page_count)?;
        let output = BibliographyExtractComputeOutput {
            attachment_id: task.subject_id.clone(),
            item_id,
            page_count,
            text_hash: extraction_text_hash(&text),
            text_chars,
            quality: quality.to_string(),
            text_content: text,
            pages,
            source_mtime: attachment.mtime,
            source_bytes: bytes.len() as i64,
        };
        let receipt = serde_json::json!({
            "attachmentId": output.attachment_id,
            "itemId": output.item_id,
            "quality": output.quality,
            "pageCount": output.page_count,
            "textHash": output.text_hash,
        })
        .to_string();
        Ok(ExecResult {
            checkpoints: Vec::new(),
            progress_total: Some(1),
            engine_output: Some(EngineOutput::BibliographyExtract(output)),
            output: ExecOutput::Success {
                outcome: "bibliography_extracted".to_string(),
                receipt,
            },
        })
    }
}

impl Default for BibliographyExtractExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::processing::scheduler::Executor for BibliographyExtractExecutor {
    fn kinds(&self) -> &[&str] {
        &["bibliography_extract"]
    }

    fn run(
        &self,
        ctx: &crate::processing::scheduler::ExecCtx,
        task: &crate::processing::scheduler::ClaimedTask,
        stop: &crate::processing::scheduler::StopFlag,
    ) -> crate::processing::scheduler::ExecResult {
        match self.stage(ctx, task, stop) {
            Ok(result) => result,
            Err(output) => crate::processing::scheduler::ExecResult {
                checkpoints: Vec::new(),
                progress_total: None,
                engine_output: None,
                output,
            },
        }
    }
}

/// Reads every page's native text through lopdf's bounded per-page decoder:
/// one entry per page in page order. A page no decoder can read records
/// `unreadable` with empty text instead of failing its siblings — the
/// whole-document text above already proved the file opens.
fn read_native_page_texts(
    bytes: &[u8],
    page_count: i64,
) -> Result<Vec<ExtractPageText>, crate::processing::scheduler::ExecOutput> {
    use crate::processing::scheduler::ExecOutput;
    let document = lopdf::Document::load_mem(bytes).map_err(|error| ExecOutput::Fatal {
        code: "extraction_failed".to_string(),
        message: format!("Failed to parse PDF for per-page text: {error}"),
    })?;
    let mut pages = Vec::with_capacity(page_count.max(0) as usize);
    for number in 1..=page_count.max(0) as u32 {
        let chunks = document
            .extract_text_chunks_with_limit(&[number], BIBLIOGRAPHY_PAGE_CONTENT_LIMIT_BYTES);
        let mut text = String::new();
        let mut readable = true;
        for chunk in chunks {
            match chunk {
                Ok(fragment) => text.push_str(&fragment),
                Err(_) => readable = false,
            }
        }
        let (quality, text_content) = if readable {
            let quality = extraction_quality(&text);
            (quality.to_string(), text)
        } else {
            ("unreadable".to_string(), String::new())
        };
        pages.push(ExtractPageText {
            page_number: number as i64,
            text_hash: extraction_text_hash(&text_content),
            text_chars: text_content.chars().count() as i64,
            quality,
            text_content,
        });
    }
    Ok(pages)
}

/// Scheduler publisher for one native extraction. Runs inside the commit
/// transaction: the row lands together with the task receipt, or nothing
/// does.
pub fn publish_bibliography_extract_output(
    conn: &rusqlite::Connection,
    task: &crate::processing::scheduler::ClaimedTask,
    output: &BibliographyExtractComputeOutput,
) -> Result<(), String> {
    if task.domain != "bibliography"
        || task.subject_kind != "attachment"
        || task.kind != "bibliography_extract"
        || task.subject_id != output.attachment_id
    {
        return Err(format!(
            "unsupported_subject: task {} cannot publish an extraction for {}",
            task.task_id, output.attachment_id
        ));
    }
    crate::bibliography::repository::upsert_extraction_in_transaction(
        conn,
        &crate::bibliography::repository::ExtractionRow {
            attachment_id: output.attachment_id.clone(),
            item_id: output.item_id.clone(),
            page_count: output.page_count,
            method: "native".to_string(),
            text_content: output.text_content.clone(),
            text_hash: output.text_hash.clone(),
            text_chars: output.text_chars,
            quality: output.quality.clone(),
            source_mtime: output.source_mtime,
            source_bytes: output.source_bytes,
        },
        processing_repository::now_ms(),
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    for page in &output.pages {
        crate::bibliography::repository::upsert_page_text_in_transaction(
            conn,
            &crate::bibliography::repository::PageTextRow {
                attachment_id: output.attachment_id.clone(),
                page_number: page.page_number,
                method: "native".to_string(),
                text_content: page.text_content.clone(),
                text_hash: page.text_hash.clone(),
                text_chars: page.text_chars,
                quality: page.quality.clone(),
            },
            processing_repository::now_ms(),
        )
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    }
    Ok(())
}
