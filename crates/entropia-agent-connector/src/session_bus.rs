//! Reaching the session keyring from a scrubbed environment (Linux).
//!
//! The keyring talks to the Secret Service over the D-Bus session bus, which
//! it finds through `DBUS_SESSION_BUS_ADDRESS`. Some assistants launch MCP
//! servers with only a handful of variables (Codex keeps `HOME`, `PATH`, `USER`
//! and little else), so the connector would find no store and report the
//! client as unpaired. When the variable is missing, the connector points it
//! at the user's standard bus socket — `$XDG_RUNTIME_DIR/bus`, then
//! `/run/user/<uid>/bus` — but only a socket the same user owns, so it can never
//! talk to another user's session.

use std::path::{Path, PathBuf};

/// The bus address to set, or `None` to leave the environment alone.
///
/// `owner_of_socket` answers the owner uid of a path when it is a socket, so a
/// test can describe the filesystem without creating one.
pub fn default_address(
    existing: Option<&str>,
    runtime_dir: Option<&Path>,
    uid: u32,
    owner_of_socket: impl Fn(&Path) -> Option<u32>,
) -> Option<String> {
    if existing.is_some_and(|value| !value.is_empty()) {
        return None;
    }
    let candidates = runtime_dir
        .map(|dir| dir.join("bus"))
        .into_iter()
        .chain(std::iter::once(PathBuf::from(format!(
            "/run/user/{uid}/bus"
        ))));
    for candidate in candidates {
        if owner_of_socket(&candidate) == Some(uid) {
            return Some(format!("unix:path={}", candidate.display()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sockets(owned: &'static [(&'static str, u32)]) -> impl Fn(&Path) -> Option<u32> {
        move |path| {
            owned
                .iter()
                .find(|(p, _)| Path::new(p) == path)
                .map(|(_, uid)| *uid)
        }
    }

    #[test]
    fn an_existing_address_is_never_replaced() {
        let found = default_address(
            Some("unix:path=/custom/bus"),
            None,
            1000,
            sockets(&[("/run/user/1000/bus", 1000)]),
        );
        assert_eq!(found, None);
    }

    #[test]
    fn the_runtime_dir_bus_comes_first() {
        let found = default_address(
            None,
            Some(Path::new("/xdg")),
            1000,
            sockets(&[("/xdg/bus", 1000), ("/run/user/1000/bus", 1000)]),
        );
        assert_eq!(found.as_deref(), Some("unix:path=/xdg/bus"));
    }

    #[test]
    fn without_a_runtime_dir_the_standard_bus_is_used() {
        let found = default_address(
            Some(""),
            None,
            1000,
            sockets(&[("/run/user/1000/bus", 1000)]),
        );
        assert_eq!(found.as_deref(), Some("unix:path=/run/user/1000/bus"));
    }

    #[test]
    fn a_socket_another_user_owns_is_never_used() {
        let found = default_address(
            None,
            Some(Path::new("/tmp/shared")),
            1000,
            sockets(&[("/tmp/shared/bus", 0), ("/run/user/1000/bus", 1001)]),
        );
        assert_eq!(found, None);
    }

    #[test]
    fn no_socket_leaves_the_environment_alone() {
        assert_eq!(default_address(None, None, 1000, sockets(&[])), None);
    }
}
