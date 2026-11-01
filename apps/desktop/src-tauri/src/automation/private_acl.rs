//! Owner-only access for the assistant's files on Windows: the counterpart of
//! the `0600` / `0700` modes `access.rs` sets on Unix.
//!
//! A new file inherits the access list of its folder. On a stock profile
//! `%APPDATA%` grants only the user, SYSTEM and Administrators, but other
//! software adds entries there (Codex's Windows sandbox, for one, grants its
//! sandbox users modify rights on the whole roaming folder), and every
//! token, token hash and instance key would inherit them. So each of these
//! files gets a protected access list (no inheritance) naming only the user
//! running the app and SYSTEM, set on the open, still empty file, before any
//! secret is written to it. A rename keeps it.

use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    SetNamedSecurityInfoW, SetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The protected access list of a file: the user and SYSTEM, full control.
fn file_sddl(user: &str) -> String {
    format!("D:P(A;;FA;;;{user})(A;;FA;;;SY)")
}

/// The same for a folder, inherited by what is created inside it.
fn dir_sddl(user: &str) -> String {
    format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)")
}

/// Restrict an open file, as soon as it exists and before it holds anything.
pub fn restrict_file(file: &std::fs::File) -> Result<(), String> {
    let sddl = file_sddl(&current_user_sid()?);
    with_dacl(&sddl, |dacl| {
        let handle = HANDLE(file.as_raw_handle());
        // SAFETY: `handle` is a live file handle borrowed from `file`, and
        // `dacl` points into a descriptor `with_dacl` keeps alive meanwhile.
        unsafe {
            SetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(dacl),
                None,
            )
        }
    })
}

/// Restrict a folder by path (it is created first by the caller).
pub fn restrict_dir(path: &Path) -> Result<(), String> {
    let sddl = dir_sddl(&current_user_sid()?);
    let name = HSTRING::from(path.as_os_str());
    with_dacl(&sddl, |dacl| {
        // SAFETY: `name` outlives the call; `dacl` is kept alive by `with_dacl`.
        unsafe {
            SetNamedSecurityInfoW(
                &name,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(dacl),
                None,
            )
        }
    })
}

/// Build the access list an SDDL string describes and hand it to `apply`.
fn with_dacl(
    sddl: &str,
    apply: impl FnOnce(*const ACL) -> windows::Win32::Foundation::WIN32_ERROR,
) -> Result<(), String> {
    let mut descriptor = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
    // SAFETY: plain out-parameter call; the descriptor is freed below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| format!("could not build the access list {sddl}: {e}"))?;
    let result = apply_descriptor(descriptor, sddl, apply);
    // SAFETY: the descriptor was allocated by the conversion with LocalAlloc.
    let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    result
}

fn apply_descriptor(
    descriptor: PSECURITY_DESCRIPTOR,
    sddl: &str,
    apply: impl FnOnce(*const ACL) -> windows::Win32::Foundation::WIN32_ERROR,
) -> Result<(), String> {
    let mut present = windows::core::BOOL::default();
    let mut defaulted = windows::core::BOOL::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: `descriptor` is a valid descriptor owned by the caller.
    unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        .map_err(|e| format!("could not read the access list {sddl}: {e}"))?;
    let status = apply(dacl);
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "could not apply the owner-only access list (Win32 error {})",
            status.0
        ))
    }
}

/// The SID of the user this process runs as, in its `S-1-5-…` form.
pub fn current_user_sid() -> Result<String, String> {
    let mut token = HANDLE(std::ptr::null_mut());
    // SAFETY: the pseudo handle of the current process needs no closing; the
    // token is closed below.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| format!("could not open the process token: {e}"))?;
    let result = token_user_sid(token);
    // SAFETY: closing the token opened above.
    let _ = unsafe { CloseHandle(token) };
    result
}

fn token_user_sid(token: HANDLE) -> Result<String, String> {
    {
        let mut needed = 0u32;
        // SAFETY: the first call only asks for the buffer size.
        let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
        if needed == 0 {
            return Err("the process token reported no user".to_string());
        }
        // u64 storage keeps TOKEN_USER's pointer field aligned.
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buffer` holds at least `needed` bytes.
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                needed,
                &mut needed,
            )
        }
        .map_err(|e| format!("could not read the process user: {e}"))?;
        // SAFETY: GetTokenInformation(TokenUser) filled a TOKEN_USER at the start.
        let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
        let mut text = PWSTR::null();
        // SAFETY: the SID points into `buffer`, alive for this call.
        unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }
            .map_err(|e| format!("could not format the process user: {e}"))?;
        // SAFETY: the conversion returned a NUL-terminated LocalAlloc string.
        let sid = unsafe { text.to_string() }
            .map_err(|e| format!("the process user is not valid UTF-16: {e}"));
        // SAFETY: freeing the string the conversion allocated.
        let _ = unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
        sid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
    };

    /// The access list of a path, as SDDL (DACL part only).
    fn dacl_sddl(path: &Path) -> String {
        let name = HSTRING::from(path.as_os_str());
        let mut descriptor = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
        // SAFETY: out-parameter call; the descriptor is freed below.
        let status = unsafe {
            GetNamedSecurityInfoW(
                &name,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                &mut descriptor,
            )
        };
        assert_eq!(status, ERROR_SUCCESS, "read {}", path.display());
        let mut text = PWSTR::null();
        // SAFETY: `descriptor` is the one just read.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                None,
            )
        }
        .unwrap();
        // SAFETY: NUL-terminated string allocated by the conversion.
        let sddl = unsafe { text.to_string() }.unwrap();
        // SAFETY: freeing what the two calls above allocated.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        sddl
    }

    #[test]
    fn a_restricted_file_names_only_the_user_and_system_and_inherits_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.txt");
        let file = std::fs::File::create(&path).unwrap();
        restrict_file(&file).unwrap();
        drop(file);
        let user = current_user_sid().unwrap();
        assert!(user.starts_with("S-1-"), "{user}");
        assert_eq!(dacl_sddl(&path), file_sddl(&user));
    }

    /// What the Codex sandbox does to `%APPDATA%`: an extra inheritable entry
    /// on the parent. The atomic write still produces an owner-only file.
    #[test]
    fn an_atomic_write_ignores_extra_entries_on_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let user = current_user_sid().unwrap();
        let widened = format!("D:(A;OICI;FA;;;WD)(A;OICI;FA;;;{user})");
        let name = HSTRING::from(dir.path().as_os_str());
        // SAFETY: `name` outlives the call; `dacl` is kept alive by `with_dacl`.
        with_dacl(&widened, |dacl| unsafe {
            SetNamedSecurityInfoW(
                &name,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(dacl),
                None,
            )
        })
        .unwrap();
        let probe = dir.path().join("probe.txt");
        std::fs::write(&probe, b"x").unwrap();
        assert!(
            dacl_sddl(&probe).contains(";;;WD)"),
            "the folder now leaks: {}",
            dacl_sddl(&probe)
        );

        let target = dir.path().join("agent-endpoint.json");
        crate::automation::access::write_atomic(&target, b"{}").unwrap();
        assert_eq!(dacl_sddl(&target), file_sddl(&user));
    }

    #[test]
    fn a_restricted_folder_passes_owner_only_access_to_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("agent-tokens");
        std::fs::create_dir(&folder).unwrap();
        restrict_dir(&folder).unwrap();
        let user = current_user_sid().unwrap();
        assert_eq!(dacl_sddl(&folder), dir_sddl(&user));
        let child = folder.join("plain.txt");
        std::fs::write(&child, b"x").unwrap();
        assert_eq!(
            dacl_sddl(&child),
            format!("D:AI(A;ID;FA;;;{user})(A;ID;FA;;;SY)"),
            "a file created inside inherits only the two entries"
        );
    }
}
