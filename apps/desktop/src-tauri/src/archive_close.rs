//! The close-time half of the archive compaction (`db::compact` decides and
//! does it; this module makes sure the app is out of the way first).
//!
//! `run_close_sequence` calls [`run`] after the canonical save and the sync
//! cycle, before the window is destroyed. The window stays open the whole time
//! and the frontend shows a notice, driven by [`COMPACTING_EVENT`].
//!
//! Exclusivity, in the order it is taken:
//! 1. the sync cycle must have settled (a cycle still running is a live writer);
//! 2. the sync engine is told to shut down so it starts no new cycle;
//! 3. the processing scheduler is stopped and must have exited, which also
//!    means no task is mid-run;
//! 4. both connections the app itself keeps (UI and worker) are locked, so no
//!    command can write through them;
//! 5. every other connection (background workers, sweeps) is covered by the
//!    TRUNCATE checkpoint inside `compact_archive`, which skips when it cannot
//!    finish within a short wait.
//!
//! Any step that cannot be taken quickly skips the compaction until the next
//! close. Nothing here can fail the close: every error ends in a log line.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tauri::{AppHandle, Emitter as _, Manager as _};

use crate::db::compact::{self, Outcome, Policy};
use crate::db::state::AppDbState;

/// Tauri event carrying `true` when the compaction starts and `false` when it
/// ends (whatever the outcome), so the frontend shows and clears its notice.
pub const COMPACTING_EVENT: &str = "app:compacting";
const LOG_SOURCE: &str = "app/close";
/// Time the notice gets to paint before the database goes quiet.
const NOTICE_PAINT: Duration = Duration::from_millis(400);
/// The scheduler checks its stop flag between ticks (idle tick: 2 s).
const SCHEDULER_STOP_BUDGET: Duration = Duration::from_secs(5);
/// How long the app's own connections may take to come free.
const APP_CONNECTIONS_BUDGET: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);

/// Stops the processing supervisor thread on demand.
pub struct SchedulerControl {
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SchedulerControl {
    pub fn new(stop: Arc<AtomicBool>, thread: JoinHandle<()>) -> Self {
        Self {
            stop,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// Raises the stop flag and waits up to `budget` for the thread to exit.
    /// `false` means it is still running a task (or the tick of one).
    pub fn stop_and_wait(&self, budget: Duration) -> bool {
        self.stop.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + budget;
        let mut slot = self.thread.lock().unwrap_or_else(|p| p.into_inner());
        let Some(thread) = slot.as_ref() else {
            return true;
        };
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(POLL);
        }
        if let Some(thread) = slot.take() {
            let _ = thread.join();
        }
        true
    }
}

/// Locks the UI and worker connections, waiting up to `budget` for commands
/// still using them. `None` when either stays busy.
pub fn lock_app_connections<'a>(
    db: &'a AppDbState,
    budget: Duration,
) -> Option<(MutexGuard<'a, Connection>, MutexGuard<'a, Connection>)> {
    let deadline = Instant::now() + budget;
    loop {
        if let (Ok(ui), Ok(worker)) = (db.ui_conn.try_lock(), db.worker_conn.try_lock()) {
            return Some((ui, worker));
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL);
    }
}

fn log(app: &AppHandle, message: impl Into<String>) {
    crate::app_logs::info(app, LOG_SOURCE, message);
}

/// The whole close-time pass. Returns when the archive is compacted or the
/// reason it was not is logged; the caller closes the window next either way.
pub async fn run(app: &AppHandle, sync_settled: bool) {
    let Some(db_path) = app.try_state::<AppDbState>().map(|db| db.db_path.clone()) else {
        return;
    };
    if !sync_settled {
        log(app, "Compactación omitida: el ciclo de sync sigue en curso");
        return;
    }
    let policy = Policy::default();

    // Cheap read-only probe first: most closes stop here, with no notice and
    // no worker stopped.
    let probe_path = db_path.clone();
    let probe =
        tauri::async_runtime::spawn_blocking(move || compact::is_worthwhile(&probe_path, &policy))
            .await;
    match probe {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(reason))) => {
            // Only worth a line when it is a real decision; an archive that
            // reclaims by itself says nothing every close.
            if !matches!(reason, compact::SkipReason::NotNeeded(_)) {
                log(app, Outcome::Skipped(reason).summary());
            }
            return;
        }
        Ok(Err(error)) => {
            log(app, format!("Compactación omitida: {error}"));
            return;
        }
        Err(error) => {
            log(app, format!("Compactación omitida: {error}"));
            return;
        }
    }

    let _ = app.emit(COMPACTING_EVENT, true);
    pause(NOTICE_PAINT).await;
    let outcome = compact_quietly(app, db_path, policy).await;
    log(app, outcome);
    let _ = app.emit(COMPACTING_EVENT, false);
}

async fn pause(duration: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration)).await;
}

/// Quiesces the app and compacts; the returned text is the log line.
async fn compact_quietly(app: &AppHandle, db_path: std::path::PathBuf, policy: Policy) -> String {
    if let Some(engine) = app.try_state::<crate::sync::engine::SyncEngine>() {
        engine.shutdown();
    }
    let app_for_work = app.clone();
    let work = tauri::async_runtime::spawn_blocking(move || -> Result<Outcome, String> {
        if let Some(scheduler) = app_for_work.try_state::<SchedulerControl>() {
            if !scheduler.stop_and_wait(SCHEDULER_STOP_BUDGET) {
                return Ok(Outcome::Skipped(compact::SkipReason::InUse));
            }
        }
        let db = app_for_work.try_state::<AppDbState>();
        let _guards = match db.as_deref() {
            Some(db) => match lock_app_connections(db, APP_CONNECTIONS_BUDGET) {
                Some(guards) => Some(guards),
                None => return Ok(Outcome::Skipped(compact::SkipReason::InUse)),
            },
            None => None,
        };
        compact::compact_archive(&db_path, &policy, &compact::available_disk_space)
    })
    .await;
    match work {
        Ok(Ok(outcome)) => outcome.summary(),
        Ok(Err(error)) => format!("Compactación fallida (el archivo sigue intacto): {error}"),
        Err(error) => format!("Compactación interrumpida: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheduler_that_exits_stops_cleanly() {
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !seen.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let control = SchedulerControl::new(stop, thread);

        assert!(control.stop_and_wait(Duration::from_secs(2)));
        // Idempotent: a second call finds nothing left to wait for.
        assert!(control.stop_and_wait(Duration::from_millis(10)));
    }

    #[test]
    fn a_scheduler_stuck_in_a_task_makes_the_compaction_skip() {
        let stop = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let hold = Arc::clone(&release);
        // Ignores the stop flag, as a long OCR or embedding run does.
        let thread = std::thread::spawn(move || {
            while !hold.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let control = SchedulerControl::new(stop, thread);

        let started = Instant::now();
        assert!(!control.stop_and_wait(Duration::from_millis(200)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the wait is bounded"
        );

        release.store(true, Ordering::SeqCst);
        assert!(control.stop_and_wait(Duration::from_secs(2)));
    }

    fn state() -> AppDbState {
        AppDbState::new(
            Connection::open_in_memory().unwrap(),
            Connection::open_in_memory().unwrap(),
            std::path::PathBuf::from("unused"),
        )
    }

    #[test]
    fn the_apps_own_connections_are_taken_when_idle() {
        let db = state();
        assert!(lock_app_connections(&db, Duration::from_millis(100)).is_some());
    }

    #[test]
    fn a_command_still_using_a_connection_blocks_the_compaction() {
        let db = state();
        let busy = db.ui_conn.lock().unwrap();
        let started = Instant::now();

        assert!(lock_app_connections(&db, Duration::from_millis(150)).is_none());

        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the wait is bounded"
        );
        drop(busy);
        assert!(lock_app_connections(&db, Duration::from_millis(100)).is_some());
    }
}
