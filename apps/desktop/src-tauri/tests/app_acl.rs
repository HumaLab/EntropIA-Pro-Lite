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

/// The label the navegador's remote-content webview will carry.
const EXTERNAL_LABEL: &str = "navegador-web";

/// Commands that must never be reachable by remote content.
const SENSITIVE_APP_COMMANDS: [&str; 4] = [
    "db_execute",
    "settings_get_all",
    "open_external_url",
    "sync_login",
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
            sync_login
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
    .map_err(|e| e.to_string())
}

fn assert_acl_rejected(result: Result<String, String>, what: &str) {
    match result {
        Err(message) if message.contains("not allowed") => {}
        other => panic!("{what}: expected an ACL rejection, got {other:?}"),
    }
}

/// An allowed call may still fail (bad arguments, no plugin state), but never
/// with the ACL's message.
fn assert_not_acl_rejected(result: Result<String, String>, what: &str) {
    if let Err(message) = result {
        assert!(
            !message.contains("not allowed"),
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
