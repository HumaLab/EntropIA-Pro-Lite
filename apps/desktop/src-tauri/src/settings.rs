use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

use crate::db::state::AppDbState;
// Bootstrap remote-source plumbing is local-ml only (the hosted managed-runtime). The
// type comes from the always-compiled `bootstrap_types`; the import + every fn below is
// gated because only the (gated) RuntimeManager consumes them.
#[cfg(feature = "local-ml")]
use crate::runtime::bootstrap_types::BootstrapRemoteSource;

const OPENROUTER_MODEL_KEY: &str = "openrouter_model";
const LEGACY_DEFAULT_OPENROUTER_MODEL: &str = "google/gemma-3-4b-it";
pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "google/gemma-4-26b-a4b-it";

// Runtime-bootstrap setting keys: consumed by the local-ml/paddle runtime
// manager and the settings redaction/migration paths, none of which compile in
// the lean lib build. Keep them available (tests and other variants use them)
// and allow them to be unused rather than cfg-gating the shared keys.
#[allow(dead_code)]
pub const RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY: &str = "runtime_bootstrap_manifest_url";
#[allow(dead_code)]
pub const RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY: &str = "runtime_bootstrap_public_key_id";
#[allow(dead_code)]
pub const RUNTIME_BOOTSTRAP_PUBLIC_KEY_KEY_PREFIX: &str = "runtime_bootstrap_public_key.";
const REDACTED_SETTING_VALUE: &str = "[redacted]";
const SECRET_REF_PREFIX: &str = "secret_ref:";
const APP_CREDENTIAL_SERVICE: &str = "com.entropia.desktop credentials";
pub const OPENROUTER_API_KEY: &str = "openrouter_api_key";
pub const ASSEMBLYAI_API_KEY: &str = "assemblyai_api_key";
pub const GLM_OCR_API_KEY: &str = "glm_ocr_api_key";
pub const ZOTERO_API_KEY: &str = "zotero_api_key";
/// The Zotero account id the stored key was last verified for. Not a secret, but
/// only meaningful next to that key, so a new or cleared key forgets it.
pub const ZOTERO_USER_ID_KEY: &str = "zotero_user_id";
const SECRET_SETTING_KEYS: [&str; 4] = [
    OPENROUTER_API_KEY,
    ASSEMBLYAI_API_KEY,
    GLM_OCR_API_KEY,
    ZOTERO_API_KEY,
];
static APP_CREDENTIAL_LOCK: Mutex<()> = Mutex::new(());
#[cfg(feature = "local-ml")]
const BUILTIN_RUNTIME_BOOTSTRAP_MANIFEST_URL_ENV: &str = "ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL";
#[cfg(feature = "local-ml")]
const BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_ENV: &str =
    "ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID";
#[cfg(feature = "local-ml")]
const BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64_ENV: &str =
    "ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64";

async fn invalidate_dependency_probe_cache_if_needed(
    key: &str,
    deps: Option<&State<'_, crate::deps::DepsState>>,
) {
    if crate::deps::should_invalidate_cache_for_setting(key) {
        if let Some(deps_state) = deps {
            crate::deps::invalidate_probe_cache(deps_state.inner()).await;
        }
        #[cfg(feature = "local-ml")]
        crate::python_discovery::invalidate_probe_cache();
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize)]
pub struct SettingEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SecretMigrationReport {
    pub migrated: usize,
    pub failed_keys: Vec<String>,
    pub storage_cleanup_failed: bool,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn settings_get(
    key: String,
    db: State<'_, AppDbState>,
) -> Result<Option<String>, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock error: {e}"))?;
    let result = get_raw_setting(&conn, &key);
    Ok(result.map(|value| ipc_setting_value(&key, &value)))
}

#[tauri::command]
pub async fn settings_set(
    key: String,
    value: String,
    db: State<'_, AppDbState>,
    deps: State<'_, crate::deps::DepsState>,
) -> Result<(), String> {
    let should_invalidate = crate::deps::should_invalidate_cache_for_setting(&key);
    {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        persist_setting(&conn, &key, &value)?;
        resume_work_after_setting_change(&conn, &key);
    }
    if should_invalidate {
        invalidate_dependency_probe_cache_if_needed(&key, Some(&deps)).await;
    }
    Ok(())
}

/// Settings that decide whether the embedding engine can initialize.
fn is_embedding_engine_setting(key: &str) -> bool {
    key == OPENROUTER_API_KEY
        || key == crate::nlp::embeddings::EMBEDDING_PROVIDER_SETTING_KEY
        || key == crate::nlp::embeddings::OPENROUTER_EMBEDDING_MODEL_SETTING_KEY
}

/// Requeues queue units parked `configuration_required` when a saved setting
/// is one the embedding engine depends on and the configuration is now valid.
/// Best effort: a failure is logged and never fails the save. Returns how many
/// units were requeued.
pub(crate) fn resume_work_after_setting_change(conn: &rusqlite::Connection, key: &str) -> usize {
    if !is_embedding_engine_setting(key) {
        return 0;
    }
    match crate::processing::repository::resume_embedding_configuration_blocked(conn) {
        Ok(resumed) => {
            if resumed > 0 {
                eprintln!(
                    "[settings] Resumed {resumed} queue unit(s) blocked on embedding configuration"
                );
            }
            resumed
        }
        Err(error) => {
            eprintln!("[settings] Could not resume configuration-blocked work: {error}");
            0
        }
    }
}

#[tauri::command]
pub async fn settings_get_all(db: State<'_, AppDbState>) -> Result<Vec<SettingEntry>, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock error: {e}"))?;
    read_visible_settings(&conn)
}

/// Every setting the Settings UI may see: the whole `app_settings` table
/// with secrets redacted, minus internal bookkeeping state — which is not
/// user configuration and must never render or break there. The command
/// delegates here so tests can pin the visibility contract.
pub fn read_visible_settings(conn: &rusqlite::Connection) -> Result<Vec<SettingEntry>, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM app_settings ORDER BY key")
        .map_err(|e| format!("Failed to prepare settings query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            let entry = SettingEntry {
                key: row.get(0)?,
                value: row.get(1)?,
            };
            Ok(redact_setting_entry(entry))
        })
        .map_err(|e| format!("Failed to query settings: {e}"))?;
    Ok(rows
        .flatten()
        .filter(|entry| !is_internal_state_key(&entry.key))
        .collect())
}

/// Local bookkeeping state, not user configuration: hidden from the
/// Settings UI's bulk read. Currently just the recently-opened works list.
fn is_internal_state_key(key: &str) -> bool {
    key == RECENTLY_OPENED_SETTING_KEY
}

fn redact_setting_entry(entry: SettingEntry) -> SettingEntry {
    if is_sensitive_setting_key(&entry.key) {
        SettingEntry {
            key: entry.key,
            value: REDACTED_SETTING_VALUE.to_string(),
        }
    } else {
        entry
    }
}

fn is_sensitive_setting_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.ends_with("_api_key")
        || normalized.contains("secret")
        || normalized.ends_with("token")
        || normalized.contains("password")
        || normalized.contains("credential")
}

#[tauri::command]
pub async fn settings_delete(
    key: String,
    db: State<'_, AppDbState>,
    deps: State<'_, crate::deps::DepsState>,
) -> Result<(), String> {
    let should_invalidate = crate::deps::should_invalidate_cache_for_setting(&key);
    {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        // DB row first: if the keyring delete then fails, only an orphaned
        // keyring entry remains (no dangling secret_ref pointing at nothing).
        conn.execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![key.as_str()],
        )
        .map_err(|e| format!("Failed to delete setting: {e}"))?;
        forget_zotero_user_id_for(&conn, &key);
        if is_secret_setting_key(&key) {
            if let Err(error) = delete_secret(&key) {
                eprintln!("[settings] Setting row deleted but credential cleanup failed: {error}");
            }
        }
    }
    if should_invalidate {
        invalidate_dependency_probe_cache_if_needed(&key, Some(&deps)).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers (for Rust-side reading, used by LLM worker)
// ---------------------------------------------------------------------------

/// Read a setting value directly from a rusqlite connection.
/// Used by the LLM worker to read API keys without going through Tauri state.
pub fn get_setting(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    let has_secret_ref = get_raw_setting(conn, key)
        .map(|value| is_secret_setting_key(key) && value.starts_with(SECRET_REF_PREFIX))
        .unwrap_or(false);
    match resolve_setting_with(conn, key, read_secret) {
        Ok(value) => {
            if has_secret_ref && value.is_none() {
                eprintln!(
                    "[settings] Protected setting '{key}' references a missing credential store entry"
                );
            }
            value
        }
        Err(error) => {
            eprintln!("[settings] Failed to resolve protected setting '{key}': {error}");
            None
        }
    }
}

pub fn resolve_api_key_input(
    conn: &rusqlite::Connection,
    key: &str,
    provided: &str,
) -> Result<String, String> {
    let provided = provided.trim();
    if !provided.is_empty() {
        return Ok(provided.to_string());
    }
    get_setting(conn, key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("No protected credential is configured for '{key}'"))
}

fn resolve_setting_with(
    conn: &rusqlite::Connection,
    key: &str,
    read: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    let Some(stored) = get_raw_setting(conn, key) else {
        return Ok(None);
    };
    if !is_secret_setting_key(key) || !stored.starts_with(SECRET_REF_PREFIX) {
        return Ok(Some(stored));
    }
    read(key)
}

pub(crate) fn get_raw_setting(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

fn ipc_setting_value(key: &str, value: &str) -> String {
    if !is_secret_setting_key(key) || value.trim().is_empty() {
        return value.to_string();
    }
    if value.starts_with(SECRET_REF_PREFIX) {
        secret_reference(key)
    } else {
        // Legacy plaintext that failed keyring migration: never echo the value
        // over IPC, but mark it distinctly so the UI does not claim it lives in
        // the system credential store.
        format!("legacy_ref:{key}")
    }
}

fn secret_reference(key: &str) -> String {
    format!("{SECRET_REF_PREFIX}{key}")
}

fn is_secret_setting_key(key: &str) -> bool {
    SECRET_SETTING_KEYS.contains(&key)
}

/// Marker the frontend recognises (src/lib/settings.ts) to explain a missing or
/// empty system credential store in plain words instead of a DBus error. Seen on
/// Linux without a Secret Service provider, or with one but no default keyring
/// (WSL, minimal installs, desktops without gnome-keyring or KWallet).
pub const CREDENTIAL_STORE_UNAVAILABLE: &str = "credential_store_unavailable";

/// Whether a keyring failure means there is no usable credential store at all,
/// as opposed to a problem with one entry.
pub fn is_credential_store_unavailable(error: &keyring::Error) -> bool {
    matches!(
        error,
        keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_)
    )
}

fn describe_credential_error(action: &str, key: &str, error: &keyring::Error) -> String {
    let message = format!("Could not {action} protected setting '{key}': {error}");
    if is_credential_store_unavailable(error) {
        format!("{CREDENTIAL_STORE_UNAVAILABLE}: {message}")
    } else {
        message
    }
}

fn credential_entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(APP_CREDENTIAL_SERVICE, key)
        .map_err(|error| format!("Could not open the system credential store: {error}"))
}

fn store_secret(key: &str, value: &str) -> Result<(), String> {
    let _guard = APP_CREDENTIAL_LOCK
        .lock()
        .map_err(|_| "System credential store lock is unavailable".to_string())?;
    credential_entry(key)?
        .set_password(value)
        .map_err(|error| describe_credential_error("store", key, &error))
}

fn read_secret(key: &str) -> Result<Option<String>, String> {
    let _guard = APP_CREDENTIAL_LOCK
        .lock()
        .map_err(|_| "System credential store lock is unavailable".to_string())?;
    match credential_entry(key)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(describe_credential_error("read", key, &error)),
    }
}

fn delete_secret(key: &str) -> Result<(), String> {
    let _guard = APP_CREDENTIAL_LOCK
        .lock()
        .map_err(|_| "System credential store lock is unavailable".to_string())?;
    match credential_entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "Could not delete protected setting '{key}' from the system credential store: {error}"
        )),
    }
}

/// A Zotero user id belongs to one key: when that key is replaced or removed the
/// id is stale, and the next verification writes it again.
fn forget_zotero_user_id_for(conn: &rusqlite::Connection, key: &str) {
    if key == ZOTERO_API_KEY {
        let _ = conn.execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![ZOTERO_USER_ID_KEY],
        );
    }
}

pub(crate) fn persist_setting(
    conn: &rusqlite::Connection,
    key: &str,
    value: &str,
) -> Result<(), String> {
    persist_setting_with(conn, key, value, store_secret, delete_secret)
}

/// [`persist_setting`] with the credential store passed in, so tests never
/// write to or delete from the real system keyring.
fn persist_setting_with(
    conn: &rusqlite::Connection,
    key: &str,
    value: &str,
    store: impl FnOnce(&str, &str) -> Result<(), String>,
    delete: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    forget_zotero_user_id_for(conn, key);
    if is_secret_setting_key(key) {
        if value.trim().is_empty() {
            conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])
                .map_err(|error| format!("Failed to delete empty protected setting: {error}"))?;
            if let Err(error) = delete(key) {
                eprintln!("[settings] Setting row deleted but credential cleanup failed: {error}");
            }
            return Ok(());
        }
        store(key, value)?;
        conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
            params![key, secret_reference(key)],
        )
        .map_err(|error| format!("Failed to save protected setting reference: {error}"))?;
        return Ok(());
    }

    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|error| format!("Failed to save setting: {error}"))?;
    Ok(())
}

pub fn migrate_legacy_api_keys(conn: &rusqlite::Connection) -> SecretMigrationReport {
    migrate_legacy_api_keys_with(conn, store_secret)
}

fn migrate_legacy_api_keys_with(
    conn: &rusqlite::Connection,
    mut store: impl FnMut(&str, &str) -> Result<(), String>,
) -> SecretMigrationReport {
    let mut report = SecretMigrationReport::default();
    let _ = conn.execute_batch("PRAGMA secure_delete = ON;");
    for key in SECRET_SETTING_KEYS {
        let Some(value) = get_raw_setting(conn, key) else {
            continue;
        };
        if value.trim().is_empty() || value.starts_with(SECRET_REF_PREFIX) {
            continue;
        }

        let result = store(key, &value).and_then(|()| {
            conn.execute(
                "UPDATE app_settings SET value = ?1 WHERE key = ?2",
                params![secret_reference(key), key],
            )
            .map(|_| ())
            .map_err(|error| format!("Failed to replace legacy protected setting: {error}"))
        });
        match result {
            Ok(()) => report.migrated += 1,
            Err(_) => report.failed_keys.push(key.to_string()),
        }
    }
    if report.migrated > 0
        && conn
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE);
                 VACUUM;
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .is_err()
    {
        report.storage_cleanup_failed = true;
    }
    report
}

/// Persist a setting value directly from Rust-side worker code.
///
/// Generic setting writer used by the paddle/local-ml write paths; in the lean
/// lib build no caller is compiled in, but it stays available (tests and other
/// build variants use it), so allow it to be unused rather than cfg-gating.
#[allow(dead_code)]
pub fn set_setting(
    conn: &rusqlite::Connection,
    key: &str,
    value: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

/// Delete a setting directly from Rust-side worker code.
#[allow(dead_code)]
pub fn delete_setting(conn: &rusqlite::Connection, key: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Recently opened works (the "opened works first" admission order)
// ---------------------------------------------------------------------------

/// Local-only bookkeeping key: the works the user opened, most recent
/// first. `app_settings` never syncs (it is not in
/// `sync::capture::SYNCED_TABLES`) and this key is hidden from the Settings
/// UI (see [`read_visible_settings`]).
pub const RECENTLY_OPENED_SETTING_KEY: &str = "bibliography_recently_opened";

/// The recently-opened list never grows past this many works.
pub const RECENTLY_OPENED_MAX_ENTRIES: usize = 500;

/// One `{ "itemId", "openedAt" }` entry of the recently-opened list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentlyOpenedEntry {
    pub item_id: String,
    pub opened_at: i64,
}

/// Records that the user opened one work at `now_ms` — the "opened works
/// first" order the library sync admits derived work in.
///
/// The list stays most-recent-first, deduplicated by work (a re-open moves
/// the work to the front with the new timestamp) and capped at
/// [`RECENTLY_OPENED_MAX_ENTRIES`]. A corrupt stored list is replaced,
/// never propagated. This is bookkeeping, not user data: read commands use
/// [`record_work_opened_best_effort`] so a failure never surfaces.
pub fn record_work_opened(
    conn: &rusqlite::Connection,
    item_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    if item_id.trim().is_empty() {
        return Err("invalid_input: an opened work needs a non-empty item id".to_string());
    }
    let mut entries = recently_opened_entries(conn);
    entries.retain(|entry| entry.item_id != item_id);
    entries.insert(
        0,
        RecentlyOpenedEntry {
            item_id: item_id.to_string(),
            opened_at: now_ms,
        },
    );
    entries.truncate(RECENTLY_OPENED_MAX_ENTRIES);
    let value = serde_json::to_string(&entries)
        .map_err(|error| format!("Failed to encode the recently-opened works: {error}"))?;
    set_setting(conn, RECENTLY_OPENED_SETTING_KEY, &value)
        .map_err(|error| format!("Failed to save the recently-opened works: {error}"))
}

/// [`record_work_opened`] for read commands: a bookkeeping failure is
/// logged and never fails the read it decorates.
pub fn record_work_opened_best_effort(conn: &rusqlite::Connection, item_id: &str, now_ms: i64) {
    if let Err(error) = record_work_opened(conn, item_id, now_ms) {
        eprintln!("[settings] Could not record opened work {item_id}: {error}");
    }
}

/// The recently-opened list, most recent first. A missing or corrupt list
/// reads as empty (logged): bookkeeping never breaks a reader.
pub fn recently_opened_entries(conn: &rusqlite::Connection) -> Vec<RecentlyOpenedEntry> {
    let Some(raw) = get_raw_setting(conn, RECENTLY_OPENED_SETTING_KEY) else {
        return Vec::new();
    };
    match serde_json::from_str(&raw) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("[settings] Discarding a corrupt recently-opened works list: {error}");
            Vec::new()
        }
    }
}

/// The open rank of every recently-opened work: 0 is the most recently
/// opened. Works missing from the map were not opened recently and keep
/// their current order wherever this ranks.
pub fn recently_opened_ranks(conn: &rusqlite::Connection) -> HashMap<String, usize> {
    recently_opened_entries(conn)
        .into_iter()
        .enumerate()
        .map(|(rank, entry)| (entry.item_id, rank))
        .collect()
}

/// One-shot rename of the stale default OpenRouter model. Rows whose
/// `openrouter_model` value still equals the legacy default get bumped to the
/// new default; user-customized values are untouched (the WHERE clause only
/// matches the exact legacy string). Feature-agnostic — runs once at setup.
pub fn migrate_legacy_default_openrouter_model(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE app_settings SET value = ?1 WHERE key = ?2 AND value = ?3",
        rusqlite::params![
            DEFAULT_OPENROUTER_MODEL,
            OPENROUTER_MODEL_KEY,
            LEGACY_DEFAULT_OPENROUTER_MODEL
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("Failed to migrate default OpenRouter model: {e}"))
}

#[cfg(feature = "local-ml")]
pub fn get_runtime_bootstrap_remote_source(
    conn: &rusqlite::Connection,
) -> Result<Option<BootstrapRemoteSource>, String> {
    get_runtime_bootstrap_remote_source_with_builtin(
        conn,
        option_env!("ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL"),
        option_env!("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID"),
        option_env!("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64"),
    )
}

#[cfg(feature = "local-ml")]
fn get_runtime_bootstrap_remote_source_with_builtin(
    conn: &rusqlite::Connection,
    builtin_manifest_url: Option<&str>,
    builtin_public_key_id: Option<&str>,
    builtin_public_key_base64: Option<&str>,
) -> Result<Option<BootstrapRemoteSource>, String> {
    let manifest_url = get_setting(conn, RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let public_key_id = get_setting(conn, RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    match runtime_bootstrap_source_from_values(
        manifest_url,
        public_key_id,
        "Remote bootstrap source",
    )? {
        Some(source) => Ok(Some(source)),
        None => builtin_runtime_bootstrap_remote_source(
            builtin_manifest_url,
            builtin_public_key_id,
            builtin_public_key_base64,
        ),
    }
}

#[cfg(feature = "local-ml")]
fn runtime_bootstrap_source_from_values(
    manifest_url: Option<String>,
    public_key_id: Option<String>,
    context: &str,
) -> Result<Option<BootstrapRemoteSource>, String> {
    match (manifest_url, public_key_id) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(format!(
            "{context} is partially configured: missing public key id"
        )),
        (None, Some(_)) => Err(format!(
            "{context} is partially configured: missing manifest URL"
        )),
        (Some(manifest_url), Some(public_key_id)) => {
            if !manifest_url.starts_with("https://") {
                return Err(format!(
                    "{context} manifest URL must use HTTPS to be considered trusted"
                ));
            }

            Ok(Some(BootstrapRemoteSource {
                manifest_url,
                public_key_id,
            }))
        }
    }
}

#[cfg(feature = "local-ml")]
fn builtin_runtime_bootstrap_remote_source(
    builtin_manifest_url: Option<&str>,
    builtin_public_key_id: Option<&str>,
    builtin_public_key_base64: Option<&str>,
) -> Result<Option<BootstrapRemoteSource>, String> {
    let manifest_url = trimmed_optional(builtin_manifest_url);
    let public_key_id = trimmed_optional(builtin_public_key_id);
    let public_key_base64 = trimmed_optional(builtin_public_key_base64);

    if manifest_url.is_none() && public_key_id.is_none() && public_key_base64.is_none() {
        return Ok(None);
    }

    if manifest_url.is_none() && public_key_id.is_none() {
        return Err(format!(
            "Built-in remote bootstrap source is partially configured: missing {BUILTIN_RUNTIME_BOOTSTRAP_MANIFEST_URL_ENV} and {BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_ENV}"
        ));
    }

    if manifest_url.is_some() && public_key_id.is_none() {
        return Err(format!(
            "Built-in remote bootstrap source is partially configured: missing {BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_ENV}"
        ));
    }

    if manifest_url.is_none() && public_key_id.is_some() {
        return Err(format!(
            "Built-in remote bootstrap source is partially configured: missing {BUILTIN_RUNTIME_BOOTSTRAP_MANIFEST_URL_ENV}"
        ));
    }

    if public_key_base64.is_none() {
        return Err(format!(
            "Built-in remote bootstrap source is partially configured: missing {BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64_ENV}"
        ));
    }

    runtime_bootstrap_source_from_values(
        manifest_url,
        public_key_id,
        "Built-in remote bootstrap source",
    )
}

#[cfg(feature = "local-ml")]
fn trimmed_optional(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "local-ml")]
pub fn get_runtime_bootstrap_public_key(
    conn: &rusqlite::Connection,
    public_key_id: &str,
) -> Result<String, String> {
    get_runtime_bootstrap_public_key_with_builtin(
        conn,
        public_key_id,
        option_env!("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID"),
        option_env!("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64"),
    )
}

#[cfg(feature = "local-ml")]
fn get_runtime_bootstrap_public_key_with_builtin(
    conn: &rusqlite::Connection,
    public_key_id: &str,
    builtin_public_key_id: Option<&str>,
    builtin_public_key_base64: Option<&str>,
) -> Result<String, String> {
    let key = format!("{RUNTIME_BOOTSTRAP_PUBLIC_KEY_KEY_PREFIX}{public_key_id}");
    if let Some(configured_key) = get_setting(conn, &key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(configured_key);
    }

    if trimmed_optional(builtin_public_key_id).as_deref() == Some(public_key_id) {
        if let Some(public_key) = trimmed_optional(builtin_public_key_base64) {
            return Ok(public_key);
        }
        return Err(format!(
            "Bootstrap public key '{public_key_id}' is selected by {BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_ENV}, but {BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64_ENV} is not configured"
        ));
    }

    Err(format!(
        "Bootstrap public key '{public_key_id}' is not configured"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn in_memory_settings_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("create app_settings");
        conn
    }

    fn boxed(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
        message.to_string().into()
    }

    // WSL, 2026-09-26: no Secret Service at all, then one with no default
    // keyring. Both must reach the UI as "no credential store", not as DBus.
    #[test]
    fn a_missing_or_empty_credential_store_is_reported_as_unavailable() {
        let no_service = keyring::Error::PlatformFailure(boxed(
            "DBus error: The name org.freedesktop.secrets was not provided by any .service files",
        ));
        let no_default = keyring::Error::NoStorageAccess(boxed("Secret Service: no result found"));
        for error in [no_service, no_default] {
            let message = describe_credential_error("store", GLM_OCR_API_KEY, &error);
            assert!(
                message.starts_with(CREDENTIAL_STORE_UNAVAILABLE),
                "{message}"
            );
            assert!(message.contains(GLM_OCR_API_KEY), "{message}");
        }
    }

    #[test]
    fn other_credential_errors_keep_their_own_message() {
        let error = keyring::Error::TooLong("password".into(), 10);
        let message = describe_credential_error("store", GLM_OCR_API_KEY, &error);
        assert!(
            !message.starts_with(CREDENTIAL_STORE_UNAVAILABLE),
            "{message}"
        );
        assert!(
            message.starts_with("Could not store protected setting"),
            "{message}"
        );
    }

    #[test]
    fn redact_setting_entry_hides_sensitive_values_for_bulk_reads() {
        for key in [
            "openrouter_api_key",
            "assemblyai_api_key",
            "glm_ocr_api_key",
            "provider_secret",
            "refresh_token",
            "account_password",
            "cloud_credential",
        ] {
            let entry = redact_setting_entry(SettingEntry {
                key: key.to_string(),
                value: "super-secret-value".to_string(),
            });

            assert_eq!(entry.key, key);
            assert_eq!(entry.value, REDACTED_SETTING_VALUE);
        }
    }

    #[test]
    fn redact_setting_entry_keeps_non_sensitive_values_for_bulk_reads() {
        for (key, value) in [
            ("openrouter_model", "google/gemma-3-4b-it"),
            ("llm_ner_max_tokens", "4096"),
            ("llm_summary_max_tokens", "512"),
        ] {
            let entry = redact_setting_entry(SettingEntry {
                key: key.to_string(),
                value: value.to_string(),
            });

            assert_eq!(entry.key, key);
            assert_eq!(entry.value, value);
        }
    }

    #[test]
    fn ipc_reads_never_return_plaintext_api_keys() {
        assert_eq!(
            ipc_setting_value(OPENROUTER_API_KEY, "secret_ref:openrouter_api_key"),
            "secret_ref:openrouter_api_key"
        );
        assert_eq!(
            ipc_setting_value(OPENROUTER_API_KEY, "sk-plaintext"),
            "legacy_ref:openrouter_api_key",
            "unmigrated plaintext must be marked as legacy, never echoed"
        );
        assert_eq!(
            ipc_setting_value("openrouter_model", "google/gemma"),
            "google/gemma"
        );
    }

    #[test]
    fn internal_reads_resolve_secret_references_without_exposing_them() {
        let conn = in_memory_settings_db();
        set_setting(&conn, OPENROUTER_API_KEY, "secret_ref:openrouter_api_key")
            .expect("save reference");

        let value = resolve_setting_with(&conn, OPENROUTER_API_KEY, |key| {
            assert_eq!(key, OPENROUTER_API_KEY);
            Ok(Some("sk-protected".to_string()))
        })
        .expect("resolve protected setting");

        assert_eq!(value.as_deref(), Some("sk-protected"));
    }

    #[test]
    fn explicit_api_key_input_takes_precedence_over_stored_value() {
        let conn = in_memory_settings_db();
        set_setting(&conn, OPENROUTER_API_KEY, "legacy-stored").expect("save key");

        assert_eq!(
            resolve_api_key_input(&conn, OPENROUTER_API_KEY, "  explicit  ")
                .expect("resolve explicit key"),
            "explicit"
        );
    }

    #[test]
    fn migration_replaces_plaintext_only_after_secret_store_succeeds() {
        let conn = in_memory_settings_db();
        set_setting(&conn, OPENROUTER_API_KEY, "sk-openrouter").expect("save legacy key");
        set_setting(&conn, ASSEMBLYAI_API_KEY, "assembly-key").expect("save legacy key");
        let mut stored = Vec::new();

        let report = migrate_legacy_api_keys_with(&conn, |key, value| {
            stored.push((key.to_string(), value.to_string()));
            (key != ASSEMBLYAI_API_KEY)
                .then_some(())
                .ok_or_else(|| "credential store unavailable".to_string())
        });

        assert_eq!(report.migrated, 1);
        assert_eq!(report.failed_keys, vec![ASSEMBLYAI_API_KEY.to_string()]);
        assert_eq!(stored.len(), 2);
        assert_eq!(
            get_raw_setting(&conn, OPENROUTER_API_KEY).as_deref(),
            Some("secret_ref:openrouter_api_key")
        );
        assert_eq!(
            get_raw_setting(&conn, ASSEMBLYAI_API_KEY).as_deref(),
            Some("assembly-key"),
            "failed migration must preserve the legacy value"
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn returns_none_when_runtime_bootstrap_source_is_not_configured() {
        let conn = in_memory_settings_db();

        let source = get_runtime_bootstrap_remote_source_with_builtin(&conn, None, None, None)
            .expect("lookup should succeed");

        assert_eq!(source, None);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn loads_runtime_bootstrap_source_from_builtin_defaults_when_settings_are_empty() {
        let conn = in_memory_settings_db();

        let source = get_runtime_bootstrap_remote_source_with_builtin(
            &conn,
            Some("https://example.com/runtime/bootstrap.json"),
            Some("entropia-root"),
            Some("base64-public-key"),
        )
        .expect("built-in source should load");

        assert_eq!(
            source,
            Some(BootstrapRemoteSource {
                manifest_url: "https://example.com/runtime/bootstrap.json".to_string(),
                public_key_id: "entropia-root".to_string(),
            })
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn rejects_partially_configured_builtin_runtime_bootstrap_source() {
        let conn = in_memory_settings_db();

        let error = get_runtime_bootstrap_remote_source_with_builtin(
            &conn,
            Some("https://example.com/runtime/bootstrap.json"),
            Some("entropia-root"),
            None,
        )
        .expect_err("partial built-in config must fail");

        assert!(error.contains("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn rejects_stray_builtin_runtime_bootstrap_public_key_without_source() {
        let conn = in_memory_settings_db();

        let error = get_runtime_bootstrap_remote_source_with_builtin(
            &conn,
            None,
            None,
            Some("base64-public-key"),
        )
        .expect_err("stray built-in key must fail");

        assert!(error.contains("ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL"));
        assert!(error.contains("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn loads_runtime_bootstrap_source_from_settings_when_complete_and_https() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY,
            "https://example.com/runtime/bootstrap.json",
        )
        .expect("save manifest url");
        set_setting(&conn, RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY, "entropia-root")
            .expect("save public key id");

        let source = get_runtime_bootstrap_remote_source(&conn).expect("lookup should succeed");

        assert_eq!(
            source,
            Some(BootstrapRemoteSource {
                manifest_url: "https://example.com/runtime/bootstrap.json".to_string(),
                public_key_id: "entropia-root".to_string(),
            })
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn rejects_partially_configured_runtime_bootstrap_source() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY,
            "https://example.com/runtime/bootstrap.json",
        )
        .expect("save manifest url");

        let error =
            get_runtime_bootstrap_remote_source(&conn).expect_err("partial config must fail");

        assert!(error.contains("missing public key id"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn rejects_non_https_runtime_bootstrap_source() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY,
            "http://example.com/runtime/bootstrap.json",
        )
        .expect("save manifest url");
        set_setting(&conn, RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY, "entropia-root")
            .expect("save public key id");

        let error =
            get_runtime_bootstrap_remote_source(&conn).expect_err("non-https config must fail");

        assert!(error.contains("HTTPS"));
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn loads_runtime_bootstrap_public_key_by_key_id() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            "runtime_bootstrap_public_key.entropia-root",
            "base64-public-key",
        )
        .expect("save key");

        let public_key =
            get_runtime_bootstrap_public_key_with_builtin(&conn, "entropia-root", None, None)
                .expect("public key should load");

        assert_eq!(public_key, "base64-public-key");
    }

    #[test]
    fn the_zotero_key_is_a_protected_setting_and_redacted_in_bulk_reads() {
        assert!(is_secret_setting_key(ZOTERO_API_KEY));
        assert_eq!(
            ipc_setting_value(ZOTERO_API_KEY, "secret_ref:zotero_api_key"),
            "secret_ref:zotero_api_key"
        );
        let entry = redact_setting_entry(SettingEntry {
            key: ZOTERO_API_KEY.to_string(),
            value: "secret_ref:zotero_api_key".to_string(),
        });
        assert_eq!(entry.value, REDACTED_SETTING_VALUE);
    }

    #[test]
    fn replacing_or_clearing_the_zotero_key_forgets_the_account_it_was_verified_for() {
        let conn = in_memory_settings_db();
        set_setting(&conn, ZOTERO_USER_ID_KEY, "4242").expect("save user id");

        // The credential store is stubbed: a test must never delete the real
        // keyring entry (an earlier version of this test erased the owner's key).
        let deleted = std::cell::RefCell::new(Vec::new());
        persist_setting_with(
            &conn,
            ZOTERO_API_KEY,
            "",
            |_, _| panic!("clearing must not store"),
            |key| {
                deleted.borrow_mut().push(key.to_string());
                Ok(())
            },
        )
        .expect("clear key");

        assert_eq!(get_raw_setting(&conn, ZOTERO_USER_ID_KEY), None);
        assert_eq!(deleted.into_inner(), vec![ZOTERO_API_KEY.to_string()]);

        set_setting(&conn, ZOTERO_USER_ID_KEY, "4242").expect("save user id");
        let stored = std::cell::RefCell::new(Vec::new());
        persist_setting_with(
            &conn,
            ZOTERO_API_KEY,
            "new-key",
            |key, _| {
                stored.borrow_mut().push(key.to_string());
                Ok(())
            },
            |_| panic!("saving must not delete"),
        )
        .expect("replace key");

        assert_eq!(get_raw_setting(&conn, ZOTERO_USER_ID_KEY), None);
        assert_eq!(stored.into_inner(), vec![ZOTERO_API_KEY.to_string()]);
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn loads_runtime_bootstrap_public_key_from_builtin_defaults() {
        let conn = in_memory_settings_db();

        let public_key = get_runtime_bootstrap_public_key_with_builtin(
            &conn,
            "entropia-root",
            Some("entropia-root"),
            Some("base64-public-key"),
        )
        .expect("built-in public key should load");

        assert_eq!(public_key, "base64-public-key");
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn rejects_missing_runtime_bootstrap_public_key() {
        let conn = in_memory_settings_db();

        let error =
            get_runtime_bootstrap_public_key_with_builtin(&conn, "entropia-root", None, None)
                .expect_err("missing public key should fail");

        assert!(error.contains("entropia-root"));
    }

    #[test]
    fn saving_the_embedding_key_resumes_configuration_blocked_work_only() {
        let conn = in_memory_settings_db();
        conn.execute_batch(
            "CREATE TABLE processing_tasks (
               id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL,
               outcome TEXT NOT NULL DEFAULT '', owner_session TEXT, next_retry_at INTEGER,
               last_error_code TEXT, last_error_message TEXT, updated_at INTEGER NOT NULL DEFAULT 0);
             INSERT INTO processing_tasks (id, kind, state, outcome, last_error_code)
               VALUES ('t1', 'embedding', 'blocked', 'configuration_required', 'configuration_required');",
        )
        .expect("queue table");
        // Pin the remote provider: Pro defaults to the local engine, which
        // needs no OpenRouter key and would make the "no key" phase valid.
        set_setting(&conn, "embedding_provider", "api").expect("provider");
        let state = |conn: &Connection| -> String {
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("state")
        };
        // Unrelated keys and an invalid configuration leave the unit parked.
        assert_eq!(
            resume_work_after_setting_change(&conn, "openrouter_model"),
            0
        );
        assert_eq!(
            resume_work_after_setting_change(&conn, OPENROUTER_API_KEY),
            0
        );
        assert_eq!(state(&conn), "blocked");
        set_setting(&conn, OPENROUTER_API_KEY, "sk-test").expect("save key");
        assert_eq!(
            resume_work_after_setting_change(&conn, "openrouter_model"),
            0
        );
        assert_eq!(
            state(&conn),
            "blocked",
            "an unrelated key never resumes work"
        );
        assert_eq!(
            resume_work_after_setting_change(&conn, OPENROUTER_API_KEY),
            1
        );
        assert_eq!(state(&conn), "pending");
    }

    // ── recently-opened works (opened works first) ────────────────────────

    fn recently_opened_rows(conn: &Connection) -> Vec<(String, i64)> {
        let raw = get_raw_setting(conn, RECENTLY_OPENED_SETTING_KEY).expect("stored list");
        serde_json::from_str::<Vec<RecentlyOpenedEntry>>(&raw)
            .expect("valid recently-opened JSON")
            .into_iter()
            .map(|entry| (entry.item_id, entry.opened_at))
            .collect()
    }

    #[test]
    fn recording_opened_works_is_deduplicated_most_recent_first() {
        let conn = in_memory_settings_db();
        record_work_opened(&conn, "item-a", 100).expect("record a");
        record_work_opened(&conn, "item-b", 200).expect("record b");
        record_work_opened(&conn, "item-a", 300).expect("re-open a");

        assert_eq!(
            recently_opened_rows(&conn),
            vec![("item-a".to_string(), 300), ("item-b".to_string(), 200)],
            "the newest open leads and the older duplicate is dropped"
        );
        let ranks = recently_opened_ranks(&conn);
        assert_eq!(ranks.get("item-a"), Some(&0));
        assert_eq!(ranks.get("item-b"), Some(&1));
    }

    #[test]
    fn recording_opened_works_caps_the_list_at_500() {
        let conn = in_memory_settings_db();
        for now in 0..(RECENTLY_OPENED_MAX_ENTRIES as i64 + 2) {
            record_work_opened(&conn, &format!("item-{now:03}"), now).expect("record");
        }

        let rows = recently_opened_rows(&conn);
        assert_eq!(rows.len(), RECENTLY_OPENED_MAX_ENTRIES);
        assert_eq!(rows.first().map(|(id, _)| id.as_str()), Some("item-501"));
        assert_eq!(rows.last().map(|(id, _)| id.as_str()), Some("item-002"));
    }

    #[test]
    fn recording_an_open_repairs_a_corrupt_list() {
        let conn = in_memory_settings_db();
        set_setting(&conn, RECENTLY_OPENED_SETTING_KEY, "not json").expect("corrupt row");

        record_work_opened(&conn, "item-a", 5).expect("record over corrupt list");

        assert_eq!(recently_opened_rows(&conn), vec![("item-a".to_string(), 5)]);
    }

    #[test]
    fn the_bulk_settings_read_hides_the_recently_opened_bookkeeping() {
        let conn = in_memory_settings_db();
        set_setting(&conn, RECENTLY_OPENED_SETTING_KEY, "[]").expect("bookkeeping row");
        set_setting(&conn, "openrouter_model", "google/gemma").expect("model row");
        set_setting(&conn, OPENROUTER_API_KEY, "sk-secret").expect("key row");

        let entries = read_visible_settings(&conn).expect("bulk read");
        let shown: Vec<(&str, &str)> = entries
            .iter()
            .map(|entry| (entry.key.as_str(), entry.value.as_str()))
            .collect();
        assert!(
            !shown
                .iter()
                .any(|(key, _)| *key == RECENTLY_OPENED_SETTING_KEY),
            "the bookkeeping key must never reach the Settings UI: {shown:?}"
        );
        assert!(shown.contains(&("openrouter_model", "google/gemma")));
        assert!(shown.contains(&(OPENROUTER_API_KEY, REDACTED_SETTING_VALUE)));
    }
}
