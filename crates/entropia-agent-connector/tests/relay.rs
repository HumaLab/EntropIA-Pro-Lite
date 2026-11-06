//! Behavior of the stdio relay: what the agent writes on stdin, what EntropIA
//! answers, and what reaches the connector's stdout.
//!
//! Every test drives the real relay against a fake endpoint over loopback and a
//! pair of duplex streams standing in for stdin and stdout, so the assertions
//! are on the wire and on stdout, never on internals.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::{closed_port, FakeApp, Proof, Reply};
use entropia_agent_connector::proof;
use entropia_agent_connector::relay::{
    self, ConnectorSource, Diagnostics, RelayConfig, TokenError,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};

const CLIENT_ID: &str = "cl_test";
const TOKEN: &str = "eat_test-token";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A credential and endpoint source a test fully controls: no keyring, no data
/// directory outside the test's own temp dir.
struct TestSource {
    discovery: PathBuf,
    token: Option<String>,
}

impl ConnectorSource for TestSource {
    fn discovery_path(&self) -> PathBuf {
        self.discovery.clone()
    }

    fn token(&self, client_id: &str) -> Result<String, TokenError> {
        assert_eq!(client_id, CLIENT_ID, "the relay asked for another client");
        match &self.token {
            Some(token) => Ok(token.clone()),
            None => Err(TokenError::Missing),
        }
    }
}

/// Diagnostics recorded instead of written to stderr, so "stderr only" is
/// observable.
#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<String>>>);

impl Diagnostics for Recording {
    fn warn(&self, message: &str) {
        self.0.lock().unwrap().push(message.to_string());
    }
}

struct Harness {
    input: DuplexStream,
    output: BufReader<DuplexStream>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    warnings: Arc<Mutex<Vec<String>>>,
    lines: Vec<String>,
}

struct Outcome {
    lines: Vec<String>,
    warnings: Vec<String>,
}

impl Harness {
    fn start(discovery: PathBuf, token: Option<&str>) -> Self {
        let source = Arc::new(TestSource {
            discovery,
            token: token.map(str::to_string),
        });
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recording(warnings.clone());
        let config = RelayConfig::new(CLIENT_ID).with_diagnostics(Arc::new(recorder));
        let (input, relay_in) = tokio::io::duplex(1 << 20);
        let (relay_out, output) = tokio::io::duplex(1 << 20);
        let task = tokio::spawn(relay::run(relay_in, relay_out, config, source));
        Self {
            input,
            output: BufReader::new(output),
            task,
            warnings,
            lines: Vec::new(),
        }
    }

    async fn send(&mut self, line: &str) {
        self.input.write_all(line.as_bytes()).await.unwrap();
        self.input.write_all(b"\n").await.unwrap();
        self.input.flush().await.unwrap();
    }

    /// The next line the connector wrote, asserted to be one whole line.
    async fn next_line(&mut self) -> String {
        let mut line = String::new();
        let read = self.output.read_line(&mut line).await.unwrap();
        assert!(read > 0, "the connector wrote nothing");
        assert!(line.ends_with('\n'), "a line reached stdout unterminated");
        let line = line.trim_end_matches('\n').to_string();
        self.lines.push(line.clone());
        line
    }

    /// Close stdin, collect whatever is left, and wait for the relay to finish.
    async fn finish(mut self) -> Outcome {
        self.input.shutdown().await.unwrap();
        let mut rest = String::new();
        self.output.read_to_string(&mut rest).await.unwrap();
        assert!(
            rest.is_empty() || rest.ends_with('\n'),
            "stdout ended with an unterminated line: {rest:?}"
        );
        for line in rest.lines() {
            self.lines.push(line.to_string());
        }
        self.task.await.expect("the relay task finished").unwrap();
        let warnings = self.warnings.lock().unwrap().clone();
        Outcome {
            lines: self.lines,
            warnings,
        }
    }
}

/// Send every line at once, then close stdin and return what came back.
async fn relay_once(discovery: PathBuf, token: Option<&str>, input: &[String]) -> Outcome {
    let mut harness = Harness::start(discovery, token);
    for line in input {
        harness.send(line).await;
    }
    harness.finish().await
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn discovery_file(dir: &Path) -> PathBuf {
    dir.join("agent-endpoint.json")
}

fn write_discovery(dir: &Path, protocol: &str, port: u16) {
    write_discovery_with_key(dir, protocol, port, common::INSTANCE_KEY);
}

/// The same file with an instance key of the test's choosing (D12).
fn write_discovery_with_key(dir: &Path, protocol: &str, port: u16, instance_key: &str) {
    write_discovery_value(
        dir,
        json!({
            "protocol": protocol,
            "mcpPath": "/mcp",
            "port": port,
            "pid": 4242,
            "appVersion": "1.1.1",
            "variant": "lite",
            "profile": Value::Null,
            "instanceKey": instance_key,
        }),
    );
}

fn write_discovery_value(dir: &Path, value: Value) {
    std::fs::write(discovery_file(dir), value.to_string()).unwrap();
}

fn initialize(id: u64) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test-assistant", "version": "0.0.0" }
        }
    })
    .to_string()
}

fn tools_list(id: u64) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list" }).to_string()
}

fn initialized_notification() -> String {
    json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string()
}

fn initialize_reply(id: u64) -> Reply {
    Reply::json(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "serverInfo": { "name": "EntropIA", "version": "1.1.1" }
            }
        })
        .to_string(),
    )
}

fn parse(line: &str) -> Value {
    serde_json::from_str(line).unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"))
}

fn error_code(response: &Value) -> &str {
    response["error"]["data"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("no stable data.code: {response}"))
}

fn rpc_error_code(response: &Value) -> i64 {
    response["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("no JSON-RPC error code: {response}"))
}

/// Every error code the connector emits for a failed request is in the
/// implementation-defined server-error family.
fn assert_rpc_family(response: &Value) {
    let code = rpc_error_code(response);
    assert!(
        (-32099..=-32000).contains(&code),
        "code {code} is outside the -32000 family"
    );
}

// ---------------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_request_gets_exactly_one_json_line_and_a_bearer_token() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::json(
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "serverInfo": { "name": "EntropIA" } } })
                .to_string(),
        )
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[initialize(1)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "EntropIA");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);

    let requests = app.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/mcp");
    let expected = format!("Bearer {TOKEN}");
    assert_eq!(requests[0].header("authorization"), Some(expected.as_str()));
    assert_eq!(
        requests[0].header("accept"),
        Some("application/json, text/event-stream")
    );
    assert!(requests[0]
        .header("content-type")
        .is_some_and(|value| value.starts_with("application/json")));
    assert_eq!(
        requests[0].header("mcp-protocol-version"),
        None,
        "a request that is not initialize must wait for a negotiated version"
    );
    assert_eq!(requests[0].body, initialize(1));
}

#[tokio::test]
async fn an_sse_response_becomes_one_line_per_data_event() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::sse(concat!(
            ": keep-alive comment\n",
            "\n",
            "event: message\n",
            "id: 1\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n",
            "\n",
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\n",
            "data: \"params\":{\"level\":\"info\"}}\n",
            "\n",
            "event: message\n",
            "data: not json at all\n",
            "\n",
        ))
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[initialize(1)]).await;

    assert_eq!(outcome.lines.len(), 2, "{:?}", outcome.lines);
    let first = parse(&outcome.lines[0]);
    assert_eq!(first["id"], 1);
    assert_eq!(first["result"]["ok"], true);
    let second = parse(&outcome.lines[1]);
    assert_eq!(second["method"], "notifications/message");
    assert_eq!(second["params"]["level"], "info");
    // The event that was not JSON never reached stdout, and was reported instead.
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
}

/// MCP 2026-07-28 clients carry their protocol version in each request's
/// `_meta`, and the endpoint (rmcp) refuses such a request with HTTP 400 unless
/// the `MCP-Protocol-Version` header says the same. Claude Code probes with
/// `server/discover` before any `initialize`, so the header comes from the
/// message itself, and it wins over a version negotiated earlier.
#[tokio::test]
async fn a_request_carrying_its_protocol_version_sends_it_as_the_header() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| match request.rpc_method().as_str() {
        "initialize" => initialize_reply(1),
        _ => Reply::json(json!({ "jsonrpc": "2.0", "id": 3, "result": {} }).to_string()),
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);
    let discover = json!({
        "jsonrpc": "2.0", "id": "probe", "method": "server/discover",
        "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } }
    })
    .to_string();
    let modern_call = json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/list",
        "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } }
    })
    .to_string();

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    harness.send(&discover).await;
    harness.next_line().await;
    harness.send(&initialize(1)).await;
    harness.next_line().await;
    harness.send(&modern_call).await;
    harness.next_line().await;
    harness.finish().await;

    let requests = app.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].rpc_method(), "server/discover");
    assert_eq!(
        requests[0].header("mcp-protocol-version"),
        Some("2026-07-28")
    );
    assert_eq!(
        requests[1].header("mcp-protocol-version"),
        None,
        "initialize"
    );
    assert_eq!(
        requests[2].header("mcp-protocol-version"),
        Some("2026-07-28")
    );
}

/// MCP 2026-07-28 Streamable HTTP routing headers (SEP-2243): the endpoint
/// refuses a request whose `Mcp-Method` (and, for named methods, `Mcp-Name`)
/// does not repeat the body. Names that are not plain header tokens travel as
/// `=?base64?...?=`.
#[tokio::test]
async fn every_request_carries_its_routing_headers() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::json(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }).to_string())
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);
    let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "entropia_status", "arguments": {} } })
    .to_string();
    let read = json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/read",
        "params": { "uri": "entropia://colección/1" } })
    .to_string();
    let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string();

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    for message in [&call, &read, &list] {
        harness.send(message).await;
        harness.next_line().await;
    }
    harness.finish().await;

    let requests = app.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].header("mcp-method"), Some("tools/call"));
    assert_eq!(requests[0].header("mcp-name"), Some("entropia_status"));
    assert_eq!(requests[1].header("mcp-method"), Some("resources/read"));
    assert_eq!(
        requests[1].header("mcp-name"),
        Some("=?base64?ZW50cm9waWE6Ly9jb2xlY2Npw7NuLzE=?=")
    );
    assert_eq!(requests[2].header("mcp-method"), Some("tools/list"));
    assert_eq!(requests[2].header("mcp-name"), None);
}

#[tokio::test]
async fn the_negotiated_protocol_version_is_sent_after_initialize_but_not_on_it() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| match request.rpc_method().as_str() {
        "initialize" => initialize_reply(1),
        _ => {
            Reply::json(json!({ "jsonrpc": "2.0", "id": 2, "result": { "tools": [] } }).to_string())
        }
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    harness.send(&initialize(1)).await;
    let first = parse(&harness.next_line().await);
    assert_eq!(first["result"]["protocolVersion"], "2025-06-18");
    harness.send(&tools_list(2)).await;
    let second = parse(&harness.next_line().await);
    assert_eq!(second["id"], 2);
    let outcome = harness.finish().await;
    assert_eq!(outcome.lines.len(), 2, "{:?}", outcome.lines);

    let requests = app.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].rpc_method(), "initialize");
    assert_eq!(requests[0].header("mcp-protocol-version"), None);
    assert_eq!(requests[1].rpc_method(), "tools/list");
    assert_eq!(
        requests[1].header("mcp-protocol-version"),
        Some("2025-06-18")
    );
}

#[tokio::test]
async fn a_notification_answered_with_202_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::empty(202)).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(
        discovery_file(dir.path()),
        Some(TOKEN),
        &[initialized_notification()],
    )
    .await;

    assert!(outcome.lines.is_empty(), "{:?}", outcome.lines);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(app.requests().len(), 1);
}

// ---------------------------------------------------------------------------
// Actionable errors (S7)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_401_becomes_an_unauthorized_error_with_the_same_id() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::status(401)).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(7)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 7);
    assert_eq!(error_code(&response), "unauthorized");
    assert_rpc_family(&response);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty() && message.contains("EntropIA")),
        "the message must tell the person what to do: {response}"
    );
}

#[tokio::test]
async fn a_403_becomes_a_forbidden_error() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::status(403)).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(3)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 3);
    assert_eq!(error_code(&response), "forbidden");
    assert_rpc_family(&response);
}

#[tokio::test]
async fn no_discovery_file_means_the_app_is_absent() {
    let dir = tempfile::tempdir().unwrap();

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(9)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 9);
    assert_eq!(error_code(&response), "app_absent");
    assert_rpc_family(&response);
}

/// S1: the assistant shows only `message`, never `data.detail`, and keeps the
/// server's stderr; so every refusal the connector builds also leaves one line
/// on the diagnostic channel with the numeric code, `data.code` and the detail.
#[tokio::test]
async fn a_refused_request_is_reported_on_diagnostics_with_its_code_and_detail() {
    let dir = tempfile::tempdir().unwrap();

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(9)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 9);
    let detail = response["error"]["data"]["detail"]
        .as_str()
        .expect("the refusal carries its detail");
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
    let warning = &outcome.warnings[0];
    let prefix = format!("{} {}: ", rpc_error_code(&response), error_code(&response));
    assert!(warning.starts_with(&prefix), "{warning:?}");
    assert!(warning.contains(detail), "{warning:?} lacks {detail:?}");
}

/// An error the app itself sent is forwarded as is and never logged: it is not
/// the connector's refusal (S1).
#[tokio::test]
async fn an_error_forwarded_from_the_app_is_not_logged_again() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::json(
            json!({
                "jsonrpc": "2.0",
                "id": 40,
                "error": { "code": -32601, "message": "no such tool" }
            })
            .to_string(),
        )
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(40)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    assert_eq!(parse(&outcome.lines[0])["error"]["code"], -32601);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

#[tokio::test]
async fn a_discovery_port_nothing_listens_on_means_the_app_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::empty(202)).await;
    let unused = closed_port();
    assert_ne!(unused, app.port);
    write_discovery(dir.path(), "entropia-agent/1", unused);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(10)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 10);
    assert_eq!(error_code(&response), "app_absent");
    assert_rpc_family(&response);
}

#[tokio::test]
async fn another_protocol_version_is_refused_before_a_request_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::json("{}")).await;
    write_discovery(dir.path(), "entropia-agent/2", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(4)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 4);
    assert_eq!(error_code(&response), "protocol_incompatible");
    assert_rpc_family(&response);
    assert!(app.requests().is_empty());
}

#[tokio::test]
async fn a_client_without_a_stored_credential_is_told_to_pair_again() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::json("{}")).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), None, &[tools_list(5)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 5);
    assert_eq!(error_code(&response), "credential_missing");
    assert_rpc_family(&response);
    assert!(app.requests().is_empty());
}

/// Windows PowerShell 5.1 prefixes what it pipes to a native program with a
/// UTF-8 byte order mark once `$OutputEncoding` is UTF-8, a common setting for
/// accented text. The mark is not part of the message, so the relay drops it
/// instead of answering a parse error.
#[tokio::test]
async fn a_byte_order_mark_before_a_message_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| match request.rpc_method().as_str() {
        "initialize" => initialize_reply(1),
        _ => Reply::json(json!({ "jsonrpc": "2.0", "id": 6, "result": {} }).to_string()),
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(
        discovery_file(dir.path()),
        Some(TOKEN),
        &[format!("\u{feff}{}", tools_list(6))],
    )
    .await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 6, "{response}");
    assert_eq!(response["result"], json!({}));
    assert_eq!(app.requests().len(), 1);
}

#[tokio::test]
async fn a_malformed_line_is_a_parse_error_and_the_relay_keeps_going() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| match request.rpc_method().as_str() {
        "initialize" => initialize_reply(1),
        _ => Reply::json(json!({ "jsonrpc": "2.0", "id": 6, "result": {} }).to_string()),
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(
        discovery_file(dir.path()),
        Some(TOKEN),
        &["{\"jsonrpc\": \"2.0\", \"id\":".to_string(), tools_list(6)],
    )
    .await;

    assert_eq!(outcome.lines.len(), 2, "{:?}", outcome.lines);
    let responses: Vec<Value> = outcome.lines.iter().map(|line| parse(line)).collect();
    let parse_error = responses
        .iter()
        .find(|response| response["id"].is_null())
        .expect("a parse error with a null id");
    assert_eq!(rpc_error_code(parse_error), -32700);
    assert_eq!(error_code(parse_error), "parse_error");
    let answered = responses
        .iter()
        .find(|response| response["id"] == 6)
        .expect("the valid request after the malformed line was answered");
    assert_eq!(answered["result"], json!({}));
    assert_eq!(app.requests().len(), 1, "the malformed line became a POST");
}

#[tokio::test]
async fn a_failing_notification_goes_to_diagnostics_and_not_to_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::status(401)).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(
        discovery_file(dir.path()),
        Some(TOKEN),
        &[initialized_notification()],
    )
    .await;

    assert!(outcome.lines.is_empty(), "{:?}", outcome.lines);
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
    assert!(
        outcome.warnings[0].to_lowercase().contains("entropia"),
        "{:?}",
        outcome.warnings
    );
}

#[tokio::test]
async fn nothing_but_json_lines_ever_reaches_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| match request.rpc_method().as_str() {
        "notifications/initialized" => Reply::status(401),
        "tools/list" => Reply::json(
            json!({ "jsonrpc": "2.0", "id": 20, "result": { "tools": [] } }).to_string(),
        ),
        _ => Reply::empty(202),
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(
        discovery_file(dir.path()),
        Some(TOKEN),
        &[
            "not json".to_string(),
            initialized_notification(),
            tools_list(20),
            initialized_notification(),
        ],
    )
    .await;

    assert_eq!(outcome.lines.len(), 2, "{:?}", outcome.lines);
    for line in &outcome.lines {
        let value = parse(line);
        assert_eq!(value["jsonrpc"], "2.0", "{line}");
        assert!(value.get("id").is_some(), "{line}");
        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "one protocol message per line: {line:?}"
        );
    }
    // The parse error was answered with a refusal (S1) and each notification that
    // failed produced one diagnostic; the 202 did not. Order is not part of the
    // contract: responses overlap.
    assert_eq!(outcome.warnings.len(), 3, "{:?}", outcome.warnings);
    let (parse_errors, refusals): (Vec<&String>, Vec<&String>) = outcome
        .warnings
        .iter()
        .partition(|warning| warning.starts_with("-32700 parse_error: "));
    assert_eq!(parse_errors.len(), 1, "{:?}", outcome.warnings);
    assert_eq!(refusals.len(), 2, "{:?}", outcome.warnings);
    for warning in refusals {
        assert!(warning.starts_with("-32002 unauthorized: "), "{warning}");
    }
}

#[tokio::test]
async fn an_empty_2xx_for_a_request_becomes_a_jsonrpc_error() {
    for status in [200u16, 202] {
        let dir = tempfile::tempdir().unwrap();
        let app = FakeApp::start(move |_| Reply::empty(status)).await;
        write_discovery(dir.path(), "entropia-agent/1", app.port);

        let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(30)]).await;

        assert_eq!(outcome.lines.len(), 1, "{status}: {:?}", outcome.lines);
        let response = parse(&outcome.lines[0]);
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 30);
        assert_eq!(error_code(&response), "endpoint_error");
        assert_eq!(rpc_error_code(&response), -32000);
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("EntropIA")),
            "the message must be actionable: {response}"
        );
    }
}

#[tokio::test]
async fn a_non_json_2xx_for_a_request_becomes_a_jsonrpc_error() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::json("<html>not json</html>")).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(31)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 31);
    assert_eq!(error_code(&response), "endpoint_error");
    assert_eq!(rpc_error_code(&response), -32000);
}

#[tokio::test]
async fn a_notification_answered_with_an_unusable_2xx_stays_on_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::json("<html>not json</html>")).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(
        discovery_file(dir.path()),
        Some(TOKEN),
        &[initialized_notification()],
    )
    .await;

    assert!(outcome.lines.is_empty(), "{:?}", outcome.lines);
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
}

#[tokio::test]
async fn an_sse_reply_without_a_json_event_answers_a_request_with_an_error() {
    let bodies = [
        ("", "no events"),
        (": keep-alive\n\n", "only a comment"),
        ("data: not json\n\n", "only a non-JSON event"),
        ("event: ping\n\nid: 4\n\n", "no data lines"),
    ];
    for (body, label) in bodies {
        let dir = tempfile::tempdir().unwrap();
        let app = FakeApp::start(move |_| Reply::sse(body)).await;
        write_discovery(dir.path(), "entropia-agent/1", app.port);

        let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(32)]).await;

        assert_eq!(outcome.lines.len(), 1, "{label}: {:?}", outcome.lines);
        let response = parse(&outcome.lines[0]);
        assert_eq!(response["jsonrpc"], "2.0", "{label}");
        assert_eq!(response["id"], 32, "{label}");
        assert_eq!(error_code(&response), "endpoint_error", "{label}");
        assert_eq!(rpc_error_code(&response), -32000, "{label}");
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("EntropIA")),
            "{label}: the message must be actionable: {response}"
        );
    }
}

#[tokio::test]
async fn an_sse_reply_with_one_json_event_and_noise_is_not_also_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::sse(format!(
            "data: not json\n\ndata: {}\n\n",
            json!({ "jsonrpc": "2.0", "id": 33, "result": {} })
        ))
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(33)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 33);
    assert!(response.get("error").is_none(), "{response}");
}

#[tokio::test]
async fn a_notification_answered_with_an_sse_reply_without_json_writes_nothing() {
    for body in ["", ": keep-alive\n\n", "data: not json\n\n"] {
        let dir = tempfile::tempdir().unwrap();
        let app = FakeApp::start(move |_| Reply::sse(body)).await;
        write_discovery(dir.path(), "entropia-agent/1", app.port);

        let outcome = relay_once(
            discovery_file(dir.path()),
            Some(TOKEN),
            &[initialized_notification()],
        )
        .await;

        assert!(outcome.lines.is_empty(), "{body:?}: {:?}", outcome.lines);
    }
}

// ---------------------------------------------------------------------------
// Instance discovery follows the app
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_restart_on_a_new_port_is_followed_between_messages() {
    let dir = tempfile::tempdir().unwrap();
    let first_app = FakeApp::start(|_| {
        Reply::json(
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "endpoint": "first" } }).to_string(),
        )
    })
    .await;
    let second_app = FakeApp::start(|_| {
        Reply::json(
            json!({ "jsonrpc": "2.0", "id": 2, "result": { "endpoint": "second" } }).to_string(),
        )
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", first_app.port);

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    harness.send(&initialize(1)).await;
    let first = parse(&harness.next_line().await);
    assert_eq!(first["result"]["endpoint"], "first");

    // The app stopped and started again: same file, new port, new pid.
    write_discovery_value(
        dir.path(),
        json!({
            "protocol": "entropia-agent/1",
            "mcpPath": "/mcp",
            "port": second_app.port,
            "pid": 5151,
            "appVersion": "1.1.1",
            "variant": "lite",
            "profile": Value::Null,
            "instanceKey": second_app.instance_key(),
        }),
    );
    harness.send(&tools_list(2)).await;
    let second = parse(&harness.next_line().await);
    assert_eq!(second["result"]["endpoint"], "second");

    let outcome = harness.finish().await;
    assert_eq!(outcome.lines.len(), 2, "{:?}", outcome.lines);
    assert_eq!(first_app.requests().len(), 1, "the old port was used again");
    assert_eq!(second_app.requests().len(), 1);
}

// ---------------------------------------------------------------------------
// Instance proof before the token leaves (D12)
// ---------------------------------------------------------------------------

/// The token never reaches a process the connector could not verify.
fn assert_no_token_sent(app: &FakeApp) {
    for request in app.all_requests() {
        assert_eq!(
            request.header("authorization"),
            None,
            "the token left the connector for an unverified instance: {request:?}"
        );
    }
}

#[tokio::test]
async fn a_challenge_proves_the_instance_before_the_token_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::json(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }).to_string())
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(1)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    assert_eq!(parse(&outcome.lines[0])["id"], 1);

    let challenges = app.challenge_requests();
    assert_eq!(challenges.len(), 1, "one challenge before the first token");
    let challenge = challenges[0].header(proof::CHALLENGE_HEADER).unwrap();
    assert_eq!(challenge.len(), 64, "32 random bytes, hex: {challenge:?}");
    assert!(
        challenge.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "the challenge is hex: {challenge:?}"
    );
    assert_eq!(challenges[0].method, "POST");
    assert_eq!(challenges[0].path, "/mcp");
    assert_eq!(
        challenges[0].header("authorization"),
        None,
        "the challenge never carries the token"
    );

    let requests = app.requests();
    assert_eq!(requests.len(), 1);
    let expected = format!("Bearer {TOKEN}");
    assert_eq!(requests[0].header("authorization"), Some(expected.as_str()));
}

#[tokio::test]
async fn every_request_is_challenged_before_its_token_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| {
        let id: Value = serde_json::from_str::<Value>(&request.body).unwrap()["id"].clone();
        Reply::json(json!({ "jsonrpc": "2.0", "id": id, "result": {} }).to_string())
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    harness.send(&initialize(1)).await;
    assert_eq!(parse(&harness.next_line().await)["id"], 1);
    harness.send(&tools_list(2)).await;
    assert_eq!(parse(&harness.next_line().await)["id"], 2);
    let outcome = harness.finish().await;
    assert_eq!(outcome.lines.len(), 2, "{:?}", outcome.lines);

    assert_eq!(
        app.challenge_requests().len(),
        2,
        "no earlier verification is trusted"
    );
    assert_eq!(app.requests().len(), 2);
    // Each token-bearing request is directly preceded by its own challenge.
    let order: Vec<bool> = app
        .all_requests()
        .iter()
        .map(|request| request.challenge)
        .collect();
    assert_eq!(order, [true, false, true, false]);
}

#[tokio::test]
async fn a_process_that_takes_over_a_verified_port_never_sees_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| {
        let id: Value = serde_json::from_str::<Value>(&request.body).unwrap()["id"].clone();
        Reply::json(json!({ "jsonrpc": "2.0", "id": id, "result": {} }).to_string())
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    harness.send(&initialize(1)).await;
    assert_eq!(parse(&harness.next_line().await)["id"], 1);
    assert_eq!(app.requests().len(), 1, "the first exchange was verified");

    // The app died and left the discovery file; another process now answers on
    // the same port without holding the instance key.
    for (mode, id) in [(Proof::Wrong, 2), (Proof::Missing, 3)] {
        app.set_proof(mode);
        harness.send(&tools_list(id)).await;
        let response = parse(&harness.next_line().await);
        assert_eq!(response["id"], id);
        assert_eq!(error_code(&response), "app_absent", "{mode:?}");
        assert_rpc_family(&response);
    }
    harness.finish().await;

    assert_eq!(app.challenge_requests().len(), 3);
    assert_eq!(
        app.requests().len(),
        1,
        "the replacement received no MCP message"
    );
    let with_token = app
        .all_requests()
        .iter()
        .filter(|request| request.header("authorization").is_some())
        .count();
    assert_eq!(
        with_token, 1,
        "only the verified exchange carried the token"
    );
}

#[tokio::test]
async fn a_new_port_is_verified_again() {
    let dir = tempfile::tempdir().unwrap();
    let first = FakeApp::start(|_| {
        Reply::json(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }).to_string())
    })
    .await;
    let second = FakeApp::start(|_| {
        Reply::json(json!({ "jsonrpc": "2.0", "id": 2, "result": {} }).to_string())
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", first.port);

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    harness.send(&initialize(1)).await;
    assert_eq!(parse(&harness.next_line().await)["id"], 1);
    write_discovery(dir.path(), "entropia-agent/1", second.port);
    harness.send(&tools_list(2)).await;
    assert_eq!(parse(&harness.next_line().await)["id"], 2);
    harness.finish().await;

    assert_eq!(first.challenge_requests().len(), 1);
    assert_eq!(
        second.challenge_requests().len(),
        1,
        "a new port proves again"
    );
}

#[tokio::test]
async fn a_changed_instance_key_is_verified_again_and_a_mismatch_is_app_absent() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::json(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }).to_string())
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let mut harness = Harness::start(discovery_file(dir.path()), Some(TOKEN));
    harness.send(&initialize(1)).await;
    assert_eq!(parse(&harness.next_line().await)["id"], 1);

    // Same port, different key: a process answering there is not the one the
    // connector verified, so the token must not be sent to it either.
    let other_key = "ee".repeat(32);
    write_discovery_with_key(dir.path(), "entropia-agent/1", app.port, &other_key);
    harness.send(&tools_list(2)).await;
    let response = parse(&harness.next_line().await);
    assert_eq!(response["id"], 2);
    assert_eq!(error_code(&response), "app_absent");
    assert_rpc_family(&response);
    harness.finish().await;

    assert_eq!(
        app.challenge_requests().len(),
        2,
        "the new key proves again"
    );
    assert_eq!(
        app.requests().len(),
        1,
        "only the verified pair got a token"
    );
}

#[tokio::test]
async fn a_wrong_proof_means_the_app_is_absent_and_the_token_never_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start_with(Proof::Wrong, |_| Reply::json("{}")).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(40)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 40);
    assert_eq!(error_code(&response), "app_absent");
    assert_rpc_family(&response);
    assert!(
        response["error"]["data"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("verificar")),
        "the detail says the instance could not be verified: {response}"
    );

    assert_eq!(app.challenge_requests().len(), 1);
    assert!(app.requests().is_empty());
    assert_no_token_sent(&app);
}

#[tokio::test]
async fn a_missing_proof_means_the_app_is_absent_and_the_token_never_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start_with(Proof::Missing, |_| Reply::json("{}")).await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(41)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 41);
    assert_eq!(error_code(&response), "app_absent");
    assert_rpc_family(&response);
    assert_eq!(app.challenge_requests().len(), 1);
    assert!(app.requests().is_empty());
    assert_no_token_sent(&app);
}

#[tokio::test]
async fn a_discovery_without_an_instance_key_is_protocol_incompatible() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::json("{}")).await;
    write_discovery_value(
        dir.path(),
        json!({
            "protocol": "entropia-agent/1",
            "mcpPath": "/mcp",
            "port": app.port,
            "pid": 4242,
            "appVersion": "1.1.1",
            "variant": "lite",
            "profile": Value::Null,
        }),
    );

    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[tools_list(42)]).await;

    assert_eq!(outcome.lines.len(), 1, "{:?}", outcome.lines);
    let response = parse(&outcome.lines[0]);
    assert_eq!(response["id"], 42);
    assert_eq!(error_code(&response), "protocol_incompatible");
    assert_rpc_family(&response);
    assert!(app.all_requests().is_empty(), "nothing was sent at all");
}

// ---------------------------------------------------------------------------
// Concurrency and line integrity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overlapping_requests_each_get_one_whole_line() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|request| {
        let id: Value = serde_json::from_str::<Value>(&request.body).unwrap()["id"].clone();
        let reply = Reply::json(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "method": request.rpc_method() }
            })
            .to_string(),
        );
        // The first request is the slow one, so the second finishes first.
        if request.rpc_method() == "slow" {
            reply.after(std::time::Duration::from_millis(150))
        } else {
            reply
        }
    })
    .await;
    write_discovery(dir.path(), "entropia-agent/1", app.port);

    let slow = json!({ "jsonrpc": "2.0", "id": 1, "method": "slow" }).to_string();
    let fast = json!({ "jsonrpc": "2.0", "id": 2, "method": "fast" }).to_string();
    let outcome = relay_once(discovery_file(dir.path()), Some(TOKEN), &[slow, fast]).await;

    assert_eq!(outcome.lines.len(), 2, "{:?}", outcome.lines);
    let mut ids: Vec<i64> = outcome
        .lines
        .iter()
        .map(|line| parse(line)["id"].as_i64().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2]);
    for line in &outcome.lines {
        let value = parse(line);
        assert_eq!(value["jsonrpc"], "2.0");
    }
    assert_eq!(app.requests().len(), 2);
}

// ---------------------------------------------------------------------------
// SSE framing
// ---------------------------------------------------------------------------

#[test]
fn sse_frames_are_split_on_blank_lines_and_multiline_data_is_joined() {
    let body = concat!(
        ": comment\n",
        "id: 1\n",
        "data: {\"a\":1}\n",
        "\n",
        "data: first\n",
        "data: second\n",
        "\n",
        "data: trailing",
    );
    assert_eq!(
        relay::parse_sse(body),
        vec![
            "{\"a\":1}".to_string(),
            "first\nsecond".to_string(),
            "trailing".to_string()
        ]
    );
    assert!(relay::parse_sse(": only a comment\n\n").is_empty());
}
