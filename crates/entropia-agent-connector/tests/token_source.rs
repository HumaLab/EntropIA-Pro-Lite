//! Where the relay gets its token in production: the OS credential store first
//! and, only when this machine has none, the owner-only file the app wrote
//! (D14).
//!
//! The store step is injected, so no test touches a real keyring; the fallback
//! itself is the real code, reading a real file in a temp directory. Two cases
//! run the whole relay against the fake endpoint, because "the token is used"
//! and "no request leaves the connector" are only observable on the wire.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{FakeApp, Reply};
use entropia_agent_connector::cli;
use entropia_agent_connector::keyring_source::{
    token_path, CredentialStore, KeyringOutcome, KeyringSource, TOKENS_DIR,
};
use entropia_agent_connector::relay::{
    self, ConnectorSource, Diagnostics, RelayConfig, TokenError,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const CLIENT_ID: &str = "cl_fallback";

/// A credential store a test scripts, so the order between the keyring and the
/// file is observable without a real one.
struct ScriptedStore(KeyringOutcome);

impl CredentialStore for ScriptedStore {
    fn read(&self, client_id: &str) -> KeyringOutcome {
        assert_eq!(client_id, CLIENT_ID, "the relay asked for another client");
        self.0.clone()
    }
}

/// Diagnostics a test does not look at: a failed notification must not print
/// into the test output.
struct Silent;

impl Diagnostics for Silent {
    fn warn(&self, _message: &str) {}
}

fn discovery_file(dir: &Path) -> PathBuf {
    dir.join("agent-endpoint.json")
}

fn write_discovery(dir: &Path, port: u16) {
    std::fs::write(
        discovery_file(dir),
        json!({
            "protocol": "entropia-agent/1",
            "mcpPath": "/mcp",
            "port": port,
            "pid": 4242,
            "appVersion": "1.1.1",
            "variant": "lite",
            "profile": Value::Null,
            "instanceKey": common::INSTANCE_KEY,
        })
        .to_string(),
    )
    .unwrap();
}

/// The fallback token file, with the Unix mode the test wants. The app writes
/// it `0600`; anything with group or other bits is what the connector refuses.
fn write_token_file(data_dir: &Path, token: &str, mode: u32) {
    let path = token_path(data_dir, CLIENT_ID);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, token).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = mode;
}

fn source(data_dir: &Path, outcome: KeyringOutcome) -> KeyringSource {
    KeyringSource::with_store(
        discovery_file(data_dir),
        data_dir,
        Box::new(ScriptedStore(outcome)),
    )
}

/// Send one `tools/list` through the whole relay and collect its stdout lines.
async fn relay_once(source: KeyringSource) -> Vec<String> {
    let source: Arc<dyn ConnectorSource> = Arc::new(source);
    let config = RelayConfig::new(CLIENT_ID).with_diagnostics(Arc::new(Silent));
    let (mut input, relay_in) = tokio::io::duplex(1 << 16);
    let (relay_out, output) = tokio::io::duplex(1 << 16);
    let task = tokio::spawn(relay::run(relay_in, relay_out, config, source));

    input
        .write_all(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    input.write_all(b"\n").await.unwrap();
    input.flush().await.unwrap();
    input.shutdown().await.unwrap();

    let mut reader = BufReader::new(output);
    let mut lines = Vec::new();
    let mut line = String::new();
    while reader.read_line(&mut line).await.unwrap() > 0 {
        lines.push(line.trim_end_matches('\n').to_string());
        line.clear();
    }
    task.await.unwrap().expect("the relay finished cleanly");
    lines
}

fn parse(line: &str) -> Value {
    serde_json::from_str(line)
        .unwrap_or_else(|error| panic!("stdout line is not JSON ({error}): {line}"))
}

#[test]
fn the_credential_store_wins_over_a_file() {
    let dir = tempfile::tempdir().unwrap();
    write_token_file(dir.path(), "eat_from_file", 0o600);
    let source = source(dir.path(), KeyringOutcome::Token("eat_from_keyring".into()));
    assert_eq!(source.token(CLIENT_ID), Ok("eat_from_keyring".to_string()));
}

#[tokio::test]
async fn a_file_token_is_used_when_the_credential_store_has_no_entry() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| {
        Reply::json(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }).to_string())
    })
    .await;
    write_discovery(dir.path(), app.port);
    write_token_file(dir.path(), "eat_from_file", 0o600);

    let lines = relay_once(source(dir.path(), KeyringOutcome::Fallback)).await;

    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(parse(&lines[0])["result"], json!({}));
    let requests = app.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        requests[0].header("authorization"),
        Some("Bearer eat_from_file"),
        "the file token is the bearer the app sees"
    );
    assert!(
        !requests[0].body.contains("eat_from_file"),
        "the token never travels in a body"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_token_file_other_users_can_read_is_refused_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let app = FakeApp::start(|_| Reply::json("{}")).await;
    write_discovery(dir.path(), app.port);
    write_token_file(dir.path(), "eat_world_readable", 0o644);

    let lines = relay_once(source(dir.path(), KeyringOutcome::Fallback)).await;

    assert_eq!(lines.len(), 1, "{lines:?}");
    let response = parse(&lines[0]);
    assert_eq!(response["error"]["code"], -32006);
    assert_eq!(response["error"]["data"]["code"], "credential_insecure");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty() && message.contains("permisos")),
        "the message must tell the person what to fix: {response}"
    );
    assert!(
        app.requests().is_empty(),
        "the MCP message was not sent with an insecure credential"
    );
    assert!(
        app.all_requests()
            .iter()
            .all(|request| request.header("authorization").is_none()),
        "no token left the connector"
    );
}

#[test]
fn neither_a_store_entry_nor_a_file_is_a_missing_credential() {
    let dir = tempfile::tempdir().unwrap();
    let source = source(dir.path(), KeyringOutcome::Fallback);
    assert_eq!(source.token(CLIENT_ID), Err(TokenError::Missing));
}

#[test]
fn another_store_error_never_falls_back_to_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write_token_file(dir.path(), "eat_should_not_be_used", 0o600);
    let source = source(
        dir.path(),
        KeyringOutcome::Failed("no pude leer la credencial".into()),
    );
    assert_eq!(
        source.token(CLIENT_ID),
        Err(TokenError::Failed("no pude leer la credencial".into()))
    );
}

#[test]
fn a_dev_profile_reads_the_token_directory_inside_its_own_data_dir() {
    let root = tempfile::tempdir().unwrap();
    let shared = cli::resolve_data_dir(Some(root.path()), None, Path::new(""));
    let profile = cli::resolve_data_dir(Some(root.path()), Some("agents"), Path::new(""));
    assert_eq!(profile, shared.join(cli::PROFILES_DIR).join("agents"));
    assert_eq!(
        token_path(&profile, CLIENT_ID),
        profile.join(TOKENS_DIR).join(format!("{CLIENT_ID}.token"))
    );

    // A release run's file in the shared root belongs to another run.
    write_token_file(&shared, "eat_release", 0o600);
    write_token_file(&profile, "eat_profile", 0o600);
    let source = source(&profile, KeyringOutcome::Fallback);
    assert_eq!(source.token(CLIENT_ID), Ok("eat_profile".to_string()));
}
