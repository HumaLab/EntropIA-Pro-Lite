//! Owner-only access for the assistant's files on Windows: the counterpart of
//! the `0600` / `0700` modes `access.rs` sets on Unix.
//!
//! A new file inherits the access list of its folder. On a stock profile
//! `%APPDATA%` grants only the user, SYSTEM and Administrators, but other
//! software adds entries there (Codex's Windows sandbox, for one, grants its
//! sandbox users modify rights on the whole roaming folder), and every
//! token, token hash and instance key would inherit them. So each of these
//! files is created with a protected access list (no inheritance) naming only
//! the user running the app and SYSTEM: given to `CreateFileW` itself, so the
//! file never exists under the inherited list, and opened exclusively, so no
//! other process holds a handle to it before the secret is written. A rename
//! keeps the list.

use std::os::windows::io::FromRawHandle;
use std::path::Path;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE,
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

/// Create `path`, which must not exist yet, readable by the user and SYSTEM
/// only from its first instant, and opened exclusively for this process.
pub fn create_private_file(path: &Path) -> Result<std::fs::File, String> {
    let sddl = file_sddl(&current_user_sid()?);
    let mut descriptor = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
    // SAFETY: plain out-parameter call; the descriptor is freed below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl.as_str()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| format!("could not build the access list {sddl}: {e}"))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name = HSTRING::from(path.as_os_str());
    // SAFETY: `name` and `attributes` (and the descriptor it points to) outlive
    // the call.
    let created = unsafe {
        CreateFileW(
            &name,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    // SAFETY: the descriptor was allocated by the conversion with LocalAlloc.
    let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    let handle = created.map_err(|e| format!("could not create {}: {e}", path.display()))?;
    // SAFETY: `handle` is a fresh, owned file handle nobody else holds.
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

/// Write `bytes` to `path` through a private sibling named `<prefix><random>`
/// and a rename, so a crash never leaves half a file and the target is
/// owner-only. The sibling is removed if any step fails.
pub fn write_atomic(path: &Path, bytes: &[u8], prefix: &str) -> Result<(), String> {
    use std::io::Write as _;
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let staged = dir.join(format!("{prefix}{}", uuid::Uuid::new_v4().simple()));
    let written = create_private_file(&staged).and_then(|mut file| {
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("Could not write {}: {e}", path.display()))
    });
    let renamed = written.and_then(|()| {
        std::fs::rename(&staged, path)
            .map_err(|e| format!("Could not replace {}: {e}", path.display()))
    });
    if renamed.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    renamed
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
        let sddl = descriptor_sddl(descriptor);
        // SAFETY: freeing the descriptor the call above allocated.
        let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        sddl
    }

    /// A descriptor's DACL as SDDL, the way Windows writes it.
    fn descriptor_sddl(descriptor: PSECURITY_DESCRIPTOR) -> String {
        let mut text = PWSTR::null();
        // SAFETY: `descriptor` is a valid descriptor owned by the caller.
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
        // SAFETY: freeing the string the conversion allocated.
        let _ = unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
        sddl
    }

    /// `entries` as Windows itself writes them back. SDDL has aliases for
    /// well-known accounts (the built-in Administrator, RID 500, prints as
    /// `LA`), so an expected list spelled with raw SIDs is compared after the
    /// same round trip as the observed one.
    fn canonical_entries(entries: &str) -> String {
        let mut descriptor = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
        // SAFETY: out-parameter call; the descriptor is freed below.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(format!("D:{entries}")),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .unwrap();
        let sddl = descriptor_sddl(descriptor);
        // SAFETY: freeing the descriptor the conversion allocated.
        let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        split(&sddl).1.to_string()
    }

    /// Split a DACL's SDDL into its control flags and its entries.
    fn split(sddl: &str) -> (&str, &str) {
        let body = sddl.strip_prefix("D:").expect("a DACL");
        let at = body.find('(').unwrap_or(body.len());
        body.split_at(at)
    }

    /// Protected (no inheritance from the folder) and exactly these entries.
    fn assert_owner_only(path: &Path, entries: &str) {
        let sddl = dacl_sddl(path);
        let (flags, aces) = split(&sddl);
        assert!(flags.contains('P'), "not protected: {sddl}");
        assert_eq!(aces, canonical_entries(entries), "{}", path.display());
    }

    /// The comparison must not depend on who runs the tests: a raw SID and its
    /// SDDL alias name the same account. CI runs as the built-in Administrator,
    /// whose SID Windows writes back as `LA`.
    #[test]
    fn expected_entries_are_compared_in_the_spelling_windows_uses() {
        assert_eq!(canonical_entries("(A;;FA;;;S-1-5-18)"), "(A;;FA;;;SY)");
        let user = current_user_sid().unwrap();
        let spelled = canonical_entries(&format!("(A;;FA;;;{user})"));
        assert!(
            spelled.starts_with("(A;;FA;;;") && spelled.ends_with(')'),
            "{spelled}"
        );
    }

    #[test]
    fn a_private_file_names_only_the_user_and_system_and_inherits_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.txt");
        drop(create_private_file(&path).unwrap());
        let user = current_user_sid().unwrap();
        assert!(user.starts_with("S-1-"), "{user}");
        assert_owner_only(&path, &format!("(A;;FA;;;{user})(A;;FA;;;SY)"));
    }

    #[test]
    fn a_private_file_is_never_created_over_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("taken.txt");
        std::fs::write(&path, b"x").unwrap();
        assert!(create_private_file(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
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
        assert_owner_only(&target, &format!("(A;;FA;;;{user})(A;;FA;;;SY)"));
        assert_eq!(std::fs::read(&target).unwrap(), b"{}");
        // Replacing it keeps the private list and leaves no staged sibling.
        crate::automation::access::write_atomic(&target, b"{\"v\":2}").unwrap();
        assert_owner_only(&target, &format!("(A;;FA;;;{user})(A;;FA;;;SY)"));
        let staged = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(crate::automation::access::TEMP_PREFIX)
            })
            .count();
        assert_eq!(staged, 0);
    }

    #[test]
    fn a_restricted_folder_passes_owner_only_access_to_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("agent-tokens");
        std::fs::create_dir(&folder).unwrap();
        restrict_dir(&folder).unwrap();
        let user = current_user_sid().unwrap();
        assert_owner_only(&folder, &format!("(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)"));
        let child = folder.join("plain.txt");
        std::fs::write(&child, b"x").unwrap();
        let sddl = dacl_sddl(&child);
        assert_eq!(
            split(&sddl).1,
            canonical_entries(&format!("(A;ID;FA;;;{user})(A;ID;FA;;;SY)")),
            "a file created inside inherits only the two entries"
        );
    }
}
