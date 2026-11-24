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
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::llama_gpu::{self, Backend, Gpu, Platform};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const HEALTH_TIMEOUT: Duration = Duration::from_millis(800);
const PID_FILE: &str = "llama-server.pid";

/// Which local feature owns the running server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerKind {
    Ocrh,
    Correction,
    /// The local Gemma text model (chat, intent, rewrite, help).
    Text,
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

/// Files needed to run one local model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerAssets {
    pub server: PathBuf,
    pub model: PathBuf,
    /// The vision projector; `None` for a text-only model.
    pub mmproj: Option<PathBuf>,
}

/// Optional override for the `llama-server` binary (dev and custom builds).
pub(crate) const SERVER_BINARY_ENV: &str = "ENTROPIA_LLAMA_SERVER";

/// Where the installer puts `llama-server`, relative to Tauri's resource dir.
/// The bundle keeps each resource's source path, so the `.deb` entry
/// `resources/tools/llama-server/*` lands under `<resource_dir>/resources/`.
pub(crate) const BUNDLED_SERVER_SUBDIR: &str = "resources/tools/llama-server";

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

/// The `ENTROPIA_LLAMA_SERVER` override, when set to a non-empty path.
pub(crate) fn env_server_override() -> Option<PathBuf> {
    std::env::var_os(SERVER_BINARY_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The server binary to run: the override, else the managed
/// `<data>/tools/llama-server` install, else the installer-bundled build.
/// When none exists it returns the managed path, so "missing" errors name
/// the place a download would fill.
pub(crate) fn resolve_server(
    db_path: &Path,
    env_server: Option<PathBuf>,
    resource_dir: Option<&Path>,
) -> PathBuf {
    env_server.unwrap_or_else(|| {
        let managed = server_dir(db_path).join(server_binary_name());
        if managed.is_file() {
            return managed;
        }
        if let Some(root) = resource_dir {
            let bundled = root.join(BUNDLED_SERVER_SUBDIR).join(server_binary_name());
            if bundled.is_file() {
                return bundled;
            }
        }
        managed
    })
}

/// Where this build gets its `llama-server` from, for the Settings cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerSource {
    /// The installer ships it (the Linux `.deb`'s CUDA build for NVIDIA,
    /// scripts/fetch-llama-server.sh).
    Bundled,
    /// Downloaded on demand, in the build this machine's GPU runs.
    Download,
    /// No GPU the local engines can use (none, or too little memory): both
    /// are GPU-only, so no build is offered.
    RequiresGpu,
}

/// Name of the file in [`server_dir`] that records which build it holds.
/// An install from before the file existed is the CUDA build.
pub(crate) const BACKEND_MARKER: &str = "backend.txt";

/// The build the managed install holds, when there is one.
pub(crate) fn installed_backend(db_path: &Path) -> Option<Backend> {
    let dir = server_dir(db_path);
    if !dir.join(server_binary_name()).is_file() {
        return None;
    }
    match std::fs::read_to_string(dir.join(BACKEND_MARKER)) {
        Ok(text) => Backend::parse(&text),
        Err(_) => Some(Backend::Cuda),
    }
}

/// Whether an installed build serves a machine that wants `wanted`. A HIP
/// install that listed no device fell back to Vulkan, which then stands.
pub(crate) fn backend_satisfies(installed: Backend, wanted: Backend) -> bool {
    installed == wanted || (wanted == Backend::Hip && installed == Backend::Vulkan)
}

/// Whether the managed install holds a build this machine cannot use (the
/// GPU changed since), so the download is offered again.
pub(crate) fn managed_server_stale(db_path: &Path) -> bool {
    match (installed_backend(db_path), llama_gpu::detected()) {
        (Some(installed), Some(gpu)) => !backend_satisfies(installed, gpu.backend),
        _ => false,
    }
}

/// Whether the resolved server is there and runs on this machine's GPU.
pub(crate) fn server_ready(server: &Path, db_path: &Path) -> bool {
    server.is_file()
        && !(server == server_dir(db_path).join(server_binary_name())
            && managed_server_stale(db_path))
}

/// Why `engine` is not offered on this machine: both local vision engines run
/// only on a GPU, NVIDIA or one with enough memory. Shown wherever a download
/// of their models is refused.
pub(crate) fn needs_gpu(engine: llama_gpu::Engine) -> String {
    format!(
        "Necesita una GPU NVIDIA, o AMD, Intel o Apple con al menos {} GB de memoria de \
         video: este equipo no tiene, así que no se descarga. Esta función usa el motor \
         remoto o PaddleOCR-VL.",
        engine.min_vram_gb()
    )
}

/// Refuses a download of `engine`'s models on a machine whose GPU cannot run
/// it (`runs` comes from [`llama_gpu::engine_runs`]).
pub(crate) fn ensure_gpu_for_vision_models(
    engine: llama_gpu::Engine,
    runs: bool,
) -> Result<(), String> {
    if runs {
        Ok(())
    } else {
        Err(needs_gpu(engine))
    }
}

pub(crate) fn server_source() -> ServerSource {
    server_source_for(Platform::current(), llama_gpu::detected().as_ref())
}

fn server_source_for(platform: Platform, gpu: Option<&Gpu>) -> ServerSource {
    match (platform, gpu.map(|gpu| gpu.backend)) {
        // The `.deb` carries the CUDA build; other platforms run what they ship.
        (Platform::Linux, Some(Backend::Cuda)) | (Platform::Other, _) => ServerSource::Bundled,
        (_, Some(_)) => ServerSource::Download,
        (Platform::Linux, None) => ServerSource::Bundled,
        (_, None) => ServerSource::RequiresGpu,
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

/// The longest a restart waits for a chat request in flight: the chat
/// client's own request timeout, past which that request has failed anyway.
const IN_FLIGHT_WAIT: Duration = Duration::from_secs(900);

/// How long the OCR or correction server must sit unused before the chat may
/// take the GPU from it: a document job asks for the server once per page, so
/// a gap between two pages is not the end of the job.
pub(crate) const OCR_IDLE_GRACE: Duration = Duration::from_secs(60);

struct Activity {
    /// Leases held now.
    count: usize,
    /// When the last lease ended (`None`: never used).
    last_end: Option<Instant>,
    /// Bumped by [`InFlight::abort_waits`]: a waiter that sees it change stops.
    aborts: u64,
}

/// Requests running on the server through a [`Lease`]. Leases are only taken
/// while the `SERVER` lock is held, and whoever stops the server holds that
/// lock while it waits for them, so no new lease starts meanwhile; a lease
/// never takes that lock, so the wait cannot deadlock.
pub(crate) struct InFlight {
    state: Mutex<Activity>,
    idle: Condvar,
}

impl InFlight {
    const fn new() -> Self {
        Self {
            state: Mutex::new(Activity {
                count: 0,
                last_end: None,
                aborts: 0,
            }),
            idle: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Activity> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lease(&'static self, base_url: String) -> Lease {
        self.lock().count += 1;
        Lease {
            base_url,
            in_flight: self,
        }
    }

    /// Block until no lease is held, `timeout` passes or
    /// [`InFlight::abort_waits`] runs. `true` when idle.
    fn wait_idle(&self, timeout: Duration) -> bool {
        let state = self.lock();
        let generation = state.aborts;
        let (state, _) = self
            .idle
            .wait_timeout_while(state, timeout, |state| {
                state.count > 0 && state.aborts == generation
            })
            .unwrap_or_else(|e| e.into_inner());
        state.count == 0
    }

    /// Wake every [`InFlight::wait_idle`] now: the app is stopping the server
    /// regardless.
    fn abort_waits(&self) {
        self.lock().aborts += 1;
        self.idle.notify_all();
    }

    /// No lease held, and none ended within `grace` before `now`.
    fn idle_for(&self, grace: Duration, now: Instant) -> bool {
        let state = self.lock();
        state.count == 0
            && state
                .last_end
                .is_none_or(|end| now.saturating_duration_since(end) >= grace)
    }
}

/// Chat requests on a Gemma server: a restart waits for them.
static CHAT_IN_FLIGHT: InFlight = InFlight::new();
/// OCR and correction requests: the chat takes the GPU only once they are
/// over (see [`OCR_IDLE_GRACE`]). Nothing waits on these, so an OCR caller
/// that asks for the server again while holding one cannot block itself.
static OCR_IN_FLIGHT: InFlight = InFlight::new();

/// A server URL plus the promise that the server stays up for this request:
/// hold it until the request is done.
pub(crate) struct Lease {
    base_url: String,
    in_flight: &'static InFlight,
}

impl Lease {
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.in_flight.lock();
        state.count = state.count.saturating_sub(1);
        if state.count == 0 {
            state.last_end = Some(Instant::now());
            drop(state);
            self.in_flight.idle.notify_all();
        }
    }
}

/// Stop `running`, first letting a chat request on a Gemma server finish.
fn stop_when_idle(running: &mut RunningServer, log_target: &str) {
    if running.kind == ServerKind::Text && !CHAT_IN_FLIGHT.wait_idle(IN_FLIGHT_WAIT) {
        log::warn!(target: log_target, "llama-server: stopping it with a chat request still in flight");
    }
    stop(running);
}

/// Return a lease on a healthy server for `assets`, starting (or restarting)
/// it if needed. A running server for another feature is stopped first: VRAM
/// fits a single model. Hold the lease for the whole job: while it lives the
/// chat does not take the GPU.
///
/// Blocking: call it from a blocking context (the OCRH subprocess path).
pub(crate) fn ensure_server(
    config: &ServerConfig,
    assets: &ServerAssets,
    db_path: &Path,
) -> Result<Lease, String> {
    let mut slot = SERVER
        .lock()
        .map_err(|_| "Local model server lock poisoned".to_string())?;
    let url = ensure_in_slot(&mut slot, config, assets, db_path)?;
    Ok(OCR_IN_FLIGHT.lease(url))
}

/// Like [`ensure_server`] for the chat: it never stops a server an OCR or
/// correction job is using, or used within [`OCR_IDLE_GRACE`]. `Ok(None)`
/// means that job holds the GPU. The lease keeps a later OCR request from
/// stopping the server under this one.
pub(crate) fn ensure_server_if_free(
    config: &ServerConfig,
    assets: &ServerAssets,
    db_path: &Path,
) -> Result<Option<Lease>, String> {
    let mut slot = SERVER
        .lock()
        .map_err(|_| "Local model server lock poisoned".to_string())?;
    let held_by = slot
        .as_mut()
        .and_then(|running| matches!(running.child.try_wait(), Ok(None)).then_some(running.kind));
    let others_idle = OCR_IN_FLIGHT.idle_for(OCR_IDLE_GRACE, Instant::now());
    if !may_take_over(held_by, config.kind, others_idle) {
        return Ok(None);
    }
    let url = ensure_in_slot(&mut slot, config, assets, db_path)?;
    Ok(Some(CHAT_IN_FLIGHT.lease(url)))
}

/// Whether a request for `wanted` may start or reuse the server while a live
/// one of `held_by` runs (`None`: no live server); `others_idle` says the
/// other feature has had no request in flight for a while.
fn may_take_over(held_by: Option<ServerKind>, wanted: ServerKind, others_idle: bool) -> bool {
    match held_by {
        None => true,
        Some(kind) => kind == wanted || others_idle,
    }
}

fn ensure_in_slot(
    slot: &mut Option<RunningServer>,
    config: &ServerConfig,
    assets: &ServerAssets,
    db_path: &Path,
) -> Result<String, String> {
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
        stop_when_idle(running, config.log_target);
        *slot = None;
    } else {
        kill_stale_server(db_path);
    }

    let running = start(config, assets, db_path)?;
    let url = base_url(running.port);
    *slot = Some(running);
    Ok(url)
}

/// Stop the server, if one is running, without waiting for a chat request in
/// flight (it falls back to CPU). Called on app exit, on an OCR engine switch
/// and when a new llama-server build replaces the installed one.
pub(crate) fn shutdown() {
    // A restart may hold the lock while it waits for a chat request; wake it
    // until the lock is ours, so exit does not hang behind that wait.
    let mut slot = loop {
        CHAT_IN_FLIGHT.abort_waits();
        match SERVER.try_lock() {
            Ok(slot) => break slot,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => break poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    if let Some(running) = slot.as_mut() {
        stop(running);
    }
    *slot = None;
}

/// Stop the server if it runs a model from `dir`. Windows refuses to delete a
/// file another process holds open, and the server keeps both GGUF files open
/// while it lives; it is started again on the next run that needs it.
pub(crate) fn stop_if_serving_from(dir: &Path) {
    if let Ok(mut slot) = SERVER.lock() {
        let serving = slot
            .as_ref()
            .is_some_and(|running| running.assets.model.starts_with(dir));
        if serving {
            if let Some(running) = slot.as_mut() {
                stop_when_idle(running, "ocr");
            }
            *slot = None;
        }
    }
}

/// Delete the named model files in `dir`, and the partial download of each,
/// returning the bytes freed. Files that are not there are skipped, so a second
/// call, or a folder that was never created, frees nothing and is not an error.
/// Only the listed names are touched: the folder and anything else in it stay.
pub(crate) fn delete_model_files(dir: &Path, filenames: &[&str]) -> Result<u64, String> {
    let mut freed = 0;
    for filename in filenames {
        let partial = Path::new(filename).with_extension("download.tmp");
        for path in [dir.join(filename), dir.join(partial)] {
            let size = match std::fs::metadata(&path) {
                Ok(meta) if meta.is_file() => meta.len(),
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("No se pudo leer {}: {e}", path.display())),
            };
            std::fs::remove_file(&path)
                .map_err(|e| format!("No se pudo borrar {}: {e}", path.display()))?;
            freed += size;
        }
    }
    Ok(freed)
}

/// The model and serving flags for one start: every layer on the GPU, the
/// projector only for a vision model.
fn server_args(config: &ServerConfig, assets: &ServerAssets, port: u16) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["-m".into(), assets.model.clone().into()];
    if let Some(mmproj) = &assets.mmproj {
        args.push("--mmproj".into());
        args.push(mmproj.clone().into());
    }
    for arg in [
        "-ngl".to_string(),
        "99".to_string(),
        "-np".to_string(),
        config.slots.to_string(),
        "-c".to_string(),
        (config.slots * config.ctx_per_slot).to_string(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
    ] {
        args.push(arg.into());
    }
    args
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
    cmd.args(server_args(config, assets, port));
    if let Some(device) = managed_device(&assets.server, db_path) {
        log::info!(target: config.log_target, "llama-server device: {device}");
        cmd.args(["--device", &device]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));
    // llama-server reads these as its own flags. A user's `LLAMA_API_KEY`
    // (set for some other Llama tool) made it demand a key the app never
    // sends: every local correction answered HTTP 401. The app owns this
    // server's configuration, so none of them may reach it.
    for name in llama_server_env_overrides(std::env::vars_os().map(|(name, _)| name)) {
        cmd.env_remove(name);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    log::info!(
        target: config.log_target,
        "Starting llama-server on 127.0.0.1:{port} (model={}, mmproj={})",
        assets.model.display(),
        assets
            .mmproj
            .as_deref()
            .map_or_else(|| "none".to_string(), |path| path.display().to_string())
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

/// The device a managed Vulkan or HIP build must use: the GPU
/// [`llama_gpu::detected`] chose. llama.cpp would otherwise split the model
/// across every device it sees, an integrated GPU included. `None` leaves
/// the choice to llama.cpp (CUDA, Metal, bundled or overridden servers).
fn managed_device(server: &Path, db_path: &Path) -> Option<String> {
    if server != server_dir(db_path).join(server_binary_name()) {
        return None;
    }
    let backend = installed_backend(db_path)?;
    if !matches!(backend, Backend::Vulkan | Backend::Hip) {
        return None;
    }
    let listing = list_devices(server).ok()?;
    let gpu = llama_gpu::detected();
    llama_gpu::pick_device(
        &listing,
        backend,
        gpu.as_ref().and_then(|gpu| gpu.name.as_deref()),
    )
}

/// `llama-server --list-devices`, stdout and stderr together.
pub(crate) fn list_devices(server: &Path) -> Result<String, String> {
    let mut cmd = Command::new(server);
    cmd.arg("--list-devices").stdin(Stdio::null());
    if let Some(dir) = server.parent() {
        cmd.current_dir(dir);
    }
    for name in llama_server_env_overrides(std::env::vars_os().map(|(name, _)| name)) {
        cmd.env_remove(name);
    }
    let output = super::paddle_vl::command_output_with_timeout(
        cmd,
        Duration::from_secs(60),
        "llama-server --list-devices",
    )?;
    Ok(format!(
        "{}
{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// The inherited variables llama-server would read as configuration:
/// `LLAMA_API_KEY` and every `LLAMA_ARG_*` (port, context, host...).
fn llama_server_env_overrides(
    names: impl Iterator<Item = std::ffi::OsString>,
) -> Vec<std::ffi::OsString> {
    names
        .filter(|name| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            name == "LLAMA_API_KEY" || name.starts_with("LLAMA_ARG_")
        })
        .collect()
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
    fn args_of(assets: &super::ServerAssets) -> Vec<String> {
        let config = super::ServerConfig {
            kind: super::ServerKind::Text,
            slots: 1,
            ctx_per_slot: 16384,
            log_target: "test",
            log_file: "test.log",
        };
        super::server_args(&config, assets, 8791)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_chat_never_takes_the_server_from_another_feature() {
        use super::{may_take_over, ServerKind};
        assert!(may_take_over(None, ServerKind::Text, false));
        assert!(may_take_over(
            Some(ServerKind::Text),
            ServerKind::Text,
            false
        ));
        assert!(!may_take_over(
            Some(ServerKind::Ocrh),
            ServerKind::Text,
            false
        ));
        assert!(!may_take_over(
            Some(ServerKind::Correction),
            ServerKind::Text,
            false
        ));
    }

    #[test]
    fn the_chat_takes_the_gpu_back_once_ocr_is_over() {
        use super::{may_take_over, ServerKind};
        assert!(may_take_over(
            Some(ServerKind::Ocrh),
            ServerKind::Text,
            true
        ));
        assert!(may_take_over(
            Some(ServerKind::Correction),
            ServerKind::Text,
            true
        ));
    }

    #[test]
    fn a_text_model_starts_without_a_projector() {
        let args = args_of(&super::ServerAssets {
            server: "llama-server".into(),
            model: "gemma.gguf".into(),
            mmproj: None,
        });
        assert!(!args.iter().any(|arg| arg == "--mmproj"), "{args:?}");
        assert_eq!(
            args,
            [
                "-m",
                "gemma.gguf",
                "-ngl",
                "99",
                "-np",
                "1",
                "-c",
                "16384",
                "--host",
                "127.0.0.1",
                "--port",
                "8791"
            ]
        );
    }

    #[test]
    fn a_vision_model_still_passes_its_projector() {
        let args = args_of(&super::ServerAssets {
            server: "llama-server".into(),
            model: "glm.gguf".into(),
            mmproj: Some("mmproj.gguf".into()),
        });
        assert_eq!(&args[..4], ["-m", "glm.gguf", "--mmproj", "mmproj.gguf"]);
    }

    #[test]
    fn deleting_models_removes_only_the_named_files_and_their_partial_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models").join("qwen");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("model.gguf"), vec![0u8; 30]).unwrap();
        std::fs::write(models.join("mmproj.gguf"), vec![0u8; 12]).unwrap();
        std::fs::write(models.join("model.download.tmp"), vec![0u8; 5]).unwrap();
        std::fs::write(models.join("notes.txt"), b"kept").unwrap();

        let freed = super::delete_model_files(&models, &["model.gguf", "mmproj.gguf"]).unwrap();

        assert_eq!(freed, 47);
        assert!(!models.join("model.gguf").exists());
        assert!(!models.join("mmproj.gguf").exists());
        assert!(!models.join("model.download.tmp").exists());
        assert!(models.join("notes.txt").is_file());

        // Nothing left to delete is not an error, and a missing folder neither.
        assert_eq!(
            super::delete_model_files(&models, &["model.gguf", "mmproj.gguf"]).unwrap(),
            0
        );
        assert_eq!(
            super::delete_model_files(&dir.path().join("absent"), &["model.gguf"]).unwrap(),
            0
        );
    }

    #[test]
    fn vision_models_are_refused_without_a_gpu_that_runs_them() {
        use super::llama_gpu::Engine;
        assert_eq!(
            super::ensure_gpu_for_vision_models(Engine::GlmOcr, false),
            Err(
                "Necesita una GPU NVIDIA, o AMD, Intel o Apple con al menos 4 GB de memoria \
                 de video: este equipo no tiene, así que no se descarga. Esta función usa el \
                 motor remoto o PaddleOCR-VL."
                    .to_string()
            )
        );
        assert!(super::ensure_gpu_for_vision_models(Engine::Qwen, false)
            .unwrap_err()
            .contains("al menos 6 GB"));
        assert_eq!(
            super::ensure_gpu_for_vision_models(Engine::GlmOcr, true),
            Ok(())
        );
        assert_eq!(
            super::ensure_gpu_for_vision_models(Engine::Qwen, true),
            Ok(())
        );
    }

    use super::*;

    fn leaked() -> &'static InFlight {
        Box::leak(Box::new(InFlight::new()))
    }

    #[test]
    fn a_restart_waits_for_the_request_in_flight() {
        let in_flight = leaked();
        let lease = in_flight.lease("http://127.0.0.1:1".into());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let idle = in_flight.wait_idle(Duration::from_secs(30));
            done_tx.send(idle).unwrap();
        });
        // Still blocked while the request runs.
        assert!(done_rx.recv_timeout(Duration::from_millis(200)).is_err());
        drop(lease);
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)), Ok(true));
        waiter.join().unwrap();
    }

    #[test]
    fn every_lease_must_end_before_the_server_is_idle() {
        let in_flight = leaked();
        let first = in_flight.lease("u".into());
        let second = in_flight.lease("u".into());
        drop(first);
        assert!(!in_flight.wait_idle(Duration::from_millis(50)));
        drop(second);
        assert!(in_flight.wait_idle(Duration::from_millis(50)));
    }

    #[test]
    fn an_idle_server_stops_without_waiting() {
        let started = Instant::now();
        assert!(leaked().wait_idle(Duration::from_secs(30)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_stuck_request_does_not_block_the_restart_forever() {
        let in_flight = leaked();
        let _lease = in_flight.lease("u".into());
        assert!(!in_flight.wait_idle(Duration::from_millis(50)));
    }

    #[test]
    fn closing_the_app_does_not_wait_for_the_chat_request() {
        let in_flight = leaked();
        let _lease = in_flight.lease("u".into());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            done_tx
                .send(in_flight.wait_idle(Duration::from_secs(30)))
                .unwrap();
        });
        assert!(done_rx.recv_timeout(Duration::from_millis(200)).is_err());
        in_flight.abort_waits();
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)), Ok(false));
        waiter.join().unwrap();
    }

    #[test]
    fn ocr_counts_as_busy_while_a_job_runs_and_shortly_after() {
        let in_flight = leaked();
        let grace = Duration::from_secs(60);
        assert!(in_flight.idle_for(grace, Instant::now()), "never used");
        let lease = in_flight.lease("u".into());
        assert!(!in_flight.idle_for(grace, Instant::now() + grace * 2));
        drop(lease);
        // Between two pages of the same job.
        assert!(!in_flight.idle_for(grace, Instant::now()));
        // The job is over.
        assert!(in_flight.idle_for(grace, Instant::now() + grace));
    }

    #[test]
    fn llama_server_never_inherits_its_own_configuration_variables() {
        let names = [
            "LLAMA_API_KEY",
            "LLAMA_ARG_PORT",
            "llama_arg_ctx_size",
            "LLAMA_BASE_URL",
            "OPENROUTER_API_KEY",
            "PATH",
        ]
        .map(std::ffi::OsString::from);
        assert_eq!(
            llama_server_env_overrides(names.into_iter()),
            ["LLAMA_API_KEY", "LLAMA_ARG_PORT", "llama_arg_ctx_size"].map(std::ffi::OsString::from)
        );
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

    fn gpu(backend: Backend) -> Gpu {
        Gpu {
            backend,
            vram: None,
            name: None,
        }
    }

    #[test]
    fn each_platform_gets_its_server_from_the_right_place() {
        let cuda = gpu(Backend::Cuda);
        let vulkan = gpu(Backend::Vulkan);
        assert_eq!(
            server_source_for(Platform::Windows, Some(&cuda)),
            ServerSource::Download
        );
        assert_eq!(
            server_source_for(Platform::Windows, Some(&vulkan)),
            ServerSource::Download
        );
        assert_eq!(
            server_source_for(Platform::Windows, None),
            ServerSource::RequiresGpu
        );
        // The .deb keeps its CUDA build; other Linux GPUs download theirs.
        assert_eq!(
            server_source_for(Platform::Linux, Some(&cuda)),
            ServerSource::Bundled
        );
        assert_eq!(
            server_source_for(Platform::Linux, Some(&vulkan)),
            ServerSource::Download
        );
        assert_eq!(
            server_source_for(Platform::MacArm, Some(&gpu(Backend::Metal))),
            ServerSource::Download
        );
        assert_eq!(
            server_source_for(Platform::MacArm, None),
            ServerSource::RequiresGpu
        );
    }

    #[test]
    fn the_marker_names_the_installed_build_and_legacy_installs_are_cuda() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.db");
        assert_eq!(installed_backend(&db), None);
        std::fs::create_dir_all(server_dir(&db)).unwrap();
        std::fs::write(server_dir(&db).join(server_binary_name()), b"s").unwrap();
        assert_eq!(installed_backend(&db), Some(Backend::Cuda));
        std::fs::write(server_dir(&db).join(BACKEND_MARKER), "vulkan").unwrap();
        assert_eq!(installed_backend(&db), Some(Backend::Vulkan));
    }

    #[test]
    fn a_hip_machine_keeps_the_vulkan_fallback_but_cuda_does_not_serve_vulkan() {
        assert!(backend_satisfies(Backend::Vulkan, Backend::Hip));
        assert!(backend_satisfies(Backend::Hip, Backend::Hip));
        assert!(!backend_satisfies(Backend::Cuda, Backend::Vulkan));
        assert!(!backend_satisfies(Backend::Vulkan, Backend::Cuda));
        assert!(!backend_satisfies(Backend::Hip, Backend::Vulkan));
    }
}
