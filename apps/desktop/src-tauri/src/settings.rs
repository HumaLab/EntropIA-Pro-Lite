use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};
use tauri::State;

use crate::db::commands::run_blocking_db_task;
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
/// Key prefixes only the backend may write: `settings_set` and
/// `settings_delete` refuse them, so a compromised renderer cannot repoint
/// the managed-runtime download at its own manifest and signing key (S-03)
/// or the folder the backend reads Zotero attachments from (S-01).
const BACKEND_ONLY_SETTING_PREFIXES: [&str; 2] = ["runtime_bootstrap_", "backend_grant."];
/// The legacy, renderer-writable Zotero data directory key: refused too, so a
/// planted value cannot come back under its old name.
const LEGACY_ZOTERO_DATA_DIR_KEY: &str = "zotero_data_dir";
/// Release builds trust only the compiled-in runtime bootstrap source when one
/// is set; debug builds let stored settings override it to test staging
/// manifests.
#[cfg(feature = "local-ml")]
const SETTINGS_MAY_OVERRIDE_BUILTIN_RUNTIME_SOURCE: bool = cfg!(debug_assertions);
const REDACTED_SETTING_VALUE: &str = "[redacted]";
const SECRET_REF_PREFIX: &str = "secret_ref:";
const APP_CREDENTIAL_SERVICE: &str = "com.entropia.desktop credentials";
pub const OPENROUTER_API_KEY: &str = "openrouter_api_key";
pub const ASSEMBLYAI_API_KEY: &str = "assemblyai_api_key";
pub const GLM_OCR_API_KEY: &str = "glm_ocr_api_key";
pub const ZOTERO_API_KEY: &str = "zotero_api_key";
/// Key for publishing Escritura documents to hlab.com.ar (`writing::publish`).
pub const HLAB_PUBLISH_KEY: &str = "hlab_publish_key";
/// The Zotero account id the stored key was last verified for. Not a secret, but
/// only meaningful next to that key, so a new or cleared key forgets it.
pub const ZOTERO_USER_ID_KEY: &str = "zotero_user_id";
const SECRET_SETTING_KEYS: [&str; 5] = [
    OPENROUTER_API_KEY,
    ASSEMBLYAI_API_KEY,
    GLM_OCR_API_KEY,
    ZOTERO_API_KEY,
    HLAB_PUBLISH_KEY,
];
static APP_CREDENTIAL_LOCK: Mutex<()> = Mutex::new(());

/// Per-key locks serializing the whole credential-store + `app_settings`
/// sequence of each secret setting (A-05a).
///
/// Acquisition order, never inverted, so there is no cycle:
/// 1. the key's lock from this map — secret keys only;
/// 2. `ui_conn` — only around the SQL, never during credential-store I/O;
/// 3. `APP_CREDENTIAL_LOCK` — inside each single credential-store call.
///
/// The key lock is requested before `ui_conn` and never while holding it.
/// Entries are created on first use and never removed: only the five
/// `SECRET_SETTING_KEYS` reach this map, so it cannot grow without bound.
static SETTINGS_KEY_LOCKS: LazyLock<Mutex<HashMap<String, &'static Mutex<()>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// [`SETTINGS_KEY_LOCKS`]' guard for `key`; `None` for non-secret keys, which
/// never touch the credential store.
fn lock_setting_key(key: &str) -> Option<MutexGuard<'static, ()>> {
    if !is_secret_setting_key(key) {
        return None;
    }
    let mutex: &'static Mutex<()> = {
        let mut locks = SETTINGS_KEY_LOCKS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        locks
            .entry(key.to_string())
            .or_insert_with(|| Box::leak(Box::new(Mutex::new(()))))
    };
    Some(mutex.lock().unwrap_or_else(|poison| poison.into_inner()))
}

/// [`lock_setting_key`] for a batch read (A-05b): every secret key among `keys`
/// is locked in `SECRET_SETTING_KEYS` order, so two concurrent multi-key reads
/// always take the same locks in the same order and cannot deadlock.
fn lock_setting_keys(keys: &[&str]) -> Vec<MutexGuard<'static, ()>> {
    let mut guards = Vec::new();
    for secret_key in SECRET_SETTING_KEYS {
        if keys.contains(&secret_key) {
            if let Some(guard) = lock_setting_key(secret_key) {
                guards.push(guard);
            }
        }
    }
    guards
}

/// The system credential store (keyring) behind secret settings, injectable so
/// tests can drive the set/read/delete sequences against an in-memory fake and
/// never touch the keyring of the machine running the tests.
pub(crate) trait SecretStore: Send + Sync {
    fn store(&self, key: &str, value: &str) -> Result<(), String>;
    fn read(&self, key: &str) -> Result<Option<String>, String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The real credential store: each call wraps one of the guarded keyring
/// helpers (`store_secret`/`read_secret`/`delete_secret`).
pub(crate) struct KeyringSecretStore;

impl SecretStore for KeyringSecretStore {
    fn store(&self, key: &str, value: &str) -> Result<(), String> {
        store_secret(key, value)
    }

    fn read(&self, key: &str) -> Result<Option<String>, String> {
        read_secret(key)
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        delete_secret(key)
    }
}
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
    let db = db.inner().clone();
    run_blocking_db_task(move || read_setting_for_ipc(&db.ui_conn, &key, &KeyringSecretStore)).await
}

#[tauri::command]
pub async fn settings_set(
    key: String,
    value: String,
    db: State<'_, AppDbState>,
    deps: State<'_, crate::deps::DepsState>,
) -> Result<(), String> {
    let should_invalidate = crate::deps::should_invalidate_cache_for_setting(&key);
    let invalidation_key = key.clone();
    let db = db.inner().clone();
    run_blocking_db_task(move || {
        set_setting_with_store(&db.ui_conn, &key, &value, &KeyringSecretStore)?;
        resume_work_after_setting_change_outside_ui_conn(&db.db_path, &key);
        Ok(())
    })
    .await?;
    if should_invalidate {
        invalidate_dependency_probe_cache_if_needed(&invalidation_key, Some(&deps)).await;
    }
    Ok(())
}

/// Whether the renderer may set or delete `key` through the settings IPC.
pub(crate) fn is_renderer_writable_setting(key: &str) -> bool {
    key != LEGACY_ZOTERO_DATA_DIR_KEY
        && !BACKEND_ONLY_SETTING_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

fn ensure_renderer_writable_setting(key: &str) -> Result<(), String> {
    if is_renderer_writable_setting(key) {
        Ok(())
    } else {
        Err(format!(
            "Setting '{key}' is managed by the app and cannot be changed from the interface"
        ))
    }
}

/// The whole [`settings_set`] sequence for a renderer-writable key:
/// renderer-writable check, per-key lock, credential-store write (secret keys
/// only, with no `ui_conn` held), then `forget_zotero_user_id_for` and the
/// reference row under `ui_conn`. A credential-store failure returns the error
/// and leaves the row untouched. The store is injected so tests never touch
/// the real keyring.
///
/// The configuration resume that follows runs in a second phase, on its own
/// connection ([`resume_work_after_setting_change_outside_ui_conn`]): the
/// repository re-reads the configuration, including secrets that live in the
/// credential store, so it must not run while `ui_conn` is held (A-05b).
fn set_setting_with_store(
    ui_conn: &Mutex<rusqlite::Connection>,
    key: &str,
    value: &str,
    store: &dyn SecretStore,
) -> Result<(), String> {
    ensure_renderer_writable_setting(key)?;
    let _key_guard = lock_setting_key(key);
    let clearing_secret = is_secret_setting_key(key) && value.trim().is_empty();
    if is_secret_setting_key(key) && !clearing_secret {
        // Secret first, without `ui_conn`: a credential-store failure must not
        // leave a reference row pointing at a secret that was never stored.
        store.store(key, value)?;
    }
    {
        let conn = ui_conn.lock().map_err(|e| format!("DB lock error: {e}"))?;
        forget_zotero_user_id_for(&conn, key);
        write_setting_row(&conn, key, value)?;
    }
    if clearing_secret {
        // Row first, then the secret (today's semantics): a credential-store
        // failure only leaves an orphaned entry and never fails the command.
        if let Err(error) = store.delete(key) {
            eprintln!("[settings] Setting row deleted but credential cleanup failed: {error}");
        }
    }
    Ok(())
}

/// The configuration resume of a settings save, on a connection of its own so
/// no `ui_conn` guard is held while the repository resolves secret references
/// from the credential store (A-05b). The new row is already committed, so a
/// fresh archive connection reads it; a failure is logged and never fails the
/// save, exactly like the in-lock pass this replaced.
fn resume_work_after_setting_change_outside_ui_conn(db_path: &std::path::Path, key: &str) {
    if !is_embedding_engine_setting(key) && !is_ocr_engine_setting(key) {
        return;
    }
    match crate::db::open::open_archive_connection(db_path) {
        Ok(conn) => {
            resume_work_after_setting_change(&conn, key);
        }
        Err(error) => {
            eprintln!("[settings] Could not resume configuration-blocked work: {error}");
        }
    }
}

/// Logs, without values and without deleting them, the backend-only
/// settings older builds let the renderer write: the runtime bootstrap source
/// and the legacy Zotero data directory, which is no longer read.
pub fn log_renderer_written_backend_settings(conn: &rusqlite::Connection) {
    let keys: Vec<String> = conn
        .prepare(
            "SELECT key FROM app_settings
             WHERE key GLOB 'runtime_bootstrap_*' OR key = 'zotero_data_dir'
             ORDER BY key",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()
        })
        .unwrap_or_default();
    if !keys.is_empty() {
        eprintln!(
            "[settings] Renderer-written settings found ({}); release builds ignore a stored runtime bootstrap source when a built-in one is set, and the legacy Zotero data directory is never read",
            keys.join(", ")
        );
    }
}

/// Settings that decide whether the embedding engine can initialize.
fn is_embedding_engine_setting(key: &str) -> bool {
    key == OPENROUTER_API_KEY
        || key == crate::nlp::embeddings::EMBEDDING_PROVIDER_SETTING_KEY
        || key == crate::nlp::embeddings::OPENROUTER_EMBEDDING_MODEL_SETTING_KEY
}

/// Settings that decide whether the OCR engine can initialize: the engine
/// selection and the GLM-OCR credential it reads.
fn is_ocr_engine_setting(key: &str) -> bool {
    key == GLM_OCR_API_KEY || key == crate::ocr::OCRH_SETTING_MODE
}

/// Runs one engine's configuration resume, logging its outcome best-effort.
fn resume_configuration_blocked_with(
    conn: &rusqlite::Connection,
    engine: &str,
    resume: impl FnOnce(&rusqlite::Connection) -> Result<usize, String>,
) -> usize {
    match resume(conn) {
        Ok(resumed) => {
            if resumed > 0 {
                eprintln!(
                    "[settings] Resumed {resumed} queue unit(s) blocked on {engine} configuration"
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

/// Requeues queue units parked on a configuration a saved setting decides,
/// once that configuration is now valid: the embedding engine's
/// (`configuration_required…` on `embedding`/`bibliography_profile` units)
/// when an embedding setting is saved, and the OCR engine's
/// (`configuration_required_ocr`, plus the legacy plain
/// `configuration_required` rows of OCR work signed with the executor's
/// `configuration:` message prefix) when an OCR setting is saved. Embedding
/// and contract-change blocks never move on an OCR save and vice versa —
/// each engine resumes only its own. Best effort: a failure is logged and
/// never fails the save. Returns how many units were requeued.
pub(crate) fn resume_work_after_setting_change(conn: &rusqlite::Connection, key: &str) -> usize {
    if is_embedding_engine_setting(key) {
        resume_configuration_blocked_with(
            conn,
            "embedding",
            crate::processing::repository::resume_embedding_configuration_blocked,
        )
    } else if is_ocr_engine_setting(key) {
        resume_configuration_blocked_with(
            conn,
            "OCR",
            crate::processing::repository::resume_ocr_configuration_blocked,
        )
    } else {
        0
    }
}

#[tauri::command]
pub async fn settings_get_all(db: State<'_, AppDbState>) -> Result<Vec<SettingEntry>, String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        read_visible_settings(&conn)
    })
    .await
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

/// The database side of [`settings_delete`], split out so tests can pin what
/// a deletion leaves behind without ever touching the system credential
/// store (the keyring cleanup stays in the command).
fn remove_setting_row(conn: &rusqlite::Connection, key: &str) -> Result<(), String> {
    // DB row first: if the keyring delete then fails, only an orphaned
    // keyring entry remains (no dangling secret_ref pointing at nothing).
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])
        .map_err(|e| format!("Failed to delete setting: {e}"))?;
    forget_zotero_user_id_for(conn, key);
    Ok(())
}

/// The whole [`settings_delete`] sequence for a renderer-writable key:
/// renderer-writable check, per-key lock, row delete under `ui_conn`, then the
/// credential cleanup after that lock is released. A credential-store failure
/// is logged and never reverts the row (today's semantics).
fn delete_setting_with_store(
    ui_conn: &Mutex<rusqlite::Connection>,
    key: &str,
    store: &dyn SecretStore,
) -> Result<(), String> {
    ensure_renderer_writable_setting(key)?;
    let _key_guard = lock_setting_key(key);
    {
        let conn = ui_conn.lock().map_err(|e| format!("DB lock error: {e}"))?;
        remove_setting_row(&conn, key)?;
    }
    if is_secret_setting_key(key) {
        if let Err(error) = store.delete(key) {
            eprintln!("[settings] Setting row deleted but credential cleanup failed: {error}");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn settings_delete(
    key: String,
    db: State<'_, AppDbState>,
    deps: State<'_, crate::deps::DepsState>,
) -> Result<(), String> {
    let should_invalidate = crate::deps::should_invalidate_cache_for_setting(&key);
    let invalidation_key = key.clone();
    let db = db.inner().clone();
    run_blocking_db_task(move || delete_setting_with_store(&db.ui_conn, &key, &KeyringSecretStore))
        .await?;
    if should_invalidate {
        invalidate_dependency_probe_cache_if_needed(&invalidation_key, Some(&deps)).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers (for Rust-side reading, used by LLM worker)
// ---------------------------------------------------------------------------

/// The stored form of one setting, before any credential-store resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingRef {
    /// No row stored for the key.
    Missing,
    /// The stored value is the setting value (plain settings, or legacy
    /// plaintext a migration has not moved to the credential store yet).
    Value(String),
    /// The stored value is a `secret_ref:` marker: the real secret lives in
    /// the credential store under `key`.
    Secret { key: String, stored: String },
}

impl SettingRef {
    /// The value as stored in `app_settings`, marker included; `None` when the
    /// key has no row. Used for the renderer-facing value.
    fn stored_value(&self) -> Option<&str> {
        match self {
            SettingRef::Missing => None,
            SettingRef::Value(value) => Some(value),
            SettingRef::Secret { stored, .. } => Some(stored),
        }
    }
}

/// First half of a setting read: fetch the stored reference from the caller's
/// connection. Resolve it with [`resolve_setting_ref`] after releasing the
/// connection, never while it is held.
pub(crate) fn read_setting_ref(conn: &rusqlite::Connection, key: &str) -> SettingRef {
    let Some(stored) = get_raw_setting(conn, key) else {
        return SettingRef::Missing;
    };
    if is_secret_setting_key(key) && stored.starts_with(SECRET_REF_PREFIX) {
        SettingRef::Secret {
            key: key.to_string(),
            stored,
        }
    } else {
        SettingRef::Value(stored)
    }
}

/// Second half of a setting read: resolve the credential store outside any
/// `ui_conn` guard.
pub(crate) fn resolve_setting_ref(
    reference: &SettingRef,
    store: &dyn SecretStore,
) -> Result<Option<String>, String> {
    match reference {
        SettingRef::Missing => Ok(None),
        SettingRef::Value(value) => Ok(Some(value.clone())),
        SettingRef::Secret { key, .. } => store.read(key),
    }
}

/// [`resolve_setting_ref`] with [`get_setting`]'s reporting contract: a
/// missing credential-store entry and a credential-store failure are logged
/// and read as `None`, so Rust-side readers keep today's empty-value
/// behaviour instead of surfacing a keyring error.
fn resolve_reference_logged(
    reference: &SettingRef,
    key: &str,
    store: &dyn SecretStore,
) -> Option<String> {
    match resolve_setting_ref(reference, store) {
        Ok(value) => {
            if matches!(reference, SettingRef::Secret { .. }) && value.is_none() {
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

/// Resolve one or more settings from the shared `ui_conn` without holding it
/// during credential-store I/O (A-05b). A-05a's fixed order holds: the secret
/// keys' per-key locks first, then `ui_conn` for the references only, then the
/// released connection while [`resolve_setting_ref`] reads the credential
/// store. Keyring failures and missing entries read as `None`, exactly like
/// [`get_setting`]; the only error is a poisoned `ui_conn`.
pub(crate) fn read_settings_unlocked(
    ui_conn: &Mutex<rusqlite::Connection>,
    keys: &[&str],
    store: &dyn SecretStore,
) -> Result<Vec<Option<String>>, String> {
    let _key_guards = lock_setting_keys(keys);
    let references: Vec<SettingRef> = {
        let conn = ui_conn.lock().map_err(|e| format!("DB lock error: {e}"))?;
        keys.iter()
            .map(|key| read_setting_ref(&conn, key))
            .collect()
    };
    Ok(keys
        .iter()
        .zip(references.iter())
        .map(|(key, reference)| resolve_reference_logged(reference, key, store))
        .collect())
}

/// [`read_settings_unlocked`] for a single key.
pub(crate) fn get_secret_setting_unlocked(
    ui_conn: &Mutex<rusqlite::Connection>,
    key: &str,
    store: &dyn SecretStore,
) -> Result<Option<String>, String> {
    Ok(read_settings_unlocked(ui_conn, &[key], store)?
        .into_iter()
        .next()
        .flatten())
}

/// [`resolve_api_key_input`] for callers that share `ui_conn` (A-05b): an
/// explicit `provided` key wins with no database or credential-store access,
/// and a blank one resolves the stored secret after the connection is
/// released. The error for a key that is not configured is unchanged.
pub(crate) fn resolve_api_key_input_unlocked(
    ui_conn: &Mutex<rusqlite::Connection>,
    key: &str,
    provided: &str,
    store: &dyn SecretStore,
) -> Result<String, String> {
    let provided = provided.trim();
    if !provided.is_empty() {
        return Ok(provided.to_string());
    }
    get_secret_setting_unlocked(ui_conn, key, store)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("No protected credential is configured for '{key}'"))
}

/// The whole [`settings_get`] sequence: per-key lock for secret keys, read the
/// reference under `ui_conn`, release it, then resolve the credential store.
/// Returns the renderer-facing value ([`ipc_setting_value`]): the resolution
/// only verifies presence and logs a missing entry, exactly like the direct
/// reads; it never exposes the secret over IPC.
fn read_setting_for_ipc(
    ui_conn: &Mutex<rusqlite::Connection>,
    key: &str,
    store: &dyn SecretStore,
) -> Result<Option<String>, String> {
    let _key_guard = lock_setting_key(key);
    let reference = {
        let conn = ui_conn.lock().map_err(|e| format!("DB lock error: {e}"))?;
        read_setting_ref(&conn, key)
    };
    if matches!(reference, SettingRef::Secret { .. }) {
        match resolve_setting_ref(&reference, store) {
            Ok(Some(_)) => {}
            Ok(None) => eprintln!(
                "[settings] Protected setting '{key}' references a missing credential store entry"
            ),
            Err(error) => {
                eprintln!("[settings] Failed to resolve protected setting '{key}': {error}");
            }
        }
    }
    Ok(reference
        .stored_value()
        .map(|value| ipc_setting_value(key, value)))
}

/// Read a setting value directly from a rusqlite connection.
/// Used by the LLM worker to read API keys without going through Tauri state.
///
/// Kept for Rust-side callers that own the connection; A-05b migrates the
/// callers that currently invoke it with `ui_conn` held to the two-step API
/// ([`read_setting_ref`] then [`resolve_setting_ref`], wrapped by
/// [`read_settings_unlocked`]).
pub fn get_setting(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    let reference = read_setting_ref(conn, key);
    resolve_reference_logged(&reference, key, &KeyringSecretStore)
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
/// write to or delete from the real system keyring. Direct Rust-side callers
/// keep their connection; the command path that releases `ui_conn` around the
/// credential-store I/O lives in [`set_setting_with_store`].
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
            write_setting_row(conn, key, value)?;
            if let Err(error) = delete(key) {
                eprintln!("[settings] Setting row deleted but credential cleanup failed: {error}");
            }
            return Ok(());
        }
        store(key, value)?;
    }
    write_setting_row(conn, key, value)
}

/// Writes the `app_settings` row for one setting: the `secret_ref` marker for a
/// stored secret, the plain value for anything else, or a row delete when a
/// secret is cleared. Runs with `ui_conn` held; the credential store is the
/// caller's job and is never touched from here.
fn write_setting_row(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<(), String> {
    if is_secret_setting_key(key) {
        if value.trim().is_empty() {
            conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])
                .map_err(|error| format!("Failed to delete empty protected setting: {error}"))?;
            return Ok(());
        }
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
        SETTINGS_MAY_OVERRIDE_BUILTIN_RUNTIME_SOURCE,
    )
}

/// Stored settings are used when no built-in source is compiled in, or when
/// `settings_may_override` allows them to replace it (debug builds).
#[cfg(feature = "local-ml")]
fn get_runtime_bootstrap_remote_source_with_builtin(
    conn: &rusqlite::Connection,
    builtin_manifest_url: Option<&str>,
    builtin_public_key_id: Option<&str>,
    builtin_public_key_base64: Option<&str>,
    settings_may_override: bool,
) -> Result<Option<BootstrapRemoteSource>, String> {
    let builtin_is_set = [
        builtin_manifest_url,
        builtin_public_key_id,
        builtin_public_key_base64,
    ]
    .into_iter()
    .any(|value| trimmed_optional(value).is_some());
    if builtin_is_set && !settings_may_override {
        return builtin_runtime_bootstrap_remote_source(
            builtin_manifest_url,
            builtin_public_key_id,
            builtin_public_key_base64,
        );
    }

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
    // The built-in key id always resolves to the built-in key: a stored
    // setting never replaces the official signing key.
    if trimmed_optional(builtin_public_key_id).as_deref() == Some(public_key_id) {
        if let Some(public_key) = trimmed_optional(builtin_public_key_base64) {
            return Ok(public_key);
        }
        return Err(format!(
            "Bootstrap public key '{public_key_id}' is selected by {BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_ENV}, but {BUILTIN_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64_ENV} is not configured"
        ));
    }

    let key = format!("{RUNTIME_BOOTSTRAP_PUBLIC_KEY_KEY_PREFIX}{public_key_id}");
    if let Some(configured_key) = get_setting(conn, &key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(configured_key);
    }

    Err(format!(
        "Bootstrap public key '{public_key_id}' is not configured"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar};
    use std::time::{Duration, Instant};

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

    /// Credential store for keys refused before any keyring I/O: reaching it
    /// at all is the failure.
    struct UnreachableSecretStore;

    impl SecretStore for UnreachableSecretStore {
        fn store(&self, key: &str, _value: &str) -> Result<(), String> {
            panic!("unexpected credential store write for '{key}'")
        }

        fn read(&self, key: &str) -> Result<Option<String>, String> {
            panic!("unexpected credential store read for '{key}'")
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            panic!("unexpected credential store delete for '{key}'")
        }
    }

    /// Simple in-memory credential store for the tests that do not need to
    /// interleave two sequences.
    #[derive(Default)]
    struct InMemorySecretStore {
        secrets: Mutex<HashMap<String, String>>,
    }

    impl InMemorySecretStore {
        fn with_secret(key: &str, value: &str) -> Self {
            let store = Self::default();
            store
                .secrets
                .lock()
                .expect("secrets")
                .insert(key.to_string(), value.to_string());
            store
        }

        fn secret(&self, key: &str) -> Option<String> {
            self.secrets.lock().expect("secrets").get(key).cloned()
        }
    }

    impl SecretStore for InMemorySecretStore {
        fn store(&self, key: &str, value: &str) -> Result<(), String> {
            self.secrets
                .lock()
                .expect("secrets")
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn read(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.secret(key))
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.secrets.lock().expect("secrets").remove(key);
            Ok(())
        }
    }

    const RUNTIME_BOOTSTRAP_KEYS: [&str; 3] = [
        RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY,
        RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY,
        "runtime_bootstrap_public_key.entropia-root",
    ];

    // S-03: a compromised renderer must not repoint the managed-runtime
    // download at its own manifest and signing key.
    #[test]
    fn the_renderer_cannot_set_the_runtime_bootstrap_trust_source() {
        let conn = Mutex::new(in_memory_settings_db());
        for key in RUNTIME_BOOTSTRAP_KEYS {
            let error = set_setting_with_store(
                &conn,
                key,
                "https://attacker.invalid/x",
                &UnreachableSecretStore,
            )
            .expect_err("a backend-only key must be refused");
            assert!(error.contains(key), "{error}");
            assert_eq!(
                get_raw_setting(&conn.lock().expect("conn"), key),
                None,
                "{key} must not be written"
            );
        }
    }

    #[test]
    fn the_renderer_cannot_delete_the_runtime_bootstrap_trust_source() {
        let conn = Mutex::new(in_memory_settings_db());
        for key in RUNTIME_BOOTSTRAP_KEYS {
            set_setting(&conn.lock().expect("conn"), key, "configured").expect("seed");
            delete_setting_with_store(&conn, key, &UnreachableSecretStore)
                .expect_err("a backend-only key must be refused");
            assert_eq!(
                get_raw_setting(&conn.lock().expect("conn"), key).as_deref(),
                Some("configured")
            );
        }
    }

    // S-01: the Zotero data directory is a root the backend reads attachment
    // files from, so only the backend's own folder picker may set it.
    #[test]
    fn the_renderer_cannot_set_or_delete_the_zotero_data_directory() {
        let conn = Mutex::new(in_memory_settings_db());
        for key in [
            crate::bibliography::processing::ZOTERO_DATA_DIR_SETTING_KEY,
            "zotero_data_dir",
        ] {
            set_setting_with_store(&conn, key, "/home/ana", &UnreachableSecretStore)
                .expect_err("a backend-granted key must be refused");
            assert_eq!(get_raw_setting(&conn.lock().expect("conn"), key), None);
            set_setting(&conn.lock().expect("conn"), key, "/home/ana/Zotero").expect("seed");
            delete_setting_with_store(&conn, key, &UnreachableSecretStore)
                .expect_err("a backend-granted key must be refused");
            assert!(get_raw_setting(&conn.lock().expect("conn"), key).is_some());
        }
    }

    #[test]
    fn the_renderer_still_sets_and_deletes_ordinary_settings() {
        let conn = Mutex::new(in_memory_settings_db());
        set_setting_with_store(&conn, "ui_theme", "dark", &UnreachableSecretStore)
            .expect("ordinary key");
        assert_eq!(
            get_raw_setting(&conn.lock().expect("conn"), "ui_theme").as_deref(),
            Some("dark")
        );
        delete_setting_with_store(&conn, "ui_theme", &UnreachableSecretStore)
            .expect("ordinary key");
        assert_eq!(
            get_raw_setting(&conn.lock().expect("conn"), "ui_theme"),
            None
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn the_builtin_runtime_bootstrap_source_wins_when_settings_may_not_override_it() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY,
            "https://attacker.invalid/bootstrap.json",
        )
        .expect("save manifest url");
        set_setting(&conn, RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY, "attacker").expect("save id");

        let source = get_runtime_bootstrap_remote_source_with_builtin(
            &conn,
            Some("https://example.com/runtime/bootstrap.json"),
            Some("entropia-root"),
            Some("base64-public-key"),
            false,
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
    fn stored_runtime_bootstrap_settings_override_the_builtin_source_only_when_allowed() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY,
            "https://staging.example.com/bootstrap.json",
        )
        .expect("save manifest url");
        set_setting(&conn, RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY, "staging").expect("save id");

        let source = get_runtime_bootstrap_remote_source_with_builtin(
            &conn,
            Some("https://example.com/runtime/bootstrap.json"),
            Some("entropia-root"),
            Some("base64-public-key"),
            true,
        )
        .expect("stored source should load");

        assert_eq!(
            source.map(|source| source.public_key_id).as_deref(),
            Some("staging")
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn a_stored_public_key_never_replaces_the_builtin_key_id() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            "runtime_bootstrap_public_key.entropia-root",
            "attacker-public-key",
        )
        .expect("save key");

        let public_key = get_runtime_bootstrap_public_key_with_builtin(
            &conn,
            "entropia-root",
            Some("entropia-root"),
            Some("official-public-key"),
        )
        .expect("built-in public key should load");

        assert_eq!(public_key, "official-public-key");
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
        let store = InMemorySecretStore::with_secret(OPENROUTER_API_KEY, "sk-protected");

        let reference = read_setting_ref(&conn, OPENROUTER_API_KEY);
        assert_eq!(
            reference,
            SettingRef::Secret {
                key: OPENROUTER_API_KEY.to_string(),
                stored: "secret_ref:openrouter_api_key".to_string(),
            }
        );
        let value = resolve_setting_ref(&reference, &store).expect("resolve protected setting");

        assert_eq!(value.as_deref(), Some("sk-protected"));
    }

    #[test]
    fn explicit_api_key_input_takes_precedence_over_stored_value() {
        let conn = Mutex::new(in_memory_settings_db());
        set_setting(
            &conn.lock().expect("conn"),
            OPENROUTER_API_KEY,
            "legacy-stored",
        )
        .expect("save key");

        assert_eq!(
            resolve_api_key_input_unlocked(
                &conn,
                OPENROUTER_API_KEY,
                "  explicit  ",
                &KeyringSecretStore
            )
            .expect("resolve explicit key"),
            "explicit"
        );
    }

    // The embedding engine cache is keyed on a config fingerprint that
    // includes the (hashed) API key, but it is only reachable through a
    // config resolved from the database. After `settings_delete` drops the
    // key row, resolution must fail closed — that is what makes the engine
    // stop using the cached key. The GLM-OCR credential is read per call and
    // must fail closed the same way.
    #[test]
    fn deleting_the_openrouter_key_makes_embedding_config_report_missing_key() {
        let conn = in_memory_settings_db();
        set_setting(
            &conn,
            crate::nlp::embeddings::EMBEDDING_PROVIDER_SETTING_KEY,
            "api",
        )
        .expect("select api provider");
        set_setting(&conn, OPENROUTER_API_KEY, "sk-test").expect("save key");
        assert!(crate::nlp::embeddings::config_from_settings(&conn).is_ok());

        remove_setting_row(&conn, OPENROUTER_API_KEY).expect("delete key");

        let error = crate::nlp::embeddings::config_from_settings(&conn)
            .err()
            .expect("a deleted key must fail embedding config resolution");
        assert!(error.contains("OpenRouter API key"), "{error}");
    }

    #[test]
    fn deleting_the_glm_ocr_key_makes_ocr_config_report_missing_key() {
        let conn = in_memory_settings_db();
        set_setting(&conn, crate::ocr::OCRH_SETTING_MODE, "glm_ocr").expect("select glm ocr");
        set_setting(&conn, GLM_OCR_API_KEY, "sk-glm").expect("save key");
        crate::ocr::ensure_selected_cloud_key(&conn).expect("stored key satisfies the config");

        remove_setting_row(&conn, GLM_OCR_API_KEY).expect("delete key");

        let error = crate::ocr::ensure_selected_cloud_key(&conn)
            .expect_err("a deleted key must fail OCR config resolution");
        assert!(error.contains("GLM-OCR"), "{error}");
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

        let source =
            get_runtime_bootstrap_remote_source_with_builtin(&conn, None, None, None, true)
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
            true,
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
            true,
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
            true,
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

    #[test]
    fn saving_the_ocr_key_resumes_ocr_blocked_work_only() {
        let conn = in_memory_settings_db();
        conn.execute_batch(
            "CREATE TABLE processing_tasks (
               id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL,
               outcome TEXT NOT NULL DEFAULT '', owner_session TEXT, next_retry_at INTEGER,
               last_error_code TEXT, last_error_message TEXT, updated_at INTEGER NOT NULL DEFAULT 0);
             INSERT INTO processing_tasks (id, kind, state, outcome, last_error_code, last_error_message)
               VALUES ('t-extract', 'bibliography_extract', 'blocked', 'configuration_required_ocr',
                       'configuration_required_ocr', 'configuration: GLM-OCR no está configurado.'),
                      ('t-corpus', 'ocr', 'blocked', 'configuration_required_ocr',
                       'configuration_required_ocr', 'configuration: PaddleOCR liviano no está disponible'),
                      ('t-legacy', 'ocr', 'blocked', 'configuration_required',
                       'configuration_required', 'configuration: GLM-OCR no está configurado.'),
                      ('t-embed', 'embedding', 'blocked', 'configuration_required_embedding',
                       'configuration_required_embedding', 'no engine'),
                      ('t-contract', 'bibliography_extract', 'blocked', 'configuration_required_extract_contract',
                       'configuration_required_extract_contract',
                       'the bibliography extraction contract changed; resume with the current configuration to re-evaluate');",
        )
        .expect("queue table");
        // GLM-OCR selected with no key yet: the configuration is still
        // invalid, so nothing may move back into the same wall.
        set_setting(
            &conn,
            crate::ocr::OCRH_SETTING_MODE,
            crate::ocr::OCRH_MODE_GLM_OCR,
        )
        .expect("mode");
        let state = |conn: &Connection, id: &str| -> String {
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("state")
        };
        assert_eq!(resume_work_after_setting_change(&conn, GLM_OCR_API_KEY), 0);
        assert_eq!(state(&conn, "t-extract"), "blocked");
        set_setting(&conn, GLM_OCR_API_KEY, "sk-test").expect("save key");
        // An unrelated key never resumes the parked work.
        assert_eq!(
            resume_work_after_setting_change(&conn, "openrouter_model"),
            0
        );
        assert_eq!(state(&conn, "t-extract"), "blocked");
        assert_eq!(resume_work_after_setting_change(&conn, GLM_OCR_API_KEY), 3);
        assert_eq!(state(&conn, "t-extract"), "pending");
        assert_eq!(state(&conn, "t-corpus"), "pending");
        assert_eq!(state(&conn, "t-legacy"), "pending");
        // Embedding-blocked work keeps its own resume path, and a
        // contract-change block is never an OCR configuration block.
        assert_eq!(state(&conn, "t-embed"), "blocked");
        assert_eq!(state(&conn, "t-contract"), "blocked");
        assert_eq!(resume_work_after_setting_change(&conn, GLM_OCR_API_KEY), 0);
    }

    #[test]
    fn saving_the_openrouter_key_does_not_touch_ocr_blocked_work() {
        let conn = in_memory_settings_db();
        conn.execute_batch(
            "CREATE TABLE processing_tasks (
               id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL,
               outcome TEXT NOT NULL DEFAULT '', owner_session TEXT, next_retry_at INTEGER,
               last_error_code TEXT, last_error_message TEXT, updated_at INTEGER NOT NULL DEFAULT 0);
             INSERT INTO processing_tasks (id, kind, state, outcome, last_error_code, last_error_message)
               VALUES ('t-embed', 'embedding', 'blocked', 'configuration_required',
                       'configuration_required', 'no engine'),
                      ('t-extract', 'bibliography_extract', 'blocked', 'configuration_required_ocr',
                       'configuration_required_ocr', 'configuration: GLM-OCR no está configurado.'),
                      ('t-corpus', 'ocr', 'blocked', 'configuration_required',
                       'configuration_required', 'configuration: GLM-OCR no está configurado.');",
        )
        .expect("queue table");
        set_setting(&conn, "embedding_provider", "api").expect("provider");
        let state = |conn: &Connection, id: &str| -> String {
            conn.query_row(
                "SELECT state FROM processing_tasks WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("state")
        };
        set_setting(&conn, OPENROUTER_API_KEY, "sk-test").expect("save key");
        // The embedding resume moves exactly the embedding-blocked unit.
        assert_eq!(
            resume_work_after_setting_change(&conn, OPENROUTER_API_KEY),
            1
        );
        assert_eq!(state(&conn, "t-embed"), "pending");
        // OCR-blocked work waits on the OCR configuration, not on OpenRouter.
        assert_eq!(state(&conn, "t-extract"), "blocked");
        assert_eq!(state(&conn, "t-corpus"), "blocked");
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

    // ── A-05a: the command sequences and the per-key lock ─────────────────

    /// Shared connection mimicking `AppDbState::ui_conn` in the sequence
    /// tests: two threads can lock it the way the commands do.
    fn shared_settings_db() -> Arc<Mutex<Connection>> {
        Arc::new(Mutex::new(in_memory_settings_db()))
    }

    /// Credential store that always fails, to pin that a failed `settings_set`
    /// reports the error and leaves no reference row behind.
    struct FailingSecretStore;

    impl SecretStore for FailingSecretStore {
        fn store(&self, key: &str, _value: &str) -> Result<(), String> {
            Err(format!("credential store unavailable for '{key}'"))
        }

        fn read(&self, _key: &str) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn delete(&self, _key: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_failing_credential_store_leaves_no_reference_row() {
        let conn = shared_settings_db();
        // The Zotero user id is only forgotten once the secret is safely
        // stored: a failed keyring write must not run any row side effect.
        set_setting(&conn.lock().expect("conn"), ZOTERO_USER_ID_KEY, "4242").expect("user id");

        let error = set_setting_with_store(&conn, ZOTERO_API_KEY, "new-key", &FailingSecretStore)
            .expect_err("keyring failure must surface");

        assert!(error.contains("credential store unavailable"), "{error}");
        let conn = conn.lock().expect("conn");
        assert_eq!(get_raw_setting(&conn, ZOTERO_API_KEY), None);
        assert_eq!(
            get_raw_setting(&conn, ZOTERO_USER_ID_KEY).as_deref(),
            Some("4242")
        );
    }

    /// One-shot flag with a bounded wait, used to force a chosen interleaving
    /// between the two credential-store operations of the racing tests.
    struct Flag {
        set: Mutex<bool>,
        condvar: Condvar,
    }

    impl Flag {
        fn new() -> Self {
            Self {
                set: Mutex::new(false),
                condvar: Condvar::new(),
            }
        }

        fn mark(&self) {
            *self.set.lock().expect("flag") = true;
            self.condvar.notify_all();
        }

        fn wait(&self, timeout: Duration) -> bool {
            let deadline = Instant::now() + timeout;
            let mut set = self.set.lock().expect("flag");
            while !*set {
                let now = Instant::now();
                if now >= deadline {
                    return false;
                }
                let (next, _) = self
                    .condvar
                    .wait_timeout(set, deadline - now)
                    .expect("flag wait");
                set = next;
            }
            true
        }
    }

    /// In-memory credential store that lets the set and delete sequences
    /// observe each other: `store` writes the secret before waiting for a
    /// delete to complete, and `delete` waits for a store before removing the
    /// secret. The waits are bounded, so the serialized (correct) case always
    /// finishes by itself.
    struct RacingSecretStore {
        secrets: Mutex<HashMap<String, String>>,
        stored: Flag,
        deleted: Flag,
        delete_started: Flag,
        wait: Duration,
    }

    impl RacingSecretStore {
        fn new(wait: Duration) -> Self {
            Self {
                secrets: Mutex::new(HashMap::new()),
                stored: Flag::new(),
                deleted: Flag::new(),
                delete_started: Flag::new(),
                wait,
            }
        }

        fn secret(&self, key: &str) -> Option<String> {
            self.secrets.lock().expect("secrets").get(key).cloned()
        }
    }

    impl SecretStore for RacingSecretStore {
        fn store(&self, key: &str, value: &str) -> Result<(), String> {
            self.secrets
                .lock()
                .expect("secrets")
                .insert(key.to_string(), value.to_string());
            self.stored.mark();
            self.deleted.wait(self.wait);
            Ok(())
        }

        fn read(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.secret(key))
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.delete_started.mark();
            self.stored.wait(self.wait);
            self.secrets.lock().expect("secrets").remove(key);
            self.deleted.mark();
            Ok(())
        }
    }

    /// The invariant the per-key lock protects: no `app_settings` row whose
    /// value is a `secret_ref:` points at a secret the credential store no
    /// longer has. Valid end states are "no row and no secret" or "row and
    /// secret"; an orphaned secret without a row is allowed.
    fn assert_no_dangling_secret_ref(conn: &Mutex<Connection>, store: &dyn SecretStore) {
        let conn = conn.lock().expect("conn");
        let mut stmt = conn
            .prepare("SELECT key, value FROM app_settings")
            .expect("prepare settings scan");
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("scan settings")
            .collect::<Result<Vec<_>, _>>()
            .expect("settings rows");
        drop(stmt);
        for (key, value) in rows {
            let Some(reference) = value.strip_prefix(SECRET_REF_PREFIX) else {
                continue;
            };
            let secret = store.read(reference).expect("fake credential store read");
            assert!(
                secret.is_some(),
                "row '{key}' references missing credential '{reference}'"
            );
        }
    }

    const RACE_WAIT: Duration = Duration::from_millis(200);

    // The interleaving the key lock exists for: set writes the secret, delete
    // completes (row and secret), and only then set inserts the reference row.
    // Without the lock that leaves a row pointing at a missing secret.
    #[test]
    fn a_set_racing_a_delete_of_one_secret_leaves_no_dangling_reference() {
        let conn = shared_settings_db();
        let store = Arc::new(RacingSecretStore::new(RACE_WAIT));

        let set_conn = Arc::clone(&conn);
        let set_store = Arc::clone(&store);
        let setter = std::thread::spawn(move || {
            set_setting_with_store(&set_conn, OPENROUTER_API_KEY, "sk-racing", &*set_store)
        });
        assert!(
            store.stored.wait(Duration::from_secs(5)),
            "the set sequence must reach the credential store first"
        );

        let delete_conn = Arc::clone(&conn);
        let delete_store = Arc::clone(&store);
        let deleter = std::thread::spawn(move || {
            delete_setting_with_store(&delete_conn, OPENROUTER_API_KEY, &*delete_store)
        });

        setter.join().expect("set thread").expect("set sequence");
        deleter
            .join()
            .expect("delete thread")
            .expect("delete sequence");

        assert_no_dangling_secret_ref(&conn, &*store);
        let conn = conn.lock().expect("conn");
        assert_eq!(get_raw_setting(&conn, OPENROUTER_API_KEY), None);
        assert_eq!(store.secret(OPENROUTER_API_KEY), None);
    }

    #[test]
    fn a_delete_racing_a_set_of_one_secret_leaves_no_dangling_reference() {
        let conn = shared_settings_db();
        let store = Arc::new(RacingSecretStore::new(RACE_WAIT));

        let delete_conn = Arc::clone(&conn);
        let delete_store = Arc::clone(&store);
        let deleter = std::thread::spawn(move || {
            delete_setting_with_store(&delete_conn, OPENROUTER_API_KEY, &*delete_store)
        });
        assert!(
            store.delete_started.wait(Duration::from_secs(5)),
            "the delete sequence must reach the credential store first"
        );

        let set_conn = Arc::clone(&conn);
        let set_store = Arc::clone(&store);
        let setter = std::thread::spawn(move || {
            set_setting_with_store(&set_conn, OPENROUTER_API_KEY, "sk-racing", &*set_store)
        });

        setter.join().expect("set thread").expect("set sequence");
        deleter
            .join()
            .expect("delete thread")
            .expect("delete sequence");

        assert_no_dangling_secret_ref(&conn, &*store);
        let conn = conn.lock().expect("conn");
        assert_eq!(
            get_raw_setting(&conn, OPENROUTER_API_KEY).as_deref(),
            Some("secret_ref:openrouter_api_key")
        );
        assert_eq!(
            store.secret(OPENROUTER_API_KEY).as_deref(),
            Some("sk-racing")
        );
    }

    /// Credential store where `store` only returns once two writes have met:
    /// with a single global lock the first set would never see the second.
    #[derive(Default)]
    struct RendezvousSecretStore {
        secrets: Mutex<HashMap<String, String>>,
        arrivals: Mutex<usize>,
        arrived: Condvar,
    }

    impl SecretStore for RendezvousSecretStore {
        fn store(&self, key: &str, value: &str) -> Result<(), String> {
            self.secrets
                .lock()
                .expect("secrets")
                .insert(key.to_string(), value.to_string());
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut arrivals = self.arrivals.lock().expect("arrivals");
            *arrivals += 1;
            self.arrived.notify_all();
            while *arrivals < 2 {
                let now = Instant::now();
                if now >= deadline {
                    return Err("concurrent stores of different keys were serialized".to_string());
                }
                let (next, _) = self
                    .arrived
                    .wait_timeout(arrivals, deadline - now)
                    .expect("rendezvous");
                arrivals = next;
            }
            Ok(())
        }

        fn read(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.secrets.lock().expect("secrets").get(key).cloned())
        }

        fn delete(&self, _key: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn concurrent_sets_of_different_secret_keys_do_not_block_each_other() {
        let conn = shared_settings_db();
        let store = Arc::new(RendezvousSecretStore::default());

        let first_conn = Arc::clone(&conn);
        let first_store = Arc::clone(&store);
        let first = std::thread::spawn(move || {
            set_setting_with_store(&first_conn, OPENROUTER_API_KEY, "sk-one", &*first_store)
        });

        let second_conn = Arc::clone(&conn);
        let second_store = Arc::clone(&store);
        let second = std::thread::spawn(move || {
            set_setting_with_store(&second_conn, GLM_OCR_API_KEY, "sk-two", &*second_store)
        });

        first.join().expect("first thread").expect("first set");
        second.join().expect("second thread").expect("second set");

        let conn = conn.lock().expect("conn");
        assert_eq!(
            get_raw_setting(&conn, OPENROUTER_API_KEY).as_deref(),
            Some("secret_ref:openrouter_api_key")
        );
        assert_eq!(
            get_raw_setting(&conn, GLM_OCR_API_KEY).as_deref(),
            Some("secret_ref:glm_ocr_api_key")
        );
    }

    /// Credential store whose `read` asserts `ui_conn` is not locked: the
    /// settings read must resolve the keyring only after releasing it.
    struct UnlockedReadStore<'a> {
        conn: &'a Mutex<Connection>,
        secret: Option<String>,
        reads: AtomicUsize,
    }

    impl SecretStore for UnlockedReadStore<'_> {
        fn store(&self, key: &str, _value: &str) -> Result<(), String> {
            panic!("unexpected credential store write for '{key}'")
        }

        fn read(&self, key: &str) -> Result<Option<String>, String> {
            assert!(
                self.conn.try_lock().is_ok(),
                "the credential store must be read after `ui_conn` is released"
            );
            assert_eq!(key, OPENROUTER_API_KEY);
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.secret.clone())
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            panic!("unexpected credential store delete for '{key}'")
        }
    }

    #[test]
    fn settings_get_resolves_the_keyring_after_releasing_the_connection() {
        let conn = shared_settings_db();
        set_setting(
            &conn.lock().expect("conn"),
            OPENROUTER_API_KEY,
            "secret_ref:openrouter_api_key",
        )
        .expect("seed reference");
        let store = UnlockedReadStore {
            conn: &conn,
            secret: Some("sk-protected".to_string()),
            reads: AtomicUsize::new(0),
        };

        let value = read_setting_for_ipc(&conn, OPENROUTER_API_KEY, &store).expect("read setting");

        assert_eq!(
            value.as_deref(),
            Some("secret_ref:openrouter_api_key"),
            "the renderer keeps seeing the stored marker, never the secret"
        );
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            1,
            "a secret reference must resolve through the credential store"
        );
    }

    #[test]
    fn settings_get_of_a_plain_key_never_touches_the_keyring() {
        let conn = shared_settings_db();
        set_setting(&conn.lock().expect("conn"), "ui_theme", "dark").expect("seed");
        let store = UnlockedReadStore {
            conn: &conn,
            secret: None,
            reads: AtomicUsize::new(0),
        };

        let value = read_setting_for_ipc(&conn, "ui_theme", &store).expect("read setting");

        assert_eq!(value.as_deref(), Some("dark"));
        assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    }

    /// A-05b's reader for command callers: the credential store must be read
    /// with `ui_conn` already released, exactly like `read_setting_for_ipc`.
    #[test]
    fn the_unlocked_secret_reader_resolves_the_keyring_after_releasing_the_connection() {
        let conn = shared_settings_db();
        set_setting(
            &conn.lock().expect("conn"),
            OPENROUTER_API_KEY,
            "secret_ref:openrouter_api_key",
        )
        .expect("seed reference");
        let store = UnlockedReadStore {
            conn: &conn,
            secret: Some("sk-protected".to_string()),
            reads: AtomicUsize::new(0),
        };

        let value =
            get_secret_setting_unlocked(&conn, OPENROUTER_API_KEY, &store).expect("read setting");

        assert_eq!(value.as_deref(), Some("sk-protected"));
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    }

    /// A-05b's batch reader keeps key order while mixing plain and secret
    /// settings, so `ensure_selected_cloud_key_unlocked` reads its mode and key
    /// from one connection acquisition.
    #[test]
    fn the_unlocked_batch_read_returns_plain_and_secret_values_in_key_order() {
        let conn = shared_settings_db();
        {
            let conn = conn.lock().expect("conn");
            set_setting(&conn, crate::ocr::OCRH_SETTING_MODE, "glm_ocr").expect("mode row");
            set_setting(&conn, GLM_OCR_API_KEY, "secret_ref:glm_ocr_api_key")
                .expect("key reference");
        }
        let store = InMemorySecretStore::with_secret(GLM_OCR_API_KEY, "sk-glm");

        let values = read_settings_unlocked(
            &conn,
            &[crate::ocr::OCRH_SETTING_MODE, GLM_OCR_API_KEY],
            &store,
        )
        .expect("batch read");

        assert_eq!(values[0].as_deref(), Some("glm_ocr"));
        assert_eq!(values[1].as_deref(), Some("sk-glm"));
    }

    /// A blank explicit key resolves the stored secret through the unlocked
    /// helper; an explicit one never touches the connection at all.
    #[test]
    fn the_unlocked_api_key_resolution_prefers_the_explicit_value() {
        let conn = shared_settings_db();
        let store = FailingSecretStore;

        assert_eq!(
            resolve_api_key_input_unlocked(&conn, OPENROUTER_API_KEY, "  explicit  ", &store)
                .expect("explicit key"),
            "explicit"
        );
        let error = resolve_api_key_input_unlocked(&conn, OPENROUTER_API_KEY, "", &store)
            .expect_err("a failing store must not yield a key");
        assert!(error.contains(OPENROUTER_API_KEY), "{error}");
    }

    /// Extracts a top-level function's source: from its signature to the first
    /// closing brace at column 0 (rustfmt keeps top-level braces there).
    fn function_source<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("missing {signature}"));
        let rest = &source[start..];
        let end = rest
            .find("\n}")
            .unwrap_or_else(|| panic!("unclosed {signature}"));
        &rest[..end]
    }

    /// A-05a's structural guarantee: the commands and their sequences never
    /// call the raw keyring helpers, so no credential I/O can sit inside a
    /// `ui_conn` scope there — they only go through the injected `SecretStore`.
    /// The scan fails if a future edit puts a direct keyring call back.
    #[test]
    fn keyring_calls_stay_out_of_the_ui_conn_scope() {
        const SOURCE: &str = include_str!("settings.rs");
        for signature in [
            "pub async fn settings_get(",
            "pub async fn settings_set(",
            "pub async fn settings_get_all(",
            "pub async fn settings_delete(",
            "fn read_setting_for_ipc(",
            "fn set_setting_with_store(",
            "fn delete_setting_with_store(",
        ] {
            let body = function_source(SOURCE, signature);
            for forbidden in ["store_secret(", "read_secret(", "delete_secret("] {
                assert!(
                    !body.contains(forbidden),
                    "{signature} must not call {forbidden} directly: the credential store is injected"
                );
            }
        }
    }
}
