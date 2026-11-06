//! The built binary's own contract: version, usage, and a refusal that happens
//! before anything touches the keyring or the archive.

use std::io::Write as _;
use std::process::{Command, Stdio};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_entropia-agent-connector"))
}

#[test]
fn version_is_printed_to_stdout_and_exits_zero() {
    let output = binary().arg("--version").output().unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("entropia-agent-connector {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn without_a_client_id_it_exits_two_and_says_which_option_is_missing() {
    let output = binary().output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stdout.is_empty(),
        "stdout carried {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--client"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// S1: Claude Code keeps the server's stderr, so a refusal the connector builds
/// must be there too — with the numeric code, `data.code` and the detail — while
/// stdout keeps carrying only the JSON-RPC response.
#[test]
fn a_refused_request_is_also_written_to_stderr_with_its_code_and_detail() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = binary()
        .args(["--client", "cl_1"])
        .env("ENTROPIA_DATA_DIR", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":7,"method":"tools/list"}}"#).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.status);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim_end_matches('\n').lines().collect();
    assert_eq!(lines.len(), 1, "stdout carried {stdout:?}");
    let response: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 7);
    assert_eq!(response["error"]["code"], -32001);
    assert_eq!(response["error"]["data"]["code"], "app_absent");
    let detail = response["error"]["data"]["detail"]
        .as_str()
        .expect("app_absent carries its detail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let reported: Vec<&str> = stderr.trim_end_matches(['\n', '\r']).lines().collect();
    assert_eq!(reported.len(), 1, "stderr carried {stderr:?}");
    let expected = format!("entropia-agent-connector: -32001 app_absent: {detail}");
    assert_eq!(reported[0], expected, "stderr: {stderr:?}");
    assert!(!stderr.contains("eat_"), "never a token: {stderr}");
}

#[test]
fn an_invalid_profile_is_refused_before_the_keyring_is_touched() {
    let output = binary()
        .args(["--client", "cl_1", "--profile", "../archive"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("perfil"), "{stderr}");
    assert!(!stderr.contains("credencial"), "{stderr}");
}
