//! Where the assistant connector is installed on this machine (D15, T11).
//!
//! The installers bundle the connector next to the app executable, so the panel
//! can name an absolute path instead of relying on `PATH`. Under MSIX the
//! packaged copy lives inside the versioned WindowsApps directory, which Windows
//! refuses to launch from an ordinary command line, so the package declares an
//! execution alias (`entropia-agent-connector.exe`) and that alias is what the
//! recipes must use.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The connector's file stem: `bundle.externalBin` in `tauri.connector.conf.json`
/// (Lite) or `tauri.connector.pro.conf.json` (Pro), which tauri-build copies
/// beside the app binary. Pro has its own name because both .deb packages install
/// into /usr/bin and may be installed together (D16).
pub const CONNECTOR_STEM: &str = if cfg!(feature = "local-ml") {
    "entropia-pro-agent-connector"
} else {
    "entropia-agent-connector"
};

/// The execution alias the MSIX declares (D15), and the only name that resolves
/// to the packaged connector from outside the package.
pub const CONNECTOR_ALIAS: &str = "entropia-agent-connector.exe";

/// How the connector is reachable on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    /// A file next to the app executable: NSIS/MSI, .deb, macOS `.app`.
    Bundled,
    /// The MSIX execution alias; the packaged file itself is not launchable.
    MsixAlias,
    /// This build carried no connector (a dev run, or a build without the
    /// overlay).
    Missing,
}

/// `agent_connector_location`'s payload, as the panel reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorLocation {
    pub installed: bool,
    /// The absolute path, or the alias name under MSIX, or `None` when missing.
    pub path: Option<String>,
    pub kind: ConnectorKind,
}

/// The file name the connector has on this platform.
///
/// `EXE_SUFFIX` is the platform's own answer (`.exe` on Windows, empty
/// elsewhere), which is also what tauri-build appends when it copies the sidecar.
pub fn connector_file_name() -> String {
    format!("{CONNECTOR_STEM}{}", std::env::consts::EXE_SUFFIX)
}

/// Decide what the recipes may name.
///
/// The packaged copy is only reported through the alias: inside WindowsApps the
/// absolute path exists but Windows blocks launching it. A package that lacks the
/// payload file is `missing`, because the alias cannot resolve either — that way
/// the panel never offers a line that cannot work.
///
/// `exe_dir` and `package_identity` are parameters, not reads, so the decision is
/// testable: [`current_location`] supplies the real ones.
pub fn resolve_location(exe_dir: Option<&Path>, package_identity: bool) -> ConnectorLocation {
    let bundled = exe_dir
        .map(|dir| dir.join(connector_file_name()))
        .filter(|candidate| candidate.is_file());

    match bundled {
        Some(_) if package_identity => ConnectorLocation {
            installed: true,
            path: Some(CONNECTOR_ALIAS.to_string()),
            kind: ConnectorKind::MsixAlias,
        },
        Some(path) => ConnectorLocation {
            installed: true,
            path: Some(path.to_string_lossy().into_owned()),
            kind: ConnectorKind::Bundled,
        },
        None => ConnectorLocation {
            installed: false,
            path: None,
            kind: ConnectorKind::Missing,
        },
    }
}

/// The connector as this running process sees it.
pub fn current_location() -> ConnectorLocation {
    resolve_location(exe_directory().as_deref(), has_package_identity())
}

/// The directory holding the running executable, where the bundlers place every
/// `bundle.externalBin` sidecar.
fn exe_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// Whether this process runs with MSIX package identity.
///
/// Only a packaged process answers `Ok`; `APPMODEL_ERROR_NO_PACKAGE` (0x80073D54)
/// is the documented "unpackaged" answer and every other failure is treated as
/// unpackaged too, with a warning: the recipes keep working with a bare command
/// instead of claiming an alias that may not exist.
///
/// `store_updates.rs` reads the same identity for the Store update check, but
/// that reader is Lite-only and needs the package family name; the recipes only
/// need this one bit, so the check stays here, in both variants.
#[cfg(windows)]
fn has_package_identity() -> bool {
    use windows::core::HRESULT;
    use windows::ApplicationModel::Package;

    /// `HRESULT_FROM_WIN32(APPMODEL_ERROR_NO_PACKAGE)`.
    const APPMODEL_ERROR_NO_PACKAGE: HRESULT = HRESULT(0x8007_3D54_u32 as i32);

    match Package::Current() {
        Ok(_) => true,
        Err(error) if error.code() == APPMODEL_ERROR_NO_PACKAGE => false,
        Err(error) => {
            log::warn!(
                target: "automation",
                "package identity read failed (HRESULT {:#010X})",
                error.code().0 as u32
            );
            false
        }
    }
}

#[cfg(not(windows))]
fn has_package_identity() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_looks_for_its_own_connector_name() {
        // Lite and Pro .deb packages both install into /usr/bin; one shared name
        // would make dpkg refuse the second package (D16).
        let expected = if cfg!(feature = "local-ml") {
            "entropia-pro-agent-connector"
        } else {
            "entropia-agent-connector"
        };
        assert_eq!(CONNECTOR_STEM, expected);
        assert_eq!(
            connector_file_name(),
            format!("{expected}{}", std::env::consts::EXE_SUFFIX)
        );
    }

    #[test]
    fn a_connector_next_to_the_executable_is_reported_as_bundled() {
        let dir = tempfile::tempdir().unwrap();
        let connector = dir.path().join(connector_file_name());
        std::fs::write(&connector, b"binary").unwrap();

        assert_eq!(
            resolve_location(Some(dir.path()), false),
            ConnectorLocation {
                installed: true,
                path: Some(connector.to_string_lossy().into_owned()),
                kind: ConnectorKind::Bundled,
            }
        );
    }

    #[test]
    fn a_packaged_app_reports_the_execution_alias_not_the_package_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(connector_file_name()), b"binary").unwrap();

        assert_eq!(
            resolve_location(Some(dir.path()), true),
            ConnectorLocation {
                installed: true,
                path: Some(CONNECTOR_ALIAS.to_string()),
                kind: ConnectorKind::MsixAlias,
            }
        );
    }

    #[test]
    fn a_package_without_the_payload_file_reports_missing() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            resolve_location(Some(dir.path()), true),
            ConnectorLocation {
                installed: false,
                path: None,
                kind: ConnectorKind::Missing,
            }
        );
    }

    #[test]
    fn an_installer_that_carried_no_connector_reports_missing() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            resolve_location(Some(dir.path()), false),
            ConnectorLocation {
                installed: false,
                path: None,
                kind: ConnectorKind::Missing,
            }
        );
    }

    #[test]
    fn an_unresolvable_executable_path_is_missing_not_an_error() {
        assert_eq!(
            resolve_location(None, false),
            ConnectorLocation {
                installed: false,
                path: None,
                kind: ConnectorKind::Missing,
            }
        );
    }

    #[test]
    fn a_directory_named_like_the_connector_is_not_an_installed_connector() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(connector_file_name())).unwrap();

        assert_eq!(
            resolve_location(Some(dir.path()), false).kind,
            ConnectorKind::Missing
        );
    }

    #[test]
    fn the_panel_payload_names_the_three_kinds() {
        let bundled = ConnectorLocation {
            installed: true,
            path: Some("/usr/bin/entropia-agent-connector".to_string()),
            kind: ConnectorKind::Bundled,
        };
        assert_eq!(
            serde_json::to_value(&bundled).unwrap(),
            serde_json::json!({
                "installed": true,
                "path": "/usr/bin/entropia-agent-connector",
                "kind": "bundled",
            })
        );
        assert_eq!(
            serde_json::to_value(ConnectorKind::MsixAlias).unwrap(),
            serde_json::json!("msix_alias")
        );
        assert_eq!(
            serde_json::to_value(ConnectorKind::Missing).unwrap(),
            serde_json::json!("missing")
        );
    }
}
