//! Microsoft Store update detection for EntropIA Lite.
//!
//! Only a Windows Lite build (no `local-ml`) installed from Store asks Store
//! whether its main package has an update. Every other build answers
//! `skipped` without touching WinRT or the cache. The app never downloads or
//! installs anything: the frontend only offers to open the Store listing.
//!
//! One query per session at most, shared by every caller, and a six-hour cache
//! in `app_settings` keyed by the installed package full name (which carries the
//! installed version, so updating invalidates it).

// The query core only runs on Windows Lite and in tests; other builds keep the
// command registered but answer `skipped`.
#![cfg_attr(
    not(all(target_os = "windows", not(feature = "local-ml"))),
    allow(dead_code)
)]

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::settings::{get_setting, set_setting};

/// The Lite Store listing. The only non-HTTP(S) URI `open_external_url` accepts,
/// and only on Windows Lite.
pub const STORE_PRODUCT_URI: &str = "ms-windows-store://pdp/?ProductId=9N328K9L95JD";

/// Whether this build asks Store for updates at all.
pub const STORE_UPDATES_SUPPORTED: bool =
    cfg!(all(target_os = "windows", not(feature = "local-ml")));

/// Package family of Lite as distributed by Store. Mirrors
/// `scripts/repack-store-msix.ps1` and `msix/README.md`.
const LITE_STORE_FAMILY_NAME: &str = "CONICET.EntropIALite_b16na7gwepwme";

const CACHE_KEY: &str = "microsoft_store_update_cache";
const CACHE_TTL_SECONDS: u64 = 6 * 60 * 60;
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreUpdateStatus {
    /// Store (or a valid cache of this installation) reports a main-package update.
    Available,
    /// Store answered without a main-package update, or that answer is cached.
    UpToDate,
    /// Not applicable: other variant/platform, no package identity, or a family
    /// other than Lite Store.
    Skipped,
    /// The query, the native context, the identity read or the wait failed.
    /// Never means "up to date".
    Unavailable,
}

impl StoreUpdateStatus {
    fn from_update_available(update_available: bool) -> Self {
        if update_available {
            Self::Available
        } else {
            Self::UpToDate
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreUpdateCache {
    package_full_name: String,
    checked_at_unix_seconds: u64,
    update_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installation {
    pub full_name: String,
    pub family_name: String,
}

/// What reading the current package identity produced.
#[derive(Debug)]
pub(crate) enum IdentityRead {
    /// The process runs without package identity (for example `tauri dev`).
    NoIdentity,
    Failed(String),
    Found(Installation),
}

/// Decide whether this installation may ask Store at all.
pub(crate) fn applicable_installation(
    read: IdentityRead,
) -> Result<Installation, StoreUpdateStatus> {
    match read {
        IdentityRead::NoIdentity => Err(StoreUpdateStatus::Skipped),
        IdentityRead::Failed(error) => {
            eprintln!("[store-updates] identity read failed: {error}");
            Err(StoreUpdateStatus::Unavailable)
        }
        IdentityRead::Found(installation) if installation.family_name == LITE_STORE_FAMILY_NAME => {
            Ok(installation)
        }
        IdentityRead::Found(_) => Err(StoreUpdateStatus::Skipped),
    }
}

/// Store lists the app and its optional packages together; only an entry of the
/// running package's own family counts, wherever it sits in the list.
fn main_package_has_update(update_families: &[String], family_name: &str) -> bool {
    update_families.iter().any(|family| family == family_name)
}

fn read_valid_cache(db: &Mutex<Connection>, full_name: &str, now: u64) -> Option<bool> {
    let raw = {
        let conn = db.lock().ok()?;
        get_setting(&conn, CACHE_KEY)?
    };
    let cache: StoreUpdateCache = match serde_json::from_str(&raw) {
        Ok(cache) => cache,
        Err(error) => {
            eprintln!("[store-updates] ignoring unreadable cache: {error}");
            return None;
        }
    };
    if cache.package_full_name != full_name {
        return None;
    }
    // A timestamp in the future is not trusted: it would inhibit queries for as
    // long as the clock stays behind it.
    let age = now.checked_sub(cache.checked_at_unix_seconds)?;
    (age < CACHE_TTL_SECONDS).then_some(cache.update_available)
}

fn persist_result(db: &Mutex<Connection>, full_name: &str, now: u64, update_available: bool) {
    let cache = StoreUpdateCache {
        package_full_name: full_name.to_string(),
        checked_at_unix_seconds: now,
        update_available,
    };
    let result = serde_json::to_string(&cache)
        .map_err(|error| error.to_string())
        .and_then(|value| {
            let conn = db.lock().map_err(|error| error.to_string())?;
            set_setting(&conn, CACHE_KEY, &value).map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        eprintln!(
            "[store-updates] cache write failed, result kept for this session only \
             (persistence across restarts not guaranteed): {error}"
        );
    }
}

/// Answer for one installation: a valid cache, or one bounded Store query.
///
/// `query` yields the package family of every update Store reports. Timing out
/// drops the query future, which is what cancels the native operation.
pub(crate) async fn check_installation<Q, Fut>(
    db: &Mutex<Connection>,
    installation: &Installation,
    clock: impl Fn() -> u64,
    query: Q,
    timeout: Duration,
) -> StoreUpdateStatus
where
    Q: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<String>, String>>,
{
    if let Some(update_available) = read_valid_cache(db, &installation.full_name, clock()) {
        return StoreUpdateStatus::from_update_available(update_available);
    }

    let update_families = match tokio::time::timeout(timeout, query()).await {
        Ok(Ok(families)) => families,
        Ok(Err(error)) => {
            eprintln!("[store-updates] query failed: {error}");
            return StoreUpdateStatus::Unavailable;
        }
        Err(_) => {
            eprintln!(
                "[store-updates] query timed out after {}s",
                timeout.as_secs_f32()
            );
            return StoreUpdateStatus::Unavailable;
        }
    };

    let update_available = main_package_has_update(&update_families, &installation.family_name);
    persist_result(db, &installation.full_name, clock(), update_available);
    StoreUpdateStatus::from_update_available(update_available)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Session-wide single check. The first caller starts it on its own task, so a
/// caller that goes away cannot cancel it or cause a second native query; every
/// caller waits for the same terminal result, including `unavailable`.
pub struct StoreUpdateState {
    sender: Mutex<Option<watch::Sender<Option<StoreUpdateStatus>>>>,
    receiver: watch::Receiver<Option<StoreUpdateStatus>>,
}

impl StoreUpdateState {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(None);
        Self {
            sender: Mutex::new(Some(sender)),
            receiver,
        }
    }

    pub(crate) async fn get_or_start<F, Fut>(&self, start: F) -> StoreUpdateStatus
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = StoreUpdateStatus> + Send + 'static,
    {
        let mut receiver = self.receiver.clone();
        let sender = self.sender.lock().ok().and_then(|mut slot| slot.take());
        if let Some(sender) = sender {
            let check = start();
            tokio::spawn(async move {
                let status = check.await;
                sender.send_replace(Some(status));
            });
        }
        // If the task dies without an answer its sender drops and this errors.
        let status = match receiver.wait_for(Option::is_some).await {
            Ok(status) => status.unwrap_or(StoreUpdateStatus::Unavailable),
            Err(_) => StoreUpdateStatus::Unavailable,
        };
        status
    }
}

#[tauri::command]
pub async fn check_microsoft_store_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, StoreUpdateState>,
) -> Result<StoreUpdateStatus, String> {
    #[cfg(all(target_os = "windows", not(feature = "local-ml")))]
    {
        use tauri::Manager;
        let db = app.state::<crate::db::state::AppDbState>().ui_conn.clone();
        Ok(state.get_or_start(move || native::check(app, db)).await)
    }

    #[cfg(not(all(target_os = "windows", not(feature = "local-ml"))))]
    {
        let _ = (app, state);
        Ok(StoreUpdateStatus::Skipped)
    }
}

#[cfg(all(target_os = "windows", not(feature = "local-ml")))]
mod native {
    use std::sync::{Arc, Mutex};

    use rusqlite::Connection;
    use tauri::{AppHandle, Manager};
    use windows::core::{Interface, HRESULT};
    use windows::ApplicationModel::Package;
    use windows::Services::Store::StoreContext;
    use windows::Win32::UI::Shell::IInitializeWithWindow;

    use super::{
        applicable_installation, check_installation, unix_now, IdentityRead, Installation,
        StoreUpdateStatus, QUERY_TIMEOUT,
    };

    /// `HRESULT_FROM_WIN32(APPMODEL_ERROR_NO_PACKAGE)`: the process has no
    /// package identity. Any other failure is not "unpackaged".
    const APPMODEL_ERROR_NO_PACKAGE: HRESULT = HRESULT(0x8007_3D54_u32 as i32);

    fn describe(stage: &str, error: &windows::core::Error) -> String {
        format!("{stage} failed (HRESULT {:#010X})", error.code().0 as u32)
    }

    fn read_identity() -> IdentityRead {
        let package = match Package::Current() {
            Ok(package) => package,
            Err(error) if error.code() == APPMODEL_ERROR_NO_PACKAGE => {
                return IdentityRead::NoIdentity
            }
            Err(error) => return IdentityRead::Failed(describe("Package::Current", &error)),
        };
        let names = package
            .Id()
            .and_then(|id| Ok((id.FullName()?, id.FamilyName()?)));
        match names {
            Ok((full_name, family_name)) => IdentityRead::Found(Installation {
                full_name: full_name.to_string_lossy(),
                family_name: family_name.to_string_lossy(),
            }),
            Err(error) => IdentityRead::Failed(describe("Package::Id", &error)),
        }
    }

    pub(super) async fn check(app: AppHandle, db: Arc<Mutex<Connection>>) -> StoreUpdateStatus {
        let installation = match applicable_installation(read_identity()) {
            Ok(installation) => installation,
            Err(status) => return status,
        };
        check_installation(
            &db,
            &installation,
            unix_now,
            || query_update_families(app),
            QUERY_TIMEOUT,
        )
        .await
    }

    /// Cancels a native Store operation unless it completed.
    struct CancelOnDrop<F: FnMut()>(Option<F>);

    impl<F: FnMut()> CancelOnDrop<F> {
        fn disarm(mut self) {
            self.0 = None;
        }
    }

    impl<F: FnMut()> Drop for CancelOnDrop<F> {
        fn drop(&mut self) {
            if let Some(cancel) = self.0.as_mut() {
                cancel();
            }
        }
    }

    /// Package family of every update Store reports for this app.
    ///
    /// Store requires the call on the UI thread with the owner window attached
    /// (`0x80070578` otherwise), so the operation starts there and is awaited
    /// here, off that thread and without the database lock.
    async fn query_update_families(app: AppHandle) -> Result<Vec<String>, String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let handle = app.clone();
        app.run_on_main_thread(move || {
            // The caller already timed out: do not start a query nobody awaits.
            if sender.is_closed() {
                return;
            }
            let started = (|| {
                let window = handle
                    .get_webview_window("main")
                    .ok_or_else(|| "main window unavailable".to_string())?;
                let hwnd = window
                    .hwnd()
                    .map_err(|error| format!("main window handle failed: {error}"))?;
                let context = StoreContext::GetDefault()
                    .map_err(|e| describe("StoreContext::GetDefault", &e))?;
                let initialize: IInitializeWithWindow = context
                    .cast()
                    .map_err(|e| describe("IInitializeWithWindow cast", &e))?;
                // SAFETY: `hwnd` is the live main window owned by Tauri, and this
                // closure runs on the UI thread that owns it, as Store requires.
                unsafe { initialize.Initialize(hwnd) }
                    .map_err(|e| describe("IInitializeWithWindow::Initialize", &e))?;
                context
                    .GetAppAndOptionalStorePackageUpdatesAsync()
                    .map_err(|e| describe("GetAppAndOptionalStorePackageUpdatesAsync", &e))
            })();
            // The receiver may have timed out while this ran: cancel the orphan.
            if let Err(Ok(operation)) = sender.send(started) {
                let _ = operation.Cancel();
            }
        })
        .map_err(|error| format!("dispatch to main thread failed: {error}"))?;

        let operation = receiver
            .await
            .map_err(|_| "main thread dropped the query".to_string())??;
        let pending = operation.clone();
        let guard = CancelOnDrop(Some(move || {
            let _ = pending.Cancel();
        }));
        let updates = operation
            .await
            .map_err(|e| describe("awaiting Store updates", &e))?;
        guard.disarm();

        let count = updates.Size().map_err(|e| describe("updates.Size", &e))?;
        let mut families = Vec::with_capacity(count as usize);
        for index in 0..count {
            let family = updates
                .GetAt(index)
                .and_then(|update| update.Package())
                .and_then(|package| package.Id())
                .and_then(|id| id.FamilyName())
                .map_err(|e| describe("reading update package family", &e))?;
            families.push(family.to_string_lossy());
        }
        Ok(families)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const FULL_NAME: &str = "CONICET.EntropIALite_1.4.0.0_x64__b16na7gwepwme";
    const HOUR: u64 = 60 * 60;
    const NOW: u64 = 1_800_000_000;

    fn lite() -> Installation {
        Installation {
            full_name: FULL_NAME.to_string(),
            family_name: LITE_STORE_FAMILY_NAME.to_string(),
        }
    }

    fn db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        Mutex::new(conn)
    }

    fn write_cache(db: &Mutex<Connection>, full_name: &str, checked_at: u64, available: bool) {
        let value = serde_json::to_string(&StoreUpdateCache {
            package_full_name: full_name.to_string(),
            checked_at_unix_seconds: checked_at,
            update_available: available,
        })
        .unwrap();
        set_setting(&db.lock().unwrap(), CACHE_KEY, &value).unwrap();
    }

    fn stored_cache(db: &Mutex<Connection>) -> Option<StoreUpdateCache> {
        get_setting(&db.lock().unwrap(), CACHE_KEY).map(|raw| serde_json::from_str(&raw).unwrap())
    }

    /// Runs one check whose query answers `answer` and counts query calls.
    async fn check_with(
        db: &Mutex<Connection>,
        now: u64,
        answer: Result<Vec<String>, String>,
    ) -> (StoreUpdateStatus, usize) {
        let calls = AtomicUsize::new(0);
        let status = check_installation(
            db,
            &lite(),
            || now,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move { answer }
            },
            QUERY_TIMEOUT,
        )
        .await;
        (status, calls.load(Ordering::SeqCst))
    }

    fn families(names: &[&str]) -> Result<Vec<String>, String> {
        Ok(names.iter().map(|name| name.to_string()).collect())
    }

    #[test]
    fn store_updates_status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&StoreUpdateStatus::UpToDate).unwrap(),
            "\"up_to_date\""
        );
    }

    #[tokio::test]
    async fn store_updates_main_package_among_optionals_is_an_update() {
        let db = db();
        let answer = families(&["CONICET.SomeOptional_abc", LITE_STORE_FAMILY_NAME]);
        let (status, _) = check_with(&db, NOW, answer).await;
        assert_eq!(status, StoreUpdateStatus::Available);
    }

    #[tokio::test]
    async fn store_updates_only_optional_packages_is_not_an_update() {
        let db = db();
        let answer = families(&["CONICET.SomeOptional_abc", "Other.Package_xyz"]);
        let (status, _) = check_with(&db, NOW, answer).await;
        assert_eq!(status, StoreUpdateStatus::UpToDate);
    }

    #[tokio::test]
    async fn store_updates_empty_collection_is_up_to_date() {
        let db = db();
        let (status, _) = check_with(&db, NOW, families(&[])).await;
        assert_eq!(status, StoreUpdateStatus::UpToDate);
    }

    #[tokio::test]
    async fn store_updates_positive_cache_is_reused_before_six_hours() {
        let db = db();
        write_cache(&db, FULL_NAME, NOW - 6 * HOUR + 1, true);
        let (status, calls) = check_with(&db, NOW, families(&[])).await;
        assert_eq!(status, StoreUpdateStatus::Available);
        assert_eq!(calls, 0);
    }

    #[tokio::test]
    async fn store_updates_cache_at_exactly_six_hours_queries_again() {
        let db = db();
        write_cache(&db, FULL_NAME, NOW - 6 * HOUR, true);
        let (status, calls) = check_with(&db, NOW, families(&[])).await;
        assert_eq!(status, StoreUpdateStatus::UpToDate);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn store_updates_cache_of_another_installation_is_ignored() {
        let db = db();
        write_cache(
            &db,
            "CONICET.EntropIALite_1.3.0.0_x64__b16na7gwepwme",
            NOW - HOUR,
            true,
        );
        let (status, calls) = check_with(&db, NOW, families(&[])).await;
        assert_eq!(status, StoreUpdateStatus::UpToDate);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn store_updates_cache_from_the_future_is_ignored() {
        let db = db();
        write_cache(&db, FULL_NAME, NOW + HOUR, true);
        let (status, calls) = check_with(&db, NOW, families(&[])).await;
        assert_eq!(status, StoreUpdateStatus::UpToDate);
        assert_eq!(calls, 1);
        assert_eq!(stored_cache(&db).unwrap().checked_at_unix_seconds, NOW);
    }

    #[tokio::test]
    async fn store_updates_corrupt_cache_is_ignored_and_replaced() {
        let db = db();
        set_setting(&db.lock().unwrap(), CACHE_KEY, "{not json").unwrap();
        let (status, calls) = check_with(&db, NOW, families(&[LITE_STORE_FAMILY_NAME])).await;
        assert_eq!(status, StoreUpdateStatus::Available);
        assert_eq!(calls, 1);
        assert!(stored_cache(&db).unwrap().update_available);
    }

    #[tokio::test]
    async fn store_updates_success_is_persisted_with_the_installation() {
        let db = db();
        check_with(&db, NOW, families(&[])).await;
        assert_eq!(
            stored_cache(&db),
            Some(StoreUpdateCache {
                package_full_name: FULL_NAME.to_string(),
                checked_at_unix_seconds: NOW,
                update_available: false,
            })
        );
    }

    #[tokio::test]
    async fn store_updates_failure_does_not_renew_the_cache() {
        let db = db();
        write_cache(&db, FULL_NAME, NOW - 7 * HOUR, true);
        let (status, calls) = check_with(&db, NOW, Err("store down".to_string())).await;
        assert_eq!(status, StoreUpdateStatus::Unavailable);
        assert_eq!(calls, 1);
        assert_eq!(
            stored_cache(&db).unwrap().checked_at_unix_seconds,
            NOW - 7 * HOUR
        );
    }

    #[tokio::test]
    async fn store_updates_timeout_is_unavailable_and_writes_nothing() {
        let db = db();
        let status = check_installation(
            &db,
            &lite(),
            || NOW,
            || async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                families(&[LITE_STORE_FAMILY_NAME])
            },
            Duration::from_millis(20),
        )
        .await;
        assert_eq!(status, StoreUpdateStatus::Unavailable);
        assert_eq!(stored_cache(&db), None);
    }

    #[tokio::test]
    async fn store_updates_failed_cache_write_keeps_the_valid_result() {
        // No app_settings table: reading misses and writing fails.
        let db = Mutex::new(Connection::open_in_memory().unwrap());
        let (status, _) = check_with(&db, NOW, families(&[LITE_STORE_FAMILY_NAME])).await;
        assert_eq!(status, StoreUpdateStatus::Available);
    }

    #[tokio::test]
    async fn store_updates_concurrent_callers_share_one_failed_check() {
        let state = Arc::new(StoreUpdateState::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let call = |state: Arc<StoreUpdateState>, starts: Arc<AtomicUsize>| async move {
            state
                .get_or_start(|| {
                    starts.fetch_add(1, Ordering::SeqCst);
                    async {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        StoreUpdateStatus::Unavailable
                    }
                })
                .await
        };
        let (first, second) = tokio::join!(
            call(state.clone(), starts.clone()),
            call(state.clone(), starts.clone())
        );
        let later = call(state.clone(), starts.clone()).await;
        assert_eq!([first, second, later], [StoreUpdateStatus::Unavailable; 3]);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn store_updates_cancelled_caller_does_not_restart_the_check() {
        let state = Arc::new(StoreUpdateState::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let start = |starts: Arc<AtomicUsize>| {
            move || {
                starts.fetch_add(1, Ordering::SeqCst);
                async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    StoreUpdateStatus::Available
                }
            }
        };
        let abandoned = tokio::time::timeout(
            Duration::from_millis(1),
            state.get_or_start(start(starts.clone())),
        )
        .await;
        assert!(abandoned.is_err());
        let status = state.get_or_start(start(starts.clone())).await;
        assert_eq!(status, StoreUpdateStatus::Available);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn store_updates_identity_filter_only_lets_lite_store_through() {
        assert_eq!(
            applicable_installation(IdentityRead::NoIdentity),
            Err(StoreUpdateStatus::Skipped)
        );
        assert_eq!(
            applicable_installation(IdentityRead::Failed("boom".to_string())),
            Err(StoreUpdateStatus::Unavailable)
        );
        let foreign = Installation {
            full_name: "CONICET.EntropIAPro_1.0.0.0_x64__zzz".to_string(),
            family_name: "CONICET.EntropIAPro_zzz".to_string(),
        };
        assert_eq!(
            applicable_installation(IdentityRead::Found(foreign)),
            Err(StoreUpdateStatus::Skipped)
        );
        assert_eq!(
            applicable_installation(IdentityRead::Found(lite())),
            Ok(lite())
        );
    }
}
