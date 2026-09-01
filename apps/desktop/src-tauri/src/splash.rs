//! Startup window: a frameless, transparent webview showing the brand mark while
//! `setup()` prepares the AppData dir and SQLite, and the main webview boots.
//!
//! The main window is declared `"visible": false` in `tauri.conf.json` /
//! `tauri.dev.conf.json`, so something *must* call [`finish`] or the app would never
//! become visible at all. Three callers cover that: the frontend once it is ready
//! (the `splash_finish` command), the startup-failure path in `lib.rs`, and the
//! watchdog below as a last resort.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const SPLASH_LABEL: &str = "splash";
const MAIN_LABEL: &str = "main";

/// Upper bound on how long the mark may stay on screen. Past this the main window is
/// revealed regardless, so a frontend that never signals readiness (broken bundle, JS
/// exception before mount) cannot leave the user staring at a logo with no way in.
const WATCHDOG: Duration = Duration::from_secs(20);

/// Lower bound, and the reason it exists: on a warm start the whole boot takes ~600 ms,
/// of which the splash webview spends the first few hundred painting. Closing on the
/// readiness signal alone turns the mark into a flicker — or into a blank flash, if the
/// window is torn down before its first paint. Holding it for this long makes the
/// startup read as deliberate. It is a floor, not a delay: boots slower than this pay
/// nothing extra.
///
/// The value is the splash.html animation budget: a 3000 ms rise from transparent to
/// solid, then a 2000 ms hold at full opacity. Shortening this cuts the hold first and
/// then truncates the rise, so change both together.
const MIN_VISIBLE: Duration = Duration::from_millis(5000);

static OPENED_AT: OnceLock<Instant> = OnceLock::new();

/// Open the startup window. Never fatal: if it cannot be created the main window is
/// revealed right away, which is exactly the pre-splash behaviour.
pub fn open(app: &AppHandle) {
    let mut builder =
        WebviewWindowBuilder::new(app, SPLASH_LABEL, WebviewUrl::App("splash.html".into()))
            .title("EntropIA")
            .inner_size(360.0, 360.0)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .center()
            .focused(true);

    // macOS rejects transparent windows unless the app opts into the private API,
    // which would compromise notarisation — degrade to an opaque window there.
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.transparent(true);
    }

    // A window shadow around an undecorated transparent window paints as a grey
    // rectangle on Windows.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        builder = builder.shadow(false);
    }

    // The startup window is transient; it should not claim its own taskbar button
    // next to the real one.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        builder = builder.skip_taskbar(true);
    }

    match builder.build() {
        Ok(_) => {
            let _ = OPENED_AT.set(Instant::now());
        }
        Err(error) => {
            eprintln!("[splash] could not open the startup window: {error}");
            reveal_main(app);
        }
    }
}

/// Hand over to the main window, honouring [`MIN_VISIBLE`]. Returns immediately: when
/// the floor has not elapsed yet the handover is deferred to a task, so the caller's
/// thread is never parked.
pub fn finish(app: &AppHandle) {
    let remaining = OPENED_AT
        .get()
        .map(|opened| MIN_VISIBLE.saturating_sub(opened.elapsed()))
        .unwrap_or_default();

    if remaining.is_zero() {
        finish_now(app);
        return;
    }

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(remaining).await;
        finish_now(&handle);
    });
}

/// Close the startup window (if still open) and reveal the main one, with no minimum
/// on-screen time. Idempotent, so the frontend, the failure path and the watchdog can
/// all call it without coordinating. Use this — not [`finish`] — whenever something is
/// about to be shown on top, since the startup window is always-on-top.
pub fn finish_now(app: &AppHandle) {
    if let Some(splash) = app.get_webview_window(SPLASH_LABEL) {
        let _ = splash.close();
    }
    reveal_main(app);
}

fn reveal_main(app: &AppHandle) {
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

/// Arm the reveal-anyway timer described on [`WATCHDOG`].
pub fn start_watchdog(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(WATCHDOG).await;
        if handle.get_webview_window(SPLASH_LABEL).is_some() {
            eprintln!(
                "[splash] the frontend never signalled readiness after {}s — revealing the main window",
                WATCHDOG.as_secs()
            );
            finish_now(&handle);
        }
    });
}

/// Called by the frontend once initialization settled (either ready or failed).
#[tauri::command]
pub fn splash_finish(app: AppHandle) {
    finish(&app);
}
