//! One persistent `llama-server` per app session, shared by every local-vision
//! feature (OCRH's GLM-OCR, local OCR correction).
//!
//! VRAM fits a single loaded model: [`ensure_server`] restarts the running
//! server when a different model (or different slot flags) is requested, so
//! two features never pin two models at once. Otherwise the lifecycle mirrors
//! the original OCRH manager: lazy start, reuse while the assets match,
//! stop on app exit ([`shutdown`]). A server left behind by a crashed session
//! is killed through its pid file on the next start.
//! `PR_SET_PDEATHSIG` is deliberately not used: it fires when the spawning
//! *thread* exits, and tokio recycles idle blocking threads after ~10 s.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const HEALTH_TIMEOUT: Duration = Duration::from_millis(800);
const PID_FILE: &str = "llama-server.pid";

/// Which local-vision feature owns the running server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerKind {
    Ocrh,
    Correction,
}

/// How to run the server for one feature.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ServerConfig {
    pub kind: ServerKind,
    /// Concurrent requests the server accepts.
    pub slots: u32,
    /// Context per slot. llama.cpp would otherwise size the KV cache from the
    /// model's training context and reserve far more VRAM than needed.
    pub ctx_per_slot: u32,
    /// `log::target` for lifecycle lines.
    pub log_target: &'static str,
    /// Log file under `<data>/logs/`.
    pub log_file: &'static str,
}

/// Files needed to run one local-vision model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerAssets {
    pub server: PathBuf,
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

/// Optional override for the `llama-server` binary (dev and custom builds).
pub(crate) const SERVER_BINARY_ENV: &str = "ENTROPIA_LLAMA_SERVER";

/// `tools/llama-server` inside the bundled installer resources.
pub(crate) const BUNDLED_SERVER_SUBDIR: &str = "tools/llama-server";

/// `<data>/tools/llama-server`, the managed location of the server build.
pub(crate) fn server_dir(db_path: &Path) -> PathBuf {
    data_dir(db_path).join("tools").join("llama-server")
}

pub(crate) fn data_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

pub(crate) fn server_binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

struct RunningServer {
    child: Child,
    kind: ServerKind,
    slots: u32,
    ctx_per_slot: u32,
    assets: ServerAssets,
    port: u16,
}

static SERVER: LazyLock<Mutex<Option<RunningServer>>> = LazyLock::new(|| Mutex::new(None));

/// Return the base URL of a healthy server for `assets`, starting (or
/// restarting) it if needed. A running server for the other feature is
/// stopped first: VRAM fits a single model.
///
/// Blocking: call it from a blocking context (the OCRH subprocess path).
pub(crate) fn ensure_server(
    config: &ServerConfig,
    assets: &ServerAssets,
    db_path: &Path,
) -> Result<String, String> {
    let mut slot = SERVER
        .lock()
        .map_err(|_| "Local model server lock poisoned".to_string())?;

    if let Some(running) = slot.as_mut() {
        let alive = matches!(running.child.try_wait(), Ok(None));
        let reusable = alive
            && running.kind == config.kind
            && running.slots == config.slots
            && running.ctx_per_slot == config.ctx_per_slot
            && running.assets == *assets
            && health_ok(running.port);
        if reusable {
            return Ok(base_url(running.port));
        }
        log::warn!(target: config.log_target, "llama-server not reusable (alive={alive}); restarting");
        stop(running);
        *slot = None;
    } else {
        kill_stale_server(db_path);
    }

    let running = start(config, assets, db_path)?;
    let url = base_url(running.port);
    *slot = Some(running);
    Ok(url)
}

/// Stop the server, if one is running. Called on app exit.
pub(crate) fn shutdown() {
    if let Ok(mut slot) = SERVER.lock() {
        if let Some(running) = slot.as_mut() {
            stop(running);
        }
        *slot = None;
    }
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn start(
    config: &ServerConfig,
    assets: &ServerAssets,
    db_path: &Path,
) -> Result<RunningServer, String> {
    let port = free_port()?;
    let log_path = data_dir(db_path).join("logs").join(config.log_file);
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
        .args(["-np", &config.slots.to_string()])
        .args(["-c", &(config.slots * config.ctx_per_slot).to_string()])
        .args(["--host", "127.0.0.1"])
        .args(["--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    log::info!(
        target: config.log_target,
        "Starting llama-server on 127.0.0.1:{port} (model={}, mmproj={})",
        assets.model.display(),
        assets.mmproj.display()
    );
    let child = cmd
        .spawn()
        .map_err(|e| format!("Cannot start {}: {e}", assets.server.display()))?;
    let mut running = RunningServer {
        child,
        kind: config.kind,
        slots: config.slots,
        ctx_per_slot: config.ctx_per_slot,
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
                target: config.log_target,
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
            log::warn!(target: "local_server", "Killing stale llama-server pid {pid}");
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
