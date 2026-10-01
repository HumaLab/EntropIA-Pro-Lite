//! The IPC access control, exercised through Tauri's real entry point.
//!
//! Every page a webview loads receives the IPC bridge and the invoke key, so
//! what stops a remote page from calling `db_execute` is the ACL alone. These
//! tests build the app with the REAL context (`generate_context!`): the actual
//! capabilities and the actual app manifest from build.rs, not a mock.
//!
//! Only the command bodies are stubs. The ACL keys on the command name, so a
//! stub named `db_execute` answers to exactly the permission the real one does,
//! and an allowed call can succeed without needing the archive, keyring and
//! sync state the real commands take.

use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[tauri::command]
fn db_execute() -> &'static str {
    "ran"
}

#[tauri::command]
fn settings_get_all() -> &'static str {
    "ran"
}

#[tauri::command]
fn open_external_url() -> &'static str {
    "ran"
}

#[tauri::command]
fn sync_login() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_open() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_navigate() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_back() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_forward() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_reload() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_new_tab() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_activate_tab() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_close_tab() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_set_bounds() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_set_visible() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_close() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_state() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_capture_page() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_capture_selection() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_download_dir() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_set_download_dir() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_save_capture() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_save_download() -> &'static str {
    "ran"
}

#[tauri::command]
fn navegador_discard_draft() -> &'static str {
    "ran"
}

/// The label of the navegador's first tab, the webview remote content lives in.
const EXTERNAL_LABEL: &str = "navegador-web-1";

/// Tabs a browser may have open (`navegador::tabs::MAX_TABS`). The module is
/// private to the crate, so the value is repeated here on purpose;
/// `tests/acl_manifest_guard.rs` reads `tabs.rs` and fails when it moves.
const MAX_TABS: u32 = 4;

/// The label of tab `n`: `navegador-web-<n>`, from `navegador::tabs::label`
/// (same guard as above for the format).
fn tab_label(n: u32) -> String {
    format!("navegador-web-{n}")
}

/// Every label a browser webview can carry: each tab up to one past the limit
/// (ids are never reused, so a long session goes beyond it), a far-off id, and
/// the label the first prototype used before tabs existed.
fn browser_tab_labels() -> Vec<String> {
    (1..=MAX_TABS + 1)
        .chain([1_000])
        .map(tab_label)
        .chain(["navegador-web".to_string()])
        .collect()
}

/// The labels of the pop-up windows a page can ask that browser for (sign-in
/// flows): `navegador-popup-<n>`, from `navegador::popup::label`. The module is
/// private to the crate, so the format is repeated here on purpose; its own unit
/// test pins `label(1)` to the first of these.
const POPUP_LABELS: [&str; 3] = [
    "navegador-popup-1",
    "navegador-popup-2",
    "navegador-popup-3",
];

/// Commands that must never be reachable by remote content.
const SENSITIVE_APP_COMMANDS: [&str; 4] = [
    "db_execute",
    "settings_get_all",
    "open_external_url",
    "sync_login",
];

/// The commands the Navegador view drives its child webview with. The page
/// inside that webview must never reach them.
const NAVEGADOR_COMMANDS: [&str; 19] = [
    "navegador_open",
    "navegador_navigate",
    "navegador_back",
    "navegador_forward",
    "navegador_reload",
    "navegador_new_tab",
    "navegador_activate_tab",
    "navegador_close_tab",
    "navegador_set_bounds",
    "navegador_set_visible",
    "navegador_close",
    "navegador_state",
    "navegador_capture_page",
    "navegador_capture_selection",
    "navegador_download_dir",
    "navegador_set_download_dir",
    "navegador_save_capture",
    "navegador_save_download",
    "navegador_discard_draft",
];

/// A file-system read through a plugin: plugin commands are ACL-checked with
/// or without a manifest, so this proves the plugin side stays closed too.
const SENSITIVE_PLUGIN_COMMAND: &str = "plugin:fs|read_file";

fn build_app() -> App<tauri::test::MockRuntime> {
    mock_builder()
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            db_execute,
            settings_get_all,
            open_external_url,
            sync_login,
            navegador_open,
            navegador_navigate,
            navegador_back,
            navegador_forward,
            navegador_reload,
            navegador_new_tab,
            navegador_activate_tab,
            navegador_close_tab,
            navegador_set_bounds,
            navegador_set_visible,
            navegador_close,
            navegador_state,
            navegador_capture_page,
            navegador_capture_selection,
            navegador_download_dir,
            navegador_set_download_dir,
            navegador_save_capture,
            navegador_save_download,
            navegador_discard_draft
        ])
        .build(tauri::generate_context!())
        .expect("build the app with its real context")
}

/// The main window, however this Tauri version materialises config windows.
fn main_webview(app: &App<tauri::test::MockRuntime>) -> WebviewWindow<tauri::test::MockRuntime> {
    app.get_webview_window("main").unwrap_or_else(|| {
        WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
            .build()
            .expect("create the main window")
    })
}

fn external_webview(
    app: &App<tauri::test::MockRuntime>,
) -> WebviewWindow<tauri::test::MockRuntime> {
    WebviewWindowBuilder::new(
        app,
        EXTERNAL_LABEL,
        WebviewUrl::External("https://example.com/".parse().unwrap()),
    )
    .build()
    .expect("create the external-content webview")
}

fn popup_webview(
    app: &App<tauri::test::MockRuntime>,
    label: &str,
) -> WebviewWindow<tauri::test::MockRuntime> {
    WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::External("https://accounts.example.com/".parse().unwrap()),
    )
    .build()
    .expect("create a popup webview")
}

fn tab_webview(
    app: &App<tauri::test::MockRuntime>,
    label: &str,
) -> WebviewWindow<tauri::test::MockRuntime> {
    WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::External("https://example.com/".parse().unwrap()),
    )
    .build()
    .expect("create a tab webview")
}

/// The URL the app's own pages are served from on this platform.
fn local_url() -> &'static str {
    if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost/"
    } else {
        "tauri://localhost/"
    }
}

fn invoke(
    webview: &WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    url: &str,
) -> Result<String, String> {
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: url.parse().unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| format!("{:?}", body.deserialize::<serde_json::Value>()))
    .map_err(|e| match e {
        serde_json::Value::String(message) => message,
        other => other.to_string(),
    })
}

/// The ACL's own rejection wording, in every form Tauri 2.11 produces it: the
/// debug messages from `RuntimeAuthority::resolve_access_message` (window and
/// webview scoped, or origin scoped) and the release message from
/// `Webview::on_message`. A plugin's own scope error (an fs path outside its
/// scope, say) also says "not allowed" but matches none of these.
fn is_acl_rejection(message: &str) -> bool {
    message.contains(" not allowed on window \"")
        || message.contains(" not allowed on origin [")
        || message.contains(" not allowed by ACL")
}

fn assert_acl_rejected(result: Result<String, String>, what: &str) {
    match result {
        Err(message) if is_acl_rejection(&message) => {}
        other => panic!("{what}: expected an ACL rejection, got {other:?}"),
    }
}

/// An allowed call may still fail (bad arguments, no plugin state), but never
/// with the ACL's message.
fn assert_not_acl_rejected(result: Result<String, String>, what: &str) {
    if let Err(message) = result {
        assert!(
            !is_acl_rejection(&message),
            "{what}: rejected by the ACL: {message}"
        );
    }
}

#[test]
fn the_configured_main_window_is_labelled_main() {
    let app = build_app();
    let labels: Vec<_> = app
        .config()
        .app
        .windows
        .iter()
        .map(|w| w.label.clone())
        .collect();
    assert_eq!(
        labels,
        ["main"],
        "the capability selects the `main` webview"
    );
}

#[test]
fn a_remote_page_cannot_call_app_or_plugin_commands() {
    let app = build_app();
    let _main = main_webview(&app);
    let external = external_webview(&app);
    for cmd in SENSITIVE_APP_COMMANDS
        .iter()
        .copied()
        .chain([SENSITIVE_PLUGIN_COMMAND])
    {
        assert_acl_rejected(
            invoke(&external, cmd, "https://example.com/"),
            &format!("{cmd} from a remote page"),
        );
    }
}

#[test]
fn the_external_webview_gets_nothing_even_when_its_page_looks_local() {
    // Access is granted to the `main` webview by label. Another webview that
    // ends up on an app URL (a redirect, a bug, a misrouted navigation) must
    // not inherit it.
    let app = build_app();
    let _main = main_webview(&app);
    let external = external_webview(&app);
    for cmd in SENSITIVE_APP_COMMANDS
        .iter()
        .copied()
        .chain([SENSITIVE_PLUGIN_COMMAND])
    {
        assert_acl_rejected(
            invoke(&external, cmd, local_url()),
            &format!("{cmd} from a non-main webview on a local URL"),
        );
    }
}

#[test]
fn the_main_webview_still_reaches_its_commands() {
    let app = build_app();
    let main = main_webview(&app);
    for cmd in SENSITIVE_APP_COMMANDS {
        let response = invoke(&main, cmd, local_url());
        assert!(
            matches!(&response, Ok(body) if body.contains("ran")),
            "{cmd} from main: {response:?}"
        );
    }
    assert_not_acl_rejected(
        invoke(&main, "plugin:dialog|open", local_url()),
        "dialog:allow-open from main",
    );
    assert_not_acl_rejected(
        invoke(&main, "plugin:fs|exists", local_url()),
        "fs:allow-exists from main",
    );
}

#[test]
fn the_main_webview_keeps_the_core_permissions_the_ui_relies_on() {
    let app = build_app();
    let main = main_webview(&app);
    // Granted in capabilities/default.json; some fail later in the mock runtime
    // (missing arguments, no window state), which is fine: only an ACL
    // rejection is a regression here.
    for cmd in [
        "plugin:event|listen",
        "plugin:window|minimize",
        "plugin:window|start_dragging",
        "plugin:webview|set_webview_zoom",
        "plugin:path|resolve_directory",
    ] {
        assert_not_acl_rejected(invoke(&main, cmd, local_url()), &format!("{cmd} from main"));
    }
    // And they stay closed to the external webview: same commands, same URL.
    let external = external_webview(&app);
    for cmd in ["plugin:event|listen", "plugin:window|minimize"] {
        assert_acl_rejected(
            invoke(&external, cmd, "https://example.com/"),
            &format!("{cmd} from a remote page"),
        );
    }
}

#[test]
fn a_lookalike_origin_is_remote_even_from_the_main_webview() {
    // `<scheme>.evil.com` hosts once passed as the app's own custom protocol
    // (GHSA-7gmj-67g7-phm9). A page there calling through `main` must be
    // treated as remote and get nothing.
    let app = build_app();
    let main = main_webview(&app);
    for url in [
        "http://ipc.example.com/",
        "http://asset.example.com/",
        "http://tauri.example.com/",
        "https://tauri.localhost.example.com/",
    ] {
        for cmd in SENSITIVE_APP_COMMANDS
            .iter()
            .copied()
            .chain([SENSITIVE_PLUGIN_COMMAND])
        {
            assert_acl_rejected(invoke(&main, cmd, url), &format!("{cmd} from {url}"));
        }
    }
}

#[test]
fn the_main_webview_reaches_the_navegador_commands() {
    let app = build_app();
    let main = main_webview(&app);
    for cmd in NAVEGADOR_COMMANDS {
        let response = invoke(&main, cmd, local_url());
        assert!(
            matches!(&response, Ok(body) if body.contains("ran")),
            "{cmd} from main: {response:?}"
        );
    }
}

#[test]
fn the_page_inside_the_navegador_cannot_drive_the_navegador() {
    // Otherwise a page could navigate itself anywhere, or hide the address
    // policy behind a command that skips it.
    let app = build_app();
    let _main = main_webview(&app);
    let external = external_webview(&app);
    for cmd in NAVEGADOR_COMMANDS {
        assert_acl_rejected(
            invoke(&external, cmd, "https://example.com/"),
            &format!("{cmd} from a remote page"),
        );
        assert_acl_rejected(
            invoke(&external, cmd, local_url()),
            &format!("{cmd} from a non-main webview on a local URL"),
        );
    }
}

#[test]
fn a_lookalike_origin_cannot_reach_the_navegador_commands_through_main() {
    let app = build_app();
    let main = main_webview(&app);
    for cmd in NAVEGADOR_COMMANDS {
        assert_acl_rejected(
            invoke(&main, cmd, "http://tauri.example.com/"),
            &format!("{cmd} from a lookalike origin"),
        );
    }
}

#[test]
fn a_popup_window_gets_nothing_from_a_page_or_from_a_local_looking_url() {
    // A sign-in popup is a real window with its own webview. It has no
    // capability, so it is as closed as the browser's own webview, whatever the
    // page in it claims to be.
    let app = build_app();
    let _main = main_webview(&app);
    for label in POPUP_LABELS {
        let popup = popup_webview(&app, label);
        for url in ["https://accounts.example.com/", local_url()] {
            for cmd in SENSITIVE_APP_COMMANDS
                .iter()
                .copied()
                .chain(NAVEGADOR_COMMANDS)
                .chain([SENSITIVE_PLUGIN_COMMAND, "plugin:event|listen"])
            {
                assert_acl_rejected(
                    invoke(&popup, cmd, url),
                    &format!("{cmd} from {label} on {url}"),
                );
            }
        }
    }
}

#[test]
fn every_browser_tab_label_gets_nothing_from_a_page_or_from_a_local_looking_url() {
    // Each tab is its own webview, so the label is what tells the ACL it is not
    // the app: no tab, whichever number it carries, may reach a command.
    let app = build_app();
    let _main = main_webview(&app);
    for label in browser_tab_labels() {
        let tab = tab_webview(&app, &label);
        for url in ["https://example.com/", local_url()] {
            for cmd in SENSITIVE_APP_COMMANDS
                .iter()
                .copied()
                .chain(NAVEGADOR_COMMANDS)
                .chain([SENSITIVE_PLUGIN_COMMAND, "plugin:event|listen"])
            {
                assert_acl_rejected(
                    invoke(&tab, cmd, url),
                    &format!("{cmd} from {label} on {url}"),
                );
            }
        }
    }
}

#[test]
fn no_capability_selects_a_popup_window() {
    // The ACL match above holds because no capability names these labels (the
    // popup windows' and the tabs'). A wildcard (`navegador-*`, `*`) or a
    // `windows` entry (a window match also covers the webviews inside it) would
    // hand a popup or a tab the app.
    let capabilities = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    for entry in std::fs::read_dir(&capabilities).expect("read capabilities") {
        let path = entry.expect("capability entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let capability: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read capability"))
                .expect("capability is JSON");
        for key in ["windows", "webviews"] {
            for pattern in capability[key].as_array().into_iter().flatten() {
                let pattern = pattern.as_str().expect("a label pattern");
                assert!(
                    !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('['),
                    "{}: `{key}` holds the pattern {pattern:?}, which could match a popup",
                    path.display()
                );
                assert!(
                    !pattern.starts_with("navegador"),
                    "{}: `{key}` names {pattern:?}: the browser, its tabs and its popups get no capability",
                    path.display()
                );
                assert!(
                    !browser_tab_labels().contains(&pattern.to_string())
                        && !POPUP_LABELS.contains(&pattern),
                    "{}: `{key}` names the browser webview {pattern:?}",
                    path.display()
                );
            }
        }
        assert!(
            capability["windows"]
                .as_array()
                .is_none_or(|w| w.is_empty()),
            "{}: a `windows` entry also covers the webviews inside that window",
            path.display()
        );
    }
}
