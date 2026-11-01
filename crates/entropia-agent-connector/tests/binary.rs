//! The built binary's own contract: version, usage, and a refusal that happens
//! before anything touches the keyring or the archive.

use std::process::Command;

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
