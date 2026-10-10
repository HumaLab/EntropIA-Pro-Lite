//! Read-only Internet Archive commands: identifier resolution, full-text
//! search, collection browsing, and item metadata.
//!
//! The `#[tauri::command]` functions below are thin wrappers over the plain
//! `pub(crate)` logic functions next to them, which take `&IaClient` /
//! `&Endpoints` directly so the logic stays testable without a Tauri handle
//! or the network. Only identifiers travel as parameters — never a URL or a
//! path — and [`ia_resolve_identifier`] returns an identifier, never a URL.

use internet_archive::{
    Availability, Endpoints, FileSource, IaClient, IaError, ItemDetails, RestrictedReason,
    SearchPage, SearchQuery,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::IaState;

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
}
