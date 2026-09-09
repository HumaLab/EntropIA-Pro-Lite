//! Venv creation and package installation for the dependency manager.
//!
//! Uses the managed uv binary to create an isolated Python 3.11 venv and
//! install each registered dependency into it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tauri::Manager;
use tokio::io::AsyncBufReadExt as _;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::{DepCheckResult, DependencyId, DependencyStatus, DepsState};
use crate::deps::checks::{probe_one, ProbePythonMode};
use crate::deps::registry::{all_deps_in_install_order, find_dep, DependencySpec};
use crate::deps::uv::{self, UvBinary};
use crate::runtime::manifest::RuntimeManifest;
use crate::runtime::status::{RuntimeState, RuntimeStatus};
use crate::runtime::{
    managed_entry_path, managed_venv_dir, managed_venv_python_path, managed_wheelhouse_dir,
    RuntimeManager,
};

// ---------------------------------------------------------------------------
// GPU / CUDA detection for PaddlePaddle automatic GPU selection
// ---------------------------------------------------------------------------

const GPU_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const CUDA_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Detect whether an NVIDIA GPU is present on the system.
///
/// Uses multiple hardware signals, not only `nvidia-smi`.
///
/// `nvidia-smi` can fail during driver/library mismatch windows (common after
/// driver upgrades before reboot). In that state the hardware still exists and
/// the dependency manager should install the GPU Paddle wheel; runtime GPU use
/// will still depend on the driver becoming healthy.
///
/// This is intentionally duplicated from `ocr::paddle_vl` to keep the
/// dependency manager decoupled from the OCR module.
fn detect_nvidia_gpu() -> bool {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.arg("-L");
    match std_command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi -L") {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let has_gpu = !stdout.trim().is_empty() && stdout.contains("GPU");
            if has_gpu {
                eprintln!("[deps/install] detect_nvidia_gpu: found GPU via nvidia-smi -L");
            }
            if has_gpu {
                return true;
            }
        }
        _ => {}
    }

    detect_nvidia_gpu_from_system_inventory()
}

#[cfg(target_os = "linux")]
fn detect_nvidia_gpu_from_system_inventory() -> bool {
    if std::fs::read_dir("/proc/driver/nvidia/gpus")
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
    {
        eprintln!("[deps/install] detect_nvidia_gpu: found GPU via /proc/driver/nvidia/gpus");
        return true;
    }

    let Ok(entries) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return false;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let vendor = std::fs::read_to_string(path.join("vendor")).unwrap_or_default();
        if vendor.trim() != "0x10de" {
            continue;
        }

        let class = std::fs::read_to_string(path.join("class")).unwrap_or_default();
        if class.trim().starts_with("0x03") {
            eprintln!("[deps/install] detect_nvidia_gpu: found GPU via PCI inventory");
            return true;
        }
    }

    false
}

#[cfg(windows)]
fn detect_nvidia_gpu_from_system_inventory() -> bool {
    let candidates = [
        std::env::var_os("ProgramFiles").map(PathBuf::from),
        std::env::var_os("ProgramW6432").map(PathBuf::from),
    ];

    for base in candidates.into_iter().flatten() {
        let smi = base
            .join("NVIDIA Corporation")
            .join("NVSMI")
            .join("nvidia-smi.exe");
        if !smi.exists() {
            continue;
        }
        let mut cmd = std::process::Command::new(&smi);
        cmd.arg("-L");
        if let Ok(output) =
            std_command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi.exe -L")
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if output.status.success() && stdout.contains("GPU") {
                eprintln!(
                    "[deps/install] detect_nvidia_gpu: found GPU via {}",
                    smi.display()
                );
                return true;
            }
        }
    }

    false
}

#[cfg(not(any(target_os = "linux", windows)))]
fn detect_nvidia_gpu_from_system_inventory() -> bool {
    false
}

/// Detect the system's CUDA version string (e.g. "12.6").
///
/// Tries, in order:
/// 1. `nvcc --version`
/// 2. `/usr/local/cuda/version.json`
/// 3. `/usr/local/cuda/version.txt`
///
/// Returns `None` if CUDA is not installed or the version cannot be parsed.
fn detect_cuda_version() -> Option<String> {
    // 1. nvcc --version
    let mut cmd = std::process::Command::new("nvcc");
    cmd.arg("--version");
    if let Ok(output) = std_command_output_with_timeout(cmd, CUDA_PROBE_TIMEOUT, "nvcc --version") {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(re) = regex::Regex::new(r"release\s+(\d+\.\d+)") {
                if let Some(caps) = re.captures(&stdout) {
                    if let Some(m) = caps.get(1) {
                        return Some(m.as_str().to_string());
                    }
                }
            }
        }
    }

    // 2. /usr/local/cuda/version.json
    if let Ok(content) = std::fs::read_to_string("/usr/local/cuda/version.json") {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(ver) = json.get("version").and_then(|v| v.as_str()) {
                return Some(ver.to_string());
            }
        }
    }

    // 3. /usr/local/cuda/version.txt
    if let Ok(content) = std::fs::read_to_string("/usr/local/cuda/version.txt") {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn std_command_output_with_timeout(
    mut cmd: std::process::Command,
    timeout: Duration,
    label: &str,
) -> Result<std::process::Output, String> {
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
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

/// Map a detected CUDA version to the PaddlePaddle stable package index URL.
///
/// PaddlePaddle distributes GPU builds on their own index (not PyPI).
/// The index path is determined by the CUDA major/minor version.
///
/// Defaults to `cu126` (CUDA 12.6) when the version is unknown — this is
/// the safest modern default for recent NVIDIA drivers.
fn paddlepaddle_cuda_index(cuda_version: Option<&str>) -> &'static str {
    let major = cuda_version
        .and_then(|v| v.split('.').next())
        .and_then(|m| m.parse::<u32>().ok())
        .unwrap_or(12);

    let minor = cuda_version
        .and_then(|v| v.split('.').nth(1))
        .and_then(|m| m.parse::<u32>().ok())
        .unwrap_or(6);

    match (major, minor) {
        (11, _) => "https://www.paddlepaddle.org.cn/packages/stable/cu118/",
        (12, 0..=5) => "https://www.paddlepaddle.org.cn/packages/stable/cu126/",
        (12, 6..=8) => "https://www.paddlepaddle.org.cn/packages/stable/cu126/",
        (12, 9..=99) => "https://www.paddlepaddle.org.cn/packages/stable/cu129/",
        (13, _) => "https://www.paddlepaddle.org.cn/packages/stable/cu130/",
        _ => "https://www.paddlepaddle.org.cn/packages/stable/cu126/",
    }
}

/// Find a wheel file in a directory whose filename starts with the given package prefix.
///
/// Wheel filenames use the format `<package>-<version>-... .whl`. This helper
/// requires the prefix to be followed by a separator (`-` or `_`) and then a
/// version digit. This prevents `paddlepaddle` from falsely matching
/// `paddlepaddle_gpu-...` (the character after the separator is `g`, not a digit).
fn find_wheel_in_dir(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !name.ends_with(".whl") {
            continue;
        }
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.starts_with('-') || rest.starts_with('_') {
                if rest.chars().nth(1).map_or(false, |c| c.is_ascii_digit()) {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Resolve the install target (spec + optional extra index) for PaddlePaddle.
///
/// Strategy:
/// 1. **Managed runtime** (`wheelhouse_dir` is `Some`): prefer a GPU wheel
///    (`paddlepaddle_gpu-*.whl`) in the wheelhouse, then a CPU wheel
///    (`paddlepaddle-*.whl`). This lets production builds bundle GPU support
///    without forcing a ~759 MB wheel by default.
/// 2. **Dev fallback** (`wheelhouse_dir` is `None`): detect NVIDIA GPU hardware.
///    If present, select `paddlepaddle-gpu` from the PaddlePaddle CUDA index.
///    If absent, select `paddlepaddle` (CPU) from PyPI.
///
/// The caller (`install_package`) passes `--extra-index-url` when an index
/// is returned, so PyPI remains available for transitive dependencies.
fn resolve_paddlepaddle_install_target(wheelhouse_dir: Option<&Path>) -> (String, Option<String>) {
    const VERSION_SPEC: &str = ">=3.2.1,<3.3.0";

    if let Some(dir) = wheelhouse_dir {
        if let Some(gpu_wheel) = find_wheel_in_dir(dir, "paddlepaddle_gpu") {
            eprintln!(
                "[deps/install] Using bundled GPU wheel: {}",
                gpu_wheel.display()
            );
            return (gpu_wheel.to_string_lossy().into_owned(), None);
        }
        if let Some(cpu_wheel) = find_wheel_in_dir(dir, "paddlepaddle") {
            eprintln!(
                "[deps/install] Using bundled CPU wheel: {}",
                cpu_wheel.display()
            );
            return (cpu_wheel.to_string_lossy().into_owned(), None);
        }
        // No matching wheel in wheelhouse — fall through to online spec.
        // In managed mode this will likely fail (offline), but that's the
        // honest behaviour; the wheelhouse should contain the wheel.
    }

    if detect_nvidia_gpu() {
        let spec = format!("paddlepaddle-gpu{VERSION_SPEC}");
        let cuda_ver = detect_cuda_version();
        let index = paddlepaddle_cuda_index(cuda_ver.as_deref()).to_string();
        eprintln!("[deps/install] NVIDIA GPU detected — selecting {spec} from {index}");
        (spec, Some(index))
    } else {
        eprintln!(
            "[deps/install] No NVIDIA GPU detected — selecting paddlepaddle{VERSION_SPEC} (CPU) from PyPI"
        );
        (format!("paddlepaddle{VERSION_SPEC}"), None)
    }
}
const UV_VENV_TIMEOUT: Duration = Duration::from_secs(180);
const UV_PIP_INSTALL_TIMEOUT: Duration = Duration::from_secs(1800);
const SUBPROCESS_TAIL_LINES: usize = 20;
const BUILD_BACKEND_SPAM_REPORT_EVERY: usize = 100;

const INSTALL_ENV_OVERRIDES: &[(&str, Option<&str>)] = &[
    ("RUST_LOG", Some("warn")),
    ("RUST_BACKTRACE", None),
    ("RUST_LIB_BACKTRACE", None),
    ("MATURIN_LOG", Some("warn")),
    ("PYO3_LOG", Some("warn")),
];

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns the directory where the managed venv lives.
///
/// Example: `<managed_runtime_root>/venv/entropia-env`
pub fn venv_path(managed_runtime_root: &Path) -> PathBuf {
    managed_venv_dir(managed_runtime_root)
}

/// Returns the path to the Python interpreter inside the managed venv.
///
/// Example: `<managed_runtime_root>/venv/entropia-env/Scripts/python.exe`
pub fn venv_python_path(managed_runtime_root: &Path) -> PathBuf {
    managed_venv_python_path(managed_runtime_root)
}

#[derive(Clone, Debug)]
pub struct ManagedRuntimeContext {
    pub managed_root: PathBuf,
    pub manifest: RuntimeManifest,
    pub status: RuntimeStatus,
}

impl ManagedRuntimeContext {
    pub fn managed_python(&self) -> PathBuf {
        managed_entry_path(&self.managed_root, &self.manifest.python_relpath)
    }

    pub fn managed_uv(&self) -> PathBuf {
        managed_entry_path(&self.managed_root, &self.manifest.uv_relpath)
    }

    pub fn wheelhouse_dir(&self) -> PathBuf {
        managed_wheelhouse_dir(&self.managed_root)
    }

    pub fn venv_dir(&self) -> PathBuf {
        venv_path(&self.managed_root)
    }

    pub fn venv_python(&self) -> PathBuf {
        venv_python_path(&self.managed_root)
    }
}

#[derive(Clone, Debug)]
pub enum InstallRuntime {
    Managed(ManagedRuntimeContext),
    DevFallback(DevFallbackContext),
}

#[derive(Clone, Debug)]
pub struct DevFallbackContext {
    pub root: PathBuf,
    pub system_python: PathBuf,
    pub venv_python: PathBuf,
    pub uv: UvBinary,
}

#[derive(Clone, Debug)]
pub struct DevFallbackPrerequisites {
    pub python: Option<PathBuf>,
    pub uv: Option<UvBinary>,
}

impl InstallRuntime {
    pub fn venv_dir(&self) -> PathBuf {
        match self {
            Self::Managed(runtime) => runtime.venv_dir(),
            Self::DevFallback(runtime) => runtime.root.clone(),
        }
    }

    pub fn venv_python(&self) -> PathBuf {
        match self {
            Self::Managed(runtime) => runtime.venv_python(),
            Self::DevFallback(runtime) => runtime.venv_python.clone(),
        }
    }

    pub fn wheelhouse_dir(&self) -> Option<PathBuf> {
        match self {
            Self::Managed(runtime) => Some(runtime.wheelhouse_dir()),
            Self::DevFallback(_) => None,
        }
    }
}

pub fn dev_fallback_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("runtime-dev").join("system-python")
}

pub fn dev_fallback_python_path(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join("Scripts").join("python.exe")
    } else {
        root.join("bin").join("python")
    }
}

pub fn dev_fallback_allowed() -> bool {
    cfg!(all(
        debug_assertions,
        any(target_os = "linux", target_os = "windows")
    ))
}

pub fn dev_fallback_platform_hint() -> &'static str {
    if dev_fallback_allowed() {
        "En Linux/Windows debug, EntropIA puede crear un venv local usando Python 3.11+ y uv del sistema para continuar sin payloads reales."
    } else if cfg!(debug_assertions) {
        if cfg!(target_os = "windows") {
            "En Windows dev el fallback online está habilitado cuando hay Python 3.11+ y uv disponible; si no aparece, instalá esos prerequisitos o hidratá un runtime-pack compatible."
        } else if cfg!(target_os = "macos") {
            "En macOS dev el fallback online no está habilitado: necesitás un runtime-pack release hidratado/compatible o una fuente bootstrap confiable."
        } else {
            "En esta plataforma dev el fallback online no está habilitado: necesitás un runtime-pack release hidratado/compatible o una fuente bootstrap confiable."
        }
    } else {
        "En release no hay fallback de desarrollo: necesitás un runtime-pack hidratado/compatible o una fuente bootstrap confiable."
    }
}

pub fn dev_fallback_available_reason() -> &'static str {
    if cfg!(target_os = "windows") {
        "Modo desarrollo Windows: podés instalar dependencias en un venv local con Python/uv del sistema mientras el runtime de release sigue pendiente."
    } else if cfg!(target_os = "linux") {
        "Modo desarrollo Linux: podés instalar dependencias en un venv local con Python/uv del sistema mientras el runtime de release sigue pendiente."
    } else {
        "Modo desarrollo: podés instalar dependencias en un venv local con Python/uv del sistema mientras el runtime de release sigue pendiente."
    }
}

fn install_runtime_unavailable_message(state: RuntimeState) -> String {
    format!(
        "El runtime de release no está listo ({state:?}) y no hay fallback de desarrollo utilizable. {}",
        dev_fallback_platform_hint()
    )
}

fn allow_online_installs() -> bool {
    dev_fallback_allowed()
}

pub fn load_install_runtime(
    app_handle: &tauri::AppHandle,
    app_data_dir: &Path,
) -> Result<InstallRuntime, String> {
    let runtime_manager = app_handle.state::<RuntimeManager>();
    if let Some(runtime) = load_managed_runtime_context(app_handle)? {
        if runtime.status.state == RuntimeState::Healthy {
            return Ok(InstallRuntime::Managed(runtime));
        }
    }

    let current_status = runtime_manager.status(app_handle)?;
    if current_status.state == RuntimeState::Fixture {
        if let Some(runtime) = load_dev_fallback_context(app_data_dir) {
            return Ok(InstallRuntime::DevFallback(runtime));
        }
    }

    let status = runtime_manager.ensure_ready_or_bootstrap_unlocked(app_handle)?;

    if status.state == RuntimeState::Healthy {
        if let Some(runtime) = load_managed_runtime_context(app_handle)? {
            return Ok(InstallRuntime::Managed(runtime));
        }
    }

    if let Some(runtime) = load_dev_fallback_context(app_data_dir) {
        return Ok(InstallRuntime::DevFallback(runtime));
    }

    Err(install_runtime_unavailable_message(status.state))
}

#[cfg(test)]
fn load_install_runtime_for_tests<F>(
    bundle_root: &Path,
    app_data_dir: &Path,
    on_progress: F,
) -> Result<InstallRuntime, String>
where
    F: FnMut(crate::runtime::status::RuntimeOperation),
{
    let manager = RuntimeManager::new();
    let status = manager.ensure_ready_or_bootstrap_for_tests(
        bundle_root,
        app_data_dir,
        crate::runtime::bootstrap::BootstrapRemoteCatalog::SourceUnavailable {
            source: None,
            reason: "Trusted remote bootstrap source is not configured in tests".to_string(),
        },
        on_progress,
    )?;

    if status.state == RuntimeState::Healthy {
        if let Some(runtime) = load_managed_runtime_context_for_tests(app_data_dir)? {
            return Ok(InstallRuntime::Managed(runtime));
        }
    }

    if let Some(runtime) = load_managed_runtime_context_for_tests(app_data_dir)? {
        if runtime.status.state == RuntimeState::Healthy {
            return Ok(InstallRuntime::Managed(runtime));
        }
    }

    if let Some(runtime) = load_dev_fallback_context(app_data_dir) {
        return Ok(InstallRuntime::DevFallback(runtime));
    }

    Err(format!(
        "El runtime de release no está listo ({:?}) y no hay fuente confiable/fallback utilizable.",
        status.state
    ))
}

fn load_dev_fallback_context(app_data_dir: &Path) -> Option<DevFallbackContext> {
    if !dev_fallback_allowed() {
        return None;
    }

    let prerequisites = inspect_dev_fallback_prerequisites(app_data_dir);
    let uv = prerequisites.uv.clone()?;
    let root = dev_fallback_root(app_data_dir);
    let venv_python = dev_fallback_python_path(&root);
    let system_python = prerequisites.python.clone()?;

    Some(DevFallbackContext {
        root,
        system_python,
        venv_python,
        uv,
    })
}

pub fn inspect_dev_fallback_prerequisites(app_data_dir: &Path) -> DevFallbackPrerequisites {
    let python = crate::python_discovery::discover_python_candidates()
        .iter()
        .find(|path| path.is_file() && python_supports_dev_fallback(path))
        .cloned();

    DevFallbackPrerequisites {
        python,
        uv: uv::UvBinary::detect_dev_fallback(app_data_dir),
    }
}

fn python_supports_dev_fallback(path: &Path) -> bool {
    crate::python_discovery::probe_python_module(
        path,
        "import sys; print('ok' if sys.version_info >= (3, 11) else 'no')",
    )
}

pub fn load_managed_runtime_context(
    app_handle: &tauri::AppHandle,
) -> Result<Option<ManagedRuntimeContext>, String> {
    let manager = app_handle.state::<RuntimeManager>();
    let app_data_dir = crate::path_utils::data_dir(&app_handle)
        .map_err(|error| format!("Failed to get app data dir: {error}"))?;
    let bundle_root = manager.hydrated_runtime_root(app_handle)?;

    let Some(managed_root) = bundle_root else {
        return Ok(None);
    };

    let manifest = RuntimeManifest::load_from_path(&managed_root.join("manifest.json"))?;
    let status = manager
        .inspect_hydrated_runtime_for_tests(&app_data_dir, &managed_root, &manifest)
        .ok_or_else(|| {
            format!(
                "No se pudo inspeccionar el runtime administrado {}",
                managed_root.display()
            )
        })?;

    Ok(Some(ManagedRuntimeContext {
        managed_root,
        manifest,
        status,
    }))
}

#[cfg(test)]
fn load_managed_runtime_context_for_tests(
    app_data_dir: &Path,
) -> Result<Option<ManagedRuntimeContext>, String> {
    let manager = RuntimeManager::new();
    let Some(managed_root) = manager.discover_hydrated_runtime_root_for_tests(app_data_dir) else {
        return Ok(None);
    };
    let manifest = RuntimeManifest::load_from_path(&managed_root.join("manifest.json"))?;
    let status = manager
        .inspect_hydrated_runtime_for_tests(app_data_dir, &managed_root, &manifest)
        .ok_or_else(|| "No se pudo inspeccionar el runtime hidratado".to_string())?;
    Ok(Some(ManagedRuntimeContext {
        managed_root,
        manifest,
        status,
    }))
}

// ---------------------------------------------------------------------------
// Venv creation
// ---------------------------------------------------------------------------

/// Create the managed venv using the hydrated runtime's uv + Python payload.
///
/// Returns the path to the venv's `python.exe`. If the venv already exists
/// (the python interpreter file is present) this is a no-op.
pub async fn create_venv(
    uv: &UvBinary,
    runtime: &ManagedRuntimeContext,
) -> Result<PathBuf, String> {
    let python_path = runtime.venv_python();

    // Already exists — nothing to do.
    if python_path.is_file() {
        ensure_windows_native_dll_sitecustomize(&python_path)?;
        return Ok(python_path);
    }

    let venv = runtime.venv_dir();
    let venv_str = venv.to_string_lossy().into_owned();
    let managed_python = runtime.managed_python();
    let managed_python_str = managed_python.to_string_lossy().into_owned();

    let mut cmd = uv.command();
    sanitize_install_subprocess_env(&mut cmd);
    cmd.args([
        "venv",
        &venv_str,
        "--python",
        &managed_python_str,
        "--offline",
    ]);
    run_and_stream(
        &mut cmd,
        "Python venv",
        |line| {
            eprintln!("[deps/install] [Python venv] {line}");
        },
        UV_VENV_TIMEOUT,
    )
    .await?;

    if !python_path.is_file() {
        return Err(
            "Error creando entorno virtual: Python del venv no encontrado después de uv venv"
                .to_string(),
        );
    }

    ensure_windows_native_dll_sitecustomize(&python_path)?;

    Ok(python_path)
}

pub async fn create_dev_fallback_venv(runtime: &DevFallbackContext) -> Result<PathBuf, String> {
    let python_path = runtime.venv_python.clone();

    if python_path.is_file() {
        ensure_windows_native_dll_sitecustomize(&python_path)?;
        return Ok(python_path);
    }

    tokio::fs::create_dir_all(&runtime.root)
        .await
        .map_err(|e| format!("Error creando directorio del fallback de desarrollo: {e}"))?;

    let root_str = runtime.root.to_string_lossy().into_owned();
    let system_python = runtime.system_python.to_string_lossy().into_owned();
    let mut cmd = runtime.uv.command();
    sanitize_install_subprocess_env(&mut cmd);
    cmd.args(["venv", &root_str, "--python", &system_python, "--seed"]);
    run_and_stream(
        &mut cmd,
        "Python dev fallback venv",
        |line| {
            eprintln!("[deps/install] [Python dev fallback venv] {line}");
        },
        UV_VENV_TIMEOUT,
    )
    .await?;

    if !python_path.is_file() {
        return Err(
            "Error creando entorno virtual de desarrollo: Python del venv no encontrado después de `uv venv`"
                .to_string(),
        );
    }

    ensure_windows_native_dll_sitecustomize(&python_path)?;

    Ok(python_path)
}

#[cfg(windows)]
fn ensure_windows_native_dll_sitecustomize(venv_python: &Path) -> Result<(), String> {
    let venv_dir = venv_python
        .parent()
        .and_then(|scripts_dir| scripts_dir.parent())
        .ok_or_else(|| {
            format!(
                "No se pudo resolver el directorio del venv desde {}",
                venv_python.display()
            )
        })?;

    let site_packages = venv_dir.join("Lib").join("site-packages");
    std::fs::create_dir_all(&site_packages).map_err(|error| {
        format!(
            "Error creando site-packages del venv administrado ({}): {error}",
            site_packages.display()
        )
    })?;

    let native_roots: Vec<String> = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let native_roots_json = serde_json::to_string(&native_roots).unwrap_or_else(|_| "[]".into());

    let content = format!(
        r#"# Auto-generated by EntropIA. Do not edit.
#
# Python 3.8+ no longer resolves extension-module dependencies from PATH.
# EntropIA ships native DLLs app-local, while wheels such as onnxruntime and
# paddlepaddle keep additional DLLs in package-specific folders. Register and
# retain those directories before any dependency probe or AI subprocess import.
import ctypes
import os

_entropia_dll_dir_handles = []
_entropia_preloaded_dlls = []

_site_packages = os.path.dirname(__file__)
_native_roots = {native_roots_json}
_candidates = list(_native_roots)
_candidates.extend([
    os.path.join(_site_packages, "onnxruntime", "capi"),
    os.path.join(_site_packages, "paddle", "libs"),
])

try:
    _candidates.extend(
        os.path.join(_site_packages, _name)
        for _name in os.listdir(_site_packages)
        if _name.endswith(".libs")
    )
except OSError:
    pass

for _path in _candidates:
    if os.path.isdir(_path):
        try:
            _entropia_dll_dir_handles.append(os.add_dll_directory(_path))
        except (AttributeError, OSError):
            pass

for _root in _native_roots:
    for _dll in [
        "vcruntime140.dll",
        "vcruntime140_1.dll",
        "msvcp140.dll",
        "msvcp140_1.dll",
        "msvcp140_2.dll",
        "msvcp140_atomic_wait.dll",
        "msvcp140_codecvt_ids.dll",
        "vcomp140.dll",
        "concrt140.dll",
        "vccorlib140.dll",
    ]:
        _dll_path = os.path.join(_root, _dll)
        if os.path.isfile(_dll_path):
            try:
                _entropia_preloaded_dlls.append(ctypes.WinDLL(_dll_path))
            except OSError:
                pass
"#
    );

    let sitecustomize = site_packages.join("sitecustomize.py");
    std::fs::write(&sitecustomize, content).map_err(|error| {
        format!(
            "Error escribiendo sitecustomize.py del venv administrado ({}): {error}",
            sitecustomize.display()
        )
    })?;

    Ok(())
}

#[cfg(not(windows))]
fn ensure_windows_native_dll_sitecustomize(_venv_python: &Path) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Persist venv paths to app_settings
// ---------------------------------------------------------------------------

/// Write all Python-path settings into `app_settings` so that every subsystem
/// (embeddings, OCR, transcription, NER) can find the managed interpreter.
pub fn persist_venv_paths(conn: &rusqlite::Connection, python_path: &Path) -> Result<(), String> {
    let path_str = python_path.to_string_lossy();

    let keys = [
        "deps_venv_python_path",
        "python.paddle_vl.path",
        "python.faster_whisper.path",
    ];

    for key in keys {
        crate::settings::set_setting(conn, key, &path_str)
            .map_err(|e| format!("Error guardando ruta Python en configuración ({key}): {e}"))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Install a single package
// ---------------------------------------------------------------------------

/// Install one dependency into the managed venv.
///
/// - Deps with `pip_spec`: `uv pip install <spec> --python <venv_python>`
/// - `Python` (no pip_spec, managed by uv): immediate `Ok(())`
///
/// Streams stderr line-by-line, calling `on_output(line)` for each line.
/// On non-zero exit returns `Err` with the last few stderr lines.
pub async fn install_package(
    uv: &UvBinary,
    dep: &DependencySpec,
    venv_python: &Path,
    wheelhouse_dir: Option<&Path>,
    reinstall: bool,
    on_output: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(), String> {
    if dep.id == DependencyId::Python {
        // Python itself is managed by `uv venv` — nothing to install.
        return Ok(());
    }

    // Best-effort free-disk precheck before the heavy ML wheel/model install: fail
    // fast with a clear Spanish message instead of dying mid-install (os error 112)
    // on a low-disk PC. If free space can't be read, proceed rather than block.
    const INSTALL_REQUIRED_FREE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
    let disk_probe_dir = venv_python.parent().unwrap_or(venv_python);
    if let Some(free) = crate::runtime::download::available_disk_space(disk_probe_dir) {
        if free < INSTALL_REQUIRED_FREE_BYTES {
            return Err(format!(
                "Espacio en disco insuficiente para instalar {}: se requieren ~{:.1} GB libres y hay {:.1} GB. Liberá espacio y volvé a intentar.",
                dep.display_name,
                INSTALL_REQUIRED_FREE_BYTES as f64 / 1_000_000_000.0,
                free as f64 / 1_000_000_000.0
            ));
        }
    }

    // Writability precheck: surface a read-only / AV-locked install dir up front
    // with a clear message instead of a cryptic uv failure mid-install.
    let install_dir = venv_python.parent().unwrap_or(venv_python);
    let write_probe = install_dir.join(".entropia-write-probe");
    if let Err(error) = std::fs::write(&write_probe, b"") {
        return Err(format!(
            "No se puede escribir en la carpeta de instalación {}: {error}. Revisá permisos o una posible exclusión de antivirus.",
            install_dir.display()
        ));
    }
    let _ = std::fs::remove_file(&write_probe);

    let (spec, extra_index_url) = if dep.id == DependencyId::PaddlePaddle {
        resolve_paddlepaddle_install_target(wheelhouse_dir)
    } else {
        let spec = managed_install_spec(dep, wheelhouse_dir)
            .ok_or_else(|| format!("Sin spec de instalación para {}", dep.display_name))?;
        (spec, None)
    };

    let python_str = venv_python.to_string_lossy().into_owned();
    let mut cmd = uv.command();
    sanitize_install_subprocess_env(&mut cmd);
    cmd.arg("pip").arg("install");
    if reinstall {
        // Retrying after a previous failure: overwrite a half-installed/corrupted
        // package instead of reusing it (a plain install would keep the corruption).
        cmd.arg("--reinstall");
    }
    // A spec may carry several space-separated packages: e.g. spaCy ships the
    // engine plus the language model, and the es_core_news_md 3.8 wheel no longer
    // declares spaCy as a dependency, so both must be installed together.
    for pkg in spec.split_whitespace() {
        cmd.arg(pkg);
    }
    cmd.args(["--python", &python_str]);
    let uses_online_indexes = extra_index_url.is_some() || wheelhouse_dir.is_none();

    if let Some(index_url) = extra_index_url {
        // PaddlePaddle GPU packages are hosted on their own index (not PyPI).
        // Use --extra-index-url so PyPI remains available for transitive deps.
        cmd.args(["--extra-index-url", &index_url]);
    } else if let Some(wheelhouse_dir) = wheelhouse_dir {
        let wheelhouse_str = wheelhouse_dir.to_string_lossy().into_owned();
        cmd.args(["--no-index", "--find-links", &wheelhouse_str]);
    } else if !allow_online_installs() {
        return Err(format!(
            "{} requiere wheelhouse administrado; el fallback online no está habilitado para este entorno. {}",
            dep.display_name,
            dev_fallback_platform_hint()
        ));
    }

    if requires_binary_only(uses_online_indexes) {
        cmd.args(["--only-binary", ":all:"]);
        cmd.env("UV_ONLY_BINARY", ":all:");
        on_output("stage: Resolviendo/descargando ruedas binarias (sin compilar desde fuente)");
    } else {
        on_output("stage: Instalando desde wheelhouse local/offline");
    }

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let install_result = run_and_stream(
        &mut cmd,
        dep.display_name,
        on_output,
        UV_PIP_INSTALL_TIMEOUT,
    )
    .await;

    if let Err(error) = install_result {
        let error = annotate_install_os_error(error);
        if requires_binary_only(uses_online_indexes) {
            return Err(format!(
                "{error}\nEntropIA instaló con --only-binary=:all: para evitar compilar paquetes Rust/PyO3 desde fuente. Si uv no encontró una rueda compatible, instalá una versión de Python/plataforma con wheels disponibles o usá el runtime/wheelhouse administrado."
            ));
        }
        return Err(error);
    }

    Ok(())
}

/// Map raw Windows OS error codes inside an install failure to actionable Spanish
/// hints. Leaves the message unchanged when no known code is present.
fn annotate_install_os_error(error: String) -> String {
    let hint = if error.contains("os error 5") {
        Some("Acceso denegado: agregá una exclusión de antivirus para la carpeta de datos de la app o cerrá programas que la estén bloqueando.")
    } else if error.contains("os error 32") {
        Some("Archivo en uso: cerrá trabajos en curso (OCR/transcripción) que estén usando el entorno y reintentá.")
    } else if error.contains("os error 112") {
        Some("Disco lleno: liberá espacio en el disco de la aplicación y volvé a intentar.")
    } else {
        None
    };
    match hint {
        Some(hint) => format!("{error}\n{hint}"),
        None => error,
    }
}

fn sanitize_install_subprocess_env(cmd: &mut Command) {
    for (key, value) in INSTALL_ENV_OVERRIDES {
        match value {
            Some(value) => {
                cmd.env(key, value);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
}

fn requires_binary_only(uses_online_indexes: bool) -> bool {
    uses_online_indexes
}

fn managed_install_spec(dep: &DependencySpec, wheelhouse_dir: Option<&Path>) -> Option<String> {
    match dep.id {
        DependencyId::Python => None,
        DependencyId::PaddlePaddle => {
            // Dynamic resolution is handled inside install_package; for probes
            // and status checks we still return the static CPU spec.
            dep.pip_spec.map(str::to_owned)
        }
        DependencyId::Spacy if wheelhouse_dir.is_some() => {
            // The es model wheel does not pull spaCy, so install both from the
            // wheelhouse (spaCy 3.8.x ships cp39–cp313 wheels). es_core_news_md
            // carries word vectors → better NER quality than the sm model.
            Some("spacy>=3.8.0,<3.9.0 es-core-news-md==3.8.0".to_string())
        }
        _ => dep.pip_spec.map(str::to_owned),
    }
}

async fn ensure_managed_prerequisites_installed(
    uv: &UvBinary,
    dep: &DependencySpec,
    venv_python: &Path,
    wheelhouse_dir: Option<&Path>,
) -> Result<(), String> {
    for prerequisite_id in dep.managed_prerequisites {
        let prerequisite = find_dep(prerequisite_id).ok_or_else(|| {
            format!(
                "Prerequisito desconocido para {}: {prerequisite_id:?}",
                dep.display_name
            )
        })?;

        let prerequisite_status = probe_one(prerequisite, venv_python).await;
        if matches!(prerequisite_status, DependencyStatus::Installed { .. }) {
            continue;
        }

        let display_name = prerequisite.display_name;
        install_package(
            uv,
            prerequisite,
            venv_python,
            wheelhouse_dir,
            false,
            move |line| {
                eprintln!("[deps/install] [{display_name}] {line}");
            },
        )
        .await?;

        let post_install_status = probe_one(prerequisite, venv_python).await;
        if !matches!(post_install_status, DependencyStatus::Installed { .. }) {
            return Err(format!(
                "No se pudo confirmar el prerequisito {} dentro del venv administrado",
                prerequisite.display_name
            ));
        }
    }

    Ok(())
}

async fn update_dependency_status(
    app: &tauri::AppHandle,
    state: &DepsState,
    id: &DependencyId,
    status: DependencyStatus,
) {
    {
        let mut map = state.0.lock().await;
        map.statuses.insert(id.clone(), status.clone());
    }

    emit_progress_best_effort(app, id.clone(), status);
}

fn emit_progress_best_effort(app: &tauri::AppHandle, id: DependencyId, status: DependencyStatus) {
    if let Err(error) = app.emit("deps://progress", DepsProgressPayload { id, status }) {
        eprintln!("[deps/install] Failed to emit dependency progress event: {error}");
    }
}

fn emit_complete_best_effort(app: &tauri::AppHandle, payload: DepsCompletePayload) {
    if let Err(error) = app.emit("deps://complete", payload) {
        eprintln!("[deps/install] Failed to emit dependency completion event: {error}");
    }
}

fn first_failed_prerequisite_message(
    dep: &DependencySpec,
    results: &[DepCheckResult],
) -> Option<String> {
    dep.managed_prerequisites.iter().find_map(|prerequisite_id| {
        let prerequisite_result = results.iter().find(|result| &result.id == prerequisite_id)?;
        if matches!(prerequisite_result.status, DependencyStatus::Installed { .. }) {
            return None;
        }

        let prerequisite_name = find_dep(prerequisite_id)
            .map(|spec| spec.display_name)
            .unwrap_or("un prerequisito");
        Some(format!(
            "{} bloqueado: {} no quedó instalado correctamente en esta corrida. Repará ese prerequisito antes de continuar.",
            dep.display_name, prerequisite_name
        ))
    })
}

fn managed_install_plan(dep: &'static DependencySpec) -> Vec<&'static DependencySpec> {
    let mut plan = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_managed_install_plan(dep, &mut seen, &mut plan);
    plan
}

fn collect_managed_install_plan(
    dep: &'static DependencySpec,
    seen: &mut std::collections::HashSet<DependencyId>,
    plan: &mut Vec<&'static DependencySpec>,
) {
    if !seen.insert(dep.id.clone()) {
        return;
    }

    for prerequisite_id in dep.managed_prerequisites {
        if let Some(prerequisite) = find_dep(prerequisite_id) {
            collect_managed_install_plan(prerequisite, seen, plan);
        }
    }

    plan.push(dep);
}

/// Helper: spawn `cmd`, stream stderr lines via `on_output`, return `Err` on
/// non-zero exit with the last few lines of stderr as the message.
async fn run_and_stream(
    cmd: &mut Command,
    display_name: &str,
    on_output: impl Fn(&str) + Send + Sync + 'static,
    max_duration: Duration,
) -> Result<(), String> {
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Error iniciando instalación de {display_name}: {e}"))?;

    let on_output = std::sync::Arc::new(on_output);
    let stdout_tail = child.stdout.take().map(|stdout| {
        let on_output = on_output.clone();
        tokio::spawn(drain_lines(stdout, "stdout", on_output))
    });
    let stderr_tail = child.stderr.take().map(|stderr| {
        let on_output = on_output.clone();
        tokio::spawn(drain_lines(stderr, "stderr", on_output))
    });

    let status = match timeout(max_duration, child.wait()).await {
        Ok(wait_result) => {
            wait_result.map_err(|e| format!("Error esperando proceso de {display_name}: {e}"))?
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stdout = join_tail(stdout_tail).await;
            let stderr = join_tail(stderr_tail).await;
            let tail = format_subprocess_tail(&stdout, &stderr);
            return Err(format!(
                "Timeout instalando {display_name}: el proceso excedió {}s y fue cancelado.{}",
                max_duration.as_secs(),
                tail
            ));
        }
    };

    let stdout = join_tail(stdout_tail).await;
    let stderr = join_tail(stderr_tail).await;

    if !status.success() {
        let tail = format_subprocess_tail(&stdout, &stderr);
        return Err(format!("Error instalando {display_name}: {tail}"));
    }

    Ok(())
}

async fn drain_lines<R>(
    stream: R,
    label: &'static str,
    on_output: std::sync::Arc<impl Fn(&str) + Send + Sync + 'static>,
) -> std::collections::VecDeque<String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut tail = std::collections::VecDeque::new();
    let mut reader = tokio::io::BufReader::new(stream).lines();
    let mut compacted_build_debug_lines = 0usize;
    while let Ok(Some(line)) = reader.next_line().await {
        if is_build_backend_debug_spam(&line) {
            compacted_build_debug_lines += 1;
            if compacted_build_debug_lines % BUILD_BACKEND_SPAM_REPORT_EVERY == 0 {
                let compacted = format!(
                    "{label}: [compactado] {compacted_build_debug_lines} líneas DEBUG del backend de build suprimidas"
                );
                on_output(&compacted);
                push_tail_line(&mut tail, compacted);
            }
            continue;
        }

        let line = format!("{label}: {line}");
        on_output(&line);
        push_tail_line(&mut tail, line);
    }

    if compacted_build_debug_lines > 0 {
        push_tail_line(
            &mut tail,
            format!(
                "{label}: [compactado] total: {compacted_build_debug_lines} líneas DEBUG del backend de build suprimidas"
            ),
        );
    }

    tail
}

fn push_tail_line(tail: &mut std::collections::VecDeque<String>, line: String) {
    if tail.len() >= SUBPROCESS_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
}

fn is_build_backend_debug_spam(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let is_debug = lower.contains(" debug ")
        || lower.starts_with("debug ")
        || lower.contains("=debug")
        || lower.contains(" debug:");
    if !is_debug {
        return false;
    }

    lower.contains("pep517:build_wheels")
        || lower.contains("build_pyo3_wheels")
        || lower.contains("build_single_pyo3_wheel")
        || lower.contains("goblin::pe")
        || lower.contains("maturin")
}

async fn join_tail(
    handle: Option<tokio::task::JoinHandle<std::collections::VecDeque<String>>>,
) -> std::collections::VecDeque<String> {
    match handle {
        Some(handle) => handle.await.unwrap_or_default(),
        None => std::collections::VecDeque::new(),
    }
}

fn format_subprocess_tail(
    stdout: &std::collections::VecDeque<String>,
    stderr: &std::collections::VecDeque<String>,
) -> String {
    let lines = stdout
        .iter()
        .chain(stderr.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.is_empty() {
        " (sin salida reciente de stdout/stderr)".to_string()
    } else {
        format!("\n{lines}")
    }
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

/// Emitted on `deps://progress` after each dep status change.
#[derive(Clone, Serialize, Deserialize)]
pub struct DepsProgressPayload {
    pub id: DependencyId,
    pub status: DependencyStatus,
}

/// Emitted on `deps://uv_progress` during uv binary download.
#[derive(Clone, Serialize, Deserialize)]
pub struct DepsUvProgressPayload {
    pub percent: u8,
    pub message: String,
}

/// Emitted on `deps://complete` when the full install run finishes.
#[derive(Clone, Serialize, Deserialize)]
pub struct DepsCompletePayload {
    pub results: Vec<DepCheckResult>,
    pub all_critical_installed: bool,
}

// ---------------------------------------------------------------------------
// Install all dependencies
// ---------------------------------------------------------------------------

/// Orchestrate a full dependency install run.
///
/// 1. Ensure the uv binary (detect → download if missing).
/// 2. Create the venv (idempotent).
/// 3. Persist venv paths in app_settings.
/// 4. Loop over `all_deps()` in registry order, skipping Python (handled by
///    uv venv). Install each, emit `deps://progress` events, continue on
///    failure.
/// 5. Emit `deps://complete`.
///
/// Always returns `Ok(())` — partial failures are reported via events.
pub async fn install_all(
    app: &tauri::AppHandle,
    state: &DepsState,
    db_path: &Path,
    app_data_dir: &Path,
) -> Result<(), String> {
    super::invalidate_probe_cache(state).await;
    let runtime = load_install_runtime(app, app_data_dir)?;

    // ── 1. Ensure uv ────────────────────────────────────────────────────────
    let uv = ensure_uv(app, app_data_dir, &runtime).await?;

    // ── 2. Create venv & update Python status ───────────────────────────────
    {
        let mut map = state.0.lock().await;
        map.statuses.insert(
            DependencyId::Python,
            DependencyStatus::Installing { percent: 0 },
        );
    }
    emit_progress_best_effort(
        app,
        DependencyId::Python,
        DependencyStatus::Installing { percent: 0 },
    );

    let venv_python = match ensure_install_runtime_venv(&runtime).await {
        Ok(p) => {
            let status = DependencyStatus::Installed {
                version: Some("3.11".to_string()),
            };
            {
                let mut map = state.0.lock().await;
                map.statuses.insert(DependencyId::Python, status.clone());
            }
            emit_progress_best_effort(app, DependencyId::Python, status);
            p
        }
        Err(e) => {
            let status = DependencyStatus::Failed { message: e.clone() };
            {
                let mut map = state.0.lock().await;
                map.statuses.insert(DependencyId::Python, status.clone());
            }
            emit_progress_best_effort(app, DependencyId::Python, status);
            return Err(e);
        }
    };

    // ── 3. Persist venv paths ────────────────────────────────────────────────
    {
        let conn = rusqlite::Connection::open(db_path)
            .map_err(|e| format!("Error abriendo base de datos para settings: {e}"))?;
        persist_venv_paths(&conn, &venv_python)
            .map_err(|e| format!("Error guardando rutas de venv: {e}"))?;
    }

    // ── 4. Install each package ──────────────────────────────────────────────
    let mut results: Vec<DepCheckResult> = Vec::new();

    // Add Python result.
    results.push(DepCheckResult {
        id: DependencyId::Python,
        status: DependencyStatus::Installed {
            version: Some("3.11".to_string()),
        },
        version: Some("3.11".to_string()),
    });

    for dep in all_deps_in_install_order() {
        if dep.id == DependencyId::Python {
            continue; // Already handled above.
        }
        if let Some(blocked_message) = first_failed_prerequisite_message(dep, &results) {
            let blocked_status = DependencyStatus::Failed {
                message: blocked_message,
            };
            {
                let mut map = state.0.lock().await;
                map.statuses.insert(dep.id.clone(), blocked_status.clone());
            }
            emit_progress_best_effort(app, dep.id.clone(), blocked_status.clone());
            results.push(DepCheckResult {
                id: dep.id.clone(),
                status: blocked_status,
                version: None,
            });
            continue;
        }

        // Mark as installing.
        let installing = DependencyStatus::Installing { percent: 0 };
        {
            let mut map = state.0.lock().await;
            map.statuses.insert(dep.id.clone(), installing.clone());
        }
        emit_progress_best_effort(app, dep.id.clone(), installing);

        // Clone handles for the closure (on_output captures dep.display_name).
        let display_name = dep.display_name;
        let install_result = install_package(
            &uv,
            dep,
            &venv_python,
            runtime.wheelhouse_dir().as_deref(),
            false,
            move |line| {
                eprintln!("[deps/install] [{display_name}] {line}");
            },
        )
        .await;

        let final_status = match install_result {
            Ok(()) => {
                let verified = probe_one(dep, &venv_python).await;
                if matches!(verified, DependencyStatus::Installed { .. }) {
                    verified
                } else {
                    DependencyStatus::Failed {
                        message: format!(
                            "No se pudo confirmar {} dentro del venv administrado después de instalarlo",
                            dep.display_name
                        ),
                    }
                }
            }
            Err(msg) => {
                eprintln!("[deps/install] failed {}: {msg}", dep.display_name);
                DependencyStatus::Failed { message: msg }
            }
        };

        {
            let mut map = state.0.lock().await;
            map.statuses.insert(dep.id.clone(), final_status.clone());
        }
        emit_progress_best_effort(app, dep.id.clone(), final_status.clone());

        results.push(DepCheckResult {
            id: dep.id.clone(),
            status: final_status,
            version: None,
        });
    }

    crate::python_discovery::invalidate_probe_cache();
    super::cache_current_statuses(state, Some(venv_python)).await;

    // ── 5. Emit complete ─────────────────────────────────────────────────────
    let all_critical_installed = results.iter().all(|r| {
        let dep = find_dep(&r.id);
        let critical = dep.map(|d| d.critical).unwrap_or(false);
        if critical {
            matches!(r.status, DependencyStatus::Installed { .. })
        } else {
            true
        }
    });

    emit_complete_best_effort(
        app,
        DepsCompletePayload {
            results,
            all_critical_installed,
        },
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Install one dependency
// ---------------------------------------------------------------------------

/// Install a single dependency by id.
///
/// - Rejects `DependencyId::Python` (managed by uv).
/// - Pre-flight: ensures uv + venv exist (returns `Err` if not).
/// - Emits `deps://progress` Installing → Installed/Failed.
/// - Re-probes the dep after install and returns the `DepCheckResult`.
pub async fn install_one(
    id: &DependencyId,
    app: &tauri::AppHandle,
    state: &DepsState,
    db_path: &Path,
    app_data_dir: &Path,
) -> Result<DepCheckResult, String> {
    super::invalidate_probe_cache(state).await;
    let runtime = load_install_runtime(app, app_data_dir)?;

    if *id == DependencyId::Python {
        return Err(
            "Python es gestionado por uv, no se puede instalar individualmente".to_string(),
        );
    }

    // Pre-flight: uv must already be present.
    let uv = ensure_uv(app, app_data_dir, &runtime).await?;

    // Ensure the managed venv exists before installing a single dependency.
    let existing_venv_python = runtime.venv_python();
    let venv_python = if existing_venv_python.is_file() {
        existing_venv_python
    } else {
        let status = DependencyStatus::Installing { percent: 0 };
        {
            let mut map = state.0.lock().await;
            map.statuses.insert(DependencyId::Python, status.clone());
        }
        emit_progress_best_effort(app, DependencyId::Python, status);

        let created = ensure_install_runtime_venv(&runtime).await?;

        {
            let conn = rusqlite::Connection::open(db_path)
                .map_err(|e| format!("Error abriendo base de datos para settings: {e}"))?;
            persist_venv_paths(&conn, &created)
                .map_err(|e| format!("Error guardando rutas de venv: {e}"))?;
        }

        let status = DependencyStatus::Installed {
            version: Some("3.11".to_string()),
        };
        {
            let mut map = state.0.lock().await;
            map.statuses.insert(DependencyId::Python, status.clone());
        }
        emit_progress_best_effort(app, DependencyId::Python, status);

        created
    };

    let dep = find_dep(id).ok_or_else(|| format!("Dependencia desconocida: {id:?}"))?;

    let install_plan = managed_install_plan(dep);

    // Force --reinstall when retrying a dep that previously failed, so a corrupted
    // half-install is overwritten cleanly instead of reused. Read the status before
    // we overwrite it with Installing below.
    let reinstall_target = matches!(
        state.0.lock().await.statuses.get(id),
        Some(DependencyStatus::Failed { .. })
    );

    // Emit Installing.
    let installing = DependencyStatus::Installing { percent: 0 };
    update_dependency_status(app, state, id, installing).await;

    for planned_dep in &install_plan[..install_plan.len().saturating_sub(1)] {
        ensure_managed_prerequisites_installed(
            &uv,
            planned_dep,
            &venv_python,
            runtime.wheelhouse_dir().as_deref(),
        )
        .await?;

        let prerequisite_status = probe_one(planned_dep, &venv_python).await;
        if matches!(prerequisite_status, DependencyStatus::Installed { .. }) {
            update_dependency_status(app, state, &planned_dep.id, prerequisite_status).await;
            continue;
        }

        update_dependency_status(
            app,
            state,
            &planned_dep.id,
            DependencyStatus::Installing { percent: 0 },
        )
        .await;

        let display_name = planned_dep.display_name;
        install_package(
            &uv,
            planned_dep,
            &venv_python,
            runtime.wheelhouse_dir().as_deref(),
            false,
            move |line| {
                eprintln!("[deps/install] [{display_name}] {line}");
            },
        )
        .await?;

        let verified = probe_one(planned_dep, &venv_python).await;
        if !matches!(verified, DependencyStatus::Installed { .. }) {
            update_dependency_status(
                app,
                state,
                &planned_dep.id,
                DependencyStatus::Failed {
                    message: format!(
                        "No se pudo confirmar {} dentro del venv administrado después de instalarlo",
                        planned_dep.display_name
                    ),
                },
            )
            .await;
            return Err(format!(
                "No se pudo confirmar {} dentro del venv administrado después de instalarlo",
                planned_dep.display_name
            ));
        }

        update_dependency_status(app, state, &planned_dep.id, verified).await;
    }

    let display_name = dep.display_name;
    let install_result = install_package(
        &uv,
        dep,
        &venv_python,
        runtime.wheelhouse_dir().as_deref(),
        reinstall_target,
        move |line| {
            eprintln!("[deps/install] [{display_name}] {line}");
        },
    )
    .await;

    if let Err(ref msg) = install_result {
        let status = DependencyStatus::Failed {
            message: msg.clone(),
        };
        update_dependency_status(app, state, id, status).await;
        return Err(msg.clone());
    }

    // Re-probe to get accurate installed status.
    // Read python path from settings if venv path has been persisted; fall
    // back to the path we already know.
    let probe_settings = rusqlite::Connection::open(db_path)
        .ok()
        .map(|conn| crate::deps::checks::load_probe_python_settings(&conn))
        .unwrap_or_default();
    let probe_python = crate::deps::checks::resolve_probe_python_with_runtime(
        probe_settings,
        ProbePythonMode::DependencyManager,
        Some(runtime.venv_python().as_path()),
        runtime_status_for_probe(&runtime),
    )
    .unwrap_or(venv_python);

    let probed_status = probe_one(dep, &probe_python).await;

    if !matches!(probed_status, DependencyStatus::Installed { .. }) {
        let message = format!(
            "No se pudo confirmar {} dentro del venv administrado después de instalarlo",
            dep.display_name
        );
        let status = DependencyStatus::Failed {
            message: message.clone(),
        };
        update_dependency_status(app, state, id, status).await;
        return Err(message);
    }

    update_dependency_status(app, state, id, probed_status.clone()).await;

    let version = match &probed_status {
        DependencyStatus::Installed { version } => version.clone(),
        _ => None,
    };

    crate::python_discovery::invalidate_probe_cache();
    super::cache_current_statuses(state, Some(probe_python.clone())).await;

    Ok(DepCheckResult {
        id: id.clone(),
        status: probed_status,
        version,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Ensure a valid uv binary is available: detect it, or download it.
/// Emits `deps://uv_progress` events during download.
async fn ensure_uv(
    app: &tauri::AppHandle,
    app_data_dir: &Path,
    runtime: &InstallRuntime,
) -> Result<UvBinary, String> {
    match runtime {
        InstallRuntime::Managed(runtime) => {
            if let Some(uv) = uv::UvBinary::detect_with_runtime(
                Some(app),
                app_data_dir,
                Some(runtime.managed_uv().as_path()),
                Some(&runtime.status),
            ) {
                return Ok(uv);
            }
        }
        InstallRuntime::DevFallback(runtime) => {
            return Ok(runtime.uv.clone());
        }
    }

    let app_clone = app.clone();
    uv::download(app_data_dir, move |percent, message| {
        crate::app_logs::info(
            &app_clone,
            "deps/uv",
            format!("Descarga uv {percent}% · {message}"),
        );
        let _ = app_clone.emit(
            "deps://uv_progress",
            DepsUvProgressPayload {
                percent,
                message: message.to_string(),
            },
        );
    })
    .await
}

async fn ensure_install_runtime_venv(runtime: &InstallRuntime) -> Result<PathBuf, String> {
    match runtime {
        InstallRuntime::Managed(runtime) => {
            let uv = UvBinary::detect_with_runtime(
                None,
                &runtime.managed_root,
                Some(runtime.managed_uv().as_path()),
                Some(&runtime.status),
            )
            .ok_or_else(|| "uv no está disponible para el runtime administrado".to_string())?;
            create_venv(&uv, runtime).await
        }
        InstallRuntime::DevFallback(runtime) => create_dev_fallback_venv(runtime).await,
    }
}

fn runtime_status_for_probe(runtime: &InstallRuntime) -> Option<&RuntimeStatus> {
    match runtime {
        InstallRuntime::Managed(runtime) => Some(&runtime.status),
        InstallRuntime::DevFallback(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::registry::find_dep;
    use crate::runtime::manifest::{ManifestEntry, RuntimeManifest};
    use crate::runtime::status::RuntimeState;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn write_file(root: &Path, relpath: &str, bytes: &[u8]) -> String {
        let path = root.join(relpath);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, bytes).expect("write file");
        format!("{:x}", Sha256::digest(bytes))
    }

    #[tokio::test]
    async fn run_and_stream_drains_stdout_without_hanging() {
        let mut cmd = if cfg!(windows) {
            let mut cmd = Command::new("cmd.exe");
            cmd.args(["/C", "for /L %i in (1,1,2000) do @echo stdout-line-%i"]);
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "for i in $(seq 1 2000); do echo stdout-line-$i; done"]);
            cmd
        };

        run_and_stream(
            &mut cmd,
            "stdout-drain-test",
            |_| {},
            Duration::from_secs(5),
        )
        .await
        .expect("stdout-heavy subprocess should complete");
    }

    #[tokio::test]
    async fn run_and_stream_times_out_and_kills_child() {
        let mut cmd = if cfg!(windows) {
            let mut cmd =
                Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
            cmd.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 3"]);
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "sleep 3"]);
            cmd
        };

        let error = run_and_stream(&mut cmd, "timeout-test", |_| {}, Duration::from_millis(100))
            .await
            .expect_err("slow subprocess should time out");

        assert!(error.contains("Timeout instalando timeout-test"));
    }

    #[test]
    fn test_online_installs_require_binary_wheels_only() {
        assert!(
            requires_binary_only(true),
            "online fallback installs must fail fast instead of compiling sdists"
        );

        assert!(
            !requires_binary_only(false),
            "offline wheelhouse installs should keep using local wheels without online binary flags"
        );
    }

    #[test]
    fn test_install_env_overrides_sanitize_rust_debug_flags() {
        assert_eq!(
            INSTALL_ENV_OVERRIDES
                .iter()
                .find(|(key, _)| *key == "RUST_LOG"),
            Some(&("RUST_LOG", Some("warn"))),
            "child uv/pip processes must not inherit RUST_LOG=debug from Tauri dev"
        );
        assert_eq!(
            INSTALL_ENV_OVERRIDES
                .iter()
                .find(|(key, _)| *key == "RUST_BACKTRACE"),
            Some(&("RUST_BACKTRACE", None)),
            "build backends should not inherit EntropIA's backtrace debug setting"
        );
    }

    #[test]
    fn test_build_backend_debug_spam_is_filtered() {
        let goblin_line = "pep517:build_wheels:build_pyo3_wheels:build_single_pyo3_wheel: goblin::pe DEBUG parsing import table";
        let maturin_line = "DEBUG maturin::build_context preparing pyo3 wheel";
        let useful_line = "Resolved 24 packages in 1.2s";

        assert!(is_build_backend_debug_spam(goblin_line));
        assert!(is_build_backend_debug_spam(maturin_line));
        assert!(!is_build_backend_debug_spam(useful_line));
    }

    #[test]
    fn std_command_output_with_timeout_kills_slow_probe() {
        let cmd = if cfg!(windows) {
            let mut cmd = std::process::Command::new(
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            );
            cmd.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 3"]);
            cmd
        } else {
            let mut cmd = std::process::Command::new("sh");
            cmd.args(["-c", "sleep 3"]);
            cmd
        };

        let started_at = Instant::now();
        let error =
            std_command_output_with_timeout(cmd, Duration::from_millis(100), "slow install probe")
                .expect_err("slow probe should time out");

        assert!(error.contains("slow install probe timed out"));
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "timeout helper should not wait for the child to finish naturally"
        );
    }

    #[test]
    fn test_venv_path_lives_under_managed_runtime_root() {
        let managed_runtime_root = Path::new("/some/app/data/runtime/2026.05.0");
        let venv = venv_path(managed_runtime_root);
        assert!(
            venv.to_string_lossy().contains("entropia-env"),
            "venv path should contain 'entropia-env'"
        );
        assert!(
            venv.starts_with(managed_runtime_root),
            "venv path should live inside the managed runtime root"
        );
    }

    #[test]
    fn test_venv_python_path_matches_platform_layout() {
        let managed_runtime_root = PathBuf::from("/some/app/data/runtime/2026.05.0");
        let python = venv_python_path(&managed_runtime_root);

        if cfg!(windows) {
            assert!(
                python.to_string_lossy().ends_with("python.exe"),
                "venv python path should end with 'python.exe'"
            );
            assert!(
                python.to_string_lossy().contains("Scripts"),
                "venv python path should go through Scripts/"
            );
        } else {
            assert!(
                python.to_string_lossy().ends_with("bin/python"),
                "venv python path should end with 'bin/python' on unix"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_native_dll_sitecustomize_registers_app_and_wheel_dirs() {
        let dir = tempdir().expect("tempdir");
        let python_path = dir
            .path()
            .join("venv")
            .join("entropia-env")
            .join("Scripts")
            .join("python.exe");
        std::fs::create_dir_all(python_path.parent().expect("scripts dir"))
            .expect("create scripts dir");
        std::fs::write(&python_path, b"fake-python").expect("write fake python");

        ensure_windows_native_dll_sitecustomize(&python_path).expect("write sitecustomize");

        let sitecustomize = dir
            .path()
            .join("venv")
            .join("entropia-env")
            .join("Lib")
            .join("site-packages")
            .join("sitecustomize.py");
        let content = std::fs::read_to_string(sitecustomize).expect("read sitecustomize");

        assert!(content.contains("os.add_dll_directory"));
        assert!(content.contains("_entropia_dll_dir_handles.append"));
        assert!(content.contains("onnxruntime"));
        assert!(content.contains("paddle"));
        assert!(content.contains("msvcp140_1.dll"));
        assert!(content.contains("vcomp140.dll"));
    }

    #[test]
    fn test_persist_venv_paths_writes_all_keys() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory DB");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("create table");

        let python_path = Path::new("/fake/venv/Scripts/python.exe");
        persist_venv_paths(&conn, python_path).expect("persist should succeed");

        let keys = [
            "deps_venv_python_path",
            "python.paddle_vl.path",
            "python.faster_whisper.path",
        ];
        for key in keys {
            let value: String = conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    rusqlite::params![key],
                    |row| row.get(0),
                )
                .expect(&format!("key '{key}' should be present"));
            assert_eq!(
                value,
                python_path.to_string_lossy().as_ref(),
                "key '{key}' should store the python path"
            );
        }
    }

    #[test]
    fn test_managed_install_plan_keeps_paddlepaddle_before_paddleocr() {
        let paddleocr = find_dep(&DependencyId::PaddleOcr).expect("PaddleOcr present");
        let plan = managed_install_plan(paddleocr)
            .into_iter()
            .map(|dep| dep.id.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            plan,
            vec![DependencyId::PaddlePaddle, DependencyId::PaddleOcr]
        );
    }

    #[test]
    fn test_managed_install_spec_preserves_regular_pip_specs() {
        let whisper = find_dep(&DependencyId::FasterWhisper).expect("faster-whisper dep present");

        assert_eq!(
            managed_install_spec(whisper, None),
            whisper.pip_spec.map(str::to_owned)
        );
    }

    #[test]
    fn test_load_managed_runtime_context_reads_hydrated_runtime_from_app_data() {
        let app_data_dir = tempdir().expect("app data dir");
        let managed_root = app_data_dir.path().join("runtime").join("2026.05.0");
        let python_relpath = if cfg!(windows) {
            "python/python.exe"
        } else {
            "python/bin/python3"
        };
        let uv_relpath = if cfg!(windows) {
            "uv/uv.exe"
        } else {
            "uv/bin/uv"
        };
        let venv_python_relpath = if cfg!(windows) {
            "venv/entropia-env/Scripts/python.exe"
        } else {
            "venv/entropia-env/bin/python"
        };
        let python_sha = write_file(&managed_root, python_relpath, b"python");
        let uv_sha = write_file(&managed_root, uv_relpath, b"uv");
        write_file(&managed_root, venv_python_relpath, b"venv-python");
        std::fs::write(
            managed_root.join("manifest.json"),
            serde_json::to_vec_pretty(&RuntimeManifest {
                pack_version: "2026.05.0".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                platform: crate::runtime::paths::current_runtime_platform(),
                payload_profile: "release".to_string(),
                release_injection_required: false,
                external_artifacts_required: vec![],
                python_relpath: python_relpath.to_string(),
                uv_relpath: uv_relpath.to_string(),
                python_files: vec![ManifestEntry {
                    path: python_relpath.to_string(),
                    sha256: python_sha,
                    size: 6,
                    executable: !cfg!(windows),
                }],
                uv_files: vec![ManifestEntry {
                    path: uv_relpath.to_string(),
                    sha256: uv_sha,
                    size: 2,
                    executable: true,
                }],
                script_files: vec![],
                wheelhouse: vec![],
                caches: vec![],
                native_assets: vec![],
            })
            .expect("serialize manifest"),
        )
        .expect("write manifest");

        let context = load_managed_runtime_context_for_tests(app_data_dir.path())
            .expect("context resolution should succeed")
            .expect("runtime context should exist");

        assert_eq!(context.status.state, RuntimeState::Healthy);
        assert_eq!(context.managed_root, managed_root);
        assert_eq!(
            context.venv_python(),
            crate::runtime::managed_venv_python_path(&managed_root)
        );
    }

    #[test]
    fn test_dev_fallback_paths_live_under_runtime_dev_root() {
        let app_data = Path::new("/tmp/entropia");
        let root = dev_fallback_root(app_data);
        let python = dev_fallback_python_path(&root);

        assert!(root.ends_with("runtime-dev/system-python"));
        if cfg!(windows) {
            assert!(python.ends_with("Scripts/python.exe"));
        } else {
            assert!(python.ends_with("bin/python"));
        }
    }

    #[test]
    fn test_install_runtime_dev_fallback_reports_no_wheelhouse() {
        let runtime = InstallRuntime::DevFallback(DevFallbackContext {
            root: PathBuf::from("/tmp/runtime-dev/system-python"),
            system_python: PathBuf::from("/usr/bin/python3"),
            venv_python: PathBuf::from("/tmp/runtime-dev/system-python/bin/python"),
            uv: UvBinary {
                path: PathBuf::from("/usr/bin/uv"),
                version: "0.10.3".to_string(),
            },
        });

        assert_eq!(runtime.wheelhouse_dir(), None);
        assert_eq!(
            runtime.venv_python(),
            PathBuf::from("/tmp/runtime-dev/system-python/bin/python")
        );
    }

    #[test]
    fn test_python_supports_dev_fallback_rejects_current_test_binary() {
        let current_exe = std::env::current_exe().expect("current exe");
        assert!(!python_supports_dev_fallback(&current_exe));
    }

    #[test]
    fn test_load_install_runtime_for_tests_bootstraps_release_bundle_before_installing() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
        let python_relpath = if cfg!(windows) {
            "python/python.exe"
        } else {
            "python/bin/python3"
        };
        let uv_relpath = if cfg!(windows) {
            "uv/uv.exe"
        } else {
            "uv/bin/uv"
        };
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        std::fs::write(
            bundle_dir.path().join("manifest.json"),
            serde_json::to_vec_pretty(&RuntimeManifest {
                pack_version: "2026.05.0".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                platform: crate::runtime::paths::current_runtime_platform(),
                payload_profile: "release".to_string(),
                release_injection_required: false,
                external_artifacts_required: vec![],
                python_relpath: python_relpath.to_string(),
                uv_relpath: uv_relpath.to_string(),
                python_files: vec![ManifestEntry {
                    path: python_relpath.to_string(),
                    sha256: python_sha,
                    size: 6,
                    executable: !cfg!(windows),
                }],
                uv_files: vec![ManifestEntry {
                    path: uv_relpath.to_string(),
                    sha256: uv_sha,
                    size: 2,
                    executable: true,
                }],
                script_files: vec![],
                wheelhouse: vec![],
                caches: vec![],
                native_assets: vec![],
            })
            .expect("serialize manifest"),
        )
        .expect("write manifest");

        let runtime =
            load_install_runtime_for_tests(bundle_dir.path(), app_data_dir.path(), |_| {})
                .expect("bootstrap should make managed runtime available");

        assert!(matches!(runtime, InstallRuntime::Managed(_)));
    }

    #[test]
    fn test_load_install_runtime_for_tests_reports_honest_bootstrap_blocker() {
        let bundle_dir = tempdir().expect("bundle dir");
        let app_data_dir = tempdir().expect("app data dir");
        let python_relpath = if cfg!(windows) {
            "python/python.exe"
        } else {
            "python/bin/python3"
        };
        let uv_relpath = if cfg!(windows) {
            "uv/uv.exe"
        } else {
            "uv/bin/uv"
        };
        let python_sha = write_file(bundle_dir.path(), python_relpath, b"python");
        let uv_sha = write_file(bundle_dir.path(), uv_relpath, b"uv");
        std::fs::write(
            bundle_dir.path().join("manifest.json"),
            serde_json::to_vec_pretty(&RuntimeManifest {
                pack_version: "2026.05.0".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                platform: crate::runtime::paths::current_runtime_platform(),
                payload_profile: "fixture".to_string(),
                release_injection_required: true,
                external_artifacts_required: vec!["relocatable-python".to_string()],
                python_relpath: python_relpath.to_string(),
                uv_relpath: uv_relpath.to_string(),
                python_files: vec![ManifestEntry {
                    path: python_relpath.to_string(),
                    sha256: python_sha,
                    size: 6,
                    executable: !cfg!(windows),
                }],
                uv_files: vec![ManifestEntry {
                    path: uv_relpath.to_string(),
                    sha256: uv_sha,
                    size: 2,
                    executable: true,
                }],
                script_files: vec![],
                wheelhouse: vec![],
                caches: vec![],
                native_assets: vec![],
            })
            .expect("serialize manifest"),
        )
        .expect("write manifest");

        let result = load_install_runtime_for_tests(bundle_dir.path(), app_data_dir.path(), |_| {});

        if dev_fallback_allowed() {
            assert!(
                matches!(result, Ok(InstallRuntime::DevFallback(_)) | Err(_)),
                "fixture runtime in Linux dev should either use the honest dev fallback or report the blocker"
            );
        } else {
            let error =
                result.expect_err("fixture runtime without source should block installs honestly");
            assert!(error.contains("fuente confiable") || error.contains("source"));
        }
    }

    // -----------------------------------------------------------------------
    // PaddlePaddle GPU automatic selection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_paddlepaddle_cuda_index_maps_cuda_11_to_cu118() {
        assert_eq!(
            paddlepaddle_cuda_index(Some("11.8")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu118/"
        );
        assert_eq!(
            paddlepaddle_cuda_index(Some("11.2")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu118/"
        );
    }

    #[test]
    fn test_paddlepaddle_cuda_index_maps_cuda_12_to_cu126() {
        assert_eq!(
            paddlepaddle_cuda_index(Some("12.0")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu126/"
        );
        assert_eq!(
            paddlepaddle_cuda_index(Some("12.6")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu126/"
        );
        assert_eq!(
            paddlepaddle_cuda_index(Some("12.8")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu126/"
        );
    }

    #[test]
    fn test_paddlepaddle_cuda_index_maps_cuda_129_to_cu129() {
        assert_eq!(
            paddlepaddle_cuda_index(Some("12.9")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu129/"
        );
        assert_eq!(
            paddlepaddle_cuda_index(Some("12.12")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu129/"
        );
    }

    #[test]
    fn test_paddlepaddle_cuda_index_maps_cuda_13_to_cu130() {
        assert_eq!(
            paddlepaddle_cuda_index(Some("13.0")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu130/"
        );
    }

    #[test]
    fn test_paddlepaddle_cuda_index_defaults_to_cu126_when_unknown() {
        assert_eq!(
            paddlepaddle_cuda_index(None),
            "https://www.paddlepaddle.org.cn/packages/stable/cu126/"
        );
        assert_eq!(
            paddlepaddle_cuda_index(Some("garbage")),
            "https://www.paddlepaddle.org.cn/packages/stable/cu126/"
        );
    }

    #[test]
    fn test_find_wheel_in_dir_matches_prefix_and_extension() {
        let dir = tempdir().expect("temp dir");
        let wheel = dir
            .path()
            .join("paddlepaddle_gpu-3.2.1-cp311-cp311-linux_x86_64.whl");
        std::fs::write(&wheel, b"fake wheel").expect("write wheel");

        let found = find_wheel_in_dir(dir.path(), "paddlepaddle_gpu");
        assert_eq!(found, Some(wheel));

        let not_found = find_wheel_in_dir(dir.path(), "paddlepaddle");
        // Should NOT match paddlepaddle_gpu because prefix is different
        assert_eq!(not_found, None);
    }

    #[test]
    fn test_find_wheel_in_dir_returns_none_for_empty_dir() {
        let dir = tempdir().expect("temp dir");
        assert_eq!(find_wheel_in_dir(dir.path(), "paddlepaddle_gpu"), None);
    }

    #[test]
    fn test_paddlepaddle_install_target_prefers_gpu_wheel_in_wheelhouse() {
        let wheelhouse = tempdir().expect("wheelhouse");
        let gpu_wheel = wheelhouse
            .path()
            .join("paddlepaddle_gpu-3.2.1-cp311-cp311-manylinux1_x86_64.whl");
        std::fs::write(&gpu_wheel, b"gpu wheel").expect("write gpu wheel");

        let (spec, index) = resolve_paddlepaddle_install_target(Some(wheelhouse.path()));

        assert_eq!(spec, gpu_wheel.to_string_lossy());
        assert_eq!(index, None, "wheelhouse install should not use extra index");
    }

    #[test]
    fn test_paddlepaddle_install_target_falls_back_to_cpu_wheel_in_wheelhouse() {
        let wheelhouse = tempdir().expect("wheelhouse");
        let cpu_wheel = wheelhouse
            .path()
            .join("paddlepaddle-3.2.1-cp311-cp311-manylinux1_x86_64.whl");
        std::fs::write(&cpu_wheel, b"cpu wheel").expect("write cpu wheel");

        let (spec, index) = resolve_paddlepaddle_install_target(Some(wheelhouse.path()));

        assert_eq!(spec, cpu_wheel.to_string_lossy());
        assert_eq!(index, None);
    }

    #[test]
    fn test_paddlepaddle_install_target_without_wheelhouse_returns_pip_spec() {
        // This test does NOT assert GPU vs CPU because it depends on whether
        // nvidia-smi is present in the test environment. We only assert that
        // the returned spec follows the expected patterns.
        let (spec, index) = resolve_paddlepaddle_install_target(None);

        assert!(
            spec.starts_with("paddlepaddle") || spec.starts_with("paddlepaddle-gpu"),
            "spec should reference a paddlepaddle package, got: {spec}"
        );
        assert!(
            spec.contains(">=3.2.1"),
            "spec should require >=3.2.1, got: {spec}"
        );
        assert!(
            spec.contains("<3.3.0"),
            "spec should cap at <3.3.0, got: {spec}"
        );

        // If GPU is detected in this environment, an index URL must be present.
        // If no GPU, index should be None.
        if spec.starts_with("paddlepaddle-gpu") {
            assert!(
                index.is_some(),
                "GPU spec must come with an extra index URL"
            );
            let idx = index.unwrap();
            assert!(
                idx.starts_with("https://www.paddlepaddle.org.cn/packages/stable/cu"),
                "index should point to PaddlePaddle CUDA index, got: {idx}"
            );
        }
    }
}
