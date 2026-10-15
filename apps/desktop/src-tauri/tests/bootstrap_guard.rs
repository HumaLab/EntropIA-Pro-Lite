//! Tests for the release bootstrap guard in `build_support/bootstrap_guard.rs`.
//!
//! The module is included (not linked) so these tests exercise the exact pure
//! decision `build.rs` calls, without running a build script or mutating the
//! process environment: every input is passed explicitly.

#[path = "../build_support/bootstrap_guard.rs"]
mod bootstrap_guard;

use bootstrap_guard::{bootstrap_guard_error, BootstrapGuardInput};

/// Fixture pack shape of the committed `resources/runtime-pack/*/manifest.json`:
/// not a real release, so the guard must demand a baked download source. Spaces
/// and newlines are deliberate: the real manifest is pretty-printed, and the
/// guard strips all whitespace before matching.
const FIXTURE_MANIFEST: &str = r#"{
    "payload_profile": "fixture",
    "release_injection_required": true
}"#;

/// A real release pack, already injected by the release pipeline: bundling it
/// means the runtime ships inside the installer and no download source is needed.
const RELEASE_MANIFEST: &str = r#"{
    "payload_profile" : "release",
    "release_injection_required" : false
}"#;

const URL: &str = "https://example.invalid/runtime/manifest.json";
const KEY_ID: &str = "entropia-runtime-test";
const KEY_BASE64: &str = "dGVzdC1rZXk=";

fn release_input<'a>(
    target_os: &'a str,
    local_ml: bool,
    manifest: Option<&'a str>,
    manifest_url: &'a str,
    public_key_id: &'a str,
    public_key_base64: &'a str,
) -> BootstrapGuardInput<'a> {
    BootstrapGuardInput {
        profile: "release",
        target_os,
        local_ml,
        manifest,
        manifest_url,
        public_key_id,
        public_key_base64,
    }
}

/// Pro release on `target_os` bundling the fixture pack with the three given vars.
fn pro_fixture<'a>(
    target_os: &'a str,
    manifest_url: &'a str,
    public_key_id: &'a str,
    public_key_base64: &'a str,
) -> BootstrapGuardInput<'a> {
    release_input(
        target_os,
        true,
        Some(FIXTURE_MANIFEST),
        manifest_url,
        public_key_id,
        public_key_base64,
    )
}

#[test]
fn debug_builds_are_never_guarded() {
    let input = BootstrapGuardInput {
        profile: "debug",
        ..pro_fixture("windows", "", "", "")
    };
    assert!(bootstrap_guard_error(&input).is_none());
}

#[test]
fn lite_releases_need_no_bootstrap_source() {
    // Lite has no runtime: every reader of these variables is compiled out
    // (`#[cfg(feature = "local-ml")]`), so the guard must not demand them.
    for target_os in ["linux", "windows"] {
        let input = release_input(target_os, false, Some(FIXTURE_MANIFEST), "", "", "");
        assert!(
            bootstrap_guard_error(&input).is_none(),
            "Lite release on {target_os} must be allowed"
        );
    }
}

#[test]
fn pro_releases_on_macos_need_no_bootstrap_source() {
    let input = pro_fixture("macos", "", "", "");
    assert!(bootstrap_guard_error(&input).is_none());
}

#[test]
fn an_unreadable_manifest_keeps_the_historical_behavior() {
    let input = release_input("linux", true, None, "", "", "");
    assert!(bootstrap_guard_error(&input).is_none());
}

#[test]
fn a_real_release_manifest_needs_no_bootstrap_source() {
    let input = release_input("linux", true, Some(RELEASE_MANIFEST), "", "", "");
    assert!(bootstrap_guard_error(&input).is_none());
}

#[test]
fn a_fixture_manifest_with_all_three_variables_is_allowed() {
    let input = pro_fixture("windows", URL, KEY_ID, KEY_BASE64);
    assert!(bootstrap_guard_error(&input).is_none());
}

#[test]
fn each_missing_variable_is_named_exactly() {
    let all = [
        "ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL",
        "ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID",
        "ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64",
    ];
    let cases = [
        (all[0], "", KEY_ID, KEY_BASE64),
        (all[1], URL, "", KEY_BASE64),
        (all[2], URL, KEY_ID, ""),
    ];
    for (missing, url, id, key) in cases {
        let message = bootstrap_guard_error(&pro_fixture("linux", url, id, key))
            .unwrap_or_else(|| panic!("{missing} missing must fail the guard"));
        assert!(message.contains(missing), "{message}");
        for other in all {
            if other != missing {
                assert!(
                    !message.contains(other),
                    "message for {missing} must not name {other}: {message}"
                );
            }
        }
    }
}

#[test]
fn whitespace_only_values_count_as_missing() {
    let input = pro_fixture("windows", "   ", "\t", "\n ");
    let message =
        bootstrap_guard_error(&input).expect("whitespace-only variables must fail the guard");
    assert!(
        message.contains("ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL"),
        "{message}"
    );
    assert!(
        message.contains("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID"),
        "{message}"
    );
    assert!(
        message.contains("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64"),
        "{message}"
    );
}

#[test]
fn the_message_names_the_platform() {
    let windows = pro_fixture("windows", "", "", "");
    let linux = pro_fixture("linux", "", "", "");
    assert!(bootstrap_guard_error(&windows)
        .expect("windows must fail without variables")
        .contains("windows-x86_64"));
    assert!(bootstrap_guard_error(&linux)
        .expect("linux must fail without variables")
        .contains("linux-x86_64"));
}
