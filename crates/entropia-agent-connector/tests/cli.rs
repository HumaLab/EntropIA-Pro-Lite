//! CLI surface, data-directory resolution and the keyring pair the app also
//! computes.

use std::path::{Path, PathBuf};

use entropia_agent_connector::cli::{self, Invocation, Options};
use entropia_agent_connector::keyring_source::keyring_target;

fn args(list: &[&str]) -> Vec<String> {
    std::iter::once("entropia-agent-connector")
        .chain(list.iter().copied())
        .map(str::to_string)
        .collect()
}

fn run(options: Options) -> Invocation {
    Invocation::Run(options)
}

// ---------------------------------------------------------------------------
// Keyring target: identical to automation::access::keyring_target
// ---------------------------------------------------------------------------

#[test]
fn without_a_profile_the_keyring_pair_is_the_real_app_s() {
    assert_eq!(
        keyring_target(None, "cl_1"),
        (
            "com.entropia.agent-connector".to_string(),
            "cl_1".to_string()
        )
    );
}

#[test]
fn a_profile_uses_the_dev_profile_service_and_account() {
    assert_eq!(
        keyring_target(Some("agents"), "cl_1"),
        (
            "com.entropia.agent-connector (dev profile)".to_string(),
            "cl_1@dev-profile:agents".to_string()
        )
    );
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[test]
fn a_client_id_is_enough() {
    assert_eq!(
        cli::parse_args(args(&["--client", "cl_1"])),
        Ok(run(Options {
            client_id: "cl_1".to_string(),
            profile: None,
        }))
    );
}

#[test]
fn a_profile_is_carried_along() {
    assert_eq!(
        cli::parse_args(args(&["--client", "cl_1", "--profile", "agents"])),
        Ok(run(Options {
            client_id: "cl_1".to_string(),
            profile: Some("agents".to_string()),
        }))
    );
    // Both flags also accept the `--flag=value` form.
    assert_eq!(
        cli::parse_args(args(&["--client=cl_1", "--profile=agents"])),
        Ok(run(Options {
            client_id: "cl_1".to_string(),
            profile: Some("agents".to_string()),
        }))
    );
}

#[test]
fn version_and_help_are_recognised() {
    assert_eq!(
        cli::parse_args(args(&["--version"])),
        Ok(Invocation::Version)
    );
    assert_eq!(cli::parse_args(args(&["-V"])), Ok(Invocation::Version));
    assert_eq!(cli::parse_args(args(&["--help"])), Ok(Invocation::Help));
    assert_eq!(cli::parse_args(args(&["-h"])), Ok(Invocation::Help));
}

#[test]
fn a_missing_client_id_is_refused_and_the_usage_is_printed() {
    let error = cli::parse_args(args(&[])).unwrap_err();
    assert!(error.0.contains("--client"), "{}", error.0);
}

#[test]
fn an_unknown_option_is_refused_rather_than_ignored() {
    let error = cli::parse_args(args(&["--client", "cl_1", "--token", "eat_x"])).unwrap_err();
    assert!(error.0.contains("--token"), "{}", error.0);
    assert!(
        !error.0.contains("eat_x") || !error.0.contains("Bearer"),
        "never echo a token: {}",
        error.0
    );
}

#[test]
fn a_flag_without_its_value_is_refused() {
    assert!(cli::parse_args(args(&["--client"])).is_err());
    assert!(cli::parse_args(args(&["--client", "cl_1", "--profile"])).is_err());
}

#[test]
fn an_invalid_profile_name_is_refused() {
    for bad in [
        "",
        "..",
        ".",
        "a/b",
        "a\\b",
        "with space",
        "ñandú",
        "a.b",
        &"a".repeat(33),
    ] {
        let error = cli::parse_args(args(&["--client", "cl_1", "--profile", bad])).unwrap_err();
        assert!(
            error.0.contains("perfil"),
            "{bad:?} was refused without saying why: {}",
            error.0
        );
    }
    assert!(cli::parse_args(args(&["--client", "cl_1", "--profile", "A_b-9"])).is_ok());
    assert!(cli::parse_args(args(&["--client", "cl_1", "--profile", &"a".repeat(32)])).is_ok());
}

#[test]
fn a_client_id_that_is_not_a_plain_name_is_refused() {
    for bad in ["", "a b", "a/b", "a\tb", &"a".repeat(65)] {
        assert!(
            cli::parse_args(args(&["--client", bad])).is_err(),
            "{bad:?} must be refused"
        );
    }
    assert!(cli::parse_args(args(&["--client", &"a".repeat(64)])).is_ok());
}

// ---------------------------------------------------------------------------
// Data directory
// ---------------------------------------------------------------------------

fn platform() -> PathBuf {
    PathBuf::from("platform-data")
}

#[test]
fn the_shared_directory_sits_under_the_platform_data_directory() {
    assert_eq!(
        cli::resolve_data_dir(None, None, &platform()),
        platform().join("com.entropia.shared")
    );
}

#[test]
fn the_override_replaces_the_platform_directory_entirely() {
    let override_dir = PathBuf::from("scratch-data");
    assert_eq!(
        cli::resolve_data_dir(Some(&override_dir), None, &platform()),
        override_dir
    );
}

#[test]
fn a_profile_is_nested_under_the_dev_profiles_directory() {
    let data = cli::resolve_data_dir(None, Some("agents"), &platform());
    assert_eq!(
        data,
        platform()
            .join("com.entropia.shared")
            .join("dev-profiles")
            .join("agents")
    );
    assert_ne!(
        data,
        cli::resolve_data_dir(None, None, &platform()),
        "a profile must never read the real archive"
    );

    let overridden =
        cli::resolve_data_dir(Some(Path::new("scratch-data")), Some("agents"), &platform());
    assert_eq!(
        overridden,
        PathBuf::from("scratch-data")
            .join("dev-profiles")
            .join("agents")
    );
}
