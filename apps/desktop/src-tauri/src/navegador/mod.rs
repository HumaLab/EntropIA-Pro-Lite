//! The embedded browser ("Navegador"). Web content is untrusted: nothing in
//! this module hands it IPC, and every address it may load goes through
//! [`url_policy`] first. Each tab is its own child webview (see [`tabs`]).
//!
//! It ships in every build (Pro and Lite, every platform). The child webview
//! needs Tauri's `unstable` feature, which `Cargo.toml` turns on for `tauri`
//! unconditionally; there is no switch that leaves the commands without a
//! browser.

pub mod bounds;
// Parsing what the viewer captured.
#[allow(dead_code)]
pub mod capture;
// Where saved captures live on disk and the one safe way into that tree.
#[allow(dead_code)]
pub mod capture_files;
pub mod commands;
#[allow(dead_code)]
pub mod download;
// Naming and limits of the popup windows the viewer opens.
#[allow(dead_code)]
pub mod popup;
// Saving what was captured: files first, then one database transaction. Pure
// over a data directory and a connection, so every build compiles it.
#[allow(dead_code)]
pub mod save;
// Reading and deleting the saved web sources (Rust owns both tables).
#[allow(dead_code)]
pub mod sources;
// Copying a page or a selection: its text rendered into a PDF.
#[allow(dead_code)]
pub mod text_copy;
// Rendering captured text into a PDF for a copy into a collection.
#[allow(dead_code)]
pub mod text_pdf;
// The startup sweep of the capture folder.
#[allow(dead_code)]
pub mod sweep;
// Labels, the limit and the tab list; the viewer builds the webviews.
#[allow(dead_code)]
pub mod tabs;
// Copying a saved source to Zotero: a durable queue and the connector writes.
#[allow(dead_code)]
pub mod zotero_copy;
// Which addresses the browser may load, checked on every navigation.
#[allow(dead_code)]
pub mod url_policy;

mod viewer;

pub use viewer::shutdown;

/// Event the main webview listens to for changes in the browser state (its tabs
/// and which one is active).
pub const STATE_EVENT: &str = "navegador://state";

/// Event the main webview listens to for downloads in quarantine.
pub const DOWNLOAD_EVENT: &str = "navegador://download";

/// Remove quarantined downloads that a crash or an abandoned draft left
/// behind. Runs off the calling thread; harmless when nothing exists.
pub fn sweep_quarantine(cache: &std::path::Path) {
    let dir = download::quarantine_dir(cache);
    std::thread::spawn(move || {
        download::sweep_stale(&dir, std::time::SystemTime::now(), download::STALE_AFTER);
    });
}

/// Sweep `<data>/web-captures/` for what a crash or a failed delete left
/// behind (see [`sweep`]). Runs on its own thread, logs one line, and can fail
/// in any way without touching startup or the archive.
pub fn sweep_captures(
    app: tauri::AppHandle,
    data_dir: std::path::PathBuf,
    db_path: std::path::PathBuf,
) {
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match crate::db::open::open_archive_connection(&db_path) {
                Ok(conn) => sweep::sweep(
                    &data_dir,
                    &conn,
                    std::time::SystemTime::now(),
                    sweep::MIN_AGE,
                )
                .summary(),
                Err(error) => format!("web-captures sweep skipped: {error}"),
            }
        }));
        let line = outcome.unwrap_or_else(|_| "web-captures sweep stopped by an error".to_string());
        crate::app_logs::info(&app, "navegador", line);
    });
}
