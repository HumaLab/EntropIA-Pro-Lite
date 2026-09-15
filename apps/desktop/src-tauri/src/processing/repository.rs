//! Durable storage behind the batch-processing queue (plan-lote.md §5).
//!
//! Every queue mutation in later units commits through this module on a
//! connection opened with [`crate::db::open::open_archive_connection`], so a
//! task receipt and its canonical result share one transaction. This file owns
//! the small read primitives Unidad 1 needs — migration presence, effective
//! PRAGMAs, and queue counters — plus the interface the scheduler builds on:
//! `prepare_batch`, `admit_or_attach`, `claim_next`, `save_checkpoint`,
//! `commit_success`, `finish_attempt`, `control_batch`, `recover_session` and
//! `read_snapshot` (those arrive in Unidades 2–3).

use rusqlite::Connection;

/// Migration that creates the processing tables. The frontend
/// `runMigrations()` applies it; the backend never runs DDL itself — it only
/// verifies presence before admitting or recovering work.
pub const MIGRATION_NAME: &str = "0032_batch_processing";

/// Error code returned when the queue schema is not applied yet. The frontend
/// treats it as a recoverable state (schema still migrating), never as a
/// per-item failure.
pub const SCHEMA_NOT_READY: &str = "schema_not_ready";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pragmas {
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: i64,
    pub busy_timeout: i64,
}

/// Reads the effective PRAGMAs of `conn`. Verification reads them back instead
/// of trusting the open call: a connection that silently lost one of these
/// must never admit queue work.
pub fn read_pragmas(conn: &Connection) -> Result<Pragmas, String> {
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read journal_mode: {e}"))?;
    let synchronous: i64 = conn
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read synchronous: {e}"))?;
    let foreign_keys: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read foreign_keys: {e}"))?;
    let busy_timeout: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read busy_timeout: {e}"))?;
    Ok(Pragmas {
        journal_mode,
        synchronous,
        foreign_keys,
        busy_timeout,
    })
}

/// True when the durable queue is usable on this connection: WAL is active,
/// commits are FULL, foreign keys are enforced, and the migration row exists.
pub fn is_schema_ready(conn: &Connection) -> Result<bool, String> {
    let pragmas = read_pragmas(conn)?;
    if !pragmas.journal_mode.eq_ignore_ascii_case("wal") {
        return Ok(false);
    }
    if pragmas.synchronous != 2 || pragmas.foreign_keys != 1 {
        return Ok(false);
    }
    Ok(is_migration_applied(conn, MIGRATION_NAME)?)
}

/// True when `_migrations` records `name`. A missing tracking table means no
/// migration has ever run here, which is also "not applied" — not an error.
pub fn is_migration_applied(conn: &Connection, name: &str) -> Result<bool, String> {
    use rusqlite::OptionalExtension as _;
    let table: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to inspect sqlite_master: {e}"))?;
    if table.is_none() {
        return Ok(false);
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM _migrations WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read _migrations: {e}"))?;
    Ok(count > 0)
}

/// Queue counters for the initialize response and the recovery banner.
/// Every count is a plain aggregate over the durable tables — no events, no
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueSummary {
    pub pending_batches: i64,
    pub active_tasks: i64,
    pub interrupted_tasks: i64,
    pub failed_tasks: i64,
    pub succeeded_tasks: i64,
}

fn count(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(|e| format!("Failed to count queue rows: {e}"))
}

/// Reads the queue summary. Callers must check [`is_schema_ready`] first: on a
/// database without the migration these tables do not exist.
pub fn read_summary(conn: &Connection) -> Result<QueueSummary, String> {
    Ok(QueueSummary {
        pending_batches: count(
            conn,
            "SELECT COUNT(*) FROM processing_batches WHERE state IN ('preparing', 'ready', 'running', 'pausing', 'paused', 'interrupted')",
        )?,
        active_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state IN ('pending', 'blocked', 'running', 'retry_wait')",
        )?,
        interrupted_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'interrupted'",
        )?,
        failed_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'failed'",
        )?,
        succeeded_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'succeeded'",
        )?,
    })
}
/// Task states that still own the work unit. Anything else is history: a new
/// admission after a terminal state transitions that same row (new
/// retry_cycle), never inserts a second live writer — the partial unique
/// index enforces it.
pub const TERMINAL_TASK_STATES: &[&str] = &["succeeded", "failed", "skipped", "cancelled"];

/// Operations a batch asked for, parsed from `processing_batches.operations`
/// (JSON array of `"ocr"` / `"embeddings"`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchOperations {
    pub ocr: bool,
    pub embeddings: bool,
}
pub fn batch_operations(conn: &Connection, batch_id: &str) -> Result<BatchOperations, String> {
    let raw: String = conn
        .query_row(
            "SELECT operations FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read batch {batch_id}: {e}"))?;
    let mut ops = BatchOperations::default();
    if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
        for entry in list {
            match entry.as_str() {
                "ocr" => ops.ocr = true,
                "embeddings" => ops.embeddings = true,
                _ => {}
            }
        }
    }
    Ok(ops)
}

/// Snapshots the batch scope: one member row per asset currently in the given
/// collections. A single `INSERT ... SELECT` statement, so concurrent imports
/// cannot leak half a scope into the batch — assets added afterwards belong
/// to a later batch. Returns the number of members added.
#[allow(dead_code)]
pub fn prepare_membership(
    conn: &Connection,
    batch_id: &str,
    collection_ids: &[String],
) -> Result<usize, String> {
    if collection_ids.is_empty() {
        return Ok(0);
    }
    let scope = serde_json::to_string(collection_ids)
        .map_err(|e| format!("Failed to encode batch scope: {e}"))?;
    let added = conn
        .execute(
            "INSERT OR IGNORE INTO processing_batch_members
               (batch_id, ordinal, asset_id_snapshot, item_id_snapshot, collection_id_snapshot, title_snapshot)
             SELECT ?1, ROW_NUMBER() OVER (ORDER BY a.created_at, a.id) - 1,
                    a.id, a.item_id, i.collection_id, i.title
             FROM assets a JOIN items i ON i.id = a.item_id
             WHERE i.collection_id IN (SELECT value FROM json_each(?2))",
            rusqlite::params![batch_id, scope],
        )
        .map_err(|e| format!("Failed to snapshot scope of batch {batch_id}: {e}"))?;
    Ok(added as usize)
}

/// Outcome of admitting one work unit for one batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmitOutcome {
    pub task_id: String,
    /// False when another batch (or an earlier page) already owned the unit
    /// and this batch only attached to it.
    pub created: bool,
}

fn live_task(conn: &Connection, kind: &str, asset_id: &str) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension as _;
    // Single source of truth with TERMINAL_TASK_STATES: the literals below
    // must stay in sync with the partial unique index, so they are built
    // from the constant instead of repeated by hand.
    let excluded = TERMINAL_TASK_STATES
        .iter()
        .map(|state| format!("'{state}'"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.query_row(
        &format!(
            "SELECT id FROM processing_tasks WHERE kind = ?1 AND asset_id_snapshot = ?2 AND state NOT IN ({excluded})"
        ),
        rusqlite::params![kind, asset_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| format!("Failed to look up live {kind} task for {asset_id}: {e}"))
}

fn link_batch_task(
    conn: &Connection,
    batch_id: &str,
    task_id: &str,
    kind: &str,
    asset_id: &str,
    dependency_task_id: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO processing_batch_tasks
           (batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
        rusqlite::params![batch_id, task_id, kind, asset_id, dependency_task_id],
    )
    .map_err(|e| format!("Failed to link task {task_id} to batch {batch_id}: {e}"))?;
    Ok(())
}

/// Admits one work unit for one batch, or attaches the batch to the live unit
/// another batch already owns. Two batches over the same asset share one
/// physical task and never start two workers on it; per-batch pause/cancel
/// only flips the link's `request_state` (Unidad 3).
#[allow(dead_code)]
pub fn admit_or_attach(
    conn: &Connection,
    batch_id: &str,
    kind: &str,
    asset_id: &str,
    input_revision: i64,
    input_fingerprint: &str,
    contract_hash: &str,
    dependency_task_id: Option<&str>,
) -> Result<AdmitOutcome, String> {
    if kind == "embedding" {
        let origin: String = conn
            .query_row(
                "SELECT origin FROM processing_batches WHERE id = ?1",
                [batch_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("invalid_selection: unknown batch {batch_id}: {error}"))?;
        if origin != "repair" {
            conn.execute(
                "UPDATE processing_asset_revisions SET auto_suppressed_revision = NULL
                 WHERE asset_id = ?1 AND auto_suppressed_revision = ?2",
                rusqlite::params![asset_id, input_revision],
            )
            .map_err(|error| {
                format!("Failed to clear repair suppression for {asset_id}: {error}")
            })?;
        }
    }
    if let Some(task_id) = live_task(conn, kind, asset_id)? {
        link_batch_task(conn, batch_id, &task_id, kind, asset_id, dependency_task_id)?;
        return Ok(AdmitOutcome {
            task_id,
            created: false,
        });
    }
    // Terminal attempts remain immutable history; a new demand gets a new identity.
    let task_id = uuid::Uuid::new_v4().to_string();
    let state = if dependency_task_id.is_some() {
        "blocked"
    } else {
        "pending"
    };
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO processing_tasks
               (id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s', 'now') * 1000, strftime('%s', 'now') * 1000)",
            rusqlite::params![
                task_id,
                kind,
                asset_id,
                input_revision,
                input_fingerprint,
                contract_hash,
                state
            ],
        )
        .map_err(|e| format!("Failed to admit {kind} task for {asset_id}: {e}"))?;
    if inserted == 0 {
        // Lost a race with a concurrent admitter (or a terminal row for the
        // same unit exists): fall back to the live row when there is one.
        if let Some(existing) = live_task(conn, kind, asset_id)? {
            link_batch_task(
                conn,
                batch_id,
                &existing,
                kind,
                asset_id,
                dependency_task_id,
            )?;
            return Ok(AdmitOutcome {
                task_id: existing,
                created: false,
            });
        }
        return Err(format!(
            "A terminal {kind} task already exists for {asset_id}; requeue it through an explicit retry"
        ));
    }
    link_batch_task(conn, batch_id, &task_id, kind, asset_id, dependency_task_id)?;
    Ok(AdmitOutcome {
        task_id,
        created: true,
    })
}

/// Admits automatic embedding repair only when the same source revision was
/// not explicitly cancelled and no user/manual request already owns it.
pub fn admit_repair_or_attach(
    conn: &Connection,
    batch_id: &str,
    asset_id: &str,
    input_revision: i64,
    input_fingerprint: &str,
    contract_hash: &str,
) -> Result<Option<AdmitOutcome>, String> {
    let origin: String = conn
        .query_row(
            "SELECT origin FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("invalid_selection: unknown batch {batch_id}: {error}"))?;
    if origin != "repair" {
        return Err(format!(
            "invalid_selection: batch {batch_id} is not a repair batch"
        ));
    }
    if live_task(conn, "embedding", asset_id)?.is_some() {
        return Ok(None);
    }
    let suppressed: Option<i64> = conn
        .query_row(
            "SELECT auto_suppressed_revision FROM processing_asset_revisions WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!(
                "Failed to read repair suppression for {asset_id}: {other}"
            )),
        })?;
    if suppressed == Some(input_revision) {
        return Ok(None);
    }
    admit_or_attach(
        conn,
        batch_id,
        "embedding",
        asset_id,
        input_revision,
        input_fingerprint,
        contract_hash,
        None,
    )
    .map(Some)
}

/// Cheap identity of the OCR input: asset id, stored path, and byte size.
/// v1 never regenerates finished OCR, so this only has to catch a source
/// swap before a claim or COMMIT — not invisible same-size rewrites.
fn ocr_fingerprint(conn: &Connection, asset_id: &str) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT id || '|' || path || '|' || COALESCE(size, -1) FROM assets WHERE id = ?1",
        [asset_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| format!("Failed to fingerprint {asset_id}: {e}"))
}

fn source_revision(conn: &Connection, asset_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = ?1",
        [asset_id],
        |row| row.get(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(0),
        other => Err(format!("Failed to read revision of {asset_id}: {other}")),
    })
}
/// Control actions a batch accepts. Pause never discards confirmed work;
/// cancel never deletes canonical results — both only withdraw demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchAction {
    Pause,
    Resume,
    Cancel,
}
/// Applies a control intent to one batch: persists `desired_state` FIRST so a
/// crash between intent and effect still converges (recovery finishes
/// `cancelling`, honors `pausing`), flips this batch's links, and bumps the
/// revision. Supervisors observe the links; in-flight units finish their
/// current checkpoint and then stop.
pub fn control_batch(
    conn: &Connection,
    batch_id: &str,
    action: BatchAction,
    expected_revision: Option<i64>,
) -> Result<(), String> {
    let row: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT state, desired_state, revision FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read batch {batch_id}: {other}")),
        })?;
    let Some((state, _desired, revision)) = row else {
        return Err(format!("invalid_selection: unknown batch {batch_id}"));
    };
    if let Some(expected) = expected_revision {
        if expected != revision {
            return Err(format!(
                "revision_conflict: batch {batch_id} is at revision {revision}, not {expected}"
            ));
        }
    }
    if ["completed", "completed_with_errors", "cancelled"].contains(&state.as_str()) {
        return Err(format!(
            "invalid_transition: batch {batch_id} is already {state}"
        ));
    }
    let valid = state != "cancelling" || action == BatchAction::Cancel;
    if !valid {
        return Err(format!(
            "invalid_transition: cannot apply {action:?} to batch {batch_id} in {state}"
        ));
    }
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("Failed to begin control of {batch_id}: {e}"))?;
    let controlled = (|| -> Result<(), String> {
        // Persist the intent first: a crash between intent and effect still
        // converges because recovery finishes `cancelling` and honors `pause`.
        let new_desired = match action {
            BatchAction::Pause => "pause",
            BatchAction::Resume => "run",
            BatchAction::Cancel => "cancel",
        };
        conn.execute(
            "UPDATE processing_batches SET desired_state = ?1,
               updated_at = strftime('%s', 'now') * 1000, revision = revision + 1
             WHERE id = ?2",
            rusqlite::params![new_desired, batch_id],
        )
        .map_err(|e| format!("Failed to persist intent on {batch_id}: {e}"))?;
        match action {
            BatchAction::Pause => {
                // running -> pausing; every other live state keeps running
                // under the persisted desired=pause (no new claims).
                conn.execute(
                    "UPDATE processing_batches SET state = 'pausing',
                       updated_at = strftime('%s', 'now') * 1000
                     WHERE id = ?1 AND state = 'running'",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to pause {batch_id}: {e}"))?;
                conn.execute(
                    "UPDATE processing_batch_tasks SET request_state = 'paused' WHERE batch_id = ?1",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to flip links of {batch_id}: {e}"))?;
            }
            BatchAction::Resume => {
                // Land where planning actually is: a complete snapshot goes
                // straight back to ready (the tick promotes to running), an
                // incomplete one re-enters preparing — resume never jumps
                // over unfinished planning, nor re-plans finished work.
                conn.execute(
                    "UPDATE processing_batches SET
                       state = CASE WHEN planning_done = 1 THEN 'ready' ELSE 'preparing' END,
                       updated_at = strftime('%s', 'now') * 1000
                     WHERE id = ?1 AND state IN ('paused', 'pausing', 'interrupted')",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to resume {batch_id}: {e}"))?;
                conn.execute(
                    "UPDATE processing_batch_tasks SET request_state = 'active'
                     WHERE batch_id = ?1 AND request_state = 'paused'",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to flip links of {batch_id}: {e}"))?;
                // Interrupted units go back to pending: the next claim
                // revalidates their input and replays only missing
                // checkpoints. Units blocked on a stale contract re-enter
                // evaluation instead of sticking forever.
                conn.execute(
                    "UPDATE processing_tasks SET state = 'pending', owner_session = NULL,
                       next_retry_at = NULL, updated_at = strftime('%s', 'now') * 1000
                     WHERE state = 'interrupted'
                       AND id IN (SELECT task_id FROM processing_batch_tasks WHERE batch_id = ?1)",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to requeue interrupted units of {batch_id}: {e}"))?;
                conn.execute(
                    "UPDATE processing_tasks SET state = 'pending', outcome = '',
                       source_invalidation_count = 0,
                       last_error_code = NULL, last_error_message = NULL,
                       updated_at = strftime('%s', 'now') * 1000
                     WHERE state = 'blocked' AND outcome IN ('configuration_changed', 'source_unstable')
                       AND id IN (SELECT task_id FROM processing_batch_tasks WHERE batch_id = ?1)",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to requeue blocked units of {batch_id}: {e}"))?;
            }
            BatchAction::Cancel => {
                conn.execute(
                    "INSERT INTO processing_asset_revisions
                       (asset_id, source_revision, invalidation_reason, auto_suppressed_revision)
                     SELECT DISTINCT t.asset_id_snapshot,
                       COALESCE(r.source_revision, t.input_revision), 'cancelled',
                       COALESCE(r.source_revision, t.input_revision)
                     FROM processing_batch_tasks l
                     JOIN processing_tasks t ON t.id = l.task_id
                     LEFT JOIN processing_asset_revisions r ON r.asset_id = t.asset_id_snapshot
                     WHERE l.batch_id = ?1 AND t.kind = 'embedding'
                     ON CONFLICT(asset_id) DO UPDATE SET
                       invalidation_reason = 'cancelled',
                       auto_suppressed_revision = excluded.auto_suppressed_revision",
                    [batch_id],
                )
                .map_err(|error| {
                    format!("Failed to suppress cancelled repairs for {batch_id}: {error}")
                })?;
                conn.execute(
                    "UPDATE processing_batches SET state = 'cancelling',
                       updated_at = strftime('%s', 'now') * 1000
                     WHERE id = ?1",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to cancel {batch_id}: {e}"))?;
                conn.execute(
                    "UPDATE processing_batch_tasks SET request_state = 'cancelled' WHERE batch_id = ?1",
                    [batch_id],
                )
                .map_err(|e| format!("Failed to flip links of {batch_id}: {e}"))?;
                cancel_orphaned_tasks(conn)?;
                maybe_finalize_batch(conn, batch_id)?;
            }
        }
        Ok(())
    })();
    match controlled {
        Ok(()) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("Failed to commit control of {batch_id}: {e}"))?;
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}
/// Long-lived system batches that own out-of-band work: deliberate manual
/// actions (`manual`) and automatic maintenance (`repair`). Created lazily,
/// always `running` with a complete (empty) snapshot, so admitted units flow
/// straight to the scheduler without a UI batch around them. Hidden from the
/// batch history by origin (Unidad 5 lists `user` batches).
pub fn ensure_system_batch(conn: &Connection, origin: &str) -> Result<String, String> {
    if origin != "manual" && origin != "repair" {
        return Err(format!("invalid_selection: unknown system origin {origin}"));
    }
    let request_id = format!("system-{origin}");
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM processing_batches WHERE request_id = ?1",
            [&request_id],
            |row| row.get::<_, String>(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read system batch {origin}: {other}")),
        })?
    {
        conn.execute(
            "UPDATE processing_batches SET state = 'running', desired_state = 'run', finished_at = NULL WHERE id = ?1 AND state IN ('completed','completed_with_errors')",
            [&id],
        ).map_err(|e| format!("Failed to reopen system batch: {e}"))?;
        return Ok(id);
    }
    let batch_id = format!("batch-system-{origin}");
    conn.execute(
        "INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'running', 'run', '[\"ocr\", \"embeddings\"]', 1, strftime('%s', 'now') * 1000, strftime('%s', 'now') * 1000)",
        rusqlite::params![batch_id, request_id, origin],
    )
    .map_err(|e| format!("Failed to create system batch {origin}: {e}"))?;
    Ok(batch_id)
}
/// A recorded control-plane request: the durable answer to "did this
/// requestId already run, and with which parameters?". Double-clicks,
/// retries, and lost IPC responses replay against this row instead of
/// duplicating batches or retry cycles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotentRequest {
    pub batch_id: Option<String>,
    pub payload_hash: String,
    pub response_json: Option<String>,
}

/// Looks up a previous request by id. `None` means first sight: proceed and
/// [`record_request`] the outcome in the same flow.
pub fn find_request(
    conn: &Connection,
    request_id: &str,
) -> Result<Option<IdempotentRequest>, String> {
    conn.query_row(
        "SELECT batch_id, payload_hash, response_json FROM processing_requests WHERE request_id = ?1",
        [request_id],
        |row| {
            Ok(IdempotentRequest {
                batch_id: row.get(0)?,
                payload_hash: row.get(1)?,
                response_json: row.get(2)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(format!("Failed to check request {request_id}: {other}")),
    })
}

/// Records a request outcome. The primary key rejects a second insert for
/// the same id, so concurrent duplicates serialize on this write.
#[allow(clippy::too_many_arguments)]
pub fn record_request(
    conn: &Connection,
    request_id: &str,
    action: &str,
    batch_id: Option<&str>,
    payload_hash: &str,
    response_json: &str,
    now_ms: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO processing_requests (request_id, action, batch_id, payload_hash, state, response_json, created_at)
         VALUES (?1, ?2, ?3, ?4, 'applied', ?5, ?6)",
        rusqlite::params![request_id, action, batch_id, payload_hash, response_json, now_ms],
    )
    .map_err(|e| format!("Failed to record request {request_id}: {e}"))?;
    Ok(())
}
/// Interrupts a running unit without recording a provider failure: the
/// attempt closes as interrupted and every confirmed checkpoint survives.
/// Recovery, pause, and cooperative stop all funnel through here.
pub fn interrupt_task(conn: &Connection, task_id: &str, lease_epoch: i64) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET state = 'interrupted', owner_session = NULL,
               updated_at = strftime('%s', 'now') * 1000
             WHERE id = ?1 AND state = 'running' AND lease_epoch = ?2",
            rusqlite::params![task_id, lease_epoch],
        )
        .map_err(|e| format!("Failed to interrupt {task_id}: {e}"))?;
    if changed == 0 {
        return Err(format!(
            "lease_lost: {task_id} is no longer owned by epoch {lease_epoch}"
        ));
    }
    close_open_attempt(conn, task_id, "interrupted")?;
    Ok(())
}

/// Batches whose snapshot still needs classification work, oldest first.
/// The tick advances a bounded page per batch so planning shares the loop
/// with execution instead of starving it on huge collections.
pub fn planning_batches(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM processing_batches
             WHERE state IN ('preparing', 'ready') AND planning_done = 0 AND desired_state != 'cancel'
             ORDER BY created_at, id LIMIT 4",
        )
        .map_err(|e| format!("Failed to list planning batches: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to list planning batches: {e}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| format!("Failed to list planning batches: {e}"))?;
    drop(stmt);
    Ok(rows)
}
/// Batches that may still transition: running work, observed pauses, and
/// unfinished cancellations. The tick sweeps them for finalization.
/// True while any unit of `item_id` is still expected to change its text.
///
/// The FTS index is keyed by item, but OCR commits arrive per asset, so a
/// naive reindex per commit re-reads and re-tokenizes the item's whole text
/// once per asset — quadratic on documents with many pages. The follow-up
/// waits on this instead of on a fixed quiet window, which collapses the same
/// whether a page takes three seconds or thirty.
///
/// Settled units (succeeded, failed, skipped, cancelled) do NOT count: a
/// withdrawn or exhausted batch must release the follow-up, never strand it.
pub fn item_has_live_queue_work(conn: &Connection, item_id: &str) -> Result<bool, String> {
    let live: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_tasks t
             JOIN assets a ON a.id = t.asset_id_snapshot
             WHERE a.item_id = ?1
               AND t.state IN ('pending', 'blocked', 'running', 'retry_wait', 'interrupted')",
            [item_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count live units of {item_id}: {e}"))?;
    Ok(live > 0)
}

pub fn open_batches(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM processing_batches
             WHERE state IN ('running', 'pausing', 'ready', 'cancelling') AND planning_done = 1
             ORDER BY created_at, id LIMIT 32",
        )
        .map_err(|e| format!("Failed to list open batches: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to list open batches: {e}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| format!("Failed to list open batches: {e}"))?;
    drop(stmt);
    Ok(rows)
}
/// Cancels tasks nobody wants anymore: pending-like units with zero active
/// links go `cancelled` (attempts closed as cancelled). Running units stay
/// for the supervisor, which revokes them at a checkpoint boundary — yanking
/// a lease mid-inference would orphan provider-side work and lie about it.
pub fn cancel_orphaned_tasks(conn: &Connection) -> Result<usize, String> {
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET state = 'cancelled', owner_session = NULL,
               next_retry_at = NULL, updated_at = strftime('%s', 'now') * 1000
             WHERE state IN ('pending', 'blocked', 'retry_wait', 'interrupted')
               AND NOT EXISTS (
                 SELECT 1 FROM processing_batch_tasks l
                 WHERE l.task_id = processing_tasks.id AND l.request_state IN ('active', 'paused'))",
            [],
        )
        .map_err(|e| format!("Failed to cancel orphaned tasks: {e}"))?;
    conn.execute(
        "UPDATE processing_attempts SET outcome = 'cancelled',
           finished_at = strftime('%s', 'now') * 1000
         WHERE outcome = 'open'
           AND task_id IN (SELECT id FROM processing_tasks WHERE state = 'cancelled')",
        [],
    )
    .map_err(|e| format!("Failed to close orphaned attempts: {e}"))?;
    Ok(changed as usize)
}

/// Cancels a running unit the supervisor no longer owns the demand for
/// (commit-time `demand_lost`). Checkpoints survive for whoever resumes the
/// unit; the attempt closes as cancelled, never as failed.
pub fn cancel_running_task(
    conn: &Connection,
    task_id: &str,
    lease_epoch: i64,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET state = 'cancelled', owner_session = NULL,
               updated_at = strftime('%s', 'now') * 1000
             WHERE id = ?1 AND state = 'running' AND lease_epoch = ?2",
            rusqlite::params![task_id, lease_epoch],
        )
        .map_err(|e| format!("Failed to cancel running {task_id}: {e}"))?;
    if changed == 0 {
        return Err(format!(
            "lease_lost: {task_id} is no longer owned by epoch {lease_epoch}"
        ));
    }
    close_open_attempt(conn, task_id, "cancelled")?;
    Ok(())
}

/// Returns a running unit to `pending` after a non-fault stop. Checkpoints
/// remain available and no provider failure is recorded.
pub fn requeue_task(conn: &Connection, task_id: &str, lease_epoch: i64) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET state = 'pending', owner_session = NULL,
               next_retry_at = NULL, updated_at = strftime('%s', 'now') * 1000
             WHERE id = ?1 AND state = 'running' AND lease_epoch = ?2",
            rusqlite::params![task_id, lease_epoch],
        )
        .map_err(|e| format!("Failed to requeue {task_id}: {e}"))?;
    if changed == 0 {
        return Err(format!(
            "lease_lost: {task_id} is no longer owned by epoch {lease_epoch}"
        ));
    }
    close_open_attempt(conn, task_id, "interrupted")?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceChangeOutcome {
    Requeued,
    Blocked,
}

/// Records an input invalidation without charging it to the provider retry
/// budget. The third invalidation in one explicit cycle blocks until resume.
pub fn record_source_change(
    conn: &Connection,
    task_id: &str,
    lease_epoch: i64,
    message: &str,
) -> Result<SourceChangeOutcome, String> {
    conn.execute_batch("SAVEPOINT processing_source_change")
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let count: i64 = conn
            .query_row(
                "SELECT source_invalidation_count FROM processing_tasks
                 WHERE id = ?1 AND state = 'running' AND lease_epoch = ?2",
                rusqlite::params![task_id, lease_epoch],
                |row| row.get(0),
            )
            .map_err(|_| {
                format!("lease_lost: {task_id} is no longer owned by epoch {lease_epoch}")
            })?;
        let next = count + 1;
        let blocked = next >= 3;
        conn.execute(
            "UPDATE processing_tasks SET state = ?1, outcome = ?2,
               source_invalidation_count = ?3, last_error_code = ?2,
               last_error_message = ?4, owner_session = NULL, next_retry_at = NULL,
               updated_at = strftime('%s', 'now') * 1000
             WHERE id = ?5 AND state = 'running' AND lease_epoch = ?6",
            rusqlite::params![
                if blocked { "blocked" } else { "pending" },
                if blocked {
                    "source_unstable"
                } else {
                    "source_changed"
                },
                next,
                message,
                task_id,
                lease_epoch,
            ],
        )
        .map_err(|error| format!("Failed to record source change for {task_id}: {error}"))?;
        conn.execute(
            "UPDATE processing_attempts SET outcome = 'interrupted',
               finished_at = strftime('%s', 'now') * 1000,
               error_code = 'source_changed', error_message = ?1
             WHERE task_id = ?2 AND outcome = 'open'",
            rusqlite::params![message, task_id],
        )
        .map_err(|error| format!("Failed to close invalidated attempt for {task_id}: {error}"))?;
        Ok(if blocked {
            SourceChangeOutcome::Blocked
        } else {
            SourceChangeOutcome::Requeued
        })
    })();
    match result {
        Ok(outcome) => {
            conn.execute_batch("RELEASE processing_source_change")
                .map_err(|error| error.to_string())?;
            Ok(outcome)
        }
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO processing_source_change; RELEASE processing_source_change",
            );
            Err(error)
        }
    }
}

/// Opens a new retry cycle over the failed units of one batch: `failed` (or a
/// single `task_id`) goes back to `pending` with a fresh attempt budget,
/// keeping the full attempt history. Dependencies that died with the unit
/// ride along one level — retrying an embedding without its OCR would just
/// fail the same way again. Cancelled units are never resurrected here.
pub fn retry_failed(
    conn: &Connection,
    batch_id: &str,
    task_id: Option<&str>,
) -> Result<usize, String> {
    let batch_state: String = conn
        .query_row(
            "SELECT state FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("invalid_selection: unknown batch {batch_id}: {error}"))?;
    if matches!(batch_state.as_str(), "cancelling" | "cancelled") {
        return Err(format!(
            "invalid_transition: cannot retry tasks while batch {batch_id} is {batch_state}"
        ));
    }
    let mut targets: Vec<(String, Option<String>)> = Vec::new();
    if let Some(single) = task_id {
        let row: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT t.state, l.dependency_task_id
                 FROM processing_tasks t
                 JOIN processing_batch_tasks l ON l.task_id = t.id
                 WHERE l.batch_id = ?1 AND t.id = ?2",
                rusqlite::params![batch_id, single],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(format!("Failed to read {single} in {batch_id}: {other}")),
            })?;
        match row {
            Some((state, dependency)) if state == "failed" => {
                targets.push((single.to_string(), dependency))
            }
            Some((state, _)) => {
                return Err(format!(
                    "invalid_transition: {single} is {state}, not failed"
                ))
            }
            None => {
                return Err(format!(
                    "invalid_selection: {single} is not part of {batch_id}"
                ))
            }
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT t.id, l.dependency_task_id
                 FROM processing_tasks t
                 JOIN processing_batch_tasks l ON l.task_id = t.id
                 WHERE l.batch_id = ?1 AND t.state = 'failed'",
            )
            .map_err(|e| format!("Failed to list failed units of {batch_id}: {e}"))?;
        targets = stmt
            .query_map([batch_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("Failed to list failed units of {batch_id}: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to list failed units of {batch_id}: {e}"))?;
    }
    // Pull in the failed dependencies of the selected units (one level).
    let mut extra = Vec::new();
    for (_, dependency) in &targets {
        if let Some(dep) = dependency {
            let dep_state: Option<String> = conn
                .query_row(
                    "SELECT state FROM processing_tasks WHERE id = ?1",
                    [dep],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(format!("Failed to read dependency {dep}: {other}")),
                })?;
            if dep_state.as_deref() == Some("failed") {
                extra.push((dep.clone(), None));
            }
        }
    }
    targets.extend(extra);
    // The batch keeps observing retried units.
    for (id, _) in &targets {
        conn.execute(
            "UPDATE processing_batch_tasks SET request_state = 'active'
             WHERE batch_id = ?1 AND task_id = ?2",
            rusqlite::params![batch_id, id],
        )
        .map_err(|e| format!("Failed to reactivate {id} in {batch_id}: {e}"))?;
    }
    let mut reopened = 0;
    for (id, _) in &targets {
        let changed = conn
            .execute(
                "UPDATE processing_tasks SET state = 'pending', outcome = '', retry_cycle = retry_cycle + 1,
                   retry_count = 0, source_invalidation_count = 0, next_retry_at = NULL,
                   last_error_code = NULL, last_error_message = NULL,
                   owner_session = NULL, updated_at = strftime('%s', 'now') * 1000
                 WHERE id = ?1 AND state = 'failed'",
                [id],
            )
            .map_err(|e| format!("Failed to reopen {id}: {e}"))?;
        reopened += changed as usize;
    }
    conn.execute(
        "UPDATE processing_batches SET revision = revision + 1, updated_at = strftime('%s', 'now') * 1000,
           state = CASE WHEN state = 'completed_with_errors' THEN 'ready' ELSE state END,
           desired_state = CASE WHEN state = 'completed_with_errors' THEN 'run' ELSE desired_state END,
           finished_at = NULL WHERE id = ?1",
        [batch_id],
    )
    .map_err(|e| format!("Failed to bump revision of {batch_id}: {e}"))?;
    Ok(reopened)
}

/// Parks queued units whose pinned contract no longer matches the effective
/// one. Configuration drift blocks with `configuration_changed` — it never
/// fails units, and running units keep their pinned contract until commit,
/// which re-checks (see `commit_success_with`).
pub fn reconcile_contracts(conn: &Connection) -> Result<usize, String> {
    let current = super::eligibility::current_embedding_contract_hash();
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET state = 'blocked', outcome = 'configuration_changed',
               last_error_code = 'configuration_changed',
               last_error_message = 'the effective embedding contract changed while this task waited; resume with the current configuration to re-evaluate',
               updated_at = strftime('%s', 'now') * 1000
             WHERE kind = 'embedding' AND state IN ('pending', 'retry_wait') AND contract_hash != ?1 AND contract_hash != ('force:' || ?1)",
            [current],
        )
        .map_err(|e| format!("Failed to reconcile contracts: {e}"))?;
    Ok(changed as usize)
}

/// Advances preparation by up to `max_pages` classification pages, then
/// promotes batches whose snapshot is complete: `preparing` with a running
/// demand becomes `ready`, and `ready` with completed planning becomes
/// `running` (recording `started_at`). Returns true when this batch needs no
/// more planning.
pub fn advance_planning(
    conn: &Connection,
    batch_id: &str,
    max_pages: usize,
    page_size: usize,
) -> Result<bool, String> {
    for _ in 0..max_pages.max(1) {
        if classify_batch_page(conn, batch_id, page_size)?.done {
            break;
        }
    }
    promote_batches(conn)?;
    let done: i64 = conn
        .query_row(
            "SELECT planning_done FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read planning of {batch_id}: {e}"))?;
    Ok(done == 1)
}

fn promote_batches(conn: &Connection) -> Result<(), String> {
    promote_prepared_batches(conn)?;
    promote_ready_batches(conn)?;
    observe_paused_batches(conn)?;
    Ok(())
}

fn promote_prepared_batches(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE processing_batches SET state = 'ready', updated_at = strftime('%s', 'now') * 1000
         WHERE state = 'preparing' AND planning_done = 1 AND desired_state != 'cancel'",
        [],
    )
    .map_err(|e| format!("Failed to promote ready batches: {e}"))?;
    Ok(())
}

/// Moves planned batches into execution. The tick calls this on every pass
/// (not only while planning advances) so a resumed `ready` batch reaches
/// `running` without waiting for new planning work.
pub fn promote_ready_batches(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE processing_batches SET state = 'running', started_at = COALESCE(started_at, strftime('%s', 'now') * 1000),
           updated_at = strftime('%s', 'now') * 1000
         WHERE state = 'ready' AND planning_done = 1 AND desired_state = 'run'",
        [],
    )
    .map_err(|e| format!("Failed to promote running batches: {e}"))?;
    Ok(())
}

fn observe_paused_batches(conn: &Connection) -> Result<(), String> {
    // A paused batch whose units all drained is observed as paused.
    conn.execute(
        "UPDATE processing_batches SET state = 'paused', updated_at = strftime('%s', 'now') * 1000
         WHERE state = 'pausing' AND desired_state = 'pause'
           AND NOT EXISTS (
             SELECT 1 FROM processing_batch_tasks l
             JOIN processing_tasks t ON t.id = l.task_id
             WHERE l.batch_id = processing_batches.id AND t.state = 'running')",
        [],
    )
    .map_err(|e| format!("Failed to observe paused batches: {e}"))?;
    Ok(())
}

/// Finalizes batches with nothing left to run: planning complete and no
/// linked unit still pending, blocked, running, waiting, or interrupted.
/// `completed` when every observed unit settled cleanly, `completed_with_errors`
/// when any linked unit failed, `cancelled` when the intent was withdrawn.
/// Returns true when the batch transitioned.
pub fn maybe_finalize_batch(conn: &Connection, batch_id: &str) -> Result<bool, String> {
    let row: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT state, desired_state, planning_done FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read {batch_id}: {other}")),
        })?;
    let Some((state, desired, planning_done)) = row else {
        return Err(format!("invalid_selection: unknown batch {batch_id}"));
    };
    let origin: String = conn
        .query_row(
            "SELECT origin FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read batch origin: {e}"))?;
    if origin != "user" {
        return Ok(false);
    }
    if desired == "pause" {
        observe_paused_batches(conn)?;
        return Ok(false);
    }
    // A cancelled batch never needs a complete snapshot: withdrawing demand
    // is final on its own. Every other path waits for planning to finish so
    // "completed" always covers the whole frozen scope.
    let needs_planning = desired != "cancel" && state != "cancelling";
    if !["running", "pausing", "cancelling", "ready"].contains(&state.as_str())
        || (needs_planning && planning_done != 1)
    {
        return Ok(false);
    }
    let open: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_tasks l
             JOIN processing_tasks t ON t.id = l.task_id
             WHERE l.batch_id = ?1 AND l.request_state != 'cancelled'
               AND t.state IN ('pending', 'blocked', 'running', 'retry_wait', 'interrupted')",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count open units of {batch_id}: {e}"))?;
    if open > 0 {
        return Ok(false);
    }
    let failed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_tasks l
             JOIN processing_tasks t ON t.id = l.task_id
             WHERE l.batch_id = ?1 AND l.request_state != 'cancelled' AND t.state = 'failed'",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count failed units of {batch_id}: {e}"))?;
    let terminal = if desired == "cancel" || state == "cancelling" {
        "cancelled"
    } else if failed > 0 {
        "completed_with_errors"
    } else {
        "completed"
    };
    conn.execute(
        "UPDATE processing_batches SET state = ?1, finished_at = strftime('%s', 'now') * 1000,
           updated_at = strftime('%s', 'now') * 1000 WHERE id = ?2",
        rusqlite::params![terminal, batch_id],
    )
    .map_err(|e| format!("Failed to finalize {batch_id}: {e}"))?;
    Ok(true)
}
/// Durable snapshot of one batch: desired vs. observed state, planning
/// progress, and unit counters derived from the tables — never from events.
#[derive(Debug, Clone)]
pub struct BatchSnapshot {
    pub id: String,
    pub request_id: String,
    pub origin: String,
    pub state: String,
    pub desired_state: String,
    pub operations: Vec<String>,
    pub planning_cursor: i64,
    pub planning_done: bool,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_error: Option<String>,
    pub members_total: i64,
    pub members_classified: i64,
    pub tasks_by_state: Vec<(String, i64)>,
    pub tasks_by_kind: Vec<(String, i64)>,
    pub collections: Vec<(String, String)>,
}

/// Reads one batch snapshot in short consistent reads. Callers display
/// `revision` and send it back as `expected_revision` on control calls.
pub fn read_batch_snapshot(conn: &Connection, batch_id: &str) -> Result<BatchSnapshot, String> {
    let row: Option<(
        String, String, String, String, String, String, i64, i64, i64, i64, i64,
        Option<i64>, Option<i64>, Option<String>,
    )> = conn
        .query_row(
            "SELECT id, request_id, origin, state, desired_state, operations, planning_cursor,
                    planning_done, revision, created_at, updated_at, started_at, finished_at, last_error
             FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read batch {batch_id}: {other}")),
        })?;
    let Some(batch) = row else {
        return Err(format!("invalid_selection: unknown batch {batch_id}"));
    };
    let operations: Vec<String> = serde_json::from_str(&batch.5).unwrap_or_default();
    let members_total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_members WHERE batch_id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count members of {batch_id}: {e}"))?;
    let members_classified: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_members WHERE batch_id = ?1 AND classification != 'unclassified'",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count classified members of {batch_id}: {e}"))?;
    let mut states = conn
        .prepare(
            "SELECT t.state, COUNT(*) FROM processing_batch_tasks l
             JOIN processing_tasks t ON t.id = l.task_id
             WHERE l.batch_id = ?1 GROUP BY t.state ORDER BY t.state",
        )
        .map_err(|e| format!("Failed to count units of {batch_id}: {e}"))?;
    let tasks_by_state = states
        .query_map([batch_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("Failed to count units of {batch_id}: {e}"))?
        .collect::<Result<Vec<(String, i64)>, _>>()
        .map_err(|e| format!("Failed to count units of {batch_id}: {e}"))?;
    let mut kinds = conn
        .prepare(
            "SELECT t.kind, COUNT(*) FROM processing_batch_tasks l
             JOIN processing_tasks t ON t.id = l.task_id
             WHERE l.batch_id = ?1 GROUP BY t.kind ORDER BY t.kind",
        )
        .map_err(|e| format!("Failed to count kinds of {batch_id}: {e}"))?;
    let tasks_by_kind = kinds
        .query_map([batch_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("Failed to count kinds of {batch_id}: {e}"))?
        .collect::<Result<Vec<(String, i64)>, _>>()
        .map_err(|e| format!("Failed to count kinds of {batch_id}: {e}"))?;
    let mut cols = conn
        .prepare(
            "SELECT collection_id_snapshot, name_snapshot FROM processing_batch_collections
             WHERE batch_id = ?1 ORDER BY name_snapshot",
        )
        .map_err(|e| format!("Failed to read collections of {batch_id}: {e}"))?;
    let collections = cols
        .query_map([batch_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("Failed to read collections of {batch_id}: {e}"))?
        .collect::<Result<Vec<(String, String)>, _>>()
        .map_err(|e| format!("Failed to read collections of {batch_id}: {e}"))?;
    Ok(BatchSnapshot {
        id: batch.0,
        request_id: batch.1,
        origin: batch.2,
        state: batch.3,
        desired_state: batch.4,
        operations,
        planning_cursor: batch.6,
        planning_done: batch.7 == 1,
        revision: batch.8,
        created_at: batch.9,
        updated_at: batch.10,
        started_at: batch.11,
        finished_at: batch.12,
        last_error: batch.13,
        members_total,
        members_classified,
        tasks_by_state,
        tasks_by_kind,
        collections,
    })
}

/// One row of the batch history list.
#[derive(Debug, Clone)]
pub struct BatchSummary {
    pub id: String,
    pub state: String,
    pub desired_state: String,
    pub operations: Vec<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub active_units: i64,
    pub failed_units: i64,
    pub succeeded_units: i64,
}

/// Newest-first batch history with keyset pagination over
/// `(created_at, id)`. `limit` clamps to 1..=200; the cursor is the last row
/// of the previous page. Returns the rows plus the cursor for the next page.
pub fn list_batches(
    conn: &Connection,
    states: Option<&[String]>,
    after: Option<(i64, String)>,
    limit: usize,
) -> Result<(Vec<BatchSummary>, Option<(i64, String)>), String> {
    let limit = limit.clamp(1, 200) as i64;
    let state_list = states.map(|list| {
        list.iter()
            .map(|state| format!("'{state}'"))
            .collect::<Vec<_>>()
            .join(", ")
    });
    // States come from our own state machine vocabulary; anything else
    // matches nothing instead of touching SQL structure.
    let allowed = [
        "preparing",
        "ready",
        "running",
        "pausing",
        "paused",
        "cancelling",
        "cancelled",
        "interrupted",
        "completed",
        "completed_with_errors",
    ];
    if let Some(list) = states {
        for state in list {
            if !allowed.contains(&state.as_str()) {
                return Err(format!("invalid_selection: unknown batch state {state}"));
            }
        }
    }
    let mut sql = String::from(
        "SELECT id, state, desired_state, operations, revision, created_at, updated_at FROM processing_batches",
    );
    // The listings are the user's own work. `manual` and `repair` containers
    // are always running by construction and never finalize, so listing them
    // puts a permanent row in "active" that no control can move.
    let mut clauses = vec!["origin = 'user'".to_string()];
    if let Some(list) = state_list {
        if !list.is_empty() {
            clauses.push(format!("state IN ({list})"));
        }
    }
    // Keyset parameters are bound, never interpolated.
    let (after_created, after_id) = after.map(|(c, i)| (c.to_string(), i)).unzip();
    if after_created.is_some() {
        clauses.push("(created_at, id) < (?1, ?2)".to_string());
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?3");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to list batches: {e}"))?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                after_created.unwrap_or_else(|| "99999999999999".to_string()),
                after_id.unwrap_or_default(),
                limit + 1
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|e| format!("Failed to list batches: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to list batches: {e}"))?;
    let has_more = rows.len() > limit as usize;
    let mut summaries = Vec::new();
    for (id, state, desired, operations, revision, created, updated) in
        rows.into_iter().take(limit as usize)
    {
        let operations: Vec<String> = serde_json::from_str(&operations).unwrap_or_default();
        let counts = |task_state: &str| -> Result<i64, String> {
            conn.query_row(
                "SELECT COUNT(*) FROM processing_batch_tasks l
                 JOIN processing_tasks t ON t.id = l.task_id
                 WHERE l.batch_id = ?1 AND t.state = ?2",
                rusqlite::params![id, task_state],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count units of {id}: {e}"))
        };
        let active = ["pending", "blocked", "running", "retry_wait", "interrupted"]
            .iter()
            .map(|s| counts(s))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        let failed_units = counts("failed")?;
        let succeeded_units = counts("succeeded")?;
        summaries.push(BatchSummary {
            id,
            state,
            desired_state: desired,
            operations,
            revision,
            created_at: created,
            updated_at: updated,
            active_units: active,
            failed_units,
            succeeded_units,
        });
    }
    let next = if has_more {
        summaries
            .last()
            .map(|last| (last.created_at, last.id.clone()))
    } else {
        None
    };
    Ok((summaries, next))
}

/// One unit row of a batch detail view. Result payloads and full attempt
/// histories stay behind `read_task_detail` — list pages never haul them.
#[derive(Debug, Clone)]
pub struct TaskSummary {
    pub task_id: String,
    pub kind: String,
    pub asset_id: String,
    pub state: String,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
    pub outcome: String,
    pub attempt_count: i64,
    pub retry_cycle: i64,
    pub next_retry_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub updated_at: i64,
    pub request_state: String,
    pub dependency_task_id: Option<String>,
}

/// Stable `task_id`-ordered unit pages for one batch (default 50, max 200).
pub fn list_tasks(
    conn: &Connection,
    batch_id: &str,
    state_filter: Option<&str>,
    kind_filter: Option<&str>,
    after_task_id: Option<&str>,
    limit: usize,
) -> Result<(Vec<TaskSummary>, Option<String>), String> {
    let limit = limit.clamp(1, 200) as i64;
    if let Some(state) = state_filter {
        if ![
            "pending",
            "blocked",
            "running",
            "retry_wait",
            "interrupted",
            "succeeded",
            "failed",
            "skipped",
            "cancelled",
        ]
        .contains(&state)
        {
            return Err(format!("invalid_selection: unknown task state {state}"));
        }
    }
    if let Some(kind) = kind_filter {
        if kind != "ocr" && kind != "embedding" {
            return Err(format!("invalid_selection: unknown task kind {kind}"));
        }
    }
    let mut sql = String::from(
        "SELECT t.id, t.kind, t.asset_id_snapshot, t.state, t.stage, t.progress_done, t.progress_total,
                t.outcome, t.attempt_count, t.retry_cycle, t.next_retry_at, t.last_error_code,
                t.last_error_message, t.updated_at, l.request_state, l.dependency_task_id
         FROM processing_batch_tasks l JOIN processing_tasks t ON t.id = l.task_id
         WHERE l.batch_id = ?1",
    );
    if state_filter.is_some() {
        sql.push_str(" AND t.state = ?2");
    }
    if kind_filter.is_some() {
        sql.push_str(" AND t.kind = ?3");
    }
    sql.push_str(" AND t.id > ?4 ORDER BY t.id LIMIT ?5");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to list units of {batch_id}: {e}"))?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                batch_id,
                state_filter.unwrap_or(""),
                kind_filter.unwrap_or(""),
                after_task_id.unwrap_or(""),
                limit + 1
            ],
            |row| {
                Ok(TaskSummary {
                    task_id: row.get(0)?,
                    kind: row.get(1)?,
                    asset_id: row.get(2)?,
                    state: row.get(3)?,
                    stage: row.get(4)?,
                    progress_done: row.get(5)?,
                    progress_total: row.get(6)?,
                    outcome: row.get(7)?,
                    attempt_count: row.get(8)?,
                    retry_cycle: row.get(9)?,
                    next_retry_at: row.get(10)?,
                    error_code: row.get(11)?,
                    error_message: row.get(12)?,
                    updated_at: row.get(13)?,
                    request_state: row.get(14)?,
                    dependency_task_id: row.get(15)?,
                })
            },
        )
        .map_err(|e| format!("Failed to list units of {batch_id}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to list units of {batch_id}: {e}"))?;
    let next = rows
        .get(limit as usize)
        .map(|_| rows[(limit as usize) - 1].task_id.clone());
    Ok((rows.into_iter().take(limit as usize).collect(), next))
}

/// Full detail of one unit: summary, checkpoint aggregates, newest-first
/// attempt history, and the batches sharing the physical task.
#[derive(Debug, Clone)]
pub struct TaskAttemptView {
    pub attempt_number: i64,
    pub lease_epoch: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub outcome: String,
    pub retryable: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TaskDetail {
    pub task_id: String,
    pub kind: String,
    pub asset_id: String,
    pub state: String,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
    pub outcome: String,
    pub attempt_count: i64,
    pub retry_cycle: i64,
    pub next_retry_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub checkpoints: Vec<(String, String, i64)>,
    pub attempts: Vec<TaskAttemptView>,
    pub shared_with_batches: Vec<String>,
}

pub fn read_task_detail(
    conn: &Connection,
    batch_id: &str,
    task_id: &str,
    attempt_limit: usize,
) -> Result<TaskDetail, String> {
    let link: Option<(String,)> = conn
        .query_row(
            "SELECT request_state FROM processing_batch_tasks WHERE batch_id = ?1 AND task_id = ?2",
            rusqlite::params![batch_id, task_id],
            |row| Ok((row.get(0)?,)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!(
                "Failed to read link {task_id} in {batch_id}: {other}"
            )),
        })?;
    if link.is_none() {
        return Err(format!(
            "invalid_selection: {task_id} is not part of {batch_id}"
        ));
    }
    let task: Option<(
        String, String, String, String, i64, i64, String, i64, i64, Option<i64>, Option<String>,
        Option<String>, i64,
    )> = conn
        .query_row(
            "SELECT kind, asset_id_snapshot, state, stage, progress_done, progress_total, outcome,
                    attempt_count, retry_cycle, next_retry_at, last_error_code, last_error_message, updated_at
             FROM processing_tasks WHERE id = ?1",
            [task_id],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    row.get(10)?, row.get(11)?, row.get(12)?,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read task {task_id}: {other}")),
        })?;
    let Some(task) = task else {
        return Err(format!("invalid_selection: unknown task {task_id}"));
    };
    let mut cps = conn
        .prepare(
            "SELECT unit_key, payload_checksum, created_at FROM processing_checkpoints
             WHERE task_id = ?1 ORDER BY unit_key",
        )
        .map_err(|e| format!("Failed to read checkpoints of {task_id}: {e}"))?;
    let checkpoints = cps
        .query_map([task_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| format!("Failed to read checkpoints of {task_id}: {e}"))?
        .collect::<Result<Vec<(String, String, i64)>, _>>()
        .map_err(|e| format!("Failed to read checkpoints of {task_id}: {e}"))?;
    let attempt_limit = attempt_limit.clamp(1, 100) as i64;
    let mut ats = conn
        .prepare(
            "SELECT attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message
             FROM processing_attempts WHERE task_id = ?1 ORDER BY attempt_number DESC LIMIT ?2",
        )
        .map_err(|e| format!("Failed to read attempts of {task_id}: {e}"))?;
    let attempts = ats
        .query_map(rusqlite::params![task_id, attempt_limit], |row| {
            Ok(TaskAttemptView {
                attempt_number: row.get(0)?,
                lease_epoch: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                outcome: row.get(4)?,
                retryable: row.get::<_, i64>(5)? == 1,
                error_code: row.get(6)?,
                error_message: row.get(7)?,
            })
        })
        .map_err(|e| format!("Failed to read attempts of {task_id}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read attempts of {task_id}: {e}"))?;
    let mut shared = conn
        .prepare("SELECT batch_id FROM processing_batch_tasks WHERE task_id = ?1 ORDER BY batch_id")
        .map_err(|e| format!("Failed to read sharers of {task_id}: {e}"))?;
    let shared_with_batches = shared
        .query_map([task_id], |row| row.get(0))
        .map_err(|e| format!("Failed to read sharers of {task_id}: {e}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| format!("Failed to read sharers of {task_id}: {e}"))?;
    Ok(TaskDetail {
        task_id: task_id.to_string(),
        kind: task.0,
        asset_id: task.1,
        state: task.2,
        stage: task.3,
        progress_done: task.4,
        progress_total: task.5,
        outcome: task.6,
        attempt_count: task.7,
        retry_cycle: task.8,
        next_retry_at: task.9,
        error_code: task.10,
        error_message: task.11,
        checkpoints,
        attempts,
        shared_with_batches,
    })
}
/// Wall-clock milliseconds. Passed in by callers so tests control time.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Diagnostic lease horizon. Elapsed time alone never grants takeover;
/// recovery requires exclusive OS ownership and increments the fencing epoch.
pub const LEASE_TTL_MS: i64 = 60_000;
const SCHEDULER_HEARTBEAT_KEY: &str = "scheduler_heartbeat";

/// Records this supervisor as alive. Called every tick; a single UPSERT on
/// a dedicated key, never inside a long transaction.
pub fn write_scheduler_heartbeat(
    conn: &Connection,
    session_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    conn.execute(
        &format!(
            "INSERT INTO processing_meta (key, value) VALUES ('{SCHEDULER_HEARTBEAT_KEY}', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value"
        ),
        [format!("{session_id}|{now_ms}")],
    )
    .map_err(|e| format!("Failed to write scheduler heartbeat: {e}"))?;
    Ok(())
}

/// Retry delays within one cycle (1 initial + 2 retries): 5 s, then 30 s,
/// each with ±20% jitter applied by the caller. Pure function, pinned by test.
pub fn retry_delay_ms(retry_count_in_cycle: i64) -> i64 {
    match retry_count_in_cycle {
        0 => 5_000,
        1 => 30_000,
        _ => 30_000,
    }
}

fn retry_delay_with_jitter_ms(task_id: &str, attempt_number: i64, retry_count: i64) -> i64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in task_id
        .as_bytes()
        .iter()
        .copied()
        .chain(attempt_number.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let permille = (hash % 401) as i64 - 200;
    let base = retry_delay_ms(retry_count);
    base + (base * permille / 1_000)
}

fn provider_retry_after_ms(message: &str) -> Option<i64> {
    let (_, suffix) = message.split_once("[retry_after_ms=")?;
    let raw = suffix.split_once(']')?.0;
    raw.parse::<i64>().ok().filter(|delay| *delay >= 0)
}

/// Maximum attempts per retry cycle before a task goes terminally failed.
pub const MAX_ATTEMPTS_PER_CYCLE: i64 = 3;

/// A task under exclusive ownership of one supervisor thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedTask {
    pub task_id: String,
    pub kind: String,
    pub asset_id: String,
    pub input_revision: i64,
    pub input_fingerprint: String,
    pub contract_hash: String,
    pub lease_epoch: i64,
    pub attempt_number: i64,
}

/// Moves `blocked` tasks whose dependency succeeded back to `pending`, and
/// fails the ones whose dependency died terminally without another live
/// path to resolve them. Runs inside the claim transaction and before every
/// claim scan, so no worker ever starts a task that cannot finish.
pub fn settle_blocked_dependents(conn: &Connection) -> Result<usize, String> {
    let rows = conn
        .prepare(
            "SELECT l.task_id, l.dependency_task_id, d.state
             FROM processing_batch_tasks l
             JOIN processing_tasks t ON t.id = l.task_id
             JOIN processing_tasks d ON d.id = l.dependency_task_id
             WHERE t.state = 'blocked' AND l.dependency_task_id IS NOT NULL",
        )
        .map_err(|e| format!("Failed to scan blocked dependents: {e}"))?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("Failed to scan blocked dependents: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to scan blocked dependents: {e}"))?;
    let mut settled = 0;
    for (task_id, dep_id, dep_state) in rows {
        if dep_state == "succeeded" {
            conn.execute(
                "UPDATE processing_tasks SET state = 'pending', stage = '', updated_at = strftime('%s', 'now') * 1000
                 WHERE id = ?1 AND state = 'blocked'",
                [&task_id],
            )
            .map_err(|e| format!("Failed to unblock {task_id}: {e}"))?;
            settled += 1;
        } else if dep_state == "failed" || dep_state == "cancelled" {
            // No other link can resolve this unit: the dependency row is the
            // single writer for its operation+asset.
            conn.execute(
                "UPDATE processing_tasks SET state = 'failed', outcome = 'dependency_failed',
                   last_error_code = 'dependency_failed',
                   last_error_message = ?2, updated_at = strftime('%s', 'now') * 1000
                 WHERE id = ?1 AND state = 'blocked'",
                rusqlite::params![task_id, format!("dependency {dep_id} ended as {dep_state}")],
            )
            .map_err(|e| format!("Failed to fail blocked {task_id}: {e}"))?;
            close_open_attempt(conn, &task_id, "failed")?;
            settled += 1;
        }
    }
    Ok(settled)
}

fn close_open_attempt(conn: &Connection, task_id: &str, outcome: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE processing_attempts SET outcome = ?1, finished_at = strftime('%s', 'now') * 1000
         WHERE task_id = ?2 AND outcome = 'open'",
        rusqlite::params![outcome, task_id],
    )
    .map_err(|e| format!("Failed to close attempt of {task_id}: {e}"))?;
    Ok(())
}

/// Claims the next runnable task for `session_id`: BEGIN IMMEDIATE, settle
/// dependents, pick the oldest runnable unit with an actively-wanted batch,
/// revalidate its input (admission data may be stale), CAS it to `running`
/// with a fresh fencing epoch, open an attempt, COMMIT — all before any
/// compute starts. Returns `None` when no unit is runnable. `kinds` lists
/// the operations this supervisor can execute; anything else stays queued.
pub fn claim_next(
    conn: &Connection,
    session_id: &str,
    kinds: &[&str],
    now_ms: i64,
) -> Result<Option<ClaimedTask>, String> {
    if kinds.is_empty() {
        return Ok(None);
    }
    for kind in kinds {
        if *kind != "ocr" && *kind != "embedding" {
            return Err(format!("unknown task kind: {kind}"));
        }
    }
    let kind_list = kinds
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("Failed to begin claim: {e}"))?;
    let claimed = (|| -> Result<Option<ClaimedTask>, String> {
        use rusqlite::OptionalExtension as _;
        settle_blocked_dependents(conn)?;
        let candidate: Option<(String, String, String, String, i64)> = conn
            .query_row(
                &format!(
                    "SELECT t.id, t.kind, t.asset_id_snapshot, t.contract_hash, t.lease_epoch
                 FROM processing_tasks t
                 WHERE t.kind IN ({kind_list})
                   AND (t.state = 'pending'
                        OR (t.state = 'retry_wait' AND t.next_retry_at IS NOT NULL AND t.next_retry_at <= ?1))
                   AND EXISTS (
                         SELECT 1 FROM processing_batch_tasks l
                         JOIN processing_batches b ON b.id = l.batch_id
                         WHERE l.task_id = t.id AND l.request_state = 'active'
                           AND b.state = 'running' AND b.desired_state = 'run')
                   AND NOT EXISTS (
                         SELECT 1 FROM processing_batch_tasks l2
                         WHERE l2.task_id = t.id AND l2.dependency_task_id IS NOT NULL
                           AND (SELECT state FROM processing_tasks d WHERE d.id = l2.dependency_task_id) != 'succeeded')
                 ORDER BY t.id LIMIT 1"
                ),
                [now_ms],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("Failed to scan runnable tasks: {e}"))?;
        let Some((task_id, kind, asset_id, contract_hash, epoch)) = candidate else {
            return Ok(None);
        };
        // The world may have moved between admission and this claim: refresh
        // the pinned input, or skip the unit without ever calling a motor.
        let validated = validate_claim_input(conn, &task_id, &kind, &asset_id, &contract_hash)?;
        let Some((input_revision, input_fingerprint)) = validated else {
            return Ok(None);
        };
        conn.execute(
            "UPDATE processing_tasks SET input_revision = ?1, input_fingerprint = ?2 WHERE id = ?3",
            rusqlite::params![input_revision, input_fingerprint, task_id],
        )
        .map_err(|e| format!("Failed to refresh input of {task_id}: {e}"))?;
        let changed = conn
            .execute(
                "UPDATE processing_tasks
                 SET state = 'running', owner_session = ?1, lease_epoch = lease_epoch + 1,
                     heartbeat_at = ?2, lease_expires_at = ?3, attempt_count = attempt_count + 1,
                     updated_at = ?2
                 WHERE id = ?4 AND state IN ('pending', 'retry_wait') AND lease_epoch = ?5",
                rusqlite::params![session_id, now_ms, now_ms + LEASE_TTL_MS, task_id, epoch],
            )
            .map_err(|e| format!("Failed to claim {task_id}: {e}"))?;
        if changed == 0 {
            // Lost a race with another supervisor: back off, do not compute.
            return Ok(None);
        }
        let attempt_number: i64 = conn
            .query_row(
                "SELECT attempt_count FROM processing_tasks WHERE id = ?1",
                [&task_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to read attempt of {task_id}: {e}"))?;
        conn.execute(
            "INSERT INTO processing_attempts (task_id, attempt_number, lease_epoch, started_at, outcome)
             VALUES (?1, ?2, ?3, ?4, 'open')",
            rusqlite::params![task_id, attempt_number, epoch + 1, now_ms],
        )
        .map_err(|e| format!("Failed to open attempt of {task_id}: {e}"))?;
        Ok(Some(ClaimedTask {
            task_id,
            kind,
            asset_id,
            input_revision,
            input_fingerprint,
            contract_hash,
            lease_epoch: epoch + 1,
            attempt_number,
        }))
    })();
    match claimed {
        Ok(task) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("Failed to commit claim: {e}"))?;
            Ok(task)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Revalidates one candidate inside the claim transaction. Returns the fresh
/// `(revision, fingerprint)` to pin, or `None` after transitioning the task
/// to a terminal-or-blocked state that needs no motor call.
fn validate_claim_input(
    conn: &Connection,
    task_id: &str,
    kind: &str,
    asset_id: &str,
    contract_hash: &str,
) -> Result<Option<(i64, String)>, String> {
    let asset_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM assets WHERE id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to check asset {asset_id}: {e}"))?;
    if asset_exists == 0 {
        // Catalog deletion wins over queued work: never recreate content.
        mark_skipped(conn, task_id, "source_deleted")?;
        return Ok(None);
    }
    if kind == "ocr" {
        let manual: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM processing_batch_tasks l JOIN processing_batches b ON b.id=l.batch_id WHERE l.task_id=?1 AND l.request_state='active' AND b.origin='manual')",
            [task_id], |row| row.get(0),
        ).map_err(|e| format!("Failed to read OCR intent: {e}"))?;
        if !manual
            && matches!(
                super::eligibility::ocr_decision(conn, asset_id)?,
                super::eligibility::OcrDecision::AlreadyDone { .. }
            )
        {
            mark_skipped(conn, task_id, "already_satisfied")?;
            return Ok(None);
        }
        let fingerprint = ocr_fingerprint(conn, asset_id)?.ok_or_else(|| {
            format!("Failed to fingerprint {asset_id}: asset row vanished mid-claim")
        })?;
        return Ok(Some((source_revision(conn, asset_id)?, fingerprint)));
    }
    let force = contract_hash.starts_with("force:");
    let contract_hash = contract_hash
        .strip_prefix("force:")
        .unwrap_or(contract_hash);
    let decision = super::eligibility::embedding_decision(conn, asset_id)?;
    if force
        && !matches!(
            decision,
            super::eligibility::EmbeddingDecision::NoSourceText
        )
        && contract_hash == super::eligibility::current_embedding_contract_hash()
    {
        return Ok(Some((
            source_revision(conn, asset_id)?,
            super::eligibility::embedding_input_fingerprint(conn, asset_id)?,
        )));
    }
    match decision {
        super::eligibility::EmbeddingDecision::Fresh => {
            // Another path satisfied the input while this task waited.
            mark_skipped(conn, task_id, "already_satisfied")?;
            Ok(None)
        }
        super::eligibility::EmbeddingDecision::NoSourceText => {
            mark_skipped(conn, task_id, "no_source_text")?;
            Ok(None)
        }
        super::eligibility::EmbeddingDecision::Eligible { .. } => {
            if contract_hash != super::eligibility::current_embedding_contract_hash() {
                mark_blocked(conn, task_id, "configuration_changed",
                    "the effective embedding contract changed while this task waited; resume with the current configuration to re-evaluate")?;
                return Ok(None);
            }
            let fingerprint = super::eligibility::embedding_input_fingerprint(conn, asset_id)?;
            Ok(Some((source_revision(conn, asset_id)?, fingerprint)))
        }
    }
}

fn mark_skipped(conn: &Connection, task_id: &str, outcome: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE processing_tasks SET state = 'skipped', outcome = ?1, owner_session = NULL,
           next_retry_at = NULL, updated_at = strftime('%s', 'now') * 1000
         WHERE id = ?2",
        rusqlite::params![outcome, task_id],
    )
    .map_err(|e| format!("Failed to skip {task_id}: {e}"))?;
    Ok(())
}

fn mark_blocked(
    conn: &Connection,
    task_id: &str,
    outcome: &str,
    message: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE processing_tasks SET state = 'blocked', outcome = ?1, last_error_code = ?1,
           last_error_message = ?2, owner_session = NULL, next_retry_at = NULL,
           updated_at = strftime('%s', 'now') * 1000
         WHERE id = ?3",
        rusqlite::params![outcome, message, task_id],
    )
    .map_err(|e| format!("Failed to block {task_id}: {e}"))?;
    Ok(())
}

/// Parks a running task whose pinned contract no longer matches the
/// effective one (or whose demand vanished mid-flight): the attempt closes
/// as interrupted, checkpoints survive, and a later resume re-evaluates.
pub fn block_running_task(
    conn: &Connection,
    task_id: &str,
    lease_epoch: i64,
    outcome: &str,
    message: &str,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET state = 'blocked', outcome = ?1, last_error_code = ?1,
               last_error_message = ?2, owner_session = NULL,
               updated_at = strftime('%s', 'now') * 1000
             WHERE id = ?3 AND state = 'running' AND lease_epoch = ?4",
            rusqlite::params![outcome, message, task_id, lease_epoch],
        )
        .map_err(|e| format!("Failed to block running {task_id}: {e}"))?;
    if changed == 0 {
        return Err(format!(
            "lease_lost: {task_id} is no longer owned by epoch {lease_epoch}"
        ));
    }
    close_open_attempt(conn, task_id, "interrupted")?;
    Ok(())
}

/// A validated, complete output unit: one PDF page, one chunk set, one
/// vector. Partial tokens or unflushed buffers must never reach this call —
/// only units whose COMMIT-equivalent already happened upstream.
#[derive(Debug, Clone)]
pub struct NewCheckpoint {
    pub unit_key: String,
    pub input_fingerprint: String,
    pub contract_hash: String,
    pub payload: String,
    pub payload_checksum: String,
}

/// Persists one checkpoint under the caller's fencing epoch. A supervisor
/// whose lease was revoked (recovery, cancel, newer claim) gets zero rows
/// updated and must stop: its writes belong to a dead attempt.
pub fn save_checkpoint(
    conn: &Connection,
    task_id: &str,
    lease_epoch: i64,
    checkpoint: &NewCheckpoint,
    now_ms: i64,
) -> Result<(), String> {
    conn.execute_batch("SAVEPOINT processing_checkpoint")
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let changed = conn
        .execute(
            "UPDATE processing_tasks SET heartbeat_at = ?1, lease_expires_at = ?2, updated_at = ?1
             WHERE id = ?3 AND state = 'running' AND lease_epoch = ?4",
            rusqlite::params![now_ms, now_ms + LEASE_TTL_MS, task_id, lease_epoch],
        )
        .map_err(|e| format!("Failed to fence checkpoint on {task_id}: {e}"))?;
        if changed == 0 {
            return Err(format!(
                "lease_lost: {task_id} is no longer owned by epoch {lease_epoch}"
            ));
        }
        conn.execute(
        "INSERT INTO processing_checkpoints
           (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(task_id, unit_key) DO UPDATE SET
           input_fingerprint = excluded.input_fingerprint,
           contract_hash = excluded.contract_hash,
           payload = excluded.payload,
           payload_checksum = excluded.payload_checksum,
           created_at = excluded.created_at",
        rusqlite::params![
            task_id,
            checkpoint.unit_key,
            checkpoint.input_fingerprint,
            checkpoint.contract_hash,
            checkpoint.payload,
            checkpoint.payload_checksum,
            now_ms
        ],
    )
    .map_err(|e| format!("Failed to save checkpoint on {task_id}: {e}"))?;
        conn.execute(
            "UPDATE processing_tasks SET progress_done =
           (SELECT COUNT(*) FROM processing_checkpoints c WHERE c.task_id = ?1
            AND c.input_fingerprint = processing_tasks.input_fingerprint
            AND c.contract_hash = processing_tasks.contract_hash)
         WHERE id = ?1",
            [task_id],
        )
        .map_err(|e| format!("Failed to advance progress of {task_id}: {e}"))?;
        Ok(())
    })();
    match result {
        Ok(()) => conn
            .execute_batch("RELEASE processing_checkpoint")
            .map_err(|e| e.to_string()),
        Err(error) => {
            let _ = conn
                .execute_batch("ROLLBACK TO processing_checkpoint; RELEASE processing_checkpoint");
            Err(error)
        }
    }
}

/// Declares how many units the task holds in total (pages, chunks). Called
/// once the manifest is known; `progress_done` only ever comes from saved
/// checkpoints, never from in-flight provider progress.
pub fn set_progress_total(
    conn: &Connection,
    task_id: &str,
    lease_epoch: i64,
    total: i64,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET progress_total = ?1 WHERE id = ?2 AND state = 'running' AND lease_epoch = ?3",
            rusqlite::params![total, task_id, lease_epoch],
        )
        .map_err(|e| format!("Failed to set progress of {task_id}: {e}"))?;
    if changed == 0 {
        return Err(format!(
            "lease_lost: {task_id} is no longer owned by epoch {lease_epoch}"
        ));
    }
    Ok(())
}

/// Refreshes the liveness mark on every task this supervisor owns. The
/// waiting loop calls it between units; executors never touch it.
pub fn heartbeat_owned(conn: &Connection, session_id: &str, now_ms: i64) -> Result<usize, String> {
    let changed = conn
        .execute(
            "UPDATE processing_tasks SET heartbeat_at = ?1, lease_expires_at = ?2
             WHERE owner_session = ?3 AND state = 'running'",
            rusqlite::params![now_ms, now_ms + LEASE_TTL_MS, session_id],
        )
        .map_err(|e| format!("Failed to heartbeat {session_id}: {e}"))?;
    Ok(changed as usize)
}

/// True while at least one batch still wants this task executed. The
/// supervisor checks it between units and after every blocking call: a task
/// nobody wants anymore stops at the next checkpoint boundary instead of
/// burning provider budget.
pub fn execution_wanted(conn: &Connection, task_id: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM processing_batch_tasks l
             JOIN processing_batches b ON b.id = l.batch_id
             WHERE l.task_id = ?1 AND l.request_state = 'active'
               AND b.state IN ('running', 'ready') AND b.desired_state = 'run'",
            [task_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to check demand for {task_id}: {e}"))?;
    Ok(count > 0)
}

/// Final publish of one task: validates lease, revision, contract, and
/// demand, runs the engine-specific `publish` (canonical rows + receipt),
/// marks `succeeded`, closes the attempt, stamps the completed revision for
/// embeddings, unblocks dependents, and bumps the batch revisions — all in
/// ONE transaction on this connection. Nothing may COMMIT inside `publish`.
pub fn commit_success_with(
    conn: &Connection,
    task_id: &str,
    lease_epoch: i64,
    kind: &str,
    outcome: &str,
    receipt_json: &str,
    publish: impl FnOnce(&Connection) -> Result<(), String>,
) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("Failed to begin commit of {task_id}: {e}"))?;
    let committed = (|| -> Result<(), String> {
        use rusqlite::OptionalExtension as _;
        let row: Option<(String, i64, String, String)> = conn
            .query_row(
                "SELECT state, input_revision, input_fingerprint, asset_id_snapshot
                 FROM processing_tasks WHERE id = ?1 AND lease_epoch = ?2",
                rusqlite::params![task_id, lease_epoch],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("Failed to read {task_id} for commit: {e}"))?;
        let Some((state, input_revision, input_fingerprint, asset_id)) = row else {
            return Err(format!(
                "lease_lost: {task_id} has no row for epoch {lease_epoch}"
            ));
        };
        if state != "running" {
            return Err(format!("lease_lost: {task_id} is {state}, not running"));
        }
        if !execution_wanted(conn, task_id)? {
            return Err(format!(
                "demand_lost: {task_id} is no longer wanted by any batch"
            ));
        }
        // The source must not have moved under the computation: revision
        // first (every text write bumps it in the same transaction), then
        // the pinned fingerprint for anything the revision cannot see.
        let current_revision = source_revision(conn, &asset_id)?;
        if current_revision != input_revision {
            return Err(format!(
                "source_changed: {asset_id} moved from revision {input_revision} to {current_revision}"
            ));
        }
        if kind == "embedding" {
            let pinned: String = conn
                .query_row(
                    "SELECT contract_hash FROM processing_tasks WHERE id=?1",
                    [task_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if pinned.strip_prefix("force:").unwrap_or(&pinned)
                != super::eligibility::current_embedding_contract_hash()
            {
                return Err("configuration_changed: embedding contract changed".to_string());
            }
            let current = super::eligibility::embedding_input_fingerprint(conn, &asset_id)?;
            if current != input_fingerprint {
                return Err(format!(
                    "source_changed: input set of {asset_id} moved mid-computation"
                ));
            }
        } else if kind == "ocr" {
            let current = ocr_fingerprint(conn, &asset_id)?
                .ok_or_else(|| format!("source_deleted: {asset_id} vanished mid-computation"))?;
            if current != input_fingerprint {
                return Err(format!(
                    "source_changed: file identity of {asset_id} moved mid-computation"
                ));
            }
        }
        publish(conn)?;
        conn.execute(
            "UPDATE processing_tasks SET state = 'succeeded', outcome = ?1, result_receipt_json = ?2,
               last_error_code = NULL, last_error_message = NULL, owner_session = NULL,
               updated_at = strftime('%s', 'now') * 1000
             WHERE id = ?3",
            rusqlite::params![outcome, receipt_json, task_id],
        )
        .map_err(|e| format!("Failed to mark {task_id} succeeded: {e}"))?;
        if kind == "embedding" {
            conn.execute(
                "UPDATE processing_asset_revisions SET embedding_completed_revision = ?1 WHERE asset_id = ?2",
                rusqlite::params![input_revision, asset_id],
            )
            .map_err(|e| format!("Failed to stamp completion of {asset_id}: {e}"))?;
        }
        close_open_attempt(conn, task_id, "succeeded")?;
        settle_blocked_dependents(conn)?;
        conn.execute(
            "UPDATE processing_batches SET revision = revision + 1, updated_at = strftime('%s', 'now') * 1000
             WHERE id IN (SELECT batch_id FROM processing_batch_tasks WHERE task_id = ?1)",
            [task_id],
        )
        .map_err(|e| format!("Failed to bump revisions for {task_id}: {e}"))?;
        Ok(())
    })();
    match committed {
        Ok(()) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("Failed to commit success of {task_id}: {e}"))?;
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Outcome of closing a failed attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailOutcome {
    RetryWait { next_retry_at: i64 },
    Failed,
}

/// Closes one attempt as failed. Transient errors within budget park the
/// task in `retry_wait` with a persisted wake-up time (the slot is freed —
/// nobody sleeps holding a worker); anything else, or the third strike in
/// a cycle, fails the task terminally with its code and message preserved.
pub fn fail_attempt(
    conn: &Connection,
    task_id: &str,
    lease_epoch: i64,
    attempt_number: i64,
    error_code: &str,
    error_message: &str,
    retryable: bool,
    provider_request_id: Option<&str>,
    now_ms: i64,
) -> Result<FailOutcome, String> {
    let row: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT state, lease_epoch, retry_count FROM processing_tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read {task_id} for failure: {other}")),
        })?;
    let Some((state, epoch, retry_count)) = row else {
        return Err(format!("lease_lost: {task_id} vanished mid-attempt"));
    };
    if state != "running" || epoch != lease_epoch {
        return Err(format!("lease_lost: {task_id} is {state} at epoch {epoch}"));
    }
    conn.execute(
        "UPDATE processing_attempts SET outcome = 'failed', finished_at = ?1, retryable = ?2,
           error_code = ?3, error_message = ?4, provider_request_id = ?5
         WHERE task_id = ?6 AND attempt_number = ?7",
        rusqlite::params![
            now_ms,
            if retryable { 1 } else { 0 },
            error_code,
            error_message,
            provider_request_id,
            task_id,
            attempt_number
        ],
    )
    .map_err(|e| format!("Failed to record failure of {task_id}: {e}"))?;
    if retryable && retry_count + 1 < MAX_ATTEMPTS_PER_CYCLE {
        let local_delay = retry_delay_with_jitter_ms(task_id, attempt_number, retry_count);
        let delay = provider_retry_after_ms(error_message)
            .map(|provider| provider.max(local_delay))
            .unwrap_or(local_delay);
        let next_retry_at = now_ms.saturating_add(delay);
        conn.execute(
            "UPDATE processing_tasks SET state = 'retry_wait', retry_count = retry_count + 1,
               next_retry_at = ?1, last_error_code = ?2, last_error_message = ?3,
               owner_session = NULL, updated_at = ?4
             WHERE id = ?5",
            rusqlite::params![next_retry_at, error_code, error_message, now_ms, task_id],
        )
        .map_err(|e| format!("Failed to park {task_id} in retry_wait: {e}"))?;
        return Ok(FailOutcome::RetryWait { next_retry_at });
    }
    conn.execute(
        "UPDATE processing_tasks SET state = 'failed', last_error_code = ?1, last_error_message = ?2,
           owner_session = NULL, next_retry_at = NULL, updated_at = ?3
         WHERE id = ?4",
        rusqlite::params![error_code, error_message, now_ms, task_id],
    )
    .map_err(|e| format!("Failed to fail {task_id}: {e}"))?;
    settle_blocked_dependents(conn)?;
    Ok(FailOutcome::Failed)
}

/// One classified member: what the batch will do about it, if anything.
#[derive(Debug, Clone)]
struct MemberWork {
    ordinal: i64,
    asset_id: String,
    classification: &'static str,
    reason: String,
    ocr_task: Option<(String, String, String)>,
    emb_task: Option<(String, String, Option<String>)>,
}

/// Progress of one classification page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifyPageOutcome {
    pub processed: usize,
    pub admitted: usize,
    /// True when no members remain unclassified.
    pub done: bool,
}

/// Classifies the next page of snapshot members and admits the needed tasks.
/// Read phase (decisions + fingerprints) runs outside the write transaction;
/// the page then commits atomically — member rows, task rows, links, cursor —
/// so a crash replays the page without duplicates. `page_size` 200 matches
/// the plan; callers repeat until `done`.
#[allow(dead_code)]
pub fn classify_batch_page(
    conn: &Connection,
    batch_id: &str,
    page_size: usize,
) -> Result<ClassifyPageOutcome, String> {
    let ops = batch_operations(conn, batch_id)?;
    let cursor: i64 = conn
        .query_row(
            "SELECT planning_cursor FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read cursor of batch {batch_id}: {e}"))?;
    let planning_done: i64 = conn
        .query_row(
            "SELECT planning_done FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read planning state of batch {batch_id}: {e}"))?;
    if planning_done == 1 {
        return Ok(ClassifyPageOutcome {
            processed: 0,
            admitted: 0,
            done: true,
        });
    }
    let mut stmt = conn
        .prepare(
            "SELECT ordinal, asset_id_snapshot FROM processing_batch_members
             WHERE batch_id = ?1 AND ordinal >= ?2 ORDER BY ordinal LIMIT ?3",
        )
        .map_err(|e| format!("Failed to read members of batch {batch_id}: {e}"))?;
    let page = stmt
        .query_map(
            rusqlite::params![batch_id, cursor, page_size as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|e| format!("Failed to read members of batch {batch_id}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read members of batch {batch_id}: {e}"))?;
    drop(stmt);
    if page.is_empty() {
        conn.execute(
            "UPDATE processing_batches SET planning_done = 1, updated_at = strftime('%s', 'now') * 1000 WHERE id = ?1",
            [batch_id],
        )
        .map_err(|e| format!("Failed to close planning of batch {batch_id}: {e}"))?;
        return Ok(ClassifyPageOutcome {
            processed: 0,
            admitted: 0,
            done: true,
        });
    }
    // Read phase: eligibility verdicts for every member, no write lock held.
    // The OCR mode pins once per page from the saved setting (frozen scope).
    let ocr_mode = super::ocr::resolve_batch_ocr_mode(conn);
    let mut works = Vec::with_capacity(page.len());
    for (ordinal, asset_id) in &page {
        works.push(plan_member(conn, *ordinal, asset_id, &ops, &ocr_mode)?);
    }
    // Write phase: one atomic page.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("Failed to begin classification page: {e}"))?;
    let outcome = (|| -> Result<ClassifyPageOutcome, String> {
        let mut admitted = 0;
        let mut last_ordinal = cursor;
        for work in &works {
            let mut ocr_id = None;
            if let Some((kind, fingerprint, contract)) = &work.ocr_task {
                let revision = source_revision(conn, &work.asset_id)?;
                let out = admit_or_attach(
                    conn,
                    batch_id,
                    kind,
                    &work.asset_id,
                    revision,
                    fingerprint,
                    contract,
                    None,
                )?;
                ocr_id = Some(out.task_id.clone());
                if out.created {
                    admitted += 1;
                }
            }
            if let Some((kind, fingerprint, dependency)) = &work.emb_task {
                let revision = source_revision(conn, &work.asset_id)?;
                let out = admit_or_attach(
                    conn,
                    batch_id,
                    kind,
                    &work.asset_id,
                    revision,
                    fingerprint,
                    &super::eligibility::current_embedding_contract_hash(),
                    if dependency.is_some() {
                        ocr_id.as_deref()
                    } else {
                        None
                    },
                )?;
                if out.created {
                    admitted += 1;
                }
            }
            conn.execute(
                "UPDATE processing_batch_members SET classification = ?1, reason = ?2
                 WHERE batch_id = ?3 AND ordinal = ?4",
                rusqlite::params![work.classification, work.reason, batch_id, work.ordinal],
            )
            .map_err(|e| format!("Failed to classify member {}: {e}", work.ordinal))?;
            last_ordinal = work.ordinal;
        }
        conn.execute(
            "UPDATE processing_batches SET planning_cursor = ?1, updated_at = strftime('%s', 'now') * 1000 WHERE id = ?2",
            rusqlite::params![last_ordinal + 1, batch_id],
        )
        .map_err(|e| format!("Failed to advance cursor of batch {batch_id}: {e}"))?;
        Ok(ClassifyPageOutcome {
            processed: works.len(),
            admitted,
            done: false,
        })
    })();
    match outcome {
        Ok(done) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("Failed to commit classification page: {e}"))?;
            Ok(done)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Decides one member: eligibility per selected operation, task admissions to
/// issue, and the durable classification for the batch preview.
fn plan_member(
    conn: &Connection,
    ordinal: i64,
    asset_id: &str,
    ops: &BatchOperations,
    ocr_mode: &str,
) -> Result<MemberWork, String> {
    let mut ocr_task = None;
    let mut emb_task = None;
    let mut notes: Vec<String> = Vec::new();
    // OCR first: an eligible OCR task becomes the embedding dependency below.
    let mut ocr_open: Option<String> = None;
    if ops.ocr {
        match super::eligibility::ocr_decision(conn, asset_id)? {
            super::eligibility::OcrDecision::Eligible => match ocr_fingerprint(conn, asset_id)? {
                Some(fingerprint) => {
                    let contract = super::ocr::ocr_task_contract(ocr_mode);
                    ocr_task = Some(("ocr".to_string(), fingerprint, contract));
                    ocr_open = Some(String::new());
                    notes.push("ocr:admit".to_string());
                }
                None => notes.push("ocr:source_missing".to_string()),
            },
            super::eligibility::OcrDecision::AlreadyDone { method } => {
                notes.push(format!("ocr:already_done({method})"));
            }
            super::eligibility::OcrDecision::UnsupportedType { asset_type } => {
                notes.push(format!("ocr:unsupported({asset_type})"));
            }
            super::eligibility::OcrDecision::ParentHasPages { page_count } => {
                notes.push(format!("ocr:parent_has_pages({page_count})"));
            }
        }
    }
    if ops.embeddings {
        match super::eligibility::embedding_decision(conn, asset_id)? {
            super::eligibility::EmbeddingDecision::Eligible { reason } => {
                // Sources are non-empty here (empty text reports NoSourceText
                // below), so the fingerprint pins the real input set.
                let fingerprint = super::eligibility::embedding_input_fingerprint(conn, asset_id)?;
                let _ = reason;
                emb_task = Some(("embedding".to_string(), fingerprint, None));
                notes.push("embeddings:admit".to_string());
            }
            super::eligibility::EmbeddingDecision::Fresh => {
                notes.push("embeddings:fresh".to_string());
            }
            super::eligibility::EmbeddingDecision::NoSourceText => {
                if ocr_open.is_some() {
                    emb_task = Some(("embedding".to_string(), String::new(), ocr_open.clone()));
                    notes.push("embeddings:wait_for_ocr".to_string());
                } else {
                    notes.push("embeddings:no_source_text".to_string());
                }
            }
        }
    }
    let classification = if ocr_task.is_some() || emb_task.is_some() {
        "admitted"
    } else if notes.iter().any(|n| n.starts_with("ocr:unsupported")) {
        "unsupported_type"
    } else if notes.iter().any(|n| n.starts_with("ocr:parent_has_pages")) {
        "parent_has_pages"
    } else if notes
        .iter()
        .any(|n| n.starts_with("embeddings:no_source_text"))
    {
        "no_source_text"
    } else {
        "already_done"
    };
    Ok(MemberWork {
        ordinal,
        asset_id: asset_id.to_string(),
        classification,
        reason: notes.join("; "),
        ocr_task,
        emb_task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mirrored migration file, exercised here so registry/file drift
    /// breaks a test instead of reaching a user database.
    const MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0032_batch_processing.sql");
    const MIGRATION_0033_SQL: &str = include_str!(
        "../../../../../packages/store/src/migrations/0033_processing_source_invalidation.sql"
    );
    const MIGRATION_0033_NAME: &str = "0033_processing_source_invalidation";

    fn migrated_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        let conn = crate::db::open::open_archive_connection(&db_path).expect("open");
        conn.execute_batch("CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
            .expect("minimal collections");
        conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
            .expect("minimal items");
        conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, sort_index INTEGER NOT NULL DEFAULT 0, size INTEGER, parent_asset_id TEXT REFERENCES assets(id) ON DELETE CASCADE, page_number INTEGER, created_at INTEGER NOT NULL);")
            .expect("minimal assets");
        conn.execute_batch("CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);")
            .expect("minimal extractions");
        conn.execute_batch("CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, language TEXT, duration_ms INTEGER, model TEXT NOT NULL, segments TEXT, confidence REAL, created_at INTEGER NOT NULL);")
            .expect("minimal transcriptions");
        conn.execute_batch("CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);")
            .expect("migrations tracking");
        // Same single-batch application the frontend runner uses for 0032,
        // then the additive 0033 counter column.
        conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
            .expect("apply 0032 mirror");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [MIGRATION_NAME],
        )
        .expect("track 0032");
        conn.execute_batch(MIGRATION_0033_SQL)
            .expect("apply 0033 mirror");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [MIGRATION_0033_NAME],
        )
        .expect("track 0033");
        (dir, conn)
    }

    #[test]
    fn an_unmigrated_database_is_not_schema_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = crate::db::open::open_archive_connection(&dir.path().join("entropia.sqlite"))
            .expect("open");
        assert_eq!(
            is_schema_ready(&conn).expect("check"),
            false,
            "no migration row, no tables: the gate must stay closed"
        );
    }

    #[test]
    fn the_mirrored_migration_applies_and_opens_the_gate() {
        let (_dir, conn) = migrated_db();
        assert!(
            is_schema_ready(&conn).expect("check"),
            "tables + tracking row + durable PRAGMAs must read ready"
        );
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'processing_%' ORDER BY name")
            .expect("tables query")
            .query_map([], |row| row.get(0))
            .expect("map")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(tables.len(), 10, "all ten processing tables: {tables:?}");
    }

    #[test]
    fn a_committed_task_survives_closing_and_reopening_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        {
            let conn = crate::db::open::open_archive_connection(&db_path).expect("open");
            conn.execute_batch("CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
                .expect("collections");
            conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
                .expect("items");
            conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);")
                .expect("assets");
            conn.execute_batch("CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);")
                .expect("extractions");
            conn.execute_batch("CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, language TEXT, duration_ms INTEGER, model TEXT NOT NULL, segments TEXT, confidence REAL, created_at INTEGER NOT NULL);")
                .expect("transcriptions");
            conn.execute_batch("CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);")
                .expect("tracking");
            conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
                .expect("apply 0032");
            conn.execute(
                "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
                [MIGRATION_NAME],
            )
            .expect("track");
            conn.execute_batch(MIGRATION_0033_SQL).expect("apply 0033");
            conn.execute(
                "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
                [MIGRATION_0033_NAME],
            )
            .expect("track 0033");
            conn.execute(
                "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t1', 'ocr', 'a1', 'succeeded', 1, 1)",
                [],
            )
            .expect("insert task");
        }
        // Process "death": drop every connection, then reopen the same file.
        let reopened = crate::db::open::open_archive_connection(&db_path).expect("reopen");
        assert!(is_schema_ready(&reopened).expect("check"));
        let state: String = reopened
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("task survived the restart");
        assert_eq!(state, "succeeded");
    }

    #[test]
    fn the_active_task_exclusion_rejects_a_second_live_writer() {
        let (_dir, conn) = migrated_db();
        conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t1', 'ocr', 'a1', 'pending', 1, 1)",
            [],
        )
        .expect("first admission");
        let second = conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t2', 'ocr', 'a1', 'pending', 1, 1)",
            [],
        );
        assert!(
            second.is_err(),
            "two live rows for one operation+asset would let two workers compute twice"
        );
    }

    #[test]
    fn source_text_writes_bump_the_asset_revision() {
        let (_dir, conn) = migrated_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a1', 'i1', 'p', 'image', 1)",
            [],
        )
        .expect("asset");
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('e1', 'a1', 'hola', 'ocr', 1)",
            [],
        )
        .expect("extraction");
        let rev: i64 = conn
            .query_row(
                "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("revision bumped by the trigger");
        assert_eq!(rev, 1);
        conn.execute(
            "UPDATE extractions SET text_content = 'hola mundo' WHERE id = 'e1'",
            [],
        )
        .expect("edit");
        let rev: i64 = conn
            .query_row(
                "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("revision bumped again");
        assert_eq!(rev, 2);
    }

    #[test]
    fn invalid_states_are_rejected_by_check_constraints() {
        let (_dir, conn) = migrated_db();
        let bad = conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t9', 'ocr', 'a9', 'finished', 1, 1)",
            [],
        );
        assert!(
            bad.is_err(),
            "'finished' is not a task state: the UI must never invent one"
        );
    }
    // ── Unidad 2: admission, eligibility, invalidation ──────────────────────

    fn batch_db() -> (tempfile::TempDir, Connection) {
        use crate::nlp::embeddings as emb;
        let (dir, conn) = migrated_db();
        std::fs::write(
            dir.path().join("a5.pdf"),
            include_bytes!("../../tests/fixtures/pdf-aes128-owner-password.pdf"),
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TABLE vec_assets(asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL,
               embedding BLOB NOT NULL, embedding_model TEXT NOT NULL DEFAULT 'legacy',
               embedding_contract TEXT NOT NULL DEFAULT 'legacy',
               dimensions INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE rag_chunks(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL,
               item_id TEXT NOT NULL, source_kind TEXT NOT NULL, source_id TEXT NOT NULL,
               chunk_ordinal INTEGER NOT NULL, text_content TEXT NOT NULL,
               start_char INTEGER NOT NULL, end_char INTEGER NOT NULL,
               source_text_hash TEXT NOT NULL, chunking_contract TEXT NOT NULL,
               embedding BLOB NOT NULL, embedding_model TEXT NOT NULL,
               embedding_contract TEXT NOT NULL, dimensions INTEGER NOT NULL);",
        )
        .expect("embedding tables");
        conn.execute(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES
               ('c1', 'legajo', 1, 1), ('c2', 'fotos', 2, 2)",
            [],
        )
        .expect("collections");
        conn.execute(
            "INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES
               ('i1', 'doc', 'c1', 1, 1), ('i2', 'foto', 'c2', 2, 2)",
            [],
        )
        .expect("items");
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES
               ('a1', 'i1', 'a1.png', 'image', 1),
               ('a2', 'i1', 'a2.png', 'image', 2),
               ('a3', 'i1', 'a3.mp3', 'audio', 3),
               ('a4', 'i1', 'a4.pdf', 'pdf', 4),
               ('a5', 'i1', 'a5.pdf', 'pdf', 5),
               ('a5p1', 'i1', 'a5p1.png', 'image', 6),
               ('a5p2', 'i1', 'a5p2.png', 'image', 7),
               ('a6', 'i2', 'a6.png', 'image', 8),
               ('a7', 'i1', 'a7.png', 'image', 9)",
            [],
        )
        .expect("assets");
        conn.execute(
            "UPDATE assets SET parent_asset_id = 'a5', page_number = 1 WHERE id = 'a5p1'",
            [],
        )
        .expect("page 1");
        conn.execute(
            "UPDATE assets SET parent_asset_id = 'a5', page_number = 2 WHERE id = 'a5p2'",
            [],
        )
        .expect("page 2");
        let short_text = "hola mundo, texto con contenido suficiente".to_string();
        let long_text = "lorem ipsum ".repeat(200);
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES
               ('e2', 'a2', ?1, 'ocr', 1),
               ('e4', 'a4', 'texto nativo del pdf', 'native', 1),
               ('e5p2', 'a5p2', 'texto de la pagina dos', 'ocr', 1),
               ('e7', 'a7', ?2, 'ocr', 1)",
            rusqlite::params![short_text, long_text],
        )
        .expect("extractions");
        conn.execute(
            "INSERT INTO transcriptions (id, asset_id, text_content, model, created_at)
             VALUES ('t6', 'a6', 'texto hablado en el audio', 'whisper', 1)",
            [],
        )
        .expect("transcriptions");
        // a2: current vector + complete chunk set → Fresh.
        conn.execute(
            "INSERT INTO vec_assets (asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES ('a2', 'i1', zeroblob(16), ?1, ?2, ?3)",
            rusqlite::params![
                emb::CANONICAL_EMBEDDING_MODEL,
                emb::CANONICAL_EMBEDDING_CONTRACT_V1,
                emb::CANONICAL_EMBEDDING_DIMENSIONS as i64,
            ],
        )
        .expect("a2 vector");
        insert_matching_chunks(&conn, "a2", "i1", "e2", &short_text);
        // a6: legacy vector → ContractMismatch.
        conn.execute(
            "INSERT INTO vec_assets (asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES ('a6', 'i2', zeroblob(16), 'legacy', 'legacy', 0)",
            [],
        )
        .expect("a6 legacy vector");
        // a7: current vector but no chunks → IncompleteChunks.
        conn.execute(
            "INSERT INTO vec_assets (asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES ('a7', 'i1', zeroblob(16), ?1, ?2, ?3)",
            rusqlite::params![
                emb::CANONICAL_EMBEDDING_MODEL,
                emb::CANONICAL_EMBEDDING_CONTRACT_V1,
                emb::CANONICAL_EMBEDDING_DIMENSIONS as i64,
            ],
        )
        .expect("a7 vector");
        (dir, conn)
    }

    fn insert_matching_chunks(
        conn: &Connection,
        asset_id: &str,
        item_id: &str,
        source_id: &str,
        text: &str,
    ) {
        use crate::nlp::embeddings as emb;
        let source = emb::RagChunkSource {
            asset_id: asset_id.to_string(),
            item_id: item_id.to_string(),
            source_kind: emb::RagChunkSourceKind::Extraction,
            source_id: source_id.to_string(),
            text: text.to_string(),
        };
        for draft in emb::plan_rag_chunks(&source).expect("plan fixture chunks") {
            conn.execute(
                "INSERT INTO rag_chunks (id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                   text_content, start_char, end_char, source_text_hash, chunking_contract,
                   embedding, embedding_model, embedding_contract, dimensions)
                 VALUES (?1, ?2, ?3, 'extraction', ?4, ?5, ?6, ?7, ?8, ?9, ?10, zeroblob(8), ?11, ?12, ?13)",
                rusqlite::params![
                    draft.id,
                    draft.asset_id,
                    draft.item_id,
                    draft.source_id,
                    draft.chunk_ordinal as i64,
                    draft.text_content,
                    draft.start_char as i64,
                    draft.end_char as i64,
                    draft.source_text_hash,
                    draft.chunking_contract,
                    emb::CANONICAL_EMBEDDING_MODEL,
                    emb::CANONICAL_EMBEDDING_CONTRACT_V1,
                    emb::CANONICAL_EMBEDDING_DIMENSIONS as i64,
                ],
            )
            .expect("insert fixture chunk");
        }
    }

    fn insert_batch(conn: &Connection, id: &str, request: &str, ops: &str) {
        conn.execute(
            "INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, created_at, updated_at)
             VALUES (?1, ?2, 'user', 'preparing', 'run', ?3, 1, 1)",
            rusqlite::params![id, request, ops],
        )
        .expect("insert batch");
    }

    fn task_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM processing_tasks", [], |row| {
            row.get(0)
        })
        .expect("count tasks")
    }

    fn member_class(conn: &Connection, batch: &str, asset: &str) -> (String, String) {
        conn.query_row(
            "SELECT classification, reason FROM processing_batch_members WHERE batch_id = ?1 AND asset_id_snapshot = ?2",
            rusqlite::params![batch, asset],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read member")
    }

    #[test]
    fn ocr_rule_distinguishes_missing_done_unsupported_and_parents() {
        use super::super::eligibility::{ocr_decision, OcrDecision};
        let (_dir, conn) = batch_db();
        assert_eq!(
            ocr_decision(&conn, "a1").expect("a1"),
            OcrDecision::Eligible
        );
        assert!(matches!(
            ocr_decision(&conn, "a2").expect("a2"),
            OcrDecision::AlreadyDone { .. }
        ));
        // Native extraction satisfies the text need: no rasterization.
        assert!(matches!(
            ocr_decision(&conn, "a4").expect("a4"),
            OcrDecision::AlreadyDone { .. }
        ));
        assert!(matches!(
            ocr_decision(&conn, "a3").expect("a3"),
            OcrDecision::UnsupportedType { .. }
        ));
        assert!(matches!(
            ocr_decision(&conn, "a5").expect("a5"),
            OcrDecision::ParentHasPages { page_count: 2 }
        ));
        assert_eq!(
            ocr_decision(&conn, "a5p1").expect("a5p1"),
            OcrDecision::Eligible
        );
    }

    #[test]
    fn empty_successful_ocr_is_done_not_missing() {
        use super::super::eligibility::{ocr_decision, OcrDecision};
        let (_dir, conn) = batch_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('ae', 'i1', 'ae.png', 'image', 10)",
            [],
        )
        .expect("asset");
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('ee', 'ae', '', 'ocr', 1)",
            [],
        )
        .expect("empty extraction");
        // Empty text with a canonical row means "recognized nothing", not
        // "never ran": requeueing it would loop forever on blank pages.
        assert!(matches!(
            ocr_decision(&conn, "ae").expect("ae"),
            OcrDecision::AlreadyDone { .. }
        ));
    }

    #[test]
    fn embedding_rule_covers_fresh_stale_and_textless() {
        use super::super::eligibility::{
            embedding_decision, EmbeddingDecision, EmbeddingStaleReason,
        };
        let (_dir, conn) = batch_db();
        assert_eq!(
            embedding_decision(&conn, "a2").expect("a2"),
            EmbeddingDecision::Fresh
        );
        assert_eq!(
            embedding_decision(&conn, "a1").expect("a1"),
            EmbeddingDecision::NoSourceText
        );
        assert_eq!(
            embedding_decision(&conn, "a4").expect("a4"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::MissingVector
            }
        );
        assert_eq!(
            embedding_decision(&conn, "a6").expect("a6"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::ContractMismatch
            }
        );
        assert_eq!(
            embedding_decision(&conn, "a7").expect("a7"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::IncompleteChunks
            }
        );
    }

    #[test]
    fn editing_source_text_invalidates_fresh_embeddings() {
        use super::super::eligibility::{
            embedding_decision, EmbeddingDecision, EmbeddingStaleReason,
        };
        let (_dir, conn) = batch_db();
        // Simulate Unidad 4 completion: the embedding run records which
        // revision it published.
        conn.execute(
            "UPDATE processing_asset_revisions SET embedding_completed_revision = source_revision WHERE asset_id = 'a2'",
            [],
        )
        .expect("completion marker");
        assert_eq!(
            embedding_decision(&conn, "a2").expect("a2 fresh"),
            EmbeddingDecision::Fresh
        );
        // The invalidation trigger (0032) bumps the revision in the same
        // transaction as the text edit — no UI refresh needed.
        conn.execute(
            "UPDATE extractions SET text_content = 'texto corregido' WHERE id = 'e2'",
            [],
        )
        .expect("edit source");
        assert_eq!(
            embedding_decision(&conn, "a2").expect("a2 stale"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::SourceChanged
            }
        );
    }

    #[test]
    fn prepare_and_classify_admits_exactly_the_missing_work() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        let added = prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        assert_eq!(added, 8, "every c1 asset snapshotted exactly once");
        let page = classify_batch_page(&conn, "b1", 200).expect("classify");
        assert_eq!(page.processed, 8);
        assert!(!page.done, "done flips on the draining call");
        let drain = classify_batch_page(&conn, "b1", 200).expect("drain");
        assert!(drain.done);
        // OCR tasks: a1, a5p1. Embeddings: a1(wait), a4, a5p1(wait), a5p2, a7.
        assert_eq!(task_count(&conn), 7);
        let (class_a2, _) = member_class(&conn, "b1", "a2");
        assert_eq!(class_a2, "already_done");
        let (class_a3, _) = member_class(&conn, "b1", "a3");
        assert_eq!(class_a3, "unsupported_type");
        let (class_a5, _) = member_class(&conn, "b1", "a5");
        assert_eq!(class_a5, "parent_has_pages");
        let (class_a1, reason_a1) = member_class(&conn, "b1", "a1");
        assert_eq!(class_a1, "admitted");
        assert!(reason_a1.contains("ocr:admit"), "{reason_a1}");
        // The embedding blocked on OCR carries the dependency link.
        let dep: Option<String> = conn
            .query_row(
                "SELECT dependency_task_id FROM processing_batch_tasks WHERE batch_id = 'b1' AND kind = 'embedding' AND asset_id_snapshot = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("dependency link");
        assert_eq!(dep, live_task(&conn, "ocr", "a1").unwrap());
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE kind = 'embedding' AND asset_id_snapshot = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("blocked state");
        assert_eq!(state, "blocked");
    }

    #[test]
    fn overlapping_batches_share_one_physical_task() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare b1");
        classify_batch_page(&conn, "b1", 200).expect("classify b1");
        classify_batch_page(&conn, "b1", 200).expect("drain b1");
        assert_eq!(task_count(&conn), 7);
        // Second batch over both collections: c1 work attaches, a6 creates.
        insert_batch(&conn, "b2", "req-2", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b2", &["c1".to_string(), "c2".to_string()]).expect("prepare b2");
        classify_batch_page(&conn, "b2", 200).expect("classify b2");
        classify_batch_page(&conn, "b2", 200).expect("drain b2");
        // Only a6's two tasks are new; everything else attached.
        assert_eq!(task_count(&conn), 9);
        let links: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batch_tasks WHERE kind = 'ocr' AND asset_id_snapshot = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("shared links");
        assert_eq!(links, 2);
    }

    #[test]
    fn crashed_classification_replays_without_duplicates() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        let first = classify_batch_page(&conn, "b1", 3).expect("page 1");
        assert_eq!(first.processed, 3);
        assert!(!first.done);
        let tasks_after_crash = task_count(&conn);
        // Crash replay: the cursor never confirmed, so the same page runs
        // again. Deterministic ids + live-task lookup make it idempotent.
        conn.execute(
            "UPDATE processing_batches SET planning_cursor = 0 WHERE id = 'b1'",
            [],
        )
        .expect("simulate lost cursor");
        let replay = classify_batch_page(&conn, "b1", 3).expect("replay");
        assert_eq!(replay.processed, 3);
        assert_eq!(task_count(&conn), tasks_after_crash);
        let members: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batch_members WHERE batch_id = 'b1'",
                [],
                |row| row.get(0),
            )
            .expect("members stable");
        assert_eq!(members, 8);
    }

    #[test]
    fn new_assets_after_the_snapshot_belong_to_a_later_batch() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a8', 'i1', 'a8.png', 'image', 10)",
            [],
        )
        .expect("late asset");
        // Classification only walks the snapshotted members.
        classify_batch_page(&conn, "b1", 200).expect("classify");
        classify_batch_page(&conn, "b1", 200).expect("drain");
        let late: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batch_members WHERE batch_id = 'b1' AND asset_id_snapshot = 'a8'",
                [],
                |row| row.get(0),
            )
            .expect("late asset excluded");
        assert_eq!(late, 0);
        let task: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_tasks WHERE kind = 'ocr' AND asset_id_snapshot = 'a8'",
                [],
                |row| row.get(0),
            )
            .expect("no task for late asset");
        assert_eq!(task, 0);
    }
    #[test]
    fn system_batches_are_stable_singletons() {
        let (_dir, conn) = batch_db();
        let manual = ensure_system_batch(&conn, "manual").expect("create manual");
        let again = ensure_system_batch(&conn, "manual").expect("reopen manual");
        assert_eq!(manual, again);
        let repair = ensure_system_batch(&conn, "repair").expect("create repair");
        assert_ne!(manual, repair);
        let (state, desired, done): (String, String, i64) = conn
            .query_row(
                "SELECT state, desired_state, planning_done FROM processing_batches WHERE id = ?1",
                [&manual],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("system batch runnable");
        assert_eq!(
            (state.as_str(), desired.as_str(), done),
            ("running", "run", 1)
        );
        assert!(!maybe_finalize_batch(&conn, &manual).unwrap());
        conn.execute(
            "UPDATE processing_batches SET state='completed' WHERE id=?1",
            [&manual],
        )
        .unwrap();
        ensure_system_batch(&conn, "manual").unwrap();
        let admitted =
            admit_or_attach(&conn, &manual, "ocr", "a1", 0, "", "ocr:light", None).unwrap();
        let claim = claim_next(&conn, "s", &["ocr"], 1).unwrap().unwrap();
        assert_eq!(claim.task_id, admitted.task_id);
        assert!(ensure_system_batch(&conn, "user").is_err());
    }

    #[test]
    fn pause_resume_cancel_converge_without_losing_confirmed_work() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        // Start: preparing with desired run promotes once planning drains.
        control_batch(&conn, "b1", BatchAction::Resume, None).expect("start");
        advance_planning(&conn, "b1", 10, 200).expect("plan");
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_batches WHERE id = 'b1'",
                [],
                |row| row.get(0),
            )
            .expect("running");
        assert_eq!(state, "running");
        // Pause withdraws demand: links flip, batch observes pausing.
        control_batch(&conn, "b1", BatchAction::Pause, None).expect("pause");
        let (state, desired): (String, String) = conn
            .query_row(
                "SELECT state, desired_state FROM processing_batches WHERE id = 'b1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("pausing");
        assert_eq!((state.as_str(), desired.as_str()), ("pausing", "pause"));
        let paused_links: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batch_tasks WHERE batch_id = 'b1' AND request_state = 'paused'",
                [],
                |row| row.get(0),
            )
            .expect("links paused");
        assert!(paused_links > 0);
        assert_eq!(cancel_orphaned_tasks(&conn).unwrap(), 0);
        assert!(!maybe_finalize_batch(&conn, "b1").unwrap());
        assert_eq!(read_batch_snapshot(&conn, "b1").unwrap().state, "paused");
        // Resume reopens demand; cancel withdraws it permanently.
        control_batch(&conn, "b1", BatchAction::Resume, None).expect("resume");
        control_batch(&conn, "b1", BatchAction::Cancel, None).expect("cancel");
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_batches WHERE id = 'b1'",
                [],
                |row| row.get(0),
            )
            .expect("cancelled");
        assert_eq!(state, "cancelled");
        // Terminal batches reject further transitions.
        assert!(control_batch(&conn, "b1", BatchAction::Resume, None).is_err());
    }

    #[test]
    fn live_queue_work_tracks_an_items_unsettled_units() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr"]"#);
        control_batch(&conn, "b1", BatchAction::Resume, None).expect("start");
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        advance_planning(&conn, "b1", 10, 200).expect("plan");

        // Planned units are still pending: the item's text is not final yet.
        assert!(item_has_live_queue_work(&conn, "i1").expect("live check"));
        // An item nobody queued has nothing outstanding.
        assert!(!item_has_live_queue_work(&conn, "i2").expect("live check"));

        conn.execute(
            "UPDATE processing_tasks SET state = 'succeeded' WHERE state != 'succeeded'",
            [],
        )
        .expect("settle everything");
        assert!(!item_has_live_queue_work(&conn, "i1").expect("live check"));

        // A cancelled unit is settled too — a withdrawn batch must not keep
        // the follow-up waiting forever.
        conn.execute(
            "UPDATE processing_tasks SET state = 'cancelled' WHERE rowid = (SELECT MIN(rowid) FROM processing_tasks)",
            [],
        )
        .expect("cancel one");
        assert!(!item_has_live_queue_work(&conn, "i1").expect("live check"));
    }

    #[test]
    fn listings_hide_system_batches() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr"]"#);
        let repair = ensure_system_batch(&conn, "repair").expect("repair");
        let manual = ensure_system_batch(&conn, "manual").expect("manual");

        // The batch lists are the user's own work. System containers own
        // out-of-band units and are always running, so showing them puts a
        // row in "active" that no human can act on and that never finishes.
        let (batches, _) = list_batches(&conn, None, None, 50).expect("list");
        let ids: Vec<&str> = batches.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["b1"]);
        assert!(!ids.contains(&repair.as_str()));
        assert!(!ids.contains(&manual.as_str()));
    }

    #[test]
    fn snapshots_list_and_detail_read_durable_state() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        control_batch(&conn, "b1", BatchAction::Resume, None).expect("start");
        advance_planning(&conn, "b1", 10, 200).expect("plan");
        let snapshot = read_batch_snapshot(&conn, "b1").expect("snapshot");
        assert_eq!(snapshot.members_total, 8);
        assert_eq!(snapshot.members_classified, 8);
        assert!(snapshot.tasks_by_state.iter().any(|(s, _)| s == "pending"));
        let (batches, next) = list_batches(&conn, None, None, 50).expect("list");
        assert_eq!(batches.len(), 1);
        assert!(next.is_none());
        let (tasks, tasks_next) =
            list_tasks(&conn, "b1", Some("pending"), None, None, 50).expect("tasks");
        assert!(!tasks.is_empty());
        assert!(tasks_next.is_none());
        assert!(tasks.iter().all(|t| t.state == "pending"));
        let detail = read_task_detail(&conn, "b1", &tasks[0].task_id, 10).expect("detail");
        assert_eq!(detail.task_id, tasks[0].task_id);
        assert!(read_task_detail(&conn, "b1", "nope", 10).is_err());
        assert!(read_batch_snapshot(&conn, "nope").is_err());
    }

    #[test]
    fn retry_opens_a_new_cycle_and_pulls_failed_dependencies() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        control_batch(&conn, "b1", BatchAction::Resume, None).expect("start");
        advance_planning(&conn, "b1", 10, 200).expect("plan");
        let ocr_id = live_task(&conn, "ocr", "a1").unwrap().unwrap();
        let embedding_id = live_task(&conn, "embedding", "a1").unwrap().unwrap();
        // Fail the OCR unit terminally: its blocked embedding must follow.
        conn.execute(
            "UPDATE processing_tasks SET state = 'failed', last_error_code = 'corrupt_pdf', last_error_message = 'locked' WHERE id = ?1",
            [&ocr_id],
        )
        .expect("fail ocr");
        settle_blocked_dependents(&conn).expect("settle");
        let dep_state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = ?1",
                [&embedding_id],
                |row| row.get(0),
            )
            .expect("dependent failed");
        assert_eq!(dep_state, "failed");
        // Retrying only the embedding reopens the OCR chain too.
        let reopened = retry_failed(&conn, "b1", Some(&embedding_id)).expect("retry");
        assert_eq!(reopened, 2);
        for id in [&ocr_id, &embedding_id] {
            let (state, cycle): (String, i64) = conn
                .query_row(
                    "SELECT state, retry_cycle FROM processing_tasks WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("reopened");
            assert_eq!((state.as_str(), cycle), ("pending", 1));
        }
        // A second retry of a non-failed unit is rejected, not duplicated.
        assert!(retry_failed(&conn, "b1", Some(&embedding_id)).is_err());
    }

    #[test]
    fn control_rejects_stale_revisions() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr"]"#);
        let revision: i64 = conn
            .query_row(
                "SELECT revision FROM processing_batches WHERE id = 'b1'",
                [],
                |row| row.get(0),
            )
            .expect("revision");
        control_batch(&conn, "b1", BatchAction::Pause, Some(revision + 99)).expect_err("stale");
        control_batch(&conn, "b1", BatchAction::Pause, Some(revision)).expect("fresh");
    }

    #[test]
    fn cancelling_batch_rejects_retry() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b", "req", r#"["ocr"]"#);
        let task = admit_or_attach(&conn, "b", "ocr", "a1", 0, "", "ocr:light", None).unwrap();
        conn.execute(
            "UPDATE processing_tasks SET state='failed' WHERE id=?1",
            [&task.task_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE processing_batches SET state='cancelling', desired_state='cancel' WHERE id='b'",
            [],
        )
        .unwrap();
        assert!(retry_failed(&conn, "b", Some(&task.task_id)).is_err());
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id=?1",
                [&task.task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "failed");
    }

    #[test]
    fn cancelling_batch_rejects_pause_and_resume() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b", "req", r#"["ocr"]"#);
        conn.execute(
            "UPDATE processing_batches SET state='cancelling', desired_state='cancel' WHERE id='b'",
            [],
        )
        .unwrap();
        assert!(control_batch(&conn, "b", BatchAction::Pause, None).is_err());
        assert!(control_batch(&conn, "b", BatchAction::Resume, None).is_err());
        let state: (String, String) = conn
            .query_row(
                "SELECT state, desired_state FROM processing_batches WHERE id='b'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("cancelling".to_string(), "cancel".to_string()));
    }

    #[test]
    fn reconcile_parks_contract_drift_without_failing() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["embeddings"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        control_batch(&conn, "b1", BatchAction::Resume, None).expect("start");
        advance_planning(&conn, "b1", 10, 200).expect("plan");
        // Simulate a provider/model change after admission.
        conn.execute(
            "UPDATE processing_tasks SET contract_hash = 'old-contract' WHERE kind = 'embedding' AND state = 'pending'",
            [],
        )
        .expect("drift contracts");
        let parked = reconcile_contracts(&conn).expect("reconcile");
        assert!(parked > 0);
        let blocked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_tasks WHERE state = 'blocked' AND outcome = 'configuration_changed'",
                [],
                |row| row.get(0),
            )
            .expect("blocked count");
        assert_eq!(blocked as usize, parked);
        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_tasks WHERE state = 'failed'",
                [],
                |row| row.get(0),
            )
            .expect("failed count");
        assert_eq!(failed, 0, "drift parks, never fails");
    }

    #[test]
    fn batch_history_paginates_newest_first() {
        let (_dir, conn) = batch_db();
        for index in 0..3 {
            let id = format!("b{index}");
            let request = format!("req-{index}");
            conn.execute(
                "INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, created_at, updated_at)
                 VALUES (?1, ?2, 'user', 'completed', 'run', '[\"ocr\"]', ?3, ?3)",
                rusqlite::params![id, request, 100 + index],
            )
            .expect("insert batch");
        }
        let (page1, cursor) = list_batches(&conn, None, None, 2).expect("page 1");
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].id, "b2");
        let cursor = cursor.expect("more pages");
        let (page2, cursor) = list_batches(&conn, None, Some(cursor), 2).expect("page 2");
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].id, "b0");
        assert!(cursor.is_none());
    }

    #[test]
    fn request_rows_answer_replay_without_duplicating() {
        let (_dir, conn) = batch_db();
        assert!(find_request(&conn, "req-1").expect("lookup").is_none());
        record_request(
            &conn,
            "req-1",
            "prepare",
            None,
            "hash-a",
            r#"{"members":2}"#,
            7,
        )
        .expect("record");
        let replay = find_request(&conn, "req-1").expect("replay").expect("row");
        assert_eq!(
            replay,
            IdempotentRequest {
                batch_id: None,
                payload_hash: "hash-a".to_string(),
                response_json: Some(r#"{"members":2}"#.to_string()),
            }
        );
        // Same id twice is a primary-key conflict: concurrent duplicates
        // serialize here instead of forking a second batch.
        assert!(record_request(&conn, "req-1", "prepare", None, "hash-a", "{}", 8).is_err());
    }

    #[test]
    fn a_terminal_result_does_not_prevent_new_revision_admission() {
        let (_dir, conn) = batch_db();
        let batch = ensure_system_batch(&conn, "manual").unwrap();
        let old = admit_or_attach(&conn, &batch, "ocr", "a1", 0, "", "ocr:light", None).unwrap();
        conn.execute(
            "UPDATE processing_tasks SET state='succeeded' WHERE id=?1",
            [&old.task_id],
        )
        .unwrap();
        let next =
            admit_or_attach(&conn, &batch, "ocr", "a1", 1, "new", "ocr:light", None).unwrap();
        assert_ne!(old.task_id, next.task_id);
        assert_eq!(
            claim_next(&conn, "s", &["ocr"], 1)
                .unwrap()
                .unwrap()
                .task_id,
            next.task_id
        );
        let old_state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id=?1",
                [&old.task_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_state, "succeeded");
    }

    #[test]
    fn retrying_a_completed_error_batch_makes_its_task_claimable() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b", "req", r#"["ocr"]"#);
        conn.execute("UPDATE processing_batches SET state='completed_with_errors',planning_done=1 WHERE id='b'", []).unwrap();
        let task = admit_or_attach(&conn, "b", "ocr", "a1", 0, "", "ocr:light", None).unwrap();
        conn.execute(
            "UPDATE processing_tasks SET state='failed' WHERE id=?1",
            [&task.task_id],
        )
        .unwrap();
        retry_failed(&conn, "b", Some(&task.task_id)).unwrap();
        promote_ready_batches(&conn).unwrap();
        assert_eq!(
            claim_next(&conn, "s", &["ocr"], 1)
                .unwrap()
                .unwrap()
                .task_id,
            task.task_id
        );
    }

    #[test]
    fn analyzing_a_paused_draft_finishes_without_claiming_work() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b", "req", r#"["ocr"]"#);
        conn.execute(
            "UPDATE processing_batches SET desired_state='pause' WHERE id='b'",
            [],
        )
        .unwrap();
        prepare_membership(&conn, "b", &["c1".to_string()]).unwrap();
        assert_eq!(planning_batches(&conn).unwrap(), vec!["b".to_string()]);
        advance_planning(&conn, "b", 10, 200).unwrap();
        let snapshot = read_batch_snapshot(&conn, "b").unwrap();
        assert!(snapshot.planning_done);
        assert_eq!(snapshot.state, "ready");
        assert!(claim_next(&conn, "s", &["ocr"], 1).unwrap().is_none());
    }

    #[test]
    fn automatic_ocr_does_not_replace_a_newly_arrived_extraction() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b", "req", r#"["ocr"]"#);
        conn.execute("UPDATE processing_batches SET state='running',desired_state='run',planning_done=1 WHERE id='b'", []).unwrap();
        let task = admit_or_attach(&conn, "b", "ocr", "a1", 0, "", "ocr:light", None).unwrap();
        conn.execute("INSERT INTO extractions(id,asset_id,text_content,method,created_at) VALUES('new','a1','edited text','ocr',1)", []).unwrap();
        assert!(claim_next(&conn, "s", &["ocr"], 1).unwrap().is_none());
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id=?1",
                [&task.task_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "skipped");
    }

    #[test]
    fn a_partial_pdf_import_is_not_classified_as_complete() {
        let (_dir, conn) = batch_db();
        conn.execute("DELETE FROM assets WHERE id='a5p2'", [])
            .unwrap();
        assert_eq!(
            super::super::eligibility::ocr_decision(&conn, "a5").unwrap(),
            super::super::eligibility::OcrDecision::Eligible
        );
    }

    #[test]
    fn repair_finds_missing_work_beyond_sixteen_fresh_assets() {
        let (_dir, conn) = batch_db();
        conn.execute("DELETE FROM assets WHERE id!='a2'", [])
            .unwrap();
        let text: String = conn
            .query_row(
                "SELECT text_content FROM extractions WHERE id='e2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for index in 0..17 {
            let id = format!("copy-{index:02}");
            let ext = format!("ext-{index}");
            conn.execute("INSERT INTO assets(id,item_id,path,type,created_at) VALUES(?1,'i1','a.png','image',?2)", rusqlite::params![id,index+10]).unwrap();
            conn.execute("INSERT INTO extractions(id,asset_id,text_content,method,created_at) VALUES(?1,?2,?3,'ocr',1)", rusqlite::params![ext,id,text]).unwrap();
            if index < 16 {
                conn.execute("INSERT INTO vec_assets SELECT ?1,item_id,embedding,embedding_model,embedding_contract,dimensions FROM vec_assets WHERE asset_id='a2'", [&id]).unwrap();
                insert_matching_chunks(&conn, &id, "i1", &ext, &text);
            }
        }
        let candidates = crate::nlp::embeddings::scan_text_assets(&conn, Some(16), true).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|row| row.asset_id.as_str())
                .collect::<Vec<_>>(),
            vec!["copy-16"]
        );
    }

    #[test]
    fn forced_backfill_preserves_published_vectors_until_replacement_succeeds() {
        let (_dir, conn) = batch_db();
        conn.execute("DELETE FROM assets WHERE id!='a2'", [])
            .unwrap();
        let before: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM vec_assets WHERE asset_id='a2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let chunks_before: Vec<Vec<u8>> = conn
            .prepare("SELECT embedding FROM rag_chunks WHERE asset_id='a2' ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let report = crate::nlp::commands::admit_backfill(&conn, true, None).unwrap();
        assert_eq!(report.requested, 1);
        let task = claim_next(&conn, "force-test", &["embedding"], now_ms())
            .unwrap()
            .expect("fresh vectors still admit forced replacement");
        fail_attempt(
            &conn,
            &task.task_id,
            task.lease_epoch,
            task.attempt_number,
            "provider_failure",
            "offline",
            false,
            None,
            now_ms(),
        )
        .unwrap();
        let after: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM vec_assets WHERE asset_id='a2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let chunks_after: Vec<Vec<u8>> = conn
            .prepare("SELECT embedding FROM rag_chunks WHERE asset_id='a2' ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(chunks_before, chunks_after);
        assert_eq!(
            super::super::eligibility::embedding_decision(&conn, "a2").unwrap(),
            super::super::eligibility::EmbeddingDecision::Fresh
        );
    }

    #[test]
    fn cancelled_revision_suppresses_automatic_repair_until_input_changes() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "user", "req-user", r#"["embeddings"]"#);
        let revision = source_revision(&conn, "a6").unwrap();
        let fingerprint =
            super::super::eligibility::embedding_input_fingerprint(&conn, "a6").unwrap();
        admit_or_attach(
            &conn,
            "user",
            "embedding",
            "a6",
            revision,
            &fingerprint,
            &super::super::eligibility::current_embedding_contract_hash(),
            None,
        )
        .unwrap();
        control_batch(&conn, "user", BatchAction::Cancel, None).unwrap();
        let suppressed: Option<i64> = conn
            .query_row(
                "SELECT auto_suppressed_revision FROM processing_asset_revisions WHERE asset_id='a6'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(suppressed, Some(revision));

        let repair = ensure_system_batch(&conn, "repair").unwrap();
        assert!(admit_repair_or_attach(
            &conn,
            &repair,
            "a6",
            revision,
            &fingerprint,
            &super::super::eligibility::current_embedding_contract_hash(),
        )
        .unwrap()
        .is_none());
        conn.execute(
            "UPDATE processing_asset_revisions SET source_revision=source_revision+1 WHERE asset_id='a6'",
            [],
        )
        .unwrap();
        assert!(admit_repair_or_attach(
            &conn,
            &repair,
            "a6",
            revision + 1,
            &super::super::eligibility::embedding_input_fingerprint(&conn, "a6").unwrap(),
            &super::super::eligibility::current_embedding_contract_hash(),
        )
        .unwrap()
        .is_some());
    }

    #[test]
    fn third_source_change_in_one_cycle_blocks_the_task() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b", "req", r#"["ocr"]"#);
        conn.execute(
            "UPDATE processing_batches SET state='running', desired_state='run', planning_done=1 WHERE id='b'",
            [],
        )
        .unwrap();
        let admitted = admit_or_attach(&conn, "b", "ocr", "a1", 0, "", "ocr:light", None).unwrap();
        for invalidation in 1..=3 {
            let task = claim_next(&conn, "worker", &["ocr"], invalidation)
                .unwrap()
                .unwrap();
            let outcome =
                record_source_change(&conn, &task.task_id, task.lease_epoch, "edited").unwrap();
            if invalidation < 3 {
                assert_eq!(outcome, SourceChangeOutcome::Requeued);
            } else {
                assert_eq!(outcome, SourceChangeOutcome::Blocked);
            }
        }
        let state: (String, String, i64) = conn
            .query_row(
                "SELECT state, outcome, source_invalidation_count FROM processing_tasks WHERE id=?1",
                [&admitted.task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            state,
            ("blocked".to_string(), "source_unstable".to_string(), 3)
        );
    }

    #[test]
    fn retry_delay_is_jittered_and_respects_longer_provider_delay() {
        let first = retry_delay_with_jitter_ms("task-a", 1, 0);
        let second = retry_delay_with_jitter_ms("task-b", 1, 0);
        assert!((4_000..=6_000).contains(&first));
        assert!((4_000..=6_000).contains(&second));
        assert_ne!(first, second);

        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b", "req", r#"["ocr"]"#);
        conn.execute(
            "UPDATE processing_batches SET state='running', desired_state='run', planning_done=1 WHERE id='b'",
            [],
        )
        .unwrap();
        admit_or_attach(&conn, "b", "ocr", "a1", 0, "", "ocr:light", None).unwrap();
        let task = claim_next(&conn, "worker", &["ocr"], 1_000)
            .unwrap()
            .unwrap();
        let outcome = fail_attempt(
            &conn,
            &task.task_id,
            task.lease_epoch,
            task.attempt_number,
            "rate_limited",
            "slow down [retry_after_ms=120000]",
            true,
            None,
            1_000,
        )
        .unwrap();
        assert_eq!(
            outcome,
            FailOutcome::RetryWait {
                next_retry_at: 121_000
            }
        );
    }
}
