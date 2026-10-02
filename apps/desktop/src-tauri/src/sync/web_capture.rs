//! Row sync for saved web captures (`web_sources`, `web_captures`), negotiated
//! with the exact capability `web-capture-v1` (PROTOCOL "Capturas web").
//!
//! Unlike writing and research, these are ordinary physical tables: the generic
//! trigger capture, push and apply paths carry them. What this module adds is
//! the capability discipline around that generic path:
//!
//! - Ordinary push/pull requests always opt in (`http`), so a capable server
//!   delivers the rows in the normal stream and a legacy one ignores the token.
//! - The capability is trusted only from an ordinary response and only for the
//!   server epoch that advertised it ([`record_capability`]). Until then the
//!   push path holds web rows back in the oplog: a legacy server answers `400`
//!   for the whole batch, which would also block the corpus rows.
//! - A client whose shared cursor already passed rows a legacy pull never saw
//!   needs one since-zero catch-up per epoch ([`run_catchup`]). It applies web
//!   rows only and never moves the shared `last_pull_seq` cursor.
//! - A remote `web_sources` tombstone queues the removal of that source's
//!   local folder ([`queue_folder_removal`], [`drain_folder_removals`]) in the
//!   same transaction as the delete; the folder is removed after commit and
//!   only when no source row with that id exists.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::apply::{apply_page, retry_pending_rows, ApplyContext};
use super::http::{PullRow, SyncApi, SyncError};
use super::pull::PULL_LIMIT;
use super::session::{meta_delete, meta_get, meta_get_i64, meta_set_i64};

/// The two wire tables the capability gates.
pub(crate) const WEB_TABLES: [&str; 2] = ["web_sources", "web_captures"];

const CAPABILITY_KEY: &str = "web_capture_capability";
const CATCHUP_KEY: &str = "web_capture_catchup_epoch";
const CATCHUP_CURSOR_KEY: &str = "web_capture_catchup_since";
/// Literal prefix of the folder-removal queue (matched with `substr`, never
/// `LIKE`: the underscores would be wildcards).
pub(crate) const FOLDER_REMOVAL_PREFIX: &str = "web_capture_remove_dir:";

/// True for the tables gated by `web-capture-v1`.
pub(crate) fn is_web_table(table: &str) -> bool {
    WEB_TABLES.contains(&table)
}

fn current_epoch(conn: &Connection) -> Result<Option<String>, String> {
    meta_get(conn, "server_epoch")
}

/// Records what an ordinary response advertised for `server_epoch`. A later
/// response that omits the token overwrites an earlier one (a rolled-back
/// server must not keep receiving web rows).
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
    .map_err(|e| format!("[sync] failed to record web capture capability: {e}"))
}

/// True only when the capability was advertised at exactly the current server
/// epoch. A malformed record fails closed.
pub(crate) fn supports_web_capture(conn: &Connection) -> Result<bool, String> {
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
    .map_err(|e| format!("[sync] failed to record web capture catch-up: {e}"))?;
    meta_delete(conn, CATCHUP_CURSOR_KEY)
}

/// Forgets every account-bound key owned by this module (logout / account
/// switch). Runs inside the caller's transaction.
pub(crate) fn clear_account_metadata(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "DELETE FROM sync_meta
          WHERE key IN (?1, ?2, ?3)
             OR substr(key, 1, length(?4)) = ?4",
        params![
            CAPABILITY_KEY,
            CATCHUP_KEY,
            CATCHUP_CURSOR_KEY,
            FOLDER_REMOVAL_PREFIX
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to clear web capture metadata: {e}"))
}

/// One since-zero catch-up per server epoch, once the capability is known.
///
/// Pages are pulled with the ordinary request (which carries the opt-in), only
/// web rows are applied, and the shared `last_pull_seq` is re-persisted at its
/// current value: corpus history is never skipped or duplicated from here. The
/// catch-up cursor is its own key, so an interrupted run resumes where it
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
    if !supports_web_capture(conn).map_err(SyncError::Decode)?
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
            .filter(|row| is_web_table(&row.table))
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
        // The pull loop's drains already ran this cycle: install what the
        // catch-up queued now instead of waiting for the next one.
        drain_folder_removals(conn, app_data_dir).map_err(SyncError::Decode)?;
        super::web_blobs::drain_pending_web_blobs(api, token, conn, app_data_dir)
            .await
            .map_err(SyncError::Decode)?;
        return Ok(());
    }
}

/// Queues the removal of `web-captures/<source_id>/` for a remote tombstone.
/// Called inside the apply transaction so the queue entry and the delete commit
/// (or roll back) together.
pub(crate) fn queue_folder_removal(conn: &Connection, source_id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO NOTHING",
        params![format!("{FOLDER_REMOVAL_PREFIX}{source_id}")],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to queue web capture folder removal: {e}"))
}

/// Removes the folders of remotely deleted sources, after the delete committed.
/// An entry whose source row exists again (re-created or never deleted) is
/// dropped without touching disk. A folder that cannot be removed stays
/// queued for the next cycle; the startup sweep is the backstop.
pub(crate) fn drain_folder_removals(
    conn: &Connection,
    app_data_dir: &Path,
) -> Result<usize, String> {
    use crate::navegador::capture_files::{self, Located};

    let mut stmt = conn
        .prepare("SELECT key FROM sync_meta WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key")
        .map_err(|e| format!("[sync] failed to read folder removal queue: {e}"))?;
    let keys: Vec<String> = stmt
        .query_map([FOLDER_REMOVAL_PREFIX], |row| row.get(0))
        .map_err(|e| format!("[sync] failed to read folder removal queue: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("[sync] failed to read folder removal queue: {e}"))?;
    drop(stmt);

    let mut removed = 0usize;
    for key in keys {
        let source_id = key[FOLDER_REMOVAL_PREFIX.len()..].to_string();
        let source_exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM web_sources WHERE id = ?1",
                [&source_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("[sync] failed to check web source {source_id}: {e}"))?;
        if source_exists.is_some() {
            meta_delete(conn, &key)?;
            continue;
        }
        let done = match capture_files::locate_source_dir(app_data_dir, &source_id) {
            Located::Missing => true,
            Located::Dir(dir) => capture_files::remove_dir_contents(&dir).dir_removed,
            // A link or an invalid id is never touched; do not retry forever.
            Located::Refused(_) => true,
        };
        if done {
            meta_delete(conn, &key)?;
            removed += 1;
        }
    }
    Ok(removed)
}
