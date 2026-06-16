//! Pull cycle (PROTOCOL "Flujo de sincronización del cliente" step 6, DESIGN
//! §4.3, §4.5, §4.9). Drives the paginated `GET /sync/pull` loop, applies each
//! page through [`super::apply`], retries parked rows at the page and cycle
//! boundaries, seeds pre-existing rows after the first full pull, and handles
//! the server-epoch / `409 cursor_ahead` reconciliation.
//!
//! The async network orchestration is generic over [`SyncApi`] so the whole
//! cycle is testable against the in-memory mock with no server.

use std::path::Path;

use rusqlite::Connection;

use crate::sync::apply::{apply_page, retry_pending_rows, ApplyContext};
use crate::sync::capture::SYNCED_TABLES;
use crate::sync::http::{SyncApi, SyncError};
use crate::sync::session::{meta_get, meta_get_i64, meta_set, meta_set_i64};

/// Default page size requested from the server (PROTOCOL: `limit` default 500,
/// max 1000).
pub const PULL_LIMIT: i64 = 500;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Summary of a pull cycle for the caller / status event.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PullCycleOutcome {
    pub pages: usize,
    pub applied: usize,
    pub skipped: usize,
    pub journaled: usize,
    pub parked: usize,
    /// True when a remote `schema_tag` ahead of the local head cut the loop short
    /// (PROTOCOL step 6 — the client must upgrade before continuing).
    pub schema_cut: bool,
    /// True when the epoch reconciliation procedure ran this cycle (DESIGN §4.9).
    pub reconciled: bool,
    /// Blobs downloaded this cycle (PROTOCOL step 7).
    pub blobs_downloaded: usize,
    /// Items reindexed into FTS this cycle (PROTOCOL step 8).
    pub fts_reindexed: usize,
}

// ---------------------------------------------------------------------------
// Epoch reconciliation (DESIGN §4.9, PROTOCOL "Época del servidor")
// ---------------------------------------------------------------------------

/// Resets the local cursor and row-versions for a full reconciliation pull
/// (DESIGN §4.9): `last_pull_seq=0`, empty `sync_row_versions`, and (when a
/// non-empty `new_epoch` is given) persist the new epoch. `sync_blob_index` is
/// intentionally retained (content-addressed; HEAD auto-corrects). Runs in one
/// transaction. The `409 cursor_ahead` path passes an empty epoch (it doesn't
/// have a fresh one yet — the next response's `check_epoch` persists it).
pub fn reset_for_reconciliation(conn: &Connection, new_epoch: &str) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin reconciliation reset: {e}"))?;
    meta_set_i64(&tx, "last_pull_seq", 0)?;
    tx.execute_batch("DELETE FROM sync_row_versions;")
        .map_err(|e| format!("[sync] failed to clear row versions: {e}"))?;
    if !new_epoch.is_empty() {
        meta_set(&tx, "server_epoch", new_epoch)?;
    }
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit reconciliation reset: {e}"))
}

/// Compares the persisted `server_epoch` with a fresh `server_epoch` sample and,
/// on mismatch (a restore that rewound `server_seq`), runs the reconciliation
/// reset (DESIGN §4.9). A first-ever epoch is just persisted. Returns true when a
/// reset actually happened.
pub fn check_epoch(conn: &Connection, server_epoch: &str) -> Result<bool, String> {
    if server_epoch.is_empty() {
        return Ok(false);
    }
    match meta_get(conn, "server_epoch")? {
        None => {
            meta_set(conn, "server_epoch", server_epoch)?;
            Ok(false)
        }
        Some(stored) if stored == server_epoch => Ok(false),
        Some(_) => {
            reset_for_reconciliation(conn, server_epoch)?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Seeding (DESIGN §4.5)
// ---------------------------------------------------------------------------

/// Seeds the oplog with `'I'` entries for every pre-existing local row the server
/// has NOT already versioned (DESIGN §4.5), for all 15 synced tables, in one
/// transaction, guarded by `sync_meta['seeded_account']`. Idempotent: re-running
/// after a crash adds no duplicates (the `WHERE NOT EXISTS` against
/// `sync_row_versions` plus the natural coalescing of identical `(table, row_id)`
/// oplog entries converge). Must run AFTER `ensure_capture` and AFTER the first
/// full `since=0` pull (which populates `sync_row_versions`).
pub fn seed_account(conn: &Connection, account_id: &str) -> Result<(), String> {
    if meta_get(conn, "seeded_account")?.as_deref() == Some(account_id) {
        return Ok(());
    }
    let now = now_ms();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin seeding tx: {e}"))?;

    for table in SYNCED_TABLES {
        // Identifiers come from the compile-time allowlist — safe to interpolate.
        let sql = format!(
            "INSERT INTO sync_oplog(table_name, row_id, op, changed_at)
             SELECT '{table}', id, 'I', ?1 FROM \"{table}\"
             WHERE NOT EXISTS (
               SELECT 1 FROM sync_row_versions v
               WHERE v.table_name = '{table}' AND v.row_id = \"{table}\".id
             )",
        );
        tx.execute(&sql, rusqlite::params![now])
            .map_err(|e| format!("[sync] failed to seed {table}: {e}"))?;
    }

    meta_set(&tx, "seeded_account", account_id)?;
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit seeding: {e}"))
}

// ---------------------------------------------------------------------------
// Pull loop (PROTOCOL step 6)
// ---------------------------------------------------------------------------

/// Runs the paginated pull loop until `has_more` is false (or a schema cut), then
/// a final pending-row retry pass (DESIGN §4.3). The local `schema_tag` is passed
/// fresh by the caller (the engine reads `_migrations` head per cycle). Each page
/// is applied in its own transaction with cursor + queues persisted atomically;
/// parked rows are retried after every page and once more at the end.
///
/// `local_schema_tag` is the head used for the request AND for the `schema_tag >
/// local` cut (PROTOCOL step 6). `app_data_dir` scopes asset rel_path validation.
pub async fn pull_loop<A: SyncApi>(
    api: &A,
    token: &str,
    local_schema_tag: &str,
    conn: &Connection,
    app_data_dir: &Path,
) -> Result<PullCycleOutcome, SyncError> {
    let mut outcome = PullCycleOutcome::default();
    let mut ctx = ApplyContext::new(app_data_dir);

    // Cycle-start pending-row retry (DESIGN §4.3).
    retry_pending_rows(conn, &mut ctx, false).map_err(SyncError::Decode)?;

    loop {
        let since = meta_get_i64(conn, "last_pull_seq").map_err(SyncError::Decode)?;
        let page = match api.pull(token, local_schema_tag, since, PULL_LIMIT).await {
            Ok(page) => page,
            Err(SyncError::Api { status: 409, .. }) => {
                // cursor_ahead → the server was restored without rotating the
                // epoch; reconcile (DESIGN §4.9) and restart the loop. The epoch
                // is left untouched (empty arg) — the next response persists it.
                reset_for_reconciliation(conn, "").map_err(SyncError::Decode)?;
                outcome.reconciled = true;
                continue;
            }
            Err(other) => return Err(other),
        };

        // Refresh the clock offset from this page's server_now_ms.
        if page.server_now_ms != 0 {
            crate::sync::push::update_clock_offset(conn, page.server_now_ms)
                .map_err(SyncError::Decode)?;
        }

        // Epoch check (DESIGN §4.9): a mismatch resets and restarts the loop.
        if check_epoch(conn, &page.server_epoch).map_err(SyncError::Decode)? {
            outcome.reconciled = true;
            continue;
        }

        // Apply the page in one transaction (cursor persisted inside).
        let page_outcome =
            apply_page(conn, &mut ctx, &page.rows, page.next_since).map_err(SyncError::Decode)?;
        outcome.pages += 1;
        outcome.applied += page_outcome.applied;
        outcome.skipped += page_outcome.skipped;
        outcome.journaled += page_outcome.journaled;
        outcome.parked += page_outcome.parked;

        // Retry parked rows after each page (DESIGN §4.3).
        retry_pending_rows(conn, &mut ctx, false).map_err(SyncError::Decode)?;

        // PROTOCOL step 6: if the server's schema_tag is ahead of ours, cut the
        // loop (we must upgrade before continuing).
        if !page.schema_tag.is_empty() && page.schema_tag.as_str() > local_schema_tag {
            outcome.schema_cut = true;
            break;
        }

        if !page.has_more {
            break;
        }
    }

    // Final pending-row retry, with parent_deleted journaling for confirmed
    // tombstones (DESIGN §4.3).
    retry_pending_rows(conn, &mut ctx, true).map_err(SyncError::Decode)?;

    // Step 7: drain the blob download queue (temp + verify + rename; per-blob
    // backoff). Network errors here do not fail the cycle — blobs stay queued.
    outcome.blobs_downloaded =
        crate::sync::blobs::drain_pending_blobs(api, token, conn, app_data_dir)
            .await
            .map_err(SyncError::Decode)?;

    // Step 8: drain the FTS reindex queue (idempotent; derived state).
    outcome.fts_reindexed =
        crate::sync::apply::drain_pending_fts(conn).map_err(SyncError::Decode)?;

    Ok(outcome)
}

#[cfg(test)]
mod tests;
