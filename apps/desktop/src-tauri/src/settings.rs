use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::state::AppDbState;
// Bootstrap remote-source plumbing is local-ml only (the hosted managed-runtime). The
// type comes from the always-compiled `bootstrap_types`; the import + every fn below is
// gated because only the (gated) RuntimeManager consumes them.
#[cfg(feature = "local-ml")]
use crate::runtime::bootstrap_types::BootstrapRemoteSource;

pub const RUNTIME_BOOTSTRAP_MANIFEST_URL_KEY: &str = "runtime_bootstrap_manifest_url";
pub const RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID_KEY: &str = "runtime_bootstrap_public_key_id";
pub const RUNTIME_BOOTSTRAP_PUBLIC_KEY_KEY_PREFIX: &str = "runtime_bootstrap_public_key.";
const REDACTED_SETTING_VALUE: &str = "[redacted]";
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
    let result = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(result)
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
        conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
            params![key.as_str(), value.as_str()],
        )
        .map_err(|e| format!("Failed to save setting: {e}"))?;
    }
    if should_invalidate {
        invalidate_dependency_probe_cache_if_needed(&key, Some(&deps)).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn settings_get_all(db: State<'_, AppDbState>) -> Result<Vec<SettingEntry>, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock error: {e}"))?;
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
    let mut entries = Vec::new();
    for row in rows {
        if let Ok(entry) = row {
            entries.push(entry);
        }
    }
    Ok(entries)
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
        || normalized.contains("token")
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
        conn.execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![key.as_str()],
        )
        .map_err(|e| format!("Failed to delete setting: {e}"))?;
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
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// Persist a setting value directly from Rust-side worker code.
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
pub fn delete_setting(conn: &rusqlite::Connection, key: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
    Ok(())
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
        let entry = redact_setting_entry(SettingEntry {
            key: "openrouter_model".to_string(),
            value: "google/gemma-3-4b-it".to_string(),
        });

        assert_eq!(entry.key, "openrouter_model");
        assert_eq!(entry.value, "google/gemma-3-4b-it");
    }

    #[test]
    fn returns_none_when_runtime_bootstrap_source_is_not_configured() {
        let conn = in_memory_settings_db();

        let source = get_runtime_bootstrap_remote_source_with_builtin(&conn, None, None, None)
            .expect("lookup should succeed");

        assert_eq!(source, None);
    }

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

    #[test]
    fn rejects_missing_runtime_bootstrap_public_key() {
        let conn = in_memory_settings_db();

        let error =
            get_runtime_bootstrap_public_key_with_builtin(&conn, "entropia-root", None, None)
                .expect_err("missing public key should fail");

        assert!(error.contains("entropia-root"));
    }
}
