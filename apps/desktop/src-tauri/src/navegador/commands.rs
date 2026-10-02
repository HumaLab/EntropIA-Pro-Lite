//! The commands the Navegador view calls. They run only for the `main`
//! webview (see `capabilities/default.json`); the child webview that shows
//! remote content has no capability at all.
//!
//! They are `async` on purpose: creating a webview blocks until the main thread
//! has built it, so running that on the main thread (where synchronous
//! commands run) would deadlock on Windows.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::capture::{CaptureDraft, CaptureKind};
use super::download;
use super::save::{self, Saved, Target};
use super::sources::{self, CopyTicket, DeleteOutcome, SourceDetail, SourceSummary};
use super::tabs::BrowserState;
use super::url_policy::{self, NavigationKind};
use super::zotero_copy::launch::{self, LaunchOutcome};
use super::zotero_copy::port::{ConnectorPort, ZoteroPort};
use super::zotero_copy::run::{
    self as zotero_run, CopyStatus, DrainReport, LibraryList, RunOptions,
};
use super::zotero_copy::store::{self as zotero_store, LibraryRef, ZoteroCopy};
use super::{bounds, viewer, UNAVAILABLE};
use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;

/// The setting that keeps the folder the person chose for downloads.
const DOWNLOAD_DIR_KEY: &str = "navegador_download_dir";

/// Where files EntropIA does not keep are saved. Mirrors `DownloadFolder` in
/// `lib/navegador-capture.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDir {
    /// The folder in use, or `None` when the system has no Downloads folder.
    pub path: Option<String>,
    /// True when it is the system's Downloads folder, not one the person chose.
    pub is_default: bool,
}

/// What the folder in use is, given what was chosen and what the system offers.
/// A chosen folder that no longer exists counts as not chosen.
fn describe_folder(
    chosen: Option<&Path>,
    default: Option<PathBuf>,
    is_dir: impl Fn(&Path) -> bool,
) -> DownloadDir {
    let chosen_in_use = chosen.filter(|dir| is_dir(dir));
    let folder = download::resolve_folder(chosen, default, is_dir);
    DownloadDir {
        path: folder.map(|dir| dir.to_string_lossy().into_owned()),
        is_default: chosen_in_use.is_none(),
    }
}

/// The folder saved in the settings, if any (an empty value means none).
fn chosen_folder(db: &AppDbState) -> Option<PathBuf> {
    let conn = db.ui_conn.lock().ok()?;
    crate::settings::get_raw_setting(&conn, DOWNLOAD_DIR_KEY)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

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

/// Show the browser at the given rectangle (logical pixels, relative to the
/// window) and load `url` in its active tab, making the first tab when there is
/// none. Reuses the browser when one is already open.
#[tauri::command]
pub async fn navegador_open(
    app: AppHandle,
    db: State<'_, AppDbState>,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<BrowserState, String> {
    ensure_available()?;
    let url = typed(&url)?;
    let bounds = bounds::sanitize(x, y, width, height)?;
    // The first download can come before the view asks for the folder.
    viewer::set_download_dir(&app, chosen_folder(&db));
    viewer::open(&app, url, bounds)
}

/// The folder non-PDF downloads are saved to: the one the person chose, or the
/// system's Downloads folder. Also tells the browser which one to use.
#[tauri::command]
pub async fn navegador_download_dir(
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<DownloadDir, String> {
    ensure_available()?;
    let chosen = chosen_folder(&db);
    viewer::set_download_dir(&app, chosen.clone());
    Ok(describe_folder(
        chosen.as_deref(),
        app.path().download_dir().ok(),
        |dir| dir.is_dir(),
    ))
}

/// Choose the folder non-PDF downloads are saved to. It has to be an existing
/// directory given as a full path; anything else is refused with the reason.
#[tauri::command]
pub async fn navegador_set_download_dir(
    app: AppHandle,
    db: State<'_, AppDbState>,
    path: String,
) -> Result<DownloadDir, String> {
    ensure_available()?;
    let dir = download::validate_folder(&path).map_err(str::to_string)?;
    {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        crate::settings::persist_setting(&conn, DOWNLOAD_DIR_KEY, &dir.to_string_lossy())?;
    }
    viewer::set_download_dir(&app, Some(dir.clone()));
    Ok(describe_folder(
        Some(&dir),
        app.path().download_dir().ok(),
        |candidate| candidate.is_dir(),
    ))
}

/// Load `url` in `tab`. Every command that acts on a page names its tab, so it
/// is the tab the person was looking at when they acted, not whichever became
/// active since.
#[tauri::command]
pub async fn navegador_navigate(
    app: AppHandle,
    tab: u32,
    url: String,
) -> Result<BrowserState, String> {
    ensure_available()?;
    viewer::navigate(&app, tab, typed(&url)?)
}

#[tauri::command]
pub async fn navegador_back(app: AppHandle, tab: u32) -> Result<(), String> {
    ensure_available()?;
    viewer::back(&app, tab)
}

#[tauri::command]
pub async fn navegador_forward(app: AppHandle, tab: u32) -> Result<(), String> {
    ensure_available()?;
    viewer::forward(&app, tab)
}

#[tauri::command]
pub async fn navegador_reload(app: AppHandle, tab: u32) -> Result<(), String> {
    ensure_available()?;
    viewer::reload(&app, tab)
}

/// A new blank tab in the foreground. Refused when the browser has the maximum
/// number of tabs.
#[tauri::command]
pub async fn navegador_new_tab(app: AppHandle) -> Result<BrowserState, String> {
    ensure_available()?;
    viewer::new_tab(&app)
}

/// Bring `tab` to the front; the others stay alive and hidden.
#[tauri::command]
pub async fn navegador_activate_tab(app: AppHandle, tab: u32) -> Result<BrowserState, String> {
    ensure_available()?;
    viewer::activate_tab(&app, tab)
}

/// Close `tab` and destroy its webview. The last tab is replaced by a blank one.
#[tauri::command]
pub async fn navegador_close_tab(app: AppHandle, tab: u32) -> Result<BrowserState, String> {
    ensure_available()?;
    viewer::close_tab(&app, tab)
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

/// Close every tab and popup and forget the browser.
#[tauri::command]
pub async fn navegador_close(app: AppHandle) -> Result<(), String> {
    ensure_available()?;
    viewer::close(&app)
}

#[tauri::command]
pub async fn navegador_state(app: AppHandle) -> Result<BrowserState, String> {
    ensure_available()?;
    viewer::state(&app)
}

/// Read the page in `tab`: its text and an HTML snapshot. Returns a draft;
/// nothing is stored. Errors are a stable code (`no_selection`, `timeout`,
/// ...), optionally followed by `: detail`.
#[tauri::command]
pub async fn navegador_capture_page(app: AppHandle, tab: u32) -> Result<CaptureDraft, String> {
    ensure_available()?;
    let draft = viewer::capture(&app, tab, CaptureKind::Page)
        .await
        .map_err(|e| e.to_string())?;
    save::holds(&app).hold_draft(draft.clone());
    Ok(draft)
}

/// Read the text selected in the page of `tab`, with the text around it.
#[tauri::command]
pub async fn navegador_capture_selection(app: AppHandle, tab: u32) -> Result<CaptureDraft, String> {
    ensure_available()?;
    let draft = viewer::capture(&app, tab, CaptureKind::Selection)
        .await
        .map_err(|e| e.to_string())?;
    save::holds(&app).hold_draft(draft.clone());
    Ok(draft)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Save the capture draft `draft_id` (a page or a selection): its files under
/// the data directory and its rows in the archive, together. The renderer only
/// names the draft; the HTML and the hash are the ones held on this side.
/// Errors are a stable code (`unknown_draft`, `db_error`, ...), optionally
/// followed by `: detail`; a failed save leaves no file and no row, and the
/// draft stays available to try again.
#[tauri::command]
pub async fn navegador_save_capture(
    app: AppHandle,
    db: State<'_, AppDbState>,
    draft_id: String,
) -> Result<Saved, String> {
    let data_dir = crate::path_utils::data_dir(&app)?;
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let target = Target {
            data_dir: &data_dir,
            conn: &conn,
            now_ms: now_ms(),
        };
        save::holds(&app)
            .save_draft(&draft_id, &target)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Save the verified PDF `download_id` out of quarantine, the same way.
#[tauri::command]
pub async fn navegador_save_download(
    app: AppHandle,
    db: State<'_, AppDbState>,
    download_id: String,
) -> Result<Saved, String> {
    let data_dir = crate::path_utils::data_dir(&app)?;
    let quarantine = download::quarantine_dir(&crate::path_utils::cache_dir(&app)?);
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let target = Target {
            data_dir: &data_dir,
            conn: &conn,
            now_ms: now_ms(),
        };
        save::holds(&app)
            .save_pdf(&download_id, &target, &quarantine)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Forget a capture draft the person dismissed, freeing its held HTML.
#[tauri::command]
pub async fn navegador_discard_draft(app: AppHandle, draft_id: String) -> Result<(), String> {
    save::holds(&app).discard_draft(&draft_id);
    Ok(())
}

/// The saved web sources, newest first, optionally only those whose title, URL
/// or captured text contain `query`. Errors are a stable code (`db_error`),
/// optionally followed by `: detail`.
#[tauri::command]
pub async fn navegador_list_sources(
    db: State<'_, AppDbState>,
    query: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<SourceSummary>, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        sources::list_sources(
            &conn,
            query.as_deref(),
            limit.map_or(sources::DEFAULT_LIMIT, |n| n as usize),
        )
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// One saved source with its captures, newest first; `null` when it no longer
/// exists. The renderer names the source by id and never sends a path.
#[tauri::command]
pub async fn navegador_source_detail(
    app: AppHandle,
    db: State<'_, AppDbState>,
    source_id: String,
) -> Result<Option<SourceDetail>, String> {
    let data_dir = crate::path_utils::data_dir(&app)?;
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        sources::source_detail(&conn, &data_dir, &source_id)
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Delete a saved source: its rows in one transaction, then its folder. Copies
/// made into a collection are not touched. A folder that will not go does not
/// fail the delete; the startup sweep removes it later and `leftoverFiles` says
/// so.
#[tauri::command]
pub async fn navegador_delete_source(
    app: AppHandle,
    db: State<'_, AppDbState>,
    source_id: String,
) -> Result<DeleteOutcome, String> {
    let data_dir = crate::path_utils::data_dir(&app)?;
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        sources::delete_source(&conn, &data_dir, &source_id)
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Where the saved PDF of capture `capture_id` is on disk, for the app's own PDF
/// viewer. The renderer names a capture and never sends a path: this side finds
/// the stored key, checks it stays inside that source's folder (never a link, a
/// regular file only) and answers the path. Nothing is downloaded, so it works
/// offline. Errors are a stable code (`not_found`, `not_a_pdf`, `file_missing`,
/// `invalid_id`, `db_error`), optionally followed by `: detail`.
#[tauri::command]
pub async fn navegador_pdf_file(
    app: AppHandle,
    db: State<'_, AppDbState>,
    capture_id: String,
) -> Result<String, String> {
    let data_dir = crate::path_utils::data_dir(&app)?;
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        sources::pdf_capture_file(&conn, &data_dir, &capture_id)
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// What copying the saved PDF of capture `capture_id` into a collection needs:
/// where the file is and where it came from. The file is found and re-hashed on
/// this side and the provenance is read from the rows, so the renderer names a
/// capture and never supplies a path or the words that vouch for it. Errors are
/// the codes of [`navegador_pdf_file`] plus `file_changed`.
#[tauri::command]
pub async fn navegador_copy_ticket(
    app: AppHandle,
    db: State<'_, AppDbState>,
    capture_id: String,
) -> Result<CopyTicket, String> {
    let data_dir = crate::path_utils::data_dir(&app)?;
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        sources::copy_ticket(&conn, &data_dir, &capture_id)
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Queue one copy of a saved source (or of one of its PDF captures) to a Zotero
/// library. The renderer names ids and a library; the item and the file are
/// built here from the rows. Idempotent: the same source, capture and library
/// answer with the row that exists. Errors are a stable code (`not_found`,
/// `not_a_pdf`, `invalid_library`, `db_error`), optionally followed by `: detail`.
#[tauri::command]
pub async fn navegador_zotero_copy_request(
    db: State<'_, AppDbState>,
    source_id: String,
    capture_id: Option<String>,
    library: LibraryRef,
) -> Result<ZoteroCopy, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        zotero_store::request(&conn, &source_id, capture_id.as_deref(), &library)
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// The copies to Zotero, newest first, of one source or of all of them.
#[tauri::command]
pub async fn navegador_zotero_copy_list(
    db: State<'_, AppDbState>,
    source_id: Option<String>,
) -> Result<Vec<ZoteroCopy>, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        zotero_store::list(&conn, source_id.as_deref())
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Send what waits, if Zotero answers. One drain at a time (a second call while
/// one runs answers with nothing to report). It never starts Zotero.
#[tauri::command]
pub async fn navegador_zotero_copy_run(
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<DrainReport, String> {
    let data_dir = crate::path_utils::data_dir(&app)?;
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let report = zotero_run::drain_exclusive(
            &conn,
            &data_dir,
            &ConnectorPort::local(),
            &RunOptions::default(),
        )?;
        Ok(report.unwrap_or(DrainReport {
            reachable: true,
            copies: Vec::new(),
        }))
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Cancel a copy that has not started and not finished.
#[tauri::command]
pub async fn navegador_zotero_copy_cancel(
    db: State<'_, AppDbState>,
    copy_id: String,
) -> Result<ZoteroCopy, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        zotero_store::cancel(&conn, &copy_id)
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
}

/// Start the installed Zotero in the background so the queue can drain. Only
/// from a button the person pressed; a second start within a minute is refused.
#[tauri::command]
pub async fn navegador_zotero_launch() -> Result<LaunchOutcome, String> {
    tokio::task::spawn_blocking(|| {
        let reachable = ConnectorPort::local().ping().is_ok();
        launch::launch(reachable)
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))
}

/// The libraries Zotero itself offers for writing (personal first, then the
/// groups). `reachable: false` when Zotero does not answer: the UI then falls
/// back to the libraries the archive knows and says so.
#[tauri::command]
pub async fn navegador_zotero_libraries() -> Result<LibraryList, String> {
    tokio::task::spawn_blocking(|| zotero_run::live_libraries(&ConnectorPort::local()))
        .await
        .map_err(|e| format!("task_failed: {e}"))?
}

/// Whether the source (and, for a PDF capture, its PDF) is already in the
/// library, before anything is copied. Reads only. Errors are a stable code
/// (`not_found`, `invalid_library`, `zotero_api_disabled`, `db_error`).
#[tauri::command]
pub async fn navegador_zotero_status(
    db: State<'_, AppDbState>,
    source_id: String,
    capture_id: Option<String>,
    library: LibraryRef,
) -> Result<CopyStatus, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        zotero_run::check_status(
            &conn,
            &ConnectorPort::local(),
            &source_id,
            capture_id.as_deref(),
            &library,
        )
    })
    .await
    .map_err(|e| format!("task_failed: {e}"))?
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
    fn the_chosen_folder_is_reported_as_not_the_default() {
        let chosen = PathBuf::from("D:/Docs");
        let default = PathBuf::from("C:/Users/x/Downloads");
        let described = describe_folder(Some(&chosen), Some(default), |_| true);
        assert_eq!(
            described,
            DownloadDir {
                path: Some("D:/Docs".into()),
                is_default: false
            }
        );
    }

    #[test]
    fn a_folder_that_vanished_falls_back_to_the_default() {
        let chosen = PathBuf::from("D:/gone");
        let default = PathBuf::from("C:/Users/x/Downloads");
        let described = describe_folder(Some(&chosen), Some(default.clone()), |dir| dir == default);
        assert_eq!(described.path.as_deref(), Some("C:/Users/x/Downloads"));
        assert!(described.is_default);
    }

    #[test]
    fn nothing_chosen_is_the_default_and_no_folder_at_all_is_none() {
        let default = PathBuf::from("C:/Users/x/Downloads");
        let described = describe_folder(None, Some(default), |_| true);
        assert!(described.is_default);
        assert_eq!(described.path.as_deref(), Some("C:/Users/x/Downloads"));
        assert_eq!(
            describe_folder(None, None, |_| true),
            DownloadDir {
                path: None,
                is_default: true
            }
        );
    }

    #[test]
    fn the_folder_reaches_the_ui_in_camel_case() {
        let json = serde_json::to_value(DownloadDir {
            path: Some("D:/Docs".into()),
            is_default: false,
        })
        .unwrap();
        assert_eq!(json["path"], "D:/Docs");
        assert_eq!(json["isDefault"], false);
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
