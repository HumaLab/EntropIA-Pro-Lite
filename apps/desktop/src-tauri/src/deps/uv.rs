//! uv binary management for the dependency manager.
//!
//! uv is the fast Python package installer used to install deps into the
//! managed venv. This module resolves a pinned uv binary from bundled Tauri
//! resources first, then falls back to development resources, a legacy managed
//! copy under app-data, and finally the system `PATH`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tauri::Manager;
use tokio::process::Command;

use crate::path_utils::normalize_windows_path;
use crate::runtime::status::{RuntimeState, RuntimeStatus};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The pinned uv version used by the dependency manager.
pub const UV_VERSION: &str = "0.6.14";
const UV_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const UV_PATH_RESOLUTION_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(unix)]
const UV_EXECUTABLE_NAME: &str = "uv";
#[cfg(windows)]
const UV_EXECUTABLE_NAME: &str = "uv.exe";

#[cfg(any(windows, test))]
const UV_DOWNLOAD_URL_WINDOWS_X86_64: &str = concat!(
    "https://github.com/astral-sh/uv/releases/download/",
    "0.6.14",
    "/uv-x86_64-pc-windows-msvc.zip"
);

#[cfg(any(windows, test))]
const UV_DOWNLOAD_URL_WINDOWS_AARCH64: &str = concat!(
    "https://github.com/astral-sh/uv/releases/download/",
    "0.6.14",
    "/uv-aarch64-pc-windows-msvc.zip"
);

/// Total request timeout for the uv download (read + body), generous enough for a
/// ~15 MB zip on a slow corporate link but bounded so a dead proxy fails instead
/// of hanging forever.
#[cfg(any(windows, test))]
const UV_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Connect-phase timeout: a misconfigured proxy/firewall surfaces fast here.
#[cfg(any(windows, test))]
const UV_DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Bounded retries for the whole download+extract before giving up, mirroring the
/// backoff precedent in `deps/mod.rs::remove_dir_all_with_retry`.
#[cfg(any(windows, test))]
const UV_DOWNLOAD_ATTEMPTS: u32 = 3;

/// Sentinel meaning "no real hash pinned yet" — verification is skipped (with a
/// warning) until a real digest is filled in. Keeps clean builds working while
/// making the unpinned state explicit and grep-able.
#[cfg(any(windows, test))]
const UV_SHA256_UNPINNED: &str = "UNPINNED";

/// Expected SHA-256 of the extracted `uv.exe` for uv 0.6.14, x86_64.
///
/// TODO(security): replace `UV_SHA256_UNPINNED` with the real lowercase-hex
/// digest of `uv.exe` extracted from the official
/// `uv-x86_64-pc-windows-msvc.zip` (uv 0.6.14). Until then verification is a
/// no-op (see `verify_pinned_uv_sha256`).
#[cfg(any(windows, test))]
const EXPECTED_UV_SHA256_WINDOWS_X86_64: &str =
    "fad4db6a8f4898abc18a8b6e403ff181be0add5bf1fc90ea91c94e8dec24703f";

/// Expected SHA-256 of the extracted `uv.exe` for uv 0.6.14, aarch64.
///
/// TODO(security): replace with the real digest from
/// `uv-aarch64-pc-windows-msvc.zip` (uv 0.6.14).
#[cfg(any(windows, test))]
const EXPECTED_UV_SHA256_WINDOWS_AARCH64: &str = UV_SHA256_UNPINNED;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A located, version-verified uv binary ready to run.
#[derive(Clone, Debug)]
pub struct UvBinary {
    pub path: PathBuf,
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct UvInspection {
    pub ready: Option<UvBinary>,
    pub detected_path: Option<PathBuf>,
    pub detected_version: Option<String>,
    pub warning: Option<String>,
}

pub fn dev_system_uv_relaxed_allowed() -> bool {
    cfg!(all(
        debug_assertions,
        any(target_os = "linux", target_os = "windows")
    ))
}

fn dev_fallback_binary_from_inspection(inspection: UvInspection) -> Option<UvBinary> {
    inspection.ready.or_else(|| {
        if !dev_system_uv_relaxed_allowed() {
            return None;
        }

        inspection
            .detected_path
            .zip(inspection.detected_version)
            .map(|(path, version)| UvBinary { path, version })
    })
}

/// The current availability state of the uv binary.
pub enum UvStatus {
    /// Binary is present and matches the expected version.
    Ready(UvBinary),
    /// Binary not found at the expected path.
    NotFound,
    /// A download is in progress.
    Downloading { percent: u8 },
    /// Download or verification failed.
    Failed(String),
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowsUvArch {
    X86_64,
    Aarch64,
}

#[cfg(any(windows, test))]
impl WindowsUvArch {
    #[cfg(any(windows, test))]
    fn resource_dir(self) -> &'static str {
        match self {
            Self::X86_64 => "windows-x86_64",
            Self::Aarch64 => "windows-aarch64",
        }
    }

    #[cfg(any(windows, test))]
    fn download_url(self) -> &'static str {
        match self {
            Self::X86_64 => UV_DOWNLOAD_URL_WINDOWS_X86_64,
            Self::Aarch64 => UV_DOWNLOAD_URL_WINDOWS_AARCH64,
        }
    }

    /// Pinned SHA-256 of the extracted `uv.exe` for this arch, or `None` while
    /// the digest is still the `UV_SHA256_UNPINNED` placeholder.
    #[cfg(any(windows, test))]
    fn expected_uv_sha256(self) -> Option<&'static str> {
        let pinned = match self {
            Self::X86_64 => EXPECTED_UV_SHA256_WINDOWS_X86_64,
            Self::Aarch64 => EXPECTED_UV_SHA256_WINDOWS_AARCH64,
        };
        (pinned != UV_SHA256_UNPINNED).then_some(pinned)
    }
}

/// Verify a file's SHA-256 against an expected lowercase-hex digest, mirroring the
/// runtime-pack hash check in `runtime/download.rs` (`Sha256` over the bytes,
/// compare hex). Returns a Spanish error on mismatch or read failure.
#[cfg(any(windows, test))]
fn verify_pinned_uv_sha256(path: &Path, expected: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else {
        // No real digest pinned yet: do not block, but make the gap visible.
        eprintln!(
            "[deps/uv] uv.exe sin hash fijado todavía — se omite la verificación de integridad (UV_SHA256_UNPINNED)"
        );
        return Ok(());
    };

    use sha2::{Digest as _, Sha256};
    let bytes = std::fs::read(path).map_err(|e| {
        format!(
            "Error verificando uv: no se pudo leer {}: {e}",
            path.display()
        )
    })?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "Integridad de uv comprometida: el hash SHA-256 de {} no coincide con el esperado (esperado {expected}, obtenido {actual}). Es posible que el antivirus o un proxy hayan modificado la descarga. Eliminá el archivo y volvé a intentar.",
            path.display()
        ))
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns the directory where the versioned uv binary lives.
///
/// Example: `<app_data_dir>/tools/uv-0.6.14/`
pub fn uv_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("tools").join(format!("uv-{UV_VERSION}"))
}

/// Returns the full path to the uv executable.
///
/// Example: `<app_data_dir>/tools/uv-0.6.14/uv.exe`
pub fn uv_exe_path(app_data_dir: &Path) -> PathBuf {
    uv_dir(app_data_dir).join(UV_EXECUTABLE_NAME)
}

pub fn runtime_managed_uv_path(managed_runtime_root: &Path, uv_relpath: &str) -> PathBuf {
    managed_runtime_root.join(uv_relpath)
}

#[cfg(any(windows, test))]
fn preferred_windows_arches() -> Vec<WindowsUvArch> {
    let mut arches = Vec::new();

    for key in ["PROCESSOR_ARCHITEW6432", "PROCESSOR_ARCHITECTURE"] {
        let Some(value) = std::env::var_os(key) else {
            continue;
        };
        let Some(arch) = parse_windows_arch_value(&value.to_string_lossy()) else {
            continue;
        };
        if !arches.contains(&arch) {
            arches.push(arch);
        }
    }

    #[cfg(target_arch = "aarch64")]
    if !arches.contains(&WindowsUvArch::Aarch64) {
        arches.push(WindowsUvArch::Aarch64);
    }

    #[cfg(target_arch = "x86_64")]
    if !arches.contains(&WindowsUvArch::X86_64) {
        arches.push(WindowsUvArch::X86_64);
    }

    if arches.is_empty() {
        arches.push(WindowsUvArch::X86_64);
    }

    arches
}

#[cfg(any(windows, test))]
fn parse_windows_arch_value(value: &str) -> Option<WindowsUvArch> {
    match value.trim().to_ascii_uppercase().as_str() {
        "AMD64" | "X86_64" | "X64" => Some(WindowsUvArch::X86_64),
        "ARM64" | "AARCH64" => Some(WindowsUvArch::Aarch64),
        _ => None,
    }
}

#[cfg(windows)]
fn bundled_uv_resource_candidates() -> Vec<String> {
    preferred_windows_arches()
        .into_iter()
        .map(|arch| format!("resources/tools/uv/{}/uv.exe", arch.resource_dir()))
        .collect()
}

#[cfg(not(windows))]
fn bundled_uv_resource_candidates() -> Vec<String> {
    Vec::new()
}

fn bundled_uv_path(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    for resource_rel in bundled_uv_resource_candidates() {
        let Some(resolved) = app_handle
            .path()
            .resolve(&resource_rel, tauri::path::BaseDirectory::Resource)
            .ok()
            .map(normalize_windows_path)
        else {
            continue;
        };

        if resolved.exists() {
            return Some(resolved);
        }
    }

    None
}

/// Best-effort check: can we create files in `dir`? Bundled uv often lands under
/// `Program Files`, which is read-only for a standard user; running from there is
/// fine, but copying/repairing next to it is not. Returns `false` when `dir` is
/// missing or a probe file cannot be written.
fn dir_is_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(".entropia-uv-write-probe.tmp");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Self-heal a bundled uv that lives in a non-writable location (e.g. Program
/// Files) or whose host dir we cannot write to: copy it ONCE into the writable
/// app-data tools dir and return that path so we can run/repair from there.
///
/// Idempotent — if a healthy managed copy already exists it is reused. Returns
/// `None` (caller keeps using the original bundled path) when the copy fails or
/// when the bundled dir is already writable.
fn self_heal_bundled_uv(bundled: &Path, app_data_dir: &Path) -> Option<PathBuf> {
    let bundled_dir = bundled.parent()?;
    // If we can write next to the bundled binary, there is nothing to heal.
    if dir_is_writable(bundled_dir) {
        return None;
    }

    let managed = uv_exe_path(app_data_dir);
    // Reuse a previously-healed copy if it is already a valid, version-matching uv.
    if detect_file(&managed).is_some() {
        return Some(managed);
    }

    let managed_dir = uv_dir(app_data_dir);
    std::fs::create_dir_all(&managed_dir).ok()?;
    std::fs::copy(bundled, &managed).ok()?;
    Some(normalize_windows_path(managed))
}

#[cfg(windows)]
fn dev_uv_candidates() -> Vec<PathBuf> {
    preferred_windows_arches()
        .into_iter()
        .map(|arch| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources")
                .join("tools")
                .join("uv")
                .join(arch.resource_dir())
                .join("uv.exe")
        })
        .collect()
}

#[cfg(not(windows))]
fn dev_uv_candidates() -> Vec<PathBuf> {
    Vec::new()
}

fn dev_uv_path() -> Option<PathBuf> {
    for path in dev_uv_candidates() {
        if path.exists() {
            return Some(normalize_windows_path(path));
        }
    }

    None
}

#[cfg(windows)]
fn resolve_system_uv_path() -> Option<PathBuf> {
    let mut cmd = std::process::Command::new("where.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as StdCommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.arg("uv")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output =
        command_output_with_timeout(cmd, UV_PATH_RESOLUTION_TIMEOUT, "where.exe uv").ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .map(normalize_windows_path)
}

#[cfg(not(windows))]
fn resolve_system_uv_path() -> Option<PathBuf> {
    let mut cmd = std::process::Command::new("which");
    cmd.arg("uv")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = command_output_with_timeout(cmd, UV_PATH_RESOLUTION_TIMEOUT, "which uv").ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug)]
enum ProbedUv {
    Ready(UvBinary),
    VersionMismatch { path: PathBuf, version: String },
    NotUsable,
}

fn version_from_output(output: &std::process::Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();

    trimmed
        .strip_prefix("uv ")
        .map(str::trim)
        .or_else(|| (!trimmed.is_empty()).then_some(trimmed))
        .map(ToOwned::to_owned)
}

fn is_compatible_uv_version(version: &str) -> bool {
    version.split_whitespace().next() == Some(UV_VERSION)
}

fn command_output_with_timeout(
    mut cmd: std::process::Command,
    timeout: Duration,
    label: &str,
) -> Result<std::process::Output, String> {
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("{label} failed to start: {error}"))?;
    let started_at = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("{label} failed to collect output: {error}"));
            }
            Ok(None) if started_at.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} failed while waiting: {error}"));
            }
        }
    }
}

fn probe_uv_command(mut cmd: std::process::Command, path: PathBuf) -> ProbedUv {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as StdCommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = command_output_with_timeout(cmd, UV_PROBE_TIMEOUT, "uv --version");

    let Ok(output) = output else {
        return ProbedUv::NotUsable;
    };

    let Some(version) = version_from_output(&output) else {
        return ProbedUv::NotUsable;
    };

    if is_compatible_uv_version(&version) {
        ProbedUv::Ready(UvBinary { path, version })
    } else {
        ProbedUv::VersionMismatch { path, version }
    }
}

fn detect_file(exe: &Path) -> Option<UvBinary> {
    if !exe.is_file() {
        return None;
    }

    match probe_uv_command(std::process::Command::new(exe), exe.to_path_buf()) {
        ProbedUv::Ready(binary) => Some(binary),
        ProbedUv::VersionMismatch { .. } | ProbedUv::NotUsable => None,
    }
}

/// Detect the bundled uv, self-healing first when its host dir is non-writable
/// (Program Files): run from a writable app-data copy so later repair/locking is
/// possible. Falls back to probing the original bundled path directly (covers the
/// common case where the bundled dir is writable, or the copy was unnecessary).
fn detect_bundled_uv_with_self_heal(bundled: &Path, app_data_dir: &Path) -> Option<UvBinary> {
    if let Some(healed) = self_heal_bundled_uv(bundled, app_data_dir) {
        if let Some(binary) = detect_file(&healed) {
            return Some(binary);
        }
    }
    detect_file(bundled)
}

fn inspect_file(exe: &Path) -> Option<UvInspection> {
    if !exe.is_file() {
        return None;
    }

    Some(match probe_uv_command(std::process::Command::new(exe), exe.to_path_buf()) {
        ProbedUv::Ready(binary) => UvInspection {
            detected_path: Some(binary.path.clone()),
            detected_version: Some(binary.version.clone()),
            ready: Some(binary),
            warning: None,
        },
        ProbedUv::VersionMismatch { path, version } => UvInspection {
            ready: None,
            detected_path: Some(path.clone()),
            detected_version: Some(version.clone()),
            warning: Some(format!(
                "Se detectó uv {version} en {}, pero EntropIA espera uv {UV_VERSION} para instalaciones administradas. No es un crash: alineá la versión o usá el runtime hidratado.",
                path.display()
            )),
        },
        ProbedUv::NotUsable => UvInspection {
            ready: None,
            detected_path: Some(exe.to_path_buf()),
            detected_version: None,
            warning: Some(format!(
                "Se encontró un ejecutable uv en {}, pero no respondió correctamente a `uv --version`.",
                exe.display()
            )),
        },
    })
}

fn detect_on_path() -> Option<UvBinary> {
    let resolved = resolve_system_uv_path().unwrap_or_else(|| PathBuf::from("uv"));
    let command_path = if resolved.is_file() {
        resolved.clone()
    } else {
        PathBuf::from("uv")
    };
    match probe_uv_command(std::process::Command::new(command_path), resolved) {
        ProbedUv::Ready(binary) => Some(binary),
        ProbedUv::VersionMismatch { .. } | ProbedUv::NotUsable => None,
    }
}

fn inspect_on_path() -> Option<UvInspection> {
    let resolved = resolve_system_uv_path().unwrap_or_else(|| PathBuf::from("uv"));
    let command_path = if resolved.is_file() {
        resolved.clone()
    } else {
        PathBuf::from("uv")
    };

    Some(match probe_uv_command(std::process::Command::new(command_path), resolved.clone()) {
        ProbedUv::Ready(binary) => UvInspection {
            detected_path: Some(binary.path.clone()),
            detected_version: Some(binary.version.clone()),
            ready: Some(binary),
            warning: None,
        },
        ProbedUv::VersionMismatch { path, version } => UvInspection {
            ready: None,
            detected_path: Some(path.clone()),
            detected_version: Some(version.clone()),
            warning: Some(format!(
                "Se detectó uv {version} en {}, pero EntropIA espera uv {UV_VERSION} para instalaciones administradas. En desarrollo esto explica el warning, no una caída de la app.",
                path.display()
            )),
        },
        ProbedUv::NotUsable => UvInspection {
            ready: None,
            detected_path: Some(resolved.clone()),
            detected_version: None,
            warning: Some(format!(
                "Se intentó usar uv desde {}, pero el ejecutable no respondió correctamente.",
                resolved.display()
            )),
        },
    })
}

// ---------------------------------------------------------------------------
// UvBinary impl
// ---------------------------------------------------------------------------

impl UvBinary {
    /// Detect a valid, version-matching uv binary using the full resolution order:
    /// bundled resource → dev fallback → managed app-data copy → system PATH.
    pub fn detect(app_handle: Option<&tauri::AppHandle>, app_data_dir: &Path) -> Option<UvBinary> {
        app_handle
            .and_then(bundled_uv_path)
            .and_then(|path| detect_bundled_uv_with_self_heal(&path, app_data_dir))
            .or_else(|| dev_uv_path().and_then(|path| detect_file(&path)))
            .or_else(|| detect_file(&uv_exe_path(app_data_dir)))
            .or_else(detect_on_path)
    }

    pub fn detect_with_runtime(
        app_handle: Option<&tauri::AppHandle>,
        app_data_dir: &Path,
        managed_runtime_uv: Option<&Path>,
        managed_runtime_status: Option<&RuntimeStatus>,
    ) -> Option<UvBinary> {
        if matches!(
            managed_runtime_status.map(|status| &status.state),
            Some(RuntimeState::Healthy)
        ) {
            if let Some(binary) = managed_runtime_uv.and_then(detect_file) {
                return Some(binary);
            }
        }

        Self::detect(app_handle, app_data_dir)
    }

    pub fn inspect_with_runtime(
        app_handle: Option<&tauri::AppHandle>,
        app_data_dir: &Path,
        managed_runtime_uv: Option<&Path>,
        managed_runtime_status: Option<&RuntimeStatus>,
    ) -> UvInspection {
        let mut first_warning = None;

        let mut inspect_candidate = |candidate: Option<UvInspection>| {
            let Some(candidate) = candidate else {
                return None;
            };
            if candidate.ready.is_some() {
                return Some(candidate);
            }
            if first_warning.is_none() && candidate.warning.is_some() {
                first_warning = Some(candidate);
            }
            None
        };

        if matches!(
            managed_runtime_status.map(|status| &status.state),
            Some(RuntimeState::Healthy)
        ) {
            if let Some(candidate) = inspect_candidate(managed_runtime_uv.and_then(inspect_file)) {
                return candidate;
            }
        }

        if let Some(candidate) = inspect_candidate(
            app_handle
                .and_then(bundled_uv_path)
                .and_then(|path| inspect_file(&path)),
        ) {
            return candidate;
        }
        if let Some(candidate) =
            inspect_candidate(dev_uv_path().and_then(|path| inspect_file(&path)))
        {
            return candidate;
        }
        if let Some(candidate) = inspect_candidate(inspect_file(&uv_exe_path(app_data_dir))) {
            return candidate;
        }
        if let Some(candidate) = inspect_candidate(inspect_on_path()) {
            return candidate;
        }

        first_warning.unwrap_or(UvInspection {
            ready: None,
            detected_path: None,
            detected_version: None,
            warning: None,
        })
    }

    pub fn detect_dev_fallback(app_data_dir: &Path) -> Option<UvBinary> {
        detect_file(&uv_exe_path(app_data_dir))
            .or_else(|| inspect_on_path().and_then(dev_fallback_binary_from_inspection))
    }

    /// Build a tokio `Command` pre-configured with `CREATE_NO_WINDOW` on
    /// Windows. Callers add args before spawning.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.path);

        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd
    }
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// Download the pinned uv binary into the managed app-data tools directory.
///
/// Resolution order elsewhere now prefers bundled resources; download remains a
/// fallback when no bundled/dev/system uv is available.
pub async fn download(
    app_data_dir: &Path,
    on_progress: impl Fn(u8, &str) + Send + 'static,
) -> Result<UvBinary, String> {
    #[cfg(not(windows))]
    {
        let _ = app_data_dir;
        let _ = on_progress;
        return Err(
            "No hay descarga administrada de uv para esta plataforma. En Linux/macOS usá el runtime hidratado (uv/bin/uv) o instalá uv en el PATH."
                .to_string(),
        );
    }

    #[cfg(windows)]
    {
        let target_arch = preferred_windows_arches()
            .into_iter()
            .next()
            .unwrap_or(WindowsUvArch::X86_64);

        let dir = uv_dir(app_data_dir);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("Error creando directorio para uv: {e}"))?;

        let exe_path = uv_exe_path(app_data_dir);

        // Bounded retry-with-backoff around the whole download+extract, mirroring
        // the backoff precedent in `deps/mod.rs::remove_dir_all_with_retry`. A
        // flaky/corporate link or a transient AV lock gets a few chances before we
        // surface an antivirus/proxy-aware message.
        let mut last_err = String::new();
        for attempt in 0..UV_DOWNLOAD_ATTEMPTS {
            match download_uv_once(target_arch, &dir, &exe_path, &on_progress).await {
                Ok(binary) => {
                    on_progress(100, "uv listo");
                    return Ok(binary);
                }
                Err(error) => {
                    last_err = error;
                    let _ = std::fs::remove_file(&exe_path);
                    if attempt + 1 < UV_DOWNLOAD_ATTEMPTS {
                        on_progress(0, "Reintentando descarga de uv…");
                        tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt + 1)))
                            .await;
                    }
                }
            }
        }

        Err(format!(
            "No se pudo preparar uv tras {UV_DOWNLOAD_ATTEMPTS} intentos: {last_err}. Probá esto: \
             (1) revisá tu conexión y, si usás un proxy corporativo, definí HTTPS_PROXY; \
             (2) agregá una excepción de antivirus para {} y para la carpeta {}; \
             (3) volvé a intentar la instalación.",
            exe_path.display(),
            dir.display()
        ))
    }
}

/// One download+extract+verify attempt for the pinned uv. Kept separate so the
/// public `download` can wrap it in a bounded retry loop. Builds a reqwest client
/// with explicit timeouts; proxy env vars (`HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY`)
/// are honored by reqwest's default system-proxy detection.
#[cfg(windows)]
async fn download_uv_once(
    target_arch: WindowsUvArch,
    dir: &Path,
    exe_path: &Path,
    on_progress: &(impl Fn(u8, &str) + Send + 'static),
) -> Result<UvBinary, String> {
    use std::io::{Read as _, Write as _};

    on_progress(0, "Descargando uv…");

    // Explicit timeouts + default system-proxy detection (honors HTTPS_PROXY).
    let client = reqwest::Client::builder()
        .timeout(UV_DOWNLOAD_TIMEOUT)
        .connect_timeout(UV_DOWNLOAD_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Error creando cliente HTTP para uv: {e}"))?;

    let mut response = client
        .get(target_arch.download_url())
        .send()
        .await
        .map_err(|e| format!("Error descargando uv: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Error descargando uv: respuesta HTTP {}",
            response.status()
        ));
    }

    let content_length: Option<u64> = response.content_length();
    let tmp_zip_path = dir.join("uv-download.zip.tmp");

    {
        let mut file = std::fs::File::create(&tmp_zip_path)
            .map_err(|e| format!("Error creando archivo temporal: {e}"))?;

        let mut downloaded: u64 = 0;
        let mut last_reported_pct: u8 = 0;

        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("Error descargando uv: {e}"))?
        {
            file.write_all(&chunk)
                .map_err(|e| format!("Error escribiendo archivo temporal: {e}"))?;
            downloaded += chunk.len() as u64;

            if let Some(total) = content_length {
                let pct = ((downloaded * 100) / total).min(99) as u8;
                let mb_boundary = (downloaded / (1024 * 1024))
                    != ((downloaded - chunk.len() as u64) / (1024 * 1024));
                if pct >= last_reported_pct + 5 || mb_boundary {
                    last_reported_pct = pct;
                    on_progress(pct, &format!("Descargando uv… {pct}%"));
                }
            }
        }
    }

    on_progress(90, "Extrayendo uv…");

    let extract_result = (|| -> Result<(), String> {
        let zip_file =
            std::fs::File::open(&tmp_zip_path).map_err(|e| format!("Error abriendo ZIP: {e}"))?;
        let mut archive =
            zip::ZipArchive::new(zip_file).map_err(|e| format!("Error extrayendo uv: {e}"))?;

        let entry_index = (0..archive.len())
            .find(|&i| {
                archive
                    .by_index(i)
                    .map(|f| {
                        let name = f.name().to_ascii_lowercase();
                        name == "uv.exe" || name.ends_with("/uv.exe")
                    })
                    .unwrap_or(false)
            })
            .ok_or_else(|| "Error extrayendo uv: uv.exe no encontrado en el ZIP".to_string())?;

        let mut entry = archive
            .by_index(entry_index)
            .map_err(|e| format!("Error extrayendo uv: {e}"))?;

        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("Error extrayendo uv: {e}"))?;

        std::fs::write(exe_path, &buf).map_err(|e| format!("Error extrayendo uv: {e}"))?;

        Ok(())
    })();

    let _ = std::fs::remove_file(&tmp_zip_path);

    extract_result?;

    on_progress(95, "Verificando uv…");

    // Pin check: reject a tampered/AV-modified binary before we trust it. No-op
    // while the digest is still the UV_SHA256_UNPINNED placeholder.
    verify_pinned_uv_sha256(exe_path, target_arch.expected_uv_sha256())?;

    detect_file(exe_path).ok_or_else(|| "Versión incorrecta de uv".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
    use tempfile::tempdir;

    #[test]
    fn test_uv_exe_path_contains_version() {
        let base = Path::new("/some/app/data");
        let exe = uv_exe_path(base);
        let exe_str = exe.to_string_lossy();
        assert!(
            exe_str.contains(UV_VERSION),
            "uv exe path should contain the version string '{UV_VERSION}', got: {exe_str}"
        );
        assert_eq!(
            exe.file_name().and_then(|name| name.to_str()),
            Some(UV_EXECUTABLE_NAME)
        );
    }

    #[test]
    fn test_uv_dir_is_parent_of_exe() {
        let base = Path::new("/some/app/data");
        let dir = uv_dir(base);
        let exe = uv_exe_path(base);
        assert_eq!(
            exe.parent().unwrap(),
            dir,
            "uv_exe_path parent should equal uv_dir"
        );
    }

    #[test]
    fn test_parse_windows_arch_value() {
        assert_eq!(
            parse_windows_arch_value("AMD64"),
            Some(WindowsUvArch::X86_64)
        );
        assert_eq!(
            parse_windows_arch_value("arm64"),
            Some(WindowsUvArch::Aarch64)
        );
        assert_eq!(parse_windows_arch_value("mips"), None);
    }

    #[test]
    fn test_preferred_windows_arches_never_empty() {
        assert!(
            !preferred_windows_arches().is_empty(),
            "preferred_windows_arches should always return at least one supported arch"
        );
    }

    #[test]
    fn test_dir_is_writable_detects_real_and_missing_dirs() {
        let dir = tempdir().expect("temp dir");
        assert!(
            dir_is_writable(dir.path()),
            "a freshly created temp dir should be writable"
        );
        assert!(
            !dir_is_writable(&dir.path().join("does-not-exist")),
            "a missing dir must not be reported as writable"
        );
    }

    #[test]
    fn test_self_heal_is_noop_when_bundled_dir_is_writable() {
        // When the bundled uv already lives in a writable dir there is nothing to
        // heal — the caller keeps using the original path.
        let bundled_dir = tempdir().expect("bundled dir");
        let bundled = bundled_dir.path().join(UV_EXECUTABLE_NAME);
        fs::write(&bundled, b"fake-uv").expect("write bundled uv");
        let app_data = tempdir().expect("app data dir");

        assert!(
            self_heal_bundled_uv(&bundled, app_data.path()).is_none(),
            "writable bundled dir must not trigger a self-heal copy"
        );
        assert!(
            !uv_exe_path(app_data.path()).exists(),
            "no managed copy should be created when self-heal is skipped"
        );
    }

    #[test]
    fn test_verify_pinned_uv_sha256_is_noop_when_unpinned() {
        let dir = tempdir().expect("temp dir");
        let exe = dir.path().join(UV_EXECUTABLE_NAME);
        fs::write(&exe, b"anything").expect("write fake uv");

        // `None` models the UV_SHA256_UNPINNED placeholder: verification is skipped.
        assert!(verify_pinned_uv_sha256(&exe, None).is_ok());
    }

    #[test]
    fn test_verify_pinned_uv_sha256_accepts_match_and_rejects_mismatch() {
        use sha2::{Digest as _, Sha256};
        let dir = tempdir().expect("temp dir");
        let exe = dir.path().join(UV_EXECUTABLE_NAME);
        let bytes = b"pinned-uv-bytes";
        fs::write(&exe, bytes).expect("write fake uv");
        let expected = format!("{:x}", Sha256::digest(bytes));

        assert!(
            verify_pinned_uv_sha256(&exe, Some(&expected)).is_ok(),
            "matching digest must pass"
        );

        let error = verify_pinned_uv_sha256(&exe, Some("deadbeef"))
            .expect_err("mismatched digest must fail");
        assert!(
            error.contains("Integridad de uv comprometida"),
            "mismatch should surface the AV/proxy-aware Spanish message, got: {error}"
        );
    }

    #[test]
    fn test_expected_uv_sha256_tracks_pinned_constants() {
        // x86_64 has a real pinned digest; aarch64 stays unpinned (`None`) until
        // its digest is filled in, so verification there remains a documented
        // no-op. Update this test alongside the pinned constants.
        assert_eq!(
            WindowsUvArch::X86_64.expected_uv_sha256(),
            Some(EXPECTED_UV_SHA256_WINDOWS_X86_64)
        );
        assert_eq!(WindowsUvArch::Aarch64.expected_uv_sha256(), None);
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_candidate_lists_are_not_empty() {
        assert!(
            !bundled_uv_resource_candidates().is_empty(),
            "windows should probe bundled uv.exe resources"
        );
        assert!(
            !dev_uv_candidates().is_empty(),
            "windows should probe dev uv.exe resources"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn test_non_windows_candidate_lists_ignore_windows_uv_resources() {
        assert!(
            bundled_uv_resource_candidates().is_empty(),
            "non-Windows hosts must not probe bundled Windows uv.exe resources"
        );
        assert!(
            dev_uv_candidates().is_empty(),
            "non-Windows hosts must not probe dev Windows uv.exe resources"
        );
        assert!(
            dev_uv_path().is_none(),
            "non-Windows hosts must not surface repo Windows uv.exe fixtures as candidates"
        );
    }

    #[test]
    fn test_runtime_managed_uv_path_appends_manifest_relative_path() {
        let root = Path::new("/tmp/runtime/2026.05.0");
        let rel = if cfg!(windows) {
            "uv/uv.exe"
        } else {
            "uv/bin/uv"
        };

        assert_eq!(runtime_managed_uv_path(root, rel), root.join(rel));
    }

    #[test]
    fn test_dev_fallback_binary_from_inspection_respects_dev_policy() {
        let inspection = UvInspection {
            ready: None,
            detected_path: Some(PathBuf::from("/usr/bin/uv")),
            detected_version: Some("0.10.3".to_string()),
            warning: Some("version mismatch".to_string()),
        };

        let binary = dev_fallback_binary_from_inspection(inspection);

        if dev_system_uv_relaxed_allowed() {
            let binary =
                binary.expect("linux debug builds should accept system uv for dev fallback");
            assert_eq!(binary.path, PathBuf::from("/usr/bin/uv"));
            assert_eq!(binary.version, "0.10.3");
        } else {
            assert!(
                binary.is_none(),
                "non-dev policy must keep exact uv pinning strict"
            );
        }
    }

    #[test]
    fn test_detect_with_runtime_prefers_healthy_managed_uv_binary() {
        let dir = tempdir().expect("temp dir");
        let managed_uv = dir.path().join(UV_EXECUTABLE_NAME);
        fs::write(&managed_uv, "not-a-real-binary").expect("write managed uv");

        let status = RuntimeStatus {
            state: RuntimeState::Healthy,
            pack_version: Some("2026.05.0".to_string()),
            repair_needed: false,
            repair_available: true,
            summary: "Runtime listo".to_string(),
            blocked_capabilities: Vec::<RuntimeCapability>::new(),
            details: vec![],
            guidance: vec![],
            bootstrap_eligible: false,
            bootstrap_required: false,
            active_operation: None,
        };

        let detected =
            UvBinary::detect_with_runtime(None, dir.path(), Some(&managed_uv), Some(&status));

        if let Some(binary) = detected {
            assert_ne!(
                binary.path, managed_uv,
                "invalid fake uv should not be accepted as the managed runtime binary"
            );
        }
    }

    #[test]
    fn test_version_from_output_parses_plain_uv_version() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"uv 0.10.3\n".to_vec(),
            stderr: Vec::new(),
        };

        assert_eq!(version_from_output(&output).as_deref(), Some("0.10.3"));
    }

    #[test]
    fn test_uv_version_accepts_build_metadata_suffix() {
        assert!(is_compatible_uv_version("0.6.14 (a4cec56dc 2025-04-09)"));
        assert!(!is_compatible_uv_version("0.6.15 (different build)"));
    }

    #[test]
    fn test_command_output_with_timeout_kills_slow_probe() {
        let mut cmd = if cfg!(windows) {
            let mut cmd = std::process::Command::new(
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            );
            cmd.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 2"]);
            cmd
        } else {
            let mut cmd = std::process::Command::new("sh");
            cmd.args(["-c", "sleep 2"]);
            cmd
        };
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let started_at = Instant::now();
        let error = command_output_with_timeout(cmd, Duration::from_millis(100), "slow uv probe")
            .expect_err("slow probe should time out");

        assert!(error.contains("timed out"));
        if !cfg!(windows) {
            assert!(
                started_at.elapsed() < Duration::from_secs(1),
                "timeout helper should not wait for the child to finish naturally"
            );
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn test_download_reports_platform_specific_error_on_non_windows() {
        let dir = tempdir().expect("temp dir");

        let error = download(dir.path(), |_percent, _message| {})
            .await
            .expect_err("non-Windows download should fail fast");

        assert!(
            error.contains("Linux/macOS") || error.contains("esta plataforma"),
            "expected platform-specific guidance, got: {error}"
        );
    }
}
