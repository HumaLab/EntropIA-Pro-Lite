//! Row sync for the global entity alias list (`entity_aliases`), negotiated
//! with the exact capability `entity-aliases-v1` (T4, S4).
//!
//! Like saved web captures, these are ordinary physical tables: the generic
//! trigger capture, push and apply paths carry them. What this module adds is
//! the capability discipline around that generic path, copied from
//! [`super::web_capture`]:
//!
//! - Ordinary push/pull requests always opt in (`http`), so a capable server
//!   delivers the rows in the normal stream and a legacy one ignores the token.
//! - The capability is trusted only from an ordinary response and only for the
//!   server epoch that advertised it ([`record_capability`]). Until then the
//!   push path holds alias rows back in the oplog: a legacy server answers
//!   `400` for the whole batch, which would also block the corpus rows.
//! - A client whose shared cursor already passed rows a legacy pull never saw
//!   needs one since-zero catch-up per epoch ([`run_catchup`]). It applies alias
//!   rows only and never moves the shared `last_pull_seq` cursor.

use std::path::Path;

use rusqlite::{params, Connection};

use super::apply::{apply_page, retry_pending_rows, ApplyContext};
use super::http::{PullRow, SyncApi, SyncError};
use super::pull::PULL_LIMIT;
use super::session::{meta_delete, meta_get, meta_get_i64, meta_set_i64};

/// The one wire table the capability gates.
pub(crate) const ALIAS_TABLES: [&str; 1] = ["entity_aliases"];

const CAPABILITY_KEY: &str = "entity_aliases_capability";
const CATCHUP_KEY: &str = "entity_aliases_catchup_epoch";
const CATCHUP_CURSOR_KEY: &str = "entity_aliases_catchup_since";

/// True for the tables gated by `entity-aliases-v1`.
pub(crate) fn is_entity_alias_table(table: &str) -> bool {
    ALIAS_TABLES.contains(&table)
}

fn current_epoch(conn: &Connection) -> Result<Option<String>, String> {
    meta_get(conn, "server_epoch")
}

/// Records what an ordinary response advertised for `server_epoch`. A later
/// response that omits the token overwrites an earlier one (a rolled-back
/// server must not keep receiving alias rows).
pub(crate) fn record_capability(
    conn: &Connection,
    server_epoch: &str,
    advertised: bool,
) -> Result<(), String> {
    let value = serde_json::json!({ "epoch": server_epoch, "advertised": advertised }).to_string();
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CAPABILITY_KEY, value],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to record entity alias capability: {e}"))
}

/// True only when the capability was advertised at exactly the current server
/// epoch. A malformed record fails closed.
pub(crate) fn supports_entity_aliases(conn: &Connection) -> Result<bool, String> {
    let Some(epoch) = current_epoch(conn)? else {
        return Ok(false);
    };
    let Some(raw) = meta_get(conn, CAPABILITY_KEY)? else {
        return Ok(false);
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(false);
    };
    Ok(parsed
        .get("advertised")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        && parsed.get("epoch").and_then(serde_json::Value::as_str) == Some(epoch.as_str()))
}

/// True until this epoch's one-time since-zero catch-up has completed.
pub(crate) fn catchup_needed(conn: &Connection, server_epoch: &str) -> Result<bool, String> {
    Ok(meta_get(conn, CATCHUP_KEY)?.as_deref() != Some(server_epoch))
}

/// Marks this epoch's catch-up complete and drops its cursor.
pub(crate) fn record_catchup_done(conn: &Connection, server_epoch: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CATCHUP_KEY, server_epoch],
    )
    .map_err(|e| format!("[sync] failed to record entity alias catch-up: {e}"))?;
    meta_delete(conn, CATCHUP_CURSOR_KEY)
}

/// Forgets every account-bound key owned by this module (logout / account
/// switch). Runs inside the caller's transaction.
pub(crate) fn clear_account_metadata(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "DELETE FROM sync_meta
          WHERE key IN (?1, ?2, ?3)",
        params![CAPABILITY_KEY, CATCHUP_KEY, CATCHUP_CURSOR_KEY],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to clear entity alias metadata: {e}"))
}

/// One since-zero catch-up per server epoch, once the capability is known.
///
/// Pages are pulled with the ordinary request (which carries the opt-in), only
/// alias rows are applied, and the shared `last_pull_seq` is re-persisted at
/// its current value: corpus history is never skipped or duplicated from here.
/// The catch-up cursor is its own key, so an interrupted run resumes where it
/// stopped instead of restarting.
pub(crate) async fn run_catchup<A: SyncApi>(
    api: &A,
    token: &str,
    schema_tag: &str,
    conn: &Connection,
    app_data_dir: &Path,
) -> Result<(), SyncError> {
    let Some(epoch) = current_epoch(conn).map_err(SyncError::Decode)? else {
        return Ok(());
    };
    if !supports_entity_aliases(conn).map_err(SyncError::Decode)?
        || !catchup_needed(conn, &epoch).map_err(SyncError::Decode)?
    {
        return Ok(());
    }

    let mut ctx = ApplyContext::new(app_data_dir);
    loop {
        let since = meta_get_i64(conn, CATCHUP_CURSOR_KEY).map_err(SyncError::Decode)?;
        let page = api.pull(token, schema_tag, since, PULL_LIMIT).await?;
        if page.server_epoch != epoch {
            // Another epoch took over mid-run: the next cycle reconciles first.
            return Ok(());
        }
        let rows: Vec<PullRow> = page
            .rows
            .into_iter()
            .filter(|row| is_entity_alias_table(&row.table))
            .collect();
        if !rows.is_empty() {
            let shared = meta_get_i64(conn, "last_pull_seq").map_err(SyncError::Decode)?;
            apply_page(conn, &mut ctx, &rows, shared).map_err(SyncError::Decode)?;
        }
        if page.has_more {
            meta_set_i64(conn, CATCHUP_CURSOR_KEY, page.next_since).map_err(SyncError::Decode)?;
            continue;
        }
        retry_pending_rows(conn, &mut ctx, false).map_err(SyncError::Decode)?;
        record_catchup_done(conn, &epoch).map_err(SyncError::Decode)?;
        return Ok(());
    }
}
