//! An isolated profile for development runs.
//!
//! `tauri dev` opens the same archive as the installed app
//! (`<data>/com.entropia.shared`), and a sync session on this machine lives in
//! the OS keyring, shared by every data directory. A dev build that applies a
//! new migration to that archive and syncs once would raise the account's
//! `schema_tag` for every device (the server keeps the lexicographic maximum),
//! and nothing local can undo it.
//!
//! Setting `ENTROPIA_DEV_PROFILE=<name>` in a **debug** build moves everything
//! into `<data>/com.entropia.shared/dev-profiles/<name>` (and the same under the
//! local-data cache directory), and turns sync off: no engine, no keyring
//! access, and the sync commands answer [`SYNC_DISABLED`]. A release build
//! never reads the variable: the read is compiled out.
//!
//! Why a name and not a path: the asset-protocol and fs scopes the shipped
//! configs declare only cover `$DATA/com.entropia.shared/**` and
//! `$LOCALDATA/com.entropia.shared/**`. A directory nested under those is
//! already inside both scopes, so no configuration widens; an arbitrary path
//! could not be scoped without widening them. The name is restricted to
//! `[A-Za-z0-9_-]` so it can never climb out of that directory.
//!
//! A variable that is set but invalid is an error, never a silent fall back:
//! the fallback would be the real archive.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The variable that selects the profile (debug builds only).
pub const ENV_VAR: &str = "ENTROPIA_DEV_PROFILE";

/// What every sync entry point answers while the profile is active. The UI maps
/// the code; the rest is for people.
pub const SYNC_DISABLED: &str =
    "sync_disabled_in_dev_profile: la sincronización está desactivada en el perfil de desarrollo";

/// Directory, under each shared directory, that holds the profiles.
pub const PROFILES_DIR: &str = "dev-profiles";

const NAME_MAX_CHARS: usize = 32;

/// Check a profile name. `None` is "no profile"; anything else must be a plain
/// name.
pub fn parse_name(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(name) = raw else {
        return Ok(None);
    };
    let plain = !name.is_empty()
        && name.chars().count() <= NAME_MAX_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if plain {
        Ok(Some(name.to_string()))
    } else {
        Err(format!(
            "{ENV_VAR} must be 1 to {NAME_MAX_CHARS} characters from A-Z, a-z, 0-9, '-' and '_' \
             (got {name:?}); refusing to fall back to the real archive"
        ))
    }
}

/// The profile to use, given whether this build may use one at all. A release
/// build (`enabled == false`) ignores `raw` whatever it holds.
pub fn requested(enabled: bool, raw: Option<&str>) -> Result<Option<String>, String> {
    if !enabled {
        return Ok(None);
    }
    parse_name(raw)
}

/// The data and cache directories of profile `name`, nested under the shared
/// ones.
pub fn profile_dirs(name: &str, shared_data: &Path, shared_cache: &Path) -> (PathBuf, PathBuf) {
    (
        shared_data.join(PROFILES_DIR).join(name),
        shared_cache.join(PROFILES_DIR).join(name),
    )
}

/// The one line the app logs at startup saying which archive it opened.
pub fn startup_line(profile: Option<&str>, data: &Path, cache: &Path) -> String {
    let (profile, sync) = match profile {
        Some(name) => (format!("dev:{name}"), "disabled"),
        None => ("shared".to_string(), "enabled"),
    };
    format!(
        "profile={profile} data_dir={} cache_dir={} sync={sync}",
        data.display(),
        cache.display()
    )
}

#[cfg(debug_assertions)]
fn env_value() -> Option<String> {
    std::env::var(ENV_VAR).ok()
}

#[cfg(not(debug_assertions))]
fn env_value() -> Option<String> {
    None
}

/// The profile this process was asked to run, from the environment. Always
/// `Ok(None)` in a release build.
pub fn from_env() -> Result<Option<String>, String> {
    requested(cfg!(debug_assertions), env_value().as_deref())
}

static ACTIVE: OnceLock<String> = OnceLock::new();

/// Record the profile this process runs. Called once, during setup, before
/// anything reads a path or starts sync.
pub fn activate(name: String) {
    let _ = ACTIVE.set(name);
}

/// The active profile's name, if any.
pub fn active() -> Option<&'static str> {
    if cfg!(debug_assertions) {
        ACTIVE.get().map(String::as_str)
    } else {
        None
    }
}

/// Whether sync must stay off. Always false in a release build.
pub fn sync_disabled() -> bool {
    active().is_some()
}

/// The guard's decision, separate from the process-wide state so it is testable.
pub fn check_sync(disabled: bool) -> Result<(), String> {
    if disabled {
        Err(SYNC_DISABLED.to_string())
    } else {
        Ok(())
    }
}

/// Call first in anything that reaches the sync account or its session: the
/// keyring entry, a server command, the engine. Refuses in the dev profile.
pub fn require_sync() -> Result<(), String> {
    check_sync(sync_disabled())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_variable_means_no_profile() {
        assert_eq!(parse_name(None), Ok(None));
    }

    #[test]
    fn a_plain_name_is_accepted() {
        for name in ["navegador", "p2-check", "A_b-9", "x"] {
            assert_eq!(parse_name(Some(name)), Ok(Some(name.to_string())), "{name}");
        }
    }

    #[test]
    fn anything_that_could_leave_the_profiles_directory_is_refused() {
        for name in [
            "",
            " ",
            "..",
            ".",
            "a/b",
            "a\\b",
            "../x",
            "C:\\temp",
            "/abs",
            "with space",
            "dot.ted",
            "tilde~",
            "ñandú",
            "nul\0",
            &"a".repeat(NAME_MAX_CHARS + 1),
        ] {
            assert!(parse_name(Some(name)).is_err(), "{name:?} must be refused");
        }
        assert!(parse_name(Some(&"a".repeat(NAME_MAX_CHARS))).is_ok());
    }

    #[test]
    fn a_build_that_cannot_use_a_profile_ignores_the_variable_completely() {
        // Not even an invalid value is an error: a release build never looks.
        assert_eq!(requested(false, Some("navegador")), Ok(None));
        assert_eq!(requested(false, Some("../../x")), Ok(None));
        assert_eq!(requested(false, Some("")), Ok(None));
    }

    #[test]
    fn a_build_that_can_use_one_reads_it_and_refuses_a_bad_value() {
        assert_eq!(
            requested(true, Some("navegador")),
            Ok(Some("navegador".into()))
        );
        assert_eq!(requested(true, None), Ok(None));
        assert!(requested(true, Some("../x")).is_err());
    }

    #[test]
    fn the_profile_lives_inside_the_directories_the_scopes_already_cover() {
        let data = Path::new("/home/u/.local/share/com.entropia.shared");
        let cache = Path::new("/home/u/.cache/com.entropia.shared");

        let (profile_data, profile_cache) = profile_dirs("navegador", data, cache);

        assert_eq!(profile_data, data.join("dev-profiles").join("navegador"));
        assert_eq!(profile_cache, cache.join("dev-profiles").join("navegador"));
        assert!(profile_data.starts_with(data));
        assert!(profile_cache.starts_with(cache));
        assert_ne!(profile_data, data, "never the real archive");
        assert_ne!(profile_cache, cache);
    }

    #[test]
    fn two_profiles_never_share_a_directory() {
        let data = Path::new("/d/com.entropia.shared");
        let cache = Path::new("/c/com.entropia.shared");
        assert_ne!(
            profile_dirs("a", data, cache),
            profile_dirs("b", data, cache)
        );
    }

    #[test]
    fn the_startup_line_names_the_profile_the_directories_and_sync() {
        let line = startup_line(
            Some("navegador"),
            Path::new("/d/com.entropia.shared/dev-profiles/navegador"),
            Path::new("/c/com.entropia.shared/dev-profiles/navegador"),
        );
        assert!(line.contains("profile=dev:navegador"), "{line}");
        assert!(line.contains("sync=disabled"), "{line}");
        assert!(line.contains("dev-profiles/navegador"), "{line}");
        assert!(!line.contains('\n'));

        let real = startup_line(
            None,
            Path::new("/d/com.entropia.shared"),
            Path::new("/c/com.entropia.shared"),
        );
        assert!(real.contains("profile=shared"), "{real}");
        assert!(real.contains("sync=enabled"), "{real}");
        assert!(!real.contains("dev:"), "{real}");
    }

    #[test]
    fn nothing_is_active_until_a_profile_is_activated() {
        // The process-wide slot is never set by these tests, so sync stays on
        // for every other test in the crate.
        assert_eq!(active(), None);
        assert!(!sync_disabled());
    }

    /// The environment is read in exactly one place, and only in a debug
    /// build. A release build must have no code that reads the variable.
    #[test]
    fn the_variable_is_read_only_by_a_debug_only_function() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let own = std::fs::read_to_string(src.join("dev_profile.rs")).unwrap();
        let runtime = own.split("#[cfg(test)]").next().unwrap();

        let reads: Vec<usize> = runtime
            .match_indices("std::env::var(ENV_VAR)")
            .map(|(at, _)| at)
            .collect();
        assert_eq!(reads.len(), 1, "one read of the variable");
        let before = &runtime[..reads[0]];
        let attribute = before
            .rfind("#[cfg(")
            .map(|at| before[at..].lines().next().unwrap_or_default())
            .unwrap_or_default();
        assert_eq!(attribute, "#[cfg(debug_assertions)]");

        // No other file reads it, by name or through the constant.
        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path.file_name().is_some_and(|n| n != "dev_profile.rs")
                {
                    let body = std::fs::read_to_string(&path).unwrap();
                    assert!(
                        !body.contains("ENTROPIA_DEV_PROFILE")
                            && !body.contains("dev_profile::ENV_VAR"),
                        "{} reads the profile variable itself",
                        path.display()
                    );
                }
            }
        }
    }

    #[test]
    fn sync_is_refused_with_the_stable_code_while_disabled_and_allowed_otherwise() {
        assert_eq!(check_sync(false), Ok(()));
        let refused = check_sync(true).unwrap_err();
        assert!(refused.starts_with("sync_disabled_in_dev_profile"));
        // Nothing is active in this process, so the real guard lets sync run.
        assert_eq!(require_sync(), Ok(()));
    }

    /// Every way the app can reach the account or the sync session must pass
    /// through the guard. The list is of the places that do: the keyring entry,
    /// the credentials every server command reads, the commands that talk to the
    /// server or change the session, and the engine's start.
    #[test]
    fn every_entry_to_the_account_is_guarded() {
        let sync = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("sync");
        let read = |name: &str| std::fs::read_to_string(sync.join(name)).unwrap();
        let (commands, session, engine) =
            (read("commands.rs"), read("session.rs"), read("engine.rs"));

        // The text of `fn <name>` up to the next item that starts a line.
        let body = |source: &str, signature: &str| -> String {
            let at = source
                .find(signature)
                .unwrap_or_else(|| panic!("{signature} not found"));
            let rest = &source[at..];
            let end = rest[signature.len()..]
                .find("\n}\n")
                .map(|n| n + signature.len())
                .unwrap_or(rest.len());
            rest[..end].to_string()
        };

        for (source, signature) in [
            (&session, "fn token_entry()"),
            (&commands, "async fn session_creds("),
            (&session, "pub async fn sync_register_account("),
            (&session, "pub async fn sync_login("),
            (&session, "pub async fn sync_logout("),
            (&commands, "pub async fn sync_now("),
            (&commands, "pub async fn sync_full_resync("),
            (&commands, "pub async fn sync_set_auto("),
        ] {
            assert!(
                body(source, signature).contains("require_sync()"),
                "{signature} does not call dev_profile::require_sync()"
            );
        }
        assert!(
            body(&engine, "pub fn start_engine(").contains("sync_disabled()"),
            "start_engine would spawn the engine in the dev profile"
        );
    }

    #[test]
    fn the_sync_disabled_message_carries_a_stable_code() {
        assert!(SYNC_DISABLED.starts_with("sync_disabled_in_dev_profile: "));
    }
}
