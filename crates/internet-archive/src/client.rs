//! Strict client: validates policy and addresses on every hop, pins the
//! validated address, follows redirects manually, and caps the body.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;
use url::{Host, Url};

use crate::error::IaError;
use crate::item::{parse_item_value, ItemDetails};
use crate::policy::FetchPolicy;
use crate::search::{is_valid_identifier, parse_search_value, Endpoints, SearchPage, SearchQuery};
use crate::{
    CONNECT_TIMEOUT, MAX_METADATA_BYTES, MAX_REDIRECTS, MAX_SEARCH_BYTES, REQUEST_TIMEOUT,
    USER_AGENT,
};

/// Stable sort for collection browsing: identifier ascending, so pages
/// are deterministic across requests.
const COLLECTION_SORT: &str = "identifier asc";

/// Strict client: validates policy and addresses on every hop, pins the
/// validated address, follows redirects manually, and caps the body.
#[derive(Debug, Clone)]
pub struct IaClient {
    policy: FetchPolicy,
}

impl IaClient {
    /// Build a client for the given policy.
    pub fn new(policy: FetchPolicy) -> Self {
        Self { policy }
    }

    /// GET `url` as JSON, rejecting bodies larger than `max_bytes`. Redirects
    /// are followed manually (at most [`MAX_REDIRECTS`]) with full
    /// re-validation; the connection for every hop is pinned to the address
    /// that was validated so DNS rebinding cannot swap it.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        max_bytes: usize,
    ) -> Result<T, IaError> {
        let initial = Url::parse(url).map_err(|e| IaError::Blocked {
            reason: format!("invalid URL: {e}"),
        })?;
        let response = self.fetch_final_response(initial).await?;
        let status = response.status();
        if status.as_u16() == 429 {
            let retry_after = parse_retry_after(response.headers());
            return Err(IaError::Http {
                status: 429,
                retry_after,
            });
        }
        if !status.is_success() {
            return Err(IaError::Http {
                status: status.as_u16(),
                retry_after: None,
            });
        }
        // Reject early when the declared length already exceeds the cap.
        // Compression is not requested (no gzip feature), so the cap
        // applies to the bytes actually parsed below.
        if let Some(len) = response.content_length() {
            if len > max_bytes as u64 {
                return Err(IaError::TooLarge { limit: max_bytes });
            }
        }
        let mut body = Vec::new();
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| IaError::Network(e.to_string()))?
        {
            body.extend_from_slice(&chunk);
            if body.len() > max_bytes {
                return Err(IaError::TooLarge { limit: max_bytes });
            }
        }
        serde_json::from_slice(&body).map_err(|e| IaError::Decode(e.to_string()))
    }

    /// Shared strict fetch: validate policy and addresses on every hop, pin
    /// the validated address, and follow redirects manually (at most
    /// [`MAX_REDIRECTS`]). Returns the first non-redirect response without
    /// interpreting its status, so JSON and download callers share the same
    /// pinning and validation instead of duplicating a weaker client.
    pub(crate) async fn fetch_final_response(
        &self,
        initial: Url,
    ) -> Result<reqwest::Response, IaError> {
        let mut current = initial;
        for hops in 0..=MAX_REDIRECTS {
            self.policy.check_url(&current)?;
            let pin = self.pinned_addr(&current).await?;
            let mut builder = reqwest::Client::builder()
                .redirect(Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .user_agent(USER_AGENT)
                .no_proxy();
            if let Some((domain, addr)) = &pin {
                builder = builder.resolve(domain.as_str(), *addr);
            }
            let client = builder
                .build()
                .map_err(|e| IaError::Network(e.to_string()))?;
            let response = client
                .get(current.clone())
                .send()
                .await
                .map_err(|e| IaError::Network(e.to_string()))?;
            let status = response.status();
            if status.is_redirection() {
                if hops == MAX_REDIRECTS {
                    return Err(IaError::TooManyRedirects);
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| IaError::Blocked {
                        reason: format!("redirect {status} without a valid Location"),
                    })?
                    .to_string();
                // Nothing is fetched from the target until the next loop
                // iteration re-validates policy and addresses.
                current = current.join(&location).map_err(|e| IaError::Blocked {
                    reason: format!("invalid redirect target: {e}"),
                })?;
                continue;
            }
            return Ok(response);
        }
        Err(IaError::TooManyRedirects)
    }

    /// Run an Advanced Search query: build `/advancedsearch.php` with
    /// `Url::query_pairs_mut` (never string concatenation), fetch it capped
    /// at [`MAX_SEARCH_BYTES`], and return the tolerant parsed page.
    ///
    /// `rows` is clamped to 1..=100 and `page` to >= 1; a page whose
    /// `page * rows` exceeds 10_000 (the deep-paging limit) is rejected with
    /// [`IaError::InvalidQuery`] before any request is sent. No `sort[]` is
    /// sent: result order is whatever the API returns.
    pub async fn search(
        &self,
        endpoints: &Endpoints,
        query: &SearchQuery,
        page: u32,
        rows: u32,
    ) -> Result<SearchPage, IaError> {
        self.advanced_search(endpoints, query, page, rows, None)
            .await
    }

    /// List one page of a collection with Advanced Search instead of the
    /// unreliable Scraping API (plan decision D5): the query is built
    /// through [`SearchQuery`] with `collection` set and `texts_only`, so
    /// escaping, `mediatype:texts` and the deep-paging rule are shared with
    /// [`IaClient::search`], plus exactly one `sort[]=identifier asc` pair
    /// for a stable order. Same `fl[]` fields, clamps and
    /// [`MAX_SEARCH_BYTES`] cap as `search`.
    ///
    /// The identifier follows the shared `^[A-Za-z0-9._-]{1,100}$` rule and
    /// is rejected with [`IaError::InvalidQuery`] before any request is
    /// sent.
    pub async fn list_collection(
        &self,
        endpoints: &Endpoints,
        collection: &str,
        page: u32,
        rows: u32,
    ) -> Result<SearchPage, IaError> {
        if !is_valid_identifier(collection) {
            return Err(IaError::InvalidQuery {
                reason: format!("invalid collection identifier: {collection:?}"),
            });
        }
        let query = SearchQuery {
            collection: Some(collection.to_string()),
            texts_only: true,
            ..SearchQuery::default()
        };
        self.advanced_search(endpoints, &query, page, rows, Some(COLLECTION_SORT))
            .await
    }

    /// Shared `/advancedsearch.php` request builder behind [`IaClient::search`]
    /// (no `sort[]`) and [`IaClient::list_collection`] (stable identifier
    /// sort). The query renders through [`SearchQuery::to_lucene`], `rows`
    /// clamps to 1..=100, `page` to >= 1, and `page * rows` past 10_000 is
    /// rejected before any request is sent.
    async fn advanced_search(
        &self,
        endpoints: &Endpoints,
        query: &SearchQuery,
        page: u32,
        rows: u32,
        sort: Option<&str>,
    ) -> Result<SearchPage, IaError> {
        const MAX_SEARCH_ROWS: u32 = 100;
        const MAX_SEARCH_OFFSET: u64 = 10_000;
        const SEARCH_FIELDS: [&str; 6] = [
            "identifier",
            "title",
            "creator",
            "date",
            "collection",
            "access-restricted-item",
        ];

        let lucene = query.to_lucene()?;
        let rows = rows.clamp(1, MAX_SEARCH_ROWS);
        let page = page.max(1);
        if (page as u64) * (rows as u64) > MAX_SEARCH_OFFSET {
            return Err(IaError::InvalidQuery {
                reason: format!(
                    "page {page} with {rows} rows exceeds the 10_000 deep-paging limit"
                ),
            });
        }
        let mut url = endpoints.base().clone();
        url.set_path("/advancedsearch.php");
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", &lucene);
            for field in SEARCH_FIELDS {
                pairs.append_pair("fl[]", field);
            }
            if let Some(sort) = sort {
                pairs.append_pair("sort[]", sort);
            }
            pairs
                .append_pair("rows", &rows.to_string())
                .append_pair("page", &page.to_string())
                .append_pair("output", "json");
        }
        let body: serde_json::Value = self.get_json(url.as_str(), MAX_SEARCH_BYTES).await?;
        Ok(parse_search_value(&body))
    }

    /// Fetch `/metadata/{identifier}` capped at [`MAX_METADATA_BYTES`] and
    /// return the tolerant parsed item. The identifier follows the same
    /// `^[A-Za-z0-9._-]{1,100}$` rule as collections and is rejected with
    /// [`IaError::InvalidQuery`] before any request is sent; an empty
    /// object (what the API answers for unknown identifiers) or a response
    /// without `metadata` maps to [`IaError::NotFound`].
    pub async fn get_item(
        &self,
        endpoints: &Endpoints,
        identifier: &str,
    ) -> Result<ItemDetails, IaError> {
        if !is_valid_identifier(identifier) {
            return Err(IaError::InvalidQuery {
                reason: format!("invalid item identifier: {identifier:?}"),
            });
        }
        let mut url = endpoints.base().clone();
        {
            let mut segments = url.path_segments_mut().map_err(|_| IaError::InvalidQuery {
                reason: "endpoint base URL cannot hold path segments".to_string(),
            })?;
            segments.clear();
            segments.push("metadata");
            segments.push(identifier);
        }
        let body: serde_json::Value = self.get_json(url.as_str(), MAX_METADATA_BYTES).await?;
        parse_item_value(identifier, &body)
    }

    /// Resolve the host and reject it when ANY address is non-public. Returns
    /// the pinned `(domain, address)` for DNS names, or `None` for IP
    /// literals (which need no pinning: there is no DNS to rebind).
    async fn pinned_addr(&self, url: &Url) -> Result<Option<(String, SocketAddr)>, IaError> {
        let port = url.port_or_known_default().unwrap_or(443);
        match url.host() {
            Some(Host::Ipv4(v4)) => {
                self.policy.ensure_addr_allowed(IpAddr::V4(v4))?;
                Ok(None)
            }
            Some(Host::Ipv6(v6)) => {
                self.policy.ensure_addr_allowed(IpAddr::V6(v6))?;
                Ok(None)
            }
            Some(Host::Domain(domain)) => {
                let addrs: Vec<SocketAddr> = tokio::net::lookup_host(format!("{domain}:{port}"))
                    .await
                    .map_err(|e| IaError::Network(e.to_string()))?
                    .collect();
                if addrs.is_empty() {
                    return Err(IaError::Network("no addresses resolved".to_string()));
                }
                for addr in &addrs {
                    self.policy.ensure_addr_allowed(addr.ip())?;
                }
                let pinned = SocketAddr::new(addrs[0].ip(), port);
                Ok(Some((domain.to_string(), pinned)))
            }
            None => Err(IaError::Blocked {
                reason: "URL has no host".to_string(),
            }),
        }
    }
}

/// Parse a `Retry-After` header in seconds; HTTP-dates and garbage map to None.
pub(crate) fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestServer;

    const FIXTURE: &str = include_str!("../tests/fixtures/advancedsearch-mar-del-plata.json");

    fn local_endpoints(server: &TestServer) -> Endpoints {
        Endpoints::new(Url::parse(server.base()).unwrap())
    }

    fn local_client() -> IaClient {
        IaClient::new(FetchPolicy::loopback_for_tests())
    }

    fn last_target(server: &TestServer) -> Url {
        let targets = server.targets();
        let target = targets.last().expect("server should have seen a request");
        Url::parse(&format!("http://127.0.0.1{target}")).unwrap()
    }

    fn pairs_of(url: &Url) -> Vec<(String, String)> {
        url.query_pairs().into_owned().collect()
    }

    fn get<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn list_collection_request_shape_exact() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{"response":{"numFound":0,"docs":[]}}"#,
        )]);
        let endpoints = local_endpoints(&server);
        let page = local_client()
            .list_collection(&endpoints, "opensource", 1, 5)
            .await
            .unwrap();
        assert_eq!(page.num_found, 0);
        assert!(page.items.is_empty());

        let url = last_target(&server);
        assert_eq!(url.path(), "/advancedsearch.php");
        let pairs = pairs_of(&url);
        assert_eq!(
            get(&pairs, "q"),
            Some("collection:(\"opensource\") AND mediatype:texts")
        );
        let sorts: Vec<&str> = pairs
            .iter()
            .filter(|(k, _)| k == "sort[]")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(sorts, ["identifier asc"]);
        let fls: Vec<&str> = pairs
            .iter()
            .filter(|(k, _)| k == "fl[]")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            fls,
            [
                "identifier",
                "title",
                "creator",
                "date",
                "collection",
                "access-restricted-item"
            ]
        );
        assert_eq!(get(&pairs, "rows"), Some("5"));
        assert_eq!(get(&pairs, "page"), Some("1"));
        assert_eq!(get(&pairs, "output"), Some("json"));
    }

    #[tokio::test]
    async fn search_request_shape_has_no_sort() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{"response":{"numFound":0,"docs":[]}}"#,
        )]);
        let endpoints = local_endpoints(&server);
        local_client()
            .search(
                &endpoints,
                &SearchQuery {
                    title: Some("Mar del Plata".to_string()),
                    ..SearchQuery::default()
                },
                1,
                50,
            )
            .await
            .unwrap();
        let url = last_target(&server);
        let pairs = pairs_of(&url);
        assert!(
            pairs.iter().all(|(k, _)| k != "sort[]"),
            "search must not send sort[], got {pairs:?}"
        );
    }

    #[tokio::test]
    async fn list_collection_invalid_id_rejected_without_request() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{"response":{"numFound":0,"docs":[]}}"#,
        )]);
        let endpoints = local_endpoints(&server);
        for bad in ["has space", "evil!", "", "a/b"] {
            let res = local_client().list_collection(&endpoints, bad, 1, 5).await;
            assert!(
                matches!(res, Err(IaError::InvalidQuery { .. })),
                "{bad:?} should be rejected, got {res:?}"
            );
        }
        assert!(
            server.targets().is_empty(),
            "no request must be sent, saw {:?}",
            server.targets()
        );
    }

    #[tokio::test]
    async fn list_collection_deep_paging_rejected_without_request() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{"response":{"numFound":0,"docs":[]}}"#,
        )]);
        let endpoints = local_endpoints(&server);
        let res = local_client()
            .list_collection(&endpoints, "opensource", 101, 100)
            .await;
        assert!(
            matches!(res, Err(IaError::InvalidQuery { .. })),
            "got {res:?}"
        );
        assert!(
            server.targets().is_empty(),
            "no request must be sent, saw {:?}",
            server.targets()
        );
        // The boundary itself is still allowed.
        local_client()
            .list_collection(&endpoints, "opensource", 100, 100)
            .await
            .unwrap();
        assert_eq!(server.targets().len(), 1);
    }

    #[tokio::test]
    async fn list_collection_parses_page_tolerantly() {
        let server = TestServer::start(vec![TestServer::json("/advancedsearch.php", 200, FIXTURE)]);
        let endpoints = local_endpoints(&server);
        let page = local_client()
            .list_collection(&endpoints, "opensource", 1, 3)
            .await
            .unwrap();
        assert_eq!(page.num_found, 283);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.items[0].identifier, "proceedingsofvic00cong");
    }

    #[tokio::test]
    async fn redirect_within_limit_followed() {
        let server = TestServer::start(vec![
            TestServer::redirect("/start", "/final"),
            TestServer::json("/final", 200, r#"{"v":1}"#),
        ]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let got: serde_json::Value = client
            .get_json(&format!("{}/start", server.base()), 1024 * 1024)
            .await
            .expect("redirect should be followed");
        assert_eq!(got["v"], 1);
    }

    #[tokio::test]
    async fn redirect_to_disallowed_host_blocked() {
        let server = TestServer::start(vec![TestServer::redirect(
            "/evil",
            "https://archive.org.evil.com/",
        )]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let res: Result<serde_json::Value, _> = client
            .get_json(&format!("{}/evil", server.base()), 1024 * 1024)
            .await;
        assert!(
            matches!(res, Err(IaError::Blocked { .. })),
            "evil redirect should be blocked, got {res:?}"
        );
    }

    #[tokio::test]
    async fn six_redirects_too_many() {
        let mut routes = Vec::new();
        for i in 0..6 {
            routes.push(TestServer::redirect(
                &format!("/r{i}"),
                &format!("/r{}", i + 1),
            ));
        }
        routes.push(TestServer::json("/r6", 200, r#"{"v":1}"#));
        let server = TestServer::start(routes);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let res: Result<serde_json::Value, _> = client
            .get_json(&format!("{}/r0", server.base()), 1024 * 1024)
            .await;
        assert!(
            matches!(res, Err(IaError::TooManyRedirects)),
            "6 redirects should fail, got {res:?}"
        );
    }

    #[tokio::test]
    async fn content_length_over_cap_rejected() {
        let server = TestServer::start(vec![TestServer::raw(
            "/big",
            200,
            vec![("Content-Length".into(), "100".into())],
            b"x".repeat(100),
        )]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let res: Result<serde_json::Value, _> =
            client.get_json(&format!("{}/big", server.base()), 10).await;
        assert!(
            matches!(res, Err(IaError::TooLarge { limit: 10 })),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn streamed_body_over_cap_rejected() {
        let server = TestServer::start(vec![TestServer::raw(
            "/stream",
            200,
            vec![],
            b"y".repeat(100),
        )]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let res: Result<serde_json::Value, _> = client
            .get_json(&format!("{}/stream", server.base()), 10)
            .await;
        assert!(
            matches!(res, Err(IaError::TooLarge { limit: 10 })),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn retry_after_parsed() {
        let server = TestServer::start(vec![TestServer::raw(
            "/limited",
            429,
            vec![("Retry-After".into(), "120".into())],
            b"{}".to_vec(),
        )]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let res: Result<serde_json::Value, _> = client
            .get_json(&format!("{}/limited", server.base()), 1024)
            .await;
        match res {
            Err(IaError::Http {
                status: 429,
                retry_after,
            }) => assert_eq!(retry_after, Some(std::time::Duration::from_secs(120))),
            other => panic!("expected 429 with retry_after, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn valid_json_decoded() {
        let server = TestServer::start(vec![TestServer::json("/ok", 200, r#"{"title":"hello"}"#)]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let got: serde_json::Value = client
            .get_json(&format!("{}/ok", server.base()), 1024)
            .await
            .unwrap();
        assert_eq!(got["title"], "hello");
    }

    #[tokio::test]
    async fn invalid_json_decode_error() {
        let server = TestServer::start(vec![TestServer::raw(
            "/bad",
            200,
            vec![("Content-Length".into(), "8".into())],
            b"not json".to_vec(),
        )]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let res: Result<serde_json::Value, _> = client
            .get_json(&format!("{}/bad", server.base()), 1024)
            .await;
        assert!(matches!(res, Err(IaError::Decode(_))), "got {res:?}");
    }
}
