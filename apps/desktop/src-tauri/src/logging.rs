//! Backend logging facade (plan §5.4).
//!
//! One global [`log::Log`] sink installed by [`init`] at the very start of
//! `run()`, with two destinations:
//!
//! - stderr, for every record at the enabled level, formatted
//!   `[<target>] <message>` — the same shape the old hand-written diagnostics
//!   used (`log::info!(target: "setup", "migrations: up to date")` prints
//!   `[setup] migrations: up to date`);
//! - the app's own log ([`crate::app_logs`]) for this crate's `warn` and
//!   `error`, once an `AppHandle` has been registered with [`attach_app`].
//!   Records emitted before that still reach stderr; they are not lost, only
//!   not persisted.
//!
//! A record without an explicit `target:` uses the last segment of its module
//! path (`entropia_desktop_lib::db::repairs` → `repairs`), which keeps plain
//! `log::info!(...)` calls readable instead of printing the full crate path.
//!
//! Levels follow the build: `Debug` and above in debug builds (parity with the
//! old verbose dev runs), `Info` and above in release. Dependency crates share
//! the global logger, but their records never reach the app log, and only their
//! `warn`/`error` records earn a stderr line: their info chatter is dropped.
//!
//! Recursion and deadlock safety: `app_logs` never calls the `log` facade, and
//! the app sink calls only `app_logs::record` (no stderr echo), so a record
//! cannot re-enter the logger. The attached handle is cloned out of the lock
//! before `app_logs` runs, so the logger holds no lock while the app-log
//! mutexes are taken.

use std::io::Write as _;
use std::sync::{OnceLock, RwLock};

use tauri::AppHandle;

use crate::app_logs::{self, AppLogLevel};

/// The registered app handle, if `attach_app` has run. `RwLock` so logging
/// from any thread stays safe while `attach_app` runs on the main thread.
static APP_HANDLE: OnceLock<RwLock<Option<AppHandle>>> = OnceLock::new();

fn app_handle_slot() -> &'static RwLock<Option<AppHandle>> {
    APP_HANDLE.get_or_init(|| RwLock::new(None))
}

/// Registers the handle the app-log sink writes to. Called from `setup` right
/// after `AppLogsState` is managed; before that, records only reach stderr.
pub fn attach_app(handle: AppHandle) {
    let slot = app_handle_slot();
    *slot.write().unwrap_or_else(|poison| poison.into_inner()) = Some(handle);
}

/// A snapshot of the registered handle, cloned so the lock is released before
/// `app_logs` runs (it takes its own locks).
fn attached_app() -> Option<AppHandle> {
    app_handle_slot()
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

struct BackendLogger;

impl log::Log for BackendLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let app = attached_app();
        let mut stderr_sink = |line: &str| {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "{line}");
        };
        let mut app_sink = |level: AppLogLevel, source: &str, message: &str| {
            if let Some(handle) = app.as_ref() {
                app_logs::record(handle, level, source, message);
            }
        };
        dispatch_record(record, &mut stderr_sink, &mut app_sink);
    }

    fn flush(&self) {}
}

static LOGGER: BackendLogger = BackendLogger;

/// Installs the facade. Safe to call more than once: the first logger wins, as
/// `log::set_logger` requires. Called at the very start of `run()`, before any
/// startup diagnostics.
pub fn init() {
    if log::set_logger(&LOGGER).is_err() {
        return;
    }
    log::set_max_level(if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });
}

/// Resolves the target a record prints under: an explicit `target:` is used
/// verbatim; a record without one uses the last segment of its module path.
fn resolve_target<'a>(target: &'a str, module_path: Option<&'a str>) -> &'a str {
    match module_path {
        Some(module) if module == target => module.rsplit("::").next().unwrap_or(module),
        _ => target,
    }
}

/// The stderr shape, byte-identical to the old hand-written diagnostics.
fn format_stderr_line(target: &str, message: &str) -> String {
    format!("[{target}] {message}")
}

/// The app-log level a record is persisted under, or `None` when it stays
/// stderr-only. `warn` and `error` land in the app log; `info` and `debug` do
/// not (per-step success lines would flood it).
fn app_sink_level(level: log::Level) -> Option<AppLogLevel> {
    match level {
        log::Level::Warn => Some(AppLogLevel::Warn),
        log::Level::Error => Some(AppLogLevel::Error),
        log::Level::Info | log::Level::Debug | log::Level::Trace => None,
    }
}

/// This crate's name as it appears at the head of our module paths, derived
/// from `module_path!()` so it can never drift from `[lib] name` in Cargo.toml.
fn crate_name() -> &'static str {
    let path = module_path!();
    path.split("::").next().unwrap_or(path)
}

/// Whether a record came from this crate. `tauri`/`wry`/`reqwest` and friends
/// share the global logger; their records must stay out of the user's app log.
fn is_our_crate(module_path: Option<&str>) -> bool {
    module_path.and_then(|module| module.split("::").next()) == Some(crate_name())
}

/// Routes one record to its sinks. Shared by the production logger and the
/// test sink so formatting and routing are exercised through one path.
///
/// Dependency records only earn a stderr line at `warn` or worse, and never
/// reach the app sink: a `[wry] …` line is not something the user asked to
/// persist.
fn dispatch_record(
    record: &log::Record,
    stderr_sink: &mut dyn FnMut(&str),
    app_sink: &mut dyn FnMut(AppLogLevel, &str, &str),
) {
    let our_crate = is_our_crate(record.module_path());
    if !our_crate && record.level() > log::Level::Warn {
        return;
    }

    let target = resolve_target(record.target(), record.module_path());
    let message = record.args().to_string();
    stderr_sink(&format_stderr_line(target, &message));
    let app_level = if our_crate {
        app_sink_level(record.level())
    } else {
        None
    };
    if let Some(level) = app_level {
        app_sink(level, target, &message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A `log::Log`-shaped sink that records into vectors instead of stderr
    /// and the app log, proving formatting and routing without touching the
    /// process-wide logger.
    #[derive(Default)]
    struct CaptureSink {
        stderr: Mutex<Vec<String>>,
        app: Mutex<Vec<(AppLogLevel, String, String)>>,
    }

    impl CaptureSink {
        fn log_record(&self, record: &log::Record) {
            let mut stderr_sink = |line: &str| {
                self.stderr
                    .lock()
                    .expect("capture stderr")
                    .push(line.to_string());
            };
            let mut app_sink = |level: AppLogLevel, source: &str, message: &str| {
                self.app.lock().expect("capture app").push((
                    level,
                    source.to_string(),
                    message.to_string(),
                ));
            };
            dispatch_record(record, &mut stderr_sink, &mut app_sink);
        }

        fn stderr_lines(&self) -> Vec<String> {
            self.stderr.lock().expect("capture stderr").clone()
        }

        fn app_entries(&self) -> Vec<(AppLogLevel, String, String)> {
            self.app.lock().expect("capture app").clone()
        }
    }

    fn log_line(sink: &CaptureSink, level: log::Level, target: &str, message: &str) {
        sink.log_record(
            &log::Record::builder()
                .level(level)
                .target(target)
                .module_path(Some(module_path!()))
                .args(format_args!("{message}"))
                .build(),
        );
    }

    /// A record shaped like one a dependency crate emits: target and module
    /// path are the crate's own path (no explicit `target:` override).
    fn log_line_from_module(sink: &CaptureSink, level: log::Level, module: &str, message: &str) {
        sink.log_record(
            &log::Record::builder()
                .level(level)
                .target(module)
                .module_path(Some(module))
                .args(format_args!("{message}"))
                .build(),
        );
    }

    #[test]
    fn explicit_target_formats_the_old_eprintln_line() {
        let sink = CaptureSink::default();
        log_line(&sink, log::Level::Info, "setup", "migrations: up to date");
        assert_eq!(sink.stderr_lines(), vec!["[setup] migrations: up to date"]);
        assert!(sink.app_entries().is_empty());
    }

    #[test]
    fn default_target_uses_the_last_module_segment() {
        assert_eq!(
            resolve_target(
                "entropia_desktop_lib::db::repairs",
                Some("entropia_desktop_lib::db::repairs")
            ),
            "repairs"
        );
        assert_eq!(
            resolve_target(
                "entropia_desktop_lib::lib",
                Some("entropia_desktop_lib::lib")
            ),
            "lib"
        );
        assert_eq!(
            resolve_target("setup", Some("entropia_desktop_lib::lib")),
            "setup"
        );
        assert_eq!(
            resolve_target("entropia_desktop_lib", None),
            "entropia_desktop_lib"
        );
    }

    #[test]
    fn dependency_warn_and_error_reach_stderr_but_not_the_app_sink() {
        let sink = CaptureSink::default();
        log_line_from_module(
            &sink,
            log::Level::Warn,
            "wry::webview",
            "webview process gone",
        );
        log_line_from_module(
            &sink,
            log::Level::Error,
            "reqwest::blocking",
            "connection reset",
        );

        assert_eq!(
            sink.stderr_lines(),
            vec![
                "[webview] webview process gone",
                "[blocking] connection reset"
            ]
        );
        assert!(sink.app_entries().is_empty());
    }

    #[test]
    fn dependency_info_is_dropped() {
        let sink = CaptureSink::default();
        log_line_from_module(
            &sink,
            log::Level::Info,
            "reqwest::blocking",
            "request completed",
        );

        assert!(sink.stderr_lines().is_empty());
        assert!(sink.app_entries().is_empty());
    }

    #[test]
    fn our_crate_warn_reaches_both_sinks() {
        let sink = CaptureSink::default();
        log_line_from_module(
            &sink,
            log::Level::Warn,
            "entropia_desktop_lib::db::repairs",
            "legacy schema left untouched",
        );

        assert_eq!(
            sink.stderr_lines(),
            vec!["[repairs] legacy schema left untouched"]
        );
        let app = sink.app_entries();
        assert_eq!(app.len(), 1);
        assert!(matches!(app[0].0, AppLogLevel::Warn));
        assert_eq!(app[0].1, "repairs");
        assert_eq!(app[0].2, "legacy schema left untouched");
    }

    #[test]
    fn info_and_debug_stay_on_stderr() {
        let sink = CaptureSink::default();
        log_line(&sink, log::Level::Info, "sync", "engine spawned");
        log_line(&sink, log::Level::Debug, "sync", "dev detail");
        assert_eq!(
            sink.stderr_lines(),
            vec!["[sync] engine spawned", "[sync] dev detail"]
        );
        assert!(sink.app_entries().is_empty());
    }

    #[test]
    fn warn_and_error_reach_stderr_and_the_app_sink() {
        let sink = CaptureSink::default();
        log_line(
            &sink,
            log::Level::Warn,
            "sync",
            "orphan .part cleanup failed: boom",
        );
        log_line(
            &sink,
            log::Level::Error,
            "runtime",
            "startup validation failed: boom",
        );

        assert_eq!(
            sink.stderr_lines(),
            vec![
                "[sync] orphan .part cleanup failed: boom",
                "[runtime] startup validation failed: boom",
            ]
        );

        let app = sink.app_entries();
        assert_eq!(app.len(), 2);
        assert!(matches!(app[0].0, AppLogLevel::Warn));
        assert_eq!(app[0].1, "sync");
        assert_eq!(app[0].2, "orphan .part cleanup failed: boom");
        assert!(matches!(app[1].0, AppLogLevel::Error));
        assert_eq!(app[1].1, "runtime");
        assert_eq!(app[1].2, "startup validation failed: boom");
    }

    #[test]
    fn records_before_attach_are_not_lost_to_stderr() {
        // Nothing in the test process calls `attach_app`, so this is exactly
        // the before-attach path: the stderr sink is unconditional and the app
        // sink is only ever wired once a handle is registered.
        assert!(attached_app().is_none());
        let sink = CaptureSink::default();
        log_line(
            &sink,
            log::Level::Warn,
            "setup",
            "instance guard unavailable: boom",
        );
        assert_eq!(
            sink.stderr_lines(),
            vec!["[setup] instance guard unavailable: boom"]
        );
    }

    #[test]
    fn app_sink_level_maps_only_warn_and_error() {
        assert!(app_sink_level(log::Level::Trace).is_none());
        assert!(app_sink_level(log::Level::Debug).is_none());
        assert!(app_sink_level(log::Level::Info).is_none());
        assert!(matches!(
            app_sink_level(log::Level::Warn),
            Some(AppLogLevel::Warn)
        ));
        assert!(matches!(
            app_sink_level(log::Level::Error),
            Some(AppLogLevel::Error)
        ));
    }
}
