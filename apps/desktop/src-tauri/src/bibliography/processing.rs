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

use crate::processing::compact::CompactVec;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::reconciliation::{
    self, BeginReconciliationInput, ReconciliationEntityKind, ReconciliationErrorInput,
    ReconciliationPageInput, ReconciliationRun, ReconciliationRunRef, ReconciliationSeenInput,
};
use super::repository::{self as catalog_repository, AttachmentInput, BibliographicItemInput};
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

/// One PDF attachment of a work as its page carries it: the work it belongs
/// to (`parent_key`, the work's Zotero key) and the catalog input to store.
#[derive(Debug, Clone)]
pub struct BibliographyAttachment {
    pub parent_key: String,
    pub input: AttachmentInput,
}

/// One answered page of a library's attachment items. `rows_read` counts
/// every row the page held (PDF or not, with a parent or not): pagination
/// advances by what was read, while `attachments` keeps only the PDF children
/// the catalog stores.
#[derive(Debug, Clone)]
pub struct BibliographyAttachmentPage {
    pub attachments: Vec<BibliographyAttachment>,
    pub rows_read: usize,
    pub total: Option<u64>,
}

pub type AttachmentPageFuture = Pin<
    Box<dyn std::future::Future<Output = Result<BibliographyAttachmentPage, ZoteroState>> + Send>,
>;

/// Converts a Zotero `file:` URL (`links.enclosure.href`) to a local path.
/// `file:///C:/a%20b.pdf` becomes `C:/a b.pdf`; `file:///home/a.pdf` keeps its
/// leading slash. Anything that is not a `file:` URL is not a local file.
fn file_url_to_path(href: &str) -> Option<String> {
    let rest = href.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let decoded = urlencoding::decode(rest).ok()?.into_owned();
    let bytes = decoded.as_bytes();
    let drive_letter = bytes.len() >= 3 && bytes[2] == b':' && bytes[1].is_ascii_alphabetic();
    Some(if drive_letter {
        decoded[1..].to_string()
    } else {
        decoded
    })
}

/// Parses one page of `/items?itemType=attachment&format=json` strictly: every
/// row must carry its key, version and data object, or the whole page is
/// [`ZoteroState::InvalidResponse`]. Only PDF and stored HTML snapshot children
/// (a `parentItem`) are kept. The readable location is the enclosure Zotero
/// reports for a file that exists on this machine, else an absolute
/// `data.path` (linked files).
pub fn attachment_page_from_json(
    body: &serde_json::Value,
    total: Option<u64>,
) -> Result<BibliographyAttachmentPage, ZoteroState> {
    let rows = body
        .as_array()
        .ok_or_else(|| ZoteroState::InvalidResponse {
            detail: "the library's answer was not a list of attachments".into(),
        })?;
    let malformed = || ZoteroState::InvalidResponse {
        detail: "an attachment of the page had no key, version or data".into(),
    };
    let mut attachments = Vec::new();
    for row in rows {
        let key = string_field(row, "key").ok_or_else(malformed)?;
        let version = row
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|version| i64::try_from(version).ok())
            .ok_or_else(malformed)?;
        let data = row
            .get("data")
            .filter(|data| data.is_object())
            .ok_or_else(malformed)?;
        let content_type = string_field(data, "contentType");
        let link_mode = string_field(data, "linkMode");
        let is_pdf = content_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("application/pdf"));
        // A web snapshot is an HTML file Zotero stored; a bare web link
        // (`linked_url`) has no file to read and stays out of the catalog.
        let is_stored_snapshot = content_type.as_deref().is_some_and(|kind| {
            kind.eq_ignore_ascii_case("text/html")
                || kind.eq_ignore_ascii_case("application/xhtml+xml")
        }) && link_mode.as_deref() != Some("linked_url");
        let Some(parent_key) = string_field(data, "parentItem") else {
            continue;
        };
        if !is_pdf && !is_stored_snapshot {
            continue;
        }
        let native_path = row
            .pointer("/links/enclosure/href")
            .and_then(serde_json::Value::as_str)
            .and_then(file_url_to_path)
            .or_else(|| {
                string_field(data, "path").filter(|path| std::path::Path::new(path).is_absolute())
            });
        attachments.push(BibliographyAttachment {
            parent_key,
            input: AttachmentInput {
                attachment_key: key,
                content_type,
                link_mode,
                filename: string_field(data, "filename"),
                native_path,
                url: string_field(data, "url"),
                md5: string_field(data, "md5"),
                mtime: data.get("mtime").and_then(serde_json::Value::as_i64),
                native_json_snapshot: row.to_string(),
                native_version: Some(version),
            },
        });
    }
    Ok(BibliographyAttachmentPage {
        attachments,
        rows_read: rows.len(),
        total,
    })
}

/// Narrow client seam for one library's pages: the executor asks for one
/// bounded page at a time and owns pacing and stopping. Collections, tags and
/// deletions stay out of the sync contract; works are reconciled, and PDF
/// attachments are cataloged by a second bounded walk
/// ([`ZoteroPageSource::fetch_attachment_page`]).
pub trait ZoteroPageSource: Send + Sync {
    fn fetch_page(&self, library: &Library, query: BibliographyPageQuery) -> PageFuture;

    /// One bounded page of the library's attachment items, or `None` when the
    /// source does not read attachments (the sync then leaves the attachment
    /// catalog exactly as it is, never treating "unsupported" as "empty").
    fn fetch_attachment_page(
        &self,
        _library: &Library,
        _query: BibliographyPageQuery,
    ) -> Option<AttachmentPageFuture> {
        None
    }
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

    fn fetch_attachment_page(
        &self,
        library: &Library,
        query: BibliographyPageQuery,
    ) -> Option<AttachmentPageFuture> {
        let client = self.client.clone();
        let library = library.clone();
        Some(Box::pin(async move {
            let answer = connector::read_attachments_page(
                &client,
                &library,
                ConnectorPage::new(query.start(), query.limit()),
            )
            .await?;
            attachment_page_from_json(&answer.body, answer.total)
        }))
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

fn sync_log(level: &str, message: &str) {
    crate::processing::scheduler::log_line(level, &format!("[bibliography] {message}"));
}

/// How the attachment walk ended.
enum AttachmentPass {
    Done,
    Stopped,
    Failed(ExecOutput),
}

/// Removes every attachment row of the library that a completed walk did not
/// see, in one transaction.
fn remove_unseen_attachments(
    conn: &mut rusqlite::Connection,
    library_row_id: &str,
    seen_ids: &HashSet<String>,
) -> Result<(), String> {
    let stored: Vec<String> = conn
        .prepare(
            "SELECT a.id FROM zotero_attachments a
               JOIN bibliographic_items i ON i.id = a.item_id
              WHERE i.library_id = ?1",
        )
        .map_err(|error| format!("Failed to list stored attachments: {error}"))?
        .query_map([library_row_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Failed to list stored attachments: {error}"))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("Failed to list stored attachments: {error}"))?;
    let gone: Vec<&String> = stored.iter().filter(|id| !seen_ids.contains(*id)).collect();
    if gone.is_empty() {
        return Ok(());
    }
    let tx = conn
        .transaction()
        .map_err(|error| format!("Failed to begin attachment removal: {error}"))?;
    for id in gone {
        tx.execute("DELETE FROM zotero_attachments WHERE id = ?1", [id])
            .map_err(|error| format!("Failed to remove attachment {id}: {error}"))?;
    }
    tx.commit()
        .map_err(|error| format!("Failed to commit attachment removal: {error}"))
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
    let profiles =
        processing_repository::admit_stale_profile_demands(conn, &output.library_row_id)?;
    // E4a-WU3: chained extraction demand for readable files, same
    // transaction and same durability.
    let extractions =
        processing_repository::admit_stale_extraction_demands(conn, &output.library_row_id)?;
    sync_log(
        "info",
        &format!(
            "library sync published: {} works, {profiles} profile and {extractions} extraction tasks queued",
            output.items_seen
        ),
    );
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
        match &output {
            ExecOutput::Retryable { code, message } | ExecOutput::Blocked { code, message } => {
                sync_log("warn", &format!("library sync paused ({code}): {message}"))
            }
            ExecOutput::Fatal { code, message } => {
                sync_log("error", &format!("library sync failed ({code}): {message}"))
            }
            ExecOutput::Stopped => sync_log("info", "library sync stopped before finishing"),
            ExecOutput::Success { .. } => {}
        }
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

    /// Catalogs the library's PDF attachments with a second bounded walk
    /// (one request per page of `page_limit` attachment items, never one per
    /// work). Each stored row is upserted under its already-cataloged parent;
    /// rows the completed walk no longer lists are removed (their extractions,
    /// chunks and embeddings cascade). Nothing is removed unless the walk
    /// finished and every page read cleanly: a failed or stopped read keeps
    /// the catalog as it was.
    fn attachment_pass(
        &self,
        ctx: &ExecCtx,
        library: &Library,
        library_row_id: &str,
        page_limit: u32,
        stop: &StopFlag,
    ) -> AttachmentPass {
        let storage = |message: String| {
            AttachmentPass::Failed(ExecOutput::Fatal {
                code: "storage_unavailable".to_string(),
                message,
            })
        };
        let invalid = |message: String| {
            AttachmentPass::Failed(ExecOutput::Retryable {
                code: "zotero_invalid_response".to_string(),
                message,
            })
        };
        let mut conn = match open_archive_connection(&ctx.db_path) {
            Ok(conn) => conn,
            Err(error) => return storage(error),
        };
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut start = 0u32;
        loop {
            if stop.stopped() {
                return AttachmentPass::Stopped;
            }
            let query = BibliographyPageQuery::new(start, page_limit);
            let Some(request) = self.source.fetch_attachment_page(library, query) else {
                // The source does not read attachments: leave the catalog be.
                return AttachmentPass::Done;
            };
            let page = match tauri::async_runtime::block_on(request) {
                Ok(page) => page,
                Err(state) => return AttachmentPass::Failed(endpoint_verdict(&state)),
            };
            if empty_while_total_remains(page.rows_read, start, page.total) {
                return invalid(format!(
                    "the library's attachment page at start {start} answered no rows while its total says more remain"
                ));
            }
            let page_end = u64::from(start).saturating_add(page.rows_read as u64);
            if page.rows_read > page_limit as usize
                || page.total.is_some_and(|total| page_end > total)
            {
                return invalid(format!(
                    "the library's attachment page at start {start} answered {} rows, outside what was asked for or reported",
                    page.rows_read
                ));
            }
            for attachment in &page.attachments {
                let parent_id: Option<String> = match conn
                    .query_row(
                        "SELECT id FROM bibliographic_items WHERE library_id = ?1 AND item_key = ?2",
                        rusqlite::params![library_row_id, &attachment.parent_key],
                        |row| row.get(0),
                    )
                    .optional()
                {
                    Ok(parent_id) => parent_id,
                    Err(error) => {
                        return storage(format!("Failed to resolve attachment parent: {error}"))
                    }
                };
                // A child of a work the catalog does not hold has nothing to
                // hang from; it is not an error and not seen.
                let Some(parent_id) = parent_id else { continue };
                match catalog_repository::upsert_attachment(
                    &mut conn,
                    &parent_id,
                    attachment.input.clone(),
                ) {
                    Ok(stored) => {
                        seen_ids.insert(stored.id);
                    }
                    Err(error) => return storage(error.message),
                }
            }
            match pagination_next(start, page.rows_read, page_limit, page.total) {
                Some(next) => start = next,
                None => break,
            }
        }
        match remove_unseen_attachments(&mut conn, library_row_id, &seen_ids) {
            Ok(()) => AttachmentPass::Done,
            Err(error) => storage(error),
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
        sync_log(
            "info",
            &format!(
                "library sync started (phase {:?}, cursor {}, total {:?})",
                run.phase, run.cursor_start, run.remote_total
            ),
        );
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
        // Items are durable; now catalog the works' PDF attachments so the
        // success publication below can chain extraction for them.
        sync_log(
            "info",
            &format!(
                "items walk complete: {} works read; walking attachments",
                run.cursor_start
            ),
        );
        match self.attachment_pass(ctx, &library, &run.library_id, page_limit, stop) {
            AttachmentPass::Done => {}
            AttachmentPass::Stopped => {
                return self.durable_verdict(
                    ctx,
                    &run,
                    checkpoints,
                    progress_total,
                    ExecOutput::Stopped,
                );
            }
            AttachmentPass::Failed(output) => {
                return self.durable_verdict(ctx, &run, checkpoints, progress_total, output);
            }
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
        sync_log(
            "info",
            "attachments walk complete; publishing the sync result",
        );
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

    fn native_page(number: i64, quality: &str, text: &str) -> ExtractPageText {
        ExtractPageText {
            page_number: number,
            method: "native".to_string(),
            text_content: text.to_string(),
            text_hash: extraction_text_hash(text),
            text_chars: text.chars().count() as i64,
            quality: quality.to_string(),
        }
    }

    /// Sparse and empty pages always go to OCR; an unreadable page only when
    /// the document has no native text at all, so an all-unreadable file does
    /// not settle as `empty` without one recognition attempt.
    #[test]
    fn unreadable_pages_reach_ocr_only_when_the_document_has_no_native_text() {
        let pages = [
            native_page(1, "unreadable", ""),
            native_page(2, "empty", ""),
            native_page(3, "sparse", "ok"),
            native_page(4, "rich", &"palabra ".repeat(20)),
        ];
        assert_eq!(ocr_candidate_pages(&pages, false), vec![2, 3]);
        assert_eq!(ocr_candidate_pages(&pages, true), vec![1, 2, 3]);
        let all_unreadable = [
            native_page(1, "unreadable", ""),
            native_page(2, "unreadable", ""),
        ];
        assert_eq!(ocr_candidate_pages(&all_unreadable, true), vec![1, 2]);
        assert!(ocr_candidate_pages(&all_unreadable, false).is_empty());
    }

    /// The page layer replaces a blank or poorer whole-document text, and
    /// never a rich one.
    #[test]
    fn the_richer_native_text_wins_unless_the_whole_document_is_rich() {
        let body = "palabra ".repeat(20);
        let pages = [native_page(1, "rich", &body)];
        assert_eq!(richer_native_text(String::new(), &pages), body.trim());
        assert_eq!(richer_native_text("2".to_string(), &pages), body.trim());
        let rich_whole = "otro texto ".repeat(20);
        assert_eq!(richer_native_text(rich_whole.clone(), &pages), rich_whole);
        assert_eq!(richer_native_text(String::new(), &[]), "");
    }

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
    /// Chunk texts with their vectors, in global ordinal order. Published
    /// atomically with the profile: chunk rows replace the work set and
    /// each vector stamps the same staging generation.
    pub chunks: Vec<StagedWorkChunk>,
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
    /// Embeds several texts, one vector per text in input order. The default
    /// keeps single-call embedders correct; providers that can batch override
    /// it. Implementations must fail the whole call rather than return a
    /// partial result.
    fn embed_many(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        texts.iter().map(|text| self.embed(text)).collect()
    }
    /// How many chunk texts one `embed_many` call should carry. One keeps the
    /// historical per-chunk granularity for embedders that do not batch.
    fn batch_hint(&self) -> usize {
        1
    }
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

    /// Resolves the engine for the current settings and releases the cache
    /// lock before any network or model work starts.
    fn engine(&self) -> Result<Arc<crate::nlp::embeddings::EmbeddingEngine>, String> {
        let conn = open_archive_connection(&self.db_path)?;
        let mut guard = self
            .cache
            .lock()
            .map_err(|e| format!("Embedding engine lock poisoned: {e}"))?;
        let crate::processing::embedding::EngineCache { cached, last_error } = &mut *guard;
        crate::nlp::ensure_embed_engine_for_current_settings(&conn, cached, last_error).ok_or_else(
            || crate::nlp::embeddings::embedding_engine_unavailable_reason(last_error.as_deref()),
        )
    }
}

impl ProfileEmbedder for EngineProfileEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.engine()?.embed_text(text)
    }

    fn embed_many(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.engine()?.embed_batch(&refs)
    }

    fn batch_hint(&self) -> usize {
        self.engine()
            .map(|engine| engine.preferred_wave())
            .unwrap_or(1)
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
    /// Chunk texts per embedding wave; `None` follows the embedder's hint.
    embed_wave: Option<usize>,
}

impl BibliographyProfileExecutor {
    pub fn new(embedder: Arc<dyn ProfileEmbedder>) -> Self {
        Self {
            embedder,
            embed_wave: None,
        }
    }

    /// Pins the number of chunk texts embedded per wave (and checkpointed
    /// together), overriding the embedder's hint.
    pub fn with_embed_wave(mut self, wave: usize) -> Self {
        self.embed_wave = Some(wave.max(1));
        self
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
                code: "configuration_required_embedding".to_string(),
                message: error,
            })?;
        if task.contract_hash != effective.hash {
            return Err(ExecOutput::Blocked {
                code: "configuration_required_embedding_contract".to_string(),
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
                Ok(CompactVec(vector))
            })
            .map_err(|error| match map_embedding_error(&error) {
                ExecOutput::Fatal { code, message } => ExecOutput::Fatal { code, message },
                other => other,
            })?
            .0;
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
        let chunks = self.stage_chunks(ctx, task, stop, &model, &contract, dimensions)?;
        let output = BibliographyProfileComputeOutput {
            item_id: task.subject_id.clone(),
            generation_id: generation.id.clone(),
            chunks,
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
            "chunkCount": output.chunks.len(),
        })
        .to_string();
        Ok(ExecResult {
            checkpoints: Vec::new(),
            progress_total: Some(1 + output.chunks.len() as i64),
            engine_output: Some(EngineOutput::BibliographyProfile(output)),
            output: ExecOutput::Success {
                outcome: "bibliography_profiled".to_string(),
                receipt,
            },
        })
    }
}

impl BibliographyProfileExecutor {
    /// Segments every chunkable page of the work's attachments and embeds
    /// the chunks in waves. Ordinals run globally per work across
    /// attachments, in (attachment, page) order; chunk ids are deterministic
    /// per (work, ordinal) so re-chunks replace instead of appending.
    ///
    /// Chunks whose vector is already stored under the same text hash and
    /// `(model, contract, dimensions)` are reused. The rest travel in waves
    /// through [`ProfileEmbedder::embed_many`]; each wave is one checkpoint
    /// whose key binds the ordered chunk hashes, so a retry resumes after the
    /// waves that succeeded and a page-text edit never serves a stale vector.
    /// A failing wave fails the whole run: nothing is staged for publish.
    #[allow(clippy::too_many_arguments)]
    fn stage_chunks(
        &self,
        ctx: &crate::processing::scheduler::ExecCtx,
        task: &crate::processing::scheduler::ClaimedTask,
        stop: &crate::processing::scheduler::StopFlag,
        model: &str,
        contract: &str,
        dimensions: usize,
    ) -> Result<Vec<StagedWorkChunk>, crate::processing::scheduler::ExecOutput> {
        use crate::processing::scheduler::ExecOutput;
        let conn = open_archive_connection(&ctx.db_path).map_err(|error| ExecOutput::Fatal {
            code: "storage_unavailable".to_string(),
            message: error,
        })?;
        let pages =
            crate::bibliography::repository::chunkable_pages_for_item(&conn, &task.subject_id)
                .map_err(|error| ExecOutput::Fatal {
                    code: "storage_unavailable".to_string(),
                    message: format!("{}: {}", error.code, error.message),
                })?;
        let mut reusable = crate::bibliography::repository::reusable_chunk_embeddings(
            &conn,
            &task.subject_id,
            model,
            contract,
            dimensions,
        )
        .map_err(|error| ExecOutput::Fatal {
            code: "storage_unavailable".to_string(),
            message: format!("{}: {}", error.code, error.message),
        })?;
        drop(conn);
        // A stored blob that does not match the contract width is not reusable.
        reusable.retain(|_, blob| blob.len() == dimensions * 4);
        // Group pages per attachment preserving order, then segment.
        let mut by_attachment: Vec<(String, Vec<crate::bibliography::chunks::PageInput>)> =
            Vec::new();
        for page in pages {
            match by_attachment.last_mut() {
                Some((attachment_id, inputs)) if *attachment_id == page.attachment_id => inputs
                    .push(crate::bibliography::chunks::PageInput {
                        page_number: page.page_number,
                        text: page.text_content,
                    }),
                _ => by_attachment.push((
                    page.attachment_id.clone(),
                    vec![crate::bibliography::chunks::PageInput {
                        page_number: page.page_number,
                        text: page.text_content,
                    }],
                )),
            }
        }
        // Segment the whole work up front: ordinals, ids, and spans are
        // fixed before any embedding starts.
        let mut staged: Vec<StagedWorkChunk> = Vec::new();
        for (attachment_id, inputs) in &by_attachment {
            for chunk in crate::bibliography::chunks::segment_pages(inputs) {
                let ordinal = staged.len() as i64;
                staged.push(StagedWorkChunk {
                    attachment_id: attachment_id.clone(),
                    ordinal,
                    chunk_id: format!("{}:{:06}", task.subject_id, ordinal),
                    input_hash: chunk.hash.clone(),
                    spans: chunk
                        .spans
                        .iter()
                        .map(|span| {
                            (
                                span.page_number,
                                span.start_char as i64,
                                span.end_char as i64,
                            )
                        })
                        .collect(),
                    text_content: chunk.text,
                    embedding: reusable.get(&chunk.hash).cloned().unwrap_or_default(),
                });
            }
        }
        // Chunks still lacking a vector, in ordinal order.
        let pending: Vec<usize> = staged
            .iter()
            .enumerate()
            .filter(|(_, chunk)| chunk.embedding.is_empty())
            .map(|(index, _)| index)
            .collect();
        let wave_size = self
            .embed_wave
            .unwrap_or_else(|| self.embedder.batch_hint())
            .max(1);
        for wave in pending.chunks(wave_size) {
            if stop.stopped() {
                return Err(ExecOutput::Stopped);
            }
            let texts: Vec<String> = wave
                .iter()
                .map(|&index| staged[index].text_content.clone())
                .collect();
            let wave_digest = {
                let mut digest = Sha256::new();
                for &index in wave {
                    digest.update(staged[index].input_hash.as_bytes());
                    digest.update([0_u8]);
                }
                format!("{:x}", digest.finalize())
            };
            let key = format!(
                "chunk-wave:{}:{}",
                staged[wave[0]].ordinal,
                &wave_digest[..16]
            );
            let vectors = ctx
                .unit(task, &key, || {
                    let vectors = self.embedder.embed_many(&texts)?;
                    if vectors.len() != texts.len() {
                        return Err(format!(
                            "Chunk embedding returned {} vectors for {} chunks",
                            vectors.len(),
                            texts.len()
                        ));
                    }
                    if vectors.iter().any(|vector| {
                        vector.len() != dimensions || vector.iter().any(|value| !value.is_finite())
                    }) {
                        return Err(format!(
                            "Chunk embedding does not satisfy {dimensions} finite dimensions"
                        ));
                    }
                    Ok(vectors.into_iter().map(CompactVec).collect::<Vec<_>>())
                })
                .map_err(|error| {
                    match crate::bibliography::selective_ocr::map_page_ocr_error(&error) {
                        ExecOutput::Fatal {
                            code: _,
                            message: _,
                        } => ExecOutput::Fatal {
                            code: "embedding_failed".to_string(),
                            message: error,
                        },
                        // Lease/demand loss and retryable/blocked verdicts
                        // pass through untouched: checkpoints stay Stopped,
                        // provider states stay honest.
                        other => other,
                    }
                })?;
            for (&index, vector) in wave.iter().zip(&vectors) {
                staged[index].embedding = crate::nlp::embeddings::floats_to_blob(&vector.0);
            }
        }
        Ok(staged)
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

/// One staged chunk: identity, text, spans, and its vector under the run's
/// staging generation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedWorkChunk {
    pub attachment_id: String,
    pub ordinal: i64,
    pub chunk_id: String,
    pub text_content: String,
    pub input_hash: String,
    pub spans: Vec<(i64, i64, i64)>,
    pub embedding: Vec<u8>,
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
    // Chunks replace the work set atomically — vectors cascade on delete
    // — then fresh rows land with their vectors under this same staging
    // generation, whose staging state was verified above.
    let chunk_rows: Vec<crate::bibliography::repository::ChunkRow> = output
        .chunks
        .iter()
        .map(|chunk| crate::bibliography::repository::ChunkRow {
            id: chunk.chunk_id.clone(),
            item_id: output.item_id.clone(),
            attachment_id: chunk.attachment_id.clone(),
            ordinal: chunk.ordinal,
            text_content: chunk.text_content.clone(),
            text_hash: chunk.input_hash.clone(),
            chunking_contract: crate::bibliography::chunks::BIBLIOGRAPHY_CHUNKING_CONTRACT_V1
                .to_string(),
            spans: chunk.spans.clone(),
        })
        .collect();
    crate::bibliography::repository::replace_work_chunks_in_transaction(
        conn,
        &output.item_id,
        &chunk_rows,
        processing_repository::now_ms(),
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    for chunk in &output.chunks {
        crate::bibliography::repository::upsert_chunk_embedding_in_transaction(
            conn,
            &crate::bibliography::repository::ChunkEmbeddingRow {
                chunk_id: chunk.chunk_id.clone(),
                generation_id: output.generation_id.clone(),
                embedding_contract: output.contract.clone(),
                embedding_model: output.model.clone(),
                dimensions: output.dimensions,
                embedding: chunk.embedding.clone(),
                input_hash: chunk.input_hash.clone(),
            },
            processing_repository::now_ms(),
        )
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
    // The publish that lands the last owed work makes the generation
    // queryable in this same commit. Activation runs in its own savepoint and
    // is idempotent, so a failure here never costs the profile that just
    // landed: the next publish, sync or restart retries it.
    if let Err(error) = crate::bibliography::generation::activate_if_complete(
        conn,
        &output.generation_id,
        processing_repository::now_ms(),
    ) {
        eprintln!(
            "[bibliography] generation {} activation deferred: {}: {}",
            output.generation_id, error.code, error.message
        );
    }
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
    /// `native`, or `ocr` once the selective pass replaced the text.
    pub method: String,
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
    /// The stored extraction already reflects this exact source identity
    /// (a duplicate demand queued before admission learned to skip it): the
    /// publisher re-proves that and writes nothing.
    pub already_current: bool,
}

fn extraction_text_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn extraction_quality(text: &str) -> &'static str {
    if text.trim().is_empty() {
        "empty"
    } else if crate::ocr::pdf::is_garbled_text(text) {
        // Raw glyph codes from a custom font encoding: not text a reader can
        // use, and nothing to protect. Graded `empty` so OCR owns the page.
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

/// A stored web page: Zotero's `text/html` snapshots (and XHTML). Checked
/// after PDF, so a mislabeled `.pdf` never reads as markup.
fn is_html_attachment(content_type: Option<&str>, filename: Option<&str>) -> bool {
    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.contains("html") {
        return true;
    }
    let filename = filename.unwrap_or_default().to_ascii_lowercase();
    [".html", ".htm", ".xhtml"]
        .iter()
        .any(|extension| filename.ends_with(extension))
}

/// The page layer of one extraction, whatever the source format.
struct ExtractedDocument {
    page_count: i64,
    text: String,
    quality: &'static str,
    pages: Vec<ExtractPageText>,
    ocr_failed_pages: Vec<i64>,
    /// A provider answered at least once (even with no text): the document
    /// went through a real OCR pass, so an `empty` verdict is final and the
    /// sync must not demand the same pass again.
    ocr_attempted: bool,
}

/// An HTML snapshot is one "page": its block paragraphs, blank-line
/// separated, so the paragraph chunker runs unchanged and citations point at
/// paragraph ranges. The attachment's content type, not the page number,
/// tells a reader this is a web page rather than page 1 of a paper. A page
/// that is all furniture yields empty text, which nothing downstream chunks.
fn extract_html_document(bytes: &[u8]) -> ExtractedDocument {
    let text = crate::bibliography::html_text::html_to_paragraphs(
        &crate::bibliography::html_text::decode_html_bytes(bytes),
    );
    let quality = if text.trim().is_empty() {
        "empty"
    } else {
        "rich"
    };
    let page = ExtractPageText {
        page_number: 1,
        method: "native".to_string(),
        text_hash: extraction_text_hash(&text),
        text_chars: text.chars().count() as i64,
        quality: quality.to_string(),
        text_content: text.clone(),
    };
    ExtractedDocument {
        page_count: 1,
        text,
        quality,
        pages: vec![page],
        ocr_failed_pages: Vec::new(),
        ocr_attempted: false,
    }
}

/// The E4a-WU2 native extraction engine: resolve, read, extract the text
/// layer, grade it. Every unresolvable state maps to an honest verdict —
/// blocked when the user can fix it (missing file, unconfigured data dir),
/// fatal when retrying could never help (unsupported MIME, corrupt file).
pub struct BibliographyExtractExecutor {
    selective_ocr: Option<(
        std::sync::Arc<dyn crate::bibliography::selective_ocr::PageRenderer>,
        std::sync::Arc<dyn crate::bibliography::selective_ocr::PageOcrProvider>,
    )>,
}

impl BibliographyExtractExecutor {
    /// Native-only extraction: the OCR pass is skipped entirely.
    pub fn new() -> Self {
        Self {
            selective_ocr: None,
        }
    }

    /// Native extraction plus production selective OCR (pdfium rendering
    /// with the configured recognition provider). Used by the production
    /// supervisor; tests inject fakes through with_selective_ocr.
    pub fn with_production_ocr(app: tauri::AppHandle, db_path: std::path::PathBuf) -> Self {
        let production = std::sync::Arc::new(ProductionSelectiveOcr::new(app, db_path));
        Self {
            selective_ocr: Some((production.clone(), production)),
        }
    }

    /// Native extraction plus selective OCR: pages whose native layer is
    /// sparse or empty are rendered and recognized page by page. Rich and
    /// unreadable pages are never sent to a provider.
    pub fn with_selective_ocr(
        renderer: std::sync::Arc<dyn crate::bibliography::selective_ocr::PageRenderer>,
        provider: std::sync::Arc<dyn crate::bibliography::selective_ocr::PageOcrProvider>,
    ) -> Self {
        Self {
            selective_ocr: Some((renderer, provider)),
        }
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
                code: "configuration_required_extract_contract".to_string(),
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
        let is_pdf = is_pdf_attachment(
            attachment.content_type.as_deref(),
            attachment.filename.as_deref(),
        );
        if !is_pdf
            && !is_html_attachment(
                attachment.content_type.as_deref(),
                attachment.filename.as_deref(),
            )
        {
            return Err(ExecOutput::Fatal {
                code: "extraction_unsupported".to_string(),
                message: format!(
                    "attachment {} is neither a PDF nor an HTML snapshot ({}); native text extraction covers those only",
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
        // A duplicate demand for a source the stored extraction already
        // reflects finishes here: no read, no extraction, no OCR, no rewrite
        // of the page rows and no profile re-chain.
        {
            let conn =
                open_archive_connection(&ctx.db_path).map_err(|error| ExecOutput::Fatal {
                    code: "storage_unavailable".to_string(),
                    message: error,
                })?;
            let current = crate::bibliography::repository::extraction_is_settled(
                &conn,
                &task.subject_id,
                attachment.mtime,
                metadata.len() as i64,
            )
            .map_err(|error| ExecOutput::Fatal {
                code: "storage_unavailable".to_string(),
                message: format!("{}: {}", error.code, error.message),
            })?;
            if current {
                let output = BibliographyExtractComputeOutput {
                    attachment_id: task.subject_id.clone(),
                    item_id,
                    page_count: 0,
                    text_content: String::new(),
                    text_hash: String::new(),
                    text_chars: 0,
                    quality: String::new(),
                    source_mtime: attachment.mtime,
                    source_bytes: metadata.len() as i64,
                    pages: Vec::new(),
                    already_current: true,
                };
                let receipt = serde_json::json!({
                    "attachmentId": output.attachment_id,
                    "itemId": output.item_id,
                    "alreadyCurrent": true,
                })
                .to_string();
                return Ok(ExecResult {
                    checkpoints: Vec::new(),
                    progress_total: Some(1),
                    engine_output: Some(EngineOutput::BibliographyExtract(output)),
                    output: ExecOutput::Success {
                        outcome: "bibliography_extract_current".to_string(),
                        receipt,
                    },
                });
            }
        }
        // The file is read straight from disk, never checkpointed: a unit
        // for it serialised every byte of the PDF as a JSON number array
        // (about 3.5x the file, 451 MB for one large book) and bought nothing,
        // since re-reading a local file costs less than decoding that row.
        let bytes = std::fs::read(&path).map_err(|error| ExecOutput::Retryable {
            code: "extraction_io".to_string(),
            message: format!("Failed to read attachment file {}: {error}", path.display()),
        })?;
        if stop.stopped() {
            return Err(ExecOutput::Stopped);
        }
        let document = if is_pdf {
            self.extract_pdf_document(ctx, task, stop, &bytes)?
        } else {
            extract_html_document(&bytes)
        };
        let ExtractedDocument {
            page_count,
            text,
            quality,
            pages,
            ocr_failed_pages,
            ocr_attempted,
        } = document;
        let text_chars = text.chars().count() as i64;
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
            already_current: false,
        };
        let receipt = serde_json::json!({
            "attachmentId": output.attachment_id,
            "itemId": output.item_id,
            "quality": output.quality,
            "pageCount": output.page_count,
            "textHash": output.text_hash,
            "ocrFailedPages": ocr_failed_pages,
            "ocrAttempted": ocr_attempted,
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

    /// PDF branch: lopdf structure and text layer, per-page native text,
    /// then the selective OCR pass over sparse pages.
    fn extract_pdf_document(
        &self,
        ctx: &crate::processing::scheduler::ExecCtx,
        task: &crate::processing::scheduler::ClaimedTask,
        stop: &crate::processing::scheduler::StopFlag,
        bytes: &[u8],
    ) -> Result<ExtractedDocument, crate::processing::scheduler::ExecOutput> {
        use crate::processing::scheduler::ExecOutput;
        // A PDF with `/Encrypt` is only locked when it needs a real user
        // password. Permissions-only protection (owner password, empty user
        // password) is common on journal articles and opens freely, so it is
        // decrypted once here and every later step — per-page text, the
        // whole-document parser, page rendering for OCR — reads plain bytes.
        // A genuinely locked file fails with unlock guidance, not with a
        // complaint about damage: re-importing an unlocked copy mints fresh
        // demand through the file-identity gate, so terminal is correct.
        let readable = crate::ocr::pdf::open_with_empty_password(bytes).map_err(|message| {
            ExecOutput::Fatal {
                code: "extraction_failed".to_string(),
                message,
            }
        })?;
        let bytes: &[u8] = &readable;
        let document = lopdf::Document::load_mem(bytes).map_err(|error| ExecOutput::Fatal {
            code: "extraction_failed".to_string(),
            message: format!("Failed to parse PDF: {error}"),
        })?;
        let page_count = document.get_pages().len() as i64;
        let pages = read_native_page_texts(bytes, page_count)?;
        // The whole-document parser is the primary text source. When it
        // fails (or panics, contained inside `extract_pdf_text`) the
        // per-page lopdf layer — already read for the page rows — still
        // holds the text, so the file is read instead of failed. Only a
        // document with no text on either path reports the parser error.
        let text = match crate::ocr::pdf::extract_pdf_text(bytes) {
            Ok(text) => text,
            Err(error) => {
                let joined = pages
                    .iter()
                    .map(|page| page.text_content.as_str())
                    .filter(|text| !text.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(
                        "

",
                    );
                if joined.is_empty() {
                    return Err(ExecOutput::Fatal {
                        code: "extraction_failed".to_string(),
                        message: error,
                    });
                }
                joined
            }
        };
        // The whole-document parser can also return less than the page layer
        // holds (an inline image ahead of the text drops everything after
        // it). When it is not already rich, the richer of the two wins.
        let text = richer_native_text(text, &pages);
        let mut quality = extraction_quality(&text);
        let mut text = text;
        // Garbled native text (raw glyph codes) is not text: when the OCR
        // pass replaced or dropped any page, the whole-document row is
        // rebuilt from the published pages so no glyph code survives in it.
        let had_garbled_native = pages
            .iter()
            .any(|page| crate::ocr::pdf::is_garbled_text(&page.text_content));
        let (pages, ocr_failed_pages, ocr_attempted) =
            self.maybe_ocr_pages(ctx, task, stop, bytes, pages, quality == "empty")?;
        // A scan has no native text, so the whole-document verdict above is
        // `empty` however well OCR reads it. When recognition replaced pages
        // and the native layer was not already rich, the document text and
        // its quality come from the pages now published, or every scanned
        // PDF would stay `empty` after its text exists.
        if (quality != "rich" || had_garbled_native)
            && pages.iter().any(|page| page.method == "ocr")
        {
            text = pages
                .iter()
                .map(|page| page.text_content.trim())
                .filter(|page_text| !page_text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            quality = extraction_quality(&text);
        }
        Ok(ExtractedDocument {
            page_count,
            text,
            quality,
            pages,
            ocr_failed_pages,
            ocr_attempted,
        })
    }

    /// Runs the selective OCR pass over pages whose native layer is
    /// sparse or empty. Rich and unreadable pages never reach a
    /// provider. Each OCR page checkpoints under `ocr-page:{n}` through
    /// `ctx.unit`, so resume reuses confirmed texts without re-sending
    /// content, and demand loss stops before the next provider call.
    /// An empty OCR answer keeps the native row untouched — unless the
    /// native text is garbled raw glyph codes, which OCR owns: the page is
    /// then recorded empty and the codes are never kept. A non-empty
    /// one replaces the page text (method `ocr`) with a fresh hash —
    /// never appended, so native fragments cannot duplicate.
    fn maybe_ocr_pages(
        &self,
        ctx: &crate::processing::scheduler::ExecCtx,
        task: &crate::processing::scheduler::ClaimedTask,
        stop: &crate::processing::scheduler::StopFlag,
        bytes: &[u8],
        pages: Vec<ExtractPageText>,
        native_blank: bool,
    ) -> Result<(Vec<ExtractPageText>, Vec<i64>, bool), crate::processing::scheduler::ExecOutput>
    {
        use crate::processing::scheduler::ExecOutput;
        let Some((renderer, provider)) = &self.selective_ocr else {
            return Ok((pages, Vec::new(), false));
        };
        let _capability = crate::bibliography::selective_ocr::probe_page_ocr_capability(
            true,
            Some(provider.name()),
        );
        let mut ocr_attempted = false;
        // Whole-document recognition first, when the provider offers it and
        // the document is mostly scan: one request per window of pages, no
        // page rendering. A window the provider rejects (not a rate limit or
        // a credential problem) drops back to the per-page path below.
        let needing = ocr_candidate_pages(&pages, native_blank);
        let mut windowed: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
        if let Some(per_request) = provider.pdf_pages_per_request() {
            if crate::bibliography::selective_ocr::should_use_pdf_mode(needing.len(), pages.len()) {
                for (first, last) in crate::bibliography::selective_ocr::plan_pdf_windows(
                    &needing,
                    pages.len(),
                    per_request,
                ) {
                    if stop.stopped() {
                        return Err(ExecOutput::Stopped);
                    }
                    let unit_key = format!("ocr-range:{first}-{last}");
                    let result: Result<Vec<String>, String> = ctx.unit(task, &unit_key, || {
                        let texts = provider.recognize_pdf_pages(bytes, first, last)?;
                        let expected = (last - first + 1) as usize;
                        if texts.len() != expected {
                            return Err(format!(
                                "provider_error: whole-document OCR returned {} pages for a range of {expected}",
                                texts.len()
                            ));
                        }
                        Ok(texts)
                    });
                    match result {
                        Ok(texts) => {
                            ocr_attempted = true;
                            for (offset, text) in texts.into_iter().enumerate() {
                                windowed.insert(i64::from(first) + offset as i64, text);
                            }
                        }
                        Err(error) => {
                            if error.starts_with("lease_lost") || error.starts_with("demand_lost") {
                                return Err(ExecOutput::Stopped);
                            }
                            match crate::bibliography::selective_ocr::map_page_ocr_error(&error) {
                                ExecOutput::Fatal { .. } => {
                                    eprintln!(
                                        "[bibliography] whole-document OCR of pages {first}-{last} failed, using per-page OCR: {error}"
                                    );
                                }
                                other => return Err(other),
                            }
                        }
                    }
                }
            }
        }
        let mut out: Vec<ExtractPageText> = Vec::with_capacity(pages.len());
        let mut ocr_failed_pages: Vec<i64> = Vec::new();
        for page in pages {
            if !needing.contains(&page.page_number) {
                out.push(page);
                continue;
            }
            let text = if let Some(text) = windowed.remove(&page.page_number) {
                text
            } else {
                if stop.stopped() {
                    return Err(ExecOutput::Stopped);
                }
                let unit_key = format!("ocr-page:{}", page.page_number);
                match ctx.unit(task, &unit_key, || {
                    let image = renderer
                        .render_page(bytes, page.page_number as u32)
                        .map_err(|error| format!("render failed: {error}"))?;
                    provider.recognize_page(&image)
                }) {
                    Ok(text) => {
                        ocr_attempted = true;
                        text
                    }
                    // A GLM-OCR answer with no content is a blank page, not
                    // a page failure: the provider answered, the page simply
                    // holds no text (whole-asset corpus OCR keeps it an
                    // error). Recorded as empty text below.
                    Err(error)
                        if crate::bibliography::selective_ocr::is_empty_ocr_page_response(
                            &error,
                        ) =>
                    {
                        ocr_attempted = true;
                        String::new()
                    }
                    Err(error) => {
                        if error.starts_with("lease_lost") || error.starts_with("demand_lost") {
                            return Err(ExecOutput::Stopped);
                        }
                        let verdict = if let Some(detail) = error.strip_prefix("render failed: ") {
                            ExecOutput::Fatal {
                                code: "extraction_failed".to_string(),
                                message: detail.to_string(),
                            }
                        } else {
                            crate::bibliography::selective_ocr::map_page_ocr_error(&error)
                        };
                        match verdict {
                            // E4b-WU4 incomplete handling: a page whose OCR
                            // hard-failed keeps its native row and is named in
                            // the receipt. Transient and configuration verdicts
                            // stay whole-task: backoff and user fixes must not
                            // masquerade as partial success.
                            ExecOutput::Fatal { message, .. } => {
                                eprintln!(
                                    "[bibliography] OCR of page {} failed: {message}",
                                    page.page_number
                                );
                                ocr_failed_pages.push(page.page_number);
                                out.push(page);
                                continue;
                            }
                            other => return Err(other),
                        }
                    }
                }
            };
            if text.trim().is_empty() {
                eprintln!(
                    "[bibliography] page {}: no text recognized",
                    page.page_number
                );
                if crate::ocr::pdf::is_garbled_text(&page.text_content) {
                    // OCR owns a garbled page: the raw glyph codes must not
                    // survive a blank answer, so the page is recorded with
                    // empty text instead of keeping the garbage.
                    out.push(ExtractPageText {
                        page_number: page.page_number,
                        method: "ocr".to_string(),
                        text_hash: extraction_text_hash(""),
                        text_chars: 0,
                        quality: "empty".to_string(),
                        text_content: String::new(),
                    });
                } else {
                    out.push(page);
                }
                continue;
            }
            let quality = extraction_quality(&text);
            out.push(ExtractPageText {
                page_number: page.page_number,
                method: "ocr".to_string(),
                text_hash: extraction_text_hash(&text),
                text_chars: text.chars().count() as i64,
                quality: quality.to_string(),
                text_content: text,
            });
        }
        Ok((out, ocr_failed_pages, ocr_attempted))
    }
}

/// Production selective OCR: pdfium page rendering plus the configured
/// recognition provider — the Paddle engine when this build compiles it,
/// the remote GLM provider otherwise. It mirrors the documentary worker's
/// provider selection without invoking its executor: missing models or a
/// missing API key surface as ordinary provider errors, which the engine
/// maps to Blocked instead of failing the task. Page numbers are 1-based;
/// the renderer converts to pdfium's 0-based index.
pub struct ProductionSelectiveOcr {
    app: tauri::AppHandle,
    // Only the remote GLM path reads it (for the API key); the Paddle build
    // recognises locally.
    #[cfg_attr(feature = "paddle-ocr", allow(dead_code))]
    db_path: std::path::PathBuf,
    #[cfg(feature = "paddle-ocr")]
    paddle: std::sync::Mutex<Option<crate::ocr::paddle::PaddleOcrProvider>>,
}

impl ProductionSelectiveOcr {
    pub fn new(app: tauri::AppHandle, db_path: std::path::PathBuf) -> Self {
        Self {
            app,
            db_path,
            #[cfg(feature = "paddle-ocr")]
            paddle: std::sync::Mutex::new(None),
        }
    }

    #[cfg(feature = "paddle-ocr")]
    fn paddle_recognize(&self, image_bytes: &[u8]) -> Result<String, String> {
        use crate::ocr::provider::OcrProvider as _;
        let mut guard = self
            .paddle
            .lock()
            .map_err(|e| format!("Paddle engine lock poisoned: {e}"))?;
        if guard.is_none() {
            let model_dir = crate::ocr::resolve_paddle_model_dir(&self.app);
            *guard = Some(crate::ocr::paddle::PaddleOcrProvider::new(model_dir)?);
        }
        // The guard lives across the call: the queue supervisor runs one
        // unit at a time, so no other page contends for the engine.
        let engine = guard
            .as_ref()
            .ok_or_else(|| "Paddle engine failed to initialize".to_string())?;
        engine.recognize(image_bytes).map(|output| output.text)
    }
}

impl crate::bibliography::selective_ocr::PageRenderer for ProductionSelectiveOcr {
    fn render_page(&self, pdf_bytes: &[u8], page_number: u32) -> Result<Vec<u8>, String> {
        // pdfium resolves its library from a path cached at startup of the
        // first OCR command; the queue worker never runs one, so without this
        // every page failed to render and a scan ended up with no text.
        crate::ocr::pdf::init_pdfium_path(&self.app);
        crate::ocr::pdf::render_pdf_page_to_image(pdf_bytes, page_number.saturating_sub(1) as usize)
    }

    fn name(&self) -> &'static str {
        "pdfium"
    }
}

impl crate::bibliography::selective_ocr::PageOcrProvider for ProductionSelectiveOcr {
    fn recognize_page(&self, image_bytes: &[u8]) -> Result<String, String> {
        #[cfg(feature = "paddle-ocr")]
        {
            self.paddle_recognize(image_bytes)
        }
        #[cfg(not(feature = "paddle-ocr"))]
        {
            let conn = open_archive_connection(&self.db_path)?;
            let api_key = crate::ocr::get_glm_ocr_api_key(&conn);
            if api_key.is_empty() {
                return Err("configuration: GLM-OCR no está configurado. Andá a Configuración > OCR y cargá una API key antes de usar OCR.".to_string());
            }
            let output =
                tauri::async_runtime::block_on(crate::ocr::process_with_glm_ocr_provider(
                    image_bytes,
                    "bibliography-page",
                    &self.app,
                    &api_key,
                    "glm_ocr",
                ))?;
            Ok(output.ocr.text)
        }
    }

    fn pdf_pages_per_request(&self) -> Option<usize> {
        // Local Paddle recognizes page images; only the remote GLM provider
        // reads a whole PDF.
        #[cfg(feature = "paddle-ocr")]
        {
            None
        }
        #[cfg(not(feature = "paddle-ocr"))]
        {
            Some(crate::ocr::MAX_GLM_PDF_PAGE_COUNT)
        }
    }

    fn recognize_pdf_pages(
        &self,
        pdf_bytes: &[u8],
        first_page: u32,
        last_page: u32,
    ) -> Result<Vec<String>, String> {
        #[cfg(feature = "paddle-ocr")]
        {
            let _ = (pdf_bytes, first_page, last_page);
            Err("pdf_mode_unsupported: the local engine reads page images only".to_string())
        }
        #[cfg(not(feature = "paddle-ocr"))]
        {
            let conn = open_archive_connection(&self.db_path)?;
            let api_key = crate::ocr::get_glm_ocr_api_key(&conn);
            if api_key.is_empty() {
                return Err("configuration: GLM-OCR no está configurado. Andá a Configuración > OCR y cargá una API key antes de usar OCR.".to_string());
            }
            let client = crate::ocr::glm_ocr::GlmOcrClient::new(api_key);
            tauri::async_runtime::block_on(crate::ocr::glm_recognize_pdf_pages(
                &client, pdf_bytes, first_page, last_page,
            ))
        }
    }

    fn name(&self) -> &str {
        #[cfg(feature = "paddle-ocr")]
        {
            "paddle"
        }
        #[cfg(not(feature = "paddle-ocr"))]
        {
            "glm"
        }
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

/// Pages the OCR pass must read: sparse and empty ones always. A page the
/// native decoder could not read is `unreadable`; it stays out of OCR while
/// the document as a whole has native text, but when the document has none at
/// all (`native_blank`) there is nothing native left to protect, so it goes
/// to OCR too instead of the file settling as `empty` without a single
/// recognition attempt.
fn ocr_candidate_pages(pages: &[ExtractPageText], native_blank: bool) -> Vec<i64> {
    pages
        .iter()
        .filter(|page| match page.quality.as_str() {
            "sparse" | "empty" => true,
            "unreadable" => native_blank,
            _ => false,
        })
        .map(|page| page.page_number)
        .collect()
}

/// Keeps the whole-document text unless it is not rich and the per-page layer
/// holds clearly more (by alphanumeric characters).
fn richer_native_text(text: String, pages: &[ExtractPageText]) -> String {
    if extraction_quality(&text) == "rich" {
        return text;
    }
    let joined = pages
        .iter()
        .map(|page| page.text_content.trim())
        .filter(|page_text| !page_text.is_empty())
        .collect::<Vec<_>>()
        .join(
            "

",
        );
    let alphanumeric = |value: &str| value.chars().filter(|c| c.is_alphanumeric()).count();
    if alphanumeric(&joined) > alphanumeric(&text) {
        joined
    } else {
        text
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
            method: "native".to_string(),
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
    if output.already_current {
        // Nothing to write, but the claim still has to be true at commit.
        let still_current = crate::bibliography::repository::extraction_matches_source(
            conn,
            &output.attachment_id,
            output.source_mtime,
            output.source_bytes,
        )
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
        if !still_current {
            return Err(format!(
                "source_changed: extraction of attachment {} moved mid-computation",
                output.attachment_id
            ));
        }
        return Ok(());
    }
    // E4c-WU3 invalidation: chain profile demand when the page layer
    // moved. The profile run re-segments and re-embeds chunks; an
    // unchanged page layer chains nothing, so re-extracts of identical
    // bytes stay silent.
    let mut pages_moved = false;
    for page in &output.pages {
        let stored: Option<String> = conn
            .query_row(
                "SELECT text_hash FROM bibliographic_page_texts
                 WHERE attachment_id = ?1 AND page_number = ?2",
                rusqlite::params![output.attachment_id, page.page_number],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Failed to compare page texts: {error}"))?;
        if stored.as_deref() != Some(page.text_hash.as_str()) {
            pages_moved = true;
            break;
        }
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
    // E4c-WU3: a moved page layer re-demands the profile (chunks derive
    // from these rows). Single-flight attaches when a live profile task
    // already exists; terminal history mints a fresh task.
    if pages_moved {
        let batch_id = processing_repository::ensure_system_batch(conn, "bibliography")
            .map_err(|error| format!("Failed to open bibliography batch: {error}"))?;
        let _ = processing_repository::admit_subject_or_attach(
            conn,
            &batch_id,
            "bibliography_profile",
            &processing_repository::TaskSubject {
                domain: "bibliography".to_string(),
                subject_kind: "item".to_string(),
                subject_id: output.item_id.clone(),
            },
            0,
            "",
            "",
            None,
        )
        .map_err(|error| format!("Failed to chain profile demand: {error}"))?;
    }
    for page in &output.pages {
        crate::bibliography::repository::upsert_page_text_in_transaction(
            conn,
            &crate::bibliography::repository::PageTextRow {
                attachment_id: output.attachment_id.clone(),
                page_number: page.page_number,
                method: page.method.clone(),
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
