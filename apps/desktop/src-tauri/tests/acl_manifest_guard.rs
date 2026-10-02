//! Keeps three lists that live in three files from drifting apart.
//!
//! Tauri only enforces its ACL on app commands when the app ships an app
//! manifest, and a command is callable from a webview only when a capability
//! grants its generated `allow-*` permission. So a command has to be named in
//! `generate_handler!` (lib.rs), in `APP_COMMANDS` (build.rs) and as a
//! permission in `capabilities/default.json`. Miss the second and the command
//! has no permission at all; miss the third and the UI can no longer call it,
//! which only shows up at runtime.
//!
//! The lists are read as text on purpose: the handler list is macro input and
//! the manifest is a build-script constant, so neither can be reached from a
//! test any other way.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn read(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Command names registered with `generate_handler!`, last path segment only.
fn registered_commands() -> BTreeSet<String> {
    let lib = read("src/lib.rs");
    let start = lib
        .find("generate_handler![")
        .expect("lib.rs registers commands with generate_handler!")
        + "generate_handler![".len();
    let body = &lib[start..start + lib[start..].find(']').expect("generate_handler! closes")];
    body.lines()
        .map(|line| {
            line.split("//")
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches(',')
        })
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.rsplit("::").next().unwrap().trim().to_string())
        .collect()
}

/// Command names listed in build.rs's `APP_COMMANDS`.
fn manifest_commands() -> BTreeSet<String> {
    let build = read("build.rs");
    let start = build
        .find("const APP_COMMANDS")
        .expect("build.rs declares APP_COMMANDS");
    let list = &build[start..];
    let list = &list[list.find("= &[").expect("APP_COMMANDS is a slice") + 4..];
    let list = &list[..list.find("];").expect("APP_COMMANDS closes")];
    list.lines()
        .map(|line| line.split("//").next().unwrap_or("").trim())
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.trim_end_matches(',').trim_matches('"').to_string())
        .collect()
}

fn capability() -> serde_json::Value {
    serde_json::from_str(&read("capabilities/default.json")).expect("default.json is JSON")
}

/// tauri-build names a command's permission `allow-<command>` with `_` as `-`.
fn permission_for(command: &str) -> String {
    format!("allow-{}", command.replace('_', "-"))
}

/// App permissions granted by the capability: bare identifiers, no plugin prefix.
fn granted_app_permissions() -> BTreeSet<String> {
    capability()["permissions"]
        .as_array()
        .expect("permissions is an array")
        .iter()
        .filter_map(|p| p.as_str())
        .filter(|p| !p.contains(':'))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_registered_command_is_in_the_app_manifest_and_nothing_stale_is() {
    let registered = registered_commands();
    let manifest = manifest_commands();
    assert!(
        registered.len() > 100,
        "parsed too few registered commands: {}",
        registered.len()
    );
    let missing: Vec<_> = registered.difference(&manifest).collect();
    let stale: Vec<_> = manifest.difference(&registered).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "build.rs APP_COMMANDS drifted from generate_handler!\n  registered but not in the manifest: {missing:?}\n  in the manifest but not registered: {stale:?}"
    );
}

#[test]
fn the_main_webview_is_granted_exactly_the_manifest_permissions() {
    let expected: BTreeSet<String> = manifest_commands()
        .iter()
        .map(|c| permission_for(c))
        .collect();
    let granted = granted_app_permissions();
    let missing: Vec<_> = expected.difference(&granted).collect();
    let stale: Vec<_> = granted.difference(&expected).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "capabilities/default.json drifted from the app manifest\n  not granted: {missing:?}\n  granted but not in the manifest: {stale:?}"
    );
}

#[test]
fn the_capability_targets_the_main_webview_not_the_main_window() {
    // A `windows` match also covers every child webview of that window, so a
    // remote page hosted inside `main` would inherit the whole capability. The
    // `webviews` selector matches on the webview's own label only.
    let capability = capability();
    assert_eq!(capability["webviews"], serde_json::json!(["main"]));
    assert!(
        capability.get("windows").is_none(),
        "capabilities/default.json must not select by window"
    );
    assert!(
        capability.get("remote").is_none(),
        "the default capability must stay local-only"
    );
}

#[test]
fn no_other_capability_file_exists_to_grant_app_commands_elsewhere() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["default.json"]);
}

/// The value of a `const NAME: <ty> = <value>;` line, as text.
fn const_value(source: &str, name: &str) -> String {
    let marker = format!("const {name}:");
    let line = source
        .lines()
        .find(|line| line.contains(&marker))
        .unwrap_or_else(|| panic!("no `{marker}` in the source"));
    line.split('=')
        .nth(1)
        .unwrap_or_else(|| panic!("`{line}` has no value"))
        .trim()
        .trim_end_matches(';')
        .trim_matches('"')
        .to_string()
}

#[test]
fn the_acl_tests_repeat_the_browser_label_format_and_limit_the_app_really_uses() {
    // `navegador::tabs` is private to the crate, so tests/app_acl.rs repeats its
    // label format and tab limit. If either moves there and not here, the ACL
    // tests would keep proving a label nobody creates any more.
    let tabs = read("src/navegador/tabs.rs");
    let acl = read("tests/app_acl.rs");
    assert_eq!(
        const_value(&tabs, "MAX_TABS"),
        const_value(&acl, "MAX_TABS"),
        "the tab limit in tests/app_acl.rs drifted from navegador/tabs.rs"
    );
    let prefix = const_value(&tabs, "LABEL_PREFIX");
    assert!(
        acl.contains(&format!("format!(\"{prefix}{{n}}\")")),
        "tests/app_acl.rs builds tab labels with another format than `{prefix}<n>`"
    );
    assert!(
        acl.contains("const EXTERNAL_LABEL: &str = \"navegador-web-1\""),
        "the first tab of the ACL tests is not `{prefix}1`"
    );
}

#[test]
fn no_capability_names_a_browser_tab_or_popup_label() {
    // The ACL refuses a tab or a popup only because nothing grants it anything.
    // The tab labels are built from the same prefix the app uses.
    let tabs = read("src/navegador/tabs.rs");
    let prefix = const_value(&tabs, "LABEL_PREFIX");
    let capability = capability();
    for key in ["webviews", "windows"] {
        for pattern in capability[key].as_array().into_iter().flatten() {
            let pattern = pattern.as_str().expect("a label pattern");
            assert!(
                !pattern.starts_with(prefix.trim_end_matches('-'))
                    && !pattern.starts_with("navegador"),
                "`{key}` names {pattern:?}: a browser tab gets no capability"
            );
        }
    }
}
