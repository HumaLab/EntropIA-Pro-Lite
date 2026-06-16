//! Sync engine (DESIGN §3.1, PROTOCOL "Flujo de sincronización del cliente").
//!
//! A SINGLE long-lived tokio task owns the sync connection in exclusive
//! ownership and drives the full cycle. Callers (the interval ticker, startup,
//! `sync_now`, and backoff retries) talk to it ONLY through an mpsc channel; the
//! task coalesces concurrent requests into AT MOST one pending run (single-flight,
//! DESIGN §3.1). `sync_now` returns immediately with the current snapshot; a
//! successful manual run cancels any pending offline backoff.
//!
//! Gated start (DESIGN §3.1): the engine refuses to run a cycle until BOTH
//! `sync_ensure_capture` has been invoked (the frontend signals JS migrations are
//! done) AND a session exists (`device_id` present). Until then every request is a
//! no-op that simply re-emits the current (`disabled`) status.
//!
//! The cycle order mirrors PROTOCOL exactly: read `schema_tag` fresh from the
//! `_migrations` head, check the server epoch, drain inherited queues, seed if a
//! new account, push, pull (which itself drains blobs + FTS at the end), then emit
//! `sync:status`. `schema_tag` is NEVER cached between cycles.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::sync::apply::{retry_pending_rows, ApplyContext};
use crate::sync::blobs::{drain_pending_blobs, prepare_asset_push, AssetPushOutcome};
use crate::sync::http::{HttpSyncApi, SyncApi, SyncError};
use crate::sync::pull::{pull_loop, seed_account};
use crate::sync::push::{
    apply_push_results, batching, build_changes, clock_offset, coalesce_ops,
    journal_and_purge_oversized, snapshot_oplog, split_into_batches, update_clock_offset,
    DEFAULT_MAX_PUSH_BYTES,
};
use crate::sync::session::{meta_get, meta_get_i64, meta_set_i64, read_token};

const LOG_SOURCE: &str = "sync/engine";

/// The Tauri event name carrying every status transition + cycle end (DESIGN §11).
pub const STATUS_EVENT: &str = "sync:status";

/// Default auto-sync interval when `sync_meta['auto_sync_interval_min']` is unset
/// (DESIGN §3.1: default on, 5 min).
const DEFAULT_INTERVAL_MIN: i64 = 5;
/// Initial offline backoff (DESIGN §11: 30 s → 5 min cap).
const BACKOFF_MIN: Duration = Duration::from_secs(30);
/// Maximum offline backoff (DESIGN §11: 5 min cap).
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// The clock-offset warning threshold (PROTOCOL "Reloj": |offset| > 5 min).
const CLOCK_WARNING_MS: i64 = 5 * 60 * 1000;

// ---------------------------------------------------------------------------
// Status (the `sync:status` event payload + the `sync_status` command snapshot)
// ---------------------------------------------------------------------------

/// The sync engine state machine (DESIGN §11, PROTOCOL flow). `disabled` renders
/// nothing in the UI (opt-in); the rest map to the status indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncState {
    /// No session / capture not yet ensured — the engine is dormant.
    Disabled,
    /// Session active, nothing in flight.
    Idle,
    /// A cycle is running right now.
    Syncing,
    /// A network error occurred; the engine is in exponential backoff.
    Offline,
    /// A 4xx the user must act on (426/clock_skew/507): no auto-retry.
    Error,
}

/// The full `sync:status` payload (DESIGN §11, PROTOCOL step 9). Emitted on every
/// transition and at the end of every cycle. Also returned verbatim by the
/// `sync_status` command for UI bootstrap.
#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub state: SyncState,
    /// Last successful sync, ms since epoch (`sync_meta['last_sync_at']`); `None`
    /// until the first successful cycle.
    pub last_sync_at: Option<i64>,
    /// Coalesced count of dirty rows awaiting push (distinct `(table, row_id)`).
    pub pending: i64,
    /// Rows awaiting blob download (`COUNT(sync_pending_blobs)`).
    pub blobs_pending: i64,
    /// Estimated bytes of own blobs not yet uploaded (sum of `sync_blob_index.size`
    /// where `uploaded = 0`). Drives the first-sync preflight in the UI (DESIGN §11:
    /// confirm when pending blob bytes exceed the threshold). A free estimate — no
    /// filesystem stat, just the cached sizes.
    pub pending_blob_bytes: i64,
    /// Unacknowledged conflicts.
    pub conflicts: i64,
    /// True when |clock offset| exceeds 5 min (PROTOCOL "Reloj"; non-blocking).
    pub clock_warning: bool,
    /// Human message for the `Error` state (426/clock_skew/507 mapping, DESIGN §11).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl SyncStatus {
    fn disabled() -> Self {
        Self {
            state: SyncState::Disabled,
            last_sync_at: None,
            pending: 0,
            blobs_pending: 0,
            pending_blob_bytes: 0,
            conflicts: 0,
            clock_warning: false,
            message: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Engine handle (held in Tauri managed state; consumed by the commands)
// ---------------------------------------------------------------------------

/// Requests the engine task accepts over its mpsc channel (DESIGN §3.1).
#[derive(Debug, Clone, Copy)]
pub enum SyncRequest {
    /// A manual `sync_now` or a backoff retry: run a cycle as soon as possible.
    SyncNow,
    /// The interval ticker fired: run a cycle if auto-sync is enabled.
    Tick,
    /// App shutdown: stop the task.
    Shutdown,
}

/// Handle to the running engine, stored in Tauri managed state. Holds a cloneable
/// sender plus a shared status snapshot the `sync_status` command reads without
/// touching the connection.
pub struct SyncEngine {
    sender: mpsc::Sender<SyncRequest>,
    status: Arc<Mutex<SyncStatus>>,
}

impl SyncEngine {
    /// Sends a request, coalescing naturally: the bounded channel + the task's
    /// drain-all loop mean a burst of `SyncNow` collapses into one pending run.
    /// A full channel (a run already queued) is treated as success — the pending
    /// run already covers this request (single-flight, DESIGN §3.1).
    pub fn request(&self, req: SyncRequest) {
        match self.sender.try_send(req) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                // A run is already queued — coalesced, nothing to do.
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // The engine task is gone (shutdown); ignore.
            }
        }
    }

    /// Signals the engine task to stop (DESIGN §3.1). Sent on app exit so the
    /// engine thread tears down cleanly instead of being killed mid-cycle. Uses a
    /// blocking send so the shutdown is not dropped when a run is queued.
    pub fn shutdown(&self) {
        // Best-effort: if the task already exited the channel is closed.
        let _ = self.sender.try_send(SyncRequest::Shutdown);
    }

    /// A snapshot of the current status for the `sync_status` command.
    pub fn snapshot(&self) -> SyncStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawns the engine + the interval ticker and returns the handle (DESIGN §3.1).
/// The engine runs on a DEDICATED OS thread with a current-thread tokio runtime:
/// `rusqlite::Connection` is `Send` but not `Sync`, so a future that holds
/// `&Connection` across an `.await` (the whole cycle does) is not `Send` and
/// cannot go on the shared multi-thread runtime — a single-threaded runtime drops
/// the `Send` requirement. The engine is dormant until gated (capture ensured +
/// session exists); the first request after the gate opens runs a startup cycle.
/// Called from `setup()`.
pub fn start_engine(app_handle: AppHandle, db_path: PathBuf) -> SyncEngine {
    // A small bounded channel: capacity 1 plus the in-flight slot gives the
    // single-flight "at most one pending run" semantics for free.
    let (sender, receiver) = mpsc::channel::<SyncRequest>(1);
    let status = Arc::new(Mutex::new(SyncStatus::disabled()));

    // Interval ticker: a separate task that pokes the engine. The engine decides
    // (per cycle, from sync_meta) whether auto-sync is on and at what cadence; the
    // ticker just fires a coarse heartbeat the engine can ignore. It touches no
    // connection, so it lives on the shared runtime.
    let tick_sender = sender.clone();
    tauri::async_runtime::spawn(async move {
        // Startup nudge so the first cycle runs once the gate opens.
        let _ = tick_sender.try_send(SyncRequest::Tick);
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if tick_sender.try_send(SyncRequest::Tick).is_err() && tick_sender.is_closed() {
                break;
            }
        }
    });

    // The engine keeps a sender clone so backoff retries can poke itself.
    let self_sender = sender.clone();
    let engine_status = Arc::clone(&status);
    let spawned = std::thread::Builder::new()
        .name("sync-engine".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(error) => {
                    eprintln!("[sync] engine failed to build its runtime: {error}");
                    return;
                }
            };
            runtime.block_on(engine_loop(
                app_handle,
                db_path,
                receiver,
                self_sender,
                engine_status,
            ));
        });
    if let Err(error) = spawned {
        eprintln!("[sync] failed to spawn engine thread: {error}");
    }

    SyncEngine { sender, status }
}

// ---------------------------------------------------------------------------
// The single long-lived task
// ---------------------------------------------------------------------------

/// The engine task body. Owns the sync connection for its entire lifetime. Loops:
/// receive a request, drain any extra queued requests (coalesce), run a cycle when
/// gated + due, manage backoff, and persist the resulting status snapshot.
async fn engine_loop(
    app_handle: AppHandle,
    db_path: PathBuf,
    mut receiver: mpsc::Receiver<SyncRequest>,
    self_sender: mpsc::Sender<SyncRequest>,
    status: Arc<Mutex<SyncStatus>>,
) {
    // The engine's OWN connection (never ui_conn / worker_conn, DESIGN §3).
    let conn = match crate::sync::open_sync_connection(&db_path) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("[sync] engine failed to open its connection: {error}");
            return;
        }
    };

    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("[sync] engine failed to resolve app_data_dir: {error}");
            return;
        }
    };

    eprintln!("[sync] engine task ready (gated until capture + session)");

    let mut backoff = BACKOFF_MIN;
    // Tracks elapsed minutes so the coarse 60 s ticker honors the configured
    // auto-sync interval without a second timer.
    let mut last_auto_run = std::time::Instant::now()
        .checked_sub(Duration::from_secs(3600))
        .unwrap_or_else(std::time::Instant::now);

    while let Some(first) = receiver.recv().await {
        // Coalesce: drain everything already queued so a burst is one run.
        let mut manual = matches!(first, SyncRequest::SyncNow);
        let mut shutdown = matches!(first, SyncRequest::Shutdown);
        while let Ok(extra) = receiver.try_recv() {
            match extra {
                SyncRequest::SyncNow => manual = true,
                SyncRequest::Shutdown => shutdown = true,
                SyncRequest::Tick => {}
            }
        }
        if shutdown {
            eprintln!("[sync] engine shutting down");
            break;
        }

        // Gate (DESIGN §3.1): refuse to run until capture ensured + session.
        if !is_gated_open(&conn) {
            publish(&app_handle, &status, &conn, SyncState::Disabled, None);
            continue;
        }

        // A Tick only runs when auto-sync is enabled AND the interval elapsed; a
        // manual SyncNow always runs and cancels backoff.
        if !manual {
            let interval = auto_interval(&conn);
            let auto_on = auto_enabled(&conn);
            if !auto_on || last_auto_run.elapsed() < interval {
                continue;
            }
        }

        publish(&app_handle, &status, &conn, SyncState::Syncing, None);

        let api = match build_api(&conn) {
            Ok(api) => api,
            Err(error) => {
                crate::app_logs::warn(&app_handle, LOG_SOURCE, format!("Config inválida: {error}"));
                publish(&app_handle, &status, &conn, SyncState::Error, Some(error));
                continue;
            }
        };

        // The device token lives ONLY in the keyring (DESIGN §8); read it once per
        // cycle and pass it into the cycle (never logged).
        let token = match read_token() {
            Ok(Some(token)) => token,
            Ok(None) => {
                publish(
                    &app_handle,
                    &status,
                    &conn,
                    SyncState::Error,
                    Some("No hay token de dispositivo en el keyring".to_string()),
                );
                continue;
            }
            Err(error) => {
                publish(&app_handle, &status, &conn, SyncState::Error, Some(error));
                continue;
            }
        };

        let warn_handle = app_handle.clone();
        let warn = move |msg: String| {
            crate::app_logs::warn(&warn_handle, LOG_SOURCE, msg);
        };
        match run_cycle(&api, &token, &conn, &app_data_dir, &warn).await {
            Ok(()) => {
                // A successful run resets backoff (cancels any pending escalation).
                backoff = BACKOFF_MIN;
                last_auto_run = std::time::Instant::now();
                publish(&app_handle, &status, &conn, SyncState::Idle, None);
            }
            Err(CycleError::Offline(detail)) => {
                crate::app_logs::warn(
                    &app_handle,
                    LOG_SOURCE,
                    format!(
                        "Sin conexión, reintento en {}s: {detail}",
                        backoff.as_secs()
                    ),
                );
                publish(&app_handle, &status, &conn, SyncState::Offline, None);
                schedule_backoff_retry(self_sender.clone(), backoff);
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
            Err(CycleError::Fatal { message }) => {
                crate::app_logs::warn(&app_handle, LOG_SOURCE, format!("Error de sync: {message}"));
                publish(&app_handle, &status, &conn, SyncState::Error, Some(message));
                // No auto-retry: stays in Error until the user acts (DESIGN §11).
            }
        }
    }
}

/// Schedules a delayed self-poke after a network failure (DESIGN §11 backoff).
/// Sends `SyncNow` so the retry bypasses the auto-sync interval gate; a successful
/// retry resets the backoff to its floor in the engine loop. A full channel means
/// a run is already queued, so the retry is redundant and dropped.
fn schedule_backoff_retry(sender: mpsc::Sender<SyncRequest>, delay: Duration) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let _ = sender.try_send(SyncRequest::SyncNow);
    });
}

// ---------------------------------------------------------------------------
// Gate + config reads
// ---------------------------------------------------------------------------

/// True when the engine may run a cycle: capture ensured (triggers installed) AND
/// a session exists (`device_id` present) (DESIGN §3.1). `triggers_version` is set
/// by `ensure_capture`; `device_id` by `sync_login`.
fn is_gated_open(conn: &Connection) -> bool {
    let captured = meta_get(conn, "triggers_version").ok().flatten().is_some();
    let session = meta_get(conn, "device_id").ok().flatten().is_some();
    captured && session
}

/// Auto-sync enabled flag (DESIGN §3.1: default ON when unset).
fn auto_enabled(conn: &Connection) -> bool {
    meta_get(conn, "auto_sync_enabled")
        .ok()
        .flatten()
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Configured auto-sync interval (DESIGN §3.1: default 5 min, floor 1 min).
fn auto_interval(conn: &Connection) -> Duration {
    let mins = meta_get_i64(conn, "auto_sync_interval_min")
        .ok()
        .filter(|m| *m > 0)
        .unwrap_or(DEFAULT_INTERVAL_MIN)
        .max(1);
    Duration::from_secs((mins as u64) * 60)
}

/// Builds the HTTP API client from the persisted `server_url` (TLS re-validated in
/// `HttpSyncApi::new`). A missing/invalid URL is a fatal config error.
fn build_api(conn: &Connection) -> Result<HttpSyncApi, String> {
    let url =
        meta_get(conn, "server_url")?.ok_or_else(|| "server_url not configured".to_string())?;
    HttpSyncApi::new(&url).map_err(String::from)
}

// ---------------------------------------------------------------------------
// The cycle
// ---------------------------------------------------------------------------

/// How a cycle failed (DESIGN §11): a transport error backs off; a 4xx the user
/// must act on stops auto-retry.
#[derive(Debug)]
pub enum CycleError {
    /// Network/transport error — back off and retry (Offline state).
    Offline(String),
    /// A 4xx (426/clock_skew/507) or unrecoverable error — Error state, no retry.
    Fatal { message: String },
}

/// Maps a [`SyncError`] surfaced from the network paths to a cycle outcome
/// (DESIGN §11, PROTOCOL flow). Network → Offline; specific 4xx → a user message.
fn classify_error(error: SyncError) -> CycleError {
    match error {
        SyncError::Network(detail) => CycleError::Offline(detail),
        SyncError::Api {
            status,
            code,
            message,
        } => {
            let msg = match (status, code.as_str()) {
                (426, _) => "Actualizá la app: el servidor exige un esquema más nuevo.".to_string(),
                (_, "clock_skew") => {
                    "Revisá el reloj del dispositivo: la hora está demasiado desfasada.".to_string()
                }
                (_, "account_suspended") => {
                    "Tu cuenta está suspendida. Contactá al administrador para reactivarla."
                        .to_string()
                }
                (_, "subscription_expired") => {
                    "Tu suscripción venció. Podés seguir descargando, pero no vas a poder subir \
                     cambios hasta renovarla."
                        .to_string()
                }
                (507, _) => "El almacenamiento del servidor está lleno.".to_string(),
                _ => format!("Error del servidor {status} ({code}): {message}"),
            };
            CycleError::Fatal { message: msg }
        }
        SyncError::InvalidUrl(detail) => CycleError::Fatal {
            message: format!("URL del servidor inválida: {detail}"),
        },
        SyncError::Decode(detail) => CycleError::Fatal {
            message: format!("Respuesta del servidor ilegible: {detail}"),
        },
    }
}

/// Runs one full sync cycle (PROTOCOL "Flujo de sincronización del cliente").
/// Generic over [`SyncApi`] so the cycle is testable end-to-end against the
/// in-memory mock. Order: schema_tag fresh → epoch (health) → drain inherited
/// queues → seed → push → pull (which drains blobs + FTS) → persist last_sync_at.
///
/// `warn` is a logging sink (the engine passes one that writes to `app_logs`;
/// tests pass a no-op) so the cycle stays free of any `AppHandle` dependency and
/// is unit-testable against the mock with a bare connection.
pub async fn run_cycle<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    app_data_dir: &std::path::Path,
    warn: &(dyn Fn(String) + Sync),
) -> Result<(), CycleError> {
    // 0. Fresh schema_tag from the _migrations head (NEVER cached, DESIGN §3.1).
    let schema_tag = read_schema_tag(conn).map_err(|e| CycleError::Fatal { message: e })?;

    let account_id = meta_get(conn, "account_id")
        .map_err(|e| CycleError::Fatal { message: e })?
        .ok_or_else(|| CycleError::Fatal {
            message: "no hay account_id en la sesión".to_string(),
        })?;

    // 0b. Epoch check via health (DESIGN §4.9). The pull loop also re-checks per
    // page; this early call catches a restore before any push goes out.
    match api.health().await {
        Ok(health) => {
            if health.server_now_ms != 0 {
                update_clock_offset(conn, health.server_now_ms)
                    .map_err(|e| CycleError::Fatal { message: e })?;
            }
            crate::sync::pull::check_epoch(conn, &health.epoch)
                .map_err(|e| CycleError::Fatal { message: e })?;
        }
        Err(error) => return Err(classify_error(error)),
    }

    // 0c. Drain inherited pending-row + blob queues from a prior interrupted cycle
    // (PROTOCOL step 0). Parked rows first so a now-present parent unblocks them.
    {
        let mut ctx = ApplyContext::new(app_data_dir);
        retry_pending_rows(conn, &mut ctx, false).map_err(|e| CycleError::Fatal { message: e })?;
    }
    drain_pending_blobs(api, token, conn, app_data_dir)
        .await
        .map_err(|e| CycleError::Fatal { message: e })?;

    // 1. Seed a brand-new account (DESIGN §4.5): a full since=0 pull populates
    // row_versions, THEN seed pre-existing rows, THEN mark seeded. The first pull
    // is part of pull_loop below for an already-seeded account; for a new account
    // we run it up front so seeding sees the server's known versions.
    let seeded = meta_get(conn, "seeded_account").map_err(|e| CycleError::Fatal { message: e })?;
    if seeded.as_deref() != Some(account_id.as_str()) {
        // Full reconciliation pull from since=0 first.
        meta_set_i64(conn, "last_pull_seq", 0).map_err(|e| CycleError::Fatal { message: e })?;
        pull_loop(api, token, &schema_tag, conn, app_data_dir)
            .await
            .map_err(classify_error)?;
        seed_account(conn, &account_id).map_err(|e| CycleError::Fatal { message: e })?;
    }

    // 2-5. Push (snapshot → coalesce → blobs-before-rows → batched POST → reconcile).
    push_cycle(api, token, &schema_tag, conn, app_data_dir, warn).await?;

    // 6-8. Pull loop (paginated apply + parked-row retries + blob/FTS drains).
    pull_loop(api, token, &schema_tag, conn, app_data_dir)
        .await
        .map_err(classify_error)?;

    // Record the successful sync time (PROTOCOL step 9 status payload).
    meta_set_i64(conn, "last_sync_at", now_ms()).map_err(|e| CycleError::Fatal { message: e })?;
    Ok(())
}

/// The push half of a cycle (PROTOCOL steps 2-5): snapshot + coalesce the oplog,
/// build wire changes (uploading asset blobs HEAD→PUT before their rows), split
/// into capped batches, POST with 413 bisection, then reconcile the results
/// (purge oplog, advance versions, apply clean winners).
async fn push_cycle<A: SyncApi>(
    api: &A,
    token: &str,
    schema_tag: &str,
    conn: &Connection,
    app_data_dir: &std::path::Path,
    warn: &(dyn Fn(String) + Sync),
) -> Result<(), CycleError> {
    let snapshot = snapshot_oplog(conn).map_err(|e| CycleError::Fatal { message: e })?;
    if snapshot == 0 {
        return Ok(()); // Nothing dirty; skip the empty push.
    }
    let ops = coalesce_ops(conn, snapshot).map_err(|e| CycleError::Fatal { message: e })?;
    let offset = clock_offset(conn).map_err(|e| CycleError::Fatal { message: e })?;
    let mut changes =
        build_changes(conn, &ops, offset).map_err(|e| CycleError::Fatal { message: e })?;

    // Asset rows: blob BEFORE row (DESIGN §7). For each asset upsert, hash → HEAD
    // → PUT, then rewrite the payload to the wire shape; a row whose blob can't be
    // made available is journaled + purged and dropped from the batch.
    let mut ready = Vec::with_capacity(changes.len());
    for mut change in changes.drain(..) {
        if change.table == "assets" && change.op == "upsert" {
            match prepare_asset_push(api, token, conn, app_data_dir, &mut change).await {
                Ok(AssetPushOutcome::Ready) => ready.push(change),
                Ok(AssetPushOutcome::Skip(reason)) => {
                    warn(format!(
                        "Asset {} omitido del push: {reason}",
                        change.row_id
                    ));
                    journal_and_purge_oversized(conn, &change, snapshot)
                        .map_err(|e| CycleError::Fatal { message: e })?;
                }
                Err(error) => {
                    // A network failure during blob upload → back off.
                    return Err(CycleError::Offline(error));
                }
            }
        } else {
            ready.push(change);
        }
    }

    if ready.is_empty() {
        return Ok(());
    }

    let (batches, oversized) = split_into_batches(ready, DEFAULT_MAX_PUSH_BYTES);
    // Journal + purge any change too large for a single request (PROTOCOL step 4).
    for change in &oversized {
        journal_and_purge_oversized(conn, change, snapshot)
            .map_err(|e| CycleError::Fatal { message: e })?;
    }

    let outcome = batching::push_all(api, token, schema_tag, batches)
        .await
        .map_err(classify_error)?;

    // Isolated oversized changes from a 413 on a 1-element batch.
    for change in &outcome.oversized {
        journal_and_purge_oversized(conn, change, snapshot)
            .map_err(|e| CycleError::Fatal { message: e })?;
    }

    if outcome.server_now_ms != 0 {
        update_clock_offset(conn, outcome.server_now_ms)
            .map_err(|e| CycleError::Fatal { message: e })?;
    }

    // Reconcile (PROTOCOL step 5): purge oplog ≤ snapshot, advance versions, apply
    // clean lww_lost winners. The winner-apply callback delegates to the apply
    // machinery (which validates the envelope + skip-if-dirty).
    apply_push_results(conn, snapshot, &outcome.results, |c, winner| {
        let mut ctx = ApplyContext::new(app_data_dir);
        crate::sync::apply::apply_row(c, &mut ctx, winner).map(|_| ())
    })
    .map_err(|e| CycleError::Fatal { message: e })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// schema_tag + status helpers
// ---------------------------------------------------------------------------

/// Reads the local `schema_tag` = the latest applied migration name (the
/// `_migrations` head, PROTOCOL "schema_tag"). Returns `""` when the table is
/// empty / absent (a fresh DB before any JS migration); the server tolerates `''`.
pub fn read_schema_tag(conn: &Connection) -> Result<String, String> {
    // The runner applies migrations in id order, so the highest id is the head.
    let head: Option<String> = conn
        .query_row(
            "SELECT name FROM _migrations ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    Ok(head.unwrap_or_default())
}

/// Coalesced count of dirty rows awaiting push (distinct `(table, row_id)`).
fn pending_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM sync_oplog GROUP BY table_name, row_id)",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

fn blobs_pending_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM sync_pending_blobs", [], |row| {
        row.get(0)
    })
    .unwrap_or(0)
}

/// Estimated bytes of own blobs not yet uploaded (DESIGN §11 first-sync preflight).
/// Sums the cached `sync_blob_index.size` for rows with `uploaded = 0`; this is a
/// free, content-derived estimate — no filesystem stat. `COALESCE` keeps the empty
/// table at `0` instead of NULL.
fn pending_blob_bytes(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(SUM(size), 0) FROM sync_blob_index WHERE uploaded = 0",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

fn conflicts_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM sync_conflicts WHERE acknowledged = 0",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Builds a fresh status snapshot from the connection + a state/message.
pub fn build_status(conn: &Connection, state: SyncState, message: Option<String>) -> SyncStatus {
    let last_sync_at = meta_get_i64(conn, "last_sync_at").ok().filter(|v| *v > 0);
    let offset = clock_offset(conn).unwrap_or(0);
    SyncStatus {
        state,
        last_sync_at,
        pending: pending_count(conn),
        blobs_pending: blobs_pending_count(conn),
        pending_blob_bytes: pending_blob_bytes(conn),
        conflicts: conflicts_count(conn),
        clock_warning: offset.abs() > CLOCK_WARNING_MS,
        message,
    }
}

/// Computes a status snapshot, stores it in the shared cell, and emits the
/// `sync:status` event (DESIGN §11). Called on every transition and cycle end.
fn publish(
    app_handle: &AppHandle,
    status: &Arc<Mutex<SyncStatus>>,
    conn: &Connection,
    state: SyncState,
    message: Option<String>,
) {
    let snapshot = build_status(conn, state, message);
    {
        let mut guard = status.lock().unwrap_or_else(|p| p.into_inner());
        *guard = snapshot.clone();
    }
    let _ = app_handle.emit(STATUS_EVENT, &snapshot);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Command-side snapshot (no engine handle needed: opens its own connection)
// ---------------------------------------------------------------------------

/// Builds a status snapshot directly from the DB for the `sync_status` command
/// when the engine has not yet published one (UI bootstrap). Disabled when no
/// session exists.
pub fn snapshot_from_db(conn: &Connection) -> SyncStatus {
    if !is_gated_open(conn) {
        return SyncStatus::disabled();
    }
    build_status(conn, SyncState::Idle, None)
}

/// Accessor used by the `sync_status` command to read the engine handle from
/// managed state (it may not exist in headless tests).
pub fn engine_snapshot(app_handle: &AppHandle, db_path: &std::path::Path) -> SyncStatus {
    if let Some(engine) = app_handle.try_state::<SyncEngine>() {
        return engine.snapshot();
    }
    // No engine (e.g. setup failed): fall back to a DB-derived snapshot.
    match crate::sync::open_sync_connection(db_path) {
        Ok(conn) => snapshot_from_db(&conn),
        Err(_) => SyncStatus::disabled(),
    }
}

#[cfg(test)]
mod tests;
