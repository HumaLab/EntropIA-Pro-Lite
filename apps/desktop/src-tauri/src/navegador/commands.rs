//! The commands the Navegador view calls. They run only for the `main`
//! webview (see `capabilities/default.json`); the child webview that shows
//! remote content has no capability at all.
//!
//! They are `async` on purpose: creating a webview blocks until the main thread
//! has built it, so running that on the main thread (where synchronous
//! commands run) would deadlock on Windows.

use tauri::AppHandle;

use super::url_policy::{self, NavigationKind};
use super::{bounds, viewer, ViewerState, UNAVAILABLE};

fn ensure_available() -> Result<(), String> {
    if viewer::AVAILABLE {
        Ok(())
    } else {
        Err(UNAVAILABLE.to_string())
    }
}

/// Everything typed in the address bar takes this way in.
fn typed(input: &str) -> Result<tauri::Url, String> {
    url_policy::check(input, NavigationKind::Typed).map_err(|reason| reason.to_string())
}

/// Create the browser at the given rectangle (logical pixels, relative to the
/// window) and load `url`. Reuses the browser when one is already open.
#[tauri::command]
pub async fn navegador_open(
    app: AppHandle,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<ViewerState, String> {
    ensure_available()?;
    let url = typed(&url)?;
    let bounds = bounds::sanitize(x, y, width, height)?;
    viewer::open(&app, url, bounds)
}

#[tauri::command]
pub async fn navegador_navigate(app: AppHandle, url: String) -> Result<ViewerState, String> {
    ensure_available()?;
    viewer::navigate(&app, typed(&url)?)
}

#[tauri::command]
pub async fn navegador_back(app: AppHandle) -> Result<(), String> {
    ensure_available()?;
    viewer::back(&app)
}

#[tauri::command]
pub async fn navegador_forward(app: AppHandle) -> Result<(), String> {
    ensure_available()?;
    viewer::forward(&app)
}

#[tauri::command]
pub async fn navegador_reload(app: AppHandle) -> Result<(), String> {
    ensure_available()?;
    viewer::reload(&app)
}

#[tauri::command]
pub async fn navegador_set_bounds(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    ensure_available()?;
    viewer::set_bounds(&app, bounds::sanitize(x, y, width, height)?)
}

#[tauri::command]
pub async fn navegador_set_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    ensure_available()?;
    viewer::set_visible(&app, visible)
}

#[tauri::command]
pub async fn navegador_close(app: AppHandle) -> Result<(), String> {
    ensure_available()?;
    viewer::close(&app)
}

#[tauri::command]
pub async fn navegador_state(app: AppHandle) -> Result<ViewerState, String> {
    ensure_available()?;
    viewer::state(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_answer_unavailable_exactly_when_the_feature_is_off() {
        match ensure_available() {
            Ok(()) => assert!(cfg!(feature = "navegador")),
            Err(message) => {
                assert!(!cfg!(feature = "navegador"));
                assert_eq!(message, UNAVAILABLE);
            }
        }
    }

    #[test]
    fn typed_input_goes_through_the_url_policy() {
        assert_eq!(
            typed("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert!(typed("javascript:alert(1)")
            .unwrap_err()
            .contains("javascript"));
        assert!(typed("http://169.254.169.254/")
            .unwrap_err()
            .contains("public internet"));
        assert!(typed("  ").unwrap_err().contains("empty"));
    }
}
