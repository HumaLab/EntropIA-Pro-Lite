//! Sync session lifecycle (DESIGN §8, §6.3) and `sync_meta` typed accessors.
//!
//! Holds the register/login/logout Tauri commands plus the small helpers the
//! push/pull slices use to read and write `sync_meta` with the right types. The
//! device token lives ONLY in the OS keyring (never in `sync_meta`, never in
//! `app_settings`, never logged — DESIGN §8), under a service name distinct from
//! the app-settings keyring so sync credentials can be wiped independently.

use rusqlite::Connection;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::db::state::AppDbState;
use crate::sync::http::{HttpSyncApi, LoginRequest, RegisterRequest, SyncApi};
use crate::sync::open_sync_connection;

/// Keyring service name for the sync device token. Distinct from the
/// app-settings service (`"com.entropia.desktop credentials"`) so a sync logout never touches LLM
/// API keys and vice versa (DESIGN §8).
const SYNC_KEYRING_SERVICE: &str = "com.entropia.lite sync";
/// Keyring entry name (account/user) for the single device token.
const TOKEN_KEY: &str = "device_token";

const LOG_SOURCE: &str = "sync/session";
pub(crate) const SYNC_SESSION_INCARNATION_KEY: &str = "sync_session_incarnation";

// ---------------------------------------------------------------------------
// sync_meta typed accessors (used across the push/pull slices)
// ---------------------------------------------------------------------------

/// Reads a `sync_meta` value, returning `None` when the key is absent.
pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(format!("[sync] failed to read sync_meta['{key}']: {other}")),
    })
}

/// Upserts a `sync_meta` value.
pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to write sync_meta['{key}']: {e}"))
}

/// Deletes a `sync_meta` key (no-op when absent).
pub fn meta_delete(conn: &Connection, key: &str) -> Result<(), String> {
    conn.execute("DELETE FROM sync_meta WHERE key = ?1", [key])
        .map(|_| ())
        .map_err(|e| format!("[sync] failed to delete sync_meta['{key}']: {e}"))
}

/// Reads a `sync_meta` value parsed as `i64`, defaulting to `0` when absent or
/// unparseable. Used for `last_pull_seq` and `clock_offset_ms`.
#[allow(dead_code)]
pub fn meta_get_i64(conn: &Connection, key: &str) -> Result<i64, String> {
    Ok(meta_get(conn, key)?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0))
}

/// Writes an `i64` `sync_meta` value.
#[allow(dead_code)]
pub fn meta_set_i64(conn: &Connection, key: &str, value: i64) -> Result<(), String> {
    meta_set(conn, key, &value.to_string())
}

/// Reads and validates the identity of the current successful login write.
/// Missing metadata remains `None`; reads never synthesize persistent identity.
pub(crate) fn read_session_incarnation(conn: &Connection) -> Result<Option<Uuid>, String> {
    let Some(value) = meta_get(conn, SYNC_SESSION_INCARNATION_KEY)? else {
        return Ok(None);
    };
    Uuid::parse_str(&value).map(Some).map_err(|_| {
        format!("[sync] sync_meta['{SYNC_SESSION_INCARNATION_KEY}'] is not a valid UUID")
    })
}

/// True when the persisted session identity (`account_id`, `server_url`,
/// `device_id`) is present and non-blank. The incarnation self-heal only ever
/// runs for a complete identity.
fn session_identity_complete(conn: &Connection) -> Result<bool, String> {
    for key in ["account_id", "server_url", "device_id"] {
        let value = meta_get(conn, key)?;
        if value.as_deref().is_none_or(|value| value.trim().is_empty()) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Upgrade self-heal: sessions created before the incarnation feature carry a
/// complete identity but no `sync_session_incarnation`, which silently disabled
/// the writing phase forever. When the identity (`account_id`, `server_url`,
/// `device_id`) is present and the incarnation is missing, mint one fresh UUID
/// and persist it exactly once — atomically, touching no other session or sync
/// state. An existing incarnation is never overwritten (a malformed one keeps
/// surfacing [`read_session_incarnation`]'s error), a session-less database is
/// never given one, and the mint never runs while a transaction is already open
/// on this connection (that would nest `unchecked_transaction` improperly; the
/// next call outside a transaction self-heals). Returns the effective
/// incarnation: `None` while no complete identity exists.
pub(crate) fn ensure_session_incarnation(conn: &Connection) -> Result<Option<Uuid>, String> {
    if let Some(value) = meta_get(conn, SYNC_SESSION_INCARNATION_KEY)? {
        // An existing incarnation (valid or not) is never replaced.
        return Uuid::parse_str(&value).map(Some).map_err(|_| {
            format!("[sync] sync_meta['{SYNC_SESSION_INCARNATION_KEY}'] is not a valid UUID")
        });
    }
    if !session_identity_complete(conn)? {
        return Ok(None);
    }
    if !conn.is_autocommit() {
        // A transaction is already open on this connection: minting would nest
        // `unchecked_transaction` improperly. Nothing is written here; the next
        // call outside a transaction self-heals.
        return Ok(None);
    }
    mint_session_incarnation(conn)
}

/// Mints the missing incarnation in one `unchecked_transaction` (the exact
/// [`write_sync_session`] pattern), re-checking inside the transaction so a
/// concurrent writer's value is never overwritten. The caller guarantees `conn`
/// is in autocommit mode.
fn mint_session_incarnation(conn: &Connection) -> Result<Option<Uuid>, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin incarnation mint transaction: {e}"))?;
    if meta_get(&tx, SYNC_SESSION_INCARNATION_KEY)?.is_some() {
        // Another writer minted first: keep its value, never replace it.
        tx.commit()
            .map_err(|e| format!("[sync] failed to commit incarnation mint: {e}"))?;
        return read_session_incarnation(conn);
    }
    if !session_identity_complete(&tx)? {
        tx.commit()
            .map_err(|e| format!("[sync] failed to commit incarnation mint: {e}"))?;
        return Ok(None);
    }
    let minted = Uuid::new_v4();
    // Conditional insert: exactly-once even against a racing writer.
    let inserted = tx
        .execute(
            "INSERT INTO sync_meta(key, value) SELECT ?1, ?2
             WHERE NOT EXISTS (SELECT 1 FROM sync_meta WHERE key = ?1)",
            rusqlite::params![SYNC_SESSION_INCARNATION_KEY, minted.to_string()],
        )
        .map_err(|e| format!("[sync] failed to persist minted session incarnation: {e}"))?;
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit incarnation mint: {e}"))?;
    if inserted > 0 {
        Ok(Some(minted))
    } else {
        read_session_incarnation(conn)
    }
}

// ---------------------------------------------------------------------------
// Token keyring helpers (DESIGN §8 — token NEVER touches SQLite or logs)
// ---------------------------------------------------------------------------

/// The keyring `(service, user)` of the device token. A dev profile gets its own
/// entry, named after the profile, so it can neither read nor replace the real
/// session and two profiles never share one.
fn token_target(profile: Option<&str>) -> (String, String) {
    match profile {
        None => (SYNC_KEYRING_SERVICE.to_string(), TOKEN_KEY.to_string()),
        Some(name) => (
            format!("{SYNC_KEYRING_SERVICE} (dev profile)"),
            format!("{TOKEN_KEY}@dev-profile:{name}"),
        ),
    }
}

fn token_entry() -> Result<keyring::Entry, String> {
    // The one door to the sync token: a dev profile without a local server
    // never opens it, and one with a local server opens only its own entry, so
    // it can neither read nor replace the real session.
    crate::dev_profile::require_sync()?;
    let (service, user) = token_target(crate::dev_profile::active());
    keyring::Entry::new(&service, &user)
        .map_err(|e| format!("[sync] failed to open keyring for device token: {e}"))
}

/// A keyring failure as the UI receives it. A missing or empty credential store
/// carries the marker src/lib/sync.ts explains in plain words (settings.rs).
fn describe_token_error(action: &str, error: &keyring::Error) -> String {
    let message = format!("[sync] failed to {action} device token in keyring: {error}");
    if crate::settings::is_credential_store_unavailable(error) {
        format!(
            "{}: {message}",
            crate::settings::CREDENTIAL_STORE_UNAVAILABLE
        )
    } else {
        message
    }
}

/// Stores the device token in the OS keyring.
pub fn store_token(token: &str) -> Result<(), String> {
    token_entry()?
        .set_password(token)
        .map_err(|e| describe_token_error("store", &e))
}

/// Reads the device token from the OS keyring, `None` when not present.
pub fn read_token() -> Result<Option<String>, String> {
    match token_entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(other) => Err(describe_token_error("read", &other)),
    }
}

/// Deletes the device token from the OS keyring (idempotent — a missing entry is
/// treated as success).
pub fn delete_token() -> Result<(), String> {
    match token_entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(other) => Err(describe_token_error("delete", &other)),
    }
}

// ---------------------------------------------------------------------------
// Logout / account-change wipe (DESIGN §6.3 — NORMATIVE)
// ---------------------------------------------------------------------------

/// `sync_meta` keys cleared on logout / account change (DESIGN §6.3).
const SESSION_META_KEYS: &[&str] = &[
    "last_pull_seq",
    "device_id",
    "account_id",
    "account_email",
    SYNC_SESSION_INCARNATION_KEY,
    "seeded_account",
    "server_epoch",
    "capture_enabled",
    "clock_offset_ms",
];

/// State tables fully emptied on logout / account change (DESIGN §6.3). Note
/// `sync_blob_index` is NOT in this list: its hashes are content-derived and
/// retained; only the `uploaded` flag is reset (see [`clear_sync_state`]).
const SESSION_STATE_TABLES: &[&str] = &[
    "sync_oplog",
    "sync_row_versions",
    "sync_conflicts",
    "sync_pending_rows",
    "sync_pending_blobs",
    "sync_web_pending_blobs",
    "sync_pending_fts",
    "sync_topic_aliases",
];

/// Wipes ALL local sync state per DESIGN §6.3, in one transaction:
/// delete the per-session state tables, clear session plus writing- and
/// research-account `sync_meta`, and reset `uploaded=0` across the WHOLE `sync_blob_index`
/// (hashes survive — they are content-derived). Does NOT touch the keyring;
/// callers handle the token separately (revoke remote first, then
/// [`delete_token`]).
pub fn clear_sync_state(conn: &Connection) -> Result<(), String> {
    let tx_guard = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin logout transaction: {e}"))?;

    for table in SESSION_STATE_TABLES {
        // Table names come from a compile-time allowlist — safe to interpolate.
        tx_guard
            .execute_batch(&format!("DELETE FROM {table};"))
            .map_err(|e| format!("[sync] failed to clear {table}: {e}"))?;
    }

    for key in SESSION_META_KEYS {
        meta_delete(&tx_guard, key)?;
    }

    crate::writing::sync_capture::clear_account_metadata(&tx_guard).map_err(|error| {
        format!(
            "[sync] failed to clear writing sync metadata: {}",
            error.message
        )
    })?;

    crate::sync::research_capture::clear_account_metadata(&tx_guard).map_err(|error| {
        format!(
            "[sync] failed to clear research sync metadata: {}",
            error.message
        )
    })?;

    crate::sync::web_capture::clear_account_metadata(&tx_guard)?;

    // Reset every blob's uploaded flag (DESIGN §6.3): uploaded=1 only ever held
    // for the account that set it; a new account must re-confirm via HEAD/PUT.
    tx_guard
        .execute_batch("UPDATE sync_blob_index SET uploaded = 0;")
        .map_err(|e| format!("[sync] failed to reset blob upload flags: {e}"))?;

    tx_guard
        .commit()
        .map_err(|e| format!("[sync] failed to commit logout transaction: {e}"))
}

// ---------------------------------------------------------------------------
// Device naming for login
// ---------------------------------------------------------------------------

/// Best-effort human device name for the login request (PROTOCOL
/// `/v1/auth/login`). Uses the OS hostname when resolvable, else a generic
/// per-platform label. Never includes anything sensitive.
fn default_device_name() -> String {
    let host = std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty());
    match host {
        Some(host) => host,
        None => format!("{} device", std::env::consts::OS),
    }
}

/// The platform string for the login request (PROTOCOL `platform`).
fn platform_label() -> String {
    std::env::consts::OS.to_string()
}

/// Atomically persists one successful login and gives it a fresh incarnation.
/// Repeating the same account/server/device identity still creates a new value.
pub(crate) fn write_sync_session(
    conn: &Connection,
    server_url: &str,
    account_id: &str,
    account_email: &str,
    device_id: &str,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin login transaction: {e}"))?;
    let incarnation = Uuid::new_v4().to_string();
    meta_set(&tx, "server_url", server_url)?;
    meta_set(&tx, "account_id", account_id)?;
    meta_set(&tx, "account_email", account_email)?;
    meta_set(&tx, "device_id", device_id)?;
    meta_set(&tx, SYNC_SESSION_INCARNATION_KEY, &incarnation)?;
    meta_set(&tx, "capture_enabled", "1")?;
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit login session: {e}"))
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Registers a new account on the server (PROTOCOL `POST /v1/auth/register`).
/// Gated server-side by `SYNC_REGISTRATION_OPEN`; surfaces the server error
/// (e.g. `registration_closed`, `email_taken`) as a `String`.
#[tauri::command]
pub async fn sync_register_account(
    server_url: String,
    email: String,
    password: String,
    app_handle: AppHandle,
) -> Result<String, String> {
    crate::dev_profile::require_sync()?;
    let server_url = crate::dev_profile::server_for(&server_url);
    // Build the API in a blocking task: the constructor validates the TLS rule
    // and reqwest client construction is cheap but not free.
    let api = HttpSyncApi::new(&server_url).map_err(String::from)?;
    let result = api
        .register(RegisterRequest { email, password })
        .await
        .map_err(String::from);
    match &result {
        Ok(_) => crate::app_logs::info(&app_handle, LOG_SOURCE, "Cuenta registrada"),
        Err(error) => {
            crate::app_logs::warn(&app_handle, LOG_SOURCE, format!("Registro falló: {error}"))
        }
    }
    result.map(|response| response.account_id)
}

async fn run_blocking(
    label: &str,
    step: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    tokio::task::spawn_blocking(step)
        .await
        .unwrap_or_else(|e| Err(format!("[sync] {label} task failed: {e}")))
}

/// Keeps what a login just created, or undoes it. The server registered the
/// device before the app stored its token (`store`) and its session
/// (`write_session`); if either fails the app shows no session, so that device
/// can never be used from here. It is revoked (best effort) and, when the token
/// already reached the keyring, the token is dropped (`forget`) before the
/// original error reaches the user. Otherwise every failed attempt would leave
/// one more orphan device on the account.
async fn keep_login_or_revoke<A: SyncApi>(
    api: &A,
    token: String,
    store: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
    write_session: impl FnOnce() -> Result<(), String> + Send + 'static,
    forget: impl FnOnce() -> Result<(), String> + Send + 'static,
    mut warn: impl FnMut(String),
) -> Result<(), String> {
    let stored = {
        let token = token.clone();
        run_blocking("token store", move || store(&token)).await
    };
    let result = match stored {
        Err(error) => Err(error),
        Ok(()) => {
            let written = run_blocking("login session", write_session).await;
            if written.is_err() {
                if let Err(error) = run_blocking("token delete", forget).await {
                    warn(format!("No se pudo borrar el token del llavero: {error}"));
                }
            }
            written
        }
    };
    if result.is_err() {
        if let Err(error) = api.logout(&token).await {
            warn(format!(
                "No se pudo dar de baja el dispositivo recién creado: {error}"
            ));
        }
    }
    result
}

/// Logs in (PROTOCOL `POST /v1/auth/login`): creates a fresh device, stores the
/// token in the keyring, persists `device_id`/`account_id`/`account_email`/
/// `server_url` plus a fresh session incarnation in `sync_meta`, and turns
/// capture ON (`capture_enabled='1'`, DESIGN §4.1). The seeding (DESIGN §4.5)
/// is performed later by the engine, not here. The token is never logged
/// (DESIGN §8).
#[tauri::command]
pub async fn sync_login(
    server_url: String,
    email: String,
    password: String,
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    crate::dev_profile::require_sync()?;
    let server_url = crate::dev_profile::server_for(&server_url);
    let validated_url =
        crate::sync::http::validate_server_url(&server_url).map_err(String::from)?;
    let api = HttpSyncApi::new(&validated_url).map_err(String::from)?;

    let device_name = default_device_name();
    let platform = platform_label();

    let response = api
        .login(LoginRequest {
            email: email.clone(),
            password,
            device_name,
            platform,
        })
        .await
        .map_err(|error| {
            crate::app_logs::warn(&app_handle, LOG_SOURCE, format!("Login falló: {error}"));
            String::from(error)
        })?;

    // Persist the token in the keyring BEFORE writing the session so a crash
    // never leaves a session pointing at a token we failed to store.
    let db_path = db.db_path.clone();
    let account_id = response.account_id.clone();
    let device_id = response.device_id.clone();
    let write_session = move || -> Result<(), String> {
        let conn = open_sync_connection(&db_path)?;
        write_sync_session(&conn, &validated_url, &account_id, &email, &device_id)
    };
    keep_login_or_revoke(
        &api,
        response.device_token.clone(),
        store_token,
        write_session,
        delete_token,
        |message| crate::app_logs::warn(&app_handle, LOG_SOURCE, message),
    )
    .await?;

    crate::app_logs::info(&app_handle, LOG_SOURCE, "Sesión de sync iniciada");
    Ok(())
}

/// Logs out (DESIGN §6.3): best-effort remote revoke of the current device
/// token, then a full local wipe of every sync state table + session
/// `sync_meta` key, `uploaded=0` on `sync_blob_index`, capture turned OFF, and
/// the token removed from the keyring. Local app data is untouched.
#[tauri::command]
pub async fn sync_logout(db: State<'_, AppDbState>, app_handle: AppHandle) -> Result<(), String> {
    crate::dev_profile::require_sync()?;
    let db_path = db.db_path.clone();

    // Read the server URL and token to attempt a best-effort remote revoke.
    let server_url = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> Result<Option<String>, String> {
            let conn = open_sync_connection(&db_path)?;
            meta_get(&conn, "server_url")
        }
    })
    .await
    .map_err(|e| format!("[sync] logout read task failed: {e}"))??;

    let token = tokio::task::spawn_blocking(read_token)
        .await
        .map_err(|e| format!("[sync] token read task failed: {e}"))??;

    // Best-effort remote revoke — never blocks the local wipe on a network error
    // (the user may be offline; the local session must still clear).
    if let (Some(url), Some(token)) = (server_url, token) {
        if let Ok(api) = HttpSyncApi::new(&url) {
            if let Err(error) = api.logout(&token).await {
                crate::app_logs::warn(
                    &app_handle,
                    LOG_SOURCE,
                    format!("Revocación remota falló (se limpia localmente igual): {error}"),
                );
            }
        }
    }

    // Local wipe (DESIGN §6.3) — capture turns off as part of clearing the
    // `capture_enabled` meta key.
    tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> Result<(), String> {
            let conn = open_sync_connection(&db_path)?;
            clear_sync_state(&conn)
        }
    })
    .await
    .map_err(|e| format!("[sync] logout wipe task failed: {e}"))??;

    // Remove the token from the keyring last.
    tokio::task::spawn_blocking(delete_token)
        .await
        .map_err(|e| format!("[sync] token delete task failed: {e}"))??;

    crate::app_logs::info(&app_handle, LOG_SOURCE, "Sesión de sync cerrada");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::capture::ensure_capture;
    use crate::sync::schema::SYNC_TABLES;
    use crate::sync::test_support::new_synced_test_db;

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .expect("count")
    }

    // Same WSL failures as settings.rs: logging in on Linux without a usable
    // Login creates the device on the server before the token is stored. When
    // the keyring refuses it, that device can never be used from here.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    const NO_KEYRING: &str = "credential_store_unavailable: no keyring";
    const NO_SESSION: &str = "[sync] failed to commit login session: disk full";

    /// Runs `keep_login_or_revoke` with canned step results; returns its result,
    /// the warnings logged and whether the token was dropped from the keyring.
    async fn finish_login(
        api: &crate::sync::test_support::MockSyncApi,
        stored: Result<(), String>,
        written: Result<(), String>,
    ) -> (Result<(), String>, Vec<String>, bool) {
        let forgotten = Arc::new(AtomicBool::new(false));
        let mut warnings = Vec::new();
        let result = keep_login_or_revoke(
            api,
            "fresh-token".to_string(),
            move |token| {
                assert_eq!(token, "fresh-token");
                stored
            },
            move || written,
            {
                let forgotten = forgotten.clone();
                move || {
                    forgotten.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
            |message| warnings.push(message),
        )
        .await;
        (result, warnings, forgotten.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn a_token_the_keyring_refuses_is_revoked_on_the_server() {
        let api = crate::sync::test_support::MockSyncApi::default();
        let (result, warnings, forgotten) =
            finish_login(&api, Err(NO_KEYRING.to_string()), Ok(())).await;
        assert_eq!(result, Err(NO_KEYRING.to_string()));
        assert_eq!(*api.logged_out.lock().unwrap(), vec!["fresh-token"]);
        assert!(warnings.is_empty(), "{warnings:?}");
        // Nothing reached the keyring, so there is nothing to drop from it.
        assert!(!forgotten);
    }

    #[tokio::test]
    async fn a_failed_revoke_is_logged_and_the_keyring_error_still_wins() {
        let api = crate::sync::test_support::MockSyncApi::default();
        *api.logout_fails.lock().unwrap() = true;
        let (result, warnings, _) = finish_login(&api, Err(NO_KEYRING.to_string()), Ok(())).await;
        assert_eq!(result, Err(NO_KEYRING.to_string()));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("connection refused"), "{warnings:?}");
    }

    // The token is in the keyring but the session never reached the database:
    // the app shows no session, so that device and its token are orphans too.
    #[tokio::test]
    async fn a_session_that_cannot_be_saved_revokes_the_device_and_drops_its_token() {
        let api = crate::sync::test_support::MockSyncApi::default();
        let (result, warnings, forgotten) =
            finish_login(&api, Ok(()), Err(NO_SESSION.to_string())).await;
        assert_eq!(result, Err(NO_SESSION.to_string()));
        assert_eq!(*api.logged_out.lock().unwrap(), vec!["fresh-token"]);
        assert!(forgotten);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[tokio::test]
    async fn the_token_is_dropped_even_when_the_revoke_fails() {
        let api = crate::sync::test_support::MockSyncApi::default();
        *api.logout_fails.lock().unwrap() = true;
        let (result, warnings, forgotten) =
            finish_login(&api, Ok(()), Err(NO_SESSION.to_string())).await;
        assert_eq!(result, Err(NO_SESSION.to_string()));
        assert!(forgotten);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[tokio::test]
    async fn a_completed_login_keeps_its_device_and_token() {
        let api = crate::sync::test_support::MockSyncApi::default();
        let (result, warnings, forgotten) = finish_login(&api, Ok(()), Ok(())).await;
        assert_eq!(result, Ok(()));
        assert!(api.logged_out.lock().unwrap().is_empty());
        assert!(!forgotten);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    // Same WSL failures as settings.rs: logging in on Linux without a usable
    // keyring must reach the UI as "no credential store", not as DBus.
    #[test]
    fn a_missing_or_empty_keyring_is_marked_for_the_ui() {
        let no_service = keyring::Error::PlatformFailure(
            "DBus error: The name org.freedesktop.secrets was not provided by any .service files"
                .to_string()
                .into(),
        );
        let no_default =
            keyring::Error::NoStorageAccess("Secret Service: no result found".to_string().into());
        for error in [no_service, no_default] {
            let message = describe_token_error("store", &error);
            assert!(
                message.starts_with(crate::settings::CREDENTIAL_STORE_UNAVAILABLE),
                "{message}"
            );
        }
    }

    #[test]
    fn other_keyring_errors_keep_their_own_message() {
        let error = keyring::Error::TooLong("password".into(), 10);
        let message = describe_token_error("store", &error);
        assert!(
            message.starts_with("[sync] failed to store device token"),
            "{message}"
        );
    }

    #[test]
    fn meta_accessors_round_trip() {
        let conn = new_synced_test_db();
        assert_eq!(meta_get(&conn, "device_id").unwrap(), None);
        meta_set(&conn, "device_id", "dev-1").unwrap();
        assert_eq!(
            meta_get(&conn, "device_id").unwrap().as_deref(),
            Some("dev-1")
        );
        meta_delete(&conn, "device_id").unwrap();
        assert_eq!(meta_get(&conn, "device_id").unwrap(), None);

        // i64 accessors default to 0 and round-trip.
        assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 0);
        meta_set_i64(&conn, "last_pull_seq", 42).unwrap();
        assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 42);
        // Unparseable value falls back to 0.
        meta_set(&conn, "clock_offset_ms", "not-a-number").unwrap();
        assert_eq!(meta_get_i64(&conn, "clock_offset_ms").unwrap(), 0);
    }

    #[test]
    fn ensure_session_incarnation_mints_once_for_an_upgraded_session() {
        let conn = new_synced_test_db();
        // Production upgrade shape: a complete pre-feature session (identity,
        // epoch, capture on) with no incarnation at all.
        for (key, value) in [
            ("account_id", "account-a"),
            ("server_url", "https://sync.example.test"),
            ("device_id", "device-a"),
            ("server_epoch", "epoch-1"),
            ("capture_enabled", "1"),
        ] {
            meta_set(&conn, key, value).unwrap();
        }
        assert_eq!(read_session_incarnation(&conn).unwrap(), None);

        let minted = ensure_session_incarnation(&conn)
            .expect("mint incarnation")
            .expect("minted incarnation");

        assert_eq!(
            read_session_incarnation(&conn)
                .expect("read minted incarnation")
                .expect("minted incarnation persisted"),
            minted
        );
        // The next call is idempotent: the same incarnation is reused and no
        // second one is ever minted.
        assert_eq!(ensure_session_incarnation(&conn).unwrap(), Some(minted));
        assert_eq!(
            meta_get(&conn, SYNC_SESSION_INCARNATION_KEY)
                .unwrap()
                .as_deref()
                .and_then(|value| Uuid::parse_str(value).ok()),
            Some(minted)
        );
        // No other session or sync state was touched.
        for (key, expected) in [
            ("account_id", "account-a"),
            ("server_url", "https://sync.example.test"),
            ("device_id", "device-a"),
            ("server_epoch", "epoch-1"),
            ("capture_enabled", "1"),
        ] {
            assert_eq!(
                meta_get(&conn, key).unwrap().as_deref(),
                Some(expected),
                "metadata for {key} untouched"
            );
        }
    }

    #[test]
    fn ensure_session_incarnation_never_replaces_an_existing_incarnation() {
        let conn = new_synced_test_db();
        write_sync_session(
            &conn,
            "https://sync.example.test",
            "account-a",
            "reader@example.test",
            "device-a",
        )
        .expect("session write");
        let login_incarnation = read_session_incarnation(&conn)
            .expect("read login incarnation")
            .expect("login incarnation");

        assert_eq!(
            ensure_session_incarnation(&conn).expect("ensure over login incarnation"),
            Some(login_incarnation)
        );
        assert_eq!(
            read_session_incarnation(&conn).unwrap(),
            Some(login_incarnation)
        );

        // A manually persisted incarnation stands just as firmly.
        meta_set(
            &conn,
            SYNC_SESSION_INCARNATION_KEY,
            "33333333-3333-4333-8333-333333333333",
        )
        .expect("seed incarnation");
        assert_eq!(
            ensure_session_incarnation(&conn).expect("ensure over seeded incarnation"),
            Some(Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap())
        );

        // Even a malformed incarnation is never overwritten: it keeps surfacing
        // the read error unchanged.
        meta_set(&conn, SYNC_SESSION_INCARNATION_KEY, "not-a-uuid").expect("seed malformed");
        let error = ensure_session_incarnation(&conn).expect_err("malformed incarnation stays");
        assert!(error.contains("is not a valid UUID"), "{error}");
        assert_eq!(
            meta_get(&conn, SYNC_SESSION_INCARNATION_KEY)
                .unwrap()
                .as_deref(),
            Some("not-a-uuid")
        );
    }

    #[test]
    fn ensure_session_incarnation_stays_missing_without_a_session() {
        let conn = new_synced_test_db();
        assert_eq!(ensure_session_incarnation(&conn).unwrap(), None);
        assert_eq!(read_session_incarnation(&conn).unwrap(), None);

        // A partial identity is not a session: still nothing is minted.
        meta_set(&conn, "account_id", "account-a").unwrap();
        assert_eq!(ensure_session_incarnation(&conn).unwrap(), None);
        assert_eq!(read_session_incarnation(&conn).unwrap(), None);
    }

    #[test]
    fn ensure_session_incarnation_never_nests_inside_an_open_transaction() {
        let conn = new_synced_test_db();
        for (key, value) in [
            ("account_id", "account-a"),
            ("server_url", "https://sync.example.test"),
            ("device_id", "device-a"),
        ] {
            meta_set(&conn, key, value).unwrap();
        }

        let tx = conn.unchecked_transaction().expect("open transaction");
        assert_eq!(
            ensure_session_incarnation(&tx).expect("ensure inside transaction"),
            None,
            "no mint while a transaction is already open"
        );
        assert_eq!(
            meta_get(&tx, SYNC_SESSION_INCARNATION_KEY).unwrap(),
            None,
            "nothing written inside the open transaction"
        );
        tx.commit().expect("commit outer transaction");

        // Outside any transaction the mint runs exactly once.
        let minted = ensure_session_incarnation(&conn)
            .expect("mint after commit")
            .expect("minted incarnation");
        assert_eq!(ensure_session_incarnation(&conn).unwrap(), Some(minted));
    }

    #[test]
    fn successful_session_writes_replace_incarnation_for_the_same_identity() {
        let conn = new_synced_test_db();
        write_sync_session(
            &conn,
            "https://sync.example.test",
            "account-a",
            "reader@example.test",
            "device-a",
        )
        .expect("first session write");
        let first = read_session_incarnation(&conn)
            .expect("read first incarnation")
            .expect("first incarnation");

        write_sync_session(
            &conn,
            "https://sync.example.test",
            "account-a",
            "reader@example.test",
            "device-a",
        )
        .expect("second session write");
        let second = read_session_incarnation(&conn)
            .expect("read second incarnation")
            .expect("second incarnation");

        assert_ne!(first, second);
        assert_eq!(
            meta_get(&conn, "account_id").unwrap().as_deref(),
            Some("account-a")
        );
        assert_eq!(
            meta_get(&conn, "device_id").unwrap().as_deref(),
            Some("device-a")
        );
    }

    #[test]
    fn failed_session_write_rolls_back_incarnation_and_other_metadata() {
        let conn = new_synced_test_db();
        write_sync_session(
            &conn,
            "https://old-sync.example.test",
            "account-old",
            "old-reader@example.test",
            "device-old",
        )
        .expect("seed session");
        let original_incarnation = read_session_incarnation(&conn)
            .expect("read original incarnation")
            .expect("original incarnation");
        conn.execute_batch(
            "CREATE TEMP TRIGGER fail_session_write
             BEFORE UPDATE OF value ON sync_meta
             WHEN OLD.key = 'capture_enabled'
             BEGIN
               SELECT RAISE(ABORT, 'forced session write failure');
             END;",
        )
        .expect("failure trigger");

        let error = write_sync_session(
            &conn,
            "https://new-sync.example.test",
            "account-new",
            "new-reader@example.test",
            "device-new",
        )
        .expect_err("session write must fail");

        assert!(error.contains("forced session write failure"), "{error}");
        assert_eq!(
            read_session_incarnation(&conn).expect("read rolled-back incarnation"),
            Some(original_incarnation)
        );
        for (key, expected) in [
            ("server_url", "https://old-sync.example.test"),
            ("account_id", "account-old"),
            ("account_email", "old-reader@example.test"),
            ("device_id", "device-old"),
            ("capture_enabled", "1"),
        ] {
            assert_eq!(
                meta_get(&conn, key).unwrap().as_deref(),
                Some(expected),
                "metadata write for {key} rolled back"
            );
        }
    }

    #[test]
    fn clear_sync_state_removes_incarnation_and_preserves_unrelated_metadata() {
        let conn = new_synced_test_db();
        write_sync_session(
            &conn,
            "https://sync.example.test",
            "account-a",
            "reader@example.test",
            "device-a",
        )
        .expect("session write");
        meta_set(&conn, "unrelated_preference", "keep-me").expect("unrelated metadata");

        clear_sync_state(&conn).expect("clear sync state");

        assert_eq!(
            read_session_incarnation(&conn).expect("read cleared incarnation"),
            None
        );
        assert_eq!(
            meta_get(&conn, "unrelated_preference").unwrap().as_deref(),
            Some("keep-me")
        );
    }

    /// Simulates a fully populated session, then asserts `clear_sync_state` wipes
    /// every account-owned sync entry and resets `uploaded`, while retaining
    /// blob hashes, unrelated metadata, and manuscript data (DESIGN §6.3).
    #[test]
    fn clear_sync_state_wipes_everything_per_design_6_3() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");

        // Populate session meta, writing-owned meta, and literal-prefix decoys.
        for (k, v) in [
            ("device_id", "dev-1"),
            ("account_id", "acc-1"),
            ("account_email", "ana@x"),
            ("server_url", "https://sync.x"),
            (
                SYNC_SESSION_INCARNATION_KEY,
                "11111111-1111-4111-8111-111111111111",
            ),
            ("seeded_account", "acc-1"),
            ("server_epoch", "ep-1"),
            ("capture_enabled", "1"),
            ("last_pull_seq", "99"),
            ("clock_offset_ms", "1500"),
            ("triggers_version", "1"),
            ("writing_outbox:doc-writing", "outbox"),
            ("writing_capability", "capability"),
            ("writing_manifest:doc-writing", "manifest"),
            ("writing_pending_assets:doc-writing", "pending"),
            ("writing_catchup_epoch", "ep-1"),
            ("writing_receive:doc-writing", "queued"),
            ("writingXoutbox:doc-writing", "literal-prefix-decoy"),
            ("writingXreceive:doc-writing", "literal-prefix-decoy"),
            ("writingXmanifest:doc-writing", "literal-prefix-decoy"),
            ("writingXpendingXassets:doc-writing", "literal-prefix-decoy"),
            ("writing_capability_extra", "unrelated"),
            ("writing_catchup_epoch_extra", "unrelated"),
        ] {
            meta_set(&conn, k, v).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO sync_oplog(table_name,row_id,op,changed_at) VALUES('items','i1','U',1);
             INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','i1',5);
             INSERT INTO sync_conflicts(id,table_name,row_id,reason,created_at)
               VALUES('cf1','items','i1','lww_lost',1);
             INSERT INTO sync_pending_rows(table_name,row_id,server_seq,deleted,changed_at,device_id)
               VALUES('items','i2',7,0,1,'dev-2');
             INSERT INTO sync_pending_blobs(asset_id,sha256,rel_path,size)
               VALUES('a1','abc','assets/x',10);
             INSERT INTO sync_pending_fts(item_id) VALUES('i1');
             INSERT INTO sync_topic_aliases(remote_id,local_id) VALUES('r1','l1');
             INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded)
               VALUES('a1','deadbeef',10,1,1);
             INSERT INTO writing_documents
               (id,title,document_type,status,schema_version,current_content_json,
                revision,created_at,updated_at)
               VALUES('doc-writing','Draft','article','active',1,'{\"doc\":{}}',0,1,1);
             INSERT INTO writing_journal
               (document_id,seq,base_revision,schema_version,delta_json,checksum,created_at)
               VALUES('doc-writing',1,0,1,'[]','checksum',1);",
        )
        .expect("populate state");

        clear_sync_state(&conn).expect("clear");

        // Every per-session state table is empty.
        for table in [
            "sync_oplog",
            "sync_row_versions",
            "sync_conflicts",
            "sync_pending_rows",
            "sync_pending_blobs",
            "sync_pending_fts",
            "sync_topic_aliases",
        ] {
            assert_eq!(count(&conn, table), 0, "{table} should be empty");
        }

        // Session and writing-account metadata are gone.
        for key in SESSION_META_KEYS.iter().copied().chain([
            "writing_outbox:doc-writing",
            "writing_capability",
            "writing_manifest:doc-writing",
            "writing_pending_assets:doc-writing",
            "writing_catchup_epoch",
            "writing_receive:doc-writing",
        ]) {
            assert_eq!(
                meta_get(&conn, key).unwrap(),
                None,
                "meta key {key} cleared"
            );
        }

        // Prefix lookalikes and unrelated metadata survive.
        for key in [
            "triggers_version",
            "writingXoutbox:doc-writing",
            "writingXmanifest:doc-writing",
            "writingXpendingXassets:doc-writing",
            "writingXreceive:doc-writing",
            "writing_capability_extra",
            "writing_catchup_epoch_extra",
        ] {
            assert!(
                meta_get(&conn, key).unwrap().is_some(),
                "unrelated meta key {key} retained"
            );
        }

        // Blob index retained but uploaded reset.
        let (sha, uploaded): (String, i64) = conn
            .query_row(
                "SELECT sha256, uploaded FROM sync_blob_index WHERE asset_id='a1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("blob index row survives");
        assert_eq!(sha, "deadbeef", "blob hash retained (content-derived)");
        assert_eq!(uploaded, 0, "uploaded flag reset");

        let (content, journal_entries): (String, i64) = conn
            .query_row(
                "SELECT d.current_content_json,
                        (SELECT COUNT(*) FROM writing_journal j WHERE j.document_id = d.id)
                   FROM writing_documents d WHERE d.id = 'doc-writing'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("manuscript survives account cleanup");
        assert_eq!(content, "{\"doc\":{}}");
        assert_eq!(journal_entries, 1, "recovery journal is manuscript data");

        // All sync_* tables still exist (only data wiped).
        for table in SYNC_TABLES {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            assert!(exists, "{table} structure retained");
        }
    }

    #[test]
    fn clear_sync_state_rolls_back_writing_metadata_with_the_account_wipe() {
        let conn = new_synced_test_db();
        for (key, value) in [
            ("device_id", "dev-1"),
            (
                SYNC_SESSION_INCARNATION_KEY,
                "22222222-2222-4222-8222-222222222222",
            ),
            ("writing_outbox:doc-1", "outbox"),
            ("writing_capability", "capability"),
            ("writing_manifest:doc-1", "manifest"),
            ("writing_pending_assets:doc-1", "pending"),
            ("writing_catchup_epoch", "ep-1"),
        ] {
            meta_set(&conn, key, value).expect("seed metadata");
        }
        conn.execute_batch(
            "INSERT INTO sync_oplog(table_name,row_id,op,changed_at)
               VALUES('items','i1','U',1);
             INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded)
               VALUES('a1','deadbeef',10,1,1);
             CREATE TEMP TRIGGER fail_sync_blob_reset
             BEFORE UPDATE OF uploaded ON sync_blob_index
             WHEN OLD.asset_id = 'a1'
             BEGIN
               SELECT RAISE(ABORT, 'forced blob reset failure');
             END;",
        )
        .expect("seed rollback fixture");

        let error = clear_sync_state(&conn).expect_err("forced reset failure");

        assert!(error.contains("forced blob reset failure"), "{error}");
        assert_eq!(count(&conn, "sync_oplog"), 1, "table delete rolled back");
        for key in [
            "device_id",
            SYNC_SESSION_INCARNATION_KEY,
            "writing_outbox:doc-1",
            "writing_capability",
            "writing_manifest:doc-1",
            "writing_pending_assets:doc-1",
            "writing_catchup_epoch",
        ] {
            assert!(
                meta_get(&conn, key).unwrap().is_some(),
                "metadata delete for {key} rolled back"
            );
        }
        let uploaded: i64 = conn
            .query_row(
                "SELECT uploaded FROM sync_blob_index WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("blob row survives");
        assert_eq!(uploaded, 1, "blob reset rolled back");
    }

    #[test]
    fn a_dev_profile_never_uses_the_real_keyring_entry() {
        let real = token_target(None);
        assert_eq!(
            real,
            (SYNC_KEYRING_SERVICE.to_string(), TOKEN_KEY.to_string())
        );
        let a = token_target(Some("alpha"));
        let b = token_target(Some("beta"));
        assert_ne!(
            a, real,
            "a profile must not read or replace the real session"
        );
        assert_ne!(a.0, real.0, "even the service differs");
        assert_ne!(a.1, real.1);
        assert_ne!(a, b, "two profiles never share a session");
        assert!(a.1.contains("alpha") && b.1.contains("beta"));
    }

    #[test]
    fn default_device_name_is_non_empty() {
        assert!(!default_device_name().is_empty());
        assert!(!platform_label().is_empty());
    }
}
