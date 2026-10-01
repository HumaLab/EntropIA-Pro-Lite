//! The browser as native child webviews of the main window, one per tab
//! (feature `navegador`, which turns on Tauri's `unstable` for
//! `Window::add_child`).
//!
//! Isolation, in order of what carries the weight:
//! - A tab's label (`navegador-web-<n>`, see [`tabs`]) has no capability, so the
//!   ACL rejects every command and plugin call from it (tests/app_acl.rs).
//! - Every tab is created incognito: a fresh, non-persistent profile with no
//!   cookies from anywhere else.
//! - Every navigation, redirect, new window and download passes through
//!   [`url_policy`] or is refused.
//! - State reaches the main webview with `emit_to("main", ..)`, never a
//!   broadcast, so a page's webview is never a target of app events.
//!
//! Tabs:
//! - Each tab is its own webview, kept alive and hidden while another tab is on
//!   screen; only the active tab is visible, and only while the browser itself
//!   is shown (the Navegador section is on screen). [`sync_views`] makes the
//!   webviews agree with that after every change. A tab that has not been given
//!   a page yet (`url` is `None`) has no webview: it is made on the first
//!   navigation.
//! - This module owns which tab is active (a page can open a tab while the
//!   person looks at another one), but every command that acts on a page takes
//!   the tab it means, so what the person saw when they clicked is what is
//!   acted on, never whatever became active in between.
//! - A page's request for a new window becomes a new tab when it is a plain
//!   link or a `window.open` without size or position (see [`tabs::placement`]);
//!   the tab is in the foreground only when the page that asked was the one on
//!   screen. The opener link (`window.opener`) cannot be kept for a tab: a tab
//!   is a child webview, and the engine only links a new window that is a real
//!   top-level window. So a script that opens a window to talk to it (Google
//!   Sign-In asks for a size) gets a popup instead.
//! - A popup (`navegador-popup-<n>`, see [`popup`]) is a real, separate window
//!   whose webview is the one the engine asked for, so `window.opener` and
//!   `postMessage` back to the page work. It is incognito like the tabs (a
//!   sign-in made in it is the browser's), has no capability, goes through the
//!   same navigation policy and download logic, can open popups of its own
//!   within the same limit (3 in all), `window.close()` closes it, and closing
//!   the browser (or the app) closes every popup. Only when the popup cannot be
//!   created does the address load in a tab's own webview; a popup refused for
//!   the limit is just refused.
//! - A download is let through only if its address passes the policy. A PDF
//!   (or a file whose name does not say what it is) lands in a quarantine
//!   directory under the cache directory, is checked when it ends and is
//!   reported to the main webview as a draft; anything else is written straight
//!   to the person's download folder, which EntropIA never opens (see
//!   [`download`]). Downloads belong to the browser, not to a tab: a draft says
//!   which tab started it, and closing that tab does not cancel it.
//! - The typed URL is remembered per tab so that the initial load of a typed
//!   `http` address, which arrives as a plain navigation, is let through; a
//!   redirect from it to `http` elsewhere is not.
//! - Back and forward run `history.back()` / `history.forward()` in the page:
//!   Tauri exposes no `can_go_back`, so the view cannot grey the buttons.
//! - While a tab's webview exists `Manager::get_webview_window("main")` returns
//!   `None` (Tauri stops treating a window with a child webview as a plain
//!   webview window), so [`shutdown`] closes them before the app destroys the
//!   main window.
//! - Locks: the state lock is never held across a call that waits for the main
//!   thread (building or closing a webview), because the webviews' callbacks
//!   run there and take it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

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
use super::tabs::{self, BrowserState, Placement, Tab, TabList};
use super::url_policy::{self, NavigationKind};
use super::{DOWNLOAD_EVENT, STATE_EVENT};

pub const AVAILABLE: bool = true;

const MAIN_LABEL: &str = "main";

/// How long the page gets to answer the capture script.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether the engine tells the size and position of a requested window only
/// when the page gave them. WebView2 and WKWebView do; wry's WebKitGTK always
/// reports none, so there every request looks like a link (see
/// [`tabs::placement`]).
const FEATURES_REPORTED: bool = !cfg!(target_os = "linux");

/// Shown when a tab's webview could not be made for a page's request.
const TAB_FAILED_MESSAGE: &str = "The new tab could not be opened";

const NO_SUCH_TAB: &str = "There is no such tab";

/// What the browser is: its tabs and where they go on screen.
#[derive(Default)]
struct Browser {
    tabs: TabList,
    /// Per tab, the last address the person typed.
    typed: HashMap<u32, Url>,
    /// Where the active tab goes: the last rect the view sent.
    bounds: Option<Bounds>,
    /// Whether the Navegador section is on screen.
    visible: bool,
}

/// What the callbacks and the commands both need to see.
#[derive(Default)]
struct Shared {
    browser: Mutex<Browser>,
    /// Downloads let through and not yet ended.
    downloads: download::Registry,
    /// Popups opened so far; numbers are never reused.
    popups_opened: AtomicU32,
    /// The folder the person chose for what EntropIA does not keep (`None`:
    /// the system's Downloads folder).
    download_dir: Mutex<Option<std::path::PathBuf>>,
}

/// Managed state wrapper, created on first open.
struct Viewer(Arc<Shared>);

/// Who asked for something: a tab's page, or a popup window's.
#[derive(Clone, Copy)]
enum Origin {
    Tab(u32),
    Popup,
}

impl Origin {
    fn tab(self) -> Option<u32> {
        match self {
            Origin::Tab(id) => Some(id),
            Origin::Popup => None,
        }
    }
}

impl Shared {
    fn browser(&self) -> MutexGuard<'_, Browser> {
        self.browser.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn snapshot(&self) -> BrowserState {
        self.browser().tabs.snapshot()
    }

    /// Change one tab; the new state, or `None` when the tab is gone.
    fn change_tab(&self, id: u32, change: impl FnOnce(&mut Tab)) -> Option<BrowserState> {
        let mut browser = self.browser();
        change(browser.tabs.get_mut(id)?);
        Some(browser.tabs.snapshot())
    }

    /// Note a refusal on `tab`, or on the active tab when the request came from
    /// somewhere that has none (a popup).
    fn block(&self, tab: Option<u32>, reason: impl Into<String>) -> BrowserState {
        let mut browser = self.browser();
        let id = tab.or(browser.tabs.active());
        if let Some(entry) = id.and_then(|id| browser.tabs.get_mut(id)) {
            entry.blocked = Some(reason.into());
        }
        browser.tabs.snapshot()
    }

    /// The address and title of the page in `tab` right now (nothing for a
    /// popup, which is no tab).
    fn page_of(&self, tab: Option<u32>) -> (Option<String>, Option<String>) {
        let browser = self.browser();
        match tab.and_then(|id| browser.tabs.get(id)) {
            Some(entry) => (entry.url.clone(), entry.title.clone()),
            None => (None, None),
        }
    }

    fn download_dir(&self) -> Option<std::path::PathBuf> {
        self.download_dir
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// How the policy should read a navigation to `url` in `tab`: only the
    /// address typed for that tab counts as typed.
    fn kind_for(&self, tab: Option<u32>, url: &Url) -> NavigationKind {
        let browser = self.browser();
        match tab.and_then(|id| browser.typed.get(&id)) {
            Some(typed) if typed == url => NavigationKind::Typed,
            _ => NavigationKind::Navigation,
        }
    }
}

fn emit_state(app: &AppHandle, state: &BrowserState) {
    // Only the main webview: a page's own webview must never be a target.
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
    tab: Option<u32>,
    url: Url,
    destination: &mut std::path::PathBuf,
) -> bool {
    // The page as it is NOW: the tab keeps moving, the download's label must not.
    let (page_url, page_title) = shared.page_of(tab);
    let refuse = |why: &'static str| {
        emit_download(
            app,
            &DownloadDraft::refused(&url, destination, why)
                .with_tab(tab)
                .with_page(page_url.as_deref(), page_title.as_deref()),
        );
        false
    };
    if let Err(blocked) = url_policy::check_url(&url, shared.kind_for(tab, &url)) {
        emit_state(app, &shared.block(tab, blocked.to_string()));
        return refuse(download::reason::BLOCKED);
    }
    let suggested = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if download::route_for(&suggested, &url) == download::Route::UserFolder {
        // Not ours to keep: straight to the person's folder, under a name that
        // overwrites nothing.
        let Some(folder) = user_folder(app, shared) else {
            return refuse(download::reason::IO_ERROR);
        };
        return match shared
            .downloads
            .begin_in_folder(&url, destination, &folder, |path| path.exists())
        {
            Ok(mut pending) => {
                shared.downloads.set_tab(&pending.id, tab);
                shared
                    .downloads
                    .set_page(&pending.id, page_url.as_deref(), page_title.as_deref());
                pending = shared.downloads.peek(&pending.id).unwrap_or(pending);
                if let Some(saved) = &pending.saved {
                    *destination = saved.clone();
                }
                emit_download(app, &DownloadDraft::started(&pending));
                true
            }
            Err(why) => refuse(why),
        };
    }
    let dir = match crate::path_utils::cache_dir(app) {
        Ok(cache) => download::quarantine_dir(&cache),
        Err(_) => return refuse(download::reason::IO_ERROR),
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return refuse(download::reason::IO_ERROR);
    }
    match shared.downloads.begin(&url, destination) {
        Ok(mut pending) => {
            shared.downloads.set_tab(&pending.id, tab);
            shared
                .downloads
                .set_page(&pending.id, page_url.as_deref(), page_title.as_deref());
            pending = shared.downloads.peek(&pending.id).unwrap_or(pending);
            *destination = download::part_path(&dir, &pending.id);
            emit_download(app, &DownloadDraft::started(&pending));
            true
        }
        Err(why) => refuse(why),
    }
}

/// Where a file EntropIA does not keep goes: the folder the person chose while
/// it is still a directory, else the system's Downloads folder.
fn user_folder(app: &AppHandle, shared: &Shared) -> Option<std::path::PathBuf> {
    download::resolve_folder(
        shared.download_dir().as_deref(),
        app.path().download_dir().ok(),
        |path| path.is_dir(),
    )
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
    // A file written to the person's folder is theirs: it is reported, never
    // read, moved or deleted.
    if let Some(saved) = pending.saved.clone() {
        let draft = if success {
            let size = std::fs::metadata(&saved).ok().map(|meta| meta.len());
            DownloadDraft::saved(&pending, &saved, size)
        } else {
            DownloadDraft::finished(&pending, Err(download::reason::INTERRUPTED))
        };
        emit_download(app, &draft);
        return;
    }
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
    // A file that turns out not to be a PDF is moved to the person's folder
    // instead of being deleted.
    let folder = user_folder(app, shared);
    let app = app.clone();
    std::thread::spawn(move || {
        let outcome =
            download::finalize_or_release(&dir, &pending.id, folder.as_deref(), &pending.file_name);
        let draft = match outcome {
            Ok(download::Outcome::Verified(verified)) => {
                let draft = DownloadDraft::finished(&pending, Ok(verified.clone()));
                // The save command finds the PDF by id, never by a path or hash
                // the renderer sends.
                super::save::holds(&app).hold_pdf(super::save::ReadyPdf {
                    id: pending.id.clone(),
                    url: pending.url.to_string(),
                    file_name: draft.file_name.clone(),
                    size: verified.size,
                    sha256: verified.sha256,
                    accessed_at: pending.accessed_at.clone(),
                    page_title: pending.page_title.clone(),
                });
                draft
            }
            Ok(download::Outcome::Saved { path, size }) => {
                DownloadDraft::saved(&pending, &path, Some(size))
            }
            Err(why) => DownloadDraft::finished(&pending, Err(why)),
        };
        emit_download(&app, &draft);
    });
}

/// Whether a webview may go to `target`, and what the main view is told. A
/// navigation of a tab moves that tab's address; one inside a popup only
/// reports why it was refused, so a popup never rewrites the address bar.
fn navigation_allowed(app: &AppHandle, shared: &Shared, origin: Origin, target: &Url) -> bool {
    match url_policy::check_url(target, shared.kind_for(origin.tab(), target)) {
        Ok(()) => {
            if let Origin::Tab(id) = origin {
                let moved = shared.change_tab(id, |tab| {
                    tab.url = Some(target.to_string());
                    tab.blocked = None;
                });
                if let Some(state) = moved {
                    emit_state(app, &state);
                }
            }
            true
        }
        Err(reason) => {
            emit_state(app, &shared.block(origin.tab(), reason.to_string()));
            false
        }
    }
}

fn download_handler(
    app: AppHandle,
    shared: Arc<Shared>,
    tab: Option<u32>,
) -> impl Fn(Webview, DownloadEvent<'_>) -> bool + Send + Sync + 'static {
    move |_view: Webview, event: DownloadEvent<'_>| match event {
        DownloadEvent::Requested { url, destination } => {
            download_requested(&app, &shared, tab, url, destination)
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

/// Close every tab's webview, whatever the tab list says: a label that looks
/// like a tab's is one this module made.
fn close_tab_webviews(app: &AppHandle) {
    for (label, view) in app.webviews() {
        if tabs::is_tab_label(&label) {
            let _ = view.close();
        }
    }
}

/// The page in `origin` asked for a new window. An address the policy refuses
/// is refused. A plain link or a `window.open` without size or position becomes
/// a tab; anything that asks for a size or a position gets a popup window (see
/// [`tabs::placement`]).
fn new_window(
    app: &AppHandle,
    shared: &Arc<Shared>,
    origin: Origin,
    target: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    if let Err(reason) = url_policy::check_url(&target, NavigationKind::NewWindow) {
        emit_state(app, &shared.block(origin.tab(), reason.to_string()));
        return NewWindowResponse::Deny;
    }
    let placement = tabs::placement(
        FEATURES_REPORTED,
        features.size().is_some(),
        features.position().is_some(),
    );
    match placement {
        Placement::Tab => {
            open_page_tab(app, shared, origin.tab(), target);
            // A tab is not a window the engine can link to its opener.
            NewWindowResponse::Deny
        }
        Placement::Popup => open_popup_for(app, shared, origin, target, features),
    }
}

/// Make a tab for a page's request. The tab exists in the list at once (the
/// person sees it appear); its webview is built on another thread, because this
/// runs inside the engine's callback. The tab is in the foreground only when
/// the page that asked is the one on screen: a page in a hidden tab cannot pull
/// the person away from what they are reading.
fn open_page_tab(app: &AppHandle, shared: &Arc<Shared>, opener: Option<u32>, target: Url) {
    let added = {
        let mut browser = shared.browser();
        let foreground = opener.is_some() && opener == browser.tabs.active();
        browser.tabs.add(foreground).ok().map(|id| {
            if let Some(tab) = browser.tabs.get_mut(id) {
                tab.url = Some(target.to_string());
            }
            (id, browser.bounds)
        })
    };
    let Some((id, bounds)) = added else {
        emit_state(app, &shared.block(opener, tabs::TOO_MANY_MESSAGE));
        return;
    };
    emit_state(app, &shared.snapshot());
    let (app, shared) = (app.clone(), shared.clone());
    std::thread::spawn(move || {
        let made = bounds
            .ok_or_else(|| "The browser area is not known".to_string())
            .and_then(|bounds| create_tab_webview(&app, &shared, id, target, bounds));
        if made.is_err() {
            {
                let mut browser = shared.browser();
                browser.typed.remove(&id);
                browser.tabs.close(id);
            }
            emit_state(&app, &shared.block(opener, TAB_FAILED_MESSAGE));
        }
        sync_views(&app, &shared);
        emit_state(&app, &shared.snapshot());
    });
}

/// A popup window for a request that asked for a size or a position. Past the
/// popup limit it is refused. If the window cannot be made the address loads in
/// a tab's own webview instead (the opener link is lost, the page still opens).
fn open_popup_for(
    app: &AppHandle,
    shared: &Arc<Shared>,
    origin: Origin,
    target: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    if !popup::has_room(popups_open(app)) {
        emit_state(app, &shared.block(origin.tab(), popup::TOO_MANY_MESSAGE));
        return NewWindowResponse::Deny;
    }
    match open_popup(app, shared, features) {
        Ok(window) => {
            close_when_page_closes(app, &window);
            NewWindowResponse::Create { window }
        }
        Err(_) => {
            let fallback = origin.tab().or(shared.browser().tabs.active());
            if let Some(view) = fallback.and_then(|id| app.get_webview(&tabs::label(id))) {
                let _ = view.navigate(target);
            }
            NewWindowResponse::Deny
        }
    }
}

/// Close the popup window when its page calls `window.close()` (Google
/// Sign-In does, after the login). Tauri 2.11.6 and wry 0.55.1 have no hook for
/// it on a webview created through `NewWindowResponse::Create`: wry's own
/// `WindowCloseRequested` handler destroys only the webview's container window
/// and leaves the top-level popup window open and empty. So on Windows this
/// subscribes to `ICoreWebView2::add_WindowCloseRequested` and destroys the
/// popup's Tauri window, on the main thread (`with_webview` runs there).
///
/// Elsewhere nothing is wired: wry has no `webViewDidClose:` delegate on macOS,
/// and on Linux its `close` signal destroys the GTK widget, not the window. A
/// popup there is closed by the person, or when the browser or the app closes.
#[cfg(windows)]
fn close_when_page_closes(app: &AppHandle, window: &WebviewWindow<Wry>) {
    use webview2_com::WindowCloseRequestedEventHandler;

    let (app, label) = (app.clone(), window.label().to_string());
    let _ = window.with_webview(move |platform| {
        // SAFETY: COM calls on the UI thread, on the controller Tauri owns for
        // this window; the handler only holds an `AppHandle` and a label.
        unsafe {
            let Ok(core) = platform.controller().CoreWebView2() else {
                return;
            };
            let mut token = 0i64;
            let _ = core.add_WindowCloseRequested(
                &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                    if popup::is_popup_label(&label) {
                        if let Some(window) = app.get_webview_window(&label) {
                            let _ = window.destroy();
                        }
                    }
                    Ok(())
                })),
                &mut token,
            );
        }
    });
}

#[cfg(not(windows))]
fn close_when_page_closes(_app: &AppHandle, _window: &WebviewWindow<Wry>) {}

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
        // Same profile as the tabs: a private one, shared with its opener.
        .incognito(true)
        .on_navigation({
            let (app, shared) = (app.clone(), shared.clone());
            move |target: &Url| navigation_allowed(&app, &shared, Origin::Popup, target)
        })
        .on_new_window({
            let (app, shared) = (app.clone(), shared.clone());
            move |target: Url, features| new_window(&app, &shared, Origin::Popup, target, features)
        })
        .on_download(download_handler(app.clone(), shared.clone(), None))
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

/// The webview of `tab`, which exists once the tab has a page.
fn tab_webview(app: &AppHandle, tab: u32) -> Result<Webview, String> {
    app.get_webview(&tabs::label(tab))
        .ok_or_else(|| "The tab has no page".to_string())
}

fn to_rect(bounds: Bounds) -> Rect {
    Rect {
        position: LogicalPosition::new(bounds.x, bounds.y).into(),
        size: LogicalSize::new(bounds.width, bounds.height).into(),
    }
}

/// Make the webviews agree with the state: the active tab's is at the browser's
/// rect and shown while the browser is shown; every other one is hidden (alive,
/// with its page and history). Safe to call after any change, and from any
/// thread that holds no lock.
fn sync_views(app: &AppHandle, shared: &Shared) {
    let (ids, active, visible, bounds) = {
        let browser = shared.browser();
        (
            browser.tabs.ids(),
            browser.tabs.active(),
            browser.visible,
            browser.bounds,
        )
    };
    for id in ids {
        let Some(view) = app.get_webview(&tabs::label(id)) else {
            continue;
        };
        if Some(id) == active {
            if let Some(bounds) = bounds {
                let _ = view.set_bounds(to_rect(bounds));
            }
            let _ = if visible { view.show() } else { view.hide() };
        } else {
            let _ = view.hide();
        }
    }
}

/// Build the webview of tab `id` on `url`. Runs on a thread that is not the
/// main one, and holds no lock while it waits for the main thread. If the tab
/// was closed while the webview was being built, the webview goes with it.
fn create_tab_webview(
    app: &AppHandle,
    shared: &Arc<Shared>,
    id: u32,
    url: Url,
    bounds: Bounds,
) -> Result<(), String> {
    let window = app
        .get_window(MAIN_LABEL)
        .ok_or_else(|| "The main window is not available".to_string())?;
    let on_navigation = {
        let (app, shared) = (app.clone(), shared.clone());
        move |target: &Url| navigation_allowed(&app, &shared, Origin::Tab(id), target)
    };
    let on_new_window = {
        let (app, shared) = (app.clone(), shared.clone());
        move |target: Url, features: NewWindowFeatures| {
            new_window(&app, &shared, Origin::Tab(id), target, features)
        }
    };
    let on_download = download_handler(app.clone(), shared.clone(), Some(id));
    let on_title = {
        let (app, shared) = (app.clone(), shared.clone());
        move |_view: Webview, title: String| {
            if let Some(state) = shared.change_tab(id, |tab| tab.title = Some(title)) {
                emit_state(&app, &state);
            }
        }
    };
    let on_load = {
        let (app, shared) = (app.clone(), shared.clone());
        move |_view: Webview, payload: tauri::webview::PageLoadPayload<'_>| {
            if payload.event() == PageLoadEvent::Finished {
                let loaded = shared.change_tab(id, |tab| tab.url = Some(payload.url().to_string()));
                if let Some(state) = loaded {
                    emit_state(&app, &state);
                }
            }
        }
    };

    let builder = WebviewBuilder::new(tabs::label(id), WebviewUrl::External(url))
        .incognito(true)
        .on_navigation(on_navigation)
        .on_new_window(on_new_window)
        .on_download(on_download)
        .on_document_title_changed(on_title)
        .on_page_load(on_load);
    let rect = to_rect(bounds);
    let view = window
        .add_child(builder, rect.position, rect.size)
        .map_err(|e| e.to_string())?;
    if shared.browser().tabs.get(id).is_none() {
        let _ = view.close();
        return Err(NO_SUCH_TAB.to_string());
    }
    Ok(())
}

/// Show the browser at `bounds` and load `url` in the active tab, making the
/// first tab when there is none. Reuses the browser when one is already open.
pub fn open(app: &AppHandle, url: Url, bounds: Bounds) -> Result<BrowserState, String> {
    let shared = shared(app);
    let tab = {
        let mut browser = shared.browser();
        browser.bounds = Some(bounds);
        browser.visible = true;
        if browser.tabs.is_empty() {
            browser
                .tabs
                .add(true)
                .map_err(|_| tabs::LIMIT_MESSAGE.to_string())?;
        }
        browser
            .tabs
            .active()
            .ok_or_else(|| NO_SUCH_TAB.to_string())?
    };
    navigate_tab(app, &shared, tab, url)
}

/// Load `url` in `tab`: in its webview, or, for a tab that has no page yet, in
/// a webview made now.
fn navigate_tab(
    app: &AppHandle,
    shared: &Arc<Shared>,
    tab: u32,
    url: Url,
) -> Result<BrowserState, String> {
    let (bounds, had_page) = {
        let mut browser = shared.browser();
        let had_page = match browser.tabs.get_mut(tab) {
            Some(entry) => {
                let had = entry.url.is_some();
                entry.url = Some(url.to_string());
                entry.blocked = None;
                had
            }
            None => return Err(NO_SUCH_TAB.to_string()),
        };
        browser.typed.insert(tab, url.clone());
        (browser.bounds, had_page)
    };
    if had_page {
        tab_webview(app, tab)?
            .navigate(url)
            .map_err(|e| e.to_string())?;
    } else {
        let made = bounds
            .ok_or_else(|| "The browser area is not known".to_string())
            .and_then(|bounds| create_tab_webview(app, shared, tab, url, bounds));
        if let Err(reason) = made {
            shared.change_tab(tab, |entry| entry.url = None);
            return Err(reason);
        }
        sync_views(app, shared);
    }
    let state = shared.snapshot();
    emit_state(app, &state);
    Ok(state)
}

pub fn navigate(app: &AppHandle, tab: u32, url: Url) -> Result<BrowserState, String> {
    navigate_tab(app, &shared(app), tab, url)
}

/// A new blank tab, in the foreground. Refused at the limit.
pub fn new_tab(app: &AppHandle) -> Result<BrowserState, String> {
    let shared = shared(app);
    {
        let mut browser = shared.browser();
        if browser.tabs.is_empty() {
            return Err("The browser is not open".to_string());
        }
        browser
            .tabs
            .add(true)
            .map_err(|_| tabs::LIMIT_MESSAGE.to_string())?;
    }
    sync_views(app, &shared);
    let state = shared.snapshot();
    emit_state(app, &state);
    Ok(state)
}

/// Bring `tab` to the front; the one that was there goes hidden, page alive.
pub fn activate_tab(app: &AppHandle, tab: u32) -> Result<BrowserState, String> {
    let shared = shared(app);
    if !shared.browser().tabs.activate(tab) {
        return Err(NO_SUCH_TAB.to_string());
    }
    sync_views(app, &shared);
    let state = shared.snapshot();
    emit_state(app, &state);
    Ok(state)
}

/// Close `tab` and destroy its webview. The only tab is replaced by a blank
/// one, so the browser is never left without an address bar.
pub fn close_tab(app: &AppHandle, tab: u32) -> Result<BrowserState, String> {
    let shared = shared(app);
    {
        let mut browser = shared.browser();
        if !browser.tabs.close(tab) {
            return Err(NO_SUCH_TAB.to_string());
        }
        browser.typed.remove(&tab);
    }
    if let Some(view) = app.get_webview(&tabs::label(tab)) {
        let _ = view.close();
    }
    sync_views(app, &shared);
    let state = shared.snapshot();
    emit_state(app, &state);
    Ok(state)
}

pub fn back(app: &AppHandle, tab: u32) -> Result<(), String> {
    tab_webview(app, tab)?
        .eval("history.back()")
        .map_err(|e| e.to_string())
}

pub fn forward(app: &AppHandle, tab: u32) -> Result<(), String> {
    tab_webview(app, tab)?
        .eval("history.forward()")
        .map_err(|e| e.to_string())
}

pub fn reload(app: &AppHandle, tab: u32) -> Result<(), String> {
    tab_webview(app, tab)?.reload().map_err(|e| e.to_string())
}

/// Remember where the active tab goes, and move it there. A browser with no
/// page yet just remembers.
pub fn set_bounds(app: &AppHandle, bounds: Bounds) -> Result<(), String> {
    let shared = shared(app);
    let active = {
        let mut browser = shared.browser();
        browser.bounds = Some(bounds);
        browser.tabs.active()
    };
    if let Some(view) = active.and_then(|id| app.get_webview(&tabs::label(id))) {
        view.set_bounds(to_rect(bounds))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Show or hide the browser as a whole: the active tab's webview follows; the
/// other tabs stay hidden either way.
pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    let shared = shared(app);
    shared.browser().visible = visible;
    sync_views(app, &shared);
    Ok(())
}

/// Close every tab and every popup, and forget the browser. The next `open`
/// starts from nothing.
pub fn close(app: &AppHandle) -> Result<(), String> {
    close_popups(app);
    let shared = shared(app);
    {
        // The list keeps counting ids, so a label is not reused while its
        // webview is still going away.
        let mut browser = shared.browser();
        browser.tabs.clear();
        browser.typed.clear();
        browser.bounds = None;
        browser.visible = false;
    }
    close_tab_webviews(app);
    // The empty state is a snapshot like any other: newer than the ones the UI
    // already has, so it is not mistaken for a stale one.
    emit_state(app, &shared.snapshot());
    Ok(())
}

/// Remember the folder the person chose for what EntropIA does not keep.
pub fn set_download_dir(app: &AppHandle, dir: Option<std::path::PathBuf>) {
    *shared(app)
        .download_dir
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = dir;
}

pub fn state(app: &AppHandle) -> Result<BrowserState, String> {
    Ok(shared(app).snapshot())
}

/// Run the capture script in the page of `tab` and validate what comes back.
///
/// The script goes through the platform's script evaluation
/// (`Webview::eval_with_callback`), not the page's IPC: the page never sees a
/// command and needs no capability. The callback hands the result, serialised
/// as JSON, to a one-shot channel so this can stay an async command; if the
/// page does not answer in [`CAPTURE_TIMEOUT`] (a hung script, a page
/// mid-navigation whose document is replaced) the capture fails instead of
/// waiting forever.
pub async fn capture(
    app: &AppHandle,
    tab: u32,
    kind: CaptureKind,
) -> Result<CaptureDraft, CaptureError> {
    let view = app
        .get_webview(&tabs::label(tab))
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
        shared.kind_for(Some(tab), url)
    })
}

/// Close every tab and popup, before the main window is destroyed.
pub fn shutdown(app: &AppHandle) {
    close_popups(app);
    close_tab_webviews(app);
}
