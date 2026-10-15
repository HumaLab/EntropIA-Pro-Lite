//! WebView2 evergreen-bootstrapper repair wrapper for the Windows MSI.
//!
//! The MSI forces `ForceInvokeWebView2Bootstrapper` on every install/update
//! (see `windows/fragments/force-webview2-bootstrapper.wxs`): Windows Sandbox
//! can expose the machine WebView2 registry key while the runtime is still
//! unavailable to the real user, so a registry-presence check alone would skip
//! a repair that is still needed. Re-running the evergreen bootstrapper is
//! idempotent, except that an already-installed runtime exits with
//! `0x80040828` (`-2147219416` signed), which WiX `Return="check"` treats as a
//! fatal error and rolls the whole install back (MSI error 1722).
//!
//! This tiny dependency-free wrapper embeds the original signed Microsoft
//! bootstrapper (`include_bytes!` of the file the pre-bundle hook downloads
//! and Authenticode-verifies), stages it via the cohesive `stage` module
//! (`windows/webview2_stage.rs`), runs it with the MSI arguments, waits, and
//! normalizes ONLY the documented already-installed result to success.
//! Exit `0` stays `0`; every other child code propagates unchanged.
//! Extraction, spawn, wait, and missing-code failures exit nonzero (fail
//! closed) so a broken repair can never masquerade as success.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
use std::env;
use std::path::Path;

#[path = "webview2_stage.rs"]
mod stage;

/// Documented WebView2 evergreen-bootstrapper result for "already installed".
/// External Microsoft EdgeUpdate logs identify `0x80040828` as success-like;
/// as a signed 32-bit process exit code it surfaces as `-2147219416`.
pub const ALREADY_INSTALLED_UNSIGNED: u32 = 0x8004_0828;
pub const ALREADY_INSTALLED: i32 = ALREADY_INSTALLED_UNSIGNED as i32;

/// Wrapper-operational failures (extraction, ACL, spawn, wait, missing child
/// code) exit here. `1603` is `ERROR_INSTALL_FAILURE`, the idiomatic fatal MSI
/// code. A child bootstrapper `1603` is NOT rewritten by this constant: it
/// passes through [`normalize_bootstrapper_exit_code`] unchanged (pinned below).
pub const OPERATIONAL_EXIT_CODE: i32 = 1603;

/// The verified Microsoft bootstrapper, embedded at bundle time.
///
/// Production builds require `WEBVIEW2_BOOTSTRAPPER_PATH` (set only for the
/// `rustc` invocation by `scripts/prepare-webview2-repair.ps1`): compiling
/// without the payload must fail closed, never silently ship an empty wrapper.
/// Unit-test builds fall back to a small synthetic payload so `rustc --test`
/// exercises the orchestration without network or WebView2 involvement.
#[cfg(not(test))]
const EMBEDDED_BOOTSTRAPPER: &[u8] = include_bytes!(env!("WEBVIEW2_BOOTSTRAPPER_PATH"));

#[cfg(test)]
const EMBEDDED_BOOTSTRAPPER: &[u8] = b"ENTROPIA-TEST-PAYLOAD";

/// Map a bootstrapper child exit code to the wrapper exit code.
///
/// Only the documented already-installed result becomes success. Everything
/// else — including `0` and every other nonzero code — passes through
/// unchanged so genuine failures still fail the MSI action.
pub fn normalize_bootstrapper_exit_code(code: i32) -> i32 {
    if code == ALREADY_INSTALLED {
        0
    } else {
        code
    }
}

/// Resolve a waited child status to the wrapper exit code.
///
/// `None` (no exit code captured) is an operational failure: fail closed
/// instead of guessing success.
pub fn normalize_child_exit(code: Option<i32>) -> Result<i32, i32> {
    match code {
        Some(child) => Ok(normalize_bootstrapper_exit_code(child)),
        None => Err(OPERATIONAL_EXIT_CODE),
    }
}

/// Run the extracted bootstrapper with the MSI arguments and wait for it.
/// No shell, no user-supplied executables: exactly this file, exactly these args.
fn spawn_bootstrapper(exe: &Path, args: &[String]) -> std::io::Result<Option<i32>> {
    std::process::Command::new(exe)
        .args(args)
        .status()
        .map(|status| status.code())
}

/// Full wrapper run over an explicit payload (production passes the embedded
/// Microsoft bootstrapper; tests pass synthetic bytes). Returns the process
/// exit code for `main`. Staging primitives come from `stage`; exit policy
/// stays here so the already-installed mapping has exactly one owner.
pub fn run(payload: &[u8], args: &[String]) -> i32 {
    let stage_dir = match stage::create_private_stage_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("webview2-repair: cannot stage private directory: {err}");
            return OPERATIONAL_EXIT_CODE;
        }
    };
    run_in_owned_stage(payload, args, &stage_dir)
}

/// Internal orchestration over an already-created private owned stage dir.
/// Not CLI-controlled: production reaches this only via `run()` with a
/// `create_private_stage_dir()` directory; tests pass their own owned dirs
/// so the path is known for cleanup assertions. No workspace/temp CLI args,
/// no arbitrary recursive removal — only the two owned paths are cleaned.
/// Guard ordering (Windows ShareDelete): hold directory + payload locks
/// THROUGH write/read-lock/byte-compare/spawn/wait, then release payload
/// lock and DIRECTORY guard BEFORE the owned cleanup call.
fn run_in_owned_stage(payload: &[u8], args: &[String], stage_dir: &Path) -> i32 {
    // Only these two owned paths are ever removed below.
    let exe = match stage::write_exclusive_file(
        stage_dir,
        "MicrosoftEdgeWebview2Setup.exe",
        payload,
    ) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("webview2-repair: cannot stage bootstrapper: {err}");
            stage::cleanup_owned_stage(stage_dir, &stage_dir.join("MicrosoftEdgeWebview2Setup.exe"));
            return OPERATIONAL_EXIT_CODE;
        }
    };
    // Hold the directory identity (no-delete share on Windows) through
    // write/read-lock/byte-compare/spawn/wait. Released BEFORE cleanup
    // below: Windows denies ShareDelete while the guard lives, so cleaning
    // up while held leaks the owned directory.
    let dir_hold = match stage::hold_stage_dir(stage_dir) {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("webview2-repair: cannot hold stage directory: {err}");
            stage::cleanup_owned_stage(stage_dir, &exe);
            return OPERATIONAL_EXIT_CODE;
        }
    };
    let exit = (|| {
        let mut locked = stage::open_read_locked(&exe)
            .map_err(|err| format!("cannot lock staged bootstrapper for reading: {err}"))?;
        let staged = stage::read_bounded(&mut locked, payload.len())
            .map_err(|err| format!("cannot read back staged bootstrapper: {err}"))?;
        if !stage::payload_matches(payload, &staged) {
            return Err("staged bootstrapper differs from the embedded payload".to_string());
        }
        // `locked` stays alive (deny write/delete share on Windows) through wait.
        let code = spawn_bootstrapper(&exe, args)
            .map_err(|err| format!("cannot launch staged bootstrapper: {err}"))?;
        // Release the payload lock first, while the directory guard is still
        // held through the wait above.
        drop(locked);
        normalize_child_exit(code).map_err(|operational| {
            format!("bootstrapper produced no exit code (operational failure {operational})")
        })
    })();
    // Release the DIRECTORY guard BEFORE the owned cleanup: on Windows the
    // guard denies delete sharing, so `remove_dir` fails while it lives.
    drop(dir_hold);
    // Best-effort cleanup of exactly the owned file and directory.
    stage::cleanup_owned_stage(stage_dir, &exe);
    match exit {
        Ok(code) => code,
        Err(message) => {
            eprintln!("webview2-repair: {message}");
            OPERATIONAL_EXIT_CODE
        }
    }
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    std::process::exit(run(EMBEDDED_BOOTSTRAPPER, &args));
}

/// Never shipped: the production binary is compiled on Windows by the
/// pre-bundle hook. Fail closed anywhere else.
#[cfg(not(windows))]
fn main() {
    eprintln!("webview2-repair: Windows-only MSI helper; refusing to run elsewhere");
    std::process::exit(OPERATIONAL_EXIT_CODE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_installed_maps_to_success() {
        assert_eq!(normalize_bootstrapper_exit_code(-2147219416), 0);
    }

    #[test]
    fn unsigned_already_installed_constant_matches_signed_code() {
        assert_eq!(ALREADY_INSTALLED_UNSIGNED, 0x8004_0828);
        assert_eq!(ALREADY_INSTALLED, -2147219416);
        assert_eq!(ALREADY_INSTALLED, ALREADY_INSTALLED_UNSIGNED as i32);
    }

    #[test]
    fn success_stays_success() {
        assert_eq!(normalize_bootstrapper_exit_code(0), 0);
    }

    #[test]
    fn adjacent_and_unrelated_codes_propagate_unchanged() {
        // Adjacent to the normalized code: never normalize a neighbour.
        for code in [
            -2147219417,
            -2147219415,
            // Other HRESULTs and MSI failures.
            -2147024891, // E_ACCESSDENIED / 0x80070005
            -2147024894, // file not found / 0x80070002
            1603,        // ERROR_INSTALL_FAILURE
            1602,        // user cancel
            3010,        // success, reboot required
            1,
            -1,
            i32::MIN,
            i32::MAX,
        ] {
            assert_eq!(
                normalize_bootstrapper_exit_code(code),
                code,
                "code {code} must propagate unchanged"
            );
        }
    }

    #[test]
    fn missing_child_code_is_an_operational_failure() {
        assert_eq!(normalize_child_exit(None), Err(OPERATIONAL_EXIT_CODE));
        assert_eq!(normalize_child_exit(Some(0)), Ok(0));
        assert_eq!(normalize_child_exit(Some(-2147219416)), Ok(0));
        assert_eq!(normalize_child_exit(Some(1603)), Ok(1603));
    }

    #[test]
    fn launch_failure_is_an_io_error_not_success() {
        let missing = std::env::temp_dir().join(stage::unique_stage_name("missing"));
        let missing_exe = missing.join("MicrosoftEdgeWebview2Setup.exe");
        let err = spawn_bootstrapper(&missing_exe, &["/silent".to_string(), "/install".to_string()])
            .expect_err("spawning a missing executable must fail");
        assert_ne!(err.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn embedded_test_payload_is_nonempty() {
        assert!(!EMBEDDED_BOOTSTRAPPER.is_empty());
    }

    #[test]
    fn run_invalid_payload_fails_closed_and_cleans_owned_stage() {
        // Native cleanup regression for the MSI-1b guard ordering: the old
        // code held the DIRECTORY guard through `cleanup_owned_stage`, so on
        // Windows ShareDelete denied `remove_dir` and the owned dir leaked.
        // Test-first note: Linux cannot show RED here — POSIX removes the
        // dir even with the guard open — so this passes locally before and
        // after; Windows CI is where the old order fails (dir remains) and
        // the fixed order passes (dir gone). Source ordering
        // (`drop(dir_hold)` before cleanup) is the Windows proof.
        // Malformed owned bytes are not executable on Linux (spawn EACCES):
        // acceptable fatal 1603; production is Windows-Runtime-only, no Linux
        // permission semantic change for tests.
        let stage_dir = stage::create_private_stage_dir().expect("owned stage dir");
        let sibling = stage::create_private_stage_dir().expect("sibling stage dir");
        let code = run_in_owned_stage(b"NOT-AN-EXECUTABLE", &[], &stage_dir);
        assert_eq!(code, OPERATIONAL_EXIT_CODE);
        assert!(!stage_dir.join("MicrosoftEdgeWebview2Setup.exe").exists());
        assert!(!stage_dir.exists(), "owned stage dir must be removed");
        assert!(sibling.exists(), "unrelated sibling must survive owned cleanup");
        stage::cleanup_owned_stage(&sibling, &sibling.join("payload.bin"));
        assert!(!sibling.exists());
    }

    #[cfg(windows)]
    #[test]
    fn run_selftest_executable_lists_and_cleans_stage() {
        // Windows-only direct-run success: stage this test binary itself and
        // run it with `--list` (synthetic self-test, no network, no MS
        // bootstrapper). Proves guard-through-spawn/wait then DIRECTORY
        // release before cleanup, with exit 0 and dir gone.
        let self_exe = std::env::current_exe().expect("current test executable");
        let bytes = std::fs::read(&self_exe).expect("read self executable");
        assert!(!bytes.is_empty());
        let stage_dir = stage::create_private_stage_dir().expect("owned stage dir");
        let code = run_in_owned_stage(&bytes, &["--list".to_string()], &stage_dir);
        assert_eq!(code, 0);
        assert!(!stage_dir.exists(), "owned stage dir must be removed after success");
    }
}
