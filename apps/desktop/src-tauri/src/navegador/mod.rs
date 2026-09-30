//! The embedded browser ("Navegador"). Web content is untrusted: nothing in
//! this module hands it IPC, and every address it may load goes through
//! [`url_policy`] first.
//!
//! The commands are always compiled and registered, so the ACL manifest is the
//! same in every build. The child webview itself needs Tauri's `unstable`
//! feature and lives behind the `navegador` Cargo feature; without it every
//! command answers with [`UNAVAILABLE`].

use serde::Serialize;

pub mod bounds;
// Parsing is compiled in every build; only the viewer that feeds it needs the
// `navegador` feature.
#[allow(dead_code)]
pub mod capture;
pub mod commands;
#[allow(dead_code)]
pub mod download;
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

/// The label of the child webview that shows remote content. It has no
/// capability, so it can call no command.
#[cfg(feature = "navegador")]
pub const WEBVIEW_LABEL: &str = "navegador-web";

/// Event the main webview listens to for changes in the browser state.
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

/// What the frontend shows about the page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ViewerState {
    /// The address the page is at, or is going to.
    pub url: Option<String>,
    pub title: Option<String>,
    /// Why the last navigation was refused, until the next one that works.
    pub blocked: Option<String>,
}
