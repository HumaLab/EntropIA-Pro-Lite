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
    format!(
        "{BASE_URL}/api/users/{library}/items?format=csljson&limit={}&start={}",
        page.limit(),
        page.start()
    )
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
