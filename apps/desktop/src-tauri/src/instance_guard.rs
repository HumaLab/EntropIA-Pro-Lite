//! One EntropIA at a time.
//!
//! Lite, Pro and the dev build share one archive (`path_utils::SHARED_DIR_NAME`)
//! and are used in turns, never together. Two at once is not merely redundant:
//! on 2026-09-19 a Store (MSIX) build opened while the dev build had the
//! archive open, and stale pages from the package's private copy ended up in
//! the real database, which no longer opened. So the second process to start
//! refuses, whichever variant it is.
//!
//! # Why a named mutex on Windows, not a lock file
//!
//! MSIX redirects writes under AppData into a per-package copy. A lock file in
//! the data directory could be copied the same way, and then each process
//! would hold a lock on its own copy and never see the other — failing exactly
//! in the case this exists for. A named kernel object lives in the session's
//! object namespace, not on disk, so every variant sees the same one.
//!
//! Elsewhere there is no such redirection, and an OS file lock in the data
//! directory does the job. Either way the operating system releases it when the
//! process ends, crash included, so a dead instance never blocks the next.
//!
//! `processing::recovery` still handles a live peer on the archive. With this
//! guard that peer should no longer exist; the handling stays as a second line.

use std::path::Path;

/// The name every variant uses. Changing it lets old and new builds run
/// together again — it is part of the contract between releases.
pub const INSTANCE_NAME: &str = "EntropIA-archive";

#[derive(Debug)]
pub enum GuardError {
    /// Another EntropIA holds the archive.
    AlreadyRunning,
    /// The guard itself could not be set up. Not proof that another instance
    /// is running, so the caller decides whether that is fatal.
    Unavailable(String),
}

/// Held for the life of the process. Dropping it lets the next instance in.
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: windows::Handle,
    #[cfg(not(windows))]
    _file: std::fs::File,
}

/// Claims the archive for this process. `data_dir` is only used where a lock
/// file stands in for the named mutex.
pub fn acquire(data_dir: &Path) -> Result<InstanceGuard, GuardError> {
    acquire_named(INSTANCE_NAME, data_dir)
}

#[cfg(windows)]
fn acquire_named(name: &str, _data_dir: &Path) -> Result<InstanceGuard, GuardError> {
    windows::create_mutex(&format!("Local\\{name}")).map(|handle| InstanceGuard { handle })
}

#[cfg(not(windows))]
fn acquire_named(name: &str, data_dir: &Path) -> Result<InstanceGuard, GuardError> {
    use fs2::FileExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data_dir.join(format!("{name}.instance.lock")))
        .map_err(|e| GuardError::Unavailable(e.to_string()))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(InstanceGuard { _file: file }),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            Err(GuardError::AlreadyRunning)
        }
        Err(error) => Err(GuardError::Unavailable(error.to_string())),
    }
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        windows::close(&self.handle);
    }
}

#[cfg(windows)]
mod windows {
    use super::GuardError;
    use std::ffi::c_void;

    // Three kernel32 calls. The standard library already links kernel32, so
    // declaring them costs no dependency.
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attributes: *const c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut c_void;
        fn GetLastError() -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const ERROR_ACCESS_DENIED: u32 = 5;

    pub struct Handle(*mut c_void);

    // The handle is only ever closed, never shared for use across threads.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}

    pub fn create_mutex(name: &str) -> Result<Handle, GuardError> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; null
        // attributes are allowed and mean the default security descriptor.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        // SAFETY: read immediately after the call it reports on.
        let last = unsafe { GetLastError() };

        if handle.is_null() {
            // A mutex someone else created with a stricter descriptor exists:
            // that is another instance as surely as ALREADY_EXISTS is.
            if last == ERROR_ACCESS_DENIED {
                return Err(GuardError::AlreadyRunning);
            }
            return Err(GuardError::Unavailable(format!(
                "CreateMutexW failed: {last}"
            )));
        }
        if last == ERROR_ALREADY_EXISTS {
            // SAFETY: a handle CreateMutexW just returned, closed once.
            unsafe { CloseHandle(handle) };
            return Err(GuardError::AlreadyRunning);
        }
        Ok(Handle(handle))
    }

    pub fn close(handle: &Handle) {
        // SAFETY: closed exactly once, from the guard's Drop.
        unsafe { CloseHandle(handle.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(label: &str) -> String {
        format!(
            "entropia-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn a_second_holder_is_refused_while_the_first_is_alive() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique("second");

        let _first = acquire_named(&name, dir.path()).expect("the first holder gets it");

        assert!(matches!(
            acquire_named(&name, dir.path()),
            Err(GuardError::AlreadyRunning)
        ));
    }

    /// Lite and Pro share one archive and are used in turns: closing one must
    /// leave the other free to open, with nothing to clean up by hand.
    #[test]
    fn the_guard_is_free_again_once_its_holder_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique("release");

        drop(acquire_named(&name, dir.path()).expect("first"));

        acquire_named(&name, dir.path()).expect("free again after the first let go");
    }

    /// The case that matters is another process, not another call: a separate
    /// PowerShell holds the mutex, and this process has to see it.
    #[cfg(windows)]
    #[test]
    fn another_process_holding_the_mutex_is_seen() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique("process");
        let ready = dir.path().join("ready");
        let script = format!(
            "$m = [System.Threading.Mutex]::new($false, 'Local\\{name}'); \
             Set-Content -Path '{}' -Value 1; Start-Sleep -Seconds 20",
            ready.display()
        );
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .spawn()
            .expect("start powershell");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the other process never took the mutex"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let refused = acquire_named(&name, dir.path());
        let _ = child.kill();
        let _ = child.wait();

        assert!(matches!(refused, Err(GuardError::AlreadyRunning)));
    }

    #[test]
    fn different_names_do_not_block_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();

        let _a = acquire_named(&unique("a"), dir.path()).expect("a");
        acquire_named(&unique("b"), other.path()).expect("b is independent");
    }
}
