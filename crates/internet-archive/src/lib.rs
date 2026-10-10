//! Strict HTTP fetch policy for the Internet Archive connector.
//!
//! Only `https://archive.org` and `https://*.archive.org` are reachable in
//! production, every resolved address is checked against the public-internet
//! predicates (mirroring `url_policy.rs` in the desktop app), the validated
//! address is pinned per request so DNS rebinding cannot swap it, redirects
//! are followed manually with re-validation, and response bodies are capped.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;
use url::{Host, Url};

/// Cap for item metadata payloads.
pub const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
/// Cap for search payloads.
pub const MAX_SEARCH_BYTES: usize = 4 * 1024 * 1024;
/// How long a single redirect chain may grow (follows, not requests).
pub const MAX_REDIRECTS: usize = 5;
/// TCP connect timeout, kept separate from the overall request timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall request timeout (connect + response body).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Descriptive agent so archive.org operators can identify the connector.
pub const USER_AGENT: &str = concat!(
    "EntropIA/",
    env!("CARGO_PKG_VERSION"),
    " (+internet-archive connector)"
);

/// Fetch failure modes. Remote input never panics; it maps to one of these.
#[derive(Debug, thiserror::Error)]
pub enum IaError {
    /// The URL or a redirect target violates the fetch policy.
    #[error("blocked: {reason}")]
    Blocked { reason: String },
    /// More than [`MAX_REDIRECTS`] redirect hops were requested.
    #[error("too many redirects")]
    TooManyRedirects,
    /// The body exceeds the caller-supplied cap.
    #[error("response body exceeds limit of {limit} bytes")]
    TooLarge { limit: usize },
    /// A non-success HTTP status. `retry_after` is set for 429 with seconds.
    #[error("http error: {status}")]
    Http {
        status: u16,
        retry_after: Option<Duration>,
    },
    /// Transport, DNS or timeout failure.
    #[error("network error: {0}")]
    Network(String),
    /// The body was received within the cap but is not the expected JSON.
    #[error("decode error: {0}")]
    Decode(String),
}

/// Which URLs may be fetched.
#[derive(Debug, Clone)]
pub struct FetchPolicy {
    allow_http_loopback: bool,
}

impl FetchPolicy {
    /// Production: only `https` archive.org hosts on port 443.
    pub fn production() -> Self {
        Self {
            allow_http_loopback: false,
        }
    }

    /// Relaxed policy for local tests: additionally allows `http` on
    /// `127.0.0.1` with any port. Only compiled for tests or with the
    /// `test-support` feature so production builds cannot enable it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_for_tests() -> Self {
        Self {
            allow_http_loopback: true,
        }
    }

    /// Reject anything that is not an allowed scheme/host/port combination.
    /// Host comparison is lowercased with a label boundary so lookalikes such
    /// as `archive.org.evil.com` or `xarchive.org` are rejected.
    pub fn check_url(&self, url: &Url) -> Result<(), IaError> {
        if !url.username().is_empty() || url.password().is_some() {
            return Err(IaError::Blocked {
                reason: "userinfo in URL is not allowed".to_string(),
            });
        }
        let host = url.host_str().ok_or_else(|| IaError::Blocked {
            reason: "URL has no host".to_string(),
        })?;
        let host_lc = host.to_ascii_lowercase();
        // Relaxed local case first: `http://127.0.0.1:<any>/`, userinfo
        // already rejected above. Everything else falls through to the
        // production rules (so `https://archive.org` also works for tests).
        if self.allow_http_loopback && host_lc == "127.0.0.1" && url.scheme() == "http" {
            return Ok(());
        }
        if url.scheme() != "https" {
            return Err(IaError::Blocked {
                reason: format!("scheme '{}' is not allowed", url.scheme()),
            });
        }
        match url.host() {
            Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => {
                return Err(IaError::Blocked {
                    reason: "IP-literal hosts are not allowed".to_string(),
                });
            }
            Some(Host::Domain(_)) => {}
            None => {
                return Err(IaError::Blocked {
                    reason: "URL has no host".to_string(),
                });
            }
        }
        let host_ok = host_lc == "archive.org"
            || (host_lc.ends_with(".archive.org")
                && host_lc.len() > ".archive.org".len()
                && !host_lc.starts_with('.')
                && !host_lc.contains(".."));
        if !host_ok {
            return Err(IaError::Blocked {
                reason: format!("host '{host}' is not allowed"),
            });
        }
        let port = url.port_or_known_default().unwrap_or(443);
        if port != 443 {
            return Err(IaError::Blocked {
                reason: format!("port {port} is not allowed"),
            });
        }
        Ok(())
    }

    fn ensure_addr_allowed(&self, ip: IpAddr) -> Result<(), IaError> {
        if is_public_ip(ip) {
            return Ok(());
        }
        // The relaxed test policy still blocks every non-public class except
        // loopback, which is where the local test server lives.
        if self.allow_http_loopback && ip.is_loopback() {
            return Ok(());
        }
        Err(IaError::Blocked {
            reason: format!("non-public address: {ip}"),
        })
    }
}

/// True when the address is on the public internet. The predicates mirror
/// `is_public_v4` / `is_public_v6` in
/// `apps/desktop/src-tauri/src/navegador/url_policy.rs`, plus documentation
/// ranges and IPv4-compatible IPv6 forms checked as their embedded IPv4.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _d] = ip.octets();
    if a == 0 {
        return false; // this network / unspecified
    }
    if a == 10 {
        return false;
    }
    if a == 100 && (64..=127).contains(&b) {
        return false; // CGNAT 100.64/10
    }
    if a == 127 {
        return false; // loopback
    }
    if a == 169 && b == 254 {
        return false; // link-local (cloud metadata lives here)
    }
    if a == 172 && (16..=31).contains(&b) {
        return false;
    }
    if a == 192 && b == 0 && c == 0 {
        return false; // IETF protocol assignments
    }
    if a == 192 && b == 0 && c == 2 {
        return false; // documentation TEST-NET-1
    }
    if a == 192 && b == 168 {
        return false;
    }
    if a == 198 && (b == 18 || b == 19) {
        return false; // benchmarking
    }
    if a == 198 && b == 51 && c == 100 {
        return false; // documentation TEST-NET-2
    }
    if a == 203 && b == 0 && c == 113 {
        return false; // documentation TEST-NET-3
    }
    if a >= 224 {
        return false; // multicast, reserved, broadcast
    }
    if ip.is_broadcast() {
        return false;
    }
    true
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // IPv4-mapped (::ffff:0:0/96) and IPv4-compatible (::: /96) forms carry an
    // IPv4 address; judge them as that IPv4.
    if s[0] == 0
        && s[1] == 0
        && s[2] == 0
        && s[3] == 0
        && s[4] == 0
        && (s[5] == 0 || s[5] == 0xffff)
    {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_public_v4(v4);
    }
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    if ip.is_unspecified() || ip.is_loopback() {
        return false;
    }
    if s[0] & 0xfe00 == 0xfc00 {
        return false; // unique-local fc00::/7
    }
    if s[0] & 0xffc0 == 0xfe80 {
        return false; // link-local fe80::/10
    }
    if s[0] & 0xff00 == 0xff00 {
        return false; // multicast ff00::/8
    }
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return false; // documentation 2001:db8::/32
    }
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) embed an IPv4 address.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_public_v4(v4);
    }
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8);
        return is_public_v4(v4);
    }
    true
}

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
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn check_production(url: &str) -> Result<(), IaError> {
        let parsed = Url::parse(url).expect("test URL must parse");
        FetchPolicy::production().check_url(&parsed)
    }

    #[test]
    fn host_rules_accept() {
        for url in [
            "https://archive.org/",
            "https://ARCHIVE.ORG/",
            "https://dn711501.ca.archive.org/",
            "https://ia801501.us.archive.org/",
            "https://foo.bar.archive.org/",
        ] {
            assert!(check_production(url).is_ok(), "{url} should be accepted");
        }
    }

    #[test]
    fn host_rules_reject_lookalikes() {
        for url in [
            "https://archive.org.evil.com/",
            "https://xarchive.org/",
            "https://fooarchive.org/",
            "https://evil.com/",
            "https://127.0.0.1/",
            "https://1.2.3.4/",
            "https://user@archive.org/",
            "https://user:pass@archive.org/",
        ] {
            assert!(
                matches!(check_production(url), Err(IaError::Blocked { .. })),
                "{url} should be blocked"
            );
        }
    }

    #[test]
    fn scheme_and_port_rejected() {
        assert!(matches!(
            check_production("http://archive.org/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(matches!(
            check_production("ftp://archive.org/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(matches!(
            check_production("https://archive.org:8443/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(matches!(
            check_production("https://archive.org:80/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(check_production("https://archive.org:443/").is_ok());
    }

    #[test]
    fn non_public_ips_blocked() {
        // (addr, should_be_public)
        let cases: Vec<(IpAddr, bool)> = vec![
            (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255)), false),
            (IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), false),
            (IpAddr::V4(Ipv4Addr::new(169, 254, 10, 20)), false),
            (IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(100, 127, 255, 255)), false),
            (IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)), false),
            (IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)), false),
            (IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), false),
            (IpAddr::V4(Ipv4Addr::new(192, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), true),
            (IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), true),
            (IpAddr::V6(Ipv6Addr::LOCALHOST), false),
            (IpAddr::V6(Ipv6Addr::UNSPECIFIED), false),
            ("fe80::1".parse().unwrap(), false),
            ("fc00::1".parse().unwrap(), false),
            ("fd00::1".parse().unwrap(), false),
            ("ff02::1".parse().unwrap(), false),
            ("2001:db8::1".parse().unwrap(), false),
            ("::ffff:10.0.0.1".parse().unwrap(), false),
            ("::ffff:8.8.8.8".parse().unwrap(), true),
            (
                IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0x0a00, 0x0001)),
                false,
            ),
            ("2606:4700:4700::1111".parse().unwrap(), true),
        ];
        for (ip, public) in cases {
            assert_eq!(is_public_ip(ip), public, "{ip} public={public}");
        }
    }

    #[test]
    fn relaxed_policy_allows_http_loopback() {
        let policy = FetchPolicy::loopback_for_tests();
        let url = Url::parse("http://127.0.0.1:12345/").unwrap();
        assert!(policy.check_url(&url).is_ok());
    }

    #[tokio::test]
    async fn redirect_within_limit_followed() {
        let server = TestServer::start(vec![
            TestServer::redirect("/start", "/final"),
            TestServer::json("/final", 200, r#"{"v":1}"#),
        ]);
        let client = IaClient::new(FetchPolicy::loopback_for_tests());
        let got: serde_json::Value = client
            .get_json(&format!("{}/start", server.base), 1024 * 1024)
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
            .get_json(&format!("{}/evil", server.base), 1024 * 1024)
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
            .get_json(&format!("{}/r0", server.base), 1024 * 1024)
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
            client.get_json(&format!("{}/big", server.base), 10).await;
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
            .get_json(&format!("{}/stream", server.base), 10)
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
            .get_json(&format!("{}/limited", server.base), 1024)
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
            .get_json(&format!("{}/ok", server.base), 1024)
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
        let res: Result<serde_json::Value, _> =
            client.get_json(&format!("{}/bad", server.base), 1024).await;
        assert!(matches!(res, Err(IaError::Decode(_))), "got {res:?}");
    }

    // Tiny hand-rolled HTTP server for tests: canned answers by exact path,
    // no external crates, no real network. Mirrors the `test_server` pattern
    // in `navegador/zotero_copy/port.rs`.
    struct Route {
        path: String,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    struct TestServer {
        base: String,
    }

    impl TestServer {
        fn redirect(path: &str, location: &str) -> Route {
            Route {
                path: path.to_string(),
                status: 302,
                headers: vec![
                    ("Location".to_string(), location.to_string()),
                    ("Content-Length".to_string(), "0".to_string()),
                ],
                body: Vec::new(),
            }
        }

        fn json(path: &str, status: u16, body: &str) -> Route {
            let bytes = body.as_bytes().to_vec();
            Route {
                path: path.to_string(),
                status,
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Content-Length".to_string(), bytes.len().to_string()),
                ],
                body: bytes,
            }
        }

        fn raw(path: &str, status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Route {
            Route {
                path: path.to_string(),
                status,
                headers,
                body,
            }
        }

        fn start(routes: Vec<Route>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let seen: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 4096];
                    let head_end = loop {
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            break None;
                        }
                        raw.extend_from_slice(&buf[..n]);
                        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                            break Some(at + 4);
                        }
                        if raw.len() > 64 * 1024 {
                            break None;
                        }
                    };
                    let Some(head_end) = head_end else { continue };
                    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
                    let mut lines = head.lines();
                    let request_line = lines.next().unwrap_or("").to_string();
                    let mut parts = request_line.split_whitespace();
                    let target = parts.nth(1).unwrap_or("/").to_string();
                    let path_only = target
                        .split(['?', '#'])
                        .next()
                        .unwrap_or(&target)
                        .to_string();
                    seen.lock()
                        .unwrap()
                        .entry(path_only.clone())
                        .and_modify(|c| *c += 1)
                        .or_insert(1);
                    let route = routes.iter().find(|r| r.path == path_only);
                    let (status, headers, body) = match route {
                        Some(r) => (r.status, r.headers.clone(), r.body.clone()),
                        None => (
                            404,
                            vec![("Content-Length".to_string(), "0".to_string())],
                            Vec::new(),
                        ),
                    };
                    let mut reply = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n");
                    for (k, v) in &headers {
                        reply.push_str(&format!("{k}: {v}\r\n"));
                    }
                    reply.push_str("\r\n");
                    let _ = stream.write_all(reply.as_bytes());
                    let _ = stream.write_all(&body);
                }
            });
            Self { base }
        }
    }
}
