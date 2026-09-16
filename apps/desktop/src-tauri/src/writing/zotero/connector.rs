//! Talking to Zotero's local API (plan-editor.md §11.2).
//!
//! Every request this module can build carries a limit and a start, because
//! spike S5 measured what happens without one: a single query came back with
//! 7.9 MB. A limit is therefore not a caller's option here — there is no
//! function that builds a request without it.

use std::time::Duration;

use super::{diagnose, ProbeOutcome, ZoteroState};

/// Where Zotero listens. §11.1: the official local API, never the SQLite file.
pub const BASE_URL: &str = "http://localhost:23119";

/// §11.3 requires a timeout to be one of the reported states, which means one
/// has to exist. Long enough for a large library, short enough that a hung
/// port does not look like a hung application.
pub const TIMEOUT: Duration = Duration::from_secs(8);

/// The most items one response may carry.
///
/// S5's 7.9 MB was a whole library in one payload. This is the cap that makes
/// paging mandatory rather than advisory.
pub const MAX_LIMIT: u32 = 100;

/// One page of a library query. Constructed only through [`Page::new`], which
/// is what makes the limit impossible to omit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    start: u32,
    limit: u32,
}

impl Page {
    /// A page, with its limit clamped to something a response can carry.
    ///
    /// Clamped rather than rejected: a caller asking for too much has made a
    /// judgement about how much it wants, not an error, and refusing the whole
    /// query would be a worse answer than giving it the first hundred.
    pub fn new(start: u32, limit: u32) -> Self {
        Self {
            start,
            limit: limit.clamp(1, MAX_LIMIT),
        }
    }

    pub fn first() -> Self {
        Self::new(0, MAX_LIMIT)
    }

    /// The page after this one, for walking a library without holding it.
    pub fn next(self) -> Self {
        Self::new(self.start.saturating_add(self.limit), self.limit)
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn limit(self) -> u32 {
        self.limit
    }
}

/// The URL for a page of items.
///
/// `format=csljson` because S5 confirmed it feeds hayagriva with no conversion
/// layer at all — asking for anything else would mean writing and maintaining a
/// translation that the two ends already agree on.
pub fn items_url(library: &str, page: Page) -> String {
    search_url(library, page, None)
}

/// The URL for a page, optionally narrowed by a search.
///
/// The search is Zotero's own (`q`), not ours. A library runs to thousands of
/// works and reading all of them to filter in memory both wastes the trip and
/// gets the answer wrong the moment anything is left unread — which is exactly
/// how a search for an author who *is* in the library came back empty.
pub fn search_url(library: &str, page: Page, query: Option<&str>) -> String {
    let mut url = format!(
        "{BASE_URL}/api/users/{library}/items?format=csljson&limit={}&start={}",
        page.limit(),
        page.start()
    );
    if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
        // `qmode=everything` searches full text and notes as well as metadata,
        // which is what someone typing an author's surname expects.
        url.push_str("&qmode=everything&q=");
        url.push_str(&urlencoding::encode(q));
    }
    url
}

/// The liveness probe. Answers even when the local API is disabled (S5), which
/// is what lets a disabled API be told apart from an absent program.
pub fn ping_url() -> String {
    format!("{BASE_URL}/connector/ping")
}

/// Classifies one HTTP outcome into a probe result.
///
/// Separated from the request so every branch is testable without a server.
pub fn classify(result: Result<u16, ProbeError>) -> ProbeOutcome {
    match result {
        Ok(status) if (200..300).contains(&status) => ProbeOutcome::Answered,
        Ok(403) => ProbeOutcome::Forbidden,
        Ok(404) => ProbeOutcome::Unreachable,
        Ok(_) => ProbeOutcome::Malformed,
        Err(ProbeError::Timeout) => ProbeOutcome::TimedOut,
        Err(ProbeError::Unreachable) => ProbeOutcome::Unreachable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    Timeout,
    Unreachable,
}

/// Asks both probes and reports the one thing worth asserting (§11.3).
pub async fn probe(client: &reqwest::Client) -> ZoteroState {
    let connector = classify(send(client, &ping_url()).await);
    let library = classify(send(client, &items_url("0", Page::new(0, 1))).await);
    diagnose(connector, library)
}

/// One page of the library, as CSL-JSON items.
///
/// Returns the items and the `Last-Modified-Version` the library reported, so
/// the caller can tell the cache which instance these came from. That header is
/// all the instance identity there is — S5 measured that `Zotero-Server-ID`
/// does not exist — which is why it is carried rather than discarded.
pub async fn fetch_items(
    client: &reqwest::Client,
    library: &str,
    page: Page,
    query: Option<&str>,
) -> Result<LibraryPage, ZoteroState> {
    let url = search_url(library, page, query);
    let response = match client.get(&url).timeout(TIMEOUT).send().await {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return Err(ZoteroState::Timeout),
        Err(_) => return Err(ZoteroState::EndpointUnavailable),
    };

    let status = response.status().as_u16();
    if status == 403 {
        return Err(ZoteroState::ApiDisabled);
    }
    if !(200..300).contains(&status) {
        return Err(ZoteroState::InvalidResponse {
            detail: format!("the library answered {status}"),
        });
    }

    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    };
    let version = header("last-modified-version");
    // How many the library has for this query. Zotero sends it, so there is no
    // reason to guess — and guessing is what truncated a library at 662.
    let total = header("total-results");

    let body = response.text().await.map_err(|error| ZoteroState::InvalidResponse {
        detail: format!("the library's answer could not be read: {error}"),
    })?;

    // Kept as text rather than parsed into our own shape: CSL-JSON is what the
    // renderer reads, so translating it here and back would be a conversion
    // layer S5 confirmed is unnecessary.
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&body).map_err(|error| ZoteroState::InvalidResponse {
            detail: format!("the library's answer was not CSL-JSON: {error}"),
        })?;

    let items: Vec<String> = items.into_iter().map(|item| item.to_string()).collect();
    // From `Total-Results`, never from how many came back.
    //
    // The API paginates over every item; `format=csljson` emits only the
    // citable ones, so a page of a hundred that holds attachments and notes
    // returns fewer than a hundred citations. Reading that as "there are no
    // more" stops early and silently, which is precisely how a library of
    // thousands was read as 662 and an author who was in it could not be found.
    //
    // Without the header there is nothing to go on, and stopping would be the
    // same silent truncation — so a full page is assumed to have more behind it
    // and the caller's own ceiling is what ends the walk.
    let has_more = match total {
        Some(total) => (u64::from(page.start()) + items.len() as u64) < total,
        None => items.len() as u32 >= page.limit(),
    };

    Ok(LibraryPage {
        items,
        version,
        total,
        has_more,
    })
}

/// A page of items, and which instance they came from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LibraryPage {
    /// Each item as CSL-JSON text, ready for the renderer with no conversion.
    pub items: Vec<String>,
    /// `Last-Modified-Version`, the only instance identity available (S5).
    pub version: Option<u64>,
    /// What the library says it holds for this query, when it says so.
    pub total: Option<u64>,
    /// Whether a further page is worth asking for.
    pub has_more: bool,
}

async fn send(client: &reqwest::Client, url: &str) -> Result<u16, ProbeError> {
    match client.get(url).timeout(TIMEOUT).send().await {
        Ok(response) => Ok(response.status().as_u16()),
        Err(error) if error.is_timeout() => Err(ProbeError::Timeout),
        Err(_) => Err(ProbeError::Unreachable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S5 measured 7.9 MB from one unlimited query. There is no way to build a
    /// request without a limit, and this is the test that keeps it that way.
    #[test]
    fn every_request_carries_a_limit_and_a_start() {
        let url = items_url("0", Page::first());

        assert!(url.contains("limit="), "{url}");
        assert!(url.contains("start="), "{url}");
    }

    #[test]
    fn a_limit_larger_than_a_response_can_carry_is_clamped_not_refused() {
        let page = Page::new(0, 10_000);

        assert_eq!(page.limit(), MAX_LIMIT);
    }

    #[test]
    fn a_limit_of_nothing_is_still_a_request_for_something() {
        assert_eq!(Page::new(0, 0).limit(), 1);
    }

    #[test]
    fn paging_walks_forward_without_overlapping() {
        let first = Page::new(0, 50);
        let second = first.next();

        assert_eq!(second.start(), 50);
        assert_eq!(second.limit(), 50);
        assert_eq!(second.next().start(), 100);
    }

    /// A library big enough to overflow the counter must not wrap around and
    /// start again at the beginning, which would page forever.
    #[test]
    fn paging_saturates_rather_than_wrapping() {
        let page = Page::new(u32::MAX - 1, 50);

        assert_eq!(page.next().start(), u32::MAX);
    }

    /// S5: `format=csljson` feeds hayagriva with no conversion layer. Asking
    /// for anything else means writing a translation both ends already agree on.
    #[test]
    fn items_are_asked_for_in_the_format_the_csl_engine_reads() {
        assert!(items_url("0", Page::first()).contains("format=csljson"));
    }

    #[test]
    fn the_local_api_is_used_rather_than_the_database() {
        assert!(items_url("0", Page::first()).starts_with("http://localhost:23119"));
        assert!(ping_url().starts_with("http://localhost:23119"));
    }

    #[test]
    fn a_refusal_is_a_permission_and_a_silence_is_not() {
        assert_eq!(classify(Ok(403)), ProbeOutcome::Forbidden);
        assert_eq!(classify(Ok(200)), ProbeOutcome::Answered);
        assert_eq!(classify(Err(ProbeError::Timeout)), ProbeOutcome::TimedOut);
        assert_eq!(
            classify(Err(ProbeError::Unreachable)),
            ProbeOutcome::Unreachable
        );
    }

    #[test]
    fn an_unexpected_status_is_not_silently_treated_as_success() {
        assert_eq!(classify(Ok(500)), ProbeOutcome::Malformed);
        assert_eq!(classify(Ok(418)), ProbeOutcome::Malformed);
    }
}
