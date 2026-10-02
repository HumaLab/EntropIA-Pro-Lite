//! The Zotero Web API key (api.zotero.org, v3), kept in Settings next to the
//! other provider keys. This module owns what the key is for today: checking it
//! against `GET /keys/current` and handing the stored key plus the account's
//! user id to the code that writes through the Web API.

use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::db::state::AppDbState;
use crate::settings::{
    get_raw_setting, get_setting, persist_setting, resolve_api_key_input, ZOTERO_API_KEY,
    ZOTERO_USER_ID_KEY,
};

pub const API_BASE: &str = "https://api.zotero.org";
const API_VERSION: &str = "3";

/// What a key may do in one library group (`id` is `all` for the default that
/// applies to every group the account belongs to).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupAccess {
    pub id: String,
    pub library: bool,
    pub write: bool,
}

/// `GET /keys/current`, reduced to what Settings shows and the writers need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyInfo {
    pub user_id: u64,
    pub username: String,
    pub personal_library: bool,
    pub personal_write: bool,
    pub groups: Vec<GroupAccess>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum KeyCheck {
    Valid(KeyInfo),
    /// Zotero does not know the key (403/404), or it cannot be a key at all.
    InvalidKey,
    /// Nothing answered: offline, or api.zotero.org is down.
    Unreachable,
}

pub fn parse_key_info(body: &str) -> Result<KeyInfo, String> {
    let json: Value = serde_json::from_str(body)
        .map_err(|_| "Zotero answered something unreadable".to_string())?;
    let user_id = json
        .get("userID")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Zotero's answer has no user id".to_string())?;
    let username = json
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let flag =
        |value: &Value, name: &str| value.get(name).and_then(Value::as_bool).unwrap_or(false);
    let access = json.get("access");
    let user = access.and_then(|a| a.get("user"));
    let mut groups: Vec<GroupAccess> = access
        .and_then(|a| a.get("groups"))
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(id, value)| GroupAccess {
                    id: id.clone(),
                    library: flag(value, "library"),
                    write: flag(value, "write"),
                })
                .collect()
        })
        .unwrap_or_default();
    // `all` first, then groups by numeric id.
    groups.sort_by_key(|g| {
        (
            g.id != "all",
            g.id.parse::<u64>().unwrap_or(u64::MAX),
            g.id.clone(),
        )
    });
    Ok(KeyInfo {
        user_id,
        username,
        personal_library: user.is_some_and(|u| flag(u, "library")),
        personal_write: user.is_some_and(|u| flag(u, "write")),
        groups,
    })
}

/// Zotero keys are alphanumeric. Anything else is not sent: it cannot be a key,
/// and a malformed header value must not end up in an error.
fn well_formed(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Ask Zotero what `key` is. `Err` is for answers this build cannot classify;
/// its text never includes the key.
pub async fn check_key_at(base: &str, key: &str) -> Result<KeyCheck, String> {
    let key = key.trim();
    if !well_formed(key) {
        return Ok(KeyCheck::InvalidKey);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "Could not build the HTTP client".to_string())?;
    let response = match client
        .get(format!("{}/keys/current", base.trim_end_matches('/')))
        .header("Zotero-API-Key", key)
        .header("Zotero-API-Version", API_VERSION)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return Ok(KeyCheck::Unreachable),
    };
    match response.status().as_u16() {
        200 => {
            let body = response
                .text()
                .await
                .map_err(|_| "Zotero's answer could not be read".to_string())?;
            parse_key_info(&body).map(KeyCheck::Valid)
        }
        403 | 404 => Ok(KeyCheck::InvalidKey),
        status => Err(format!("Zotero answered HTTP {status}")),
    }
}

/// The stored key and the account it belongs to, for code that writes through
/// the Web API. `None` until a key is saved *and* verified: verifying is what
/// learns the user id.
#[allow(dead_code)]
#[derive(Clone, PartialEq, Eq)]
pub struct ZoteroCredentials {
    pub key: String,
    pub user_id: u64,
}

impl std::fmt::Debug for ZoteroCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoteroCredentials")
            .field("key", &"[redacted]")
            .field("user_id", &self.user_id)
            .finish()
    }
}

#[allow(dead_code)]
pub fn stored_credentials(conn: &Connection) -> Option<ZoteroCredentials> {
    let key = get_setting(conn, ZOTERO_API_KEY)?.trim().to_string();
    if key.is_empty() {
        return None;
    }
    let user_id = get_raw_setting(conn, ZOTERO_USER_ID_KEY)?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(ZoteroCredentials { key, user_id })
}

fn remember_user_id(conn: &Connection, user_id: u64) -> Result<(), String> {
    persist_setting(conn, ZOTERO_USER_ID_KEY, &user_id.to_string())
}

/// Verify a Zotero key. A blank `api_key` checks the stored one, and a stored key
/// that checks out also records the account id for the writers.
#[tauri::command]
pub async fn zotero_verify_key(
    api_key: String,
    db: State<'_, AppDbState>,
) -> Result<KeyCheck, String> {
    let checking_stored = api_key.trim().is_empty();
    let key = {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|error| format!("DB lock error: {error}"))?;
        resolve_api_key_input(&conn, ZOTERO_API_KEY, &api_key)?
    };
    let check = check_key_at(API_BASE, &key).await?;
    if let (true, KeyCheck::Valid(info)) = (checking_stored, &check) {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|error| format!("DB lock error: {error}"))?;
        remember_user_id(&conn, info.user_id)?;
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navegador::zotero_copy::port::test_server::{canned, dead_base, serve};
    use rusqlite::Connection;

    const KEY: &str = "AbCdEf0123456789abcdEFgh";

    const FULL_ACCESS: &str = r#"{
        "key": "AbCdEf0123456789abcdEFgh",
        "userID": 4242,
        "username": "agus",
        "displayName": "Agus",
        "access": {
            "user": {"library": true, "files": true, "notes": true, "write": true},
            "groups": {
                "all": {"library": true, "write": false},
                "777": {"library": true, "write": true},
                "12": {"library": true, "write": false}
            }
        }
    }"#;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn settings_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn parses_the_account_and_what_each_library_allows() {
        let info = parse_key_info(FULL_ACCESS).unwrap();
        assert_eq!(info.user_id, 4242);
        assert_eq!(info.username, "agus");
        assert!(info.personal_library);
        assert!(info.personal_write);
        // "all" first, then groups by numeric id.
        let groups: Vec<(&str, bool, bool)> = info
            .groups
            .iter()
            .map(|g| (g.id.as_str(), g.library, g.write))
            .collect();
        assert_eq!(
            groups,
            vec![
                ("all", true, false),
                ("12", true, false),
                ("777", true, true)
            ]
        );
    }

    #[test]
    fn a_read_only_key_with_no_group_access_parses_with_everything_off() {
        let body = r#"{"userID": 1, "username": "u", "access": {"user": {"library": true}}}"#;
        let info = parse_key_info(body).unwrap();
        assert!(info.personal_library);
        assert!(!info.personal_write);
        assert!(info.groups.is_empty());
    }

    #[test]
    fn an_answer_without_a_user_id_is_not_a_key_description() {
        assert!(parse_key_info(r#"{"username": "u"}"#).is_err());
        assert!(parse_key_info("not json").is_err());
    }

    #[test]
    fn a_valid_key_is_sent_in_the_header_with_the_api_version() {
        let server = serve(vec![canned("GET", "/keys/current", 200, FULL_ACCESS)]);
        let check = runtime().block_on(check_key_at(&server.base, KEY)).unwrap();
        assert!(matches!(check, KeyCheck::Valid(ref info) if info.user_id == 4242));
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].target, "/keys/current");
        assert_eq!(
            seen[0].headers.get("zotero-api-key").map(String::as_str),
            Some(KEY)
        );
        assert_eq!(
            seen[0]
                .headers
                .get("zotero-api-version")
                .map(String::as_str),
            Some("3")
        );
        // The key travels in the header only, never in the address.
        assert!(!seen[0].target.contains(KEY));
    }

    #[test]
    fn forbidden_and_not_found_mean_an_invalid_key() {
        for status in [403, 404] {
            let server = serve(vec![canned("GET", "/keys/current", status, "Forbidden")]);
            let check = runtime().block_on(check_key_at(&server.base, KEY)).unwrap();
            assert_eq!(check, KeyCheck::InvalidKey, "status {status}");
        }
    }

    #[test]
    fn nothing_listening_means_zotero_could_not_be_reached() {
        let check = runtime().block_on(check_key_at(&dead_base(), KEY)).unwrap();
        assert_eq!(check, KeyCheck::Unreachable);
    }

    #[test]
    fn other_statuses_are_an_error_that_never_carries_the_key() {
        let server = serve(vec![canned("GET", "/keys/current", 500, KEY)]);
        let error = runtime()
            .block_on(check_key_at(&server.base, KEY))
            .unwrap_err();
        assert!(error.contains("500"), "{error}");
        assert!(!error.contains(KEY), "{error}");
    }

    #[test]
    fn a_malformed_key_is_rejected_without_a_request() {
        let server = serve(vec![canned("GET", "/keys/current", 200, FULL_ACCESS)]);
        for bad in ["", "   ", "has space inside", "line\nbreak", "ñandú"] {
            let check = runtime().block_on(check_key_at(&server.base, bad)).unwrap();
            assert_eq!(check, KeyCheck::InvalidKey, "{bad:?}");
        }
        assert!(server.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn credentials_need_both_the_key_and_the_user_id() {
        let conn = settings_db();
        assert!(stored_credentials(&conn).is_none());

        // Plaintext rows resolve without the OS keyring, so the lookup is
        // testable here; real rows hold a `secret_ref:` instead.
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('zotero_api_key', ?1)",
            [KEY],
        )
        .unwrap();
        assert!(
            stored_credentials(&conn).is_none(),
            "a key whose account was never verified has no user id yet"
        );

        remember_user_id(&conn, 4242).unwrap();
        let credentials = stored_credentials(&conn).unwrap();
        assert_eq!(credentials.key, KEY);
        assert_eq!(credentials.user_id, 4242);
    }

    #[test]
    fn credentials_never_print_the_key() {
        let credentials = ZoteroCredentials {
            key: KEY.to_string(),
            user_id: 7,
        };
        let shown = format!("{credentials:?}");
        assert!(!shown.contains(KEY), "{shown}");
        assert!(shown.contains('7'));
    }

    #[test]
    fn a_blank_user_id_row_is_not_an_account() {
        let conn = settings_db();
        conn.execute_batch(
            "INSERT INTO app_settings (key, value) VALUES ('zotero_api_key', 'k');
             INSERT INTO app_settings (key, value) VALUES ('zotero_user_id', 'abc');",
        )
        .unwrap();
        assert!(stored_credentials(&conn).is_none());
    }
}
