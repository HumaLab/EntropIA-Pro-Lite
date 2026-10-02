//! Starting Zotero in the background so a waiting copy can go through.
//!
//! Only ever on an explicit action of the person, never from a drain. The
//! executable comes from a short list of install locations that are checked on
//! disk (no shell, no search of the PATH, no registry read: the installer's
//! default folders are what this build knows), it is started with no arguments,
//! and a second start inside [`MIN_GAP`] is refused so a button pressed twice
//! cannot open two Zotero windows.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// The shortest time between two starts.
pub const MIN_GAP: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Mac,
    Other,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchOutcome {
    AlreadyRunning,
    Started,
    NotFound,
    TooSoon,
    Unsupported,
    SpawnFailed,
}

/// The installed Zotero, from the installer's default folders. `env` reads an
/// environment variable and `exists` asks the disk; both are injected so the
/// search is testable.
pub fn find_executable(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<String>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match platform {
        Platform::Windows => ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(|name| env(name))
            .map(|base| PathBuf::from(base).join("Zotero").join("zotero.exe"))
            .collect(),
        Platform::Mac => vec![PathBuf::from("/Applications/Zotero.app")],
        Platform::Other => Vec::new(),
    };
    candidates.into_iter().find(|path| exists(path))
}

/// The decision, pure: whether to start `exe` now. `last` remembers the last
/// successful start; a failed spawn does not count.
pub fn decide(
    platform: Platform,
    reachable: bool,
    now: Instant,
    last: &mut Option<Instant>,
    exe: Option<PathBuf>,
    spawn: &mut dyn FnMut(&Path) -> std::io::Result<()>,
) -> LaunchOutcome {
    if reachable {
        return LaunchOutcome::AlreadyRunning;
    }
    if platform == Platform::Other {
        return LaunchOutcome::Unsupported;
    }
    let Some(exe) = exe else {
        return LaunchOutcome::NotFound;
    };
    if let Some(previous) = *last {
        if now.saturating_duration_since(previous) < MIN_GAP {
            return LaunchOutcome::TooSoon;
        }
    }
    match spawn(&exe) {
        Ok(()) => {
            *last = Some(now);
            LaunchOutcome::Started
        }
        Err(_) => LaunchOutcome::SpawnFailed,
    }
}

static LAST_START: Mutex<Option<Instant>> = Mutex::new(None);

/// Starts the installed Zotero, detached and with no arguments. No shell: the
/// path is spawned directly (macOS goes through `open`, with the app bundle as
/// its only argument).
pub fn launch(reachable: bool) -> LaunchOutcome {
    let platform = Platform::current();
    let exe = find_executable(
        platform,
        &|name: &str| std::env::var(name).ok(),
        &|path: &Path| path.exists(),
    );
    let mut last = LAST_START
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    decide(
        platform,
        reachable,
        Instant::now(),
        &mut last,
        exe,
        &mut |path: &Path| spawn_detached(path),
    )
}

fn spawn_detached(path: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = Command::new(path);

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: Zotero outlives this app.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    command.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn env(name: &str) -> Option<String> {
        match name {
            "ProgramFiles" => Some(r"C:\Program Files".into()),
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".into()),
            "LOCALAPPDATA" => Some(r"C:\Users\x\AppData\Local".into()),
            _ => None,
        }
    }

    #[test]
    fn windows_checks_the_installer_folders_in_order() {
        let found = find_executable(Platform::Windows, &env, &|path: &Path| {
            path.ends_with("Zotero/zotero.exe") || path.ends_with(r"Zotero\zotero.exe")
        });
        assert_eq!(
            found,
            Some(
                PathBuf::from(r"C:\Program Files")
                    .join("Zotero")
                    .join("zotero.exe")
            )
        );

        let only_user = find_executable(Platform::Windows, &env, &|path: &Path| {
            path.to_string_lossy().contains("AppData")
        });
        assert!(only_user.unwrap().to_string_lossy().contains("AppData"));
    }

    #[test]
    fn nothing_on_disk_is_not_found_and_other_platforms_are_unsupported() {
        assert_eq!(
            find_executable(Platform::Windows, &env, &|_: &Path| false),
            None
        );
        assert_eq!(
            find_executable(Platform::Other, &env, &|_: &Path| true),
            None
        );
        assert_eq!(
            find_executable(Platform::Mac, &env, &|path: &Path| path
                == Path::new("/Applications/Zotero.app")),
            Some(PathBuf::from("/Applications/Zotero.app"))
        );
    }

    #[test]
    fn a_running_zotero_is_never_started_again() {
        let spawned = Cell::new(0);
        let mut last = None;
        let outcome = decide(
            Platform::Windows,
            true,
            Instant::now(),
            &mut last,
            Some(PathBuf::from("zotero.exe")),
            &mut |_: &Path| {
                spawned.set(spawned.get() + 1);
                Ok(())
            },
        );
        assert_eq!(outcome, LaunchOutcome::AlreadyRunning);
        assert_eq!(spawned.get(), 0);
    }

    #[test]
    fn no_executable_is_reported_and_nothing_is_spawned() {
        let mut last = None;
        let outcome = decide(
            Platform::Windows,
            false,
            Instant::now(),
            &mut last,
            None,
            &mut |_: &Path| panic!("must not spawn"),
        );
        assert_eq!(outcome, LaunchOutcome::NotFound);
        assert_eq!(
            decide(
                Platform::Other,
                false,
                Instant::now(),
                &mut last,
                None,
                &mut |_: &Path| panic!()
            ),
            LaunchOutcome::Unsupported
        );
    }

    #[test]
    fn a_second_start_inside_the_gap_is_refused_and_one_after_it_is_allowed() {
        let spawned = Cell::new(0);
        let mut last = None;
        let t0 = Instant::now();
        let exe = || Some(PathBuf::from("zotero.exe"));
        let mut spawn = |_: &Path| {
            spawned.set(spawned.get() + 1);
            Ok(())
        };
        assert_eq!(
            decide(Platform::Windows, false, t0, &mut last, exe(), &mut spawn),
            LaunchOutcome::Started
        );
        assert_eq!(
            decide(
                Platform::Windows,
                false,
                t0 + Duration::from_secs(5),
                &mut last,
                exe(),
                &mut spawn
            ),
            LaunchOutcome::TooSoon
        );
        assert_eq!(
            decide(
                Platform::Windows,
                false,
                t0 + MIN_GAP + Duration::from_secs(1),
                &mut last,
                exe(),
                &mut spawn
            ),
            LaunchOutcome::Started
        );
        assert_eq!(spawned.get(), 2);
    }

    #[test]
    fn a_failed_spawn_does_not_burn_the_gap() {
        let mut last = None;
        let t0 = Instant::now();
        let exe = Some(PathBuf::from("zotero.exe"));
        let failed = decide(
            Platform::Windows,
            false,
            t0,
            &mut last,
            exe.clone(),
            &mut |_: &Path| Err(std::io::Error::other("denied")),
        );
        assert_eq!(failed, LaunchOutcome::SpawnFailed);
        let ok = decide(
            Platform::Windows,
            false,
            t0,
            &mut last,
            exe,
            &mut |_: &Path| Ok(()),
        );
        assert_eq!(ok, LaunchOutcome::Started);
    }

    #[test]
    fn outcomes_reach_the_ui_as_snake_case_words() {
        assert_eq!(
            serde_json::to_value(LaunchOutcome::AlreadyRunning).unwrap(),
            "already_running"
        );
        assert_eq!(
            serde_json::to_value(LaunchOutcome::SpawnFailed).unwrap(),
            "spawn_failed"
        );
        assert_eq!(
            serde_json::to_value(LaunchOutcome::TooSoon).unwrap(),
            "too_soon"
        );
    }
}
