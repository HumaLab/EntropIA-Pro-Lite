//! Read-only Internet Archive commands: identifier resolution, full-text
//! search, collection browsing, and item metadata.
//!
//! The `#[tauri::command]` functions below are thin wrappers over the plain
//! `pub(crate)` logic functions next to them, which take `&IaClient` /
//! `&Endpoints` directly so the logic stays testable without a Tauri handle
//! or the network. Only identifiers travel as parameters — never a URL or a
//! path — and [`ia_resolve_identifier`] returns an identifier, never a URL.

use internet_archive::{
    Availability, Endpoints, FileSource, IaClient, IaError, ItemDetails, PdfFile, RestrictedReason,
    SearchPage, SearchQuery,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tauri::{Manager as _, State};

use super::provenance_db::ProvenanceSnapshot;
use super::IaState;
use crate::db::state::AppDbState;
use internet_archive::job::{ImportTarget, Job, JobFile};
use internet_archive::prepare::{summarize, DuplicateStatus, PrepareSummary};
use internet_archive::provenance::ItemSnapshot;
use internet_archive::store::{new_job_id, JobStore};

use super::runner::{apply_end_of_run_cancel, decide_start, run_job, update, LiveDownloader};
use internet_archive::download::DownloadLimits;
use internet_archive::job::{FileState, JobState};
use internet_archive::store::{is_valid_job_id, StoreError};
use internet_archive::verify::PageLimits;

/// Structured search filters, as the frontend sends them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQueryDto {
    /// Free text: split on whitespace, each term a quoted literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Literal matched against the `title` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Literal matched against the `creator` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// Literal matched against the `language` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Collection identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// Inclusive lower year bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_from: Option<u16>,
    /// Inclusive upper year bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_to: Option<u16>,
    /// When true (default), the query also requires `mediatype:texts`.
    #[serde(default = "default_texts_only")]
    pub texts_only: bool,
}

fn default_texts_only() -> bool {
    true
}

impl Default for SearchQueryDto {
    fn default() -> Self {
        Self {
            text: None,
            title: None,
            creator: None,
            language: None,
            collection: None,
            year_from: None,
            year_to: None,
            texts_only: true,
        }
    }
}

/// Blank input is no input: whitespace-only filters do not count as a search
/// clause, and an empty collection must not fail identifier validation.
fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl SearchQueryDto {
    pub(crate) fn to_search_query(&self) -> SearchQuery {
        SearchQuery {
            text: non_empty(&self.text),
            title: non_empty(&self.title),
            creator: non_empty(&self.creator),
            language: non_empty(&self.language),
            collection: non_empty(&self.collection),
            year_from: self.year_from,
            year_to: self.year_to,
            texts_only: self.texts_only,
        }
    }
}

/// One normalized search hit, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchItemDto {
    pub identifier: String,
    pub title: Option<String>,
    pub creators: Vec<String>,
    pub date: Option<String>,
    pub collections: Vec<String>,
    pub restricted: bool,
}

/// One page of search results, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPageDto {
    pub num_found: u64,
    pub items: Vec<SearchItemDto>,
}

impl From<internet_archive::SearchItem> for SearchItemDto {
    fn from(item: internet_archive::SearchItem) -> Self {
        Self {
            identifier: item.identifier,
            title: item.title,
            creators: item.creators,
            date: item.date,
            collections: item.collections,
            restricted: item.restricted,
        }
    }
}

impl From<SearchPage> for SearchPageDto {
    fn from(page: SearchPage) -> Self {
        Self {
            num_found: page.num_found,
            items: page.items.into_iter().map(SearchItemDto::from).collect(),
        }
    }
}

/// One PDF-ish file entry, as the frontend reads it. `availability` is a
/// plain string tag (see [`availability_tag`]), never a nested object.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfFileDto {
    pub name: String,
    pub format: Option<String>,
    pub source: String,
    pub size: Option<u64>,
    pub mtime: Option<u64>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub availability: String,
}

/// Normalized item metadata with its PDF file list, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetailsDto {
    pub identifier: String,
    pub title: Option<String>,
    pub creators: Vec<String>,
    pub date: Option<String>,
    pub languages: Vec<String>,
    pub collections: Vec<String>,
    pub rights: Option<String>,
    pub license_url: Option<String>,
    pub mediatype: Option<String>,
    pub restricted: bool,
    pub pdfs: Vec<PdfFileDto>,
    pub truncated: bool,
}

impl From<internet_archive::PdfFile> for PdfFileDto {
    fn from(file: internet_archive::PdfFile) -> Self {
        Self {
            availability: availability_tag(&file.availability).to_string(),
            name: file.name,
            format: file.format,
            source: file_source_tag(&file.source),
            size: file.size,
            mtime: file.mtime,
            md5: file.md5,
            sha1: file.sha1,
        }
    }
}

impl From<ItemDetails> for ItemDetailsDto {
    fn from(details: ItemDetails) -> Self {
        Self {
            identifier: details.identifier,
            title: details.title,
            creators: details.creators,
            date: details.date,
            languages: details.languages,
            collections: details.collections,
            rights: details.rights,
            license_url: details.license_url,
            mediatype: details.mediatype,
            restricted: details.restricted,
            pdfs: details.pdfs.into_iter().map(PdfFileDto::from).collect(),
            truncated: details.truncated,
        }
    }
}

/// Availability as a stable string tag for the frontend: `available`,
/// `encrypted`, `private`, `item_restricted`, `unsafe_name`.
pub(crate) fn availability_tag(availability: &Availability) -> &'static str {
    match availability {
        Availability::Available => "available",
        Availability::Restricted(RestrictedReason::Encrypted) => "encrypted",
        Availability::Restricted(RestrictedReason::Private) => "private",
        Availability::Restricted(RestrictedReason::ItemRestricted) => "item_restricted",
        Availability::Restricted(RestrictedReason::UnsafeName) => "unsafe_name",
    }
}

/// Declared `source` bucket as a plain string: `original`, `derivative`,
/// the published value verbatim, or `unknown`.
pub(crate) fn file_source_tag(source: &FileSource) -> String {
    match source {
        FileSource::Original => "original".to_string(),
        FileSource::Derivative => "derivative".to_string(),
        FileSource::Other(name) => name.clone(),
        FileSource::Unknown => "unknown".to_string(),
    }
}

/// Stable machine code for an [`IaError`]: `invalid_query`, `not_found`,
/// `blocked`, `too_many_redirects`, `too_large`, `http_<status>`,
/// `network`, `decode`, `already_exists`, `io`, `verify_failed`,
/// `too_many_pages`.
pub(crate) fn ia_error_code(error: &IaError) -> String {
    match error {
        IaError::InvalidQuery { .. } => "invalid_query".to_string(),
        IaError::NotFound { .. } => "not_found".to_string(),
        IaError::Blocked { .. } => "blocked".to_string(),
        IaError::TooManyRedirects => "too_many_redirects".to_string(),
        IaError::TooLarge { .. } | IaError::FileTooLarge { .. } => "too_large".to_string(),
        IaError::Http { status, .. } => format!("http_{status}"),
        IaError::Network(_) => "network".to_string(),
        IaError::Decode(_) => "decode".to_string(),
        IaError::AlreadyExists { .. } => "already_exists".to_string(),
        IaError::Io(_) => "io".to_string(),
        IaError::VerifyFailed { .. } => "verify_failed".to_string(),
        IaError::TooManyPages { .. } => "too_many_pages".to_string(),
    }
}

/// An [`IaError`] as `"<code>: <message>"`, like the nearby commands report
/// errors. Staging paths never reach the message: the variants that carry
/// one ([`IaError::AlreadyExists`]) or might ([`IaError::Io`]) are replaced
/// with a generic message.
pub(crate) fn ia_error(error: &IaError) -> String {
    let code = ia_error_code(error);
    let message = match error {
        // The crate reports the staging destination here; paths stay private.
        IaError::AlreadyExists { .. } => "already exists".to_string(),
        // Never forwarded verbatim: some producers format paths into it.
        IaError::Io(_) => "local storage error".to_string(),
        IaError::Http {
            status,
            retry_after,
        } => match retry_after {
            Some(delay) => format!("http error {status} (retry after {}s)", delay.as_secs()),
            None => format!("http error {status}"),
        },
        other => other.to_string(),
    };
    format!("{code}: {message}")
}

/// Resolve user input (a bare identifier or an archive.org URL) to a bare
/// identifier. Returns an identifier, never a URL.
pub(crate) fn resolve_identifier_logic(input: &str) -> Result<String, String> {
    internet_archive::resolve_identifier(input).map_err(|error| ia_error(&error))
}

/// Run a structured search over `page` (1-based) with `rows` per page.
pub(crate) async fn search_logic(
    client: &IaClient,
    endpoints: &Endpoints,
    query: &SearchQueryDto,
    page: u32,
    rows: u32,
) -> Result<SearchPageDto, String> {
    client
        .search(endpoints, &query.to_search_query(), page, rows)
        .await
        .map(SearchPageDto::from)
        .map_err(|error| ia_error(&error))
}

/// List one page of a collection (identifier only, never a URL or a path).
pub(crate) async fn list_collection_logic(
    client: &IaClient,
    endpoints: &Endpoints,
    collection: &str,
    page: u32,
    rows: u32,
) -> Result<SearchPageDto, String> {
    client
        .list_collection(endpoints, collection, page, rows)
        .await
        .map(SearchPageDto::from)
        .map_err(|error| ia_error(&error))
}

/// Fetch tolerant item metadata with its PDF availability (identifier only).
pub(crate) async fn get_item_logic(
    client: &IaClient,
    endpoints: &Endpoints,
    identifier: &str,
) -> Result<ItemDetailsDto, String> {
    client
        .get_item(endpoints, identifier)
        .await
        .map(ItemDetailsDto::from)
        .map_err(|error| ia_error(&error))
}

/// Resolve user input (a bare identifier or an archive.org URL) to a bare
/// identifier. Returns an identifier, never a URL.
#[tauri::command]
pub async fn ia_resolve_identifier(input: String) -> Result<String, String> {
    resolve_identifier_logic(&input)
}

/// Run a structured search over `page` (1-based) with `rows` per page.
#[tauri::command]
pub async fn ia_search(
    query: SearchQueryDto,
    page: u32,
    rows: u32,
    state: State<'_, IaState>,
) -> Result<SearchPageDto, String> {
    search_logic(&state.client, &state.endpoints, &query, page, rows).await
}

/// List one page of a collection (identifier only, never a URL or a path).
#[tauri::command]
pub async fn ia_list_collection(
    collection: String,
    page: u32,
    rows: u32,
    state: State<'_, IaState>,
) -> Result<SearchPageDto, String> {
    list_collection_logic(&state.client, &state.endpoints, &collection, page, rows).await
}

/// Fetch tolerant item metadata with its PDF availability (identifier only).
#[tauri::command]
pub async fn ia_get_item(
    identifier: String,
    state: State<'_, IaState>,
) -> Result<ItemDetailsDto, String> {
    get_item_logic(&state.client, &state.endpoints, &identifier).await
}

// ---------------------------------------------------------------------------
// Import preparation (task 21b)
// ---------------------------------------------------------------------------
//
// A job always targets an EXISTING collection id; the frontend creates a new
// collection first through the existing store service, so only
// [`ImportTarget::ExistingCollection`] is ever built here. Only identifiers
// travel as parameters — never a URL or a path.

/// Selection limits for one preparation: 1..=100 selections, 1..=1000 files.
const MAX_PREPARE_SELECTIONS: usize = 100;
/// Total files across all selections in one preparation.
const MAX_PREPARE_FILES: usize = 1000;

/// One requested `(identifier, files)` pair, as the frontend sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionDto {
    /// Bare item identifier (validated against the crate validator).
    pub identifier: String,
    /// Requested file names within the item.
    pub file_names: Vec<String>,
}

/// Duplicate classification of one prepared file, as the frontend reads it:
/// `{ "status": "new" }`, `{ "status": "already_in_target", "itemId" }`,
/// `{ "status": "new_version", "previousItemId" }`, or
/// `{ "status": "in_other_collection", "itemId", "collectionId" }`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DuplicateStatusDto {
    New,
    AlreadyInTarget {
        #[serde(rename = "itemId")]
        item_id: String,
    },
    NewVersion {
        #[serde(rename = "previousItemId")]
        previous_item_id: String,
    },
    InOtherCollection {
        #[serde(rename = "itemId")]
        item_id: String,
        #[serde(rename = "collectionId")]
        collection_id: String,
    },
}

impl From<&DuplicateStatus> for DuplicateStatusDto {
    fn from(status: &DuplicateStatus) -> Self {
        match status {
            DuplicateStatus::New => Self::New,
            DuplicateStatus::AlreadyInTarget { item_id } => Self::AlreadyInTarget {
                item_id: item_id.clone(),
            },
            DuplicateStatus::NewVersion { previous_item_id } => Self::NewVersion {
                previous_item_id: previous_item_id.clone(),
            },
            DuplicateStatus::InOtherCollection {
                item_id,
                collection_id,
            } => Self::InOtherCollection {
                item_id: item_id.clone(),
                collection_id: collection_id.clone(),
            },
        }
    }
}

/// One row of the preparation table, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareEntryDto {
    pub identifier: String,
    pub file_name: String,
    /// Availability tag (`available` here: only importable files are listed).
    pub availability: String,
    pub size: Option<u64>,
    /// Duplicate classification against the local provenance.
    pub duplicate: Option<DuplicateStatusDto>,
}

impl From<&internet_archive::prepare::PrepareEntry> for PrepareEntryDto {
    fn from(entry: &internet_archive::prepare::PrepareEntry) -> Self {
        Self {
            identifier: entry.identifier.clone(),
            file_name: entry.file_name.clone(),
            availability: availability_tag(&entry.availability).to_string(),
            size: entry.size,
            duplicate: entry.duplicate.as_ref().map(DuplicateStatusDto::from),
        }
    }
}

/// Counts and rows for the preparation screen, as the frontend reads them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareSummaryDto {
    pub total_files: usize,
    pub available_files: usize,
    pub restricted_files: usize,
    pub known_bytes: u64,
    pub unknown_size_files: usize,
    pub new_files: usize,
    pub already_in_target: usize,
    pub in_other_collection: usize,
    pub new_version: usize,
    pub entries: Vec<PrepareEntryDto>,
}

impl From<&PrepareSummary> for PrepareSummaryDto {
    fn from(summary: &PrepareSummary) -> Self {
        Self {
            total_files: summary.total_files,
            available_files: summary.available_files,
            restricted_files: summary.restricted_files,
            known_bytes: summary.known_bytes,
            unknown_size_files: summary.unknown_size_files,
            new_files: summary.new_files,
            already_in_target: summary.already_in_target,
            in_other_collection: summary.in_other_collection,
            new_version: summary.new_version,
            entries: summary.entries.iter().map(PrepareEntryDto::from).collect(),
        }
    }
}

/// One requested file that did not enter the job, as the frontend reads it.
/// `reason` is `not_found` or `unavailable:<availability-tag>`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedDto {
    pub identifier: String,
    pub file_name: String,
    pub reason: String,
}

/// The preparation result, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareResultDto {
    pub job_id: String,
    pub summary: PrepareSummaryDto,
    pub rejected: Vec<RejectedDto>,
}

/// Item metadata source for import preparation. The production
/// implementation reads through [`IaClient::get_item`]; tests substitute
/// fixture maps, so no prepare test touches the network.
pub(crate) trait ItemFetcher {
    async fn fetch_item(&self, identifier: &str) -> Result<ItemDetails, String>;
}

/// Production fetcher: [`IaClient::get_item`] with the shared
/// `"<code>: <message>"` error shape.
pub(crate) struct LiveFetcher<'a> {
    pub client: &'a IaClient,
    pub endpoints: &'a Endpoints,
}

impl ItemFetcher for LiveFetcher<'_> {
    async fn fetch_item(&self, identifier: &str) -> Result<ItemDetails, String> {
        self.client
            .get_item(self.endpoints, identifier)
            .await
            .map_err(|error| ia_error(&error))
    }
}

/// Prepare an import job over an existing collection: validate the request,
/// open the provenance snapshot (synchronously, under the caller's SQLite
/// lock — never held across an `.await`), fetch each identifier once in
/// order, include every requested `Available` PDF via [`JobFile::from_pdf`],
/// report the rest as `rejected`, persist the job with [`JobStore::create`],
/// and return the job id with its [`summarize`] summary. Fails with
/// `nothing_to_import` (creating no job) when no requested file is
/// available.
///
/// Generic over the fetcher so tests inject fixture metadata without the
/// network; [`prepare_import_logic`] below is the production entry point
/// that binds `&IaClient` / `&Endpoints`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_import_with_fetcher<F>(
    fetcher: &F,
    store: &JobStore,
    open_provenance: impl FnOnce(&[String]) -> Result<ProvenanceSnapshot, String>,
    now: u64,
    collection_id: &str,
    selections: &[SelectionDto],
) -> Result<PrepareResultDto, String>
where
    F: ItemFetcher,
{
    if selections.is_empty() || selections.len() > MAX_PREPARE_SELECTIONS {
        return Err(format!(
            "invalid_request: expected 1..={} selections, got {}",
            MAX_PREPARE_SELECTIONS,
            selections.len()
        ));
    }
    let total_files: usize = selections.iter().map(|s| s.file_names.len()).sum();
    if total_files == 0 || total_files > MAX_PREPARE_FILES {
        return Err(format!(
            "invalid_request: expected 1..={MAX_PREPARE_FILES} files in total, got {total_files}"
        ));
    }
    for selection in selections {
        if selection.file_names.is_empty() {
            return Err(format!(
                "invalid_request: selection for {:?} names no files",
                selection.identifier
            ));
        }
        // The crate validator only accepts bare identifiers: anything else
        // (a URL, garbage) either fails or resolves to a different string,
        // and both cases are rejected here.
        let valid = internet_archive::resolve_identifier(&selection.identifier)
            .ok()
            .is_some_and(|resolved| resolved == selection.identifier);
        if !valid {
            return Err(format!(
                "invalid_request: invalid identifier {:?}",
                selection.identifier
            ));
        }
    }
    {
        let mut seen = HashSet::new();
        for selection in selections {
            for file_name in &selection.file_names {
                if !seen.insert((selection.identifier.as_str(), file_name.as_str())) {
                    return Err(format!(
                        "invalid_request: duplicate file {file_name:?} for {:?}",
                        selection.identifier
                    ));
                }
            }
        }
    }
    // Unique identifiers in first-seen order: one `get_item` each, even when
    // several selections share an identifier.
    let mut identifiers = Vec::new();
    {
        let mut seen = HashSet::new();
        for selection in selections {
            if seen.insert(selection.identifier.as_str()) {
                identifiers.push(selection.identifier.clone());
            }
        }
    }
    // Synchronous SQLite section, opened under the caller's lock: existence
    // plus provenance load, then the guard drops before any `.await` below.
    let snapshot = open_provenance(&identifiers)?;
    if !snapshot.collection_exists {
        return Err(format!("collection_not_found: {collection_id:?}"));
    }
    // Sequential fetches; a fetcher error already carries its own code.
    let mut details_by_id = HashMap::with_capacity(identifiers.len());
    for identifier in &identifiers {
        let details = fetcher.fetch_item(identifier).await?;
        details_by_id.insert(identifier.clone(), details);
    }
    let target = ImportTarget::ExistingCollection {
        collection_id: collection_id.to_string(),
    };
    let mut job_files = Vec::new();
    let mut summarize_inputs: Vec<(String, PdfFile)> = Vec::new();
    let mut rejected = Vec::new();
    let mut snapshots: Vec<ItemSnapshot> = Vec::new();
    let mut snapshotted = HashSet::new();
    for selection in selections {
        // Every selection identifier was fetched above.
        let details = details_by_id
            .get(&selection.identifier)
            .expect("prepare fetched every selected identifier");
        for file_name in &selection.file_names {
            match details.pdfs.iter().find(|pdf| pdf.name == *file_name) {
                Some(pdf) if pdf.availability == Availability::Available => {
                    // Unreachable in practice: availability was just checked.
                    let job_file = JobFile::from_pdf(&selection.identifier, pdf)
                        .map_err(|error| format!("invalid_request: {error}"))?;
                    job_files.push(job_file);
                    summarize_inputs.push((selection.identifier.clone(), pdf.clone()));
                    // One snapshot per identifier that contributed a file.
                    if snapshotted.insert(selection.identifier.clone()) {
                        snapshots.push(ItemSnapshot::from(details));
                    }
                }
                Some(pdf) => rejected.push(RejectedDto {
                    identifier: selection.identifier.clone(),
                    file_name: file_name.clone(),
                    reason: format!("unavailable:{}", availability_tag(&pdf.availability)),
                }),
                None => rejected.push(RejectedDto {
                    identifier: selection.identifier.clone(),
                    file_name: file_name.clone(),
                    reason: "not_found".to_string(),
                }),
            }
        }
    }
    if job_files.is_empty() {
        return Err("nothing_to_import: no requested file is available for import".to_string());
    }
    // Non-empty and de-duplicated by the validation above.
    let mut job = Job::new(new_job_id(), now, target.clone(), job_files)
        .map_err(|error| format!("invalid_request: {error}"))?;
    job.items = snapshots;
    let summary = summarize(&summarize_inputs, &target, &snapshot.provenance);
    store
        .create(&job)
        .map_err(|error| format!("io: cannot persist the import job: {error}"))?;
    Ok(PrepareResultDto {
        job_id: job.id.clone(),
        summary: PrepareSummaryDto::from(&summary),
        rejected,
    })
}

/// Production entry point for import preparation: the plain logic over
/// `&IaClient` / `&Endpoints`, testable without a Tauri handle. Binds the
/// live fetcher and delegates to [`prepare_import_with_fetcher`].
#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_import_logic(
    client: &IaClient,
    endpoints: &Endpoints,
    store: &JobStore,
    open_provenance: impl FnOnce(&[String]) -> Result<ProvenanceSnapshot, String>,
    now: u64,
    collection_id: &str,
    selections: &[SelectionDto],
) -> Result<PrepareResultDto, String> {
    let fetcher = LiveFetcher { client, endpoints };
    prepare_import_with_fetcher(
        &fetcher,
        store,
        open_provenance,
        now,
        collection_id,
        selections,
    )
    .await
}

/// Prepare an import job over an EXISTING collection id: validate the
/// selections, fetch item metadata, classify against the local provenance,
/// persist the job, and return its id with the summary and rejections.
/// Only identifiers travel as parameters — never a URL or a path.
#[tauri::command]
pub async fn ia_prepare_import(
    collection_id: String,
    selections: Vec<SelectionDto>,
    ia: State<'_, IaState>,
    db: State<'_, AppDbState>,
) -> Result<PrepareResultDto, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    prepare_import_logic(
        &ia.client,
        &ia.endpoints,
        &ia.store,
        |identifiers| {
            // Locked, queried, and dropped before the first `.await`: the
            // mutex is never held across the network below.
            let conn = db
                .ui_conn
                .lock()
                .map_err(|_| "io: local storage error".to_string())?;
            ProvenanceSnapshot::open(&conn, &collection_id, identifiers)
                .map_err(|error| format!("db: {error}"))
        },
        now,
        &collection_id,
        &selections,
    )
    .await
}

// ---------------------------------------------------------------------------
// Job runner (task 22)
//
// Thin wrappers over the testable [`run_job`] core: commands take only a job
// id (validated by the store) and file indexes in order — never a path or a
// URL. Every state change is persisted with `JobStore::save` and then
// reported as `ia://job-updated` with `{ jobId }` (best effort, logged).
// ---------------------------------------------------------------------------

/// One file of an import job, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFileStatusDto {
    pub index: usize,
    pub identifier: String,
    pub file_name: String,
    pub display_file_name: String,
    pub state: FileState,
    pub attempts: u32,
    pub error: Option<String>,
    pub expected_size: Option<u64>,
    pub page_count: Option<u32>,
}

/// An import job with its files, as the frontend reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatusDto {
    pub id: String,
    pub state: JobState,
    pub target_collection_id: Option<String>,
    pub files: Vec<JobFileStatusDto>,
    pub running: bool,
}

/// A resumable job for the "Retomar importación" list: only non-terminal
/// jobs are listed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummaryDto {
    pub id: String,
    pub state: JobState,
    pub target_collection_id: Option<String>,
    pub total_files: usize,
    pub verified_files: usize,
}

/// One verified file ready for the frontend import (task 23).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedFileDto {
    pub index: usize,
    pub identifier: String,
    pub file_name: String,
    pub display_file_name: String,
    pub item_title: String,
    pub staging_path: String,
    pub page_count: u32,
    pub sha256: String,
    pub extra_metadata: serde_json::Value,
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub(crate) fn emit_job_updated(app: &tauri::AppHandle, job_id: &str) {
    use tauri::Emitter as _;
    if let Err(error) = app.emit("ia://job-updated", serde_json::json!({ "jobId": job_id })) {
        log::warn!(target: "ia", "job-updated emit failed for {job_id}: {error}");
    }
}

pub(crate) fn job_load_error(error: StoreError) -> String {
    match error {
        StoreError::NotFound(_) => "not_found: import job not found".to_string(),
        StoreError::InvalidId(_) => "invalid_request: invalid job id".to_string(),
        _ => "io: local storage error".to_string(),
    }
}

pub(crate) fn target_collection_id(job: &Job) -> Option<String> {
    match &job.target {
        ImportTarget::ExistingCollection { collection_id } => Some(collection_id.clone()),
        ImportTarget::NewCollection { .. } => None,
    }
}

pub(crate) fn job_to_status(job: &Job, running: bool) -> JobStatusDto {
    let files = job
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| JobFileStatusDto {
            index,
            identifier: file.identifier.clone(),
            file_name: file.file_name.clone(),
            display_file_name: internet_archive::display_file_name(
                &file.file_name,
                &file.identifier,
            ),
            state: file.state,
            attempts: file.attempts,
            error: file.error.clone(),
            expected_size: file.expected_size,
            page_count: file.page_count,
        })
        .collect();
    JobStatusDto {
        id: job.id.clone(),
        state: job.state,
        target_collection_id: target_collection_id(job),
        files,
        running,
    }
}

/// One `Verified` file of `job` as the frontend import reads it: absolute
/// staging path plus the provenance value passed unchanged as
/// `extraMetadata`. Shared by [`verified_files_logic`] and the task-23
/// begin handoff so both hand the frontend the same payload.
pub(crate) fn verified_file_entry(
    store: &JobStore,
    job: &Job,
    index: usize,
    file: &JobFile,
    now: u64,
) -> Result<VerifiedFileDto, String> {
    let staging_name = file
        .staging_name
        .clone()
        .ok_or_else(|| "io: verified file has no staging file".to_string())?;
    let sha256 = file
        .sha256
        .clone()
        .filter(|hash| !hash.trim().is_empty())
        .ok_or_else(|| "io: verified file has no hash".to_string())?;
    let page_count = file
        .page_count
        .ok_or_else(|| "io: verified file has no page count".to_string())?;
    let dest = store
        .staging_path(&job.id, &staging_name)
        .map_err(|_| "io: local storage error".to_string())?;
    let display = internet_archive::display_file_name(&file.file_name, &file.identifier);
    let item_title = job
        .item(&file.identifier)
        .and_then(|snapshot| snapshot.title.clone())
        .unwrap_or_else(|| display.strip_suffix(".pdf").unwrap_or(&display).to_string());
    let provenance = internet_archive::build_provenance(file, job.item(&file.identifier), now)
        .map_err(|_| "io: cannot build provenance".to_string())?;
    Ok(VerifiedFileDto {
        index,
        identifier: file.identifier.clone(),
        file_name: file.file_name.clone(),
        display_file_name: display,
        item_title,
        staging_path: dest.display().to_string(),
        page_count,
        sha256,
        extra_metadata: serde_json::json!({
            internet_archive::PROVENANCE_KEY: provenance,
        }),
    })
}

/// Verified files of `job` with their absolute staging paths and the
/// provenance value the frontend passes unchanged as `extraMetadata`.
pub(crate) fn verified_files_logic(
    store: &JobStore,
    job: &Job,
    now: u64,
) -> Result<Vec<VerifiedFileDto>, String> {
    let mut out = Vec::new();
    for (index, file) in job.files.iter().enumerate() {
        if file.state != FileState::Verified {
            continue;
        }
        out.push(verified_file_entry(store, job, index, file, now)?);
    }
    Ok(out)
}

/// Non-terminal jobs for "Retomar importación": root directory names
/// validated with [`is_valid_job_id`] (no new crate API), loaded through
/// the store, terminal jobs skipped, unloadable entries skipped.
pub(crate) fn list_jobs_logic(store: &JobStore) -> Result<Vec<JobSummaryDto>, String> {
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
        if job.state.is_terminal() {
            continue;
        }
        let verified = job
            .files
            .iter()
            .filter(|file| file.state == FileState::Verified)
            .count();
        out.push(JobSummaryDto {
            id: job.id.clone(),
            state: job.state,
            target_collection_id: target_collection_id(&job),
            total_files: job.files.len(),
            verified_files: verified,
        });
    }
    Ok(out)
}

/// Start (or resume) a job download. One job at a time: rejected with
/// `job_running` while another job runs. Accepts `Draft` (`Ready` then
/// `Running`), `Ready` (`Running`), or `Running` without an active runner
/// (resume, no transition); anything else is `invalid_state`.
///
/// `active` is held for the whole check → load → transition → save →
/// register sequence (lock order `active` first, then `jobs` inside
/// [`update`]), so concurrent starts serialize with each other and with
/// cancels: no cancel in the window is overwritten by the start's save.
/// The runner clears the registry when it ends, also on error; a cancel
/// that arrives after the run but before the clear is applied at the end
/// of the run instead of being lost.
#[tauri::command]
pub fn ia_start_import(
    job_id: String,
    ia: State<'_, IaState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let mut active_guard = ia
        .active
        .lock()
        .map_err(|_| "io: local storage error".to_string())?;
    if active_guard.is_some() {
        return Err("job_running: another import is already running".to_string());
    }
    let (_, job) = update(&ia.store, &ia.jobs, &job_id, |job| {
        decide_start(job.state)?;
        let now = now_secs();
        match job.state {
            JobState::Draft => {
                job.set_job_state(JobState::Ready, now)
                    .map_err(|_| "invalid_state: cannot start this job".to_string())?;
                job.set_job_state(JobState::Running, now)
                    .map_err(|_| "invalid_state: cannot start this job".to_string())?;
            }
            JobState::Ready => {
                job.set_job_state(JobState::Running, now)
                    .map_err(|_| "invalid_state: cannot start this job".to_string())?;
            }
            JobState::Running => {}
            _ => return Err("invalid_state: cannot start this job".to_string()),
        }
        Ok(())
    })?;
    let (tx, rx) = tokio::sync::watch::channel(false);
    // Still holding `active`: no concurrent start or direct cancel can slip
    // between the save above and this registration.
    *active_guard = Some(super::ActiveRun {
        job_id: job_id.clone(),
        cancel: tx,
    });
    let rx_flag = rx.clone();
    drop(active_guard);
    emit_job_updated(&app, &job.id);
    let store = ia.store.clone();
    let client = ia.client.clone();
    let endpoints = ia.endpoints.clone();
    let app_clone = app.clone();
    let job_id_clone = job_id.clone();
    tauri::async_runtime::spawn(async move {
        let downloader = LiveDownloader {
            client: &client,
            endpoints: &endpoints,
            limits: DownloadLimits::clamped(None),
            retry: internet_archive::RetryPolicy::default(),
        };
        let clock = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0)
        };
        let on_update = |updated: &Job| {
            emit_job_updated(&app_clone, &updated.id);
        };
        // The writer lock lives in the managed state: borrow it for the
        // run (the borrow stays inside this task, which owns `app_clone`).
        // The runner itself only ever takes `jobs`, never `active`.
        let run_result = match app_clone.try_state::<IaState>() {
            Some(state) => {
                run_job(
                    &store,
                    &state.jobs,
                    &job_id_clone,
                    &downloader,
                    rx,
                    (DownloadLimits::clamped(None), PageLimits::clamped(None)),
                    &clock,
                    &on_update,
                )
                .await
            }
            None => Err("io: local storage error".to_string()),
        };
        if let Err(error) = run_result {
            log::warn!(target: "ia", "import runner for {job_id_clone} ended with error: {error}");
        }
        // End of run: `active` first, then `jobs` inside the helper. A
        // cancel that arrived after `run_job` returned but before this
        // clear still sees the active run when sent; if the flag is set
        // now, apply the same cancel effects instead of losing it.
        if let Some(state) = app_clone.try_state::<IaState>() {
            if let Ok(mut active) = state.active.lock() {
                let flag = *rx_flag.borrow();
                if flag {
                    let now = now_secs();
                    match apply_end_of_run_cancel(&store, &state.jobs, &job_id_clone, true, now) {
                        Ok((applied, _)) => {
                            if applied {
                                log::info!(target: "ia", "applied end-of-run cancel for {job_id_clone}");
                            }
                        }
                        Err(error) => {
                            log::warn!(target: "ia", "end-of-run cancel for {job_id_clone} failed: {error}");
                        }
                    }
                }
                if active
                    .as_ref()
                    .is_some_and(|run| run.job_id == job_id_clone)
                {
                    *active = None;
                }
            }
        }
        emit_job_updated(&app_clone, &job_id_clone);
    });
    Ok(())
}

/// Cancel a job download. When it is the active run the cancel signal is
/// sent and the runner finishes the cancellation; otherwise the job is
/// cancelled directly (same effects as the end-of-run path). Holds `active`
/// for the whole decision and takes `jobs` inside the helper for the
/// load → cancel → save (lock order `active` → `jobs`). Idempotent on
/// terminal jobs.
#[tauri::command]
pub fn ia_cancel_import(
    job_id: String,
    ia: State<'_, IaState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let active_guard = ia
        .active
        .lock()
        .map_err(|_| "io: local storage error".to_string())?;
    if let Some(run) = active_guard.as_ref() {
        if run.job_id == job_id {
            let _ = run.cancel.send(true);
            return Ok(());
        }
    }
    let (applied, job) = apply_end_of_run_cancel(&ia.store, &ia.jobs, &job_id, true, now_secs())?;
    drop(active_guard);
    if applied {
        emit_job_updated(&app, &job.id);
    }
    Ok(())
}

/// Remove one abandoned preparation draft: only a `Draft` job goes away,
/// anything else is refused and left untouched. Testable without a Tauri
/// handle; the command below only adds the lock ordering.
///
/// A draft never downloads, so removing its folder frees nothing but the
/// manifest itself. Unknown ids report `not_found` through [`job_load_error`].
pub(crate) fn discard_draft_logic(store: &JobStore, job_id: &str) -> Result<(), String> {
    let job = store.load(job_id).map_err(job_load_error)?;
    if job.state != JobState::Draft {
        return Err("invalid_state: import job is not a draft".to_string());
    }
    store.remove_job(job_id).map_err(job_load_error)
}

/// Discard an abandoned preparation draft (see [`discard_draft_logic`]).
/// Holds `active` for the whole decision and takes `jobs` for the
/// load → check → remove (lock order `active` → `jobs`, like the cancel
/// path). A draft that is somehow the active run is refused: starting flips
/// a job out of `Draft` first, so that case never happens in practice.
/// Never touches a job once `ia_start_import` has accepted it.
#[tauri::command]
pub fn ia_discard_draft(job_id: String, ia: State<'_, IaState>) -> Result<(), String> {
    let active_guard = ia
        .active
        .lock()
        .map_err(|_| "io: local storage error".to_string())?;
    if active_guard
        .as_ref()
        .is_some_and(|run| run.job_id == job_id)
    {
        return Err("invalid_state: import job is running".to_string());
    }
    let _jobs_guard = ia
        .jobs
        .lock()
        .map_err(|_| "io: local storage error".to_string())?;
    discard_draft_logic(&ia.store, &job_id)
}

/// Current job state with its files, plus whether a runner works on it.
#[tauri::command]
pub fn ia_get_import_status(
    job_id: String,
    ia: State<'_, IaState>,
) -> Result<JobStatusDto, String> {
    let job = ia.store.load(&job_id).map_err(job_load_error)?;
    let running = ia
        .active
        .lock()
        .map(|active| active.as_ref().is_some_and(|run| run.job_id == job_id))
        .unwrap_or(false);
    Ok(job_to_status(&job, running))
}

/// Non-terminal jobs for "Retomar importación".
#[tauri::command]
pub fn ia_list_jobs(ia: State<'_, IaState>) -> Result<Vec<JobSummaryDto>, String> {
    list_jobs_logic(&ia.store)
}

/// Verified files of a job with absolute staging paths and provenance
/// `extraMetadata` for the frontend import.
#[tauri::command]
pub fn ia_list_verified_files(
    job_id: String,
    ia: State<'_, IaState>,
) -> Result<Vec<VerifiedFileDto>, String> {
    let job = ia.store.load(&job_id).map_err(job_load_error)?;
    verified_files_logic(&ia.store, &job, now_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn available_pdf(name: &str) -> internet_archive::PdfFile {
        internet_archive::PdfFile {
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

    #[test]
    fn availability_tags_cover_every_restriction_reason() {
        let cases = [
            (Availability::Available, "available"),
            (
                Availability::Restricted(RestrictedReason::Encrypted),
                "encrypted",
            ),
            (
                Availability::Restricted(RestrictedReason::Private),
                "private",
            ),
            (
                Availability::Restricted(RestrictedReason::ItemRestricted),
                "item_restricted",
            ),
            (
                Availability::Restricted(RestrictedReason::UnsafeName),
                "unsafe_name",
            ),
        ];
        for (availability, expected) in cases {
            assert_eq!(availability_tag(&availability), expected);
        }
    }

    #[test]
    fn pdf_dto_carries_the_availability_tag_and_source_string() {
        let dto = PdfFileDto::from(available_pdf("doc.pdf"));
        let value = serde_json::to_value(&dto).expect("dto serializes");
        assert_eq!(value["availability"], "available");
        assert_eq!(value["source"], "original");
        assert_eq!(value["name"], "doc.pdf");
        assert_eq!(value["size"], 100);
        // camelCase keys, no snake_case remnants.
        assert!(value.get("availability").is_some());
        assert!(value.get("file_name").is_none());
    }

    #[test]
    fn query_dto_uses_camel_case_keys_and_defaults_texts_only_on() {
        let dto = SearchQueryDto {
            text: Some("Mar del Plata".to_string()),
            year_from: Some(1990),
            ..SearchQueryDto::default()
        };
        let value = serde_json::to_value(&dto).expect("dto serializes");
        assert_eq!(value["text"], "Mar del Plata");
        assert_eq!(value["yearFrom"], 1990);
        assert_eq!(value["textsOnly"], true);
        assert!(value.get("year_from").is_none());
        assert!(value.get("texts_only").is_none());

        // A missing `textsOnly` still means texts only.
        let parsed: SearchQueryDto = serde_json::from_value(serde_json::json!({
            "text": "x",
        }))
        .expect("dto deserializes");
        assert!(parsed.texts_only);
        assert_eq!(parsed.year_from, None);
    }

    #[test]
    fn search_page_dto_reports_num_found_in_camel_case() {
        let dto = SearchPageDto::from(SearchPage {
            num_found: 283,
            items: vec![internet_archive::SearchItem {
                identifier: "some-id".to_string(),
                title: Some("Some title".to_string()),
                creators: vec!["Creator".to_string()],
                date: None,
                collections: vec!["opensource".to_string()],
                restricted: false,
            }],
        });
        let value = serde_json::to_value(&dto).expect("dto serializes");
        assert_eq!(value["numFound"], 283);
        assert_eq!(value["items"][0]["identifier"], "some-id");
        assert!(value.get("num_found").is_none());
    }

    #[test]
    fn item_dto_uses_camel_case_keys() {
        let dto = ItemDetailsDto::from(ItemDetails {
            identifier: "some-id".to_string(),
            title: None,
            creators: vec![],
            date: None,
            languages: vec!["eng".to_string()],
            collections: vec![],
            rights: None,
            license_url: Some("https://example.org/license".to_string()),
            mediatype: Some("texts".to_string()),
            restricted: false,
            pdfs: vec![available_pdf("doc.pdf")],
            truncated: false,
        });
        let value = serde_json::to_value(&dto).expect("dto serializes");
        assert_eq!(value["licenseUrl"], "https://example.org/license");
        assert_eq!(value["pdfs"][0]["availability"], "available");
        assert!(value.get("license_url").is_none());
    }

    #[test]
    fn blank_query_filters_become_no_filter() {
        let dto = SearchQueryDto {
            text: Some("   ".to_string()),
            collection: Some("".to_string()),
            ..SearchQueryDto::default()
        };
        let query = dto.to_search_query();
        assert_eq!(query.text, None);
        assert_eq!(query.collection, None);
    }

    #[test]
    fn error_mapping_codes_are_stable() {
        let cases: Vec<(IaError, &str)> = vec![
            (
                IaError::InvalidQuery {
                    reason: "bad".to_string(),
                },
                "invalid_query",
            ),
            (
                IaError::NotFound {
                    identifier: "x".to_string(),
                },
                "not_found",
            ),
            (
                IaError::Blocked {
                    reason: "no".to_string(),
                },
                "blocked",
            ),
            (IaError::TooManyRedirects, "too_many_redirects"),
            (IaError::TooLarge { limit: 10 }, "too_large"),
            (IaError::FileTooLarge { limit: 10 }, "too_large"),
            (
                IaError::Http {
                    status: 429,
                    retry_after: Some(Duration::from_secs(120)),
                },
                "http_429",
            ),
            (
                IaError::Http {
                    status: 500,
                    retry_after: None,
                },
                "http_500",
            ),
            (IaError::Network("down".to_string()), "network"),
            (IaError::Decode("bad json".to_string()), "decode"),
            (
                IaError::AlreadyExists {
                    path: "/staging/internet-archive/secret.pdf".to_string(),
                },
                "already_exists",
            ),
            (IaError::Io("broken".to_string()), "io"),
            (
                IaError::VerifyFailed {
                    reason: "bad header".to_string(),
                },
                "verify_failed",
            ),
            (
                IaError::TooManyPages {
                    pages: 5,
                    max_pages: 3,
                },
                "too_many_pages",
            ),
        ];
        for (error, code) in &cases {
            assert_eq!(ia_error_code(error), *code, "code for {error:?}");
            let message = ia_error(error);
            assert!(
                message.starts_with(&format!("{code}: ")),
                "message {message:?} should start with `{code}: `"
            );
        }
    }

    #[test]
    fn error_messages_never_carry_staging_paths() {
        let staging = "/data/staging/internet-archive/job/x.pdf";
        let errors = [
            IaError::AlreadyExists {
                path: staging.to_string(),
            },
            IaError::Io(format!("cannot write {staging}")),
        ];
        for error in &errors {
            let message = ia_error(error);
            assert!(
                !message.contains("staging"),
                "message {message:?} leaks a staging path"
            );
            assert!(
                !message.contains(staging),
                "message {message:?} leaks a staging path"
            );
        }
    }

    #[cfg(test)]
    struct MapFetcher {
        items: std::collections::HashMap<String, ItemDetails>,
    }

    #[cfg(test)]
    impl ItemFetcher for MapFetcher {
        async fn fetch_item(&self, identifier: &str) -> Result<ItemDetails, String> {
            self.items
                .get(identifier)
                .cloned()
                .ok_or_else(|| format!("not_found: {identifier}"))
        }
    }

    #[cfg(test)]
    fn prepare_store() -> (tempfile::TempDir, JobStore) {
        let dir = tempfile::tempdir().expect("temp store root");
        let store = JobStore::new(dir.path().join("internet-archive"));
        (dir, store)
    }

    /// Opener over the `col-1` test database: locks nothing (the caller owns
    /// the in-memory connection) and mirrors the command's snapshot shape.
    #[cfg(test)]
    fn open_col_1(
        conn: &rusqlite::Connection,
    ) -> impl FnOnce(&[String]) -> Result<ProvenanceSnapshot, String> + '_ {
        |identifiers| Ok(ProvenanceSnapshot::open(conn, "col-1", identifiers).expect("snapshot"))
    }

    #[cfg(test)]
    fn empty_snapshot() -> ProvenanceSnapshot {
        ProvenanceSnapshot {
            collection_exists: true,
            provenance: SqliteProvenance::default(),
        }
    }

    #[cfg(test)]
    fn selection(identifier: &str, file_names: &[&str]) -> SelectionDto {
        SelectionDto {
            identifier: identifier.to_string(),
            file_names: file_names.iter().map(|name| name.to_string()).collect(),
        }
    }

    #[test]
    fn selection_dto_deserializes_camel_case_file_names() {
        let selection: SelectionDto = serde_json::from_value(serde_json::json!({
            "identifier": "some-id",
            "fileNames": ["a.pdf"],
        }))
        .expect("selection deserializes");
        assert_eq!(selection.identifier, "some-id");
        assert_eq!(selection.file_names, vec!["a.pdf"]);
    }

    #[test]
    fn prepare_result_dto_uses_camel_case_keys() {
        let result = PrepareResultDto {
            job_id: "job-1".to_string(),
            summary: PrepareSummaryDto {
                total_files: 1,
                available_files: 1,
                restricted_files: 0,
                known_bytes: 10,
                unknown_size_files: 0,
                new_files: 0,
                already_in_target: 1,
                in_other_collection: 0,
                new_version: 0,
                entries: vec![PrepareEntryDto {
                    identifier: "some-id".to_string(),
                    file_name: "a.pdf".to_string(),
                    availability: "available".to_string(),
                    size: Some(10),
                    duplicate: Some(DuplicateStatusDto::from(
                        &DuplicateStatus::AlreadyInTarget {
                            item_id: "item-1".to_string(),
                        },
                    )),
                }],
            },
            rejected: vec![RejectedDto {
                identifier: "other-id".to_string(),
                file_name: "missing.pdf".to_string(),
                reason: "not_found".to_string(),
            }],
        };
        let value = serde_json::to_value(&result).expect("result serializes");
        assert_eq!(value["jobId"], "job-1");
        assert_eq!(value["summary"]["totalFiles"], 1);
        assert_eq!(value["summary"]["knownBytes"], 10);
        assert_eq!(value["summary"]["alreadyInTarget"], 1);
        assert_eq!(value["summary"]["entries"][0]["fileName"], "a.pdf");
        assert_eq!(
            value["summary"]["entries"][0]["duplicate"]["status"],
            "already_in_target"
        );
        assert_eq!(
            value["summary"]["entries"][0]["duplicate"]["itemId"],
            "item-1"
        );
        assert_eq!(value["rejected"][0]["fileName"], "missing.pdf");
        assert_eq!(value["rejected"][0]["reason"], "not_found");
        assert!(value.get("job_id").is_none());
        assert!(value["summary"].get("total_files").is_none());
    }

    #[test]
    fn duplicate_status_dto_tags_carry_their_ids() {
        let new = serde_json::to_value(DuplicateStatusDto::from(&DuplicateStatus::New)).unwrap();
        assert_eq!(new, serde_json::json!({ "status": "new" }));
        let version =
            serde_json::to_value(DuplicateStatusDto::from(&DuplicateStatus::NewVersion {
                previous_item_id: "item-0".to_string(),
            }))
            .unwrap();
        assert_eq!(
            version,
            serde_json::json!({ "status": "new_version", "previousItemId": "item-0" })
        );
        let other = serde_json::to_value(DuplicateStatusDto::from(
            &DuplicateStatus::InOtherCollection {
                item_id: "item-9".to_string(),
                collection_id: "col-x".to_string(),
            },
        ))
        .unwrap();
        assert_eq!(
            other,
            serde_json::json!({
                "status": "in_other_collection",
                "itemId": "item-9",
                "collectionId": "col-x",
            })
        );
    }

    #[tokio::test]
    async fn prepare_rejects_empty_selections() {
        let fetcher = MapFetcher {
            items: HashMap::new(),
        };
        let (_dir, store) = prepare_store();
        let error = prepare_import_with_fetcher(
            &fetcher,
            &store,
            |_| Ok(empty_snapshot()),
            1,
            "col-1",
            &[],
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("invalid_request: "), "got {error:?}");
    }

    #[tokio::test]
    async fn prepare_rejects_oversized_and_empty_file_lists() {
        let fetcher = MapFetcher {
            items: HashMap::new(),
        };
        let (_dir, store) = prepare_store();
        // 101 selections.
        let too_many: Vec<SelectionDto> = (0..101)
            .map(|i| selection(&format!("id-{i}"), &["a.pdf"]))
            .collect();
        let error = prepare_import_with_fetcher(
            &fetcher,
            &store,
            |_| Ok(empty_snapshot()),
            1,
            "col-1",
            &too_many,
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("invalid_request: "), "got {error:?}");
        // 1001 files in a single selection.
        let big_files: Vec<String> = (0..1001).map(|i| format!("f{i}.pdf")).collect();
        let big = vec![SelectionDto {
            identifier: "some-id".to_string(),
            file_names: big_files,
        }];
        let error = prepare_import_with_fetcher(
            &fetcher,
            &store,
            |_| Ok(empty_snapshot()),
            1,
            "col-1",
            &big,
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("invalid_request: "), "got {error:?}");
        // A selection naming no files at all.
        let error = prepare_import_with_fetcher(
            &fetcher,
            &store,
            |_| Ok(empty_snapshot()),
            1,
            "col-1",
            &[selection("some-id", &[])],
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("invalid_request: "), "got {error:?}");
    }

    #[tokio::test]
    async fn prepare_rejects_invalid_identifiers() {
        let fetcher = MapFetcher {
            items: HashMap::new(),
        };
        let (_dir, store) = prepare_store();
        for bad in [
            "",
            "has space",
            "a/b",
            // A URL is not a bare identifier, even when it resolves.
            "https://archive.org/details/some-id",
        ] {
            let error = prepare_import_with_fetcher(
                &fetcher,
                &store,
                |_| Ok(empty_snapshot()),
                1,
                "col-1",
                &[selection(bad, &["a.pdf"])],
            )
            .await
            .unwrap_err();
            assert!(
                error.starts_with("invalid_request: "),
                "identifier {bad:?} gave {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn prepare_rejects_duplicate_file_pairs() {
        let fetcher = MapFetcher {
            items: HashMap::new(),
        };
        let (_dir, store) = prepare_store();
        // Same pair twice in one selection, and split across two selections
        // sharing the identifier: both are duplicates.
        for selections in [
            vec![selection("some-id", &["a.pdf", "a.pdf"])],
            vec![
                selection("some-id", &["a.pdf"]),
                selection("some-id", &["a.pdf"]),
            ],
        ] {
            let error = prepare_import_with_fetcher(
                &fetcher,
                &store,
                |_| Ok(empty_snapshot()),
                1,
                "col-1",
                &selections,
            )
            .await
            .unwrap_err();
            assert!(error.starts_with("invalid_request: "), "got {error:?}");
        }
    }

    #[tokio::test]
    async fn prepare_unknown_collection_fails_before_any_fetch() {
        // Empty fetcher: if the logic fetched before checking existence,
        // the error would be `not_found`, not `collection_not_found`.
        let fetcher = MapFetcher {
            items: HashMap::new(),
        };
        let (_dir, store) = prepare_store();
        let error = prepare_import_with_fetcher(
            &fetcher,
            &store,
            |_| {
                Ok(ProvenanceSnapshot {
                    collection_exists: false,
                    provenance: SqliteProvenance::default(),
                })
            },
            1,
            "col-gone",
            &[selection("some-id", &["a.pdf"])],
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("collection_not_found: "), "got {error:?}");
    }

    #[cfg(test)]
    const OPEN_FIXTURE_ID: &str = "30-festival-notas-para-una-contra-historia-del-cine-argentino";
    #[cfg(test)]
    const LENDING_FIXTURE_ID: &str = "proceedingsofvic00cong";

    /// Both metadata fixtures parsed through the real crate parser
    /// ([`internet_archive::parse_item_value`]) — the offline stand-in for
    /// the network in prepare tests. No server, no fetch: the raw API bytes
    /// go straight into the parser production uses.
    #[cfg(test)]
    fn fixture_details() -> std::collections::HashMap<String, ItemDetails> {
        let mut items = HashMap::new();
        for (identifier, body) in [
            (
                OPEN_FIXTURE_ID,
                include_str!(
                    "../../../../../crates/internet-archive/tests/fixtures/metadata-open-text-pdf.json"
                ),
            ),
            (
                LENDING_FIXTURE_ID,
                include_str!(
                    "../../../../../crates/internet-archive/tests/fixtures/metadata-lending-encrypted.json"
                ),
            ),
        ] {
            let value: serde_json::Value =
                serde_json::from_str(body).expect("fixture is valid JSON");
            let details = internet_archive::parse_item_value(identifier, &value)
                .expect("fixture parses through the crate parser");
            items.insert(identifier.to_string(), details);
        }
        items
    }

    #[cfg(test)]
    use crate::internet_archive::provenance_db::{
        insert_test_collection, insert_test_item, provenance_metadata, test_db, SqliteProvenance,
    };

    #[tokio::test]
    async fn prepare_open_pdf_becomes_a_job_file() {
        let fixtures = fixture_details();
        let pdf_name = fixtures[OPEN_FIXTURE_ID].pdfs[0].name.clone();
        let conn = test_db();
        insert_test_collection(&conn, "col-1");
        let (_dir, store) = prepare_store();
        let fetcher = MapFetcher { items: fixtures };
        let result = prepare_import_with_fetcher(
            &fetcher,
            &store,
            |identifiers| {
                assert_eq!(identifiers, &[OPEN_FIXTURE_ID.to_string()]);
                Ok(ProvenanceSnapshot::open(&conn, "col-1", identifiers).expect("snapshot"))
            },
            1_700_000_000,
            "col-1",
            &[selection(OPEN_FIXTURE_ID, &[pdf_name.as_str()])],
        )
        .await
        .expect("prepare succeeds");
        assert!(result.rejected.is_empty());
        assert_eq!(result.summary.new_files, 1);
        assert_eq!(result.summary.available_files, 1);
        assert_eq!(result.summary.entries.len(), 1);
        assert_eq!(
            result.summary.entries[0].duplicate,
            Some(DuplicateStatusDto::New)
        );
        let job = store.load(&result.job_id).expect("job persisted");
        assert_eq!(job.files.len(), 1);
        assert_eq!(job.files[0].identifier, OPEN_FIXTURE_ID);
        assert_eq!(job.files[0].file_name, pdf_name);
        assert_eq!(job.state, internet_archive::job::JobState::Draft);
        assert!(
            matches!(&job.target, ImportTarget::ExistingCollection { collection_id } if collection_id == "col-1")
        );
        // One snapshot for the identifier that contributed the file.
        assert_eq!(job.items.len(), 1);
        assert_eq!(job.items[0].identifier, OPEN_FIXTURE_ID);
    }

    #[tokio::test]
    async fn prepare_restricted_files_are_rejected_with_their_tags() {
        let fixtures = fixture_details();
        let open_pdf = fixtures[OPEN_FIXTURE_ID].pdfs[0].name.clone();
        let lending_names: Vec<String> = fixtures[LENDING_FIXTURE_ID]
            .pdfs
            .iter()
            .map(|pdf| pdf.name.clone())
            .collect();
        assert_eq!(lending_names.len(), 3);
        let conn = test_db();
        insert_test_collection(&conn, "col-1");
        let (_dir, store) = prepare_store();
        let fetcher = MapFetcher { items: fixtures };
        let lending_refs: Vec<&str> = lending_names.iter().map(String::as_str).collect();
        let result = prepare_import_with_fetcher(
            &fetcher,
            &store,
            open_col_1(&conn),
            1_700_000_000,
            "col-1",
            &[
                selection(OPEN_FIXTURE_ID, &[open_pdf.as_str()]),
                selection(LENDING_FIXTURE_ID, &lending_refs),
            ],
        )
        .await
        .expect("prepare succeeds");
        // The open PDF is new; every lending file is rejected with its tag.
        assert_eq!(result.summary.new_files, 1);
        let reasons: Vec<(&str, &str)> = result
            .rejected
            .iter()
            .map(|entry| (entry.identifier.as_str(), entry.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (LENDING_FIXTURE_ID, "unavailable:encrypted"),
                (LENDING_FIXTURE_ID, "unavailable:private"),
                (LENDING_FIXTURE_ID, "unavailable:encrypted"),
            ]
        );
    }

    #[tokio::test]
    async fn prepare_nothing_importable_creates_no_job() {
        let fixtures = fixture_details();
        let lending_names: Vec<String> = fixtures[LENDING_FIXTURE_ID]
            .pdfs
            .iter()
            .map(|pdf| pdf.name.clone())
            .collect();
        let conn = test_db();
        insert_test_collection(&conn, "col-1");
        let (dir, store) = prepare_store();
        let fetcher = MapFetcher { items: fixtures };
        let lending_refs: Vec<&str> = lending_names.iter().map(String::as_str).collect();
        // All restricted.
        let error = prepare_import_with_fetcher(
            &fetcher,
            &store,
            open_col_1(&conn),
            1_700_000_000,
            "col-1",
            &[selection(LENDING_FIXTURE_ID, &lending_refs)],
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("nothing_to_import: "), "got {error:?}");
        // All unknown file names.
        let error = prepare_import_with_fetcher(
            &fetcher,
            &store,
            open_col_1(&conn),
            1_700_000_000,
            "col-1",
            &[selection(OPEN_FIXTURE_ID, &["no-such-file.pdf"])],
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("nothing_to_import: "), "got {error:?}");
        // No job directory was created for either failure.
        assert!(!dir.path().join("internet-archive").exists());
    }

    #[tokio::test]
    async fn prepare_unknown_file_name_is_rejected_as_not_found() {
        let fixtures = fixture_details();
        let open_pdf = fixtures[OPEN_FIXTURE_ID].pdfs[0].name.clone();
        let conn = test_db();
        insert_test_collection(&conn, "col-1");
        let (_dir, store) = prepare_store();
        let fetcher = MapFetcher { items: fixtures };
        let result = prepare_import_with_fetcher(
            &fetcher,
            &store,
            open_col_1(&conn),
            1_700_000_000,
            "col-1",
            &[selection(
                OPEN_FIXTURE_ID,
                &[open_pdf.as_str(), "no-such-file.pdf"],
            )],
        )
        .await
        .expect("prepare succeeds");
        assert_eq!(result.summary.new_files, 1);
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].identifier, OPEN_FIXTURE_ID);
        assert_eq!(result.rejected[0].file_name, "no-such-file.pdf");
        assert_eq!(result.rejected[0].reason, "not_found");
    }

    #[tokio::test]
    async fn prepare_existing_provenance_in_target_is_already_in_target() {
        let fixtures = fixture_details();
        let pdf = fixtures[OPEN_FIXTURE_ID].pdfs[0].clone();
        let md5 = pdf.md5.clone().expect("fixture pdf has an md5");
        let conn = test_db();
        insert_test_collection(&conn, "col-1");
        insert_test_item(
            &conn,
            "item-1",
            "col-1",
            Some(&provenance_metadata(OPEN_FIXTURE_ID, &pdf.name, Some(&md5))),
        );
        let (_dir, store) = prepare_store();
        let fetcher = MapFetcher { items: fixtures };
        let result = prepare_import_with_fetcher(
            &fetcher,
            &store,
            open_col_1(&conn),
            1_700_000_000,
            "col-1",
            &[selection(OPEN_FIXTURE_ID, &[pdf.name.as_str()])],
        )
        .await
        .expect("prepare succeeds");
        // Still in the job (availability gates inclusion); the summary flags
        // the duplicate for the later download/import tasks.
        assert_eq!(result.summary.already_in_target, 1);
        assert_eq!(
            result.summary.entries[0].duplicate,
            Some(DuplicateStatusDto::AlreadyInTarget {
                item_id: "item-1".to_string(),
            })
        );
        let job = store.load(&result.job_id).expect("job persisted");
        assert_eq!(job.files.len(), 1);
    }

    #[tokio::test]
    async fn prepare_other_collection_and_new_version_statuses() {
        let fixtures = fixture_details();
        let pdf = fixtures[OPEN_FIXTURE_ID].pdfs[0].clone();
        let md5 = pdf.md5.clone().expect("fixture pdf has an md5");
        // Same version elsewhere: independent copy, never a link.
        let conn = test_db();
        insert_test_collection(&conn, "col-1");
        insert_test_collection(&conn, "col-other");
        insert_test_item(
            &conn,
            "item-9",
            "col-other",
            Some(&provenance_metadata(OPEN_FIXTURE_ID, &pdf.name, Some(&md5))),
        );
        let (_dir, store) = prepare_store();
        let fetcher = MapFetcher {
            items: fixtures.clone(),
        };
        let result = prepare_import_with_fetcher(
            &fetcher,
            &store,
            open_col_1(&conn),
            1_700_000_000,
            "col-1",
            &[selection(OPEN_FIXTURE_ID, &[pdf.name.as_str()])],
        )
        .await
        .expect("prepare succeeds");
        assert_eq!(result.summary.in_other_collection, 1);
        assert_eq!(
            result.summary.entries[0].duplicate,
            Some(DuplicateStatusDto::InOtherCollection {
                item_id: "item-9".to_string(),
                collection_id: "col-other".to_string(),
            })
        );
        // Different version in the target: a separate document, never an
        // overwrite.
        let conn = test_db();
        insert_test_collection(&conn, "col-1");
        insert_test_item(
            &conn,
            "item-1",
            "col-1",
            Some(&provenance_metadata(
                OPEN_FIXTURE_ID,
                &pdf.name,
                Some("00000000000000000000000000000000"),
            )),
        );
        let (_dir, store) = prepare_store();
        let fetcher = MapFetcher { items: fixtures };
        let result = prepare_import_with_fetcher(
            &fetcher,
            &store,
            open_col_1(&conn),
            1_700_000_000,
            "col-1",
            &[selection(OPEN_FIXTURE_ID, &[pdf.name.as_str()])],
        )
        .await
        .expect("prepare succeeds");
        assert_eq!(result.summary.new_version, 1);
        assert_eq!(
            result.summary.entries[0].duplicate,
            Some(DuplicateStatusDto::NewVersion {
                previous_item_id: "item-1".to_string(),
            })
        );
    }

    #[test]
    fn resolve_logic_returns_identifiers_never_urls() {
        assert_eq!(
            resolve_identifier_logic("https://archive.org/details/my-item_01").unwrap(),
            "my-item_01"
        );
        assert_eq!(
            resolve_identifier_logic("my-item_01").unwrap(),
            "my-item_01"
        );
        let error = resolve_identifier_logic("https://example.com/details/x").unwrap_err();
        assert!(
            error.starts_with("invalid_query: "),
            "unexpected error {error:?}"
        );
    }

    #[test]
    fn job_status_dto_uses_camel_case_keys() {
        use internet_archive::job::{FileState, JobState};
        let dto = JobStatusDto {
            id: "job-1".to_string(),
            state: JobState::Running,
            target_collection_id: Some("col-1".to_string()),
            files: vec![JobFileStatusDto {
                index: 0,
                identifier: "some-id".to_string(),
                file_name: "a.pdf".to_string(),
                display_file_name: "a.pdf".to_string(),
                state: FileState::Verified,
                attempts: 2,
                error: None,
                expected_size: Some(100),
                page_count: Some(3),
            }],
            running: true,
        };
        let value = serde_json::to_value(&dto).expect("dto serializes");
        assert_eq!(value["id"], "job-1");
        assert_eq!(value["state"], "running");
        assert_eq!(value["targetCollectionId"], "col-1");
        assert_eq!(value["running"], true);
        assert_eq!(value["files"][0]["fileName"], "a.pdf");
        assert_eq!(value["files"][0]["displayFileName"], "a.pdf");
        assert_eq!(value["files"][0]["expectedSize"], 100);
        assert_eq!(value["files"][0]["pageCount"], 3);
        assert!(value.get("target_collection_id").is_none());
        assert!(value["files"][0].get("file_name").is_none());
    }

    #[test]
    fn verified_files_logic_builds_provenance_and_staging_path() {
        use internet_archive::job::{FileState, ImportTarget, Job, JobFile, JobState};
        use internet_archive::provenance::{ItemSnapshot, PROVENANCE_KEY};
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobStore::new(dir.path().join("internet-archive"));
        let pdf = available_pdf("doc.pdf");
        let mut file = JobFile::from_pdf("some-id", &pdf).expect("available");
        file.state = FileState::Verified;
        file.staging_name = Some(internet_archive::store::new_staging_name());
        file.sha256 = Some("abc123".to_string());
        file.page_count = Some(7);
        let staging = file.staging_name.clone().unwrap();
        let mut job = Job::new(
            uuid::Uuid::new_v4().hyphenated().to_string(),
            1_700_000_000,
            ImportTarget::ExistingCollection {
                collection_id: "col-1".to_string(),
            },
            vec![file],
        )
        .expect("job");
        job.set_job_state(JobState::Ready, 1_700_000_001).unwrap();
        job.set_job_state(JobState::Running, 1_700_000_002).unwrap();
        job.items = vec![ItemSnapshot {
            identifier: "some-id".to_string(),
            title: Some("Some title".to_string()),
            creators: vec![],
            date: None,
            languages: vec![],
            collections: vec!["opensource".to_string()],
            rights: None,
            license_url: None,
        }];
        store.create(&job).unwrap();
        // The staging file must exist for the path to be meaningful.
        let dest = store.staging_path(&job.id, &staging).unwrap();
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"%PDF-1.4 fake").unwrap();
        let out = verified_files_logic(&store, &job, 1_700_000_010).expect("verified list");
        assert_eq!(out.len(), 1);
        let entry = &out[0];
        assert_eq!(entry.index, 0);
        assert_eq!(entry.identifier, "some-id");
        assert_eq!(entry.page_count, 7);
        assert_eq!(entry.sha256, "abc123");
        assert_eq!(entry.item_title, "Some title");
        // Staging path is absolute and inside the store root.
        assert!(entry.staging_path.contains("internet-archive"));
        assert!(
            std::path::Path::new(&entry.staging_path).starts_with(store.root()),
            "stagingPath {} not inside {:?}",
            entry.staging_path,
            store.root()
        );
        let value = serde_json::to_value(entry).expect("dto serializes");
        assert_eq!(value["fileName"], "doc.pdf");
        assert_eq!(value["displayFileName"], "doc.pdf");
        assert_eq!(value["itemTitle"], "Some title");
        assert_eq!(
            value["stagingPath"],
            serde_json::Value::String(entry.staging_path.clone())
        );
        assert!(value.get("extraMetadata").is_some());
        let provenance = &value["extraMetadata"][PROVENANCE_KEY];
        assert_eq!(provenance["identifier"], "some-id");
        assert_eq!(provenance["file_name"], "doc.pdf");
        assert_eq!(provenance["sha256_local"], "abc123");
        assert_eq!(provenance["page_count"], 7);
    }

    #[test]
    fn list_jobs_logic_lists_only_non_terminal() {
        use internet_archive::job::{FileState, ImportTarget, Job, JobFile, JobState};
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobStore::new(dir.path().join("internet-archive"));
        let mk = |state: JobState| {
            let file = JobFile::from_pdf("some-id", &available_pdf("a.pdf")).expect("pdf");
            let mut job = Job::new(
                uuid::Uuid::new_v4().hyphenated().to_string(),
                1_700_000_000,
                ImportTarget::ExistingCollection {
                    collection_id: "col-1".to_string(),
                },
                vec![file],
            )
            .expect("job");
            // Drive the state machine legally: Draft -> Ready -> Running -> terminal.
            if state != JobState::Draft {
                job.set_job_state(JobState::Ready, 1).unwrap();
                if state != JobState::Ready {
                    job.set_job_state(JobState::Running, 2).unwrap();
                    if state != JobState::Running {
                        // Settle needs terminal files; force one Failed then settle.
                        job.files[0].state = FileState::Failed;
                        let settled = job.settle(3).expect("settles to Failed");
                        assert_eq!(settled, JobState::Failed);
                    }
                }
            }
            assert_eq!(job.state, state);
            job
        };
        let running = mk(JobState::Running);
        let failed = mk(JobState::Failed);
        store.create(&running).unwrap();
        store.create(&failed).unwrap();
        let out = list_jobs_logic(&store).expect("list");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, running.id);
        let value = serde_json::to_value(&out[0]).expect("dto serializes");
        assert!(value.get("targetCollectionId").is_some());
        assert!(value.get("totalFiles").is_some());
    }

    fn discard_test_store(state: JobState) -> (tempfile::TempDir, JobStore, Job) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobStore::new(dir.path().join("internet-archive"));
        let file = JobFile::from_pdf("some-id", &available_pdf("doc.pdf")).expect("pdf");
        let mut job = Job::new(
            uuid::Uuid::new_v4().hyphenated().to_string(),
            1_700_000_000,
            ImportTarget::ExistingCollection {
                collection_id: "col-1".to_string(),
            },
            vec![file],
        )
        .expect("job");
        // Drive the state machine legally, like the list_jobs test does.
        if state != JobState::Draft {
            job.set_job_state(JobState::Ready, 1).unwrap();
            if state != JobState::Ready {
                job.set_job_state(JobState::Running, 2).unwrap();
                if state != JobState::Running {
                    job.files[0].state = FileState::Failed;
                    let settled = job.settle(3).expect("settles");
                    assert_eq!(settled, state);
                }
            }
        }
        assert_eq!(job.state, state);
        store.create(&job).unwrap();
        (dir, store, job)
    }

    #[test]
    fn discard_draft_removes_the_job_folder() {
        let (_dir, store, job) = discard_test_store(JobState::Draft);
        discard_draft_logic(&store, &job.id).expect("draft discards");
        let err = store.load(&job.id).expect_err("job folder is gone");
        assert!(matches!(err, StoreError::NotFound(_)), "error: {err:?}");
    }

    #[test]
    fn discard_draft_refuses_a_running_job_untouched() {
        let (_dir, store, job) = discard_test_store(JobState::Running);
        let err = discard_draft_logic(&store, &job.id).expect_err("running job refused");
        assert!(err.starts_with("invalid_state:"), "error: {err}");
        let kept = store.load(&job.id).expect("job untouched");
        assert_eq!(kept.state, JobState::Running);
        assert_eq!(kept.files.len(), 1);
    }

    #[test]
    fn discard_draft_refuses_a_terminal_job_untouched() {
        let (_dir, store, job) = discard_test_store(JobState::Failed);
        let err = discard_draft_logic(&store, &job.id).expect_err("terminal job refused");
        assert!(err.starts_with("invalid_state:"), "error: {err}");
        let kept = store.load(&job.id).expect("job untouched");
        assert_eq!(kept.state, JobState::Failed);
        assert_eq!(kept.files.len(), 1);
    }

    #[test]
    fn discard_draft_reports_an_unknown_job_as_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobStore::new(dir.path().join("internet-archive"));
        let err = discard_draft_logic(&store, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
            .expect_err("unknown job");
        assert!(err.starts_with("not_found:"), "error: {err}");
    }

    #[test]
    fn discard_draft_rejects_an_invalid_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobStore::new(dir.path().join("internet-archive"));
        let err = discard_draft_logic(&store, "nope").expect_err("invalid id");
        assert!(err.starts_with("invalid_request:"), "error: {err}");
    }
}
