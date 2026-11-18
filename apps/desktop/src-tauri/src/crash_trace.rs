//! Traces of an exit that skipped the normal close (R24).
//!
//! Pro 1.8.0 vanished once with no `app/close` line, no Windows Error
//! Reporting entry and no dump: the log could not tell a panic, a dead
//! WebView2 and an outside kill apart. Three traces close that gap:
//!
//! - a panic hook that writes the panic to the app log before the default
//!   hook runs;
//! - on Windows, the main webview's `ProcessFailed` event (browser, renderer
//!   or GPU process gone), logged as it happens;
//! - a session marker written at startup and removed on every orderly exit:
//!   a marker found at the next startup means the previous session ended
//!   without closing, whatever killed it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const SESSION_MARKER: &str = "session-running";

static MARKER_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Log every panic, caught or not, then run the previous hook. Caught panics
/// (third-party parsers under `catch_unwind`) are logged too: they are rare
/// and worth seeing.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_else(|| "unknown".to_string());
        log::error!(
            target: "panic",
            "panic en el hilo {} ({location}): {}",
            thread.name().unwrap_or("sin nombre"),
            panic_message(info.payload())
        );
        previous(info);
    }));
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(sin mensaje)")
}

/// Note a session that ended without closing, then mark this one as running.
/// Call once the app log is attached, so the warning is persisted.
pub fn begin_session(log_dir: &Path) {
    let marker = log_dir.join(SESSION_MARKER);
    if let Some(previous) = read_previous_session(&marker) {
        log::warn!(
            target: "app/session",
            "La sesión anterior ({previous}) terminó sin cerrar: la app se cortó, se colgó o la cerraron desde afuera"
        );
    }
    // The log lines around it carry the times; the pid tells the sessions apart.
    let stamp = format!("pid {}", std::process::id());
    let _ = std::fs::create_dir_all(log_dir);
    if let Err(error) = std::fs::write(&marker, stamp) {
        log::warn!(target: "app/session", "No se pudo marcar la sesión: {error}");
    }
    let _ = MARKER_PATH.set(marker);
}

fn read_previous_session(marker: &Path) -> Option<String> {
    let text = std::fs::read_to_string(marker).ok()?;
    let text = text.trim();
    Some(if text.is_empty() {
        "sin datos".to_string()
    } else {
        text.to_string()
    })
}

/// The session closed in order: drop the marker.
pub fn end_session() {
    if let Some(marker) = MARKER_PATH.get() {
        let _ = std::fs::remove_file(marker);
    }
}

/// Log the main webview's process failures (browser, renderer, GPU...).
#[cfg(windows)]
pub fn watch_webview_processes(window: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND;
    use webview2_com::ProcessFailedEventHandler;

    let result = window.with_webview(|platform| {
        // SAFETY: COM calls on the UI thread, on the controller Tauri owns
        // for this window; the handler captures nothing.
        unsafe {
            let Ok(core) = platform.controller().CoreWebView2() else {
                return;
            };
            let mut token = 0i64;
            let _ = core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(|_, args| {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    if let Some(args) = args {
                        let _ = args.ProcessFailedKind(&mut kind);
                    }
                    log::error!(
                        target: "webview",
                        "Falló un proceso de WebView2: {}",
                        process_failed_kind(kind.0)
                    );
                    Ok(())
                })),
                &mut token,
            );
        }
    });
    if let Err(error) = result {
        log::warn!(target: "webview", "No se pudo vigilar WebView2: {error}");
    }
}

#[cfg(not(windows))]
pub fn watch_webview_processes(_window: &tauri::WebviewWindow) {}

/// `COREWEBVIEW2_PROCESS_FAILED_KIND` values, from the WebView2 SDK.
#[cfg_attr(not(windows), allow(dead_code))]
fn process_failed_kind(kind: i32) -> &'static str {
    match kind {
        0 => "el proceso del navegador terminó (la ventana queda inutilizable)",
        1 => "el proceso de render terminó",
        2 => "el proceso de render no responde",
        3 => "el render de un marco terminó",
        4 => "un proceso utilitario terminó",
        5 => "el ayudante del sandbox terminó",
        6 => "el proceso de GPU terminó",
        7 => "un plugin PPAPI terminó",
        8 => "el broker PPAPI terminó",
        _ => "un proceso desconocido terminó",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_left_behind_names_the_unclosed_session() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(SESSION_MARKER);
        assert_eq!(
            read_previous_session(&marker),
            None,
            "a clean start says nothing"
        );
        std::fs::write(&marker, "pid 42").unwrap();
        assert_eq!(read_previous_session(&marker).as_deref(), Some("pid 42"));
    }

    #[test]
    fn panic_payloads_read_as_text() {
        assert_eq!(panic_message(&"boom"), "boom");
        assert_eq!(panic_message(&String::from("boom")), "boom");
        assert_eq!(panic_message(&42), "(sin mensaje)");
    }
}
