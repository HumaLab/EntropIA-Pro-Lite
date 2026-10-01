//! The embedded browser ("Navegador"). Web content is untrusted: nothing in
//! this module hands it IPC, and every address it may load goes through
//! [`url_policy`] first. Each tab is its own child webview (see [`tabs`]).
//!
//! The commands are always compiled and registered, so the ACL manifest is the
//! same in every build. The child webview itself needs Tauri's `unstable`
//! feature and lives behind the `navegador` Cargo feature; without it every
//! command answers with [`UNAVAILABLE`].

pub mod bounds;
// Parsing is compiled in every build; only the viewer that feeds it needs the
// `navegador` feature.
#[allow(dead_code)]
pub mod capture;
// Where saved captures live on disk and the one safe way into that tree.
#[allow(dead_code)]
pub mod capture_files;
pub mod commands;
#[allow(dead_code)]
pub mod download;
// Naming and limits are pure; only the viewer that opens the windows needs the
// `navegador` feature.
#[allow(dead_code)]
pub mod popup;
// Saving what was captured: files first, then one database transaction. Pure
// over a data directory and a connection, so every build compiles it.
#[allow(dead_code)]
pub mod save;
// Reading and deleting the saved web sources (Rust owns both tables).
#[allow(dead_code)]
pub mod sources;
// Rendering captured text into a PDF for a copy into a collection.
#[allow(dead_code)]
pub mod text_pdf;
// The startup sweep of the capture folder.
#[allow(dead_code)]
pub mod sweep;
// Labels, the limit and the tab list are pure; the viewer that builds the
// webviews needs the `navegador` feature.
#[allow(dead_code)]
pub mod tabs;
// The navigation callbacks that use the rest of the policy exist only when the
// `navegador` feature is on.
#[allow(dead_code)]
pub mod url_policy;

#[cfg(feature = "navegador")]
mod viewer;
#[cfg(not(feature = "navegador"))]
#[path = "viewer_unavailable.rs"]
mod viewer;

pub use viewer::shutdown;

/// Event the main webview listens to for changes in the browser state (its tabs
/// and which one is active).
#[cfg(feature = "navegador")]
pub const STATE_EVENT: &str = "navegador://state";

/// Event the main webview listens to for downloads in quarantine.
#[cfg(feature = "navegador")]
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
        eprintln!("[navegador] {line}");
        crate::app_logs::info(&app, "navegador", line);
    });
}

/// What every command answers when the build has no browser.
pub const UNAVAILABLE: &str = "The embedded browser is not available in this build";
