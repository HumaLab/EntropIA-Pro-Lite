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
//! - A popup (`window.open`, `target=_blank`) whose address passes the policy
//!   opens as a real, separate window (`navegador-popup-<n>`, see [`popup`]),
//!   whose webview is the one the engine asked for, so `window.opener` and
//!   `postMessage` back to the page work (Google Sign-In in popup mode needs
//!   that). It is incognito like the browser, so a sign-in made in it is the
//!   browser's, it has no capability, it goes through the same navigation
//!   policy and download logic, it can open popups of its own within the same
//!   limit (3 in all), `window.close()` closes it, and closing the browser (or
//!   the app) closes every popup. Only when the popup cannot be created (the
//!   engine or the OS refused) does the address load in the browser's own
//!   webview, as before; a popup refused for the limit is just refused.
//! - A download is let through only if its address passes the policy; it lands
//!   in a quarantine directory under the cache directory, is checked when it
//!   ends and is reported to the main webview as a draft (see [`download`]).
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
use std::time::Duration;

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::webview::{
    DownloadEvent, NewWindowFeatures, NewWindowResponse, PageLoadEvent, WebviewBuilder,
};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, Url, Webview, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, Wry,
};

use super::bounds::Bounds;
use super::capture::{self, code, CaptureDraft, CaptureError, CaptureKind};
use super::download::{self, DownloadDraft};
use super::popup;
use super::url_policy::{self, NavigationKind};
use super::{ViewerState, DOWNLOAD_EVENT, STATE_EVENT, WEBVIEW_LABEL};

pub const AVAILABLE: bool = true;

const MAIN_LABEL: &str = "main";

/// How long the page gets to answer the capture script.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the callbacks and the commands both need to see.
#[derive(Default)]
struct Shared {
    /// The last address the person typed.
    typed: Mutex<Option<Url>>,
    info: Mutex<ViewerState>,
    /// Downloads let through and not yet ended.
    downloads: download::Registry,
    /// Popups opened so far; numbers are never reused.
    popups_opened: AtomicU32,
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

fn emit_download(app: &AppHandle, draft: &DownloadDraft) {
    let _ = app.emit_to(MAIN_LABEL, DOWNLOAD_EVENT, draft);
}

/// The webview asked to download `url`. Returning `true` lets it start, at the
/// quarantine path written into `destination`.
fn download_requested(
    app: &AppHandle,
    shared: &Shared,
    url: Url,
    destination: &mut std::path::PathBuf,
) -> bool {
    let refuse = |why: &'static str| {
        emit_download(app, &DownloadDraft::refused(&url, destination, why));
        false
    };
    if let Err(blocked) = url_policy::check_url(&url, shared.kind_for(&url)) {
        let state = shared.update(|s| s.blocked = Some(blocked.to_string()));
        emit_state(app, &state);
        return refuse(download::reason::BLOCKED);
    }
    let dir = match crate::path_utils::cache_dir(app) {
        Ok(cache) => download::quarantine_dir(&cache),
        Err(_) => return refuse(download::reason::IO_ERROR),
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return refuse(download::reason::IO_ERROR);
    }
    match shared.downloads.begin(&url, destination) {
        Ok(pending) => {
            *destination = download::part_path(&dir, &pending.id);
            emit_download(app, &DownloadDraft::started(&pending));
            true
        }
        Err(why) => refuse(why),
    }
}

/// A download ended. Only downloads this module let through are looked at, and
/// only through the path this module chose for them.
fn download_finished(
    app: &AppHandle,
    shared: &Shared,
    url: Url,
    path: Option<std::path::PathBuf>,
    success: bool,
) {
    // A failed or cancelled download reports no path: match it by address.
    let pending = path
        .as_deref()
        .and_then(|p| shared.downloads.take_by_path(p))
        .or_else(|| shared.downloads.take_by_url(&url));
    let Some(pending) = pending else { return };
    let Ok(cache) = crate::path_utils::cache_dir(app) else {
        emit_download(
            app,
            &DownloadDraft::finished(&pending, Err(download::reason::IO_ERROR)),
        );
        return;
    };
    let dir = download::quarantine_dir(&cache);
    if !success {
        download::discard_part(&dir, &pending.id);
        emit_download(
            app,
            &DownloadDraft::finished(&pending, Err(download::reason::INTERRUPTED)),
        );
        return;
    }
    // Hashing up to 100 MB does not belong on the thread the webview calls on.
    let app = app.clone();
    std::thread::spawn(move || {
        let outcome = download::finalize(&dir, &pending.id);
        emit_download(&app, &DownloadDraft::finished(&pending, outcome));
    });
}

/// Whether the browser may go to `target`, and what the main view is told. A
/// navigation of the browser itself moves its address; one inside a popup only
/// reports why it was refused, so a popup never rewrites the address bar.
fn navigation_allowed(app: &AppHandle, shared: &Shared, target: &Url, in_popup: bool) -> bool {
    match url_policy::check_url(target, shared.kind_for(target)) {
        Ok(()) => {
            if !in_popup {
                let state = shared.update(|s| {
                    s.url = Some(target.to_string());
                    s.blocked = None;
                });
                emit_state(app, &state);
            }
            true
        }
        Err(reason) => {
            let state = shared.update(|s| s.blocked = Some(reason.to_string()));
            emit_state(app, &state);
            false
        }
    }
}

fn download_handler(
    app: AppHandle,
    shared: Arc<Shared>,
) -> impl Fn(Webview, DownloadEvent<'_>) -> bool + Send + Sync + 'static {
    move |_view: Webview, event: DownloadEvent<'_>| match event {
        DownloadEvent::Requested { url, destination } => {
            download_requested(&app, &shared, url, destination)
        }
        DownloadEvent::Finished { url, path, success } => {
            download_finished(&app, &shared, url, path, success);
            true
        }
        // `DownloadEvent` is non-exhaustive: an event this code does not
        // know is not a download it agreed to.
        _ => false,
    }
}

/// How many popups are open right now.
fn popups_open(app: &AppHandle) -> usize {
    popup::count_open(app.webview_windows().keys().map(String::as_str))
}

/// Close every popup window. Runs when the browser closes and when the app
/// exits, so no popup outlives the page that opened it.
fn close_popups(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if popup::is_popup_label(&label) {
            let _ = window.destroy();
        }
    }
}

/// The page asked for a new window. An address the policy refuses is refused;
/// past the popup limit it is refused too. Otherwise a real popup window is
/// created and handed to the engine, which loads the requested address into it
/// and links it to its opener. If that cannot be done the address loads in the
/// browser's own webview instead (the opener link is lost, the page still opens).
fn new_window(
    app: &AppHandle,
    shared: &Arc<Shared>,
    target: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    if let Err(reason) = url_policy::check_url(&target, NavigationKind::NewWindow) {
        let state = shared.update(|s| s.blocked = Some(reason.to_string()));
        emit_state(app, &state);
        return NewWindowResponse::Deny;
    }
    if !popup::has_room(popups_open(app)) {
        let state = shared.update(|s| s.blocked = Some(popup::TOO_MANY_MESSAGE.to_string()));
        emit_state(app, &state);
        return NewWindowResponse::Deny;
    }
    match open_popup(app, shared, features) {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => {
            if let Some(view) = app.get_webview(WEBVIEW_LABEL) {
                let _ = view.navigate(target);
            }
            NewWindowResponse::Deny
        }
    }
}

/// Build the popup window. It starts on `about:blank`: the engine navigates it
/// to the address the page asked for once it is linked to its opener.
fn open_popup(
    app: &AppHandle,
    shared: &Arc<Shared>,
    features: NewWindowFeatures,
) -> tauri::Result<WebviewWindow<Wry>> {
    let n = shared.popups_opened.fetch_add(1, Ordering::Relaxed) + 1;
    let label = popup::label(n);
    let blank: Url = "about:blank".parse().expect("a constant address");
    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(blank))
        // The features (size, position) win when the page gave any; and they
        // carry the opener's environment, which the engine requires to link them.
        .inner_size(900.0, 700.0)
        .window_features(features)
        // Same profile as the browser: a private one, shared with its opener.
        .incognito(true)
        .on_navigation({
            let (app, shared) = (app.clone(), shared.clone());
            move |target: &Url| navigation_allowed(&app, &shared, target, true)
        })
        .on_new_window({
            let (app, shared) = (app.clone(), shared.clone());
            move |target: Url, features| new_window(&app, &shared, target, features)
        })
        .on_download(download_handler(app.clone(), shared.clone()))
        .on_document_title_changed(|window, title| {
            let _ = window.set_title(&title);
        })
        .build()
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
        move |target: &Url| navigation_allowed(&app, &shared, target, false)
    };
    let on_new_window = {
        let (app, shared) = (app.clone(), shared.clone());
        move |target: Url, features: NewWindowFeatures| new_window(&app, &shared, target, features)
    };
    let on_download = download_handler(app.clone(), shared.clone());
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
    close_popups(app);
    let view = webview(app)?;
    let shared = shared(app);
    *shared.typed.lock().unwrap_or_else(|e| e.into_inner()) = None;
    shared.update(|s| *s = ViewerState::default());
    view.close().map_err(|e| e.to_string())
}

pub fn state(app: &AppHandle) -> Result<ViewerState, String> {
    Ok(shared(app).update(|_| {}))
}

/// Run the capture script in the page and validate what comes back.
///
/// The script goes through the platform's script evaluation
/// (`Webview::eval_with_callback`), not the page's IPC: the page never sees a
/// command and needs no capability. The callback hands the result, serialised
/// as JSON, to a one-shot channel so this can stay an async command; if the
/// page does not answer in [`CAPTURE_TIMEOUT`] (a hung script, a page
/// mid-navigation whose document is replaced) the capture fails instead of
/// waiting forever.
pub async fn capture(app: &AppHandle, kind: CaptureKind) -> Result<CaptureDraft, CaptureError> {
    let view = app
        .get_webview(WEBVIEW_LABEL)
        .ok_or_else(|| CaptureError::new(code::NOT_OPEN))?;
    let shared = shared(app);

    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Mutex::new(Some(tx));
    view.eval_with_callback(capture::script_for(kind), move |result| {
        let sender = tx.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
    })
    .map_err(|e| CaptureError::with_detail(code::SCRIPT_FAILED, e.to_string()))?;

    let raw = match tokio::time::timeout(CAPTURE_TIMEOUT, rx).await {
        Ok(Ok(raw)) => raw,
        Ok(Err(_)) => return Err(CaptureError::new(code::SCRIPT_FAILED)),
        Err(_) => return Err(CaptureError::new(code::TIMEOUT)),
    };
    capture::parse_capture(&raw, kind, capture::now_rfc3339(), |url| {
        shared.kind_for(url)
    })
}

/// Close the browser if it is open, before the main window is destroyed.
pub fn shutdown(app: &AppHandle) {
    close_popups(app);
    if let Some(view) = app.get_webview(WEBVIEW_LABEL) {
        let _ = view.close();
    }
}
