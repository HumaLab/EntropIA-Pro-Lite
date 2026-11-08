//! The relay: newline-delimited JSON-RPC on stdin, one POST per message to the
//! running app, and one line per answer on stdout.
//!
//! Nothing else ever reaches stdout, because every line goes through a single
//! writer task: the app's JSON body, one line per SSE `data:` event, or a
//! JSON-RPC error explaining why there is no answer (S5, S7). Diagnostics take
//! the [`Diagnostics`] channel, which is stderr in production.
//!
//! The discovery file is read again for every message, so an app restart on a
//! new port is followed without restarting the agent; the credential is read
//! from the keyring for every message too, so revocation applies to the next
//! request and nothing is cached in the process. The D12 instance proof is not
//! cached either: every request that carries `Authorization` is preceded by its
//! own challenge, so an earlier verification never vouches for a later request.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::discovery::{self, Discovery};
use crate::error::ConnectorError;
use crate::proof;

#[cfg(windows)]
#[path = "pipe.rs"]
mod pipe;

/// Keep status handling and reply classification identical for both transports.
enum Response {
    Http(reqwest::Response),
    #[cfg(windows)]
    Pipe(hyper::Response<hyper::body::Incoming>),
}

impl Response {
    fn status(&self) -> reqwest::StatusCode {
        match self {
            Self::Http(response) => response.status(),
            #[cfg(windows)]
            Self::Pipe(response) => response.status(),
        }
    }

    fn headers(&self) -> &reqwest::header::HeaderMap {
        match self {
            Self::Http(response) => response.headers(),
            #[cfg(windows)]
            Self::Pipe(response) => response.headers(),
        }
    }

    async fn chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::Http(response) => response
                .chunk()
                .await
                .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
                .map_err(|error| error.to_string()),
            #[cfg(windows)]
            Self::Pipe(response) => pipe::chunk(response.body_mut())
                .await
                .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
                .map_err(|error| error.to_string()),
        }
    }
}

#[derive(Default)]
struct Transport {
    #[cfg(windows)]
    pipe: Option<pipe::Pipe>,
}

impl Transport {
    async fn select(discovery: &Discovery) -> Result<Self, ConnectorError> {
        #[cfg(windows)]
        if let Some(name) = &discovery.pipe {
            return Ok(Self {
                pipe: pipe::Pipe::select(name).await.map_err(|error| {
                    ConnectorError::app_absent(format!(
                        "no pude conectar con {}: {error}",
                        discovery.endpoint()
                    ))
                })?,
            });
        }
        let _ = discovery;
        Ok(Self::default())
    }

    async fn send(
        &mut self,
        request: reqwest::RequestBuilder,
        url: &str,
    ) -> Result<Response, ConnectorError> {
        #[cfg(windows)]
        if let Some(pipe) = &mut self.pipe {
            let request = request.build().map_err(|error| {
                ConnectorError::app_absent(format!("no pude conectar con {url}: {error}"))
            })?;
            return pipe.send(request).await.map(Response::Pipe);
        }
        request.send().await.map(Response::Http).map_err(|error| {
            ConnectorError::app_absent(format!("no pude conectar con {url}: {error}"))
        })
    }
}

/// Where the relay gets its credential and its endpoint from. Production reads
/// the OS keyring and the app's data directory; tests implement it directly, so
/// no test touches either.
pub trait ConnectorSource: Send + Sync {
    /// The discovery file to read for the next message.
    fn discovery_path(&self) -> PathBuf;
    /// The client's token, or why there is none.
    fn token(&self, client_id: &str) -> Result<String, TokenError>;
}

/// Why a token could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// No token anywhere: the store has no entry for this client (it was never
    /// paired, or it was revoked) and no fallback file exists.
    Missing,
    /// The fallback token file is readable by group or others, so it cannot be
    /// trusted as a credential (D14).
    Insecure(String),
    /// The credential store itself failed in a way that is not "unavailable".
    Failed(String),
    /// The credential store cannot be reached from this process and there is
    /// no fallback file, so whether the client is paired is unknown.
    StoreUnavailable(String),
}

/// The MCP Streamable HTTP routing headers (SEP-2243) of one message:
/// `Mcp-Method` always, and `Mcp-Name` for the methods that name a target. The
/// endpoint refuses a 2026-07-28 request whose headers do not repeat its body;
/// older protocol versions ignore them.
pub fn routing_headers(body: &str) -> Vec<(&'static str, String)> {
    let Ok(message) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Vec::new();
    };
    let mut headers = Vec::new();
    if is_header_token(method) {
        headers.push(("Mcp-Method", method.to_string()));
    }
    let key = match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => Some("uri"),
        "tasks/get" | "tasks/update" | "tasks/cancel" => Some("taskId"),
        _ => None,
    };
    let name = key.and_then(|key| message.get("params")?.get(key)?.as_str());
    if let Some(name) = name {
        headers.push(("Mcp-Name", encode_header_value(name)));
    }
    headers
}

/// A value as it may travel in an `Mcp-*` header: itself when it is visible
/// ASCII without edge whitespace, else wrapped as `=?base64?...?=` (the same
/// rule as the endpoint's rmcp).
fn encode_header_value(value: &str) -> String {
    use base64::Engine as _;
    let edge_space = value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']);
    let invisible = value
        .chars()
        .any(|c| (c as u32) < 0x20 || (c as u32) > 0x7E);
    let looks_encoded = value.starts_with("=?base64?") && value.ends_with("?=");
    if !value.is_empty() && (edge_space || invisible || looks_encoded) {
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value)
        )
    } else {
        value.to_string()
    }
}

/// Whether a method name can be a header value as is (it always is for MCP's
/// own methods; a malformed one is simply not promoted to a header).
fn is_header_token(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_graphic())
}

/// The protocol version an MCP 2026-07-28 request carries in
/// `params._meta["io.modelcontextprotocol/protocolVersion"]`, if any.
fn request_protocol_version(body: &str) -> Option<String> {
    let message: Value = serde_json::from_str(body).ok()?;
    message
        .get("params")?
        .get("_meta")?
        .get("io.modelcontextprotocol/protocolVersion")?
        .as_str()
        .filter(|version| !version.is_empty())
        .map(str::to_string)
}

/// Where diagnostics go. Never stdout: that channel belongs to the protocol.
pub trait Diagnostics: Send + Sync {
    fn warn(&self, message: &str);
}

/// The production sink: one line per diagnostic on stderr.
pub struct StderrDiagnostics;

impl Diagnostics for StderrDiagnostics {
    fn warn(&self, message: &str) {
        eprintln!("entropia-agent-connector: {message}");
    }
}

/// Everything the relay needs that is not the streams themselves.
pub struct RelayConfig {
    /// The client id paired in EntropIA.
    pub client_id: String,
    /// Where diagnostics go.
    pub diagnostics: Arc<dyn Diagnostics>,
}

impl RelayConfig {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            diagnostics: Arc::new(StderrDiagnostics),
        }
    }

    pub fn with_diagnostics(mut self, diagnostics: Arc<dyn Diagnostics>) -> Self {
        self.diagnostics = diagnostics;
        self
    }
}

/// What the endpoint answered to one message.
enum Reply {
    /// 202 (or an empty 2xx body): nothing goes to stdout for a notification.
    Empty,
    /// One JSON document.
    Json(String),
    /// A `text/event-stream` body, one or more `data:` events.
    Sse(String),
}

struct Connector {
    client_id: String,
    source: Arc<dyn ConnectorSource>,
    http: reqwest::Client,
    /// The version `initialize` negotiated, sent on every later request.
    negotiated: Mutex<Option<String>>,
    diagnostics: Arc<dyn Diagnostics>,
    /// The one channel every stdout line goes through.
    lines: mpsc::Sender<String>,
}

/// Relay stdin to the app until stdin closes, then wait for every request that
/// was already in flight.
///
/// Responses may overlap (a slow tool call does not block a later message) but
/// each line is written whole by [`write_lines`], so two answers never
/// interleave.
pub async fn run<R, W>(
    reader: R,
    writer: W,
    config: RelayConfig,
    source: Arc<dyn ConnectorSource>,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|error| {
            std::io::Error::other(format!("no pude preparar el cliente HTTP: {error}"))
        })?;
    let (lines, pending) = mpsc::channel::<String>(64);
    let writer_task = tokio::spawn(write_lines(writer, pending));
    let connector = Arc::new(Connector {
        client_id: config.client_id,
        source,
        http,
        negotiated: Mutex::new(None),
        diagnostics: config.diagnostics,
        lines,
    });

    let mut reader = BufReader::new(reader);
    let mut input = String::new();
    let mut in_flight = tokio::task::JoinSet::new();
    loop {
        input.clear();
        if reader.read_line(&mut input).await? == 0 {
            break;
        }
        // A byte order mark is encoding noise, not message: Windows PowerShell
        // 5.1 adds one to what it pipes once `$OutputEncoding` is UTF-8.
        let message = input
            .trim_end_matches(['\n', '\r'])
            .trim_start_matches('\u{feff}')
            .to_string();
        if message.trim().is_empty() {
            continue;
        }
        let connector = Arc::clone(&connector);
        in_flight.spawn(async move { connector.handle(&message).await });
        // Keep the set bounded: a long session must not accumulate tasks.
        if in_flight.len() > 64 {
            let _ = in_flight.join_next().await;
        }
    }
    while in_flight.join_next().await.is_some() {}
    // Every task held a clone of the connector, which holds the channel
    // sender; the writer ends once the last one is gone.
    drop(connector);
    writer_task
        .await
        .map_err(|error| std::io::Error::other(format!("el escritor terminó mal: {error}")))?;
    Ok(())
}

/// The one writer: a whole line at a time, flushed before the next.
async fn write_lines<W>(mut writer: W, mut pending: mpsc::Receiver<String>)
where
    W: AsyncWrite + Unpin,
{
    let mut broken = false;
    while let Some(line) = pending.recv().await {
        if broken {
            continue;
        }
        let written = async {
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await
        }
        .await;
        if written.is_err() {
            // The agent stopped reading; keep draining so the senders finish
            // instead of blocking forever.
            broken = true;
        }
    }
    let _ = writer.flush().await;
}

impl Connector {
    async fn handle(&self, raw: &str) {
        let message: Value = match serde_json::from_str(raw) {
            Ok(message) => message,
            Err(error) => {
                self.emit(Value::Null, &ConnectorError::parse_error(error))
                    .await;
                return;
            }
        };
        let id = message.get("id").cloned();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let outcome = self.forward(&method, raw).await;

        // A notification never gets a response: its failure is a diagnostic.
        let Some(id) = id else {
            match outcome {
                Err(error) => self.diagnostics.warn(&error.describe()),
                Ok(Reply::Json(body)) => {
                    if let Err(error) = Self::check_body(&body) {
                        self.diagnostics
                            .warn(&ConnectorError::invalid_reply(error).describe());
                    }
                }
                Ok(Reply::Empty) | Ok(Reply::Sse(_)) => {}
            }
            return;
        };

        match outcome {
            Ok(Reply::Empty) => self.emit(id, &ConnectorError::empty_reply()).await,
            Ok(Reply::Json(body)) => {
                self.remember_version(&method, &body);
                if let Err(error) = self.send_body(&body).await {
                    self.emit(id, &ConnectorError::invalid_reply(error)).await;
                }
            }
            Ok(Reply::Sse(body)) => {
                let mut forwarded = 0usize;
                for event in parse_sse(&body) {
                    match serde_json::from_str::<Value>(&event) {
                        Ok(value) => {
                            self.remember_version(&method, &event);
                            let _ = self.lines.send(value.to_string()).await;
                            forwarded += 1;
                        }
                        Err(error) => self.diagnostics.warn(&format!(
                            "EntropIA envió un evento SSE que no es JSON ({error})"
                        )),
                    }
                }
                // A stream with no JSON event leaves the agent waiting for this
                // id for good, so the silence becomes an error like W2's.
                if forwarded == 0 {
                    self.emit(id, &ConnectorError::empty_stream()).await;
                }
            }
            Err(error) => self.emit(id, &error).await,
        }
    }

    /// Read the endpoint, prove the instance, read the credential, and POST one
    /// message. The order matters: no token leaves the connector towards a
    /// process that has not just proved it holds this discovery file (D12).
    async fn forward(&self, method: &str, body: &str) -> Result<Reply, ConnectorError> {
        let discovery = self.discovery().await?;
        let mut transport = Transport::select(&discovery).await?;
        self.verify_instance(&discovery, &mut transport).await?;
        let token = self
            .source
            .token(&self.client_id)
            .map_err(ConnectorError::credential)?;
        let url = discovery.endpoint();

        let mut request = self
            .http
            .post(&url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("authorization", format!("Bearer {token}"))
            .body(body.to_string());
        // An MCP 2026-07-28 request names its own version in `_meta`, and the
        // endpoint refuses it unless the header agrees, so that version wins.
        // Otherwise the header describes the negotiated session, which cannot
        // be on the request that negotiates it.
        let version = request_protocol_version(body).or_else(|| {
            if method == "initialize" {
                return None;
            }
            self.negotiated
                .lock()
                .ok()
                .and_then(|negotiated| negotiated.clone())
        });
        if let Some(version) = version {
            request = request.header("MCP-Protocol-Version", version);
        }
        for (name, value) in routing_headers(body) {
            request = request.header(name, value);
        }

        let response = transport.send(request, &url).await?;
        let status = response.status().as_u16();
        match status {
            401 => Err(ConnectorError::unauthorized()),
            403 => Err(ConnectorError::forbidden()),
            200..=299 => self.read_reply(response, &url).await,
            other => Err(ConnectorError::unexpected_status(other)),
        }
    }

    async fn discovery(&self) -> Result<Discovery, ConnectorError> {
        let path = self.source.discovery_path();
        let bytes = tokio::fs::read(&path).await.map_err(|error| {
            ConnectorError::app_absent(format!("no pude leer {}: {error}", path.display()))
        })?;
        discovery::parse(&bytes, &path)
    }

    /// Prove, before every token, that the process holding the advertised port
    /// read the same discovery file (D12). Nothing is remembered between calls:
    /// a port verified a moment ago may belong to another process now.
    ///
    /// The challenge is a POST to `/mcp` with no `Authorization`: the guard
    /// answers it with the usual 401 plus the HMAC of the challenge under the
    /// instance key. A missing or wrong proof is `app_absent` and the token is
    /// never sent.
    async fn verify_instance(
        &self,
        discovery: &Discovery,
        transport: &mut Transport,
    ) -> Result<(), ConnectorError> {
        let url = discovery.endpoint();
        let challenge = proof::challenge().map_err(|error| {
            ConnectorError::instance_unverified(format!("no pude generar el desafío: {error}"))
        })?;
        let response = transport
            .send(
                self.http
                    .post(&url)
                    .header(proof::CHALLENGE_HEADER, &challenge),
                &url,
            )
            .await?;
        let status = response.status().as_u16();
        let answer = response
            .headers()
            .get(proof::PROOF_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string());
        let expected = proof::proof_for(&discovery.instance_key, &challenge);
        let proven = status == 401
            && answer.is_some_and(|answer| {
                proof::constant_time_eq(answer.as_bytes(), expected.as_bytes())
            });
        if !proven {
            return Err(ConnectorError::instance_unverified(format!(
                "no pude verificar que {url} sea la instancia anunciada por {} (HTTP {status})",
                self.source.discovery_path().display()
            )));
        }
        Ok(())
    }

    async fn read_reply(&self, mut response: Response, url: &str) -> Result<Reply, ConnectorError> {
        let streamed = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("text/event-stream")
            });

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            ConnectorError::app_absent(format!("se cortó la respuesta de {url}: {error}"))
        })? {
            body.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&body).to_string();

        if streamed {
            Ok(Reply::Sse(text))
        } else if text.trim().is_empty() {
            Ok(Reply::Empty)
        } else {
            Ok(Reply::Json(text))
        }
    }

    /// Write one JSON document as exactly one line, whatever its formatting.
    /// The parse error comes back so a request can be answered instead of
    /// leaving the agent waiting (W2).
    async fn send_body(&self, body: &str) -> Result<(), serde_json::Error> {
        match serde_json::from_str::<Value>(body) {
            Ok(value) => {
                let _ = self.lines.send(value.to_string()).await;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Whether a body the endpoint sent is JSON at all, without writing it: a
    /// notification's body is never forwarded, but an unusable one is still a
    /// diagnostic.
    fn check_body(body: &str) -> Result<(), serde_json::Error> {
        serde_json::from_str::<Value>(body).map(|_| ())
    }

    /// A JSON-RPC error response for a request that could not be answered.
    ///
    /// The same refusal also leaves one line on the diagnostic channel (S1):
    /// assistants show only `error.message`, never `data.detail`, and keep the
    /// server's stderr, so the detail the person needs is only visible there.
    async fn emit(&self, id: Value, error: &ConnectorError) {
        self.diagnostics.warn(&error.describe());
        let mut data = serde_json::Map::new();
        data.insert("code".to_string(), json!(error.code));
        if let Some(detail) = &error.detail {
            data.insert("detail".to_string(), json!(detail));
        }
        let value = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": error.rpc_code,
                "message": error.message,
                "data": Value::Object(data),
            }
        });
        let _ = self.lines.send(value.to_string()).await;
    }

    /// Keep the version `initialize` negotiated, to send on later requests.
    fn remember_version(&self, method: &str, body: &str) {
        if method != "initialize" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(body) else {
            return;
        };
        let Some(version) = value["result"]["protocolVersion"].as_str() else {
            return;
        };
        if let Ok(mut negotiated) = self.negotiated.lock() {
            *negotiated = Some(version.to_string());
        }
    }
}

/// The `data` payloads of an SSE body, in order.
///
/// A blank line ends an event, `:` starts a comment, and several `data:` lines
/// in one event are joined with a newline, exactly as the event stream format
/// says.
pub fn parse_sse(body: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut data: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.is_empty() {
            if !data.is_empty() {
                events.push(data.join("\n"));
                data.clear();
            }
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        if field != "data" {
            continue;
        }
        let value = value.strip_prefix(' ').unwrap_or(value);
        data.push(value.to_string());
    }
    if !data.is_empty() {
        events.push(data.join("\n"));
    }
    events
}
