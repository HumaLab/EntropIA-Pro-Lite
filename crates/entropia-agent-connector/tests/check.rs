mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{FakeApp, Reply, INSTANCE_KEY};
use entropia_agent_connector::cli;
use entropia_agent_connector::relay::{self, ConnectorSource, RelayConfig, TokenError};
use serde_json::json;

const TOKEN: &str = "eat_check-test-secret";
struct Source(PathBuf, bool);
impl ConnectorSource for Source {
    fn discovery_path(&self) -> PathBuf {
        self.0.clone()
    }
    fn token(&self, _: &str) -> Result<String, TokenError> {
        if self.1 {
            Ok(TOKEN.into())
        } else {
            Err(TokenError::Missing)
        }
    }
}

#[test]
fn check_cli_forms_and_errors() {
    for args in [
        vec!["--check", "--client", "cl_test", "--profile", "test"],
        vec!["--client=cl_test", "--profile=test", "--check"],
    ] {
        let parsed = cli::parse_args(std::iter::once("connector").chain(args).map(str::to_string));
        assert!(parsed.is_ok(), "{parsed:?}");
        assert!(format!("{parsed:?}").contains("Check"));
    }
    for args in [
        vec!["--check"],
        vec!["--check", "--check", "--client=x"],
        vec!["--check=true", "--client=x"],
        vec!["--check", "--client=x", "--client=y"],
        vec!["--check", "--client=x", "--profile=a", "--profile=b"],
    ] {
        assert!(
            cli::parse_args(std::iter::once("connector").chain(args).map(str::to_string)).is_err()
        );
    }
    assert!(cli::USAGE.contains("--check"));
}

#[tokio::test]
async fn check_success_and_failures_use_real_relay_without_exposing_token() {
    for case in ["ok", "app_absent", "credential_missing", "unauthorized"] {
        let app = FakeApp::start(move |request| {
            if case == "unauthorized" { return Reply::status(401); }
            match request.rpc_method().as_str() {
                "initialize" => Reply::json(json!({"jsonrpc":"2.0", "id":1, "result":{"protocolVersion":"2025-03-26", "serverInfo":{"version":"2.3.4"}}}).to_string()),
                "notifications/initialized" => Reply::empty(202),
                "tools/list" => {
                    assert_eq!(request.header("MCP-Protocol-Version"), Some("2025-03-26"));
                    Reply::json(json!({"jsonrpc":"2.0", "id":2, "result":{"tools":[{}, {}]}}).to_string())
                }
                _ => panic!("unexpected method"),
            }
        }).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("discovery.json");
        if case != "app_absent" {
            std::fs::write(
                &path,
                json!({"port":app.port,"protocol":"entropia-agent/1","instanceKey":INSTANCE_KEY})
                    .to_string(),
            )
            .unwrap();
        }
        let (exit, line) = relay::check(
            RelayConfig::new("cl_test"),
            Arc::new(Source(path, case != "credential_missing")),
        )
        .await;
        assert_eq!(exit, if case == "ok" { 0 } else { 1 }, "{line}");
        assert_eq!(line.lines().count(), 1);
        assert!(!line.contains(TOKEN));
        if case == "ok" {
            assert_eq!(line, "ok: EntropIA 2.3.4 · 2 herramientas");
            assert_eq!(
                app.requests()
                    .iter()
                    .map(|r| r.rpc_method())
                    .collect::<Vec<_>>(),
                ["initialize", "notifications/initialized", "tools/list"]
            );
        } else {
            assert!(line.starts_with(&format!("{case}:")), "{line}");
        }
    }
}

#[tokio::test]
async fn check_deadline_bounds_endpoint_and_blocking_credentials() {
    use std::time::{Duration, Instant};
    struct SlowSource(PathBuf);
    impl ConnectorSource for SlowSource {
        fn discovery_path(&self) -> PathBuf {
            self.0.clone()
        }
        fn token(&self, _: &str) -> Result<String, TokenError> {
            std::thread::sleep(Duration::from_millis(500));
            Ok(TOKEN.into())
        }
    }
    let app = FakeApp::start(|_| Reply::empty(200).after(Duration::from_secs(60))).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("discovery.json");
    std::fs::write(
        &path,
        json!({"port":app.port,"protocol":"entropia-agent/1","instanceKey":INSTANCE_KEY})
            .to_string(),
    )
    .unwrap();
    for source in [
        Arc::new(Source(path.clone(), true)) as Arc<dyn ConnectorSource>,
        Arc::new(SlowSource(path)),
    ] {
        let start = Instant::now();
        let (exit, line) = relay::check_with_timeout(
            RelayConfig::new("cl_test"),
            source,
            Duration::from_millis(100),
        )
        .await;
        assert_eq!(exit, 1);
        assert_eq!(
            line,
            "timeout: EntropIA no respondió a tiempo: abrí la aplicación y volvé a intentarlo."
        );
        assert!(start.elapsed() < Duration::from_millis(400));
        assert!(!line.contains(TOKEN));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn check_profile_routes_binary_to_profile_data() {
    use entropia_agent_connector::keyring_source::{keyring_target, token_path};
    let app = FakeApp::start(|request| match request.rpc_method().as_str() {
        "initialize" => Reply::json(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","serverInfo":{"version":"1.0"}}}).to_string()),
        "notifications/initialized" => Reply::empty(202),
        "tools/list" => Reply::json(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}}).to_string()),
        _ => panic!("unexpected request"),
    }).await;
    let dir = tempfile::tempdir().unwrap();
    let profile = "check-profile";
    let client = "cl_check_profile_fixture";
    let data = cli::resolve_data_dir(Some(dir.path()), Some(profile), dir.path());
    let token = token_path(&data, client);
    std::fs::create_dir_all(token.parent().unwrap()).unwrap();
    std::fs::write(&token, TOKEN).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::fs::write(
        data.join(entropia_agent_connector::discovery::DISCOVERY_FILE),
        json!({"port":app.port,"protocol":"entropia-agent/1","instanceKey":INSTANCE_KEY})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        keyring_target(Some(profile), client),
        (
            "com.entropia.agent-connector (dev profile)".into(),
            "cl_check_profile_fixture@dev-profile:check-profile".into(),
        )
    );
    for with_profile in [true, false] {
        let mut command =
            std::process::Command::new(env!("CARGO_BIN_EXE_entropia-agent-connector"));
        command
            .args(["--check", "--client", client])
            .env("ENTROPIA_DATA_DIR", dir.path());
        if with_profile {
            command.args(["--profile", profile]);
        }
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(if with_profile { 0 } else { 1 }));
        if with_profile {
            assert_eq!(stdout, "ok: EntropIA 1.0 · 0 herramientas\n");
        } else {
            assert!(stdout.starts_with("app_absent:"));
        }
        assert_eq!(stdout.lines().count(), 1);
        assert!(output.stderr.is_empty());
        assert!(!stdout.contains(TOKEN));
    }
}

#[test]
fn check_binary_absent_and_missing_client() {
    let dir = tempfile::tempdir().unwrap();
    for client in [false, true] {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_entropia-agent-connector"));
        cmd.arg("--check").env("ENTROPIA_DATA_DIR", dir.path());
        if client {
            cmd.arg("--client=cl_check");
        }
        let output = cmd.output().unwrap();
        assert_eq!(output.status.code(), Some(if client { 1 } else { 2 }));
        if client {
            assert!(String::from_utf8_lossy(&output.stdout).starts_with("app_absent:"));
            assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 1);
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
        }
        assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
    }
}
