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

/// What every command answers when the build has no browser.
pub const UNAVAILABLE: &str = "The embedded browser is not available in this build";
