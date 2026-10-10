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
        let mut current = Url::parse(url).map_err(|e| IaError::Blocked {
            reason: format!("invalid URL: {e}"),
        })?;
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
            return serde_json::from_slice(&body).map_err(|e| IaError::Decode(e.to_string()));
        }
        Err(IaError::TooManyRedirects)
    }

    /// Run an Advanced Search query: build `/advancedsearch.php` with
    /// `Url::query_pairs_mut` (never string concatenation), fetch it capped
    /// at [`MAX_SEARCH_BYTES`], and return the tolerant parsed page.
    ///
    /// `rows` is clamped to 1..=100 and `page` to >= 1; a page whose
    /// `page * rows` exceeds 10_000 (the deep-paging limit) is rejected with
    /// [`IaError::InvalidQuery`] before any request is sent.
    pub async fn search(
        &self,
        endpoints: &Endpoints,
        query: &SearchQuery,
        page: u32,
        rows: u32,
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
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestServer;

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
