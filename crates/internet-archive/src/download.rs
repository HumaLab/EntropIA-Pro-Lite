//! Streaming download to staging (Phase 3 task 17).
//!
//! Downloads `https://archive.org/download/{identifier}/{file}` with the
//! same strict policy, address pinning and manual redirect loop as the JSON
//! client, streaming to `<dest>.partial` under a byte cap. No `.part`
//! files are kept and no `Range` resume is attempted: an interrupted
//! download is deleted and started again by the caller.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::client::{FetchKind, IaClient};
use crate::error::IaError;
use crate::search::{is_valid_identifier, Endpoints};

/// Default per-file download cap (500 MiB).
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;
/// Absolute per-file download cap (2 GiB); requests above are clamped.
pub const ABSOLUTE_MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Byte cap for one download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadLimits {
    /// Maximum bytes accepted for one file.
    pub max_bytes: u64,
}

impl DownloadLimits {
    /// Clamp an optional request: `None` uses the default, `0` (disable)
    /// and anything above the absolute max clamp to the absolute max, and
    /// values in between are honored. Never returns `0` nor anything above
    /// [`ABSOLUTE_MAX_DOWNLOAD_BYTES`].
    #[must_use]
    pub fn clamped(requested: Option<u64>) -> Self {
        match requested {
            None => Self {
                max_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            },
            Some(0) => Self {
                max_bytes: ABSOLUTE_MAX_DOWNLOAD_BYTES,
            },
            Some(v) => Self {
                max_bytes: v.clamp(1, ABSOLUTE_MAX_DOWNLOAD_BYTES),
            },
        }
    }
}

/// Retry tuning for transient download failures (5xx and network errors).
/// 4xx (including 429) is never retried here; the caller decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// How many times to retry after the first attempt (2 = 3 attempts).
    pub max_retries: u32,
    /// Base delay between attempts; backs off exponentially.
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::from_millis(10),
        }
    }
}

impl RetryPolicy {
    /// Zero-delay policy for tests: same retry count, no sleeping.
    #[must_use]
    pub fn no_delay() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::ZERO,
        }
    }
}

/// True when `remote_name` is a safe relative download path: non-empty,
/// not absolute, no `..` segment, no backslash, no control characters.
/// Mirrors the `is_unsafe_name` rule in `item.rs` (inverted).
fn is_valid_remote_name(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.starts_with('/') {
        return false;
    }
    if value.contains('\\') {
        return false;
    }
    if value.split('/').any(|segment| segment == "..") {
        return false;
    }
    if value.chars().any(|c| c.is_control()) {
        return false;
    }
    true
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".partial");
    PathBuf::from(s)
}

fn remove_partial_quiet(partial: &Path) {
    let _ = std::fs::remove_file(partial);
}

async fn backoff_sleep(retry: &RetryPolicy, attempt: u32) {
    if retry.base_delay.is_zero() {
        return;
    }
    let delay = retry.base_delay.saturating_mul(2u32.pow(attempt.min(10)));
    tokio::time::sleep(delay).await;
}

impl IaClient {
    /// Download one file with the default [`RetryPolicy`].
    pub async fn download_file(
        &self,
        endpoints: &Endpoints,
        identifier: &str,
        remote_name: &str,
        dest: &Path,
        limits: &DownloadLimits,
        expected_size: Option<u64>,
    ) -> Result<u64, IaError> {
        self.download_file_with_retry(
            endpoints,
            identifier,
            remote_name,
            dest,
            limits,
            expected_size,
            &RetryPolicy::default(),
        )
        .await
    }

    /// Download one file with an injectable [`RetryPolicy`] (tests pass
    /// [`RetryPolicy::no_delay`] so they never sleep for seconds).
    #[allow(clippy::too_many_arguments)]
    pub async fn download_file_with_retry(
        &self,
        endpoints: &Endpoints,
        identifier: &str,
        remote_name: &str,
        dest: &Path,
        limits: &DownloadLimits,
        expected_size: Option<u64>,
        retry: &RetryPolicy,
    ) -> Result<u64, IaError> {
        if !is_valid_identifier(identifier) {
            return Err(IaError::InvalidQuery {
                reason: format!("invalid item identifier: {identifier:?}"),
            });
        }
        if !is_valid_remote_name(remote_name) {
            return Err(IaError::InvalidQuery {
                reason: format!("invalid remote file name: {remote_name:?}"),
            });
        }
        if let Some(size) = expected_size {
            if size > limits.max_bytes {
                return Err(IaError::FileTooLarge {
                    limit: limits.max_bytes,
                });
            }
        }
        let partial = partial_path(dest);
        // Do not overwrite an existing destination. This early check saves
        // a download; the late check below closes the race where `dest`
        // appears while we stream.
        if std::fs::symlink_metadata(dest).is_ok() {
            remove_partial_quiet(&partial);
            return Err(IaError::AlreadyExists {
                path: dest.display().to_string(),
            });
        }
        // Build `/download/{id}/{file}` with `Url` path segments so each
        // `/`-separated piece of `remote_name` is percent-encoded by the
        // URL type; never string concatenation.
        let mut url = endpoints.base().clone();
        {
            let mut segments = url.path_segments_mut().map_err(|_| IaError::InvalidQuery {
                reason: "endpoint base URL cannot hold path segments".to_string(),
            })?;
            segments.clear();
            segments.push("download");
            segments.push(identifier);
            for piece in remote_name.split('/') {
                segments.push(piece);
            }
        }

        let mut attempt: u32 = 0;
        loop {
            let response = match self
                .fetch_final_response(url.clone(), FetchKind::Download)
                .await
            {
                Ok(response) => response,
                Err(IaError::Network(_)) if attempt < retry.max_retries => {
                    backoff_sleep(retry, attempt).await;
                    attempt += 1;
                    continue;
                }
                Err(other) => {
                    remove_partial_quiet(&partial);
                    return Err(other);
                }
            };
            let status = response.status();
            let code = status.as_u16();
            if code == 429 {
                let retry_after = crate::client::parse_retry_after(response.headers());
                remove_partial_quiet(&partial);
                return Err(IaError::Http {
                    status: 429,
                    retry_after,
                });
            }
            if code == 401 || code == 403 || code == 404 {
                remove_partial_quiet(&partial);
                return Err(IaError::Http {
                    status: code,
                    retry_after: None,
                });
            }
            if status.is_server_error() {
                if attempt < retry.max_retries {
                    remove_partial_quiet(&partial);
                    backoff_sleep(retry, attempt).await;
                    attempt += 1;
                    continue;
                }
                remove_partial_quiet(&partial);
                return Err(IaError::Http {
                    status: code,
                    retry_after: None,
                });
            }
            if !status.is_success() {
                remove_partial_quiet(&partial);
                return Err(IaError::Http {
                    status: code,
                    retry_after: None,
                });
            }
            // Reject early when the declared length already exceeds the
            // cap, before creating any file.
            if let Some(len) = response.content_length() {
                if len > limits.max_bytes {
                    remove_partial_quiet(&partial);
                    return Err(IaError::FileTooLarge {
                        limit: limits.max_bytes,
                    });
                }
            }
            match stream_to_partial(response, &partial, limits).await {
                Ok(written) => {
                    // Late existence check closes the TOCTOU race with the
                    // early check above.
                    if std::fs::symlink_metadata(dest).is_ok() {
                        remove_partial_quiet(&partial);
                        return Err(IaError::AlreadyExists {
                            path: dest.display().to_string(),
                        });
                    }
                    if let Err(e) = std::fs::rename(&partial, dest) {
                        // A race that created `dest` between the check and
                        // the rename still must not overwrite it.
                        if std::fs::symlink_metadata(dest).is_ok() {
                            remove_partial_quiet(&partial);
                            return Err(IaError::AlreadyExists {
                                path: dest.display().to_string(),
                            });
                        }
                        remove_partial_quiet(&partial);
                        return Err(IaError::Io(e.to_string()));
                    }
                    return Ok(written);
                }
                Err(StreamError::TooLarge) => {
                    remove_partial_quiet(&partial);
                    return Err(IaError::FileTooLarge {
                        limit: limits.max_bytes,
                    });
                }
                Err(StreamError::Network(_)) if attempt < retry.max_retries => {
                    remove_partial_quiet(&partial);
                    backoff_sleep(retry, attempt).await;
                    attempt += 1;
                    continue;
                }
                Err(StreamError::Network(message)) => {
                    remove_partial_quiet(&partial);
                    return Err(IaError::Network(message));
                }
                Err(StreamError::Io(message)) => {
                    remove_partial_quiet(&partial);
                    return Err(IaError::Io(message));
                }
            }
        }
    }
}

enum StreamError {
    TooLarge,
    Network(String),
    Io(String),
}

/// Stream a successful response body into `partial`, counting bytes and
/// stopping as soon as the count exceeds `limits.max_bytes`. On success
/// the file is flushed and `sync_all`ed; the caller renames it onto `dest`.
async fn stream_to_partial(
    response: reqwest::Response,
    partial: &Path,
    limits: &DownloadLimits,
) -> Result<u64, StreamError> {
    let mut file = std::fs::File::create(partial).map_err(|e| StreamError::Io(e.to_string()))?;
    let mut written: u64 = 0;
    let mut response = response;
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|e| StreamError::Network(e.to_string()))?;
        let Some(bytes) = chunk else {
            break;
        };
        written = written.saturating_add(bytes.len() as u64);
        if written > limits.max_bytes {
            drop(file);
            return Err(StreamError::TooLarge);
        }
        file.write_all(&bytes)
            .map_err(|e| StreamError::Io(e.to_string()))?;
    }
    file.flush().map_err(|e| StreamError::Io(e.to_string()))?;
    file.sync_all()
        .map_err(|e| StreamError::Io(e.to_string()))?;
    drop(file);
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Timeouts;
    use crate::policy::FetchPolicy;
    use crate::test_support::TestServer;
    use url::Url;

    fn local_client() -> IaClient {
        IaClient::new(FetchPolicy::loopback_for_tests())
    }

    fn local_endpoints(server: &TestServer) -> Endpoints {
        Endpoints::new(Url::parse(server.base()).unwrap())
    }

    fn partial_of(dest: &Path) -> PathBuf {
        let mut s = dest.as_os_str().to_owned();
        s.push(".partial");
        PathBuf::from(s)
    }

    #[test]
    fn limits_clamp_table() {
        assert_eq!(
            DownloadLimits::clamped(None).max_bytes,
            DEFAULT_MAX_DOWNLOAD_BYTES
        );
        assert_eq!(
            DownloadLimits::clamped(Some(DEFAULT_MAX_DOWNLOAD_BYTES)).max_bytes,
            DEFAULT_MAX_DOWNLOAD_BYTES
        );
        assert_eq!(DownloadLimits::clamped(Some(1024)).max_bytes, 1024);
        // Disable (0) is clamped, never honored as 0.
        let zero = DownloadLimits::clamped(Some(0)).max_bytes;
        assert_ne!(zero, 0);
        assert!(zero <= ABSOLUTE_MAX_DOWNLOAD_BYTES);
        // Over the absolute max clamps down.
        assert_eq!(
            DownloadLimits::clamped(Some(ABSOLUTE_MAX_DOWNLOAD_BYTES + 1)).max_bytes,
            ABSOLUTE_MAX_DOWNLOAD_BYTES
        );
        assert_eq!(
            DownloadLimits::clamped(Some(u64::MAX)).max_bytes,
            ABSOLUTE_MAX_DOWNLOAD_BYTES
        );
        assert_eq!(
            DownloadLimits::clamped(Some(ABSOLUTE_MAX_DOWNLOAD_BYTES)).max_bytes,
            ABSOLUTE_MAX_DOWNLOAD_BYTES
        );
    }

    #[tokio::test]
    async fn request_target_encodes_segments() {
        let body = b"%PDF-1.4 small".to_vec();
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/scans/vol%201.pdf",
            200,
            vec![("Content-Length".into(), body.len().to_string())],
            body.clone(),
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("vol 1.pdf");
        let limits = DownloadLimits::clamped(None);
        let n = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "scans/vol 1.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap();
        assert_eq!(n as usize, body.len());
        let targets = server.targets();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0], "/download/myid/scans/vol%201.pdf");
    }

    #[tokio::test]
    async fn expected_size_over_cap_sends_no_request() {
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/a.pdf",
            200,
            vec![],
            b"hello".to_vec(),
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("a.pdf");
        let limits = DownloadLimits::clamped(Some(10));
        let err = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "a.pdf",
                &dest,
                &limits,
                Some(11),
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, IaError::FileTooLarge { .. }), "got {err:?}");
        assert!(
            format!("{err}").contains("archivo demasiado grande"),
            "display must contain marker, got {err}"
        );
        assert!(server.targets().is_empty());
        assert!(!dest.exists());
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn content_length_over_cap_writes_nothing() {
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/big.pdf",
            200,
            vec![("Content-Length".into(), "100".into())],
            b"x".repeat(100),
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("big.pdf");
        let limits = DownloadLimits::clamped(Some(10));
        let err = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "big.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, IaError::FileTooLarge { .. }), "got {err:?}");
        assert!(!dest.exists());
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn streaming_body_over_cap_deletes_partial() {
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/stream.pdf",
            200,
            vec![],
            b"y".repeat(100),
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("stream.pdf");
        let limits = DownloadLimits::clamped(Some(10));
        let err = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "stream.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, IaError::FileTooLarge { .. }), "got {err:?}");
        assert!(!dest.exists());
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn download_slower_than_json_total_succeeds_with_idle_timeout() {
        // 10 one-byte chunks spaced 60 ms apart take ~540 ms on the wire:
        // past a 200 ms total budget, but fine under a 300 ms idle budget.
        let body: Vec<u8> = (0..10u8).collect();
        let server = TestServer::start(vec![TestServer::chunked(
            "/download/myid/slow.pdf",
            200,
            vec![("Content-Length".into(), body.len().to_string())],
            body.clone(),
            1,
            Duration::from_millis(60),
        )]);
        let timeouts = Timeouts {
            connect: Duration::from_secs(10),
            json_total: Duration::from_millis(200),
            download_idle: Duration::from_millis(300),
            download_total: Duration::from_secs(30),
        };
        let client = IaClient::with_timeouts(FetchPolicy::loopback_for_tests(), timeouts);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("slow.pdf");
        let limits = DownloadLimits::clamped(None);
        let n = client
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "slow.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .expect("download under the idle budget must succeed");
        assert_eq!(n as usize, body.len());
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn download_idle_stall_maps_to_network_without_trace() {
        let body = b"0123456789".to_vec();
        let server = TestServer::start(vec![TestServer::stall(
            "/download/myid/stall.pdf",
            200,
            vec![("Content-Length".into(), body.len().to_string())],
            body,
            Duration::from_secs(1),
        )]);
        let timeouts = Timeouts {
            connect: Duration::from_secs(10),
            json_total: Duration::from_secs(30),
            download_idle: Duration::from_millis(200),
            download_total: Duration::from_secs(30),
        };
        let client = IaClient::with_timeouts(FetchPolicy::loopback_for_tests(), timeouts);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("stall.pdf");
        let limits = DownloadLimits::clamped(None);
        let retry = RetryPolicy {
            max_retries: 0,
            base_delay: Duration::ZERO,
        };
        let err = client
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "stall.pdf",
                &dest,
                &limits,
                None,
                &retry,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, IaError::Network(_)),
            "idle stall must map to Network, got {err:?}"
        );
        assert!(!dest.exists());
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn success_writes_dest_and_removes_partial() {
        let body = b"%PDF-1.4 fake body".to_vec();
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/ok.pdf",
            200,
            vec![("Content-Length".into(), body.len().to_string())],
            body.clone(),
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("ok.pdf");
        let limits = DownloadLimits::clamped(None);
        let n = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "ok.pdf",
                &dest,
                &limits,
                Some(body.len() as u64),
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap();
        assert_eq!(n as usize, body.len());
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn existing_dest_not_overwritten() {
        let body = b"%PDF-1.4 new".to_vec();
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/keep.pdf",
            200,
            vec![("Content-Length".into(), body.len().to_string())],
            body,
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("keep.pdf");
        std::fs::write(&dest, b"original").unwrap();
        let limits = DownloadLimits::clamped(None);
        let err = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "keep.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, IaError::AlreadyExists { .. }), "got {err:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"original");
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn forbidden_status_not_retried() {
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/denied.pdf",
            403,
            vec![("Content-Length".into(), "0".into())],
            Vec::new(),
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("denied.pdf");
        let limits = DownloadLimits::clamped(None);
        let err = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "myid",
                "denied.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, IaError::Http { status: 403, .. }),
            "got {err:?}"
        );
        assert_eq!(server.count("/download/myid/denied.pdf"), 1);
        assert!(!dest.exists());
        assert!(!partial_of(&dest).exists());
    }

    #[tokio::test]
    async fn retry_500_then_200() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let body = b"%PDF-1.4 recovered".to_vec();
        let body_len = body.len();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&hits);
        let body_for_thread = body.clone();
        std::thread::spawn(move || {
            let mut n = 0usize;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let head_end = loop {
                    let r = stream.read(&mut buf).unwrap_or(0);
                    if r == 0 {
                        break None;
                    }
                    raw.extend_from_slice(&buf[..r]);
                    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(at + 4);
                    }
                    if raw.len() > 64 * 1024 {
                        break None;
                    }
                };
                let Some(head_end) = head_end else { continue };
                let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
                let target = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                recorded.lock().unwrap().push(target);
                n += 1;
                if n == 1 {
                    let reply = "HTTP/1.1 500 X\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(reply.as_bytes());
                } else {
                    let reply = format!(
                        "HTTP/1.1 200 X\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                        body_for_thread.len()
                    );
                    let _ = stream.write_all(reply.as_bytes());
                    let _ = stream.write_all(&body_for_thread);
                    break;
                }
            }
        });
        let endpoints = Endpoints::new(Url::parse(&base).unwrap());
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("flap.pdf");
        let limits = DownloadLimits::clamped(None);
        let n = local_client()
            .download_file_with_retry(
                &endpoints,
                "myid",
                "flap.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap();
        assert_eq!(n as usize, body_len);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!partial_of(&dest).exists());
        assert_eq!(hits.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn invalid_remote_names_send_no_request() {
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/x.pdf",
            200,
            vec![],
            b"hello".to_vec(),
        )]);
        let endpoints = local_endpoints(&server);
        let client = local_client();
        let tmp = tempfile::tempdir().unwrap();
        let limits = DownloadLimits::clamped(None);
        for bad in [
            "../x.pdf",
            "/abs.pdf",
            "",
            "a\\b.pdf",
            "a\u{0}.pdf",
            "a/../b.pdf",
        ] {
            let dest = tmp.path().join("out.pdf");
            let _ = std::fs::remove_file(&dest);
            let _ = std::fs::remove_file(partial_of(&dest));
            let err = client
                .download_file_with_retry(
                    &endpoints,
                    "myid",
                    bad,
                    &dest,
                    &limits,
                    None,
                    &RetryPolicy::no_delay(),
                )
                .await
                .unwrap_err();
            assert!(
                matches!(err, IaError::InvalidQuery { .. }),
                "{bad:?} should be InvalidQuery, got {err:?}"
            );
        }
        assert!(
            server.targets().is_empty(),
            "no request must be sent, saw {:?}",
            server.targets()
        );
    }

    #[tokio::test]
    async fn invalid_identifier_sends_no_request() {
        let server = TestServer::start(vec![TestServer::raw(
            "/download/myid/a.pdf",
            200,
            vec![],
            b"hello".to_vec(),
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("a.pdf");
        let limits = DownloadLimits::clamped(None);
        let err = local_client()
            .download_file_with_retry(
                &local_endpoints(&server),
                "has space",
                "a.pdf",
                &dest,
                &limits,
                None,
                &RetryPolicy::no_delay(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, IaError::InvalidQuery { .. }), "got {err:?}");
        assert!(server.targets().is_empty());
    }
}
