//! The browser as a native child webview of the main window (feature
//! `navegador`, which turns on Tauri's `unstable` for `Window::add_child`).
//!
//! Isolation, in order of what carries the weight:
//! - The child webview's label has no capability, so the ACL rejects every
//!   command and plugin call from it (tests/app_acl.rs).
//! - It is created incognito: a fresh, non-persistent profile with no cookies
//!   from anywhere else.
//! - Every navigation, redirect, popup and download passes through
//!   [`url_policy`] or is refused.
//! - State reaches the main webview with `emit_to("main", ..)`, never a
//!   broadcast, so the page's webview is never a target of app events.
//!
//! Choices worth knowing:
//! - A popup (`window.open`, `target=_blank`) is never opened as a window; if
//!   its address passes the policy it loads in this same webview.
//! - Downloads are refused until capture handles them.
//! - The typed URL is remembered so that the initial load of a typed `http`
//!   address, which arrives as a plain navigation, is let through; a redirect
//!   from it to `http` elsewhere is not.
//! - Back and forward run `history.back()` / `history.forward()` in the page:
//!   Tauri exposes no `can_go_back`, so the view cannot grey the buttons.
//! - While this webview exists `Manager::get_webview_window("main")` returns
//!   `None` (Tauri stops treating a window with a child webview as a plain
//!   webview window), so [`shutdown`] closes it before the app destroys the
//!   main window.

use std::sync::{Arc, Mutex};

use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, Url, Webview, WebviewUrl,
};

use super::bounds::Bounds;
use super::url_policy::{self, NavigationKind};
use super::{ViewerState, STATE_EVENT, WEBVIEW_LABEL};

pub const AVAILABLE: bool = true;

const MAIN_LABEL: &str = "main";

/// What the callbacks and the commands both need to see.
#[derive(Default)]
struct Shared {
    /// The last address the person typed.
    typed: Mutex<Option<Url>>,
    info: Mutex<ViewerState>,
}

/// Managed state wrapper, created on first open.
struct Viewer(Arc<Shared>);

impl Shared {
    fn update(&self, change: impl FnOnce(&mut ViewerState)) -> ViewerState {
        let mut info = self.info.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut info);
        info.clone()
    }

    fn remember_typed(&self, url: &Url) {
        *self.typed.lock().unwrap_or_else(|e| e.into_inner()) = Some(url.clone());
    }

    /// How the policy should read a navigation to `url`.
    fn kind_for(&self, url: &Url) -> NavigationKind {
        let typed = self.typed.lock().unwrap_or_else(|e| e.into_inner());
        if typed.as_ref() == Some(url) {
            NavigationKind::Typed
        } else {
            NavigationKind::Navigation
        }
    }
}

fn emit_state(app: &AppHandle, state: &ViewerState) {
    // Only the main webview: the page's own webview must never be a target.
    let _ = app.emit_to(MAIN_LABEL, STATE_EVENT, state);
}

fn shared(app: &AppHandle) -> Arc<Shared> {
    // `manage` keeps the first value when one exists.
    app.manage(Viewer(Arc::default()));
    app.state::<Viewer>().0.clone()
}

fn webview(app: &AppHandle) -> Result<Webview, String> {
    app.get_webview(WEBVIEW_LABEL)
        .ok_or_else(|| "The browser is not open".to_string())
}

fn to_rect(bounds: Bounds) -> Rect {
    Rect {
        position: LogicalPosition::new(bounds.x, bounds.y).into(),
        size: LogicalSize::new(bounds.width, bounds.height).into(),
    }
}

pub fn open(app: &AppHandle, url: Url, bounds: Bounds) -> Result<ViewerState, String> {
    if app.get_webview(WEBVIEW_LABEL).is_some() {
        set_bounds(app, bounds)?;
        set_visible(app, true)?;
        return navigate(app, url);
    }
    let window = app
        .get_window(MAIN_LABEL)
        .ok_or_else(|| "The main window is not available".to_string())?;
    let shared = shared(app);
    shared.remember_typed(&url);
    let state = shared.update(|s| {
        *s = ViewerState {
            url: Some(url.to_string()),
            ..ViewerState::default()
        }
    });

    let on_navigation = {
        let (app, shared) = (app.clone(), shared.clone());
        move |target: &Url| match url_policy::check_url(target, shared.kind_for(target)) {
            Ok(()) => {
                let state = shared.update(|s| {
                    s.url = Some(target.to_string());
                    s.blocked = None;
                });
                emit_state(&app, &state);
                true
            }
            Err(reason) => {
                let state = shared.update(|s| s.blocked = Some(reason.to_string()));
                emit_state(&app, &state);
                false
            }
        }
    };
    let on_new_window = {
        let (app, shared) = (app.clone(), shared.clone());
        move |target: Url, _features| {
            match url_policy::check_url(&target, NavigationKind::NewWindow) {
                Ok(()) => {
                    if let Some(view) = app.get_webview(WEBVIEW_LABEL) {
                        let _ = view.navigate(target);
                    }
                }
                Err(reason) => {
                    let state = shared.update(|s| s.blocked = Some(reason.to_string()));
                    emit_state(&app, &state);
                }
            }
            NewWindowResponse::Deny
        }
    };
    let on_download = {
        let (app, shared) = (app.clone(), shared.clone());
        move |_view: Webview, event: DownloadEvent<'_>| {
            if matches!(event, DownloadEvent::Requested { .. }) {
                let state = shared
                    .update(|s| s.blocked = Some("Downloads are not available yet".to_string()));
                emit_state(&app, &state);
            }
            false
        }
    };
    let on_title = {
        let (app, shared) = (app.clone(), shared.clone());
        move |_view: Webview, title: String| {
            let state = shared.update(|s| s.title = Some(title));
            emit_state(&app, &state);
        }
    };
    let on_load = {
        let (app, shared) = (app.clone(), shared);
        move |_view: Webview, payload: tauri::webview::PageLoadPayload<'_>| {
            if payload.event() == PageLoadEvent::Finished {
                let state = shared.update(|s| s.url = Some(payload.url().to_string()));
                emit_state(&app, &state);
            }
        }
    };

    let builder = WebviewBuilder::new(WEBVIEW_LABEL, WebviewUrl::External(url))
        .incognito(true)
        .on_navigation(on_navigation)
        .on_new_window(on_new_window)
        .on_download(on_download)
        .on_document_title_changed(on_title)
        .on_page_load(on_load);
    let rect = to_rect(bounds);
    window
        .add_child(builder, rect.position, rect.size)
        .map_err(|e| e.to_string())?;
    Ok(state)
}

pub fn navigate(app: &AppHandle, url: Url) -> Result<ViewerState, String> {
    let view = webview(app)?;
    let shared = shared(app);
    shared.remember_typed(&url);
    let state = shared.update(|s| {
        s.url = Some(url.to_string());
        s.blocked = None;
    });
    view.navigate(url).map_err(|e| e.to_string())?;
    Ok(state)
}

pub fn back(app: &AppHandle) -> Result<(), String> {
    webview(app)?
        .eval("history.back()")
        .map_err(|e| e.to_string())
}

pub fn forward(app: &AppHandle) -> Result<(), String> {
    webview(app)?
        .eval("history.forward()")
        .map_err(|e| e.to_string())
}

pub fn reload(app: &AppHandle) -> Result<(), String> {
    webview(app)?.reload().map_err(|e| e.to_string())
}

pub fn set_bounds(app: &AppHandle, bounds: Bounds) -> Result<(), String> {
    webview(app)?
        .set_bounds(to_rect(bounds))
        .map_err(|e| e.to_string())
}

pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    let view = webview(app)?;
    if visible { view.show() } else { view.hide() }.map_err(|e| e.to_string())
}

pub fn close(app: &AppHandle) -> Result<(), String> {
    let view = webview(app)?;
    let shared = shared(app);
    *shared.typed.lock().unwrap_or_else(|e| e.into_inner()) = None;
    shared.update(|s| *s = ViewerState::default());
    view.close().map_err(|e| e.to_string())
}

pub fn state(app: &AppHandle) -> Result<ViewerState, String> {
    Ok(shared(app).update(|_| {}))
}

/// Close the browser if it is open, before the main window is destroyed.
pub fn shutdown(app: &AppHandle) {
    if let Some(view) = app.get_webview(WEBVIEW_LABEL) {
        let _ = view.close();
    }
}
