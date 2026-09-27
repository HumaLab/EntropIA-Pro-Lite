//! Sync session lifecycle (DESIGN §8, §6.3) and `sync_meta` typed accessors.
//!
//! Holds the register/login/logout Tauri commands plus the small helpers the
//! push/pull slices use to read and write `sync_meta` with the right types. The
//! device token lives ONLY in the OS keyring (never in `sync_meta`, never in
//! `app_settings`, never logged — DESIGN §8), under a service name distinct from
//! the app-settings keyring so sync credentials can be wiped independently.

use rusqlite::Connection;
use tauri::{AppHandle, State};

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

// ---------------------------------------------------------------------------
// Token keyring helpers (DESIGN §8 — token NEVER touches SQLite or logs)
// ---------------------------------------------------------------------------

fn token_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SYNC_KEYRING_SERVICE, TOKEN_KEY)
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
    "sync_pending_fts",
    "sync_topic_aliases",
];

/// Wipes ALL local sync state per DESIGN §6.3, in one transaction:
/// delete the per-session state tables, clear the session `sync_meta` keys, and
/// reset `uploaded=0` across the WHOLE `sync_blob_index` (hashes survive — they
/// are content-derived). Does NOT touch the keyring; callers handle the token
/// separately (revoke remote first, then [`delete_token`]).
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
/// `server_url` in `sync_meta`, and turns capture ON (`capture_enabled='1'`,
/// DESIGN §4.1). The seeding (DESIGN §4.5) is performed later by the engine, not
/// here. The token is never logged (DESIGN §8).
#[tauri::command]
pub async fn sync_login(
    server_url: String,
    email: String,
    password: String,
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<(), String> {
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
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("[sync] failed to begin login transaction: {e}"))?;
        meta_set(&tx, "server_url", &validated_url)?;
        meta_set(&tx, "account_id", &account_id)?;
        meta_set(&tx, "account_email", &email)?;
        meta_set(&tx, "device_id", &device_id)?;
        meta_set(&tx, "capture_enabled", "1")?;
        tx.commit()
            .map_err(|e| format!("[sync] failed to commit login session: {e}"))
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

    /// Simulates a fully populated session, then asserts `clear_sync_state` wipes
    /// every state table + session key and resets `uploaded`, while retaining
    /// blob hashes and not touching app data (DESIGN §6.3).
    #[test]
    fn clear_sync_state_wipes_everything_per_design_6_3() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");

        // Populate session meta and every state table.
        for (k, v) in [
            ("device_id", "dev-1"),
            ("account_id", "acc-1"),
            ("account_email", "ana@x"),
            ("server_url", "https://sync.x"),
            ("seeded_account", "acc-1"),
            ("server_epoch", "ep-1"),
            ("capture_enabled", "1"),
            ("last_pull_seq", "99"),
            ("clock_offset_ms", "1500"),
            ("triggers_version", "1"),
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
               VALUES('a1','deadbeef',10,1,1);",
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

        // Session meta keys are gone.
        for key in SESSION_META_KEYS {
            assert_eq!(
                meta_get(&conn, key).unwrap(),
                None,
                "meta key {key} cleared"
            );
        }

        // triggers_version (NOT a session key) survives.
        assert_eq!(
            meta_get(&conn, "triggers_version").unwrap().as_deref(),
            Some("1"),
            "non-session meta retained"
        );

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
    fn default_device_name_is_non_empty() {
        assert!(!default_device_name().is_empty());
        assert!(!platform_label().is_empty());
    }
}
