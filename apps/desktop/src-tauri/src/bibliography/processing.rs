//! Bibliographic sync execution behind the batch queue (E2b-2).
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
//! # What this build can and cannot honestly report
//!
//! E2b-2 owns **dispatch and enumeration only**. Per-page durable catalog
//! transactions and the success receipt belong to E2b-3, so a fully
//! enumerated library parks `blocked` on `bibliography_publisher_pending`
//! instead of confirming a success the archive never received. That is also
//! why [`BibliographySyncExecutor::production`] exists but is not
//! registered in the scheduler startup: registering it now would claim
//! admitted work only to park every library blocked. E2b-3 registers it
//! beside the OCR/embedding engines once its publisher lands. Confirmed
//! pages survive as ordinary queue checkpoints (complete, checksummed page
//! units) so a resume or an E2b-3 upgrade can build on them; no catalog
//! table is written here.
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
//!
//! No message claims Zotero is closed or not installed: nothing observable
//! here distinguishes those, and the writing workspace's diagnosis rules
//! (§11.3) forbid the assertion.

use std::pin::Pin;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::open::open_archive_connection;
use crate::processing::repository::BIBLIOGRAPHY_SYNC_CONTRACT;
use crate::processing::scheduler::{
    ClaimedTask, ExecCtx, ExecOutput, ExecResult, Executor, NewCheckpoint, StopFlag,
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
    /// The row exists and names an addressable local-API library.
    Present(Library),
    /// No such row (or no table): the subject is gone from the catalog.
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
    let stored: Option<(String, String)> = conn
        .query_row(
            "SELECT library_type, library_id FROM zotero_libraries WHERE id = ?1",
            [library_row_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("no such table") {
                // Pre-0038 archive: there is no bibliography catalog to sync.
                return "library_absent".to_string();
            }
            format!("Failed to read Zotero library {library_row_id}: {message}")
        })?;
    let Some((library_type, library_id)) = stored else {
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
        Ok(library) => Ok(LibrarySubject::Present(library)),
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

/// One staged page as queue checkpoint payload: a complete, checksummed unit
/// E2b-3's durable page transactions can build on. Checkpoint-compatible
/// only — nothing here writes the catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PageCheckpoint {
    start: u32,
    items: Vec<BibliographyPageItem>,
    library_version: Option<u64>,
}

/// The E2b-2 bibliographic sync engine: resolves the claimed subject, walks
/// the mirrored library page by page, and reports honest verdicts. Holds no
/// queue state and writes no queue or catalog table — the supervisor
/// persists the staged checkpoints and publishes the verdict.
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

    /// The production engine over the local Zotero API, exposed for E2b-3 to
    /// register beside the OCR/embedding executors once the per-page
    /// publisher exists. Deliberately NOT registered by this build's
    /// scheduler startup: a claimed task would enumerate its whole library
    /// and park blocked on `bibliography_publisher_pending`, burning local
    /// API traffic to change queue state a human did not ask for.
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

    fn verdict(&self, checkpoints: Vec<NewCheckpoint>, output: ExecOutput) -> ExecResult {
        ExecResult {
            checkpoints,
            progress_total: None,
            engine_output: None,
            output,
        }
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
        let library = match resolve_library_subject(&ctx.db_path, &task.subject_id) {
            Ok(LibrarySubject::Present(library)) => library,
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
            Err(error) if error == "library_absent" => {
                return self.fatal(
                    "library_missing",
                    format!(
                        "library {} no longer exists in the bibliography catalog",
                        task.subject_id
                    ),
                )
            }
            Err(error) => return self.fatal("storage_unavailable", error),
        };

        let mut checkpoints: Vec<NewCheckpoint> = Vec::new();
        let mut progress_total: Option<i64> = None;
        let mut start: u32 = 0;
        loop {
            // Observed before every request: once stopped, no next request.
            if stop.stopped() {
                return self.verdict(checkpoints, ExecOutput::Stopped);
            }
            let query = BibliographyPageQuery::new(start, self.page_limit);
            // One page in flight, resolved on this thread: an in-flight
            // request is never preempted, only the next one is withheld.
            let page = match tauri::async_runtime::block_on(self.source.fetch_page(&library, query))
            {
                Ok(page) => page,
                Err(state) => {
                    return self.verdict(checkpoints, endpoint_verdict(&state));
                }
            };
            if empty_while_total_remains(page.items.len(), start, page.total) {
                return self.verdict(
                    checkpoints,
                    ExecOutput::Retryable {
                        code: "zotero_invalid_response".to_string(),
                        message: format!(
                            "the library's page at start {start} answered no items while its total says works remain"
                        ),
                    },
                );
            }
            if page.items.len() > self.page_limit as usize {
                return self.verdict(
                    checkpoints,
                    ExecOutput::Retryable {
                        code: "zotero_invalid_response".to_string(),
                        message: format!(
                            "the library's page at start {start} answered {} items, more than the {} asked for",
                            page.items.len(),
                            self.page_limit
                        ),
                    },
                );
            }
            if progress_total.is_none() {
                progress_total = page.total.map(|total| total as i64);
            }
            if page.items.is_empty() {
                break;
            }
            let payload = serde_json::to_string(&PageCheckpoint {
                start,
                items: page.items.clone(),
                library_version: page.library_version,
            })
            .unwrap_or_else(|error| format!("{{\"serialize_failed\":\"{error}\"}}"));
            checkpoints.push(NewCheckpoint {
                unit_key: format!("page:{start}"),
                input_fingerprint: task.input_fingerprint.clone(),
                contract_hash: task.contract_hash.clone(),
                payload_checksum: format!("{:x}", Sha256::digest(payload.as_bytes())),
                payload,
            });
            match pagination_next(start, page.items.len(), self.page_limit, page.total) {
                Some(next) => start = next,
                None => break,
            }
        }
        // Observed after the last page too: a run whose demand vanished at
        // the end reports Stopped, not a completion verdict.
        if stop.stopped() {
            return self.verdict(checkpoints, ExecOutput::Stopped);
        }
        // E2b-2 has no catalog publisher: a full enumeration parks blocked
        // on an explicit code instead of confirming a receipt the archive
        // never received. E2b-3 replaces this with per-page durable
        // transactions and a real success receipt.
        let pages_read = checkpoints.len();
        ExecResult {
            checkpoints,
            progress_total,
            engine_output: None,
            output: ExecOutput::Blocked {
                code: "bibliography_publisher_pending".to_string(),
                message: format!(
                    "library {} enumerated in {} page(s); the durable catalog publisher arrives in E2b-3, so this build parks the sync instead of confirming it",
                    task.subject_id,
                    pages_read
                ),
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
