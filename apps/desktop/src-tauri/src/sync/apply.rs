//! Pull-apply semantics (PROTOCOL "Semántica de apply" — NORMATIVE, DESIGN
//! §4.3–§4.7). Reconciles untrusted pulled rows against the LOCAL schema before
//! they touch SQL, applies a whole page in ONE transaction with deferred FK +
//! per-row SAVEPOINT, parks FK violators, and persists cursor / row-versions /
//! blob+FTS queues atomically with the page.
//!
//! Hard invariants enforced here:
//! - `INSERT ... ON CONFLICT(id) DO UPDATE` only — `INSERT OR REPLACE` is
//!   PROHIBITED (it reassigns `items.rowid`, breaking the contentless FTS5
//!   contract, and fires `ON DELETE CASCADE` that destroys local children).
//! - Every value is bound as a parameter; only validated identifiers are ever
//!   interpolated (`is_safe_identifier` + `quote_identifier`).
//! - Skip-if-dirty: a row with pending oplog entries is never overwritten, and a
//!   tombstone whose cascade-reachable children are dirty is fully deferred.
//! - `assets.rel_path` is UNTRUSTED: rejected for absolute/UNC/drive/`..`/empty,
//!   required `assets/` prefix, then `ensure_within_dir` before the local path is
//!   rewritten into the row and the blob is enqueued.
//!
//! Most of this surface is driven by the engine slice (single-flight tokio task,
//! next slice); here it is exercised by the unit tests. The module-level
//! `allow(dead_code)` matches the C1/C2 convention for forward-looking API and is
//! removed once the engine wires it up.
#![allow(dead_code)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::db::util::{is_safe_identifier, json_to_sql_param, quote_identifier};
use crate::sync::capture::{is_synced_table, SYNCED_TABLES_FK_ORDER};
use crate::sync::cascade::direct_cascade_edges;
use crate::sync::http::PullRow;
use crate::sync::session::{meta_get, meta_set, meta_set_i64};

/// Tables whose conflict target is `(asset_id)` rather than `(id)`, with
/// `id = excluded.id` folded into the UPDATE so stray IDs converge (DESIGN §4.6,
/// PROTOCOL "Semántica de apply" step 3).
const ASSET_KEYED_TABLES: &[&str] = &["extractions", "transcriptions", "layouts"];

/// Outcome of applying a single row within a page (before commit).
#[derive(Debug, PartialEq, Eq)]
pub enum RowOutcome {
    /// The row was applied (upsert or delete) and its version recorded.
    Applied,
    /// The row was skipped because it is locally dirty (skip-if-dirty) or already
    /// at/above this version. No version change.
    Skipped,
    /// The row was journaled `apply_error` (bad envelope, rel_path attack, …) and
    /// the cursor advances past it.
    Journaled,
}

/// Context shared across a page apply: identifies the app-data dir (for asset
/// rel_path validation) and accumulates the `(table, column)` schema-drift keys
/// already journaled this cycle so each is recorded only once.
pub struct ApplyContext<'a> {
    pub app_data_dir: &'a Path,
    /// `(table, column)` pairs already journaled `schema_drift` this run.
    pub drift_seen: HashSet<(String, String)>,
}

impl<'a> ApplyContext<'a> {
    pub fn new(app_data_dir: &'a Path) -> Self {
        Self {
            app_data_dir,
            drift_seen: HashSet::new(),
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Envelope + column reconciliation (PROTOCOL "Semántica de apply" steps 1, 2, 4)
// ---------------------------------------------------------------------------

/// Why an envelope is rejected outright (journaled `apply_error`, never applied).
#[derive(Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    /// `table` not in the synced allowlist or not a safe identifier.
    BadTable,
    /// `payload.id` is missing or `!= row_id`.
    IdMismatch,
    /// Upsert with a null/absent payload (a conforming server never sends this).
    MissingPayload,
}

/// Validates the wire envelope identity (PROTOCOL step 1): table is allowlisted
/// and safe, and (for upserts) `payload.id == row_id`. Returns the payload object
/// on success.
pub fn validate_upsert_envelope<'p>(
    table: &str,
    row_id: &str,
    payload: &'p Value,
) -> Result<&'p Map<String, Value>, EnvelopeError> {
    if !is_synced_table(table) || !is_safe_identifier(table) {
        return Err(EnvelopeError::BadTable);
    }
    let obj = payload.as_object().ok_or(EnvelopeError::MissingPayload)?;
    match obj.get("id").and_then(Value::as_str) {
        Some(id) if id == row_id => Ok(obj),
        _ => Err(EnvelopeError::IdMismatch),
    }
}

/// The non-generated local columns of `table` (`PRAGMA table_xinfo`, `hidden=0`).
/// Reuses the push-side reader so both directions see an identical column set.
fn local_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    crate::sync::push::non_generated_columns(conn, table)
}

// ---------------------------------------------------------------------------
// Skip-if-dirty (PROTOCOL step 6, DESIGN §4.4)
// ---------------------------------------------------------------------------

/// True when `(table, row_id)` has ANY pending oplog entry — a local edit not
/// yet pushed. Such a row is never overwritten by a remote apply.
pub fn row_has_pending_oplog(conn: &Connection, table: &str, row_id: &str) -> Result<bool, String> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sync_oplog WHERE table_name = ?1 AND row_id = ?2 LIMIT 1",
            rusqlite::params![table, row_id],
            |row| row.get(0),
        )
        .ok();
    Ok(found.is_some())
}

/// True when applying a tombstone for `(table, row_id)` would destroy a locally
/// dirty cascade-reachable child (DESIGN §4.4). Walks the static cascade graph
/// (depth ≤ 2 for the synced schema): a dirty direct child, or a dirty
/// grandchild reachable through an existing intermediate child row, defers the
/// whole tombstone.
pub fn tombstone_has_dirty_cascade_child(
    conn: &Connection,
    table: &str,
    row_id: &str,
) -> Result<bool, String> {
    for (child_table, fk_col) in direct_cascade_edges(table) {
        // Enumerate the local child rows pointing at this parent.
        let child_ids = child_rows_for_parent(conn, child_table, fk_col, row_id)?;
        for child_id in &child_ids {
            if row_has_pending_oplog(conn, child_table, child_id)? {
                return Ok(true);
            }
            // Recurse one more level (e.g. items → assets is RESTRICT so never
            // reached, but assets → extractions etc. terminate at leaves).
            if tombstone_has_dirty_cascade_child(conn, child_table, child_id)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Local child row ids of `child_table` whose `fk_col` equals `parent_id`.
fn child_rows_for_parent(
    conn: &Connection,
    child_table: &str,
    fk_col: &str,
    parent_id: &str,
) -> Result<Vec<String>, String> {
    if !is_safe_identifier(child_table) || !is_safe_identifier(fk_col) {
        return Err(format!(
            "[sync] unsafe cascade identifier {child_table}.{fk_col}"
        ));
    }
    let sql = format!(
        "SELECT id FROM {} WHERE {} = ?1",
        quote_identifier(child_table),
        quote_identifier(fk_col)
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("[sync] failed to prepare cascade child lookup: {e}"))?;
    let ids = stmt
        .query_map([parent_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("[sync] failed to query cascade children: {e}"))?;
    let mut out = Vec::new();
    for id in ids {
        out.push(id.map_err(|e| format!("[sync] failed to read cascade child id: {e}"))?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Row version cursor
// ---------------------------------------------------------------------------

/// The locally-known server version for `(table, row_id)`, or `0` when unseen.
pub fn known_version(conn: &Connection, table: &str, row_id: &str) -> Result<i64, String> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT server_seq FROM sync_row_versions WHERE table_name = ?1 AND row_id = ?2",
            rusqlite::params![table, row_id],
            |row| row.get(0),
        )
        .ok();
    Ok(seq.unwrap_or(0))
}

fn record_version(
    conn: &Connection,
    table: &str,
    row_id: &str,
    server_seq: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq) VALUES (?1, ?2, ?3)
         ON CONFLICT(table_name, row_id) DO UPDATE SET server_seq = excluded.server_seq",
        rusqlite::params![table, row_id, server_seq],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to record row version: {e}"))
}

// ---------------------------------------------------------------------------
// Topic aliasing (DESIGN §4.7)
// ---------------------------------------------------------------------------

/// Resolves a topic alias for `remote_id`: the local topic id it maps to, if any.
pub fn topic_alias(conn: &Connection, remote_id: &str) -> Result<Option<String>, String> {
    let local: Option<String> = conn
        .query_row(
            "SELECT local_id FROM sync_topic_aliases WHERE remote_id = ?1",
            [remote_id],
            |row| row.get(0),
        )
        .ok();
    Ok(local)
}

/// Records a topic alias `remote_id → local_id` and journals `unique_collision`
/// for observability (DESIGN §4.7). Idempotent.
fn record_topic_alias(conn: &Connection, remote_id: &str, local_id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_topic_aliases(remote_id, local_id) VALUES (?1, ?2)
         ON CONFLICT(remote_id) DO UPDATE SET local_id = excluded.local_id",
        rusqlite::params![remote_id, local_id],
    )
    .map_err(|e| format!("[sync] failed to record topic alias: {e}"))?;
    journal_conflict(
        conn,
        "topics",
        remote_id,
        "unique_collision",
        None,
        Some(&format!("aliased to local topic {local_id}")),
    )
}

/// Resolves the effective topic id for a remote topic upsert. When the remote
/// topic's `name` already exists locally under a DIFFERENT id, registers an alias
/// (`remote_id → local_id`) and returns the local id so the remote row is NOT
/// inserted as a duplicate. Otherwise returns `None` (apply the remote row
/// verbatim).
fn resolve_topic_collision(
    conn: &Connection,
    remote_id: &str,
    payload: &Map<String, Value>,
) -> Result<Option<String>, String> {
    let Some(name) = payload.get("name").and_then(Value::as_str) else {
        return Ok(None);
    };
    let local_id: Option<String> = conn
        .query_row("SELECT id FROM topics WHERE name = ?1", [name], |row| {
            row.get(0)
        })
        .ok();
    match local_id {
        Some(local) if local != remote_id => {
            record_topic_alias(conn, remote_id, &local)?;
            Ok(Some(local))
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Schema drift journaling (PROTOCOL step 4)
// ---------------------------------------------------------------------------

/// Journals `schema_drift` for an unknown payload column, once per `(table,
/// column)` per cycle (PROTOCOL step 4).
fn journal_schema_drift(
    conn: &Connection,
    ctx: &mut ApplyContext,
    table: &str,
    column: &str,
    dropped_value: &Value,
) -> Result<(), String> {
    let key = (table.to_string(), column.to_string());
    if ctx.drift_seen.contains(&key) {
        return Ok(());
    }
    ctx.drift_seen.insert(key);
    let conflict_id = format!("drift-{table}-{column}");
    conn.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, loser_payload, winner_summary, created_at, acknowledged)
         VALUES (?1, ?2, ?3, 'schema_drift', ?4, ?5, ?6, 0)
         ON CONFLICT(id) DO NOTHING",
        rusqlite::params![
            conflict_id,
            table,
            column,
            dropped_value.to_string(),
            format!("unknown column '{column}' dropped on apply"),
            now_ms()
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to journal schema_drift: {e}"))
}

/// Journals a conflict row with the given reason (DESIGN §6 schema). `loser` is
/// the full payload (for apply_error/parent_deleted), `winner` a human summary.
pub fn journal_conflict(
    conn: &Connection,
    table: &str,
    row_id: &str,
    reason: &str,
    loser: Option<&str>,
    winner: Option<&str>,
) -> Result<(), String> {
    let conflict_id = format!("{reason}-{table}-{row_id}");
    conn.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, loser_payload, winner_summary, created_at, acknowledged)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
         ON CONFLICT(id) DO UPDATE SET loser_payload = excluded.loser_payload, created_at = excluded.created_at",
        rusqlite::params![conflict_id, table, row_id, reason, loser, winner, now_ms()],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to journal {reason}: {e}"))
}

// ---------------------------------------------------------------------------
// Asset rel_path validation (PROTOCOL "Transformación de assets" — NORMATIVE)
// ---------------------------------------------------------------------------

/// Why an inbound asset `rel_path` is rejected (journaled `apply_error`, the row
/// is NOT written).
#[derive(Debug, PartialEq, Eq)]
pub enum InboundRelPathError {
    Empty,
    NotUnderAssets,
    Absolute,
    DriveOrUnc,
    Traversal,
    OutsideAppData(String),
}

impl std::fmt::Display for InboundRelPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InboundRelPathError::Empty => write!(f, "rel_path is empty"),
            InboundRelPathError::NotUnderAssets => write!(f, "rel_path is not under assets/"),
            InboundRelPathError::Absolute => write!(f, "rel_path is absolute"),
            InboundRelPathError::DriveOrUnc => write!(f, "rel_path has a drive or UNC prefix"),
            InboundRelPathError::Traversal => write!(f, "rel_path contains '..'/'.' segments"),
            InboundRelPathError::OutsideAppData(m) => write!(f, "rel_path escapes app-data: {m}"),
        }
    }
}

/// Validates an UNTRUSTED inbound `rel_path` and resolves it to a local absolute
/// path under `app_data_dir` (PROTOCOL "Transformación de assets"). Performs the
/// string-level rejections (a) BEFORE any join, then `ensure_within_dir` (c)
/// after reconstruction:
///
/// 1. Normalize `\` → `/`; reject empty.
/// 2. Reject absolute (`/`-leading), drive (`C:`), UNC (`//`), and any `..`/`.`
///    component.
/// 3. Require an `assets/` prefix.
/// 4. Join under `app_data_dir` and pass through `ensure_within_dir`.
pub fn validate_inbound_rel_path(
    rel_path: &str,
    app_data_dir: &Path,
) -> Result<PathBuf, InboundRelPathError> {
    let norm = rel_path.trim().replace('\\', "/");
    if norm.is_empty() {
        return Err(InboundRelPathError::Empty);
    }
    // Drive letter (C:) or UNC-ish before any other check.
    if norm.len() >= 2 && norm.as_bytes()[1] == b':' {
        return Err(InboundRelPathError::DriveOrUnc);
    }
    if norm.starts_with("//") {
        return Err(InboundRelPathError::DriveOrUnc);
    }
    if norm.starts_with('/') {
        return Err(InboundRelPathError::Absolute);
    }
    // Reject any traversal/dot component (split on '/').
    for component in norm.split('/') {
        if component == ".." || component == "." {
            return Err(InboundRelPathError::Traversal);
        }
    }
    if !norm.starts_with("assets/") || norm == "assets/" {
        return Err(InboundRelPathError::NotUnderAssets);
    }

    // Reconstruct the local path component-by-component (never feed the raw
    // string to PathBuf join in one go — split keeps separators controlled).
    let mut candidate = app_data_dir.to_path_buf();
    for component in norm.split('/') {
        candidate.push(component);
    }

    // ensure_within_dir canonicalizes; app_data_dir must exist (it always does at
    // runtime). It also refuses any residual `..` in a missing tail.
    crate::path_utils::ensure_within_dir(&candidate, app_data_dir)
        .map(|_| candidate)
        .map_err(InboundRelPathError::OutsideAppData)
}

// ---------------------------------------------------------------------------
// Upsert / delete statement application (PROTOCOL "Semántica de apply" step 3)
// ---------------------------------------------------------------------------

/// Builds the column intersection between the payload and the local non-generated
/// columns, journaling `schema_drift` for unknown payload keys (once per
/// `(table, column)`). Returns the ordered `(column, value)` pairs to bind. `id`
/// is always included when present (it is the conflict pivot / converged key).
fn intersect_columns(
    conn: &Connection,
    ctx: &mut ApplyContext,
    table: &str,
    payload: &Map<String, Value>,
    local_cols: &[String],
) -> Result<Vec<(String, Value)>, String> {
    let local_set: HashSet<&str> = local_cols.iter().map(String::as_str).collect();
    let mut pairs = Vec::new();
    for (key, value) in payload {
        if !is_safe_identifier(key) {
            // A malformed key can never be a real column; treat as drift.
            journal_schema_drift(conn, ctx, table, key, value)?;
            continue;
        }
        if local_set.contains(key.as_str()) {
            pairs.push((key.clone(), value.clone()));
        } else {
            journal_schema_drift(conn, ctx, table, key, value)?;
        }
    }
    Ok(pairs)
}

/// Applies one upsert row inside the page transaction (within its own SAVEPOINT,
/// managed by the caller). Performs the column intersection, builds an
/// `INSERT ... ON CONFLICT DO UPDATE`, and binds all values as params. Returns
/// `Err` only on a SQL failure the caller should map to park/journal.
fn apply_upsert(
    conn: &Connection,
    ctx: &mut ApplyContext,
    table: &str,
    payload: &Map<String, Value>,
) -> Result<(), String> {
    let local_cols = local_columns(conn, table)?;
    let pairs = intersect_columns(conn, ctx, table, payload, &local_cols)?;
    if pairs.is_empty() {
        return Err(format!("[sync] no applicable columns for {table}"));
    }

    let columns: Vec<&str> = pairs.iter().map(|(c, _)| c.as_str()).collect();
    let quoted_cols: Vec<String> = columns.iter().map(|c| quote_identifier(c)).collect();
    let placeholders: Vec<String> = (1..=pairs.len()).map(|i| format!("?{i}")).collect();

    // Conflict target: (asset_id) for the one-per-asset tables (with id=excluded.id
    // to converge stray ids), else (id).
    let asset_keyed = ASSET_KEYED_TABLES.contains(&table);
    let conflict_target = if asset_keyed { "asset_id" } else { "id" };

    // DO UPDATE assignments: every column except the conflict pivot, plus
    // id=excluded.id for the asset-keyed tables (PROTOCOL step 3 / DESIGN §4.6).
    let mut updates: Vec<String> = Vec::new();
    for col in &columns {
        if *col == conflict_target {
            continue;
        }
        updates.push(format!("{c} = excluded.{c}", c = quote_identifier(col)));
    }
    if asset_keyed && columns.contains(&"id") {
        updates.push("\"id\" = excluded.\"id\"".to_string());
    }
    // A pure-pivot row (only the conflict column present) has nothing to update;
    // DO NOTHING keeps it idempotent.
    let on_conflict = if updates.is_empty() {
        format!("ON CONFLICT({conflict_target}) DO NOTHING")
    } else {
        format!(
            "ON CONFLICT({conflict_target}) DO UPDATE SET {}",
            updates.join(", ")
        )
    };

    let sql = format!(
        "INSERT INTO {table} ({cols}) VALUES ({vals}) {on_conflict}",
        table = quote_identifier(table),
        cols = quoted_cols.join(", "),
        vals = placeholders.join(", "),
    );

    let params: Vec<Box<dyn rusqlite::ToSql>> =
        pairs.iter().map(|(_, v)| json_to_sql_param(v)).collect();
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    conn.execute(&sql, param_refs.as_slice())
        .map(|_| ())
        .map_err(|e| format!("[sync] upsert into {table} failed: {e}"))
}

/// Applies a delete (tombstone) for `(table, row_id)`. Uses a plain `DELETE`,
/// which triggers `ON DELETE CASCADE` as intended for the remote intent — the
/// dirty-child guard runs BEFORE this is called.
fn apply_delete(conn: &Connection, table: &str, row_id: &str) -> Result<(), String> {
    let sql = format!("DELETE FROM {} WHERE id = ?1", quote_identifier(table));
    conn.execute(&sql, [row_id])
        .map(|_| ())
        .map_err(|e| format!("[sync] delete from {table} failed: {e}"))
}

// ---------------------------------------------------------------------------
// Asset payload rewrite + blob/FTS enqueue (DESIGN §7, PROTOCOL flow steps 6-8)
// ---------------------------------------------------------------------------

/// Rewrites an inbound `assets` payload for local storage: validates `rel_path`,
/// replaces it with the local absolute `path`, and drops the wire-only
/// `rel_path`/`sha256`/`size` keys that have no local column (size IS a local
/// column, so it is preserved). Returns the resolved `(local_path, sha256, size,
/// rel_path)` so the caller can enqueue the blob. On a rel_path failure returns
/// `Err` (caller journals `apply_error`, skips the row).
fn rewrite_asset_payload(
    payload: &mut Map<String, Value>,
    app_data_dir: &Path,
) -> Result<(PathBuf, String, i64, String), InboundRelPathError> {
    let rel_path = payload
        .get("rel_path")
        .and_then(Value::as_str)
        .ok_or(InboundRelPathError::Empty)?
        .to_string();
    let local_path = validate_inbound_rel_path(&rel_path, app_data_dir)?;

    let sha256 = payload
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let size = payload.get("size").and_then(Value::as_i64).unwrap_or(0);

    // Rewrite the local `path` column from the validated rel_path. `size` stays
    // (it is a real assets column). `rel_path`/`sha256` are wire-only → dropped
    // (they'd be journaled as drift otherwise; drop them silently here since the
    // protocol mandates their presence on the wire).
    payload.insert(
        "path".to_string(),
        Value::String(local_path.to_string_lossy().into_owned()),
    );
    payload.remove("rel_path");
    payload.remove("sha256");

    Ok((local_path, sha256, size, rel_path))
}

/// Enqueues a blob download in `sync_pending_blobs` (idempotent upsert) within
/// the page transaction (DESIGN §7 — the sha256 lives only here, not in
/// `assets`).
fn enqueue_pending_blob(
    conn: &Connection,
    asset_id: &str,
    sha256: &str,
    rel_path: &str,
    size: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_pending_blobs(asset_id, sha256, rel_path, size, retry_count, last_error, last_attempt_at)
         VALUES (?1, ?2, ?3, ?4, 0, NULL, NULL)
         ON CONFLICT(asset_id) DO UPDATE SET
           sha256 = excluded.sha256, rel_path = excluded.rel_path, size = excluded.size",
        rusqlite::params![asset_id, sha256, rel_path, size],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to enqueue pending blob for {asset_id}: {e}"))
}

/// Enqueues an FTS reindex for the item resolved from an applied row (DESIGN
/// §4 / PROTOCOL step 8). Resolution: `items` directly; `extractions`/
/// `transcriptions` via asset→item; `notes`/`assets` via their `item_id`.
/// No-op for tables that do not feed FTS.
fn enqueue_pending_fts(
    conn: &Connection,
    table: &str,
    payload: &Map<String, Value>,
) -> Result<(), String> {
    let item_id: Option<String> = match table {
        "items" => payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string),
        "notes" | "assets" => payload
            .get("item_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        "extractions" | "transcriptions" => {
            let asset_id = payload.get("asset_id").and_then(Value::as_str);
            match asset_id {
                Some(aid) => crate::nlp::lookup_item_id_for_asset(conn, aid)?,
                None => None,
            }
        }
        _ => None,
    };
    if let Some(item_id) = item_id {
        conn.execute(
            "INSERT INTO sync_pending_fts(item_id) VALUES (?1) ON CONFLICT(item_id) DO NOTHING",
            [&item_id],
        )
        .map_err(|e| format!("[sync] failed to enqueue pending fts for {item_id}: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Single-row apply (shared by the page apply and the pending-row retry)
// ---------------------------------------------------------------------------

/// Applies one pulled row (upsert or delete) against the local DB inside the
/// already-open page transaction. Assumes `applying='1'` is already set by the
/// caller. Handles skip-if-dirty, envelope validation, topic aliasing, asset
/// rewrite + blob/FTS enqueue, schema drift, and version recording. SQL failures
/// bubble up as `Err` so the caller can park/journal the row.
///
/// Returns the [`RowOutcome`]; the caller advances the page cursor regardless
/// (PROTOCOL step 5 — the cursor always advances).
pub fn apply_row(
    conn: &Connection,
    ctx: &mut ApplyContext,
    row: &PullRow,
) -> Result<RowOutcome, String> {
    let table = row.table.as_str();
    let row_id = row.row_id.as_str();

    // Already at/above this version → idempotent skip (PROTOCOL pull: skip rows
    // already seen).
    if known_version(conn, table, row_id)? >= row.server_seq && row.server_seq != 0 {
        return Ok(RowOutcome::Skipped);
    }

    if row.deleted {
        // Tombstone: skip if locally dirty OR any cascade child is dirty.
        if row_has_pending_oplog(conn, table, row_id)?
            || tombstone_has_dirty_cascade_child(conn, table, row_id)?
        {
            return Ok(RowOutcome::Skipped);
        }
        apply_delete(conn, table, row_id)?;
        record_version(conn, table, row_id, row.server_seq)?;
        return Ok(RowOutcome::Applied);
    }

    // Upsert path. Validate the envelope first.
    let Some(payload_value) = &row.payload else {
        journal_conflict(
            conn,
            table,
            row_id,
            "apply_error",
            None,
            Some("upsert with null payload"),
        )?;
        return Ok(RowOutcome::Journaled);
    };
    let payload_obj = match validate_upsert_envelope(table, row_id, payload_value) {
        Ok(obj) => obj.clone(),
        Err(_) => {
            journal_conflict(
                conn,
                table,
                row_id,
                "apply_error",
                Some(&payload_value.to_string()),
                Some("invalid envelope (table/id mismatch)"),
            )?;
            return Ok(RowOutcome::Journaled);
        }
    };

    // Skip-if-dirty for upserts.
    if row_has_pending_oplog(conn, table, row_id)? {
        return Ok(RowOutcome::Skipped);
    }

    // Topic collision → alias and do NOT insert the duplicate remote topic.
    if table == "topics" {
        if let Some(_local) = resolve_topic_collision(conn, row_id, &payload_obj)? {
            // The local topic already exists; just record the version so we don't
            // re-process this remote topic, and skip the insert.
            record_version(conn, table, row_id, row.server_seq)?;
            return Ok(RowOutcome::Applied);
        }
    }

    let mut payload_obj = payload_obj;

    // item_topics: rewrite topic_id through any alias (DESIGN §4.7).
    if table == "item_topics" {
        if let Some(remote_topic) = payload_obj.get("topic_id").and_then(Value::as_str) {
            if let Some(local_topic) = topic_alias(conn, remote_topic)? {
                payload_obj.insert("topic_id".to_string(), Value::String(local_topic));
            }
        }
    }

    // assets: validate rel_path, rewrite to local path, enqueue blob + fts.
    let mut blob_enqueue: Option<(String, String, i64)> = None;
    if table == "assets" {
        match rewrite_asset_payload(&mut payload_obj, ctx.app_data_dir) {
            Ok((_local, sha256, size, rel_path)) => {
                blob_enqueue = Some((sha256, rel_path, size));
            }
            Err(err) => {
                journal_conflict(
                    conn,
                    table,
                    row_id,
                    "apply_error",
                    Some(&payload_value.to_string()),
                    Some(&format!("rel_path rejected: {err}")),
                )?;
                return Ok(RowOutcome::Journaled);
            }
        }
    }

    apply_upsert(conn, ctx, table, &payload_obj)?;
    record_version(conn, table, row_id, row.server_seq)?;

    // Post-apply queues (in the SAME transaction — DESIGN §4.3).
    if table == "assets" {
        if let Some((sha256, rel_path, size)) = blob_enqueue {
            // Only enqueue if the local file is missing or the hash is unknown.
            let local_path = ctx
                .app_data_dir
                .join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if blob_needs_download(conn, row_id, &local_path, &sha256) {
                enqueue_pending_blob(conn, row_id, &sha256, &rel_path, size)?;
            }
        }
    }
    enqueue_pending_fts(conn, table, &payload_obj)?;

    Ok(RowOutcome::Applied)
}

/// True when the asset blob must be downloaded: the local file is absent, OR its
/// cached hash (in `sync_blob_index`) does not match the wire sha256. Cheap mtime
/// re-hash is left to the push side; here we only consult presence + cached hash.
fn blob_needs_download(conn: &Connection, asset_id: &str, local_path: &Path, sha256: &str) -> bool {
    if !local_path.exists() {
        return true;
    }
    let cached: Option<String> = conn
        .query_row(
            "SELECT sha256 FROM sync_blob_index WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .ok();
    cached.as_deref() != Some(sha256)
}

// ---------------------------------------------------------------------------
// Page apply (PROTOCOL flow step 6, DESIGN §4.3 — one transaction per page)
// ---------------------------------------------------------------------------

/// Sorts pulled rows for one page: upserts in parents-before-children order,
/// deletes in children-before-parents order (DESIGN §4.10). Stable within a
/// table so server_seq order is otherwise preserved. Returns a new ordering of
/// references into `rows`.
fn order_page(rows: &[PullRow]) -> Vec<&PullRow> {
    let pos = |t: &str| {
        SYNCED_TABLES_FK_ORDER
            .iter()
            .position(|x| *x == t)
            .unwrap_or(usize::MAX)
    };
    let mut ordered: Vec<&PullRow> = rows.iter().collect();
    ordered.sort_by(|a, b| {
        // Deletes come after upserts overall is not required; what matters is the
        // FK ordering: upserts ascending, deletes descending. Group by op first so
        // upserts (parents→children) precede deletes (children→parents).
        match (a.deleted, b.deleted) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            (false, false) => pos(&a.table).cmp(&pos(&b.table)),
            (true, true) => pos(&b.table).cmp(&pos(&a.table)),
        }
    });
    ordered
}

/// Result of applying a whole page.
#[derive(Debug, Default)]
pub struct PageOutcome {
    pub applied: usize,
    pub skipped: usize,
    pub journaled: usize,
    pub parked: usize,
    /// The highest `server_seq` applied/seen in the page (the new cursor).
    pub max_seq: i64,
}

/// Applies a full pull page in ONE transaction (DESIGN §4.3, PROTOCOL step 6):
///
/// 1. `BEGIN; PRAGMA defer_foreign_keys=ON; applying='1'`.
/// 2. Rows ordered parents→children (upserts) / children→parents (deletes),
///    each in its own SAVEPOINT; a SQL failure parks the row.
/// 3. Persist `last_pull_seq` (= `next_since`) in the SAME transaction.
/// 4. COMMIT. On a deferred-FK COMMIT failure: `PRAGMA foreign_key_check` per
///    touched table (transaction still open), map rowid→TEXT id, ROLLBACK,
///    re-BEGIN + re-set `applying`, re-apply the page MINUS the violators (which
///    are parked into `sync_pending_rows`), then COMMIT.
///
/// `next_since` is the cursor to persist; passing the page's `next_since` keeps
/// the cursor monotonic even when the last row was skipped.
pub fn apply_page(
    conn: &Connection,
    ctx: &mut ApplyContext,
    rows: &[PullRow],
    next_since: i64,
) -> Result<PageOutcome, String> {
    let ordered = order_page(rows);

    // Iteratively park FK violators until the page commits or a round finds no
    // NEW violator. A single round is insufficient for a multi-level FK chain: on
    // a bulk first pull a child and its grandchild can both land in one page while
    // their common ancestor arrives in a later page — parking the child surfaces
    // the grandchild as a fresh violator on re-apply. Each round is a
    // self-contained BEGIN/COMMIT, so accumulating the park set and retrying is
    // safe (parked rows are persisted into sync_pending_rows for a later page).
    let mut violators: HashSet<(String, String)> = HashSet::new();
    loop {
        match try_apply_page(conn, ctx, &ordered, next_since, &violators) {
            Ok(outcome) => return Ok(outcome),
            Err(PageError::Sql(msg)) => return Err(msg),
            Err(PageError::CommitFkViolation { offending }) => {
                let before = violators.len();
                violators.extend(offending);
                if violators.len() == before {
                    // No new violator parked this round → genuinely unresolvable.
                    return Err("[sync] page still violates FK after parking violators".to_string());
                }
            }
        }
    }
}

enum PageError {
    Sql(String),
    CommitFkViolation {
        offending: HashSet<(String, String)>,
    },
}

/// One attempt at applying the ordered page. Rows whose `(table, row_id)` is in
/// `park` are NOT applied — they are parked into `sync_pending_rows`. Manages the
/// transaction manually so a deferred-FK COMMIT failure can be inspected before
/// rollback (the failed COMMIT leaves the transaction open).
fn try_apply_page(
    conn: &Connection,
    ctx: &mut ApplyContext,
    ordered: &[&PullRow],
    next_since: i64,
    park: &HashSet<(String, String)>,
) -> Result<PageOutcome, PageError> {
    conn.execute_batch("BEGIN; PRAGMA defer_foreign_keys=ON;")
        .map_err(|e| PageError::Sql(format!("[sync] failed to begin page tx: {e}")))?;
    if let Err(e) = meta_set(conn, "applying", "1") {
        let _ = conn.execute_batch("ROLLBACK;");
        return Err(PageError::Sql(e));
    }

    let mut outcome = PageOutcome::default();
    let mut touched_tables: HashSet<String> = HashSet::new();

    for row in ordered {
        let key = (row.table.clone(), row.row_id.clone());
        if row.server_seq > outcome.max_seq {
            outcome.max_seq = row.server_seq;
        }

        if park.contains(&key) {
            // Park this violator (DESIGN §4.3): record it for a later retry.
            if let Err(e) = park_row(conn, row) {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(PageError::Sql(e));
            }
            outcome.parked += 1;
            continue;
        }

        // Per-row SAVEPOINT (PROTOCOL step 5): a residual SQL failure reverts only
        // this row, then it is parked (drift-attributable) or journaled.
        if let Err(e) = conn.execute_batch("SAVEPOINT row_sp;") {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(PageError::Sql(format!("[sync] savepoint failed: {e}")));
        }

        match apply_row(conn, ctx, row) {
            Ok(RowOutcome::Applied) => {
                conn.execute_batch("RELEASE row_sp;")
                    .map_err(|e| PageError::Sql(format!("[sync] release savepoint: {e}")))?;
                touched_tables.insert(row.table.clone());
                outcome.applied += 1;
            }
            Ok(RowOutcome::Skipped) => {
                conn.execute_batch("RELEASE row_sp;")
                    .map_err(|e| PageError::Sql(format!("[sync] release savepoint: {e}")))?;
                outcome.skipped += 1;
            }
            Ok(RowOutcome::Journaled) => {
                conn.execute_batch("RELEASE row_sp;")
                    .map_err(|e| PageError::Sql(format!("[sync] release savepoint: {e}")))?;
                outcome.journaled += 1;
            }
            Err(_sql_err) => {
                // Row-local failure → revert just this row and park it as
                // drift-attributable (PROTOCOL step 5).
                conn.execute_batch("ROLLBACK TO row_sp; RELEASE row_sp;")
                    .map_err(|e| PageError::Sql(format!("[sync] rollback savepoint: {e}")))?;
                park_row(conn, row).map_err(PageError::Sql)?;
                outcome.parked += 1;
            }
        }
    }

    // Persist the cursor in the SAME transaction (DESIGN §4.3).
    if let Err(e) = meta_set_i64(conn, "last_pull_seq", next_since) {
        let _ = conn.execute_batch("ROLLBACK;");
        return Err(PageError::Sql(e));
    }
    if let Err(e) = meta_set(conn, "applying", "0") {
        let _ = conn.execute_batch("ROLLBACK;");
        return Err(PageError::Sql(e));
    }

    // COMMIT. A deferred-FK violation surfaces here (the failed COMMIT leaves the
    // transaction OPEN so we can inspect it before rollback — DESIGN §4.3).
    match conn.execute_batch("COMMIT;") {
        Ok(()) => Ok(outcome),
        Err(commit_err) => {
            // Collect the offending TEXT ids per touched table while the tx is
            // still open.
            let offending = collect_fk_violators(conn, &touched_tables).unwrap_or_default();
            // Diagnostic: log the distinct unsatisfied FK edges (child_table ->
            // parent_table) so a genuinely-unresolvable violation is identifiable.
            if let Ok(mut stmt) = conn.prepare("PRAGMA foreign_key_check") {
                let edges: std::collections::BTreeSet<String> = stmt
                    .query_map([], |r| {
                        Ok(format!(
                            "{}->{}",
                            r.get::<_, String>(0).unwrap_or_default(),
                            r.get::<_, String>(2).unwrap_or_default()
                        ))
                    })
                    .map(|rows| rows.filter_map(Result::ok).collect())
                    .unwrap_or_default();
                if !edges.is_empty() {
                    eprintln!(
                        "[sync] page FK violations (child->parent): {}",
                        edges.into_iter().collect::<Vec<_>>().join(", ")
                    );
                }
            }
            let _ = conn.execute_batch("ROLLBACK;");
            if offending.is_empty() {
                // Not an FK issue we can recover from by parking.
                Err(PageError::Sql(format!(
                    "[sync] page commit failed (non-FK): {commit_err}"
                )))
            } else {
                Err(PageError::CommitFkViolation { offending })
            }
        }
    }
}

/// Runs `PRAGMA foreign_key_check({table})` for every touched table inside the
/// still-open transaction and maps each offending rowid back to its TEXT `id`,
/// returning the `(table, row_id)` set to park (DESIGN §4.3).
fn collect_fk_violators(
    conn: &Connection,
    touched: &HashSet<String>,
) -> Result<HashSet<(String, String)>, String> {
    let mut offending = HashSet::new();
    for table in touched {
        if !is_safe_identifier(table) {
            continue;
        }
        // foreign_key_check returns: (table, rowid, referred_table, fk_id).
        let sql = format!("PRAGMA foreign_key_check({})", quote_identifier(table));
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("[sync] failed to prepare fk check for {table}: {e}"))?;
        let rowids = stmt
            .query_map([], |row| row.get::<_, Option<i64>>(1))
            .map_err(|e| format!("[sync] failed to run fk check for {table}: {e}"))?;
        let mut violating_rowids = Vec::new();
        for rowid in rowids {
            if let Some(rid) = rowid.map_err(|e| format!("[sync] fk check rowid: {e}"))? {
                violating_rowids.push(rid);
            }
        }
        drop(stmt);
        for rowid in violating_rowids {
            let id: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT id FROM {} WHERE rowid = ?1",
                        quote_identifier(table)
                    ),
                    [rowid],
                    |row| row.get(0),
                )
                .ok();
            if let Some(id) = id {
                offending.insert((table.clone(), id));
            }
        }
    }
    Ok(offending)
}

/// Parks a row in `sync_pending_rows` for a later retry (DESIGN §4.3), stamping
/// the current local schema head so it can be reattempted when the head changes
/// (PROTOCOL step 5 drift parking). Idempotent upsert; bumps `retry_count`.
fn park_row(conn: &Connection, row: &PullRow) -> Result<(), String> {
    let payload = row.payload.as_ref().map(|p| p.to_string());
    let schema_head = meta_get(conn, "schema_head").ok().flatten();
    conn.execute(
        "INSERT INTO sync_pending_rows(table_name, row_id, server_seq, deleted, changed_at, device_id, payload, retry_count, parked_schema_head)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)
         ON CONFLICT(table_name, row_id) DO UPDATE SET
           server_seq = excluded.server_seq, deleted = excluded.deleted,
           changed_at = excluded.changed_at, device_id = excluded.device_id,
           payload = excluded.payload, retry_count = sync_pending_rows.retry_count + 1,
           parked_schema_head = excluded.parked_schema_head",
        rusqlite::params![
            row.table,
            row.row_id,
            row.server_seq,
            row.deleted as i64,
            row.changed_at,
            row.device_id,
            payload,
            schema_head
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to park row {}/{}: {e}", row.table, row.row_id))
}

// ---------------------------------------------------------------------------
// FTS drain (PROTOCOL flow step 8, DESIGN §4)
// ---------------------------------------------------------------------------

/// Drains `sync_pending_fts`, reindexing each queued item through the existing
/// `nlp::fts::index_item_from_db` code path (the SAME path the NLP worker uses,
/// so the contentless FTS5 `rowid == items.rowid` contract is preserved). A
/// queue entry is deleted only after its reindex succeeds; an item that no longer
/// exists locally is treated as a success (its reindex is a no-op) and removed.
/// Returns the number of items reindexed. Not run inside the page transaction —
/// FTS is derived state and is rebuilt idempotently.
pub fn drain_pending_fts(conn: &Connection) -> Result<usize, String> {
    let item_ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT item_id FROM sync_pending_fts")
            .map_err(|e| format!("[sync] failed to prepare pending fts read: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("[sync] failed to query pending fts: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("[sync] failed to read pending fts: {e}"))?);
        }
        out
    };

    let mut reindexed = 0usize;
    for item_id in item_ids {
        crate::nlp::fts::index_item_from_db(conn, &item_id)?;
        conn.execute(
            "DELETE FROM sync_pending_fts WHERE item_id = ?1",
            [&item_id],
        )
        .map_err(|e| format!("[sync] failed to clear pending fts for {item_id}: {e}"))?;
        reindexed += 1;
    }
    Ok(reindexed)
}

// ---------------------------------------------------------------------------
// Pending-row retry (DESIGN §4.3 — drain parking after each page / cycle bounds)
// ---------------------------------------------------------------------------

/// A parked row reconstructed from `sync_pending_rows`.
struct ParkedRow {
    pull: PullRow,
}

fn read_pending_rows(conn: &Connection) -> Result<Vec<ParkedRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT table_name, row_id, server_seq, deleted, changed_at, device_id, payload
             FROM sync_pending_rows ORDER BY server_seq ASC",
        )
        .map_err(|e| format!("[sync] failed to prepare pending rows read: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            let table: String = row.get(0)?;
            let row_id: String = row.get(1)?;
            let server_seq: i64 = row.get(2)?;
            let deleted: i64 = row.get(3)?;
            let changed_at: i64 = row.get(4)?;
            let device_id: String = row.get(5)?;
            let payload: Option<String> = row.get(6)?;
            Ok((
                table, row_id, server_seq, deleted, changed_at, device_id, payload,
            ))
        })
        .map_err(|e| format!("[sync] failed to query pending rows: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        let (table, row_id, server_seq, deleted, changed_at, device_id, payload) =
            row.map_err(|e| format!("[sync] failed to read pending row: {e}"))?;
        let payload_value = payload
            .as_deref()
            .and_then(|p| serde_json::from_str::<Value>(p).ok());
        out.push(ParkedRow {
            pull: PullRow {
                table,
                row_id,
                server_seq,
                deleted: deleted != 0,
                changed_at,
                device_id,
                payload: payload_value,
            },
        });
    }
    Ok(out)
}

fn delete_pending_row(conn: &Connection, table: &str, row_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM sync_pending_rows WHERE table_name = ?1 AND row_id = ?2",
        rusqlite::params![table, row_id],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to delete pending row {table}/{row_id}: {e}"))
}

/// True when a parent row exists locally for a parked child that references it.
/// Used by the end-of-cycle `parent_deleted` decision: a parked child whose
/// parent is neither present locally NOR itself parked is treated as confirmed
/// tombstoned (DESIGN §4.3).
fn parent_present_or_parked(conn: &Connection, child: &PullRow) -> Result<bool, String> {
    // The FK parent of a child depends on its table. We derive the parent
    // (table, id) from the payload's FK column.
    let Some(payload) = &child.payload else {
        return Ok(true); // No payload to inspect — be conservative, keep parked.
    };
    let parent_refs: &[(&str, &str)] = match child.table.as_str() {
        "items" => &[("collections", "collection_id")],
        "assets" | "notes" | "entities" | "triples" | "item_topics" => &[("items", "item_id")],
        "extractions" | "transcriptions" | "layouts" | "annotations" => &[("assets", "asset_id")],
        "rag_messages" => &[("rag_conversations", "conversation_id")],
        _ => &[],
    };
    for (parent_table, fk_col) in parent_refs {
        // item_topics has TWO parents (item + topic); check both via separate arms.
        let Some(parent_id) = payload.get(*fk_col).and_then(Value::as_str) else {
            continue;
        };
        let present: bool = conn
            .query_row(
                &format!(
                    "SELECT 1 FROM {} WHERE id = ?1",
                    quote_identifier(parent_table)
                ),
                [parent_id],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if present {
            return Ok(true);
        }
        let parked: bool = conn
            .query_row(
                "SELECT 1 FROM sync_pending_rows WHERE table_name = ?1 AND row_id = ?2",
                rusqlite::params![parent_table, parent_id],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if parked {
            return Ok(true);
        }
    }
    // item_topics has a second parent (topics) — check it too.
    if child.table == "item_topics" {
        if let Some(topic_id) = payload.get("topic_id").and_then(Value::as_str) {
            let present: bool = conn
                .query_row("SELECT 1 FROM topics WHERE id = ?1", [topic_id], |_| {
                    Ok(true)
                })
                .unwrap_or(false);
            let parked: bool = conn
                .query_row(
                    "SELECT 1 FROM sync_pending_rows WHERE table_name = 'topics' AND row_id = ?1",
                    [topic_id],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if present || parked {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Retries every parked row once, inside a single transaction with deferred FK
/// and `applying='1'` (DESIGN §4.3). A row that now applies cleanly leaves the
/// parking and feeds the same post-apply pipeline; a row that still fails stays
/// parked. When `final_pass` is true (end of cycle), a still-parked child whose
/// parent is confirmed tombstoned (absent locally AND not parked) is journaled
/// `parent_deleted` and removed from parking. Returns the number of rows drained
/// (successfully applied).
pub fn retry_pending_rows(
    conn: &Connection,
    ctx: &mut ApplyContext,
    final_pass: bool,
) -> Result<usize, String> {
    let parked = read_pending_rows(conn)?;
    if parked.is_empty() {
        return Ok(0);
    }

    // FK is enforced IMMEDIATELY here (no defer): a still-orphaned row must fail
    // inside its own SAVEPOINT so it can be re-parked or journaled
    // `parent_deleted`, rather than optimistically "applying" and then breaking
    // the whole commit (the parent, if it had arrived, would already be present).
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin pending-row retry tx: {e}"))?;
    meta_set(&tx, "applying", "1")?;

    let mut drained = 0usize;
    for parked_row in &parked {
        tx.execute_batch("SAVEPOINT retry_sp;")
            .map_err(|e| format!("[sync] retry savepoint: {e}"))?;
        match apply_row(&tx, ctx, &parked_row.pull) {
            Ok(RowOutcome::Applied) | Ok(RowOutcome::Skipped) | Ok(RowOutcome::Journaled) => {
                tx.execute_batch("RELEASE retry_sp;")
                    .map_err(|e| format!("[sync] retry release: {e}"))?;
                delete_pending_row(&tx, &parked_row.pull.table, &parked_row.pull.row_id)?;
                drained += 1;
            }
            Err(_) => {
                tx.execute_batch("ROLLBACK TO retry_sp; RELEASE retry_sp;")
                    .map_err(|e| format!("[sync] retry rollback: {e}"))?;
                // On the final pass, decide parent_deleted when the parent is
                // confirmed tombstoned.
                if final_pass && !parent_present_or_parked(&tx, &parked_row.pull)? {
                    journal_conflict(
                        &tx,
                        &parked_row.pull.table,
                        &parked_row.pull.row_id,
                        "parent_deleted",
                        parked_row
                            .pull
                            .payload
                            .as_ref()
                            .map(|p| p.to_string())
                            .as_deref(),
                        Some("parent confirmed tombstoned"),
                    )?;
                    delete_pending_row(&tx, &parked_row.pull.table, &parked_row.pull.row_id)?;
                }
            }
        }
    }

    meta_set(&tx, "applying", "0")?;
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit pending-row retry: {e}"))?;
    Ok(drained)
}

#[cfg(test)]
mod tests;
