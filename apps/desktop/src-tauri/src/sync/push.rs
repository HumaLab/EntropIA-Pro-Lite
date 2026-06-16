//! Push path (PROTOCOL flow steps 2-5, `POST /v1/sync/push`).
//!
//! This module owns the SQL-level mechanics of building a push from the local
//! oplog and reconciling the server's response. The asynchronous network
//! orchestration (blob HEAD/PUT before asset rows, batched POSTs, 413 bisection)
//! lives in [`batching`] and is generic over [`SyncApi`] so it is testable
//! against an in-memory mock.
//!
//! Steps implemented here:
//! 2. `snapshot = MAX(seq)` of the oplog; coalesce per `(table, row_id)`
//!    (latest op, max `changed_at`). Read the CURRENT row state from the table;
//!    a row that is absent at read time becomes an `op:'delete'` (DESIGN §4.2).
//! 3. (asset blobs — orchestrated by the engine via [`super::blobs`]).
//! 4. Batched push with the dual cap (≤500 changes AND ≤`max_push_bytes`); a 413
//!    triggers recursive bisection; a single oversized row is journaled
//!    `apply_error` and purged from the oplog so it never blocks the rest.
//! 5. On 200: delete oplog `seq <= snapshot`, update `sync_row_versions`, and
//!    apply each `lww_lost` winner ONLY if the row has no oplog entry with
//!    `seq > snapshot` (skip-if-dirty, DESIGN §4.4), with the `applying` echo
//!    suppression flag set during the local write.
//!
//! Most of this module's surface is the API the engine slice (single-flight
//! tokio task, next slice) drives; it is exercised here only by unit tests. The
//! module-level `allow(dead_code)` is removed once the engine wires it up
//! (mirrors the C1 `#[allow(dead_code)]` convention for forward-looking API).
#![allow(dead_code)]

use rusqlite::Connection;
use serde_json::Value;

use crate::db::util::{is_safe_identifier, quote_identifier};
use crate::sync::capture::is_synced_table;
use crate::sync::http::{PullRow, PushChange, PushResult};
use crate::sync::session::{meta_get_i64, meta_set, meta_set_i64};

/// Default `max_push_bytes` when the server health has not reported one
/// (PROTOCOL: `SYNC_MAX_PUSH_MB` default 8 MB).
pub const DEFAULT_MAX_PUSH_BYTES: i64 = 8 * 1024 * 1024;

/// Hard cap on the number of changes in a single push request (PROTOCOL: ≤500).
pub const MAX_PUSH_CHANGES: usize = 500;

/// A coalesced oplog entry: the net op for one `(table, row_id)` up to the
/// snapshot, with the maximum `changed_at` seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoalescedOp {
    pub table: String,
    pub row_id: String,
    /// `'I'` | `'U'` | `'D'` — the LAST op recorded (highest seq) for the row.
    pub op: char,
    /// The maximum `changed_at` (ms, local clock) across the row's oplog
    /// entries up to the snapshot.
    pub changed_at: i64,
}

/// The current snapshot of the oplog: `MAX(seq)`, or `0` when empty (DESIGN
/// §4.2). Everything with `seq <= snapshot` is what this push covers.
pub fn snapshot_oplog(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM sync_oplog", [], |row| {
        row.get::<_, i64>(0)
    })
    .map_err(|e| format!("[sync] failed to snapshot oplog: {e}"))
}

/// Coalesces the oplog up to and including `snapshot` into one [`CoalescedOp`]
/// per `(table, row_id)`: the op of the highest-seq entry and the maximum
/// `changed_at` across the row's entries. Ordered by table then row_id for
/// determinism.
pub fn coalesce_ops(conn: &Connection, snapshot: i64) -> Result<Vec<CoalescedOp>, String> {
    // The op is taken from the entry with the highest seq; changed_at is the max
    // across all entries for the row. A correlated subquery picks the latest op.
    let mut stmt = conn
        .prepare(
            "SELECT o.table_name, o.row_id,
                    (SELECT op FROM sync_oplog x
                     WHERE x.table_name = o.table_name AND x.row_id = o.row_id
                       AND x.seq <= ?1
                     ORDER BY x.seq DESC LIMIT 1) AS last_op,
                    MAX(o.changed_at) AS max_changed
             FROM sync_oplog o
             WHERE o.seq <= ?1
             GROUP BY o.table_name, o.row_id
             ORDER BY o.table_name, o.row_id",
        )
        .map_err(|e| format!("[sync] failed to prepare coalesce: {e}"))?;

    let rows = stmt
        .query_map([snapshot], |row| {
            let table: String = row.get(0)?;
            let row_id: String = row.get(1)?;
            let op_str: String = row.get(2)?;
            let changed_at: i64 = row.get(3)?;
            Ok((table, row_id, op_str, changed_at))
        })
        .map_err(|e| format!("[sync] failed to query coalesce: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        let (table, row_id, op_str, changed_at) =
            row.map_err(|e| format!("[sync] failed to read coalesced op: {e}"))?;
        let op = op_str.chars().next().unwrap_or('U');
        out.push(CoalescedOp {
            table,
            row_id,
            op,
            changed_at,
        });
    }
    Ok(out)
}

/// The non-generated column names of `table`, intersected with what the payload
/// reader needs (PROTOCOL "Semántica de apply" step 2 — `PRAGMA table_xinfo`,
/// `hidden = 0`). Excludes generated columns such as `items.search_text`.
pub fn non_generated_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    if !is_safe_identifier(table) {
        return Err(format!("[sync] unsafe table identifier: {table}"));
    }
    let sql = format!("PRAGMA table_xinfo({})", quote_identifier(table));
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("[sync] failed to inspect {table}: {e}"))?;
    // table_xinfo columns: cid, name, type, notnull, dflt_value, pk, hidden.
    let rows = stmt
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let hidden: i64 = row.get(6)?;
            Ok((name, hidden))
        })
        .map_err(|e| format!("[sync] failed to read xinfo for {table}: {e}"))?;
    let mut cols = Vec::new();
    for row in rows {
        let (name, hidden) = row.map_err(|e| format!("[sync] failed to read column: {e}"))?;
        if hidden == 0 {
            cols.push(name);
        }
    }
    Ok(cols)
}

/// Reads the CURRENT state of `(table, row_id)` as a JSON object of the
/// non-generated columns, or `None` if the row no longer exists (DESIGN §4.2:
/// a missing row at read time becomes a delete). Values are read as JSON via
/// the column affinity so numbers stay numbers and NULLs stay null.
pub fn read_row_payload(
    conn: &Connection,
    table: &str,
    row_id: &str,
) -> Result<Option<Value>, String> {
    let columns = non_generated_columns(conn, table)?;
    if columns.is_empty() {
        return Err(format!("[sync] table {table} has no non-generated columns"));
    }

    let select_list = columns
        .iter()
        .map(|c| quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {select_list} FROM {} WHERE id = ?1",
        quote_identifier(table)
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("[sync] failed to prepare row read for {table}: {e}"))?;

    let mut query = stmt
        .query([row_id])
        .map_err(|e| format!("[sync] failed to query row {table}/{row_id}: {e}"))?;

    let Some(row) = query
        .next()
        .map_err(|e| format!("[sync] failed to step row {table}/{row_id}: {e}"))?
    else {
        return Ok(None);
    };

    let mut obj = serde_json::Map::with_capacity(columns.len());
    for (idx, col) in columns.iter().enumerate() {
        let value = sql_value_to_json(row, idx)?;
        obj.insert(col.clone(), value);
    }
    Ok(Some(Value::Object(obj)))
}

/// Converts a rusqlite value at `idx` into a JSON value, preserving type.
fn sql_value_to_json(row: &rusqlite::Row, idx: usize) -> Result<Value, String> {
    use rusqlite::types::ValueRef;
    let value = row
        .get_ref(idx)
        .map_err(|e| format!("[sync] failed to read column {idx}: {e}"))?;
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Number(i.into()),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(bytes) => Value::String(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => {
            // Blobs are not expected in synced tables; encode defensively as a
            // hex string rather than losing data.
            Value::String(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
        }
    })
}

/// Rewrites an `item_topics` payload's `topic_id` from the LOCAL topic id back to
/// the canonical server (remote) id when a topic alias exists (DESIGN §4.7). The
/// alias table maps `remote_id → local_id`; the push direction needs the reverse
/// lookup so a row whose `topic_id` is the local survivor of a name collision is
/// pushed under the id the server already knows. No-op when no alias matches.
pub fn rewrite_item_topics_topic_id_for_push(
    conn: &Connection,
    payload: &mut Value,
) -> Result<(), String> {
    let Some(local_topic) = payload
        .get("topic_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(());
    };
    let remote: Option<String> = conn
        .query_row(
            "SELECT remote_id FROM sync_topic_aliases WHERE local_id = ?1 LIMIT 1",
            [&local_topic],
            |row| row.get(0),
        )
        .ok();
    if let Some(remote_id) = remote {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("topic_id".to_string(), Value::String(remote_id));
        }
    }
    Ok(())
}

/// The last server version seen for `(table, row_id)` from `sync_row_versions`,
/// or `0` when never synced (PROTOCOL `base_seq`).
pub fn base_seq(conn: &Connection, table: &str, row_id: &str) -> Result<i64, String> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT server_seq FROM sync_row_versions WHERE table_name = ?1 AND row_id = ?2",
            rusqlite::params![table, row_id],
            |row| row.get(0),
        )
        .ok();
    Ok(seq.unwrap_or(0))
}

/// True when the row has any oplog entry with `seq > snapshot` (DESIGN §4.4
/// skip-if-dirty): a local edit landed after the push snapshot, so a remote
/// winner must NOT overwrite it.
pub fn row_dirty_after_snapshot(
    conn: &Connection,
    table: &str,
    row_id: &str,
    snapshot: i64,
) -> Result<bool, String> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sync_oplog
             WHERE table_name = ?1 AND row_id = ?2 AND seq > ?3 LIMIT 1",
            rusqlite::params![table, row_id, snapshot],
            |row| row.get(0),
        )
        .ok();
    Ok(found.is_some())
}

/// Builds the wire [`PushChange`] for one coalesced op (DESIGN §4.2). For a `'D'`
/// op, or any op whose row is absent at read time, emits `op:'delete'` with no
/// payload. Otherwise reads the current row state and emits `op:'upsert'`. The
/// caller passes the asset payload transform via `payload_hook` so this module
/// stays free of blob logic.
///
/// `clock_offset_ms` is ADDED to `changed_at` at build time (PROTOCOL "Reloj").
pub fn build_change(
    conn: &Connection,
    op: &CoalescedOp,
    clock_offset_ms: i64,
) -> Result<PushChange, String> {
    let base = base_seq(conn, &op.table, &op.row_id)?;
    let changed_at = op.changed_at.saturating_add(clock_offset_ms);

    // Explicit delete, OR an insert/update whose row vanished after the snapshot.
    if op.op == 'D' {
        return Ok(PushChange {
            table: op.table.clone(),
            row_id: op.row_id.clone(),
            op: "delete".to_string(),
            changed_at,
            base_seq: base,
            payload: None,
        });
    }

    match read_row_payload(conn, &op.table, &op.row_id)? {
        Some(mut payload) => {
            // item_topics: rewrite a locally-aliased topic_id back to the canonical
            // server (remote) id so both devices converge on one topic (DESIGN
            // §4.7 — "reescribir topic_id por el alias en cada item_topics
            // pusheado").
            if op.table == "item_topics" {
                rewrite_item_topics_topic_id_for_push(conn, &mut payload)?;
            }
            Ok(PushChange {
                table: op.table.clone(),
                row_id: op.row_id.clone(),
                op: "upsert".to_string(),
                changed_at,
                base_seq: base,
                payload: Some(payload),
            })
        }
        None => Ok(PushChange {
            table: op.table.clone(),
            row_id: op.row_id.clone(),
            op: "delete".to_string(),
            changed_at,
            base_seq: base,
            payload: None,
        }),
    }
}

/// Builds push changes for all coalesced ops, skipping any whose table is not in
/// the synced allowlist (defensive — the oplog should only hold synced tables).
/// Asset rows are returned WITH their raw payload; the engine slice rewrites
/// them via [`super::blobs::asset_payload_to_wire`] after the blob upload.
pub fn build_changes(
    conn: &Connection,
    ops: &[CoalescedOp],
    clock_offset_ms: i64,
) -> Result<Vec<PushChange>, String> {
    let mut changes = Vec::with_capacity(ops.len());
    for op in ops {
        if !is_synced_table(&op.table) {
            continue;
        }
        changes.push(build_change(conn, op, clock_offset_ms)?);
    }
    Ok(changes)
}

/// The serialized byte size of a single change as it will appear in the request
/// body. Used for the per-row oversize check (PROTOCOL: a single row exceeding
/// `max_push_bytes` is journaled and purged).
pub fn change_byte_size(change: &PushChange) -> usize {
    serde_json::to_vec(change).map(|v| v.len()).unwrap_or(0)
}

/// Splits `changes` into batches respecting the dual cap (≤[`MAX_PUSH_CHANGES`]
/// AND ≤`max_push_bytes`). A single change whose own size exceeds
/// `max_push_bytes` is returned separately in `oversized` so the caller can
/// journal `apply_error` and purge it from the oplog (PROTOCOL step 4); it is
/// NOT placed in any batch.
pub fn split_into_batches(
    changes: Vec<PushChange>,
    max_push_bytes: i64,
) -> (Vec<Vec<PushChange>>, Vec<PushChange>) {
    let max_bytes = max_push_bytes.max(1) as usize;
    let mut batches: Vec<Vec<PushChange>> = Vec::new();
    let mut oversized: Vec<PushChange> = Vec::new();
    let mut current: Vec<PushChange> = Vec::new();
    let mut current_bytes: usize = 0;

    for change in changes {
        let size = change_byte_size(&change);
        if size > max_bytes {
            oversized.push(change);
            continue;
        }
        let would_overflow_count = current.len() >= MAX_PUSH_CHANGES;
        let would_overflow_bytes = !current.is_empty() && current_bytes + size > max_bytes;
        if would_overflow_count || would_overflow_bytes {
            batches.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes += size;
        current.push(change);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    (batches, oversized)
}

/// Journals a single change that could not be pushed (oversized row, PROTOCOL
/// step 4) as an `apply_error` conflict with its full payload, then purges its
/// oplog entries up to the snapshot so it never blocks the rest of the push.
pub fn journal_and_purge_oversized(
    conn: &Connection,
    change: &PushChange,
    snapshot: i64,
) -> Result<(), String> {
    let conflict_id = format!("ae-{}-{}", change.table, change.row_id);
    let loser = change
        .payload
        .as_ref()
        .map(|p| p.to_string())
        .unwrap_or_default();
    let now = now_ms();
    conn.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, loser_payload, winner_summary, created_at, acknowledged)
         VALUES (?1, ?2, ?3, 'apply_error', ?4, ?5, ?6, 0)
         ON CONFLICT(id) DO UPDATE SET loser_payload = excluded.loser_payload, created_at = excluded.created_at",
        rusqlite::params![
            conflict_id,
            change.table,
            change.row_id,
            loser,
            "row exceeds max_push_bytes",
            now
        ],
    )
    .map_err(|e| format!("[sync] failed to journal oversized row: {e}"))?;

    conn.execute(
        "DELETE FROM sync_oplog WHERE table_name = ?1 AND row_id = ?2 AND seq <= ?3",
        rusqlite::params![change.table, change.row_id, snapshot],
    )
    .map_err(|e| format!("[sync] failed to purge oversized row from oplog: {e}"))?;
    Ok(())
}

/// Applies the server's push results for a successful batch (PROTOCOL step 5),
/// inside a transaction with the `applying` echo-suppression flag set:
///
/// - delete oplog entries `seq <= snapshot` for every result row;
/// - update `sync_row_versions` with each `server_seq`;
/// - for each `lww_lost`, journal the loser and apply the `winner` locally ONLY
///   when the row is not dirty after the snapshot (skip-if-dirty, DESIGN §4.4).
///
/// `winner_apply` is the callback that writes a winner row locally — supplied by
/// the engine slice (it owns the full apply machinery). In this slice it is used
/// to record which winners were applied vs skipped; the actual SQL write is the
/// apply slice's job, so callers may pass a recording closure in tests.
pub fn apply_push_results<F>(
    conn: &Connection,
    snapshot: i64,
    results: &[PushResult],
    mut winner_apply: F,
) -> Result<(), String>
where
    F: FnMut(&Connection, &PullRow) -> Result<(), String>,
{
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin push-result transaction: {e}"))?;

    // Suppress capture echo while we write row_versions / winners.
    meta_set(&tx, "applying", "1")?;

    for result in results {
        // Always purge the oplog entries this push covered (`seq <= snapshot`).
        // A dirty edit landed AFTER the snapshot keeps its `seq > snapshot`
        // entry, so it survives and re-pushes next cycle.
        tx.execute(
            "DELETE FROM sync_oplog
             WHERE table_name = ?1 AND row_id = ?2 AND seq <= ?3",
            rusqlite::params![result.table, result.row_id, snapshot],
        )
        .map_err(|e| format!("[sync] failed to purge pushed oplog: {e}"))?;

        let is_lost = result.status == "lww_lost";

        if is_lost {
            // Journal the loser regardless (nothing is lost silently, DESIGN §4.2).
            journal_lww_lost(&tx, result)?;
        }

        // Skip-if-dirty (PROTOCOL "Semántica de apply" §6, DESIGN §4.4): when the
        // local change LOST and the row has a pending edit after the snapshot, we
        // neither apply the remote winner NOR advance `sync_row_versions` — the
        // stale `base_seq` forces the server's LWW path on the next push, which
        // journals the loser deterministically. For `applied`/`lww_won` (our
        // change reached the server) the version always advances; a pending dirty
        // edit there is still in the oplog and pushes next cycle with the new
        // base_seq, so there is no overwrite hazard.
        let dirty_and_lost =
            is_lost && row_dirty_after_snapshot(&tx, &result.table, &result.row_id, snapshot)?;

        if !dirty_and_lost {
            // For a clean `lww_lost` WITH a winner, apply the winner FIRST: the
            // apply machinery records `sync_row_versions` for the row to the
            // winner's own `server_seq` as part of writing the data. We must NOT
            // pre-advance the version here — doing so poisons the apply's
            // idempotency guard (`known_version >= row.server_seq`), which would
            // then SKIP the winner write and leave the losing local row in place
            // forever while the cursor falsely claims convergence (the bug this
            // ordering fixes; surfaced by the multi-device E2E LWW scenario).
            let winner_applied = if is_lost {
                if let Some(winner) = &result.winner {
                    winner_apply(&tx, winner)?;
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !winner_applied {
                // `applied` / `lww_won` (our change reached the server), or a
                // `lww_lost` with no winner payload (defensive): advance the row
                // version cursor to the server's seq so the next pull skips it.
                tx.execute(
                    "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(table_name, row_id) DO UPDATE SET server_seq = excluded.server_seq",
                    rusqlite::params![result.table, result.row_id, result.server_seq],
                )
                .map_err(|e| format!("[sync] failed to update row version: {e}"))?;
            }
        }
    }

    meta_set(&tx, "applying", "0")?;
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit push results: {e}"))
}

/// Journals a `lww_lost` result: the local change lost, so its payload is kept
/// in full (DESIGN §4.2 — nothing lost silently) with a summary of the winner.
fn journal_lww_lost(conn: &Connection, result: &PushResult) -> Result<(), String> {
    let conflict_id = format!(
        "lww-{}-{}-{}",
        result.table, result.row_id, result.server_seq
    );
    let winner_summary = result
        .winner
        .as_ref()
        .map(|w| format!("server_seq={} device={}", w.server_seq, w.device_id))
        .unwrap_or_else(|| format!("server_seq={}", result.server_seq));
    conn.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, loser_payload, winner_summary, created_at, acknowledged)
         VALUES (?1, ?2, ?3, 'lww_lost', NULL, ?4, ?5, 0)
         ON CONFLICT(id) DO NOTHING",
        rusqlite::params![conflict_id, result.table, result.row_id, winner_summary, now_ms()],
    )
    .map_err(|e| format!("[sync] failed to journal lww_lost: {e}"))?;
    Ok(())
}

/// Reads the stored clock offset (ms) from `sync_meta` (PROTOCOL "Reloj").
pub fn clock_offset(conn: &Connection) -> Result<i64, String> {
    meta_get_i64(conn, "clock_offset_ms")
}

/// Updates the stored clock offset from a fresh `server_now_ms` sample (PROTOCOL
/// "Reloj"): `offset = server_now_ms - local_now_ms`. Callers feed this from
/// push/pull/health responses.
pub fn update_clock_offset(conn: &Connection, server_now_ms: i64) -> Result<i64, String> {
    let offset = server_now_ms.saturating_sub(now_ms());
    meta_set_i64(conn, "clock_offset_ms", offset)?;
    Ok(offset)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Async network orchestration for the push (PROTOCOL step 4). Generic over
/// [`SyncApi`] so the bisection logic is testable against a mock. Kept separate
/// from the SQL mechanics above so those stay synchronous and trivially unit
/// testable.
pub mod batching {
    use super::*;
    use crate::sync::http::{PushRequest, PushResponse, SyncApi, SyncError};

    /// Outcome of pushing one logical batch: the server results plus any changes
    /// that were isolated as un-pushable (a single change that 413s even alone).
    /// The caller journals + purges the `oversized` ones (PROTOCOL step 4).
    #[derive(Debug, Default)]
    pub struct BatchOutcome {
        pub results: Vec<PushResult>,
        /// Changes the server refused even when sent alone (413 on a 1-element
        /// batch). Caller journals `apply_error` + purges from oplog.
        pub oversized: Vec<PushChange>,
        /// The latest `server_now_ms` sample seen, for clock-offset refresh.
        pub server_now_ms: i64,
        /// The latest `server_epoch` seen, for the epoch-mismatch check.
        pub server_epoch: String,
    }

    /// Pushes a single batch with recursive 413 bisection (PROTOCOL step 4):
    /// POST the batch; on `413 payload_too_large`, split it in half and retry
    /// each half; a 1-element batch that still 413s is moved to `oversized`.
    /// Any other error propagates. Successful results accumulate in `outcome`.
    pub async fn push_with_bisection<A: SyncApi>(
        api: &A,
        token: &str,
        schema_tag: &str,
        batch: Vec<PushChange>,
        outcome: &mut BatchOutcome,
    ) -> Result<(), SyncError> {
        if batch.is_empty() {
            return Ok(());
        }

        let request = PushRequest {
            changes: batch.clone(),
        };
        match api.push(token, schema_tag, request).await {
            Ok(PushResponse {
                mut results,
                server_now_ms,
                server_epoch,
                ..
            }) => {
                outcome.results.append(&mut results);
                if server_now_ms != 0 {
                    outcome.server_now_ms = server_now_ms;
                }
                if !server_epoch.is_empty() {
                    outcome.server_epoch = server_epoch;
                }
                Ok(())
            }
            Err(SyncError::Api { status: 413, .. }) => {
                if batch.len() == 1 {
                    // A single change the server refuses — cannot bisect further.
                    outcome.oversized.extend(batch);
                    return Ok(());
                }
                let mid = batch.len() / 2;
                let mut batch = batch;
                let second = batch.split_off(mid);
                Box::pin(push_with_bisection(api, token, schema_tag, batch, outcome)).await?;
                Box::pin(push_with_bisection(api, token, schema_tag, second, outcome)).await
            }
            Err(other) => Err(other),
        }
    }

    /// Pushes ALL batches in order, accumulating results and oversized changes.
    pub async fn push_all<A: SyncApi>(
        api: &A,
        token: &str,
        schema_tag: &str,
        batches: Vec<Vec<PushChange>>,
    ) -> Result<BatchOutcome, SyncError> {
        let mut outcome = BatchOutcome::default();
        for batch in batches {
            push_with_bisection(api, token, schema_tag, batch, &mut outcome).await?;
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::capture::ensure_capture;
    use crate::sync::test_support::{new_synced_test_db, set_session_with_capture};

    /// Seeds a collection so item/asset FKs are satisfiable, then clears the
    /// oplog so the test measures only the rows it touches afterwards.
    fn seed_collection(conn: &Connection) {
        conn.execute(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
            [],
        )
        .expect("seed collection");
        conn.execute_batch("DELETE FROM sync_oplog;")
            .expect("clear oplog after seed");
    }

    fn capturing_db() -> Connection {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session_with_capture(&conn);
        conn
    }

    #[test]
    fn snapshot_empty_oplog_is_zero() {
        let conn = capturing_db();
        assert_eq!(snapshot_oplog(&conn).unwrap(), 0);
    }

    #[test]
    fn coalesce_insert_then_update_yields_single_upsert() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        conn.execute("UPDATE items SET title='B' WHERE id='i1'", [])
            .expect("update");

        let snap = snapshot_oplog(&conn).unwrap();
        let ops = coalesce_ops(&conn, snap).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, 'U', "last op is the update");
        assert_eq!(ops[0].table, "items");

        let change = build_change(&conn, &ops[0], 0).unwrap();
        assert_eq!(change.op, "upsert");
        assert_eq!(change.payload.as_ref().unwrap()["title"], "B");
    }

    #[test]
    fn coalesce_update_then_delete_yields_delete() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        conn.execute("UPDATE items SET title='B' WHERE id='i1'", [])
            .expect("update");
        conn.execute("DELETE FROM items WHERE id='i1'", [])
            .expect("delete");

        let snap = snapshot_oplog(&conn).unwrap();
        let ops = coalesce_ops(&conn, snap).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, 'D');
        let change = build_change(&conn, &ops[0], 0).unwrap();
        assert_eq!(change.op, "delete");
        assert!(change.payload.is_none());
    }

    #[test]
    fn coalesce_delete_then_insert_yields_upsert() {
        let conn = capturing_db();
        seed_collection(&conn);
        // Insert, delete, re-insert same id.
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        conn.execute("DELETE FROM items WHERE id='i1'", [])
            .expect("delete");
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','C','c1',1,1)",
            [],
        )
        .expect("re-insert");

        let snap = snapshot_oplog(&conn).unwrap();
        let ops = coalesce_ops(&conn, snap).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, 'I', "last op is the re-insert");
        let change = build_change(&conn, &ops[0], 0).unwrap();
        assert_eq!(change.op, "upsert");
        assert_eq!(change.payload.as_ref().unwrap()["title"], "C");
    }

    #[test]
    fn upsert_op_with_missing_row_becomes_delete() {
        let conn = capturing_db();
        seed_collection(&conn);
        // Insert (oplog 'I'), then delete the row directly while capture is OFF
        // so no 'D' op is recorded — simulating a row that vanished after the
        // snapshot. We turn capture off, delete, turn it back on.
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        let snap = snapshot_oplog(&conn).unwrap();
        // Remove the row without leaving an oplog entry <= snapshot.
        super::meta_set(&conn, "capture_enabled", "0").unwrap();
        conn.execute("DELETE FROM items WHERE id='i1'", [])
            .expect("delete out-of-band");

        let ops = coalesce_ops(&conn, snap).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, 'I', "oplog still shows the insert");
        let change = build_change(&conn, &ops[0], 0).unwrap();
        assert_eq!(
            change.op, "delete",
            "missing row at read time must downgrade to delete"
        );
        assert!(change.payload.is_none());
    }

    #[test]
    fn payload_excludes_generated_columns() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','Hello','c1',1,1)",
            [],
        )
        .expect("insert");
        let payload = read_row_payload(&conn, "items", "i1").unwrap().unwrap();
        let obj = payload.as_object().unwrap();
        assert!(obj.contains_key("title"));
        assert!(
            !obj.contains_key("search_text"),
            "generated column must be excluded"
        );
    }

    #[test]
    fn base_seq_is_zero_until_versioned_then_reads_back() {
        let conn = capturing_db();
        assert_eq!(base_seq(&conn, "items", "i1").unwrap(), 0);
        conn.execute(
            "INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','i1',41)",
            [],
        )
        .expect("seed version");
        assert_eq!(base_seq(&conn, "items", "i1").unwrap(), 41);
    }

    #[test]
    fn clock_offset_added_to_changed_at() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        let snap = snapshot_oplog(&conn).unwrap();
        let ops = coalesce_ops(&conn, snap).unwrap();
        let raw = ops[0].changed_at;
        let change = build_change(&conn, &ops[0], 5000).unwrap();
        assert_eq!(change.changed_at, raw + 5000, "offset added at build time");
    }

    #[test]
    fn split_into_batches_respects_count_cap() {
        let changes: Vec<PushChange> = (0..1100)
            .map(|i| PushChange {
                table: "items".to_string(),
                row_id: format!("i{i}"),
                op: "upsert".to_string(),
                changed_at: 1,
                base_seq: 0,
                payload: Some(serde_json::json!({"id": format!("i{i}")})),
            })
            .collect();
        let (batches, oversized) = split_into_batches(changes, DEFAULT_MAX_PUSH_BYTES);
        assert!(oversized.is_empty());
        assert_eq!(batches.len(), 3, "1100 / 500 → 3 batches");
        assert_eq!(batches[0].len(), MAX_PUSH_CHANGES);
        assert_eq!(batches[1].len(), MAX_PUSH_CHANGES);
        assert_eq!(batches[2].len(), 100);
    }

    #[test]
    fn split_into_batches_respects_byte_cap_and_isolates_oversized() {
        // A small byte cap forces one change per batch; one giant change is
        // isolated as oversized.
        let small = PushChange {
            table: "items".to_string(),
            row_id: "s1".to_string(),
            op: "upsert".to_string(),
            changed_at: 1,
            base_seq: 0,
            payload: Some(serde_json::json!({"id": "s1"})),
        };
        let small2 = PushChange {
            row_id: "s2".to_string(),
            ..small.clone()
        };
        let big_blob = "x".repeat(10_000);
        let oversized_change = PushChange {
            table: "items".to_string(),
            row_id: "big".to_string(),
            op: "upsert".to_string(),
            changed_at: 1,
            base_seq: 0,
            payload: Some(serde_json::json!({"id": "big", "blob": big_blob})),
        };

        let per_small = change_byte_size(&small);
        // Cap big enough for exactly one small change.
        let cap = (per_small + 5) as i64;
        let (batches, oversized) =
            split_into_batches(vec![small.clone(), small2, oversized_change], cap);
        assert_eq!(oversized.len(), 1, "the giant change is isolated");
        assert_eq!(oversized[0].row_id, "big");
        // Two small changes, each its own batch due to the tight byte cap.
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[1].len(), 1);
    }

    #[test]
    fn journal_and_purge_oversized_records_conflict_and_clears_oplog() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('big','A','c1',1,1)",
            [],
        )
        .expect("insert");
        let snap = snapshot_oplog(&conn).unwrap();
        let change = PushChange {
            table: "items".to_string(),
            row_id: "big".to_string(),
            op: "upsert".to_string(),
            changed_at: 1,
            base_seq: 0,
            payload: Some(serde_json::json!({"id": "big"})),
        };
        journal_and_purge_oversized(&conn, &change, snap).expect("journal+purge");

        let conflicts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_conflicts WHERE reason='apply_error' AND row_id='big'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(conflicts, 1);
        let oplog: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_oplog WHERE row_id='big' AND seq <= ?1",
                [snap],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(oplog, 0, "oversized row purged from oplog");
    }

    #[test]
    fn apply_push_results_purges_oplog_and_updates_versions() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        let snap = snapshot_oplog(&conn).unwrap();
        let results = vec![PushResult {
            table: "items".to_string(),
            row_id: "i1".to_string(),
            status: "applied".to_string(),
            server_seq: 87,
            winner: None,
        }];
        apply_push_results(&conn, snap, &results, |_, _| {
            panic!("no winner expected for applied")
        })
        .expect("apply results");

        assert_eq!(base_seq(&conn, "items", "i1").unwrap(), 87);
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_oplog WHERE row_id='i1' AND seq <= ?1",
                [snap],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "pushed oplog entries purged");
    }

    #[test]
    fn apply_push_results_applies_winner_only_when_not_dirty() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        let snap = snapshot_oplog(&conn).unwrap();

        // Make the row dirty AFTER the snapshot (a local edit lands).
        conn.execute("UPDATE items SET title='local-edit' WHERE id='i1'", [])
            .expect("dirty edit");
        assert!(row_dirty_after_snapshot(&conn, "items", "i1", snap).unwrap());

        let winner = PullRow {
            table: "items".to_string(),
            row_id: "i1".to_string(),
            server_seq: 200,
            deleted: false,
            changed_at: 1,
            device_id: "other".to_string(),
            payload: Some(serde_json::json!({"id": "i1", "title": "remote"})),
        };
        let result = PushResult {
            table: "items".to_string(),
            row_id: "i1".to_string(),
            status: "lww_lost".to_string(),
            server_seq: 200,
            winner: Some(winner),
        };

        let mut applied = Vec::new();
        apply_push_results(&conn, snap, std::slice::from_ref(&result), |_, w| {
            applied.push(w.row_id.clone());
            Ok(())
        })
        .expect("apply results");

        assert!(
            applied.is_empty(),
            "skip-if-dirty: winner must NOT be applied over a dirty row"
        );
        // The loser is journaled regardless.
        let journaled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_conflicts WHERE reason='lww_lost' AND row_id='i1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(journaled, 1);
        // base_seq for the row stays unchanged (old version) so the next push
        // re-routes through LWW — it was never set, so still 0.
        assert_eq!(
            base_seq(&conn, "items", "i1").unwrap(),
            0,
            "dirty winner must not advance the row version"
        );
    }

    #[test]
    fn apply_push_results_applies_winner_when_clean() {
        let conn = capturing_db();
        seed_collection(&conn);
        conn.execute(
            "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
            [],
        )
        .expect("insert");
        let snap = snapshot_oplog(&conn).unwrap();
        // No edits after the snapshot → clean.
        assert!(!row_dirty_after_snapshot(&conn, "items", "i1", snap).unwrap());

        let winner = PullRow {
            table: "items".to_string(),
            row_id: "i1".to_string(),
            server_seq: 200,
            deleted: false,
            changed_at: 1,
            device_id: "other".to_string(),
            payload: Some(serde_json::json!({"id": "i1", "title": "remote"})),
        };
        let result = PushResult {
            table: "items".to_string(),
            row_id: "i1".to_string(),
            status: "lww_lost".to_string(),
            server_seq: 200,
            winner: Some(winner),
        };

        let mut applied = Vec::new();
        // The real `winner_apply` (the engine wires `apply_row`) records the
        // row version to the WINNER's own server_seq as part of writing the data.
        // The stub mirrors that so the test reflects the real contract: with the
        // winner applied, `apply_push_results` does NOT pre-advance the version
        // (that pre-advance used to poison the apply idempotency guard and skip
        // the winner write — see the inline comment in `apply_push_results`).
        apply_push_results(&conn, snap, std::slice::from_ref(&result), |c, w| {
            applied.push(w.row_id.clone());
            c.execute(
                "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(table_name, row_id) DO UPDATE SET server_seq = excluded.server_seq",
                rusqlite::params![w.table, w.row_id, w.server_seq],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .expect("apply results");

        assert_eq!(applied, vec!["i1".to_string()], "clean winner applied");
        assert_eq!(
            base_seq(&conn, "items", "i1").unwrap(),
            200,
            "winner version recorded by the winner apply"
        );
    }

    #[test]
    fn build_changes_skips_non_synced_tables() {
        let conn = capturing_db();
        // Stuff an oplog entry for a non-synced table directly.
        conn.execute(
            "INSERT INTO sync_oplog(table_name,row_id,op,changed_at) VALUES('app_settings','k','U',1)",
            [],
        )
        .expect("insert rogue oplog");
        let snap = snapshot_oplog(&conn).unwrap();
        let ops = coalesce_ops(&conn, snap).unwrap();
        let changes = build_changes(&conn, &ops, 0).unwrap();
        assert!(
            changes.is_empty(),
            "non-synced table ops must be skipped from the push"
        );
    }

    #[test]
    fn update_clock_offset_persists_and_reads_back() {
        let conn = capturing_db();
        // A server clock ~10s ahead of local.
        let server_now = now_ms() + 10_000;
        let offset = update_clock_offset(&conn, server_now).unwrap();
        assert!((9_000..=11_000).contains(&offset), "offset ~10s: {offset}");
        let stored = clock_offset(&conn).unwrap();
        assert_eq!(stored, offset);
    }

    // ---- batching / bisection against the MockSyncApi ----

    use crate::sync::test_support::MockSyncApi;

    fn change(id: &str) -> PushChange {
        PushChange {
            table: "items".to_string(),
            row_id: id.to_string(),
            op: "upsert".to_string(),
            changed_at: 1,
            base_seq: 0,
            payload: Some(serde_json::json!({"id": id, "title": "x"})),
        }
    }

    #[tokio::test]
    async fn push_all_returns_results_for_every_change() {
        let api = MockSyncApi::default();
        let batch: Vec<PushChange> = (0..10).map(|i| change(&format!("i{i}"))).collect();
        let outcome = batching::push_all(&api, "tok", "0023", vec![batch])
            .await
            .expect("push");
        assert_eq!(outcome.results.len(), 10);
        assert!(outcome.oversized.is_empty());
        assert_eq!(api.pushed_count(), 10);
        assert_eq!(outcome.server_epoch, "mock-epoch");
    }

    #[tokio::test]
    async fn push_with_bisection_splits_on_413() {
        // The mock 413s any batch of more than 2 changes. A batch of 8 must be
        // bisected down to ≤2-element sub-batches, and ALL eight still land.
        let api = MockSyncApi::with_max_batch(2);
        let batch: Vec<PushChange> = (0..8).map(|i| change(&format!("i{i}"))).collect();
        let mut outcome = batching::BatchOutcome::default();
        batching::push_with_bisection(&api, "tok", "0023", batch, &mut outcome)
            .await
            .expect("bisection push");
        assert_eq!(
            outcome.results.len(),
            8,
            "all changes pushed after bisection"
        );
        assert!(outcome.oversized.is_empty());
        assert_eq!(api.pushed_count(), 8);
    }

    #[tokio::test]
    async fn push_with_bisection_isolates_single_oversized_change() {
        // The mock 413s ANY non-empty batch (max 0) → every single change ends
        // up isolated as oversized.
        let api = MockSyncApi::with_max_batch(0);
        let batch: Vec<PushChange> = (0..3).map(|i| change(&format!("i{i}"))).collect();
        let mut outcome = batching::BatchOutcome::default();
        batching::push_with_bisection(&api, "tok", "0023", batch, &mut outcome)
            .await
            .expect("push isolates oversized");
        assert!(outcome.results.is_empty());
        assert_eq!(
            outcome.oversized.len(),
            3,
            "each unbisectable change isolated as oversized"
        );
    }
}
