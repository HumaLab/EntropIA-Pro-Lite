//! The loopback MCP endpoint: where an assistant connects, and who may.
//!
//! One listener on `127.0.0.1:0` (never a fixed port, never `0.0.0.0`), served
//! with hyper/tower in front of rmcp's Streamable HTTP transport. The transport
//! runs stateless (`legacy_session_mode: false`), answers JSON (no SSE), caps
//! request bodies at 1 MiB and validates the loopback `Host` itself; the guard
//! in [`GuardedEndpoint`] rejects everything else before rmcp sees it, in the
//! order [D6](../../../../../odd/tasks/agent-mcp-connector.md) fixes: path,
//! `Host`, `Origin`, bearer token. Only then does the authenticated
//! [`access::Principal`] reach the tool, through the request extensions the
//! transport copies into the tool context. A request that carries an instance
//! challenge and no credential is answered by the proof path of
//! [D12](../../../../../odd/tasks/agent-mcp-connector.md) instead: same refusal,
//! plus the HMAC that lets the connector trust this process before it sends a
//! token.
//!
//! While the endpoint runs, [`DISCOVERY_FILE`] (D7) tells the connector which
//! port to call; stopping removes it. The app owns that file and the endpoint
//! (S11): the instance guard already guarantees a single writer.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use http::header::{self, HeaderMap, HeaderValue};
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::{
    session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tower_service::Service;

use crate::automation::access::{self, AgentAccess};
use crate::automation::approvals::ApprovalStore;
use crate::automation::corpus_tools::CorpusEvents;
use crate::automation::mcp::AgentServer;
use crate::automation::proof;

/// Discovery file the connector reads while the endpoint is up.
pub const DISCOVERY_FILE: &str = "agent-endpoint.json";

/// The only path the endpoint answers on.
pub const MCP_PATH: &str = "/mcp";

/// Protocol version shared with the connector and the docs.
pub const PROTOCOL: &str = "entropia-agent/1";

/// Largest request body accepted, in bytes (D6).
pub const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// The only address the listener binds.
pub const LOOPBACK: &str = "127.0.0.1";

/// The app version the endpoint and the discovery file report. `Cargo.toml` is
/// the single source `pnpm version:bump` rewrites.
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Which build this is: `pro` with local inference, `lite` without.
pub fn variant_name() -> &'static str {
    if cfg!(feature = "local-ml") {
        "pro"
    } else {
        "lite"
    }
}

/// The dev profile running right now; always `None` in a release build.
pub fn profile_name() -> Option<String> {
    crate::dev_profile::active().map(str::to_string)
}

/// What the connector reads to find the endpoint (D7, D12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub protocol: String,
    pub mcp_path: String,
    pub port: u16,
    pub pid: u32,
    pub app_version: String,
    pub variant: String,
    pub profile: Option<String>,
    /// This start's proof key (D12). Readable only by the user: the file is
    /// `0600`, and the key is never sent over HTTP.
    pub instance_key: String,
}

impl Discovery {
    fn at(port: u16, instance_key: String) -> Self {
        Self {
            protocol: PROTOCOL.to_string(),
            mcp_path: MCP_PATH.to_string(),
            port,
            pid: std::process::id(),
            app_version: app_version().to_string(),
            variant: variant_name().to_string(),
            profile: profile_name(),
            instance_key,
        }
    }
}

/// One running listener: its port, the token that cancels it and the file that
/// advertises it.
struct Running {
    port: u16,
    cancel: CancellationToken,
    discovery_path: PathBuf,
}

impl Running {
    /// Stop serving and stop advertising. Idempotent: cancellation and a missing
    /// file are both no-ops the second time.
    fn stop(&self) {
        self.cancel.cancel();
        let _ = std::fs::remove_file(&self.discovery_path);
    }
}

/// Owns the endpoint's lifecycle for the whole app (D8).
///
/// [`sync`](Self::sync) is the one operation the enable toggle and startup
/// share: it starts the endpoint when access is enabled and nothing is
/// running, and stops it when access is disabled. A start failure is returned,
/// not panicked on: the caller logs it and the app keeps running.
pub struct AgentEndpointState {
    dir: PathBuf,
    /// The archive the tools read (D10): a tool call opens its own read-only
    /// connection from here, never the UI connection.
    db_path: PathBuf,
    /// The cache directory; the audit trail lives under its `logs/` (S9).
    cache_dir: PathBuf,
    running: Mutex<Option<Running>>,
    /// Paid work assistants asked for, waiting for the owner (D3). Outlives
    /// endpoint restarts within one app run; an app restart clears it.
    approvals: Arc<ApprovalStore>,
    /// Announces corpus writes so the app's views refresh.
    corpus_events: Arc<CorpusEvents>,
}

impl AgentEndpointState {
    pub fn new(
        dir: impl Into<PathBuf>,
        db_path: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            dir: dir.into(),
            db_path: db_path.into(),
            cache_dir: cache_dir.into(),
            running: Mutex::new(None),
            approvals: Arc::new(ApprovalStore::default()),
            corpus_events: Arc::new(CorpusEvents::default()),
        }
    }

    /// The announcer of corpus writes the endpoint makes.
    pub fn corpus_events(&self) -> Arc<CorpusEvents> {
        self.corpus_events.clone()
    }

    /// The audit trail the tools write, for what the owner answers in the app.
    pub fn audit_log(&self) -> crate::automation::audit::AuditLog {
        crate::automation::audit::AuditLog::new(self.cache_dir.join("logs"))
    }

    /// The cache directory: the app's own, so an approved delete removes the
    /// same thumbnails its commands would.
    pub fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    /// The approvals store the endpoint fills and the app answers.
    pub fn approvals(&self) -> Arc<ApprovalStore> {
        self.approvals.clone()
    }

    /// Start or stop so the endpoint reflects the access state. Async only so
    /// the server task it spawns is guaranteed a runtime.
    pub async fn sync(&self, access: &Arc<AgentAccess>) -> Result<(), String> {
        self.apply(access)
    }

    fn apply(&self, access: &Arc<AgentAccess>) -> Result<(), String> {
        let mut running = self
            .running
            .lock()
            .map_err(|_| "agent endpoint state lock is poisoned".to_string())?;
        match (running.is_some(), access.is_enabled()) {
            (true, true) | (false, false) => Ok(()),
            (false, true) => {
                *running = Some(start(
                    &self.dir,
                    access.clone(),
                    ToolContext {
                        db_path: self.db_path.clone(),
                        cache_dir: self.cache_dir.clone(),
                        approvals: self.approvals.clone(),
                        corpus_events: self.corpus_events.clone(),
                    },
                )?);
                Ok(())
            }
            (true, false) => {
                if let Some(current) = running.take() {
                    current.stop();
                }
                Ok(())
            }
        }
    }

    /// Stop serving and remove the discovery file. Safe to call at any time and
    /// however many times; the close path does.
    pub fn stop(&self) {
        if let Ok(mut running) = self.running.lock() {
            if let Some(current) = running.take() {
                current.stop();
            }
        }
    }

    /// The port the endpoint listens on, while it does.
    pub fn port(&self) -> Option<u16> {
        self.running
            .lock()
            .ok()?
            .as_ref()
            .map(|running| running.port)
    }
}

/// What every tool call is built with: the archive it reads and writes, the
/// cache that receives the audit trail, and the two channels to the app
/// (approvals the owner answers, corpus writes the views refresh on).
#[derive(Clone)]
struct ToolContext {
    db_path: PathBuf,
    cache_dir: PathBuf,
    approvals: Arc<ApprovalStore>,
    corpus_events: Arc<CorpusEvents>,
}

/// Bind, advertise and spawn. The listener is bound before the discovery file
/// is written, so the file never names a port nobody listens on; if either
/// step fails nothing keeps running.
fn start(dir: &Path, access: Arc<AgentAccess>, context: ToolContext) -> Result<Running, String> {
    let listener = std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|e| format!("could not bind the assistant endpoint on {LOOPBACK}: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not read the assistant endpoint address: {e}"))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("could not make the assistant endpoint non-blocking: {e}"))?;
    // Handed to the runtime here, before anything is advertised: a listener the
    // runtime refuses must fail the start, not die later inside `serve` while
    // the discovery file names its port.
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| format!("could not hand the assistant endpoint to the runtime: {e}"))?;

    let discovery_path = dir.join(DISCOVERY_FILE);
    let instance_key = proof::mint_instance_key()?;
    let bytes = serde_json::to_vec_pretty(&Discovery::at(port, instance_key.clone()))
        .map_err(|e| format!("could not serialize {}: {e}", discovery_path.display()))?;
    access::write_atomic(&discovery_path, &bytes)?;

    let cancel = CancellationToken::new();
    let service = GuardedEndpoint::new(access, port, instance_key, cancel.clone(), context);
    tokio::spawn(serve(listener, service, cancel.clone()));
    Ok(Running {
        port,
        cancel,
        discovery_path,
    })
}

/// Accept connections until cancelled. Each connection gets its own task so one
/// slow client cannot hold up the endpoint.
async fn serve(
    listener: tokio::net::TcpListener,
    service: GuardedEndpoint,
    cancel: CancellationToken,
) {
    let connections = cancel.clone();
    accept_loop(
        || listener.accept(),
        move |(stream, _peer)| {
            tokio::spawn(serve_connection(
                TokioIo::new(stream),
                service.clone(),
                connections.clone(),
            ));
        },
        cancel,
    )
    .await;
}

/// Keep accepting until cancelled. A failed accept never ends the endpoint:
/// the usual causes pass (a client that hung up, file descriptors running out),
/// and returning would leave the state reporting a running endpoint and the
/// discovery file advertising a port nobody serves. Failures back off (see
/// [`accept_backoff`]) and only the first of a run is logged.
async fn accept_loop<T, F, Fut>(
    mut accept: F,
    mut on_connection: impl FnMut(T),
    cancel: CancellationToken,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    let mut failures: u32 = 0;
    loop {
        let accepted = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = accept() => accepted,
        };
        match accepted {
            Ok(connection) => {
                failures = 0;
                on_connection(connection);
            }
            // The client gave up between the handshake and the accept.
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionAborted => {}
            Err(error) => {
                if failures == 0 {
                    log::warn!(target: "automation", "the assistant endpoint could not accept a connection, retrying: {error}");
                }
                failures = failures.saturating_add(1);
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(accept_backoff(failures)) => {}
                }
            }
        }
    }
}

/// The wait after the `failures`-th consecutive failed accept: 50 ms doubling up
/// to one second.
fn accept_backoff(failures: u32) -> std::time::Duration {
    let doublings = failures.saturating_sub(1).min(5);
    std::time::Duration::from_millis((50_u64 << doublings).min(1000))
}

async fn serve_connection<I>(io: I, service: GuardedEndpoint, cancel: CancellationToken)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let connection = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, hyper_util::service::TowerToHyperService::new(service));
    tokio::pin!(connection);
    tokio::select! {
        result = connection.as_mut() => {
            if let Err(error) = result {
                log::debug!(target: "automation", "assistant connection ended: {error}");
            }
        }
        _ = cancel.cancelled() => {
            connection.as_mut().graceful_shutdown();
            let _ = connection.await;
        }
    }
}

/// The endpoint as a tower service: the request guard, then rmcp.
#[derive(Clone)]
struct GuardedEndpoint {
    access: Arc<AgentAccess>,
    inner: StreamableHttpService<AgentServer, NeverSessionManager>,
    /// The exact `Host` values the endpoint answers to (D6).
    hosts: Vec<String>,
    /// This start's proof key (D12); never leaves the process except as an HMAC.
    instance_key: String,
}

impl GuardedEndpoint {
    fn new(
        access: Arc<AgentAccess>,
        port: u16,
        instance_key: String,
        cancel: CancellationToken,
        context: ToolContext,
    ) -> Self {
        let hosts = vec![format!("{LOOPBACK}:{port}"), format!("localhost:{port}")];
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_sse_keep_alive(None)
            .with_allowed_hosts(hosts.iter().cloned())
            .enforce_origin_validation()
            .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES)
            .with_cancellation_token(cancel);
        let tool_access = access.clone();
        Self {
            access,
            inner: StreamableHttpService::new(
                // One server per request: the tools read the archive from the
                // path it was built with and never touch the UI connection.
                move || {
                    Ok(AgentServer::new(
                        context.db_path.clone(),
                        context.cache_dir.clone(),
                        context.approvals.clone(),
                        context.corpus_events.clone(),
                        tool_access.clone(),
                    ))
                },
                Arc::new(NeverSessionManager::default()),
                config,
            ),
            hosts,
            instance_key,
        }
    }

    async fn handle(&self, mut request: Request<Incoming>) -> Response<BoxBody> {
        // D6's order is part of the contract: the path first (a wrong path is a
        // wrong endpoint, whatever it carries), then the two headers a rebinding
        // or browser request cannot fake, then the credential.
        if request.uri().path() != MCP_PATH {
            return reject(
                StatusCode::NOT_FOUND,
                "not_found",
                "Esta dirección no es el endpoint del asistente en esta EntropIA: usá /mcp.",
            );
        }
        if !self.accepts_host(&request) {
            return reject(
                StatusCode::FORBIDDEN,
                "forbidden_host",
                "La petición llegó dirigida a un nombre de host que no es el local: el endpoint solo atiende 127.0.0.1 y localhost.",
            );
        }
        if request.headers().contains_key(header::ORIGIN) {
            return reject(
                StatusCode::FORBIDDEN,
                "forbidden_origin",
                "Un navegador no puede usar el endpoint del asistente: conectalo con el conector de EntropIA.",
            );
        }
        // D12: a caller that read the discovery file proves it before spending a
        // credential. It is a refusal like any other (401), plus the proof; a
        // malformed challenge earns no proof at all. The Origin check above runs
        // first on purpose: a browser must not be able to obtain a proof.
        if let Some(challenge) = request.headers().get(proof::CHALLENGE_HEADER) {
            if !request.headers().contains_key(header::AUTHORIZATION) {
                return match challenge
                    .to_str()
                    .ok()
                    .filter(|challenge| proof::is_hex_64(challenge))
                {
                    Some(challenge) => challenge_response(&self.instance_key, challenge),
                    None => unauthorized(),
                };
            }
        }
        let Some(principal) =
            bearer_token(request.headers()).and_then(|token| self.access.authenticate(token))
        else {
            return unauthorized();
        };
        // No session caches this: the next request authenticates again, so a
        // revocation or a disabled endpoint applies to it (S1, S3).
        request.extensions_mut().insert(principal);

        let mut inner = self.inner.clone();
        match inner.call(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }

    /// Whether the `Host` header names this endpoint. Exact match, port included:
    /// `evil.example:<port>` is exactly what a DNS-rebinding page sends.
    fn accepts_host(&self, request: &Request<Incoming>) -> bool {
        request
            .headers()
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|host| {
                self.hosts
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(host))
            })
    }
}

/// The bearer token of an `Authorization` header, if it carries one.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

/// A refusal the connector can read: a stable code for it to branch on and one
/// sentence for the person reading the assistant's output.
fn reject(status: StatusCode, code: &str, message: &str) -> Response<BoxBody> {
    let body = serde_json::json!({ "error": code, "message": message });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    let mut response = Response::new(Full::new(Bytes::from(bytes)).boxed());
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn unauthorized() -> Response<BoxBody> {
    let mut response = reject(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "Falta la credencial del asistente o ya no sirve: volvé a emparejarlo en EntropIA.",
    );
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

/// The D12 answer: the same refusal as [`unauthorized`], carrying the proof
/// that lets a caller confirm it is this app and not whatever holds the port of
/// a stale discovery file.
fn challenge_response(instance_key: &str, challenge: &str) -> Response<BoxBody> {
    let mut response = unauthorized();
    let proof = proof::proof_for(instance_key, challenge);
    if let Ok(value) = HeaderValue::from_str(&proof) {
        response.headers_mut().insert(proof::PROOF_HEADER, value);
    }
    response
}

type BoxBody = http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>;

impl Service<Request<Incoming>> for GuardedEndpoint {
    type Response = Response<BoxBody>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Incoming>) -> Self::Future {
        let this = self.clone();
        Box::pin(async move { Ok(this.handle(request).await) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_backoff_grows_from_50_ms_and_caps_at_one_second() {
        let waits: Vec<u64> = (1..=8)
            .map(|failures| accept_backoff(failures).as_millis() as u64)
            .collect();
        assert_eq!(waits, vec![50, 100, 200, 400, 800, 1000, 1000, 1000]);
        assert_eq!(
            accept_backoff(u32::MAX),
            std::time::Duration::from_millis(1000)
        );
    }

    /// A failed accept (EMFILE, a reset) must not end the endpoint: before this,
    /// `serve` returned and left the state reporting a running endpoint and the
    /// discovery file advertising a port nobody served.
    #[tokio::test]
    async fn the_accept_loop_survives_failed_accepts_and_keeps_serving() {
        let outcomes = Arc::new(Mutex::new(std::collections::VecDeque::from(vec![
            Err(std::io::Error::other("too many open files")),
            Err(std::io::Error::other("too many open files")),
            Ok(7_u32),
            Err(std::io::Error::from(std::io::ErrorKind::ConnectionAborted)),
            Ok(8_u32),
        ])));
        let served = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();

        let next = outcomes.clone();
        let accept = move || {
            let next = next.lock().unwrap().pop_front();
            async move {
                match next {
                    Some(outcome) => outcome,
                    None => std::future::pending().await,
                }
            }
        };
        let seen = served.clone();
        let stop = cancel.clone();
        let on_connection = move |connection: u32| {
            let mut seen = seen.lock().unwrap();
            seen.push(connection);
            if seen.len() == 2 {
                stop.cancel();
            }
        };

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            accept_loop(accept, on_connection, cancel),
        )
        .await
        .expect("the loop returns once cancelled");
        assert_eq!(*served.lock().unwrap(), vec![7, 8]);
    }

    #[tokio::test]
    async fn cancelling_during_a_backoff_ends_the_accept_loop_at_once() {
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let accept = move || {
            stop.cancel();
            async { Err::<u32, _>(std::io::Error::other("refused")) }
        };
        let started = std::time::Instant::now();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            accept_loop(accept, |_: u32| {}, cancel),
        )
        .await
        .expect("cancellation ends the loop");
        assert!(started.elapsed() < std::time::Duration::from_millis(40));
    }
    use crate::automation::access::tests::MemorySecrets;
    use serde_json::{json, Value};

    struct Harness {
        dir: tempfile::TempDir,
        secrets: Arc<MemorySecrets>,
        access: Arc<AgentAccess>,
        endpoint: Arc<AgentEndpointState>,
        client: reqwest::Client,
        port: u16,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.endpoint.stop();
        }
    }

    impl Harness {
        fn url(&self, path: &str) -> String {
            format!("http://{LOOPBACK}:{}{path}", self.port)
        }

        /// Pair a client and return its id and its token, which only the secret
        /// store holds in production.
        fn pair(&self, name: &str) -> (String, String) {
            let client = self.access.create_client(name).unwrap();
            let token = self.secrets.token(&client.id).expect("stored token");
            (client.id, token)
        }

        fn mcp_request(&self, token: Option<&str>) -> reqwest::RequestBuilder {
            let mut request = self
                .client
                .post(self.url(MCP_PATH))
                .header("accept", "application/json, text/event-stream")
                .header("content-type", "application/json");
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            request
        }

        async fn rpc(&self, token: Option<&str>, body: Value) -> reqwest::Response {
            self.mcp_request(token).json(&body).send().await.unwrap()
        }

        /// The discovery file as the connector reads it.
        fn discovery(&self) -> Discovery {
            let bytes = std::fs::read(self.dir.path().join(DISCOVERY_FILE))
                .expect("the endpoint advertises itself while it runs");
            serde_json::from_slice(&bytes).expect("the discovery file is D7's shape")
        }
    }

    /// The request body cap (D6) is enforced before any tool runs: an oversized
    /// call from a paired client is refused with 413, not parsed.
    #[tokio::test]
    async fn a_body_over_the_cap_is_refused_before_any_tool_runs() {
        let h = harness().await;
        let (_id, token) = h.pair("Claude Code");
        let padding = "x".repeat(MAX_REQUEST_BODY_BYTES + 1);
        let response = h
            .mcp_request(Some(&token))
            .body(format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{"pad":"{padding}"}}}}"#
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// The app and the connector each declare the protocol string and the
    /// keyring service (the connector is a separate crate and cannot import
    /// them). A change on one side only would fail every connection at runtime;
    /// this pins both sides together.
    #[test]
    fn the_connector_declares_the_same_protocol_and_keyring_service() {
        let discovery =
            include_str!("../../../../../crates/entropia-agent-connector/src/discovery.rs");
        let keyring =
            include_str!("../../../../../crates/entropia-agent-connector/src/keyring_source.rs");
        assert!(
            discovery.contains(&format!("pub const PROTOCOL: &str = \"{PROTOCOL}\";")),
            "connector discovery.rs must declare PROTOCOL = {PROTOCOL:?}"
        );
        assert!(
            keyring.contains(&format!(
                "pub const KEYRING_SERVICE: &str = \"{}\";",
                access::KEYRING_SERVICE
            )),
            "connector keyring_source.rs must declare KEYRING_SERVICE = {:?}",
            access::KEYRING_SERVICE
        );
    }

    async fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let access = Arc::new(AgentAccess::load(dir.path(), secrets.clone()));
        access.set_enabled(true).unwrap();
        let db_path = crate::automation::test_archive::seeded_archive(dir.path());
        let endpoint = Arc::new(AgentEndpointState::new(
            dir.path().to_path_buf(),
            db_path,
            dir.path().join("cache"),
        ));
        endpoint.sync(&access).await.unwrap();
        let port = endpoint.port().expect("the endpoint is running");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        Harness {
            dir,
            secrets,
            access,
            endpoint,
            client,
            port,
        }
    }

    fn initialize_body() -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test-assistant", "version": "0.0.0" }
            }
        })
    }

    async fn error_body(response: reqwest::Response) -> Value {
        let status = response.status();
        response
            .json::<Value>()
            .await
            .unwrap_or_else(|e| panic!("{status} did not carry a JSON error body: {e}"))
    }

    #[tokio::test]
    async fn a_request_without_a_credential_is_refused_with_a_bearer_challenge() {
        let harness = harness().await;
        let response = harness.rpc(None, initialize_body()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer")
        );
        let body = error_body(response).await;
        assert_eq!(body["error"], "unauthorized");
        assert!(body["message"].as_str().is_some_and(|m| m.len() > 20));
    }

    #[tokio::test]
    async fn a_token_that_matches_no_client_is_refused() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Claude Code");
        let response = harness
            .rpc(Some(&format!("{token}00")), initialize_body())
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(error_body(response).await["error"], "unauthorized");
    }

    #[tokio::test]
    async fn disabling_access_refuses_the_next_request_with_the_same_token() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Codex");
        assert_eq!(
            harness.rpc(Some(&token), initialize_body()).await.status(),
            StatusCode::OK
        );
        harness.access.set_enabled(false).unwrap();
        let response = harness.rpc(Some(&token), initialize_body()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(error_body(response).await["error"], "unauthorized");
    }

    #[tokio::test]
    async fn a_revoked_client_is_refused_on_the_next_request() {
        let harness = harness().await;
        let (id, copied_token) = harness.pair("OpenCode");
        assert_eq!(
            harness
                .rpc(Some(&copied_token), initialize_body())
                .await
                .status(),
            StatusCode::OK
        );
        assert!(harness.access.revoke_client(&id).unwrap());
        let response = harness.rpc(Some(&copied_token), initialize_body()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(error_body(response).await["error"], "unauthorized");
    }

    #[tokio::test]
    async fn a_browser_origin_is_refused_even_with_a_valid_token() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Pi");
        let response = harness
            .mcp_request(Some(&token))
            .header("origin", "https://evil.example")
            .json(&initialize_body())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(error_body(response).await["error"], "forbidden_origin");
    }

    #[tokio::test]
    async fn a_host_that_is_not_loopback_is_refused() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Pi");
        let response = harness
            .mcp_request(Some(&token))
            .header("host", format!("evil.example:{}", harness.port))
            .json(&initialize_body())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(error_body(response).await["error"], "forbidden_host");
    }

    #[tokio::test]
    async fn another_path_is_not_found() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Pi");
        let response = harness
            .client
            .post(harness.url("/elsewhere"))
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .json(&initialize_body())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(error_body(response).await["error"], "not_found");
    }

    #[tokio::test]
    async fn a_paired_client_completes_the_handshake_lists_tools_and_reports_itself() {
        let harness = harness().await;
        let (id, token) = harness.pair("Claude Code");

        let initialize = harness.rpc(Some(&token), initialize_body()).await;
        assert_eq!(initialize.status(), StatusCode::OK);
        let initialize: Value = initialize.json().await.unwrap();
        assert_eq!(initialize["result"]["serverInfo"]["name"], "entropia");
        assert_eq!(initialize["result"]["serverInfo"]["version"], app_version());

        let list = harness
            .rpc(
                Some(&token),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            )
            .await;
        assert_eq!(list.status(), StatusCode::OK);
        let list: Value = list.json().await.unwrap();
        let tools = list["result"]["tools"].as_array().expect("tools array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "approvals_status",
                "collections_create",
                "collections_delete",
                "collections_list",
                "collections_update",
                "documents_delete",
                "documents_get_text",
                "documents_import",
                "documents_index",
                "documents_text_update",
                "entropia_status",
                "import_sources_list",
                "items_get",
                "items_list",
                "items_update",
                "jobs_get",
                "jobs_list",
                "processing_control",
                "processing_prepare",
                "processing_start",
                "search_items",
                "search_passages"
            ]
        );

        let call = harness
            .rpc(
                Some(&token),
                json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tools/call",
                    "params": { "name": "entropia_status", "arguments": {} }
                }),
            )
            .await;
        assert_eq!(call.status(), StatusCode::OK);
        let call: Value = call.json().await.unwrap();
        assert_eq!(call["result"]["isError"], false);
        let status = &call["result"]["structuredContent"];
        assert_eq!(status["protocol"], PROTOCOL);
        assert_eq!(status["appVersion"], app_version());
        assert_eq!(status["variant"], variant_name());
        assert_eq!(status["client"]["id"], id);
        assert_eq!(status["client"]["name"], "Claude Code");
        assert_eq!(status["client"]["scopes"], json!(["read"]));
    }

    #[tokio::test]
    async fn the_discovery_file_describes_the_running_endpoint_and_disappears_on_stop() {
        let harness = harness().await;
        let path = harness.dir.path().join(DISCOVERY_FILE);
        let bytes = std::fs::read(&path).expect("discovery file written on start");
        let raw: Value = serde_json::from_slice(&bytes).unwrap();
        let keys: Vec<&str> = raw
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in [
            "protocol",
            "mcpPath",
            "port",
            "pid",
            "appVersion",
            "variant",
            "profile",
            "instanceKey",
        ] {
            assert!(keys.contains(&key), "the connector reads {key}: {keys:?}");
        }
        assert_eq!(keys.len(), 8, "no field beyond D7's and D12's: {keys:?}");
        let discovery: Discovery = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(discovery.protocol, PROTOCOL);
        assert_eq!(discovery.mcp_path, MCP_PATH);
        assert_eq!(discovery.port, harness.port);
        assert_eq!(discovery.pid, std::process::id());
        assert_eq!(discovery.app_version, app_version());
        assert_eq!(discovery.variant, variant_name());
        assert_eq!(discovery.profile, profile_name());
        assert_eq!(
            discovery.instance_key.len(),
            2 * proof::INSTANCE_KEY_BYTES,
            "D12: 32 random bytes, hex-encoded"
        );
        assert!(
            discovery
                .instance_key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
            "the key is hex: {:?}",
            discovery.instance_key
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the user may read the port");
        }

        harness.endpoint.stop();
        assert_eq!(harness.endpoint.port(), None);
        assert!(!path.exists(), "a stopped endpoint advertises nothing");
    }

    #[tokio::test]
    async fn each_start_mints_a_new_instance_key() {
        let harness = harness().await;
        let first = harness.discovery().instance_key;
        harness.endpoint.stop();
        harness.endpoint.sync(&harness.access).await.unwrap();
        let second = harness.discovery().instance_key;
        assert_ne!(
            first, second,
            "D12: a restart must not reuse the key a stale file could name"
        );
    }

    #[tokio::test]
    async fn a_challenge_is_answered_with_the_hmac_of_the_key_in_the_discovery_file() {
        let harness = harness().await;
        let discovery = harness.discovery();
        let challenge = "ab".repeat(proof::INSTANCE_KEY_BYTES);
        let response = harness
            .client
            .post(harness.url(MCP_PATH))
            .header(proof::CHALLENGE_HEADER, &challenge)
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer"),
            "a challenge is refused like any unauthenticated request"
        );
        let proof_header = response
            .headers()
            .get(proof::PROOF_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = error_body(response).await;
        assert_eq!(body["error"], "unauthorized");
        assert_eq!(
            proof_header.as_deref(),
            Some(proof::proof_for(&discovery.instance_key, &challenge).as_str()),
            "the connector verifies the instance with this HMAC"
        );
    }

    #[tokio::test]
    async fn a_malformed_challenge_is_refused_without_a_proof() {
        let harness = harness().await;
        for challenge in ["zz".repeat(proof::INSTANCE_KEY_BYTES), "abcd".to_string()] {
            let response = harness
                .client
                .post(harness.url(MCP_PATH))
                .header(proof::CHALLENGE_HEADER, &challenge)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{challenge}");
            assert!(
                response.headers().get(proof::PROOF_HEADER).is_none(),
                "a malformed challenge must not earn a proof: {challenge}"
            );
            assert_eq!(error_body(response).await["error"], "unauthorized");
        }
    }

    #[tokio::test]
    async fn a_browser_cannot_obtain_a_proof() {
        let harness = harness().await;
        let challenge = "cd".repeat(proof::INSTANCE_KEY_BYTES);
        let response = harness
            .client
            .post(harness.url(MCP_PATH))
            .header(proof::CHALLENGE_HEADER, &challenge)
            .header("origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            response.headers().get(proof::PROOF_HEADER).is_none(),
            "the Origin refusal must apply to challenge requests"
        );
        assert_eq!(error_body(response).await["error"], "forbidden_origin");
    }

    #[tokio::test]
    async fn a_get_or_a_delete_on_the_endpoint_is_not_allowed() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Claude Code");
        for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
            let response = harness
                .client
                .request(method.clone(), harness.url(MCP_PATH))
                .header("accept", "application/json, text/event-stream")
                .header("authorization", format!("Bearer {token}"))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method}: only POST carries MCP messages"
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::ALLOW)
                    .and_then(|value| value.to_str().ok()),
                Some("POST")
            );
            let body = response.text().await.unwrap();
            assert!(
                serde_json::from_str::<Value>(&body).is_err(),
                "{method} returned MCP data: {body}"
            );
        }
    }

    #[tokio::test]
    async fn the_path_with_a_trailing_slash_is_not_the_endpoint() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Codex");
        let response = harness
            .client
            .post(harness.url("/mcp/"))
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .json(&initialize_body())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(error_body(response).await["error"], "not_found");
    }

    #[tokio::test]
    async fn disabling_access_stops_serving_and_enabling_it_again_restarts() {
        let harness = harness().await;
        let (_id, token) = harness.pair("Pi");
        let first_port = harness.endpoint.port().unwrap();

        harness.access.set_enabled(false).unwrap();
        harness.endpoint.sync(&harness.access).await.unwrap();
        assert_eq!(harness.endpoint.port(), None);
        assert!(!harness.dir.path().join(DISCOVERY_FILE).exists());
        let refused = harness.client.get(harness.url(MCP_PATH)).send().await;
        assert!(
            refused.is_err(),
            "nothing listens once the endpoint stopped"
        );

        harness.access.set_enabled(true).unwrap();
        harness.endpoint.sync(&harness.access).await.unwrap();
        let port = harness.endpoint.port().expect("restarted");
        assert_ne!(port, first_port, "each start takes its own ephemeral port");
        assert!(harness.dir.path().join(DISCOVERY_FILE).exists());
        let response = harness
            .client
            .post(format!("http://{LOOPBACK}:{port}{MCP_PATH}"))
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .json(&initialize_body())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_listener_answers_on_loopback_only() {
        let harness = harness().await;
        assert_eq!(
            harness.rpc(None, initialize_body()).await.status(),
            StatusCode::UNAUTHORIZED,
            "the guard answered, so the listener is up on loopback"
        );
        let Some(lan) = local_network_ipv4() else {
            return; // no network interface to try: binding 0.0.0.0 cannot be told apart
        };
        let attempt = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            harness
                .client
                .get(format!("http://{lan}:{}{MCP_PATH}", harness.port))
                .send(),
        )
        .await;
        assert!(
            matches!(attempt, Ok(Err(_))),
            "the endpoint must not answer on the machine's LAN address {lan}"
        );
    }

    /// The IPv4 address another machine would use to reach this one, if there is
    /// one. A UDP `connect` only asks the routing table: it sends no packet.
    fn local_network_ipv4() -> Option<std::net::IpAddr> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect(("203.0.113.1", 9)).ok()?;
        let ip = socket.local_addr().ok()?.ip();
        (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
    }
}
