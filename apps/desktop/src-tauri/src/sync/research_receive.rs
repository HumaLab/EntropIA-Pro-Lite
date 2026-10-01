//! Durable staging for research receive.
//!
//! Virtual `research_envelopes` rows must not enter generic `sync_pending_rows`:
//! that retry path journals and deletes tables it cannot apply. Pulled rows
//! live in `sync_meta` (`research_receive:<job_id>`) until a settle applies
//! them to the SEPARATE research state database, so staging survives a crash
//! before `research/estado.sqlite` was touched. This module performs no HTTP
//! and advances no cursor.
//!
//! Every stored row is bound to the account/server/device/epoch/session scope
//! copied from persisted session metadata and is removed only by exact
//! captured-value compare-and-delete. A malformed or unsupported row is never
//! partially applied: it is retained with a bounded error and a bounded
//! attempt count so the generic queue tooling can surface it.

// Receive staging is active (IS5a); the blanket allow covers the remaining
// surfaces (push-side helpers and self-transactional enqueue) reserved for the
// future IS5b push activation and its tests.
#![allow(dead_code)]

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::http::PullRow;
use super::research_capture::{ENVELOPE_TABLE, RECEIVE_PREFIX};
use super::research_envelope::{ResearchError, ResearchResult};
use super::research_transport::{apply_pulled_row, PullApplyOutcome, PulledResearchRow};
use super::session::{read_session_incarnation, SYNC_SESSION_INCARNATION_KEY};

const MAX_ERROR_CHARS: usize = 512;

/// Scope copied from persisted session metadata, never supplied by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResearchReceiveScope {
    pub(crate) account_id: String,
    pub(crate) server_url: String,
    pub(crate) device_id: String,
    pub(crate) server_epoch: String,
    pub(crate) session_incarnation: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredReceive {
    scope: ResearchReceiveScope,
    server_seq: i64,
    deleted: bool,
    changed_at: i64,
    device_id: String,
    payload: Option<Value>,
    #[serde(default)]
    attempts: u32,
    #[serde(default)]
    last_error: Option<String>,
}

/// One durable queued research row and the exact stored value required to
/// remove it later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueuedResearchReceive {
    pub(crate) job_id: String,
    pub(crate) server_seq: i64,
    pub(crate) deleted: bool,
    pub(crate) changed_at: i64,
    pub(crate) device_id: String,
    pub(crate) payload: Option<Value>,
    pub(crate) attempts: u32,
    pub(crate) last_error: Option<String>,
    pub(crate) captured_value: String,
}

/// Why one row was not queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchReceiveEnqueueError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Immutable queue snapshot safe to carry across an await. Holds the exact
/// captured value the settle compares against.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedResearchReceive {
    pub(crate) job_id: String,
    pub(crate) scope: ResearchReceiveScope,
    pub(crate) captured_value: String,
    pub(crate) row: PulledResearchRow,
}

/// Why settlement did not apply the prepared row. The queued row survives
/// every `Err` so nothing is lost silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchReceivePending {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Durably stores one pulled research row for the given session scope. Shape
/// validation only: a missing payload or a malformed envelope is a wire fact
/// that must be retained and classified by [`apply_pulled_row`], never
/// silently dropped here.
///
/// This entry point owns its own transaction; use [`stage_research_receive`]
/// when the caller already opened one (the pull page staging transaction).
pub(crate) fn enqueue_research_receive(
    conn: &Connection,
    row: &PullRow,
) -> Result<(), ResearchReceiveEnqueueError> {
    validate_row(row)?;
    let scope = current_scope(conn)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| local_error(format!("failed to start receive enqueue: {error}")))?;
    stage_research_receive(&tx, &scope, row)?;
    tx.commit()
        .map_err(|error| local_error(format!("failed to commit research receive: {error}")))
}

/// Transaction-safe staging: the same compare-and-set semantics as
/// [`enqueue_research_receive`] but with NO transaction management, so the
/// pull page staging can enqueue rows and advance its cursor in ONE SQLite
/// transaction. Newer sequences replace, identical same-sequence rows are
/// idempotent no-ops, conflicting same-sequence rows and foreign-scope rows
/// are rejected (fail closed), and older sequences never overwrite newer
/// staged state.
pub(crate) fn stage_research_receive(
    conn: &Connection,
    scope: &ResearchReceiveScope,
    row: &PullRow,
) -> Result<(), ResearchReceiveEnqueueError> {
    let validated = validate_row(row)?;
    let key = receive_key(&validated.job_id);
    let existing = read_exact(conn, &key).map_err(|error| local_error(error.message))?;
    if let Some((stored, _)) = existing {
        if stored.scope != *scope {
            return Err(enqueue_error(
                "research_receive_scope_conflict",
                "queued research row belongs to another session",
            ));
        }
        if stored.server_seq > validated.server_seq {
            return Ok(());
        }
        if stored.server_seq == validated.server_seq {
            if stored.wire_identity() == validated {
                return Ok(());
            }
            return Err(enqueue_error(
                "research_receive_payload_conflict",
                "equal server sequence has conflicting research payload",
            ));
        }
    }

    let stored = StoredReceive {
        scope: scope.clone(),
        server_seq: validated.server_seq,
        deleted: validated.deleted,
        changed_at: validated.changed_at,
        device_id: validated.device_id,
        payload: validated.payload,
        attempts: 0,
        last_error: None,
    };
    write_exact(conn, &key, &stored)
        .map_err(|error| local_error(format!("failed to store research receive: {error}")))
}

/// Reads every well-formed queued row in stable key order. Corrupt records
/// fail the read: they are sync-state errors, never silently skipped.
pub(crate) fn queued_research_receives(
    conn: &Connection,
) -> ResearchResult<Vec<QueuedResearchReceive>> {
    let mut statement = conn
        .prepare(
            "SELECT key, value FROM sync_meta
              WHERE substr(key, 1, length(?1)) = ?1
              ORDER BY key",
        )
        .map_err(|error| sql_error("Failed to read research receive queue", error))?;
    let rows = statement
        .query_map(params![RECEIVE_PREFIX], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| sql_error("Failed to read research receive queue", error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| sql_error("Failed to read research receive queue", error))?;

    rows.into_iter()
        .map(|(key, value)| parse_queued(&key, &value))
        .collect()
}

/// Prepares one queued row without retaining the database across an await.
/// The returned snapshot carries the exact captured value the settle compares.
pub(crate) fn prepare_research_receive(
    conn: &Connection,
    job_id: &str,
) -> Result<Option<PreparedResearchReceive>, ResearchReceivePending> {
    let queued = queued_research_receives(conn)
        .map_err(local_pending)?
        .into_iter()
        .find(|row| row.job_id == job_id);
    let Some(queued) = queued else {
        return Ok(None);
    };
    let scope = current_scope(conn).map_err(enqueue_to_pending)?;
    let stored = parse_stored(&queued.captured_value).map_err(local_pending)?;
    if stored.scope != scope {
        return Err(pending(
            "research_receive_scope_changed",
            "queued research row no longer matches the current session",
        ));
    }
    Ok(Some(PreparedResearchReceive {
        job_id: queued.job_id.clone(),
        scope,
        captured_value: queued.captured_value,
        row: PulledResearchRow {
            job_id: queued.job_id,
            server_seq: queued.server_seq,
            deleted: queued.deleted,
            changed_at: queued.changed_at,
            device_id: queued.device_id,
            payload: queued.payload,
        },
    }))
}

/// Rechecks the session and the exact queue identity, applies the row to the
/// separate research state database, then settles the stage: terminal
/// outcomes delete the exact captured value, `Deferred`/`Unsupported` retain
/// it with a bounded error. The research state write happens before the queue
/// deletion, so a crash in between replays an idempotent apply.
pub(crate) fn settle_research_receive(
    conn: &Connection,
    state: &Connection,
    artifacts_root: &Path,
    prepared: &PreparedResearchReceive,
) -> Result<PullApplyOutcome, ResearchReceivePending> {
    let current = current_scope(conn).map_err(enqueue_to_pending)?;
    if current != prepared.scope {
        return Err(pending(
            "research_receive_scope_changed",
            "session changed before research receive settlement",
        ));
    }
    let key = receive_key(&prepared.job_id);
    let stored = read_exact(conn, &key).map_err(local_pending)?;
    let Some((_, captured)) = stored else {
        return Err(pending(
            "research_receive_queue_changed",
            "queued research row disappeared before settlement",
        ));
    };
    if captured != prepared.captured_value {
        return Err(pending(
            "research_receive_queue_changed",
            "a newer queued research row replaced the prepared row",
        ));
    }

    let outcome = apply_pulled_row(
        conn,
        state,
        artifacts_root,
        &prepared.scope.device_id,
        &prepared.row,
    )
    .map_err(local_pending)?;
    match &outcome {
        PullApplyOutcome::Applied { .. }
        | PullApplyOutcome::NoOp
        | PullApplyOutcome::Stale
        | PullApplyOutcome::OwnChangeObserved => {
            delete_exact(conn, &key, &prepared.captured_value).map_err(local_pending)?;
        }
        PullApplyOutcome::Deferred { reason } | PullApplyOutcome::Unsupported { reason } => {
            retain_error(conn, prepared, reason).map_err(local_pending)?;
        }
    }
    Ok(outcome)
}

/// Retains one prepared row WITHOUT applying anything: the caller (the report
/// blob-before-apply gate) could not make the declared report bytes available,
/// so settling now would install a projection without its report file. The
/// bounded attempt count and reason keep the row retryable and visible.
pub(crate) fn defer_research_receive(
    conn: &Connection,
    prepared: &PreparedResearchReceive,
    reason: &str,
) -> Result<(), ResearchReceivePending> {
    let current = current_scope(conn).map_err(enqueue_to_pending)?;
    if current != prepared.scope {
        return Err(pending(
            "research_receive_scope_changed",
            "session changed before research receive deferral",
        ));
    }
    retain_error(conn, prepared, reason).map_err(local_pending)
}

/// Retains one unapplied row with a bounded attempt count and a bounded error.
fn retain_error(
    conn: &Connection,
    prepared: &PreparedResearchReceive,
    reason: &str,
) -> ResearchResult<()> {
    let key = receive_key(&prepared.job_id);
    let (mut stored, _) = read_exact(conn, &key)?.ok_or_else(|| {
        ResearchError::new(
            "research_receive_queue_changed",
            "queued research row disappeared before retention",
        )
    })?;
    stored.attempts = stored.attempts.saturating_add(1);
    stored.last_error = Some(truncate_error(reason));
    let value = serde_json::to_string(&stored)
        .map_err(|error| ResearchError::new("research_receive_corrupt", error.to_string()))?;
    let changed = conn
        .execute(
            "UPDATE sync_meta SET value = ?1 WHERE key = ?2 AND value = ?3",
            params![value, key, prepared.captured_value],
        )
        .map_err(|error| sql_error("Failed to retain research receive error", error))?;
    if changed != 1 {
        return Err(ResearchError::new(
            "research_receive_queue_changed",
            "queued research row changed before error retention",
        ));
    }
    Ok(())
}

fn delete_exact(conn: &Connection, key: &str, captured: &str) -> ResearchResult<()> {
    let changed = conn
        .execute(
            "DELETE FROM sync_meta WHERE key = ?1 AND value = ?2",
            params![key, captured],
        )
        .map_err(|error| sql_error("Failed to delete settled research receive", error))?;
    if changed != 1 {
        return Err(ResearchError::new(
            "research_receive_queue_changed",
            "exact queued research row was not deleted",
        ));
    }
    Ok(())
}

fn parse_stored(value: &str) -> ResearchResult<StoredReceive> {
    serde_json::from_str(value).map_err(|error| {
        ResearchError::new(
            "research_receive_corrupt",
            format!("cannot parse research receive: {error}"),
        )
    })
}

fn parse_queued(key: &str, value: &str) -> ResearchResult<QueuedResearchReceive> {
    let job_id = key
        .strip_prefix(RECEIVE_PREFIX)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            ResearchError::new(
                "research_receive_corrupt",
                format!("research receive key {key:?} has no job id"),
            )
        })?
        .to_string();
    let stored: StoredReceive = serde_json::from_str(value).map_err(|error| {
        ResearchError::new(
            "research_receive_corrupt",
            format!("cannot parse research receive {key:?}: {error}"),
        )
    })?;
    if stored.server_seq <= 0 || stored.device_id.is_empty() {
        return Err(ResearchError::new(
            "research_receive_corrupt",
            format!("research receive {key:?} has invalid sequence or device"),
        ));
    }
    Ok(QueuedResearchReceive {
        job_id,
        server_seq: stored.server_seq,
        deleted: stored.deleted,
        changed_at: stored.changed_at,
        device_id: stored.device_id,
        payload: stored.payload,
        attempts: stored.attempts,
        last_error: stored.last_error.map(|error| truncate_error(&error)),
        captured_value: value.to_string(),
    })
}

struct ValidatedRow {
    job_id: String,
    server_seq: i64,
    deleted: bool,
    changed_at: i64,
    device_id: String,
    payload: Option<Value>,
}

impl StoredReceive {
    fn wire_identity(&self) -> ValidatedRow {
        ValidatedRow {
            job_id: String::new(),
            server_seq: self.server_seq,
            deleted: self.deleted,
            changed_at: self.changed_at,
            device_id: self.device_id.clone(),
            payload: self.payload.clone(),
        }
    }
}

impl PartialEq<ValidatedRow> for ValidatedRow {
    fn eq(&self, other: &Self) -> bool {
        self.server_seq == other.server_seq
            && self.deleted == other.deleted
            && self.changed_at == other.changed_at
            && self.device_id == other.device_id
            && self.payload == other.payload
    }
}

/// Structural wire validation only. Upsert payload presence and envelope
/// validity are classified by [`apply_pulled_row`] and retained as
/// `Unsupported`, so a malformed row is never silently dropped.
fn validate_row(row: &PullRow) -> Result<ValidatedRow, ResearchReceiveEnqueueError> {
    if row.table != ENVELOPE_TABLE {
        return Err(enqueue_error(
            "research_receive_wrong_table",
            format!("row table {:?} is not {ENVELOPE_TABLE}", row.table),
        ));
    }
    if row.row_id.is_empty() || row.server_seq <= 0 || row.device_id.is_empty() {
        return Err(enqueue_error(
            "research_receive_invalid_row",
            "research row requires nonempty id/device and positive server sequence",
        ));
    }
    Ok(ValidatedRow {
        job_id: row.row_id.clone(),
        server_seq: row.server_seq,
        deleted: row.deleted,
        changed_at: row.changed_at,
        device_id: row.device_id.clone(),
        payload: row.payload.clone(),
    })
}

fn current_scope(conn: &Connection) -> Result<ResearchReceiveScope, ResearchReceiveEnqueueError> {
    Ok(ResearchReceiveScope {
        account_id: required_meta(conn, "account_id")?,
        server_url: required_meta(conn, "server_url")?,
        device_id: required_meta(conn, "device_id")?,
        server_epoch: required_meta(conn, "server_epoch")?,
        session_incarnation: read_session_incarnation(conn)
            .map_err(local_error)?
            .ok_or_else(|| {
                enqueue_error(
                    "research_receive_missing_session",
                    format!("sync session metadata {SYNC_SESSION_INCARNATION_KEY:?} is missing"),
                )
            })?,
    })
}

fn required_meta(conn: &Connection, key: &str) -> Result<String, ResearchReceiveEnqueueError> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| local_error(format!("failed to read session metadata {key:?}: {error}")))?
        .filter(|value: &String| !value.is_empty());
    value.ok_or_else(|| {
        enqueue_error(
            "research_receive_missing_session",
            format!("sync session metadata {key:?} is missing"),
        )
    })
}

fn read_exact(conn: &Connection, key: &str) -> ResearchResult<Option<(StoredReceive, String)>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sql_error("Failed to read research receive", error))?;
    raw.map(|raw| {
        serde_json::from_str(&raw)
            .map(|stored| (stored, raw))
            .map_err(|error| {
                ResearchError::new(
                    "research_receive_corrupt",
                    format!("cannot parse research receive {key:?}: {error}"),
                )
            })
    })
    .transpose()
}

fn write_exact(
    conn: &Connection,
    key: &str,
    stored: &StoredReceive,
) -> Result<(), rusqlite::Error> {
    let value = serde_json::to_string(stored).map_err(|error| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error,
        )))
    })?;
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map(|_| ())
}

fn receive_key(job_id: &str) -> String {
    format!("{RECEIVE_PREFIX}{job_id}")
}

fn truncate_error(error: &str) -> String {
    error.chars().take(MAX_ERROR_CHARS).collect()
}

fn enqueue_to_pending(error: ResearchReceiveEnqueueError) -> ResearchReceivePending {
    ResearchReceivePending {
        code: error.code,
        message: error.message,
    }
}

fn local_pending(error: ResearchError) -> ResearchReceivePending {
    pending("research_receive_local_state", error.message)
}

fn pending(code: &'static str, message: impl Into<String>) -> ResearchReceivePending {
    ResearchReceivePending {
        code,
        message: message.into(),
    }
}

fn enqueue_error(code: &'static str, message: impl Into<String>) -> ResearchReceiveEnqueueError {
    ResearchReceiveEnqueueError {
        code,
        message: message.into(),
    }
}

fn local_error(message: impl Into<String>) -> ResearchReceiveEnqueueError {
    enqueue_error("research_receive_local_state", message)
}

fn sql_error(context: &str, error: rusqlite::Error) -> ResearchError {
    ResearchError::sql(context, error)
}
