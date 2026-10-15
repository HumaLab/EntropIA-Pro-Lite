//! Secure private staging for the WebView2 repair wrapper (MSI-1a).
//!
//! Cohesive native staging/security unit: unique temp names, exclusive writes,
//! private directory creation, read-locked integrity check, bounded read-back,
//! directory-identity hold, and owned-only cleanup. Compiles standalone as
//! `rustc --test webview2_stage.rs` AND imported by `webview2-repair.rs` via
//! `#[path = "webview2_stage.rs"] mod stage;`. Tests use only synthetic
//! payloads — no `WEBVIEW2_BOOTSTRAPPER_PATH`, no network, no WebView2 launch.
//!
//! Security model: the payload is trusted (verified before bundling) but the
//! staging area is not. The stage directory is created exclusively under the
//! system temp root with a unique name and a private DACL applied atomically
//! at creation on Windows (`CreateDirectoryW` with `SECURITY_ATTRIBUTES`,
//! never create-then-chmod). Only the owned file and directory are removed
//! afterwards — never a recursive delete.
//!
//! Windows DACL: protected from inheritance; full control for SYSTEM,
//! Administrators, Owner Rights, and Creator Owner. `OW` (Owner Rights) is
//! required in addition to `CO`: `CO` alone only flows via inheritance, while
//! `OW` grants the current owner (standard non-admin user) directly, which
//! matters even when CI runs elevated. No ACE grants anyone else.
//!
//! The prior create-dir-then-`SetFileSecurityByPath` ordering is treated as an
//! unproven hardening point (not a proven exploit): this module removes the
//! window entirely by passing the descriptor at creation. No global ACL
//! modifications, no handle following into pre-existing directories.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique, non-guessable stage file/directory name.
pub fn unique_stage_name(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let count = STAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "entropia-webview2-{}-{:x}-{:x}-{:x}",
        tag,
        std::process::id(),
        nanos,
        count
    )
}

/// Byte-exact payload check used after the locked read-back.
pub fn payload_matches(expected: &[u8], actual: &[u8]) -> bool {
    expected == actual
}

/// Atomically create `dir/name` with exactly `bytes`.
///
/// `create_new` fails when the name already exists instead of truncating or
/// following a planted file: a collision fails closed and keeps the
/// pre-existing bytes.
pub fn write_exclusive_file(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let path = dir.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    use std::io::Write as _;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(path)
}

/// Exclusively create a uniquely named private stage directory.
///
/// Windows: `CreateDirectoryW` with private `SECURITY_ATTRIBUTES` from
/// creation (no post-create ACL window). POSIX fallback (tests only):
/// `create_dir` + `0700` so Linux CI exercises the same round-trip logic.
pub fn create_private_stage_dir() -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        windows::create_private_stage_dir_from_creation()
    }
    #[cfg(not(windows))]
    {
        let base = std::env::temp_dir();
        for _ in 0..32 {
            let dir = base.join(unique_stage_name("stage"));
            match fs::create_dir(&dir) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt as _;
                        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
                    }
                    return Ok(dir);
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique private stage directory",
        ))
    }
}

/// Open the staged payload read-only, denying write/delete sharing to
/// everyone else on Windows. Hold the returned handle through child exit.
/// POSIX fallback is a plain read open (no mandatory locking there).
pub fn open_read_locked(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        windows::open_read_locked_impl(path)
    }
    #[cfg(not(windows))]
    {
        OpenOptions::new().read(true).open(path)
    }
}

/// Read back at most `expected_len + 1` bytes: a length mismatch (short,
/// long, or trailing bytes after a close/reopen race) fails the compare.
pub fn read_bounded(file: &mut File, expected_len: usize) -> io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    file.take(expected_len.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Best-effort cleanup of exactly the owned file and directory.
/// Never recursive; never touches anything outside these two paths.
pub fn cleanup_owned_stage(dir: &Path, file: &Path) {
    let _ = fs::remove_file(file);
    let _ = fs::remove_dir(dir);
}

/// Hold the stage directory identity (no-delete sharing) from creation
/// through the read-locked spawn when possible. Windows holds an open
/// directory handle without `FILE_SHARE_DELETE` so the container cannot be
/// renamed/removed mid-flight; POSIX is a no-op guard for test parity.
pub fn hold_stage_dir(dir: &Path) -> io::Result<DirGuard> {
    #[cfg(windows)]
    {
        windows::hold_stage_dir_impl(dir)
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        Ok(DirGuard { _private: () })
    }
}

#[cfg(not(windows))]
pub struct DirGuard {
    _private: (),
}

#[cfg(windows)]
mod windows {
    //! Windows-only staging: private creation-time ACL, share-denied locks.
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    type LPCWSTR = *const u16;
    type PSecDesc = *mut c_void;
    type HANDLE = *mut c_void;

    const SDDL_REVISION: u32 = 1;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const GENERIC_READ: u32 = 0x8000_0000;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const INVALID_HANDLE: HANDLE = (-1isize) as HANDLE;

    /// Protected DACL; full control for SYSTEM, Administrators, Owner Rights,
    /// and Creator Owner. `OW` grants the current (possibly standard
    /// non-admin) owner directly; `CO` alone would only apply via inheritance.
    const STAGE_SDDL: &str =
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)(A;OICI;FA;;;CO)";

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: PSecDesc,
        inherit: i32,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string_sd: LPCWSTR,
            revision: u32,
            sd: *mut PSecDesc,
            sd_size: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
        fn CreateDirectoryW(path: LPCWSTR, sa: *const SecurityAttributes) -> i32;
        fn CreateFileW(
            name: LPCWSTR,
            access: u32,
            share: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: HANDLE,
        ) -> HANDLE;
        fn CloseHandle(handle: HANDLE) -> i32;
    }

    fn wide_null(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn last_error(message: &str) -> io::Error {
        let os = io::Error::last_os_error();
        io::Error::new(os.kind(), format!("{message}: {os}"))
    }

    /// Create one uniquely named directory WITH the private descriptor.
    /// Pre-created (planted) names fail with `AlreadyExists` and are retried;
    /// any other failure fails closed without touching existing directories.
    pub fn create_private_stage_dir_from_creation() -> io::Result<PathBuf> {
        let sddl: Vec<u16> = STAGE_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sd: PSecDesc = std::ptr::null_mut();
        let mut sd_size: u32 = 0;
        // SAFETY: ConvertString... writes `sd` on success; freed once below.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION,
                &mut sd,
                &mut sd_size,
            )
        };
        if converted == 0 || sd.is_null() {
            return Err(last_error("convert stage security descriptor"));
        }
        let sa = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor: sd,
            inherit: 0,
        };
        let base = std::env::temp_dir();
        let mut result = Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique private stage directory",
        ));
        for _ in 0..32 {
            let dir = base.join(super::unique_stage_name("stage"));
            let path = wide_null(dir.as_os_str());
            // SAFETY: `path` is nul-terminated; `sa` borrows live `sd`.
            let created = unsafe { CreateDirectoryW(path.as_ptr(), &sa) };
            if created != 0 {
                result = Ok(dir);
                break;
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::AlreadyExists {
                result = Err(io::Error::new(
                    err.kind(),
                    format!("create private stage directory: {err}"),
                ));
                break;
            }
        }
        // SAFETY: `sd` came from the converter above; free exactly once.
        unsafe {
            LocalFree(sd);
        }
        result
    }

    pub fn open_read_locked_impl(path: &Path) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt as _;
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
    }

    pub fn close_dir_handle(handle: HANDLE) {
        if !handle.is_null() && handle != INVALID_HANDLE {
            // SAFETY: handle from CreateFileW below; closed exactly once via guard Drop.
            unsafe {
                CloseHandle(handle);
            }
        }
    }

    /// Open the just-created directory without delete sharing: while this
    /// guard lives, the container cannot be renamed or removed out from under
    /// the staged payload.
    pub fn hold_stage_dir_impl(dir: &Path) -> io::Result<super::DirGuard> {
        let path = wide_null(dir.as_os_str());
        // SAFETY: nul-terminated path; backup semantics allow directory open.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE {
            Err(last_error("hold stage directory identity"))
        } else {
            Ok(super::DirGuard { handle })
        }
    }
}

// Windows guard type (real handle holder). Declared here so the public
// `hold_stage_dir` signature is identical on both platforms.
#[cfg(windows)]
pub struct DirGuard {
    handle: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl Drop for DirGuard {
    fn drop(&mut self) {
        windows::close_dir_handle(self.handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn payload_compare_detects_any_difference() {
        assert!(payload_matches(b"abc", b"abc"));
        assert!(!payload_matches(b"abc", b"abd"));
        assert!(!payload_matches(b"abc", b"ab"));
        assert!(!payload_matches(b"abc", b"abcd"));
        assert!(!payload_matches(b"", b"x"));
    }

    #[test]
    fn exclusive_write_fails_closed_on_collision() {
        let dir = env::temp_dir().join(unique_stage_name("test"));
        fs::create_dir(&dir).expect("test temp dir");
        let bytes = b"first";
        let path = write_exclusive_file(&dir, "payload.bin", bytes).expect("first write");
        assert_eq!(fs::read(&path).expect("read back"), bytes);
        let err =
            write_exclusive_file(&dir, "payload.bin", b"second").expect_err("collision must fail");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // The colliding write must not have replaced the bytes.
        assert_eq!(fs::read(&path).expect("read back"), bytes);
        cleanup_owned_stage(&dir, &path);
        assert!(!path.exists());
        assert!(!dir.exists());
    }

    #[test]
    fn real_staging_roundtrip_keeps_exact_bytes() {
        let payload = b"ENTROPIA-TEST-PAYLOAD-ROUNDTRIP";
        let dir = create_private_stage_dir().expect("private stage dir");
        let exe = write_exclusive_file(&dir, "MicrosoftEdgeWebview2Setup.exe", payload)
            .expect("exclusive stage write");
        let mut locked = open_read_locked(&exe).expect("read-locked open");
        let staged = read_bounded(&mut locked, payload.len()).expect("bounded read-back");
        assert!(payload_matches(payload, &staged));
        drop(locked);
        cleanup_owned_stage(&dir, &exe);
        assert!(!exe.exists(), "owned file must be removed");
        assert!(!dir.exists(), "owned dir must be removed");
    }

    #[test]
    fn bounded_read_detects_trailing_bytes() {
        let dir = create_private_stage_dir().expect("private stage dir");
        let payload = b"abc";
        let path = write_exclusive_file(&dir, "payload.bin", b"abcd").expect("write longer");
        let mut locked = open_read_locked(&path).expect("read-locked open");
        let staged = read_bounded(&mut locked, payload.len()).expect("bounded read");
        assert!(
            !payload_matches(payload, &staged),
            "trailing byte must fail the compare"
        );
        drop(locked);
        cleanup_owned_stage(&dir, &path);
    }

    #[test]
    fn cleanup_removes_only_owned_paths() {
        let dir = create_private_stage_dir().expect("private stage dir");
        let sibling = create_private_stage_dir().expect("sibling stage dir");
        let path = write_exclusive_file(&dir, "payload.bin", b"owned").expect("write");
        cleanup_owned_stage(&dir, &path);
        assert!(!dir.exists());
        assert!(sibling.exists(), "sibling stage must survive owned cleanup");
        cleanup_owned_stage(&sibling, &sibling.join("payload.bin"));
        assert!(!sibling.exists());
    }

    #[cfg(unix)]
    #[test]
    fn posix_stage_dir_is_exclusive_0700() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = create_private_stage_dir().expect("private stage dir");
        let mode = fs::metadata(&dir).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "POSIX fallback must be 0700");
        cleanup_owned_stage(&dir, &dir.join("payload.bin"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_read_lock_denies_write_and_delete() {
        let payload = b"ENTROPIA-LOCK-TEST";
        let dir = create_private_stage_dir().expect("private stage dir");
        let exe = write_exclusive_file(&dir, "MicrosoftEdgeWebview2Setup.exe", payload)
            .expect("stage write");
        let mut locked = open_read_locked(&exe).expect("read-locked open");
        // While the deny-write/delete lock is held, a new write open and a
        // delete must both fail on Windows.
        let write_err = OpenOptions::new()
            .write(true)
            .open(&exe)
            .expect_err("write open must be denied while locked");
        assert_ne!(write_err.kind(), io::ErrorKind::NotFound);
        let delete_err = fs::remove_file(&exe).expect_err("delete must be denied while locked");
        assert_ne!(delete_err.kind(), io::ErrorKind::NotFound);
        // Integrity still holds through the lock.
        let staged = read_bounded(&mut locked, payload.len()).expect("bounded read");
        assert!(payload_matches(payload, &staged));
        drop(locked);
        // After all locks are released, owned cleanup succeeds.
        cleanup_owned_stage(&dir, &exe);
        assert!(!exe.exists());
        assert!(!dir.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_dir_hold_prevents_rename_until_released() {
        let dir = create_private_stage_dir().expect("private stage dir");
        let guard = hold_stage_dir(&dir).expect("hold directory identity");
        let renamed = dir.with_extension("renamed");
        let rename_err =
            fs::rename(&dir, &renamed).expect_err("rename must fail while dir is held");
        assert_ne!(rename_err.kind(), io::ErrorKind::NotFound);
        drop(guard);
        // After release, owned cleanup still owns exactly the original dir.
        cleanup_owned_stage(&dir, &dir.join("payload.bin"));
        assert!(!dir.exists());
        assert!(!renamed.exists());
    }
}
