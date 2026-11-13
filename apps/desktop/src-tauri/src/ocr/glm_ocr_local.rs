//! Local GLM-OCR: asset resolution and the persistent `llama-server` it runs on.
//!
//! OCRH's local GLM-OCR backend reuses the PaddleVL subprocess contract
//! (`scripts/glm_ocr_local.py` prints the same sentinel JSON). The script runs
//! layout detection on CPU and sends every box to one `llama-server` process
//! that keeps GLM-OCR loaded on the GPU, so the model is loaded once per app
//! session instead of once per box.
//!
//! The server is started lazily on the first OCRH run, reused while its assets
//! do not change, and stopped on app exit ([`shutdown`]). A server left behind
//! by a crashed session is killed through its pid file on the next start.
//! `PR_SET_PDEATHSIG` is deliberately not used: it fires when the spawning
//! *thread* exits, and tokio recycles idle blocking threads after ~10 s.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Recognition model: Q8_0 decoder. Measured against f16 on scanned press
/// pages, it changes ~0.2% of characters while being 1.5x faster.
pub const MODEL_FILENAME: &str = "GLM-OCR-Q8_0.gguf";
/// Vision projector kept at f16: quantizing the image encoder is what hurts
/// recognition most (Q8 projector tripled the disagreement with f16).
pub const MMPROJ_FILENAME: &str = "GLM-OCR-mmproj-f16.gguf";
/// Folder under the app models directory that holds both GGUF files.
pub const MODELS_SUBDIR: &str = "glm-ocr";
/// Optional override for the `llama-server` binary (dev and custom builds).
pub const SERVER_BINARY_ENV: &str = "ENTROPIA_LLAMA_SERVER";
/// Setting that picks the local OCRH engine: `auto`, `glm_ocr` or `paddle_vl`.
pub const OCRH_LOCAL_ENGINE_SETTING: &str = "ocrh_local_engine";

pub const ENGINE_AUTO: &str = "auto";
pub const ENGINE_GLM_OCR: &str = "glm_ocr";

/// Concurrent requests the server accepts; the script sends this many boxes at once.
pub const PARALLEL_SLOTS: u32 = 4;
/// Context per slot. One box is ~150 prompt tokens plus at most 2048 generated;
/// llama.cpp would otherwise size the KV cache from the model's 131k training
/// context and reserve over 5 GB of VRAM. Output is identical either way.
const CONTEXT_PER_SLOT: u32 = 4096;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const HEALTH_TIMEOUT: Duration = Duration::from_millis(800);
const PID_FILE: &str = "llama-server.pid";

/// Files needed to run local GLM-OCR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlmLocalAssets {
    pub server: PathBuf,
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

/// `<data>/models/glm-ocr`, next to the local LLM models.
pub fn models_dir(db_path: &Path) -> PathBuf {
    data_dir(db_path).join("models").join(MODELS_SUBDIR)
}

/// `<data>/tools/llama-server`, the managed location of the server build.
pub fn server_dir(db_path: &Path) -> PathBuf {
    data_dir(db_path).join("tools").join("llama-server")
}

fn data_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

fn server_binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Resolve every asset, or explain which ones are missing.
pub fn resolve_assets(db_path: &Path) -> Result<GlmLocalAssets, String> {
    let env_server = std::env::var_os(SERVER_BINARY_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    resolve_assets_from(db_path, env_server)
}

fn resolve_assets_from(
    db_path: &Path,
    env_server: Option<PathBuf>,
) -> Result<GlmLocalAssets, String> {
    let server = env_server.unwrap_or_else(|| server_dir(db_path).join(server_binary_name()));
    let models = models_dir(db_path);
    let assets = GlmLocalAssets {
        server,
        model: models.join(MODEL_FILENAME),
        mmproj: models.join(MMPROJ_FILENAME),
    };
    let missing: Vec<String> = [&assets.server, &assets.model, &assets.mmproj]
        .into_iter()
        .filter(|path| !path.is_file())
        .map(|path| path.display().to_string())
        .collect();
    if missing.is_empty() {
        Ok(assets)
    } else {
        Err(format!(
            "GLM-OCR local no está instalado; faltan: {}",
            missing.join(", ")
        ))
    }
}

struct RunningServer {
    child: Child,
    assets: GlmLocalAssets,
    port: u16,
}

static SERVER: OnceLock<Mutex<Option<RunningServer>>> = OnceLock::new();

fn server_slot() -> &'static Mutex<Option<RunningServer>> {
    SERVER.get_or_init(|| Mutex::new(None))
}

/// Return the base URL of a healthy server for `assets`, starting it if needed.
///
/// Blocking: call it from a blocking context (the OCRH subprocess path).
pub fn ensure_server(assets: &GlmLocalAssets, db_path: &Path) -> Result<String, String> {
    let mut slot = server_slot()
        .lock()
        .map_err(|_| "GLM-OCR server lock poisoned".to_string())?;

    if let Some(running) = slot.as_mut() {
        let alive = matches!(running.child.try_wait(), Ok(None));
        if alive && running.assets == *assets && health_ok(running.port) {
            return Ok(base_url(running.port));
        }
        log::warn!(target: "glm_ocr_local", "llama-server not reusable (alive={alive}); restarting");
        stop(running);
        *slot = None;
    } else {
        kill_stale_server(db_path);
    }

    let running = start(assets, db_path)?;
    let url = base_url(running.port);
    *slot = Some(running);
    Ok(url)
}

/// Stop the server, if one is running. Called on app exit.
pub fn shutdown() {
    if let Some(mutex) = SERVER.get() {
        if let Ok(mut slot) = mutex.lock() {
            if let Some(running) = slot.as_mut() {
                stop(running);
            }
            *slot = None;
        }
    }
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn start(assets: &GlmLocalAssets, db_path: &Path) -> Result<RunningServer, String> {
    let port = free_port()?;
    let log_path = data_dir(db_path).join("logs").join("llama-server.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| format!("Cannot create {}: {e}", log_path.display()))?;
    let log_err = log_file
        .try_clone()
        .map_err(|e| format!("Cannot clone log handle: {e}"))?;

    let mut cmd = Command::new(&assets.server);
    cmd.arg("-m")
        .arg(&assets.model)
        .arg("--mmproj")
        .arg(&assets.mmproj)
        .args(["-ngl", "99"])
        .args(["-np", &PARALLEL_SLOTS.to_string()])
        .args(["-c", &(PARALLEL_SLOTS * CONTEXT_PER_SLOT).to_string()])
        .args(["--host", "127.0.0.1"])
        .args(["--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));
    if let Some(dir) = assets.server.parent() {
        // A self-contained build ships libllama/libggml next to the binary.
        cmd.current_dir(dir);
        #[cfg(target_os = "linux")]
        {
            let mut paths = vec![dir.to_path_buf()];
            if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("LD_LIBRARY_PATH", joined);
            }
        }
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    log::info!(
        target: "glm_ocr_local",
        "Starting llama-server on 127.0.0.1:{port} (model={}, mmproj={})",
        assets.model.display(),
        assets.mmproj.display()
    );
    let child = cmd
        .spawn()
        .map_err(|e| format!("Cannot start {}: {e}", assets.server.display()))?;
    let mut running = RunningServer {
        child,
        assets: assets.clone(),
        port,
    };
    write_pid_file(db_path, running.child.id());

    let started = Instant::now();
    while started.elapsed() < STARTUP_TIMEOUT {
        if let Ok(Some(status)) = running.child.try_wait() {
            return Err(format!(
                "llama-server exited during startup ({status}). Log:\n{}",
                log_tail(&log_path)
            ));
        }
        if health_ok(port) {
            log::info!(
                target: "glm_ocr_local",
                "llama-server ready in {:.1}s",
                started.elapsed().as_secs_f32()
            );
            return Ok(running);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    stop(&mut running);
    Err(format!(
        "llama-server did not become healthy within {}s. Log:\n{}",
        STARTUP_TIMEOUT.as_secs(),
        log_tail(&log_path)
    ))
}

fn stop(running: &mut RunningServer) {
    let _ = running.child.kill();
    let _ = running.child.wait();
}

fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Cannot reserve a local port for llama-server: {e}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("Cannot read reserved port: {e}"))
}

/// Plain-TCP `GET /health`: no HTTP client runtime, so it is safe from any thread.
fn health_ok(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, HEALTH_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(HEALTH_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HEALTH_TIMEOUT));
    if stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut head = [0u8; 32];
    match stream.read(&mut head) {
        Ok(n) => is_http_ok(&head[..n]),
        Err(_) => false,
    }
}

fn is_http_ok(status_line: &[u8]) -> bool {
    let line = String::from_utf8_lossy(status_line);
    let mut parts = line.split_whitespace();
    parts.next().is_some_and(|proto| proto.starts_with("HTTP/")) && parts.next() == Some("200")
}

fn log_tail(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|content| {
            let lines: Vec<&str> = content.lines().collect();
            lines[lines.len().saturating_sub(15)..].join("\n")
        })
        .unwrap_or_default()
}

fn pid_file(db_path: &Path) -> PathBuf {
    server_dir(db_path).join(PID_FILE)
}

fn write_pid_file(db_path: &Path, pid: u32) {
    let path = pid_file(db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, pid.to_string()).ok();
}

/// Kill a server a crashed session left running, so it does not hold VRAM.
/// Only a process whose command line still names llama-server is touched.
fn kill_stale_server(db_path: &Path) {
    let path = pid_file(db_path);
    let Some(pid) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
    else {
        return;
    };
    #[cfg(target_os = "linux")]
    {
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        if String::from_utf8_lossy(&cmdline).contains("llama-server") {
            log::warn!(target: "glm_ocr_local", "Killing stale llama-server pid {pid}");
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
    std::fs::remove_file(path).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_assets_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        let error = resolve_assets_from(&db, None).unwrap_err();
        assert!(error.contains(MODEL_FILENAME));
        assert!(error.contains(MMPROJ_FILENAME));
        assert!(error.contains(server_binary_name()));
    }

    #[test]
    fn complete_assets_resolve_under_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        std::fs::create_dir_all(models_dir(&db)).unwrap();
        std::fs::create_dir_all(server_dir(&db)).unwrap();
        std::fs::write(models_dir(&db).join(MODEL_FILENAME), b"m").unwrap();
        std::fs::write(models_dir(&db).join(MMPROJ_FILENAME), b"p").unwrap();
        std::fs::write(server_dir(&db).join(server_binary_name()), b"s").unwrap();

        let assets = resolve_assets_from(&db, None).unwrap();
        assert_eq!(
            assets.model,
            dir.path().join("models/glm-ocr").join(MODEL_FILENAME)
        );
        assert_eq!(
            assets.server,
            dir.path()
                .join("tools/llama-server")
                .join(server_binary_name())
        );
    }

    #[test]
    fn env_override_replaces_the_managed_server_path() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        let custom = dir.path().join("custom-server");
        let error = resolve_assets_from(&db, Some(custom.clone())).unwrap_err();
        assert!(error.contains(&custom.display().to_string()));
    }

    #[test]
    fn only_a_200_status_line_is_healthy() {
        assert!(is_http_ok(b"HTTP/1.1 200 OK\r\n"));
        assert!(!is_http_ok(b"HTTP/1.1 503 Service Unavailable\r\n"));
        assert!(!is_http_ok(b"garbage"));
    }

    #[test]
    fn health_check_fails_on_a_closed_port() {
        let port = free_port().unwrap();
        assert!(!health_ok(port));
    }
}
