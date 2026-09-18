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

/// The most keys one `itemKey` request may name (the Zotero API's own limit).
pub const MAX_KEYS: usize = 50;

/// The URL for a page of the library's works.
///
/// `/items/top` rather than `/items`: child attachments and notes are not
/// works anyone cites, but `format=csljson` over `/items` emits them anyway —
/// on a real library of ~5,000 items, ~2,000 rows named "Full Text PDF" and 22
/// pages that bought nothing.
///
/// `format=csljson` because S5 confirmed it feeds hayagriva with no conversion
/// layer at all — asking for anything else would mean writing and maintaining a
/// translation that the two ends already agree on.
pub fn items_url(library: &str, page: Page) -> String {
    format!(
        "{BASE_URL}/api/users/{library}/items/top?format=csljson&limit={}&start={}",
        page.limit(),
        page.start()
    )
}

/// The URL for a page of Zotero's own search (`q`).
///
/// Over `/items` and as `json`, not `/items/top` as CSL: a match inside a PDF
/// is reported on the attachment, `/items/top` then drops the work entirely,
/// and only `json` says which work an attachment belongs to. The works
/// themselves are fetched afterwards by [`works_url`].
///
/// `qmode=everything` searches full text and notes as well as metadata. The
/// list already holds the whole library's metadata, so full text is what this
/// search adds.
pub fn search_url(library: &str, page: Page, query: &str) -> String {
    format!(
        "{BASE_URL}/api/users/{library}/items?format=json&limit={}&start={}&qmode=everything&q={}",
        page.limit(),
        page.start(),
        urlencoding::encode(query.trim())
    )
}

/// The URL for specific works, in the same format the library was read in.
///
/// The same format matters: a CSL `id` is the citation key when the work has
/// one and a URI when it does not, and a citation is stored under that `id`.
/// `/items/top` because `itemKey` over `/items` also returns the children.
pub fn works_url(library: &str, keys: &[String]) -> String {
    let keys = &keys[..keys.len().min(MAX_KEYS)];
    format!(
        "{BASE_URL}/api/users/{library}/items/top?format=csljson&limit={MAX_KEYS}&start=0&itemKey={}",
        urlencoding::encode(&keys.join(","))
    )
}

/// The works a page of search hits points at, best match first.
///
/// A hit inside an attachment or a note is its parent work; a hit on a work
/// is itself. Repeats are dropped, and the list stops at what one
/// [`works_url`] request can name.
pub fn works_from_hits(hits: &[serde_json::Value]) -> Vec<String> {
    let mut works: Vec<String> = Vec::new();
    for hit in hits {
        let parent = hit.pointer("/data/parentItem").and_then(|v| v.as_str());
        let own = hit.get("key").and_then(|v| v.as_str());
        let Some(key) = parent.or(own).filter(|key| !key.is_empty()) else {
            continue;
        };
        if !works.iter().any(|seen| seen == key) {
            works.push(key.to_string());
            if works.len() == MAX_KEYS {
                break;
            }
        }
    }
    works
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
) -> Result<LibraryPage, ZoteroState> {
    let answer = ask(client, &items_url(library, page)).await?;

    // Kept as text rather than parsed into our own shape: CSL-JSON is what the
    // renderer reads, so translating it here and back would be a conversion
    // layer S5 confirmed is unnecessary.
    let items: Vec<String> = answer.items.iter().map(ToString::to_string).collect();
    // From `Total-Results`, never from how many came back: a page can carry
    // fewer citable works than it counts, and reading that as "there are no
    // more" is how a library of thousands was once read as 662.
    //
    // Without the header there is nothing to go on, and stopping would be the
    // same silent truncation — so a full page is assumed to have more behind it
    // and the caller's own walk is what ends it.
    let has_more = match answer.total {
        Some(total) => u64::from(page.start()) + u64::from(page.limit()) < total,
        None => items.len() as u32 >= page.limit(),
    };

    Ok(LibraryPage {
        items,
        version: answer.version,
        total: answer.total,
        has_more,
    })
}

/// The works Zotero's own search finds, as CSL-JSON, best match first.
///
/// Two requests: the search, which reports hits on attachments and notes as
/// well as on works, and then the works those hits belong to. Only the first
/// page of hits is used — this supplements a list that already filters the
/// whole library's metadata, it does not replace it.
pub async fn search_works(
    client: &reqwest::Client,
    library: &str,
    query: &str,
) -> Result<LibraryPage, ZoteroState> {
    let hits = ask(client, &search_url(library, Page::first(), query)).await?;
    let keys = works_from_hits(&hits.items);
    if keys.is_empty() {
        return Ok(LibraryPage {
            items: Vec::new(),
            version: hits.version,
            total: Some(0),
            has_more: false,
        });
    }

    let works = ask(client, &works_url(library, &keys)).await?;
    Ok(LibraryPage {
        items: works.items.iter().map(ToString::to_string).collect(),
        version: works.version,
        total: Some(keys.len() as u64),
        has_more: false,
    })
}

/// One answer from the library: its headers, and its body parsed as JSON.
struct Answer {
    version: Option<u64>,
    total: Option<u64>,
    items: Vec<serde_json::Value>,
}

async fn ask(client: &reqwest::Client, url: &str) -> Result<Answer, ZoteroState> {
    let response = match client.get(url).timeout(TIMEOUT).send().await {
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

    let body = response
        .text()
        .await
        .map_err(|error| ZoteroState::InvalidResponse {
            detail: format!("the library's answer could not be read: {error}"),
        })?;
    let items = serde_json::from_str(&body).map_err(|error| ZoteroState::InvalidResponse {
        detail: format!("the library's answer was not a list of items: {error}"),
    })?;

    Ok(Answer {
        version,
        total,
        items,
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

    /// Child attachments ("Full Text PDF") and notes are not works anyone
    /// cites, yet `format=csljson` over `/items` emits them. On a library of
    /// ~5,000 items that was ~2,000 rows of noise and 22 extra pages.
    #[test]
    fn the_library_is_read_without_its_child_attachments_and_notes() {
        assert!(items_url("0", Page::first()).contains("/api/users/0/items/top?"));
    }

    /// `/items/top` with `q` drops a work whose match is in its PDF, so the
    /// search runs over every item and asks for `json`, the only format that
    /// says which work an attachment belongs to.
    #[test]
    fn a_search_runs_over_every_item_and_reports_parents() {
        let url = search_url("0", Page::first(), "Acha y Pérez");

        assert!(url.contains("/api/users/0/items?"), "{url}");
        assert!(url.contains("format=json"), "{url}");
        assert!(url.contains("qmode=everything"), "{url}");
        assert!(url.contains("q=Acha%20y%20P%C3%A9rez"), "{url}");
        assert!(url.contains("limit=") && url.contains("start="), "{url}");
    }

    /// The works are fetched in the same format as the library, so a result
    /// carries the same CSL `id` a citation was stored under.
    #[test]
    fn works_are_fetched_by_key_in_the_librarys_own_format() {
        let url = works_url("0", &["AAAA1111".into(), "BBBB2222".into()]);

        assert!(url.contains("/api/users/0/items/top?"), "{url}");
        assert!(url.contains("format=csljson"), "{url}");
        assert!(url.contains("itemKey=AAAA1111%2CBBBB2222"), "{url}");
        assert!(url.contains("limit=") && url.contains("start="), "{url}");
    }

    #[test]
    fn a_hit_inside_an_attachment_is_reported_as_its_work() {
        let hits = serde_json::json!([
            { "key": "PDF00001", "data": { "parentItem": "BOOK0001" } },
            { "key": "BOOK0002", "data": { "itemType": "book" } },
            { "key": "PDF00002", "data": { "parentItem": "BOOK0001" } },
            { "key": "BOOK0001", "data": { "itemType": "book" } },
        ]);

        assert_eq!(
            works_from_hits(hits.as_array().unwrap()),
            vec!["BOOK0001".to_string(), "BOOK0002".to_string()]
        );
    }

    /// `itemKey` takes at most fifty keys; the first fifty works are the best
    /// matches Zotero ranked, and the list shows its own matches above them.
    #[test]
    fn no_more_works_are_asked_for_than_one_request_can_name() {
        let hits: Vec<serde_json::Value> = (0..80)
            .map(|i| serde_json::json!({ "key": format!("K{i:07}"), "data": {} }))
            .collect();

        assert_eq!(works_from_hits(&hits).len(), MAX_KEYS);
    }

    #[test]
    fn a_hit_without_a_key_is_skipped_rather_than_guessed() {
        let hits = serde_json::json!([{ "data": {} }, { "key": "", "data": {} }]);

        assert!(works_from_hits(hits.as_array().unwrap()).is_empty());
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
