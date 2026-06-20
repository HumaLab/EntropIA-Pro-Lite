//! Remaining sync Tauri commands (DESIGN §11). The status/control surface the
//! Settings card binds to: status snapshot, manual sync, auto-sync toggle, device
//! management, conflict journal, usage, and account deletion. None of these touch
//! `app_settings`/`SETTINGS_KEYS` — sync config lives ONLY in `sync_meta` + the
//! keyring (DESIGN §11).
//!
//! Every command that talks to the server reads the device token from the keyring
//! (NEVER logged, DESIGN §8) and builds an [`HttpSyncApi`] from the persisted
//! `server_url` (TLS re-validated). Long-running / blocking SQLite work runs on
//! the blocking pool with the engine's own-connection discipline (never `ui_conn`).

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::db::state::AppDbState;
use crate::sync::engine::{engine_snapshot, SyncEngine, SyncRequest, SyncStatus};
use crate::sync::http::{
    DeleteAccountRequest, DeviceInfo, HttpSyncApi, NotificationItem, PlanCatalogItem,
    PlanChangeRequestResponse, SyncApi, UsageResponse,
};
use crate::sync::open_sync_connection;
use crate::sync::session::{clear_sync_state, delete_token, meta_get, meta_set, read_token};

const LOG_SOURCE: &str = "sync";

/// Reads the persisted `server_url` and the keyring token, erroring when either is
/// missing (the user is not logged in). The token is never logged.
async fn session_creds(db_path: std::path::PathBuf) -> Result<(String, String), String> {
    let url = tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let conn = open_sync_connection(&db_path)?;
        meta_get(&conn, "server_url")
    })
    .await
    .map_err(|e| format!("[sync] creds read task failed: {e}"))??
    .ok_or_else(|| "No hay una sesión de sync configurada".to_string())?;

    let token = tokio::task::spawn_blocking(read_token)
        .await
        .map_err(|e| format!("[sync] token read task failed: {e}"))??
        .ok_or_else(|| "No hay token de dispositivo".to_string())?;

    Ok((url, token))
}

// ---------------------------------------------------------------------------
// Status + manual sync + auto-sync toggle
// ---------------------------------------------------------------------------

/// Returns the current sync status snapshot for UI bootstrap (DESIGN §11). Reads
/// the engine's last-published status when available, else derives one from the DB.
#[tauri::command]
pub async fn sync_status(
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<SyncStatus, String> {
    let db_path = db.db_path.clone();
    let handle = app_handle.clone();
    tokio::task::spawn_blocking(move || engine_snapshot(&handle, &db_path))
        .await
        .map_err(|e| format!("[sync] status task failed: {e}"))
}

/// Triggers a manual sync run (DESIGN §3.1). Returns the current status
/// immediately; the engine coalesces concurrent requests into at most one pending
/// run. A no-op (returns the current snapshot) when the engine is not running.
#[tauri::command]
pub async fn sync_now(
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<SyncStatus, String> {
    if let Some(engine) = app_handle.try_state::<SyncEngine>() {
        engine.request(SyncRequest::SyncNow);
    }
    let db_path = db.db_path.clone();
    let handle = app_handle.clone();
    tokio::task::spawn_blocking(move || engine_snapshot(&handle, &db_path))
        .await
        .map_err(|e| format!("[sync] sync_now task failed: {e}"))
}

/// Sets the auto-sync toggle + interval (DESIGN §11). Persists to `sync_meta`
/// (`auto_sync_enabled`, `auto_sync_interval_min`) and nudges the engine so the
/// new cadence applies. `interval_min` is clamped to ≥ 1.
#[tauri::command]
pub async fn sync_set_auto(
    enabled: bool,
    interval_min: i64,
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let db_path = db.db_path.clone();
    let interval = interval_min.max(1);
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = open_sync_connection(&db_path)?;
        meta_set(&conn, "auto_sync_enabled", if enabled { "1" } else { "0" })?;
        meta_set(&conn, "auto_sync_interval_min", &interval.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| format!("[sync] set_auto task failed: {e}"))??;

    // Poke the engine so an enable takes effect without waiting for the ticker.
    if enabled {
        if let Some(engine) = app_handle.try_state::<SyncEngine>() {
            engine.request(SyncRequest::Tick);
        }
    }
    crate::app_logs::info(
        &app_handle,
        LOG_SOURCE,
        format!(
            "Auto-sync {} (cada {interval} min)",
            if enabled { "activado" } else { "desactivado" }
        ),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

/// Lists the account's devices (PROTOCOL `GET /v1/devices`).
#[tauri::command]
pub async fn sync_list_devices(db: State<'_, AppDbState>) -> Result<Vec<DeviceInfo>, String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    let response = api.devices(&token).await.map_err(String::from)?;
    Ok(response.devices)
}

/// Revokes another device by id (PROTOCOL `DELETE /v1/devices/{id}`). A device
/// cannot revoke itself here — the UI uses `sync_logout` for that.
#[tauri::command]
pub async fn sync_revoke_device(
    device_id: String,
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    api.revoke(&token, &device_id).await.map_err(String::from)?;
    crate::app_logs::info(&app_handle, LOG_SOURCE, "Dispositivo revocado");
    Ok(())
}

// ---------------------------------------------------------------------------
// Conflicts journal
// ---------------------------------------------------------------------------

/// One conflict journal entry surfaced to the UI (DESIGN §6 schema).
#[derive(Debug, Clone, Serialize)]
pub struct ConflictEntry {
    pub id: String,
    pub table_name: String,
    pub row_id: String,
    pub reason: String,
    pub loser_payload: Option<String>,
    pub winner_summary: Option<String>,
    pub created_at: i64,
    pub acknowledged: bool,
}

/// Lists conflict journal entries newest-first, paginated (DESIGN §11). `limit` is
/// clamped to a sane window; `offset` paginates. Returns unacknowledged + ack'd
/// (the UI filters); newest first by `created_at`.
#[tauri::command]
pub async fn sync_list_conflicts(
    limit: Option<i64>,
    offset: Option<i64>,
    db: State<'_, AppDbState>,
) -> Result<Vec<ConflictEntry>, String> {
    let db_path = db.db_path.clone();
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let offset = offset.unwrap_or(0).max(0);
    tokio::task::spawn_blocking(move || -> Result<Vec<ConflictEntry>, String> {
        let conn = open_sync_connection(&db_path)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, table_name, row_id, reason, loser_payload, winner_summary,
                        created_at, acknowledged
                 FROM sync_conflicts
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| format!("[sync] failed to prepare conflicts query: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![limit, offset], |row| {
                Ok(ConflictEntry {
                    id: row.get(0)?,
                    table_name: row.get(1)?,
                    row_id: row.get(2)?,
                    reason: row.get(3)?,
                    loser_payload: row.get(4)?,
                    winner_summary: row.get(5)?,
                    created_at: row.get(6)?,
                    acknowledged: row.get::<_, i64>(7)? != 0,
                })
            })
            .map_err(|e| format!("[sync] failed to query conflicts: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("[sync] failed to read conflict: {e}"))?);
        }
        Ok(out)
    })
    .await
    .map_err(|e| format!("[sync] list_conflicts task failed: {e}"))?
}

/// Acknowledges a conflict by id (DESIGN §11): sets `acknowledged=1` so it drops
/// out of the unacknowledged count surfaced in the status event.
#[tauri::command]
pub async fn sync_ack_conflict(
    conflict_id: String,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = open_sync_connection(&db_path)?;
        conn.execute(
            "UPDATE sync_conflicts SET acknowledged = 1 WHERE id = ?1",
            [&conflict_id],
        )
        .map(|_| ())
        .map_err(|e| format!("[sync] failed to ack conflict {conflict_id}: {e}"))
    })
    .await
    .map_err(|e| format!("[sync] ack_conflict task failed: {e}"))?
}

// ---------------------------------------------------------------------------
// Usage + account deletion
// ---------------------------------------------------------------------------

/// Returns the account's storage usage (PROTOCOL `GET /v1/usage`).
#[tauri::command]
pub async fn sync_get_usage(db: State<'_, AppDbState>) -> Result<UsageResponse, String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    api.usage(&token).await.map_err(String::from)
}

// ---------------------------------------------------------------------------
// Plans + plan-change request + notifications (NOTIFICATIONS.md)
// ---------------------------------------------------------------------------

/// Lists the plan catalog (PROTOCOL `GET /v1/plans`). Powers the upgrade modal's
/// `<select>`; ordered ascending by price/quota by the server. No subscription
/// gating: an expired/suspended account still sees the catalog.
#[tauri::command]
pub async fn sync_list_plans(db: State<'_, AppDbState>) -> Result<Vec<PlanCatalogItem>, String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    api.list_plans(&token).await.map_err(String::from)
}

/// Requests a plan change (PROTOCOL `POST /v1/plan-change-request`). Returns the
/// created request. A `409 plan_request_pending` surfaces as a clear String error
/// ("Ya tenés una solicitud de cambio de plan en revisión.") so the UI can guide
/// the user instead of showing a raw API code; other API errors pass through.
#[tauri::command]
pub async fn sync_request_plan_change(
    requested_plan_id: String,
    note: Option<String>,
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<PlanChangeRequestResponse, String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    let result = api
        .request_plan_change(&token, &requested_plan_id, note.as_deref())
        .await
        .map_err(|error| {
            // Translate the pending-request conflict to a user-facing message; other
            // errors keep their uniform `String` rendering.
            if error.api_code() == Some("plan_request_pending") {
                "Ya tenés una solicitud de cambio de plan en revisión.".to_string()
            } else {
                String::from(error)
            }
        });
    match &result {
        Ok(_) => crate::app_logs::info(
            &app_handle,
            LOG_SOURCE,
            "Solicitud de cambio de plan enviada",
        ),
        Err(error) => crate::app_logs::warn(
            &app_handle,
            LOG_SOURCE,
            format!("Solicitud de cambio de plan falló: {error}"),
        ),
    }
    result
}

/// Lists the user's in-app notifications (PROTOCOL `GET /v1/notifications`).
/// `since` is an exclusive id cursor (empty/`"0"` ⇒ from the start); `limit` is
/// capped at 100 server-side.
#[tauri::command]
pub async fn sync_list_notifications(
    since: Option<String>,
    limit: Option<i64>,
    db: State<'_, AppDbState>,
) -> Result<Vec<NotificationItem>, String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    api.list_notifications(&token, since.as_deref(), limit)
        .await
        .map_err(String::from)
}

/// Marks a notification as read (PROTOCOL `POST /v1/notifications/{id}/read`).
/// Idempotent; a 404 surfaces as a String error.
#[tauri::command]
pub async fn sync_mark_notification_read(
    id: String,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    api.mark_notification_read(&token, &id)
        .await
        .map_err(String::from)
}

/// Deletes a notification from the user's in-app inbox (PROTOCOL `DELETE /v1/notifications/{id}`).
/// A 404 surfaces as a String error.
#[tauri::command]
pub async fn sync_delete_notification(
    id: String,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;
    api.delete_notification(&token, &id)
        .await
        .map_err(String::from)
}

/// Deletes the account's server-side data (PROTOCOL `DELETE /v1/account`,
/// re-auth with password). On a 204 the local sync state is wiped (the same
/// procedure as logout, DESIGN §6.3) since every device token just died; local
/// APP data is untouched. The password is never logged.
#[tauri::command]
pub async fn sync_delete_account(
    password: String,
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let (url, token) = session_creds(db.db_path.clone()).await?;
    let api = HttpSyncApi::new(&url).map_err(String::from)?;

    api.delete_account(&token, DeleteAccountRequest { password })
        .await
        .map_err(|error| {
            crate::app_logs::warn(
                &app_handle,
                LOG_SOURCE,
                format!("Borrado de cuenta falló: {error}"),
            );
            String::from(error)
        })?;

    // 204: every device token is dead. Run the local logout procedure (DESIGN
    // §6.3) — wipe sync state + remove the keyring token. App data stays.
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = open_sync_connection(&db_path)?;
        clear_sync_state(&conn)
    })
    .await
    .map_err(|e| format!("[sync] delete_account wipe task failed: {e}"))??;

    tokio::task::spawn_blocking(delete_token)
        .await
        .map_err(|e| format!("[sync] token delete task failed: {e}"))??;

    crate::app_logs::info(
        &app_handle,
        LOG_SOURCE,
        "Datos del servidor borrados; sesión local limpiada",
    );
    Ok(())
}
