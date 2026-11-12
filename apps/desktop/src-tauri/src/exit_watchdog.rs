//! Makes sure the process ends once the main window is gone.
//!
//! Destroying the main window normally leads, within milliseconds, to
//! `RunEvent::ExitRequested` and `RunEvent::Exit`. On Windows that last step
//! waits for WebView2: with its browser or GPU process unresponsive, the window
//! is destroyed (`WindowEvent::Destroyed` arrives) but the event loop never
//! reaches `Exit`, and the process stays alive with no window. It keeps the
//! instance guard (the next launch reports "EntropIA ya está abierta") and the
//! sync engine running against the archive.
//!
//! Reproduced by suspending the WebView2 GPU or browser process before closing:
//! `Exit` arrived only once the process was resumed, 15 s later.
//!
//! With the browser process hung it is worse: `Destroyed` never arrives at all.
//! So the watchdog is armed as soon as the close sequence (or a repeated close)
//! destroys the main window, and again on `Destroyed` for any other way the
//! window goes; only the first arming counts. If the normal exit
//! path has not run when the grace period ends, it runs the fallback: the same
//! cleanup `Exit` does, then `std::process::exit`. Ending there is safe because
//! everything that matters is already durable: SQLite commits are
//! `synchronous=FULL` in WAL mode and the sync queues resume on the next launch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How long the process may take to reach `RunEvent::Exit` after the main
/// window is destroyed. A healthy exit takes milliseconds; the close sequence
/// has already waited up to six seconds before destroying the window.
pub const EXIT_GRACE: Duration = Duration::from_secs(3);

/// Shared between the run-event handler (which arms it on `Destroyed` and marks
/// the normal exit) and the countdown thread.
#[derive(Clone, Default)]
pub struct ExitWatchdog {
    armed: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
}

impl ExitWatchdog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that the normal exit path is running; an armed countdown then
    /// stands down instead of forcing the exit.
    pub fn mark_exited(&self) {
        self.exited.store(true, Ordering::SeqCst);
    }

    /// Starts the countdown. Only the first call arms it; later calls return
    /// `false` and do nothing. When `grace` elapses without [`Self::mark_exited`],
    /// `on_timeout` runs on the countdown thread.
    pub fn arm<F>(&self, grace: Duration, on_timeout: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        if self.armed.swap(true, Ordering::SeqCst) {
            return false;
        }
        let exited = Arc::clone(&self.exited);
        std::thread::Builder::new()
            .name("exit-watchdog".to_string())
            .spawn(move || {
                std::thread::sleep(grace);
                if !exited.load(Ordering::SeqCst) {
                    on_timeout();
                }
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const SHORT: Duration = Duration::from_millis(50);
    const WAIT: Duration = Duration::from_secs(2);

    #[test]
    fn forces_the_exit_when_the_normal_exit_never_comes() {
        let watchdog = ExitWatchdog::new();
        let (tx, rx) = mpsc::channel();

        assert!(watchdog.arm(SHORT, move || tx.send(()).unwrap()));

        assert_eq!(rx.recv_timeout(WAIT), Ok(()));
    }

    #[test]
    fn stands_down_when_the_normal_exit_runs_in_time() {
        let watchdog = ExitWatchdog::new();
        let (tx, rx) = mpsc::channel();

        assert!(watchdog.arm(Duration::from_millis(200), move || tx.send(()).unwrap()));
        watchdog.mark_exited();

        assert!(rx.recv_timeout(Duration::from_millis(600)).is_err());
    }

    #[test]
    fn stands_down_when_the_exit_ran_before_it_was_armed() {
        let watchdog = ExitWatchdog::new();
        let (tx, rx) = mpsc::channel();

        watchdog.mark_exited();
        assert!(watchdog.arm(SHORT, move || tx.send(()).unwrap()));

        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
    }

    #[test]
    fn arms_only_once() {
        let watchdog = ExitWatchdog::new();
        let (tx, rx) = mpsc::channel();
        let second = tx.clone();

        assert!(watchdog.arm(SHORT, move || tx.send(1).unwrap()));
        assert!(!watchdog.arm(SHORT, move || second.send(2).unwrap()));

        assert_eq!(rx.recv_timeout(WAIT), Ok(1));
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn a_clone_shares_the_same_countdown() {
        let watchdog = ExitWatchdog::new();
        let handler_copy = watchdog.clone();
        let (tx, rx) = mpsc::channel();

        assert!(watchdog.arm(Duration::from_millis(200), move || tx.send(()).unwrap()));
        handler_copy.mark_exited();

        assert!(rx.recv_timeout(Duration::from_millis(600)).is_err());
    }
}
