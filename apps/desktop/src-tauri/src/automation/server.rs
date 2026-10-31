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
//! transport copies into the tool context.
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
use crate::automation::mcp::AgentServer;

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

/// What the connector reads to find the endpoint (D7).
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
}

impl Discovery {
    fn at(port: u16) -> Self {
        Self {
            protocol: PROTOCOL.to_string(),
            mcp_path: MCP_PATH.to_string(),
            port,
            pid: std::process::id(),
            app_version: app_version().to_string(),
            variant: variant_name().to_string(),
            profile: profile_name(),
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
    running: Mutex<Option<Running>>,
}

impl AgentEndpointState {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            running: Mutex::new(None),
        }
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
                *running = Some(start(&self.dir, access.clone())?);
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

/// Bind, advertise and spawn. The listener is bound before the discovery file
/// is written, so the file never names a port nobody listens on; if either
/// step fails nothing keeps running.
fn start(dir: &Path, access: Arc<AgentAccess>) -> Result<Running, String> {
    let listener = std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|e| format!("could not bind the assistant endpoint on {LOOPBACK}: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not read the assistant endpoint address: {e}"))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("could not make the assistant endpoint non-blocking: {e}"))?;

    let discovery_path = dir.join(DISCOVERY_FILE);
    let bytes = serde_json::to_vec_pretty(&Discovery::at(port))
        .map_err(|e| format!("could not serialize {}: {e}", discovery_path.display()))?;
    access::write_atomic(&discovery_path, &bytes)?;

    let cancel = CancellationToken::new();
    let service = GuardedEndpoint::new(access, port, cancel.clone());
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
    listener: std::net::TcpListener,
    service: GuardedEndpoint,
    cancel: CancellationToken,
) {
    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!(target: "automation", "the assistant endpoint listener is unusable: {error}");
            return;
        }
    };
    loop {
        let (stream, _peer) = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionAborted => continue,
                Err(error) => {
                    log::warn!(target: "automation", "the assistant endpoint stopped accepting connections: {error}");
                    return;
                }
            },
        };
        let service = service.clone();
        let connection_cancel = cancel.clone();
        tokio::spawn(serve_connection(
            TokioIo::new(stream),
            service,
            connection_cancel,
        ));
    }
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
}

impl GuardedEndpoint {
    fn new(access: Arc<AgentAccess>, port: u16, cancel: CancellationToken) -> Self {
        let hosts = vec![format!("{LOOPBACK}:{port}"), format!("localhost:{port}")];
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_sse_keep_alive(None)
            .with_allowed_hosts(hosts.iter().cloned())
            .enforce_origin_validation()
            .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES)
            .with_cancellation_token(cancel);
        Self {
            access,
            inner: StreamableHttpService::new(
                || Ok(AgentServer::new()),
                Arc::new(NeverSessionManager::default()),
                config,
            ),
            hosts,
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
    }

    async fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let access = Arc::new(AgentAccess::load(dir.path(), secrets.clone()));
        access.set_enabled(true).unwrap();
        let endpoint = Arc::new(AgentEndpointState::new(dir.path().to_path_buf()));
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
        assert_eq!(names, vec!["entropia_status"]);

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
        ] {
            assert!(keys.contains(&key), "the connector reads {key}: {keys:?}");
        }
        assert_eq!(keys.len(), 7, "no field beyond D7's: {keys:?}");
        let discovery: Discovery = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(discovery.protocol, PROTOCOL);
        assert_eq!(discovery.mcp_path, MCP_PATH);
        assert_eq!(discovery.port, harness.port);
        assert_eq!(discovery.pid, std::process::id());
        assert_eq!(discovery.app_version, app_version());
        assert_eq!(discovery.variant, variant_name());
        assert_eq!(discovery.profile, profile_name());

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
