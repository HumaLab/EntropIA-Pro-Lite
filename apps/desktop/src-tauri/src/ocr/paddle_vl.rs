//! PaddleOCR-VL engine — layout-aware OCR via Python subprocess.
//!
//! Calls the paddle_vl.py script which runs PaddleOCR-VL to perform
//! both layout detection and OCR in a single pass. Returns structured
//! results with text, blocks, and regions.
//!
//! Fallback chain: PaddleVL → lightweight PaddleOCR (if PaddleVL fails or times out)

use super::paddle_vl_types::{LayoutV3Output, PaddleVlOutput};
use crate::path_utils::normalize_windows_path;
use crate::runtime::{
    managed_hf_cache_dir, managed_paddlex_cache_dir, managed_script_path, managed_venv_python_path,
    RuntimeManager,
};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
    /// Path to the layout-only `layout_v3.py` script (same PP-DocLayoutV3
    /// model the GLM-OCR engine uses, without any recognition).
    pub layout_script_path: PathBuf,
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
    /// When set, the engine runs `glm_ocr_local.py` (layout on CPU, GLM-OCR on
    /// a local llama-server) instead of PaddleOCR-VL. Same output contract.
    pub glm_local: Option<GlmLocalBackend>,
    /// When set, `paddle_vl.py` keeps layout on CPU and sends recognition to
    /// PaddleOCR-VL served by a local llama-server.
    pub vl_server: Option<GlmLocalBackend>,
}

/// What the local GLM-OCR backend needs to start its server.
#[derive(Clone)]
pub struct GlmLocalBackend {
    pub assets: super::glm_ocr_local::GlmLocalAssets,
    /// Archive database path; the server's models, pid file and log live next to it.
    pub db_path: PathBuf,
}

/// The PaddleOCR-VL engine — spawns Python as a child process.
///
/// Each call spawns a fresh Python process. No persistent state.
#[derive(Clone)]
pub struct PaddleVlEngine {
    config: PaddleVlConfig,
}

/// Shared Python subprocess setup for the OCRH helpers (`paddle_vl.py`,
/// `glm_ocr_local.py`, `layout_v3.py`): CPU thread tuning, Paddle flags,
/// HF/PaddleX cache dirs and offline mode.
///
/// Returns the configured command plus the tuning values for logging.
fn build_ocrh_command(
    config: &PaddleVlConfig,
    script_path: &Path,
    image_path: &str,
    device: &str,
    extra_args: &[String],
) -> Result<(Command, String, &'static str), String> {
    // Determine optimal thread count: all logical cores capped at 8.
    // Going beyond 8 typically hurts due to memory bandwidth + scheduler overhead.
    let cpu_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .to_string();

    // oneDNN (MKL-DNN) is a CPU-only accelerator. Forcing it on a GPU run
    // makes Paddle 3.2 mix oneDNN kernels into the CUDA pipeline and fail
    // inference with "event_waiter_[1][0] shall not be nullptr", which
    // silently degraded every GPU OCRH run to CPU (and to timeouts).
    let use_mkldnn = if device == "gpu" { "0" } else { "1" };
    let mut cmd = Command::new(&config.python_path);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.arg(script_path).arg(image_path);
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd
        // CPU performance tuning — must be set BEFORE the Python process starts
        // because OMP/MKL libraries read these once at import time.
        .env("OMP_NUM_THREADS", &cpu_threads)
        .env("MKL_NUM_THREADS", &cpu_threads)
        .env("OPENBLAS_NUM_THREADS", &cpu_threads)
        .env("FLAGS_use_mkldnn", use_mkldnn)
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

    if let Some(cache_dir) = config.hf_cache_dir.as_deref() {
        ensure_cache_dir("HuggingFace", cache_dir)?;
    }
    if let Some(cache_dir) = config.paddlex_cache_dir.as_deref() {
        ensure_cache_dir("PaddleX", cache_dir)?;
    }

    let offline_value = if config.offline_mode { "1" } else { "0" };
    cmd.env("HF_HUB_OFFLINE", offline_value)
        .env("TRANSFORMERS_OFFLINE", offline_value);

    if let Some(cache_dir) = config.hf_cache_dir.as_deref() {
        cmd.env("HF_HOME", cache_dir).env("HF_HUB_CACHE", cache_dir);
    }

    if let Some(cache_dir) = config.paddlex_cache_dir.as_deref() {
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

    if device == "gpu" && serializes_cuda_launches_on(std::env::consts::OS) {
        cmd.env("CUDA_LAUNCH_BLOCKING", "1");
    }

    Ok((cmd, cpu_threads, use_mkldnn))
}

/// Whether GPU pages run CUDA kernels one at a time on this OS.
///
/// Linux: Paddle 3.2 hangs at random mid-page on both an RTX 4060 Laptop and
/// an RTX 5090 (driver 580): GPU at 0 %, one CPU core spinning, forever.
/// Launching kernels one at a time stopped it (0 hangs in 23 runs) at a cost:
/// a dense newspaper page took 166 s instead of 110 s on the 4060, 35 s
/// instead of 20 s on the 5090.
///
/// Windows: under WDDM the same setting kept that page on an A10G (driver
/// 621.14) past the 900 s page timeout twice, while without it 4/4 runs
/// finished in 189-199 s with all 51 blocks. A page that does stall there is
/// still caught by the GPU stall watchdog and run once more.
fn serializes_cuda_launches_on(os: &str) -> bool {
    os == "linux"
}

/// The line `paddle_vl.py` writes once the pipeline is loaded and the page
/// itself starts. Before it the GPU is legitimately idle (imports, model load).
const PAGE_STARTED_MARKER: &str = "[paddle_vl] Processing:";

/// How long the GPU may sit idle in the middle of a page before the run
/// counts as stuck. A healthy page keeps the GPU busy between short CPU steps.
const GPU_STALL_WINDOW: Duration = Duration::from_secs(120);

/// How often the GPU's utilization is sampled while a page runs.
const GPU_UTIL_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Runs per page on the GPU: the first, plus one retry when it got stuck.
const GPU_PAGE_ATTEMPTS: usize = 2;

/// Why one run of a page did not produce text.
#[derive(Debug, PartialEq)]
enum PageRunError {
    /// The GPU sat idle mid-page and the run was killed; a new run usually works.
    GpuStalled(String),
    /// Any other failure; running the page again would fail the same way.
    Failed(String),
}

/// Run a page up to `attempts` times, starting again only after a stuck GPU.
fn run_with_stall_retry<T>(
    attempts: usize,
    mut run: impl FnMut(usize) -> Result<T, PageRunError>,
) -> Result<T, String> {
    let mut attempt = 1;
    loop {
        match run(attempt) {
            Ok(value) => return Ok(value),
            Err(PageRunError::Failed(message)) => return Err(message),
            Err(PageRunError::GpuStalled(message)) if attempt >= attempts => return Err(message),
            Err(PageRunError::GpuStalled(message)) => {
                log::warn!(
                    target: "paddle_vl",
                    "Page stuck on the GPU (attempt {attempt} of {attempts}), starting it again: {message}"
                );
                attempt += 1;
            }
        }
    }
}

/// Tracks how long the GPU has been idle during a page.
struct GpuStallWatch {
    window: Duration,
    last_busy: Instant,
}

impl GpuStallWatch {
    fn new(now: Instant, window: Duration) -> Self {
        Self {
            window,
            last_busy: now,
        }
    }

    /// Record one utilization sample and say whether the page is stuck.
    /// An unreadable sample counts as busy: missing data never kills a page.
    fn observe(&mut self, utilization: Option<u32>, now: Instant) -> bool {
        if utilization != Some(0) {
            self.last_busy = now;
        }
        self.idle_for(now) >= self.window
    }

    fn idle_for(&self, now: Instant) -> Duration {
        now.duration_since(self.last_busy)
    }
}

/// The busiest GPU's utilization, from `nvidia-smi` output with one percentage per line.
fn parse_gpu_utilization(stdout: &str) -> Option<u32> {
    stdout
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .max()
}

fn read_gpu_utilization() -> Option<u32> {
    let mut cmd = Command::new("nvidia-smi");
    cmd.args([
        "--query-gpu=utilization.gpu",
        "--format=csv,noheader,nounits",
    ]);
    let output =
        command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi utilization").ok()?;
    if !output.status.success() {
        return None;
    }
    parse_gpu_utilization(&String::from_utf8_lossy(&output.stdout))
}

impl PaddleVlEngine {
    /// Validate configuration and create the engine.
    ///
    /// NOTE: Python interpreter was already validated by `which_python_for_paddle_vl()`
    /// which ran `from paddleocr import PaddleOCRVL; print('ok')` successfully.
    /// Redundant verification (e.g., `python --version`) is skipped — the
    /// discovery module already proved the interpreter works.
    pub fn init(config: PaddleVlConfig) -> Result<Self, String> {
        // Verify scripts exist
        if !config.script_path.exists() {
            return Err(format!(
                "PaddleVL script not found: {}",
                config.script_path.display()
            ));
        }
        if !config.layout_script_path.exists() {
            return Err(format!(
                "Layout script not found: {}",
                config.layout_script_path.display()
            ));
        }

        if std::env::var("ENTROPIA_VERBOSE_STARTUP")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            log::info!(
                target: "paddle_vl",
                "Engine configured: python={}, script={}",
                config.python_path.display(),
                config.script_path.display(),
            );
        }

        Ok(Self { config })
    }

    /// Whether this engine runs local GLM-OCR rather than PaddleOCR-VL.
    pub fn uses_glm_local(&self) -> bool {
        self.config.glm_local.is_some()
    }

    /// Which local backend this engine was built for.
    pub fn backend_kind(&self) -> BackendKind {
        if self.config.glm_local.is_some() {
            BackendKind::GlmOcr
        } else if self.config.vl_server.is_some() {
            BackendKind::PaddleVlServer
        } else {
            BackendKind::PaddleVlNative
        }
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

    /// Maximum time (in seconds) to wait for the layout-only subprocess.
    ///
    /// PP-DocLayoutV3 on CPU reads a newspaper page in ~6-25 s once the model
    /// is cached; 5 minutes cover cold starts without hanging OCRH forever.
    const LAYOUT_TIMEOUT_SECS: u64 = 300;

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
        run_with_stall_retry(GPU_PAGE_ATTEMPTS, |_attempt| {
            let mut stalled = false;
            self.detect_once(image_path, &mut stalled)
                .map_err(|message| {
                    if stalled {
                        PageRunError::GpuStalled(message)
                    } else {
                        PageRunError::Failed(message)
                    }
                })
        })
    }

    /// One run of a page. Sets `stalled` when it was killed because the GPU
    /// sat idle mid-page, the one failure worth running again.
    fn detect_once(&self, image_path: &str, stalled: &mut bool) -> Result<PaddleVlOutput, String> {
        log::info!(
            target: "paddle_vl",
            "Spawning PaddleOCR-VL for: {} (device={})",
            image_path, self.config.device
        );

        // Held until this page is done: meanwhile the chat leaves the GPU alone.
        let mut _server_lease = None;
        let extra_args: Vec<String> = if let Some(glm) = &self.config.glm_local {
            // Layout runs on CPU inside the script; recognition goes to the
            // persistent llama-server, started here on first use.
            let lease = super::glm_ocr_local::ensure_server(&glm.assets, &glm.db_path)?;
            let url = lease.base_url().to_string();
            _server_lease = Some(lease);
            vec![
                "--server-url".to_string(),
                url,
                "--parallel".to_string(),
                super::glm_ocr_local::PARALLEL_SLOTS.to_string(),
            ]
        } else if let Some(vl) = &self.config.vl_server {
            // Layout runs on CPU inside the script; PaddleOCR-VL recognition
            // goes to the persistent llama-server, started here on first use.
            let lease = super::paddle_vl_local::ensure_server(&vl.assets, &vl.db_path)?;
            let url = lease.base_url().to_string();
            _server_lease = Some(lease);
            vec![
                "--device".to_string(),
                self.config.device.clone(),
                "--vl-server-url".to_string(),
                url,
                "--vl-max-concurrency".to_string(),
                super::paddle_vl_local::PARALLEL_SLOTS.to_string(),
            ]
        } else {
            vec!["--device".to_string(), self.config.device.clone()]
        };
        let (mut cmd, cpu_threads, use_mkldnn) = build_ocrh_command(
            &self.config,
            &self.config.script_path,
            image_path,
            &self.config.device,
            &extra_args,
        )?;

        let offline_value = if self.config.offline_mode { "1" } else { "0" };
        log::info!(
            target: "paddle_vl",
            "device={}, CPU threads: {cpu_threads}, FLAGS_use_mkldnn={use_mkldnn}, offline_mode={offline_value}",
            self.config.device,
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
        // Only PaddleOCR-VL on the GPU is watched: the local GLM backend runs
        // recognition in llama-server, whose GPU use says nothing about this page.
        let watch_gpu = self.config.glm_local.is_none()
            && self.config.vl_server.is_none()
            && self.config.device == "gpu";
        let page_started = Arc::new(AtomicBool::new(false));
        let stderr_reader = child.stderr.take().map(|stderr| {
            let page_started = page_started.clone();
            std::thread::spawn(move || {
                drain_child_pipe_flagging(stderr, PAGE_STARTED_MARKER, page_started)
            })
        });
        let mut stall_watch: Option<GpuStallWatch> = None;
        let mut last_gpu_sample = Instant::now();

        log::info!(
            target: "paddle_vl",
            "Waiting for PaddleVL (timeout: {}s, progress logs every {}s)...",
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
                    log::info!(
                        target: "paddle_vl",
                        "Complete: {} blocks, {} regions, device={} (took {:.1}s)",
                        result.blocks.len(),
                        result.regions.len(),
                        actual,
                        start.elapsed().as_secs_f64()
                    );

                    return Ok(result);
                }
                Ok(None) => {
                    if watch_gpu
                        && page_started.load(Ordering::SeqCst)
                        && last_gpu_sample.elapsed() >= GPU_UTIL_SAMPLE_INTERVAL
                    {
                        let now = Instant::now();
                        last_gpu_sample = now;
                        let watch = stall_watch
                            .get_or_insert_with(|| GpuStallWatch::new(now, GPU_STALL_WINDOW));
                        if watch.observe(read_gpu_utilization(), now) {
                            let idle = watch.idle_for(now).as_secs();
                            log::warn!(
                                target: "paddle_vl",
                                "GPU idle for {idle}s in the middle of the page, killing PaddleVL process"
                            );
                            let _ = child.kill();
                            let _ = child.wait();
                            let stdout_buf = join_child_reader(stdout_reader);
                            let stderr_buf = join_child_reader(stderr_reader);
                            let tail = format_child_output_tail(
                                &String::from_utf8_lossy(&stdout_buf),
                                &String::from_utf8_lossy(&stderr_buf),
                            );
                            *stalled = true;
                            return Err(format!(
                                "PaddleOCR-VL stopped making progress on the GPU ({idle}s with the GPU idle).{tail}"
                            ));
                        }
                    }

                    // Process still running — check timeout
                    if start.elapsed() > timeout {
                        log::warn!(
                            target: "paddle_vl",
                            "⏰ TIMEOUT after {}s, killing PaddleVL process",
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
                        log::info!(
                            target: "paddle_vl",
                            "⏳ Still running... {}s elapsed (timeout at {}s)",
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

    /// Run PP-DocLayoutV3 on an image file (layout boxes only, no text).
    ///
    /// Used by the lightweight OCR path to read segment by segment in reading
    /// order instead of the full page at once. Same Python environment as the
    /// other OCRH helpers; the layout model comes from the same PaddleX cache.
    /// On timeout (5 minutes), kills the subprocess and returns an error.
    pub fn detect_layout(&self, image_path: &str) -> Result<LayoutV3Output, String> {
        log::info!(
            target: "paddle_vl",
            "Spawning layout_v3 for: {image_path}"
        );
        let (mut cmd, _threads, _mkldnn) = build_ocrh_command(
            &self.config,
            &self.config.layout_script_path,
            image_path,
            "cpu",
            &[],
        )?;
        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn layout process (python={}): {e}",
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

        let timeout = std::time::Duration::from_secs(Self::LAYOUT_TIMEOUT_SECS);
        let start = std::time::Instant::now();
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
                        return Err(format!(
                            "layout_v3 script failed (exit code {exit_code}).\n\
                             Python: {}\n\
                             Script: {}\n\
                             Stderr tail: {}",
                            self.config.python_path.display(),
                            self.config.layout_script_path.display(),
                            stderr
                                .chars()
                                .rev()
                                .take(500)
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect::<String>(),
                        ));
                    }

                    // Extract JSON between sentinels
                    let json_str = extract_sentinel_json(&stdout);

                    // Check for error key in JSON
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(error) = parsed.get("error") {
                            let error_msg = error.as_str().unwrap_or("Unknown error");
                            return Err(format!("layout_v3 script reported error: {error_msg}"));
                        }
                    }

                    let result: LayoutV3Output = serde_json::from_str(json_str).map_err(|e| {
                        let preview = if json_str.len() > 300 {
                            &json_str[..300]
                        } else {
                            json_str
                        };
                        format!("Failed to parse layout_v3 JSON: {e}\nExtracted: {preview}")
                    })?;

                    log::info!(
                        target: "paddle_vl",
                        "Layout complete: {} boxes (took {:.1}s)",
                        result.boxes.len(),
                        start.elapsed().as_secs_f64()
                    );

                    return Ok(result);
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "layout_v3 timed out after {}s for {image_path}",
                            Self::LAYOUT_TIMEOUT_SECS
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(e) => {
                    return Err(format!("Failed to check layout process status: {e}"));
                }
            }
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
/// Whether an NVIDIA GPU is present (nvidia-smi plus OS inventory).
pub(crate) fn detect_nvidia_gpu() -> bool {
    // Fast path: nvidia-smi -L lists GPUs in ~50-100ms.
    let mut cmd = Command::new("nvidia-smi");
    cmd.arg("-L");
    match command_output_with_timeout(cmd, GPU_PROBE_TIMEOUT, "nvidia-smi -L") {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let has_gpu = !stdout.trim().is_empty() && stdout.contains("GPU");
            if has_gpu {
                log::info!(target: "paddle_vl", "detect_nvidia_gpu: found GPU via nvidia-smi -L");
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
        log::info!(target: "paddle_vl", "detect_nvidia_gpu: found GPU via /proc/driver/nvidia/gpus");
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
            log::info!(target: "paddle_vl", "detect_nvidia_gpu: found GPU via PCI inventory");
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
                log::info!(
                    target: "paddle_vl",
                    "detect_nvidia_gpu: found GPU via {}",
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

pub(crate) fn command_output_with_timeout(
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

/// Like `drain_child_pipe`, but raises `flag` once a line containing `marker` arrives.
fn drain_child_pipe_flagging(
    pipe: impl std::io::Read + Send + 'static,
    marker: &'static str,
    flag: Arc<AtomicBool>,
) -> Vec<u8> {
    let mut reader = std::io::BufReader::new(pipe);
    let mut buffer = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if String::from_utf8_lossy(&line).contains(marker) {
                    flag.store(true, Ordering::SeqCst);
                }
                buffer.extend_from_slice(&line);
            }
        }
    }
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

/// The log source of the engine's setup lines. OCRL builds the shared engine
/// for its layout model only, so its lines must not read as an OCRH run
/// (finding 12).
pub(crate) fn engine_log_source(requested_by: &super::OcrMode) -> &'static str {
    match requested_by {
        super::OcrMode::High => "OCRH",
        super::OcrMode::Light => "OCRL",
    }
}

fn managed_python_line(requested_by: &super::OcrMode, python: &Path) -> String {
    match requested_by {
        super::OcrMode::High => format!(
            "PaddleOCR-VL usará Python del runtime administrado: {}",
            python.display()
        ),
        super::OcrMode::Light => format!(
            "El layout de OCRL usará Python del runtime administrado: {}",
            python.display()
        ),
    }
}

fn incomplete_cache_line(requested_by: &super::OcrMode) -> &'static str {
    match requested_by {
        super::OcrMode::High => {
            "Cache PaddleOCR-VL incompleta; OCRH permitirá descarga online administrada"
        }
        super::OcrMode::Light => {
            "Cache de PaddleX incompleta; el layout de OCRL permitirá descarga online administrada"
        }
    }
}

pub fn create_paddle_vl_engine_result(
    app_handle: &tauri::AppHandle,
    settings_db_path: &std::path::Path,
    requested_by: super::OcrMode,
) -> Result<PaddleVlEngine, String> {
    let log_source = engine_log_source(&requested_by);
    let runtime_root = managed_runtime_root_for_paddle_vl(app_handle)
        .ok()
        .flatten();
    let resource_dir = bundled_resource_dir(app_handle);
    let script_path = resolve_paddle_vl_script_path_from_roots(
        runtime_root.as_deref(),
        resource_dir.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    );
    let app_data_dir = crate::path_utils::cache_dir(app_handle)
        .ok()
        .or_else(|| settings_db_path.parent().map(Path::to_path_buf));
    let (hf_cache_dir, paddlex_cache_dir) =
        resolve_paddle_vl_cache_dirs(runtime_root.as_deref(), app_data_dir.as_deref());
    let offline_mode =
        runtime_root.is_some() && paddle_vl_caches_look_complete(paddlex_cache_dir.as_deref());
    if runtime_root.is_some() && !offline_mode {
        crate::app_logs::warn(app_handle, log_source, incomplete_cache_line(&requested_by));
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
                log_source,
                managed_python_line(&requested_by, &candidate),
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
    // PaddleOCR-VL's own GPU path is Paddle's CUDA build: NVIDIA only. GLM-OCR
    // runs on any GPU llama-server has a build for.
    let has_gpu = detect_nvidia_gpu();
    let desired = resolve_desired_backend_with_gpu(settings_db_path, resource_dir.as_deref())?;
    if let DesiredBackend::PaddleVlServer(assets) = desired {
        if requested_by == super::OcrMode::High {
            crate::app_logs::info(
                app_handle,
                log_source,
                format!(
                    "OCRH local usará PaddleOCR-VL en llama-server ({})",
                    assets.server.display()
                ),
            );
        }
        let config = PaddleVlConfig {
            python_path,
            // The bundled script, never the runtime pack's copy: a pack
            // installed before this backend existed has no --vl-server-url.
            script_path: resolve_paddle_vl_script_path_from_roots(
                None,
                resource_dir.as_deref(),
                Path::new(env!("CARGO_MANIFEST_DIR")),
            ),
            layout_script_path: resolve_ocrh_script_path_from_roots(
                runtime_root.as_deref(),
                resource_dir.as_deref(),
                Path::new(env!("CARGO_MANIFEST_DIR")),
                "layout_v3.py",
            ),
            hf_cache_dir,
            paddlex_cache_dir,
            offline_mode,
            // Layout stays on CPU so Paddle does not preallocate the VRAM
            // llama-server needs.
            device: "cpu".to_string(),
            glm_local: None,
            vl_server: Some(GlmLocalBackend {
                assets,
                db_path: settings_db_path.to_path_buf(),
            }),
        };
        return PaddleVlEngine::init(config)
            .map_err(|e| format!("No se pudo iniciar PaddleOCR-VL en llama-server: {e}"));
    }
    if let DesiredBackend::GlmOcr(assets) = desired {
        // OCRL only runs the layout script; GLM-OCR is not part of its run.
        if requested_by == super::OcrMode::High {
            crate::app_logs::info(
                app_handle,
                log_source,
                format!(
                    "OCRH local usará GLM-OCR (llama-server: {})",
                    assets.server.display()
                ),
            );
        }
        let config = PaddleVlConfig {
            python_path,
            script_path: resolve_ocrh_script_path_from_roots(
                runtime_root.as_deref(),
                resource_dir.as_deref(),
                Path::new(env!("CARGO_MANIFEST_DIR")),
                "glm_ocr_local.py",
            ),
            layout_script_path: resolve_ocrh_script_path_from_roots(
                runtime_root.as_deref(),
                resource_dir.as_deref(),
                Path::new(env!("CARGO_MANIFEST_DIR")),
                "layout_v3.py",
            ),
            hf_cache_dir,
            paddlex_cache_dir,
            offline_mode,
            // Only the layout model runs inside the script, and it runs on CPU.
            device: "cpu".to_string(),
            glm_local: Some(GlmLocalBackend {
                assets,
                db_path: settings_db_path.to_path_buf(),
            }),
            vl_server: None,
        };
        return PaddleVlEngine::init(config)
            .map_err(|e| format!("No se pudo iniciar GLM-OCR local: {e}"));
    }

    let device = if has_gpu {
        log::info!(target: "paddle_vl", "NVIDIA GPU detected — preferring GPU for PaddleOCR-VL");
        "gpu".to_string()
    } else {
        log::info!(target: "paddle_vl", "No NVIDIA GPU detected — using CPU for PaddleOCR-VL");
        "cpu".to_string()
    };

    match PaddleVlEngine::init(PaddleVlConfig {
        python_path,
        script_path,
        layout_script_path: resolve_ocrh_script_path_from_roots(
            runtime_root.as_deref(),
            resource_dir.as_deref(),
            Path::new(env!("CARGO_MANIFEST_DIR")),
            "layout_v3.py",
        ),
        hf_cache_dir,
        paddlex_cache_dir,
        offline_mode,
        device,
        glm_local: None,
        vl_server: None,
    }) {
        Ok(engine) => Ok(engine),
        Err(e) => Err(format!("❌ Failed to create PaddleVLEngine: {e}")),
    }
}

/// Installer-shipped server builds live under the Tauri resource directory
/// (`resources/tools/llama-server`); `None` outside a bundled app (dev).
fn bundled_resource_dir(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app_handle.path().resource_dir().ok()
}

/// Decide whether OCRH local runs on GLM-OCR.
///
/// `glm_ocr` demands it (missing assets is an error); `auto` uses it when the
/// assets are installed and an NVIDIA GPU is present, since GLM-OCR on CPU is
/// far slower than PaddleOCR-VL; anything else keeps PaddleOCR-VL.
fn select_glm_local_assets(
    setting: &str,
    has_gpu: bool,
    assets: Result<super::glm_ocr_local::GlmLocalAssets, String>,
) -> Result<Option<super::glm_ocr_local::GlmLocalAssets>, String> {
    use super::glm_ocr_local::{ENGINE_AUTO, ENGINE_GLM_OCR};
    match setting.trim().to_lowercase().as_str() {
        ENGINE_GLM_OCR => assets.map(Some),
        ENGINE_AUTO => match assets {
            Ok(assets) if has_gpu => Ok(Some(assets)),
            Ok(_) => {
                log::info!(target: "paddle_vl", "GLM-OCR local installed but no NVIDIA GPU; using PaddleOCR-VL");
                Ok(None)
            }
            Err(reason) => {
                log::info!(target: "paddle_vl", "GLM-OCR local unavailable ({reason}); using PaddleOCR-VL");
                Ok(None)
            }
        },
        _ => Ok(None),
    }
}

/// The `ocrh_local_engine` value that asks for PaddleOCR-VL.
const ENGINE_PADDLE_VL: &str = "paddle_vl";

/// Decide whether OCRH local runs PaddleOCR-VL on llama-server.
///
/// Consulted only once GLM-OCR was not chosen. `auto` and `paddle_vl` use it
/// when the GGUF models and the server are installed and the GPU can hold
/// them; otherwise PaddleOCR-VL runs natively.
fn select_vl_server_assets(
    setting: &str,
    gpu_fits: bool,
    assets: Result<super::glm_ocr_local::GlmLocalAssets, String>,
) -> Option<super::glm_ocr_local::GlmLocalAssets> {
    let setting = setting.trim().to_lowercase();
    if setting != super::glm_ocr_local::ENGINE_AUTO && setting != ENGINE_PADDLE_VL {
        return None;
    }
    match assets {
        Ok(assets) if gpu_fits => Some(assets),
        Ok(_) => {
            log::info!(target: "paddle_vl", "PaddleOCR-VL GGUF installed but the GPU cannot hold it; running PaddleOCR-VL natively");
            None
        }
        Err(reason) => {
            log::info!(target: "paddle_vl", "PaddleOCR-VL on llama-server unavailable ({reason}); running PaddleOCR-VL natively");
            None
        }
    }
}

/// The local backend OCRH should run.
#[derive(Clone)]
pub enum DesiredBackend {
    /// PaddleOCR-VL inside the Python subprocess (Paddle, CPU or CUDA).
    PaddleVlNative,
    /// PaddleOCR-VL recognition on a local llama-server.
    PaddleVlServer(super::glm_ocr_local::GlmLocalAssets),
    /// GLM-OCR recognition on a local llama-server.
    GlmOcr(super::glm_ocr_local::GlmLocalAssets),
}

/// Which backend an engine runs, without its assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    PaddleVlNative,
    PaddleVlServer,
    GlmOcr,
}

impl BackendKind {
    /// Whether the backend keeps a llama-server running.
    pub fn uses_server(self) -> bool {
        self != BackendKind::PaddleVlNative
    }
}

impl DesiredBackend {
    pub fn kind(&self) -> BackendKind {
        match self {
            DesiredBackend::PaddleVlNative => BackendKind::PaddleVlNative,
            DesiredBackend::PaddleVlServer(_) => BackendKind::PaddleVlServer,
            DesiredBackend::GlmOcr(_) => BackendKind::GlmOcr,
        }
    }
}

/// Pick the backend from the setting and what is installed. The PaddleOCR-VL
/// checks are lazy: they run only when GLM-OCR was not chosen.
fn select_backend(
    setting: &str,
    glm_gpu: bool,
    glm_assets: Result<super::glm_ocr_local::GlmLocalAssets, String>,
    vl_gpu: impl FnOnce() -> bool,
    vl_assets: impl FnOnce() -> Result<super::glm_ocr_local::GlmLocalAssets, String>,
) -> Result<DesiredBackend, String> {
    if let Some(assets) = select_glm_local_assets(setting, glm_gpu, glm_assets)? {
        return Ok(DesiredBackend::GlmOcr(assets));
    }
    let assets = vl_assets();
    let gpu_fits = assets.is_ok() && vl_gpu();
    Ok(select_vl_server_assets(setting, gpu_fits, assets).map_or(
        DesiredBackend::PaddleVlNative,
        DesiredBackend::PaddleVlServer,
    ))
}

/// Resolve which local backend OCRH should use right now, without building it.
///
/// Reads the `ocrh_local_engine` setting plus a filesystem check for the
/// GLM-OCR and PaddleOCR-VL GGUF assets. An `Err` (the `glm_ocr` setting
/// demanding GLM-OCR whose assets are missing) tells the caller to keep
/// whatever engine is already running instead of thrashing the cache on
/// every task.
///
/// The server binary may ship inside the installer resources, so the Tauri
/// resource directory is consulted alongside the managed install.
pub fn resolve_desired_backend(
    app_handle: &tauri::AppHandle,
    settings_db_path: &Path,
) -> Result<DesiredBackend, String> {
    resolve_desired_backend_with_gpu(
        settings_db_path,
        bundled_resource_dir(app_handle).as_deref(),
    )
}

fn resolve_desired_backend_with_gpu(
    settings_db_path: &Path,
    resource_dir: Option<&Path>,
) -> Result<DesiredBackend, String> {
    use super::llama_gpu::{engine_runs, Engine};
    let engine_setting = crate::db::open::open_archive_connection(settings_db_path)
        .ok()
        .and_then(|conn| {
            crate::settings::get_setting(&conn, super::glm_ocr_local::OCRH_LOCAL_ENGINE_SETTING)
        })
        .unwrap_or_else(|| super::glm_ocr_local::ENGINE_AUTO.to_string());
    select_backend(
        &engine_setting,
        engine_runs(Engine::GlmOcr),
        super::glm_ocr_local::resolve_assets_with_resource_dir(settings_db_path, resource_dir),
        || engine_runs(Engine::PaddleVl),
        || super::paddle_vl_local::resolve_assets_with_resource_dir(settings_db_path, resource_dir),
    )
}

fn resolve_paddle_vl_script_path_from_roots(
    managed_root: Option<&Path>,
    resource_dir: Option<&Path>,
    manifest_dir: &Path,
) -> PathBuf {
    resolve_ocrh_script_path_from_roots(managed_root, resource_dir, manifest_dir, "paddle_vl.py")
}

/// Find an OCRH helper script: the managed runtime first, then the scripts the
/// installer ships under the resource directory, then the source tree (dev).
/// The runtime pack carries only some of them (`layout_v3.py` and
/// `glm_ocr_local.py` come with the installer alone), and `manifest_dir` is the
/// build machine's path in a release, so skipping the resource directory left
/// OCRH and the light-OCR layout without their script.
fn resolve_ocrh_script_path_from_roots(
    managed_root: Option<&Path>,
    resource_dir: Option<&Path>,
    manifest_dir: &Path,
    script_name: &str,
) -> PathBuf {
    if let Some(root) = managed_root {
        let managed = managed_script_path(root, script_name);
        if managed.exists() {
            return managed;
        }
    }
    if let Some(resource_dir) = resource_dir {
        let bundled = resource_dir.join("scripts").join(script_name);
        if bundled.exists() {
            return normalize_windows_path(bundled);
        }
    }

    normalize_windows_path(manifest_dir.join("scripts").join(script_name))
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

    let layout_ready = layout_model.join("inference.yml").is_file()
        && layout_model.join("inference.pdiparams").is_file()
        && (layout_model.join("inference.json").is_file()
            || layout_model.join("inference.pdmodel").is_file());

    // The VL model dir carries the pack's model version (PaddleOCR-VL-1.5,
    // -1.6, ...). Matching any version keeps the completeness check valid as
    // the seeded cache ships newer models; a hardcoded -1.5 made every later
    // pack look incomplete and silently forced online downloads.
    let vl_ready = std::fs::read_dir(&official_models)
        .map(|entries| {
            entries.flatten().any(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let is_vl_dir = name.starts_with("PaddleOCR-VL-");
                if !is_vl_dir {
                    return false;
                }
                let dir = entry.path();
                dir.join("model.safetensors").is_file()
                    || dir.join("model_state.pdparams").is_file()
                    || dir.join("inference.pdparams").is_file()
            })
        })
        .unwrap_or(false);

    layout_ready && vl_ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use std::cell::RefCell;
    use tempfile::tempdir;

    #[test]
    fn a_light_run_logs_the_engine_setup_as_ocrl_not_paddleocr_vl() {
        use crate::ocr::OcrMode;
        let python = Path::new("C:/runtime/python.exe");
        assert_eq!(engine_log_source(&OcrMode::Light), "OCRL");
        let line = managed_python_line(&OcrMode::Light, python);
        assert!(line.contains("OCRL"), "{line}");
        assert!(!line.contains("PaddleOCR-VL"), "{line}");
        assert!(!incomplete_cache_line(&OcrMode::Light).contains("OCRH"));
    }

    #[test]
    fn a_high_run_keeps_its_ocrh_lines() {
        use crate::ocr::OcrMode;
        let python = Path::new("C:/runtime/python.exe");
        assert_eq!(engine_log_source(&OcrMode::High), "OCRH");
        assert_eq!(
            managed_python_line(&OcrMode::High, python),
            "PaddleOCR-VL usará Python del runtime administrado: C:/runtime/python.exe"
        );
        assert_eq!(
            incomplete_cache_line(&OcrMode::High),
            "Cache PaddleOCR-VL incompleta; OCRH permitirá descarga online administrada"
        );
    }

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

        let resolved = resolve_paddle_vl_script_path_from_roots(
            Some(runtime_dir.path()),
            None,
            manifest_dir.path(),
        );

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

        let resolved = resolve_paddle_vl_script_path_from_roots(None, None, manifest_dir.path());

        assert_eq!(resolved, canonical_script);
    }

    #[test]
    fn resolves_installer_layout_script_when_the_runtime_lacks_it() {
        let runtime_dir = tempdir().expect("runtime dir");
        let resource_dir = tempdir().expect("resource dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let runtime_scripts = runtime_dir.path().join("scripts");
        std::fs::create_dir_all(&runtime_scripts).expect("create runtime scripts");
        std::fs::write(runtime_scripts.join("paddle_vl.py"), "print('runtime')")
            .expect("write runtime script");
        let bundled = resource_dir.path().join("scripts").join("layout_v3.py");
        std::fs::create_dir_all(bundled.parent().expect("bundled parent"))
            .expect("create bundled scripts");
        std::fs::write(&bundled, "print('bundled')").expect("write bundled script");

        let layout = resolve_ocrh_script_path_from_roots(
            Some(runtime_dir.path()),
            Some(resource_dir.path()),
            manifest_dir.path(),
            "layout_v3.py",
        );
        let paddle_vl = resolve_ocrh_script_path_from_roots(
            Some(runtime_dir.path()),
            Some(resource_dir.path()),
            manifest_dir.path(),
            "paddle_vl.py",
        );

        assert_eq!(layout, normalize_windows_path(bundled));
        assert_eq!(paddle_vl, runtime_scripts.join("paddle_vl.py"));
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
            .join("PaddleOCR-VL-1.6");
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
                    blocked_reason: None,
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
                    blocked_reason: None,
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
            layout_script_path: PathBuf::from("/tmp/layout_v3.py"),
            hf_cache_dir: None,
            paddlex_cache_dir: None,
            offline_mode: false,
            device: "gpu".to_string(),
            glm_local: None,
            vl_server: None,
        };
        assert_eq!(config.device, "gpu");
    }

    fn glm_assets() -> crate::ocr::glm_ocr_local::GlmLocalAssets {
        crate::ocr::glm_ocr_local::GlmLocalAssets {
            server: PathBuf::from("/x/llama-server"),
            model: PathBuf::from("/x/m.gguf"),
            mmproj: PathBuf::from("/x/p.gguf"),
        }
    }

    #[test]
    fn auto_uses_glm_only_with_assets_and_gpu() {
        assert!(select_glm_local_assets("auto", true, Ok(glm_assets()))
            .unwrap()
            .is_some());
        assert!(select_glm_local_assets("auto", false, Ok(glm_assets()))
            .unwrap()
            .is_none());
        assert!(select_glm_local_assets("auto", true, Err("missing".into()))
            .unwrap()
            .is_none());
    }

    #[test]
    fn forced_glm_reports_missing_assets() {
        assert_eq!(
            select_glm_local_assets("glm_ocr", false, Err("missing".into())).unwrap_err(),
            "missing"
        );
        assert!(select_glm_local_assets("GLM_OCR", false, Ok(glm_assets()))
            .unwrap()
            .is_some());
    }

    #[test]
    fn paddle_vl_setting_never_uses_glm() {
        assert!(select_glm_local_assets("paddle_vl", true, Ok(glm_assets()))
            .unwrap()
            .is_none());
    }

    fn missing() -> Result<crate::ocr::glm_ocr_local::GlmLocalAssets, String> {
        Err("missing".into())
    }

    fn kind_for(
        setting: &str,
        glm: Result<crate::ocr::glm_ocr_local::GlmLocalAssets, String>,
        vl: Result<crate::ocr::glm_ocr_local::GlmLocalAssets, String>,
        vl_gpu: bool,
    ) -> BackendKind {
        select_backend(setting, true, glm, || vl_gpu, || vl)
            .unwrap()
            .kind()
    }

    #[test]
    fn installed_paddle_vl_gguf_runs_on_llama_server() {
        assert_eq!(
            kind_for("auto", missing(), Ok(glm_assets()), true),
            BackendKind::PaddleVlServer
        );
        assert_eq!(
            kind_for("paddle_vl", Ok(glm_assets()), Ok(glm_assets()), true),
            BackendKind::PaddleVlServer
        );
    }

    #[test]
    fn paddle_vl_runs_natively_without_its_gguf_or_a_fitting_gpu() {
        assert_eq!(
            kind_for("auto", missing(), missing(), true),
            BackendKind::PaddleVlNative
        );
        assert_eq!(
            kind_for("paddle_vl", missing(), Ok(glm_assets()), false),
            BackendKind::PaddleVlNative
        );
    }

    #[test]
    fn glm_ocr_still_wins_when_chosen() {
        assert_eq!(
            kind_for("auto", Ok(glm_assets()), Ok(glm_assets()), true),
            BackendKind::GlmOcr
        );
        assert_eq!(
            kind_for("glm_ocr", Ok(glm_assets()), Ok(glm_assets()), true),
            BackendKind::GlmOcr
        );
        assert!(select_backend("glm_ocr", true, missing(), || true, || Ok(glm_assets())).is_err());
    }

    #[test]
    fn an_engine_built_for_the_server_reports_its_backend() {
        let mut config = config_on("cpu");
        config.vl_server = Some(GlmLocalBackend {
            assets: glm_assets(),
            db_path: PathBuf::from("/x/archive.db"),
        });
        let engine = PaddleVlEngine { config };
        assert_eq!(engine.backend_kind(), BackendKind::PaddleVlServer);
        assert!(!engine.uses_glm_local());
        assert!(BackendKind::PaddleVlServer.uses_server());
        assert!(!BackendKind::PaddleVlNative.uses_server());
    }

    fn config_on(device: &str) -> PaddleVlConfig {
        PaddleVlConfig {
            python_path: PathBuf::from("/usr/bin/python"),
            script_path: PathBuf::from("/tmp/paddle_vl.py"),
            layout_script_path: PathBuf::from("/tmp/layout_v3.py"),
            hf_cache_dir: None,
            paddlex_cache_dir: None,
            offline_mode: false,
            device: device.to_string(),
            glm_local: None,
            vl_server: None,
        }
    }

    fn env_of(cmd: &Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(name, _)| *name == key)
            .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
    }

    #[test]
    fn only_linux_runs_cuda_kernels_one_at_a_time() {
        assert!(serializes_cuda_launches_on("linux"));
        // Under WDDM the same setting kept a dense page on an A10G past the
        // 900 s page timeout, while without it 4/4 runs finished in ~190 s.
        assert!(!serializes_cuda_launches_on("windows"));
        assert!(!serializes_cuda_launches_on("macos"));
    }

    #[test]
    fn a_gpu_page_runs_cuda_kernels_one_at_a_time_where_the_host_needs_it() {
        let script = PathBuf::from("/tmp/paddle_vl.py");
        let (gpu, _, _) =
            build_ocrh_command(&config_on("gpu"), &script, "page.png", "gpu", &[]).unwrap();
        let expected = serializes_cuda_launches_on(std::env::consts::OS).then_some("1");
        assert_eq!(env_of(&gpu, "CUDA_LAUNCH_BLOCKING").as_deref(), expected);

        let (cpu, _, _) =
            build_ocrh_command(&config_on("cpu"), &script, "page.png", "cpu", &[]).unwrap();
        assert_eq!(env_of(&cpu, "CUDA_LAUNCH_BLOCKING"), None);
        assert_eq!(env_of(&cpu, "FLAGS_use_mkldnn").as_deref(), Some("1"));
    }

    #[test]
    fn gpu_utilization_is_read_from_the_busiest_gpu() {
        assert_eq!(parse_gpu_utilization("0\n"), Some(0));
        assert_eq!(parse_gpu_utilization("0\n 37 \n"), Some(37));
        assert_eq!(parse_gpu_utilization(""), None);
        assert_eq!(parse_gpu_utilization("[N/A]\n"), None);
    }

    #[test]
    fn a_page_counts_as_stuck_only_after_the_gpu_sat_idle_for_the_whole_window() {
        let window = Duration::from_secs(120);
        let start = Instant::now();
        let mut watch = GpuStallWatch::new(start, window);

        assert!(!watch.observe(Some(0), start + Duration::from_secs(60)));
        // Any work resets the window.
        assert!(!watch.observe(Some(12), start + Duration::from_secs(90)));
        assert!(!watch.observe(Some(0), start + Duration::from_secs(200)));
        assert!(watch.observe(Some(0), start + Duration::from_secs(210)));
    }

    #[test]
    fn an_unreadable_gpu_never_marks_a_page_as_stuck() {
        let window = Duration::from_secs(120);
        let start = Instant::now();
        let mut watch = GpuStallWatch::new(start, window);
        assert!(!watch.observe(None, start + Duration::from_secs(500)));
        assert!(!watch.observe(Some(0), start + Duration::from_secs(600)));
    }

    #[test]
    fn a_stuck_gpu_page_is_retried_once_and_then_reported() {
        let mut calls = 0;
        let ok = run_with_stall_retry(2, |_| {
            calls += 1;
            if calls == 1 {
                Err(PageRunError::GpuStalled("stuck".into()))
            } else {
                Ok("text")
            }
        });
        assert_eq!(ok, Ok("text"));
        assert_eq!(calls, 2);

        let mut calls = 0;
        let err = run_with_stall_retry::<()>(2, |_| {
            calls += 1;
            Err(PageRunError::GpuStalled("stuck again".into()))
        });
        assert_eq!(calls, 2);
        assert!(err.unwrap_err().contains("stuck again"));
    }

    #[test]
    fn an_ordinary_page_failure_is_not_retried() {
        let mut calls = 0;
        let err = run_with_stall_retry::<()>(2, |_| {
            calls += 1;
            Err(PageRunError::Failed("bad image".into()))
        });
        assert_eq!(calls, 1);
        assert_eq!(err, Err("bad image".to_string()));
    }

    #[test]
    fn the_stderr_reader_flags_when_the_page_itself_starts() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let input: &[u8] = b"[paddle_vl] Import done\n[paddle_vl] Processing: page.png\nmore\n";
        let out = drain_child_pipe_flagging(input, PAGE_STARTED_MARKER, flag.clone());
        assert_eq!(out, input);
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));

        let quiet = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        drain_child_pipe_flagging(
            &b"[paddle_vl] Import done\n"[..],
            PAGE_STARTED_MARKER,
            quiet.clone(),
        );
        assert!(!quiet.load(std::sync::atomic::Ordering::SeqCst));
    }
}
