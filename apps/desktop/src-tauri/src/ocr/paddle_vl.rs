//! PaddleOCR-VL engine — layout-aware OCR via Python subprocess.
//!
//! Calls the paddle_vl.py script which runs PaddleOCR-VL to perform
//! both layout detection and OCR in a single pass. Returns structured
//! results with text, blocks, and regions.
//!
//! Fallback chain: PaddleVL → lightweight PaddleOCR (if PaddleVL fails or times out)

use super::paddle_vl_types::PaddleVlOutput;
use crate::path_utils::normalize_windows_path;
use crate::runtime::{
    managed_hf_cache_dir, managed_paddlex_cache_dir, managed_script_path, managed_venv_python_path,
    RuntimeManager,
};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::Manager;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const GPU_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Configuration for the PaddleOCR-VL engine.
#[derive(Clone)]
pub struct PaddleVlConfig {
    /// Path to the Python interpreter with paddleocr installed.
    pub python_path: PathBuf,
    /// Path to the paddle_vl.py script.
    pub script_path: PathBuf,
    /// HuggingFace cache directory, if available.
    pub hf_cache_dir: Option<PathBuf>,
    /// PaddleX cache directory, if available.
    pub paddlex_cache_dir: Option<PathBuf>,
    /// Whether the subprocess must run without network access.
    ///
    /// A configured cache directory does not necessarily mean the cache is
    /// hydrated. Dev/fallback app-owned caches stay online so PaddleX can
    /// download missing official models into EntropIA-owned storage.
    pub offline_mode: bool,
    /// Preferred compute device: "gpu" or "cpu".
    /// The Python subprocess will validate GPU support and fall back to CPU
    /// if the paddlepaddle-gpu stack is not installed or GPU init fails.
    pub device: String,
}

/// The PaddleOCR-VL engine — spawns Python as a child process.
///
/// Each call spawns a fresh Python process. No persistent state.
#[derive(Clone)]
pub struct PaddleVlEngine {
    config: PaddleVlConfig,
}

impl PaddleVlEngine {
    /// Validate configuration and create the engine.
    ///
    /// NOTE: Python interpreter was already validated by `which_python_for_paddle_vl()`
    /// which ran `from paddleocr import PaddleOCRVL; print('ok')` successfully.
    /// Redundant verification (e.g., `python --version`) is skipped — the
    /// discovery module already proved the interpreter works.
    pub fn init(config: PaddleVlConfig) -> Result<Self, String> {
        // Verify script exists
        if !config.script_path.exists() {
            return Err(format!(
                "PaddleVL script not found: {}",
                config.script_path.display()
            ));
        }

        if std::env::var("ENTROPIA_VERBOSE_STARTUP")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            eprintln!(
                "[paddle_vl] Engine configured: python={}, script={}",
                config.python_path.display(),
                config.script_path.display(),
            );
        }

        Ok(Self { config })
    }

    /// Maximum time (in seconds) to wait for PaddleVL subprocess to complete.
    ///
    /// PaddleOCR-VL on CPU has very different timing depending on state:
    ///   - First-ever run: model downloads (~150 MB) + cold import + first inference
    ///     can take 5-15 minutes on slow connections / older CPUs.
    ///   - Models cached, cold Python: ~30-60s for import + pipeline init + first inference
    ///   - Subsequent runs (same process): would be ~5-15s, but we spawn fresh each time.
    ///
    /// We give 15 minutes (900s) of headroom. Progress is logged every 30s so the
    /// user knows the subprocess is still alive.
    const PADDLE_VL_TIMEOUT_SECS: u64 = 900;

    /// Interval (in seconds) for logging progress updates while waiting for the subprocess.
    /// At 30s intervals, a 900s timeout produces ~30 progress logs.
    const PROGRESS_LOG_INTERVAL_SECS: u64 = 30;

    /// Run PaddleOCR-VL on an image file.
    ///
    /// Spawns the Python subprocess, passes the image path,
    /// and parses the sentinel-wrapped JSON output.
    /// On timeout (15 minutes), kills the subprocess and returns an error.
    ///
    /// Sets CPU optimization env vars on the subprocess to maximize throughput:
    ///   - OMP/MKL/OPENBLAS_NUM_THREADS: parallelize matrix ops across cores
    ///   - FLAGS_use_mkldnn=1: enable Paddle's oneDNN acceleration
    ///   - HF_HUB_DISABLE_PROGRESS_BARS=1: silence noisy HF download progress
    pub fn detect(&self, image_path: &str) -> Result<PaddleVlOutput, String> {
        eprintln!(
            "[paddle_vl] Spawning PaddleOCR-VL for: {} (device={})",
            image_path, self.config.device
        );

        // Determine optimal thread count: all logical cores capped at 8.
        // Going beyond 8 typically hurts due to memory bandwidth + scheduler overhead.
        let cpu_threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4)
            .to_string();

        let mut cmd = Command::new(&self.config.python_path);
        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.arg(&self.config.script_path)
            .arg(image_path)
            .arg("--device")
            .arg(&self.config.device)
            // CPU performance tuning — must be set BEFORE the Python process starts
            // because OMP/MKL libraries read these once at import time.
            .env("OMP_NUM_THREADS", &cpu_threads)
            .env("MKL_NUM_THREADS", &cpu_threads)
            .env("OPENBLAS_NUM_THREADS", &cpu_threads)
            .env("FLAGS_use_mkldnn", "1")
            .env("FLAGS_use_avx", "1")
            // Disable Paddle's new PIR executor — it crashes with
            // ConvertPirAttribute2RuntimeAttribute on some paddle/paddleocr combos.
            .env("FLAGS_enable_pir_api", "0")
            // Silence HuggingFace progress bars (would pollute stderr/stdout)
            .env("HF_HUB_DISABLE_PROGRESS_BARS", "1")
            .env("HF_HUB_DISABLE_TELEMETRY", "1")
            // PaddleX otherwise performs a slow connectivity preflight before
            // trying the actual model source. The download itself can still run.
            .env("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(ref cache_dir) = self.config.hf_cache_dir {
            ensure_cache_dir("HuggingFace", cache_dir)?;
        }
        if let Some(ref cache_dir) = self.config.paddlex_cache_dir {
            ensure_cache_dir("PaddleX", cache_dir)?;
        }

        let offline_value = if self.config.offline_mode { "1" } else { "0" };
        cmd.env("HF_HUB_OFFLINE", offline_value)
            .env("TRANSFORMERS_OFFLINE", offline_value);

        if let Some(ref cache_dir) = self.config.hf_cache_dir {
            cmd.env("HF_HOME", cache_dir).env("HF_HUB_CACHE", cache_dir);
        }

        if let Some(ref cache_dir) = self.config.paddlex_cache_dir {
            let modelscope_cache_dir = cache_dir.join("modelscope");
            ensure_cache_dir("ModelScope", &modelscope_cache_dir)?;
            cmd.env("PADDLE_PDX_CACHE_HOME", cache_dir)
                // Legacy/no-op for current PaddleX, but harmless for older builds.
                .env("PADDLEX_HOME", cache_dir)
                // Keep ModelScope downloads/locks inside EntropIA's managed
                // runtime instead of the user's global ~/.cache/modelscope.
                .env("MODELSCOPE_CACHE", &modelscope_cache_dir)
                .env("MODELSCOPE_HOME", &modelscope_cache_dir);
        }

        eprintln!(
            "[paddle_vl] device={}, CPU threads: {cpu_threads}, MKLDNN+AVX enabled, offline_mode={}",
            self.config.device, offline_value
        );

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn PaddleVL process (python={}): {e}",
                self.config.python_path.display()
            )
        })?;

        let stdout_reader = child
            .stdout
            .take()
            .map(|stdout| std::thread::spawn(move || drain_child_pipe(stdout)));
        let stderr_reader = child
            .stderr
            .take()
            .map(|stderr| std::thread::spawn(move || drain_child_pipe(stderr)));

        eprintln!(
            "[paddle_vl] Waiting for PaddleVL (timeout: {}s, progress logs every {}s)...",
            Self::PADDLE_VL_TIMEOUT_SECS,
            Self::PROGRESS_LOG_INTERVAL_SECS
        );

        // Wait for the process with a timeout using polling.
        // try_wait() checks if the child has exited without blocking.
        let timeout = std::time::Duration::from_secs(Self::PADDLE_VL_TIMEOUT_SECS);
        let start = std::time::Instant::now();
        let check_interval = std::time::Duration::from_millis(500);
        let mut last_progress_log = std::time::Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = child.wait();
                    let stdout_buf = join_child_reader(stdout_reader);
                    let stderr_buf = join_child_reader(stderr_reader);

                    let stdout = String::from_utf8_lossy(&stdout_buf);
                    let stderr = String::from_utf8_lossy(&stderr_buf);

                    if !status.success() {
                        let exit_code = status.code().unwrap_or(-1);

                        // Classify known internal errors so fallback messages are actionable
                        let diagnostic = classify_paddlevl_failure(&stderr, &stdout);
                        let diagnostic_note = diagnostic
                            .as_deref()
                            .unwrap_or("PaddleVL subprocess exited with a non-zero code.");

                        return Err(format!(
                            "PaddleVL script failed (exit code {exit_code}). {diagnostic_note}\n\
                             Python: {}\n\
                             Script: {}\n\
                             Stderr: {}\n\
                             Stdout: {}",
                            self.config.python_path.display(),
                            self.config.script_path.display(),
                            if stderr.len() > 500 {
                                &stderr[..500]
                            } else {
                                &stderr
                            },
                            if stdout.len() > 500 {
                                &stdout[..500]
                            } else {
                                &stdout
                            },
                        ));
                    }

                    // Extract JSON between sentinels
                    let json_str = extract_sentinel_json(&stdout);

                    // Check for error key in JSON
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(error) = parsed.get("error") {
                            let error_msg = error.as_str().unwrap_or("Unknown error");
                            return Err(format!("PaddleVL script reported error: {error_msg}"));
                        }
                    }

                    let result: PaddleVlOutput = serde_json::from_str(json_str).map_err(|e| {
                        let preview = if json_str.len() > 300 {
                            &json_str[..300]
                        } else {
                            json_str
                        };
                        format!(
                            "Failed to parse PaddleVL JSON: {e}\n\
                             Extracted: {preview}\n\
                             Stderr: {}",
                            if stderr.len() > 500 {
                                &stderr[..500]
                            } else {
                                &stderr
                            }
                        )
                    })?;

                    let actual = result.actual_device.as_deref().unwrap_or("unknown");
                    eprintln!(
                        "[paddle_vl] Complete: {} blocks, {} regions, device={} (took {:.1}s)",
                        result.blocks.len(),
                        result.regions.len(),
                        actual,
                        start.elapsed().as_secs_f64()
                    );

                    return Ok(result);
                }
                Ok(None) => {
                    // Process still running — check timeout
                    if start.elapsed() > timeout {
                        eprintln!(
                            "[paddle_vl] ⏰ TIMEOUT after {}s, killing PaddleVL process",
                            start.elapsed().as_secs()
                        );
                        let _ = child.kill();
                        let _ = child.wait(); // reap the process
                        let stdout_buf = join_child_reader(stdout_reader);
                        let stderr_buf = join_child_reader(stderr_reader);
                        let stdout = String::from_utf8_lossy(&stdout_buf);
                        let stderr = String::from_utf8_lossy(&stderr_buf);
                        let tail = format_child_output_tail(&stdout, &stderr);
                        return Err(format!(
                            "PaddleVL timed out after {}s. The model may still be downloading or your CPU is heavily loaded — try again later.{tail}",
                            start.elapsed().as_secs(),
                        ));
                    }

                    // Periodic progress log so the user knows the subprocess is alive
                    if last_progress_log.elapsed().as_secs() >= Self::PROGRESS_LOG_INTERVAL_SECS {
                        eprintln!(
                            "[paddle_vl] ⏳ Still running... {}s elapsed (timeout at {}s)",
                            start.elapsed().as_secs(),
                            Self::PADDLE_VL_TIMEOUT_SECS
                        );
                        last_progress_log = std::time::Instant::now();
                    }

                    std::thread::sleep(check_interval);
                }
                Err(e) => {
                    return Err(format!("Failed to check PaddleVL process status: {e}"));
                }
            }
        }
    }

    /// Check if the PaddleVL engine is available.
    #[allow(dead_code)] // kept for potential future use
    pub fn is_available(&self) -> bool {
        let mut probe_cmd = Command::new(&self.config.python_path);
        #[cfg(windows)]
        {
            probe_cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let result = probe_cmd
            .args(["-c", "from paddleocr import PaddleOCRVL; print('ok')"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        match result {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.trim() == "ok"
            }
            _ => false,
        }
    }
}

/// Extract JSON content between `===VL_JSON_BEGIN===` and `===VL_JSON_END===`
/// sentinels. Falls back to full output if sentinels not found.
fn extract_sentinel_json(output: &str) -> &str {
    const BEGIN: &str = "===VL_JSON_BEGIN===";
    const END: &str = "===VL_JSON_END===";

    if let Some(start_idx) = output.find(BEGIN) {
        let content_start = start_idx + BEGIN.len();
        if let Some(end_idx) = output[content_start..].find(END) {
            let json_content = &output[content_start..content_start + end_idx];
            return json_content.trim();
        }
    }

    output.trim()
}

/// Detect whether a PaddleVL failure is caused by the known PIR executor bug.
///
/// When PaddlePaddle's new PIR executor encounters an unsupported attribute
/// conversion, it emits:
///   `(Unimplemented) ConvertPirAttribute2RuntimeAttribute not support [...]`
/// This is a framework-level bug, not a user error. Disabling PIR via
/// `FLAGS_enable_pir_api=0` is the recommended workaround.
fn is_pir_executor_error(stderr: &str) -> bool {
    stderr.contains("ConvertPirAttribute2RuntimeAttribute")
        || stderr.contains("pir::ArrayAttribute")
        || stderr.contains("FLAGS_enable_pir_api")
}

/// Classify a PaddleVL failure and return a human-readable diagnostic string.
///
/// This helps the Rust-side logs and fallback messages explain *why* PaddleVL
/// failed, so users (and maintainers) don't waste time chasing red herrings.
fn classify_paddlevl_failure(stderr: &str, stdout: &str) -> Option<String> {
    if is_pir_executor_error(stderr) || is_pir_executor_error(stdout) {
        return Some(
            "PaddlePaddle PIR/oneDNN executor bug detected (Paddle#77340). \
             This crash affects paddlepaddle >=3.3.0 on CPU. \
             EntropIA's dependency registry enforces paddlepaddle>=3.2.1,<3.3.0. \
             If you see this error, your environment may have an unsupported version installed manually. \
             Fix: Reset the environment from the app (Entorno → Resetear entorno) so EntropIA can install the correct version automatically."
                .to_string(),
        );
    }
    if is_paddlex_cache_permission_error(stderr) || is_paddlex_cache_permission_error(stdout) {
        return Some(
            "PaddleOCR-VL cannot read/write its PaddleX model cache. \
             EntropIA points PaddleX at an app-owned cache via PADDLE_PDX_CACHE_HOME; \
             if this persists, clear the affected PaddleX cache directory or check folder permissions."
                .to_string(),
        );
    }
    None
}

fn is_paddlex_cache_permission_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("permission denied")
        && (lower.contains(".paddlex")
            || lower.contains("official_models")
            || lower.contains("paddle_pdx_cache_home")
            || lower.contains("paddlex"))
}

/// Score a Python candidate by how likely it is to be a dedicated PaddleOCR-VL env.
///
/// Higher score = better candidate. Used to prioritize purpose-built envs (e.g.
/// `ppocrvl-py312`, `paddle2`) over the base Conda interpreter, which tends to
/// be slower because it has more packages loaded into the import path.
fn score_python_candidate(path: &Path) -> i32 {
    let path_str = path.to_string_lossy().to_lowercase();
    let mut score = 0;

    // Strong signals: name contains paddle/ocr/vl/pp keywords
    if path_str.contains("ppocrvl") {
        score += 100;
    }
    if path_str.contains("paddle") {
        score += 50;
    }
    if path_str.contains("pp3") || path_str.contains("ppv") {
        score += 30;
    }
    if path_str.contains("ocr") {
        score += 20;
    }

    // Bonus for being in an envs/ subdirectory (dedicated env)
    if path_str.contains("\\envs\\") || path_str.contains("/envs/") {
        score += 25;
    }

    // Penalty for being the base Conda Python (no envs/ in path, root of conda dir)
    // These tend to have many unrelated packages and slower imports.
    if !path_str.contains("\\envs\\")
        && !path_str.contains("/envs/")
        && (path_str.contains("miniconda") || path_str.contains("anaconda"))
    {
        score -= 10;
    }

    score
}

/// Find the Python interpreter on the system that has `PaddleOCRVL` available.
///
/// Uses the shared Python candidate cache to avoid redundant filesystem scans.
/// Probes candidates sorted by their likelihood of being a dedicated PaddleOCR-VL
/// environment (scored by path heuristics).
///
/// CRITICAL: The probe verifies `PaddleOCRVL` specifically, not just `paddleocr`.
/// The `paddleocr` package can be installed without the `[doc-parser]` extra,
/// in which case `PaddleOCRVL` is missing and the subprocess would crash later.
pub fn which_python_for_paddle_vl(settings_db_path: Option<&std::path::Path>) -> Option<PathBuf> {
    crate::python_discovery::which_python_for_module_scored(
        "paddle_vl",
        "paddle_vl",
        "PaddleOCRVL",
        "from paddleocr import PaddleOCRVL; print('ok')",
        settings_db_path,
        &score_python_candidate,
    )
}

/// Detect whether an NVIDIA GPU is present on the system.
///
/// This is a *hardware* check only — it does NOT verify that the Python
/// paddlepaddle-gpu stack is installed or functional. The Python subprocess
/// will validate software-level GPU support and fall back to CPU if needed.
///
/// Detection strategy (fast, non-blocking):
///   1. Run `nvidia-smi -L` (list GPUs). If it returns successfully with
///      output containing "GPU", an NVIDIA GPU is present.
///   2. Fall back to OS hardware inventory. This matters on Linux when
///      `nvidia-smi` fails with a driver/library mismatch after driver updates;
///      the hardware still exists, even if runtime GPU use may require rebooting
///      or fixing the driver stack.
///   3. On Windows, also check for `nvidia-smi.exe` in Program Files.
///
/// Returns `false` if no hardware signal reports an NVIDIA GPU.
fn detect_nvidia_gpu() -> bool {
    // Fast path: nvidia-smi -L lists GPUs in ~50-100ms.
    let mut cmd = Command::new("nvidia-smi");
    cmd.arg("-L");
    match command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi -L") {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let has_gpu = !stdout.trim().is_empty() && stdout.contains("GPU");
            if has_gpu {
                eprintln!("[paddle_vl] detect_nvidia_gpu: found GPU via nvidia-smi -L");
            }
            has_gpu
        }
        _ => {
            // nvidia-smi not available or temporarily broken — try hardware inventory.
            detect_nvidia_gpu_from_system_inventory()
        }
    }
}

#[cfg(target_os = "linux")]
fn detect_nvidia_gpu_from_system_inventory() -> bool {
    if std::fs::read_dir("/proc/driver/nvidia/gpus")
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
    {
        eprintln!("[paddle_vl] detect_nvidia_gpu: found GPU via /proc/driver/nvidia/gpus");
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
            eprintln!("[paddle_vl] detect_nvidia_gpu: found GPU via PCI inventory");
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
        let mut cmd = Command::new(&smi);
        cmd.arg("-L");
        if let Ok(output) = command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi.exe -L")
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if output.status.success() && stdout.contains("GPU") {
                eprintln!(
                    "[paddle_vl] detect_nvidia_gpu: found GPU via {}",
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

fn command_output_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    label: &str,
) -> Result<Output, String> {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

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

fn drain_child_pipe(mut pipe: impl std::io::Read + Send + 'static) -> Vec<u8> {
    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer);
    buffer
}

fn join_child_reader(handle: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

fn format_child_output_tail(stdout: &str, stderr: &str) -> String {
    let lines = stderr
        .lines()
        .chain(stdout.lines())
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    if lines.trim().is_empty() {
        "".to_string()
    } else {
        format!("\nÚltima salida PaddleVL:\n{lines}")
    }
}

pub fn create_paddle_vl_engine_result(
    app_handle: &tauri::AppHandle,
    settings_db_path: &std::path::Path,
) -> Result<PaddleVlEngine, String> {
    let runtime_root = managed_runtime_root_for_paddle_vl(app_handle)
        .ok()
        .flatten();
    let script_path = resolve_paddle_vl_script_path_from_roots(
        runtime_root.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    );
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .ok()
        .or_else(|| settings_db_path.parent().map(Path::to_path_buf));
    let (hf_cache_dir, paddlex_cache_dir) =
        resolve_paddle_vl_cache_dirs(runtime_root.as_deref(), app_data_dir.as_deref());
    let offline_mode =
        runtime_root.is_some() && paddle_vl_caches_look_complete(paddlex_cache_dir.as_deref());
    if runtime_root.is_some() && !offline_mode {
        crate::app_logs::warn(
            app_handle,
            "ocrh",
            "Cache PaddleOCR-VL incompleta; OCRH permitirá descarga online administrada",
        );
    }

    // Find Python interpreter with PaddleOCR-VL. OCRH should prefer the
    // freshly hydrated managed venv when release runtime is healthy, even if a
    // stale/system Python selection exists in settings. Keep this preference
    // local to OCRH so other Python-backed features keep their existing policy.
    let managed_python = runtime_root.as_deref().and_then(|root| {
        let candidate = managed_venv_python_path(root);
        if candidate.is_file()
            && crate::python_discovery::probe_python_module(
                &candidate,
                "from paddleocr import PaddleOCRVL; print('ok')",
            )
        {
            crate::app_logs::info(
                app_handle,
                "ocrh",
                format!(
                    "PaddleOCR-VL usará Python del runtime administrado: {}",
                    candidate.display()
                ),
            );
            Some(candidate)
        } else {
            None
        }
    });
    let python_path = match managed_python
        .or_else(|| which_python_for_paddle_vl(Some(settings_db_path)))
    {
        Some(p) => p,
        None => {
            return Err(
                "No Python with PaddleOCRVL found — OCRH local is unavailable. Reinstall PaddleOCR or check the managed venv compatibility."
                    .to_string(),
            );
        }
    };

    // Auto-detect GPU and prefer it, but let the Python subprocess validate
    // software-level support and fall back to CPU if GPU init fails.
    let device = if detect_nvidia_gpu() {
        eprintln!("[paddle_vl] NVIDIA GPU detected — preferring GPU for PaddleOCR-VL");
        "gpu".to_string()
    } else {
        eprintln!("[paddle_vl] No NVIDIA GPU detected — using CPU for PaddleOCR-VL");
        "cpu".to_string()
    };

    match PaddleVlEngine::init(PaddleVlConfig {
        python_path,
        script_path,
        hf_cache_dir,
        paddlex_cache_dir,
        offline_mode,
        device,
    }) {
        Ok(engine) => Ok(engine),
        Err(e) => Err(format!("❌ Failed to create PaddleVLEngine: {e}")),
    }
}

fn resolve_paddle_vl_script_path_from_roots(
    managed_root: Option<&Path>,
    manifest_dir: &Path,
) -> PathBuf {
    if let Some(root) = managed_root {
        let managed = managed_script_path(root, "paddle_vl.py");
        if managed.exists() {
            return managed;
        }
    }

    normalize_windows_path(manifest_dir.join("scripts/paddle_vl.py"))
}

fn ensure_cache_dir(label: &str, cache_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(cache_dir).map_err(|error| {
        format!(
            "Failed to create {label} cache directory {}: {error}",
            cache_dir.display()
        )
    })
}

fn managed_runtime_root_for_paddle_vl(
    app_handle: &tauri::AppHandle,
) -> Result<Option<PathBuf>, String> {
    managed_runtime_root_for_paddle_vl_with(
        || RuntimeManager::new().ensure_ready_or_bootstrap(app_handle),
        || RuntimeManager::new().hydrated_runtime_root(app_handle),
    )
}

fn managed_runtime_root_for_paddle_vl_with<E, H>(
    ensure_ready_or_bootstrap: E,
    hydrated_runtime_root: H,
) -> Result<Option<PathBuf>, String>
where
    E: FnOnce() -> Result<crate::runtime::status::RuntimeStatus, String>,
    H: FnOnce() -> Result<Option<PathBuf>, String>,
{
    let status = ensure_ready_or_bootstrap()?;
    if status.state != crate::runtime::status::RuntimeState::Healthy {
        return Ok(None);
    }

    hydrated_runtime_root()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use std::cell::RefCell;
    use tempfile::tempdir;

    #[test]
    fn test_extract_sentinel_json() {
        let output =
            "some noise\n===VL_JSON_BEGIN==={\"text\":\"hello\"}\n===VL_JSON_END===\nmore noise";
        let extracted = extract_sentinel_json(output);
        assert_eq!(extracted, r#"{"text":"hello"}"#);
    }

    #[test]
    fn test_extract_sentinel_json_fallback() {
        let output = r#"{"text":"hello"}"#;
        let extracted = extract_sentinel_json(output);
        assert_eq!(extracted, r#"{"text":"hello"}"#);
    }

    #[test]
    fn test_detects_pir_executor_error_from_stderr() {
        let stderr = "Exception from the 'cv' worker: (Unimplemented) ConvertPirAttribute2RuntimeAttribute not support [pir::ArrayAttribute<pir::DoubleAttribute>]";
        assert!(is_pir_executor_error(stderr));
        let classification = classify_paddlevl_failure(stderr, "");
        assert!(classification.is_some());
        let msg = classification.unwrap();
        assert!(msg.contains("PIR/oneDNN executor bug"));
        assert!(
            msg.contains("<3.3.0"),
            "diagnostic should mention the version cap"
        );
    }

    #[test]
    fn test_detects_pir_executor_error_from_stdout() {
        let stdout = "some prefix\nConvertPirAttribute2RuntimeAttribute\nsuffix";
        // The standalone check also detects it (the function scans any text)
        assert!(is_pir_executor_error(stdout));
        // classify_paddlevl_failure scans both stderr and stdout
        let classification = classify_paddlevl_failure("", stdout);
        assert!(classification.is_some());
    }

    #[test]
    fn test_no_false_positive_on_unrelated_error() {
        let stderr = "CUDA out of memory";
        assert!(!is_pir_executor_error(stderr));
        assert!(classify_paddlevl_failure(stderr, "").is_none());
    }

    #[test]
    fn test_detects_paddlex_cache_permission_error() {
        let stderr =
            "PermissionError: [Errno 13] Permission denied: 'C:\\Users\\test\\.paddlex\\official_models\\PP-DocLayoutV3\\inference.yml'";
        let classification = classify_paddlevl_failure(stderr, "");
        assert!(classification.is_some());
        assert!(classification.unwrap().contains("PaddleX model cache"));
    }

    #[test]
    fn resolves_managed_paddle_vl_script_before_dev_fallbacks() {
        let runtime_dir = tempdir().expect("runtime dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let managed_script = runtime_dir.path().join("scripts").join("paddle_vl.py");
        std::fs::create_dir_all(managed_script.parent().expect("script parent"))
            .expect("create script dir");
        std::fs::write(&managed_script, "print('ok')").expect("write managed script");

        let resolved =
            resolve_paddle_vl_script_path_from_roots(Some(runtime_dir.path()), manifest_dir.path());

        assert_eq!(resolved, managed_script);
    }

    #[test]
    fn resolves_canonical_paddle_vl_script_without_using_legacy_resources() {
        let manifest_dir = tempdir().expect("manifest dir");
        let legacy_script = manifest_dir.path().join("resources/scripts/paddle_vl.py");
        let canonical_script = manifest_dir.path().join("scripts/paddle_vl.py");
        std::fs::create_dir_all(legacy_script.parent().expect("legacy parent"))
            .expect("create legacy dir");
        std::fs::create_dir_all(canonical_script.parent().expect("canonical parent"))
            .expect("create canonical dir");
        std::fs::write(&legacy_script, "print('legacy')").expect("write legacy script");
        std::fs::write(&canonical_script, "print('canonical')").expect("write canonical script");

        let resolved = resolve_paddle_vl_script_path_from_roots(None, manifest_dir.path());

        assert_eq!(resolved, canonical_script);
    }

    #[test]
    fn derives_managed_paddle_vl_cache_dirs_from_runtime_root() {
        let runtime_dir = tempdir().expect("runtime dir");

        let app_data_dir = tempdir().expect("app data dir");
        let (hf_cache, paddlex_cache) =
            resolve_paddle_vl_cache_dirs(Some(runtime_dir.path()), Some(app_data_dir.path()));

        assert_eq!(hf_cache, Some(runtime_dir.path().join("caches").join("hf")));
        assert_eq!(
            paddlex_cache,
            Some(runtime_dir.path().join("caches").join("paddlex"))
        );
    }

    #[test]
    fn paddle_vl_cache_completeness_requires_real_vl_weights() {
        let cache_dir = tempdir().expect("cache dir");
        let layout = cache_dir
            .path()
            .join("official_models")
            .join("PP-DocLayoutV3");
        let vl = cache_dir
            .path()
            .join("official_models")
            .join("PaddleOCR-VL-1.5");
        std::fs::create_dir_all(&layout).expect("layout dir");
        std::fs::create_dir_all(&vl).expect("vl dir");
        std::fs::write(layout.join("inference.yml"), "mode: paddle").expect("layout yml");
        std::fs::write(layout.join("inference.json"), "{}").expect("layout json");
        std::fs::write(layout.join("inference.pdiparams"), "weights").expect("layout params");
        std::fs::create_dir_all(vl.join(".cache").join("huggingface").join("download"))
            .expect("metadata dir");
        std::fs::write(
            vl.join(".cache")
                .join("huggingface")
                .join("download")
                .join("model.safetensors.metadata"),
            "metadata only",
        )
        .expect("metadata");

        assert!(!paddle_vl_caches_look_complete(Some(cache_dir.path())));

        std::fs::write(vl.join("model.safetensors"), "real weights").expect("vl weights");

        assert!(paddle_vl_caches_look_complete(Some(cache_dir.path())));
    }

    #[test]
    fn derives_app_owned_paddle_vl_cache_dirs_without_runtime_root() {
        let app_data_dir = tempdir().expect("app data dir");

        let (hf_cache, paddlex_cache) =
            resolve_paddle_vl_cache_dirs(None, Some(app_data_dir.path()));

        assert_eq!(hf_cache, Some(app_data_dir.path().join("hf_cache")));
        assert_eq!(
            paddlex_cache,
            Some(app_data_dir.path().join("paddlex_cache"))
        );
    }

    #[test]
    fn paddle_vl_runtime_resolution_bootstraps_before_using_managed_assets() {
        let calls = RefCell::new(Vec::new());
        let expected = PathBuf::from("/tmp/runtime-ready");

        let resolved = managed_runtime_root_for_paddle_vl_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::Healthy,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: true,
                    summary: "Runtime listo".to_string(),
                    blocked_capabilities: vec![],
                    details: vec![],
                    guidance: vec![],
                    bootstrap_eligible: false,
                    bootstrap_required: false,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(expected.clone()))
            },
        )
        .expect("managed runtime should resolve");

        assert_eq!(resolved, Some(expected));
        assert_eq!(calls.into_inner(), vec!["ensure_ready", "hydrated_root"]);
    }

    #[test]
    fn paddle_vl_runtime_resolution_honors_blocked_bootstrap_status() {
        let calls = RefCell::new(Vec::new());

        let resolved = managed_runtime_root_for_paddle_vl_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedOffline,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "Bootstrap offline".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Ocr],
                    details: vec!["offline".to_string()],
                    guidance: vec!["Reintentá".to_string()],
                    bootstrap_eligible: true,
                    bootstrap_required: true,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(PathBuf::from("/tmp/stale-runtime")))
            },
        )
        .expect("blocked bootstrap should degrade gracefully");

        assert_eq!(resolved, None);
        assert_eq!(calls.into_inner(), vec!["ensure_ready"]);
    }

    #[test]
    fn detect_nvidia_gpu_returns_bool_without_panicking() {
        // We can't assert true/false because the test environment may or may not
        // have nvidia-smi. The contract is: it must return a bool and never panic.
        let _result = detect_nvidia_gpu();
        // If we get here, the function didn't panic — success.
    }

    #[test]
    fn paddle_vl_output_deserializes_with_actual_device() {
        let json = r#"{
            "text": "hello",
            "method": "paddle_vl",
            "blocks": [],
            "regions": [],
            "image_width": 100,
            "image_height": 200,
            "actual_device": "gpu"
        }"#;
        let output: PaddleVlOutput = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(output.actual_device, Some("gpu".to_string()));
    }

    #[test]
    fn paddle_vl_output_deserializes_without_actual_device() {
        // Backwards compatibility: old Python script may not emit actual_device.
        let json = r#"{
            "text": "hello",
            "method": "paddle_vl",
            "blocks": [],
            "regions": [],
            "image_width": 100,
            "image_height": 200
        }"#;
        let output: PaddleVlOutput = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(output.actual_device, None);
    }

    #[test]
    fn paddle_vl_config_includes_device() {
        let config = PaddleVlConfig {
            python_path: PathBuf::from("/usr/bin/python"),
            script_path: PathBuf::from("/tmp/paddle_vl.py"),
            hf_cache_dir: None,
            paddlex_cache_dir: None,
            offline_mode: false,
            device: "gpu".to_string(),
        };
        assert_eq!(config.device, "gpu");
    }
}

fn resolve_paddle_vl_cache_dirs(
    managed_root: Option<&Path>,
    app_data_dir: Option<&Path>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    if let Some(root) = managed_root {
        return (
            Some(managed_hf_cache_dir(root)),
            Some(managed_paddlex_cache_dir(root)),
        );
    }

    if let Some(app_data) = app_data_dir {
        return (
            Some(app_data.join("hf_cache")),
            Some(app_data.join("paddlex_cache")),
        );
    }

    (None, None)
}

fn paddle_vl_caches_look_complete(paddlex_cache_dir: Option<&Path>) -> bool {
    let Some(cache_dir) = paddlex_cache_dir else {
        return false;
    };

    let official_models = cache_dir.join("official_models");
    let layout_model = official_models.join("PP-DocLayoutV3");
    let vl_model = official_models.join("PaddleOCR-VL-1.5");

    let layout_ready = layout_model.join("inference.yml").is_file()
        && layout_model.join("inference.pdiparams").is_file()
        && (layout_model.join("inference.json").is_file()
            || layout_model.join("inference.pdmodel").is_file());

    let vl_ready = vl_model.join("model.safetensors").is_file()
        || vl_model.join("model_state.pdparams").is_file()
        || vl_model.join("inference.pdparams").is_file();

    layout_ready && vl_ready
}
