//! Durable staging for inactive writing receive.
//!
//! Virtual `writing_envelopes` rows must not enter generic `sync_pending_rows`:
//! that retry path journals and deletes tables it cannot apply. Queued rows
//! live in `sync_meta` until a later download/settle slice applies them.
//! This module performs no HTTP and advances no cursor.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::http::{HealthLimits, PullRow, SyncApi};
use super::session::{read_session_incarnation, SYNC_SESSION_INCARNATION_KEY};
use super::writing_blobs::ensure_writing_blobs_installed;
use crate::writing::repository::{WritingError, WritingResult};
use crate::writing::sync_capture::ENVELOPE_TABLE;
use crate::writing::sync_envelope::WritingEnvelopeV1;
use crate::writing::sync_transport::{apply_pulled_row, PullApplyOutcome, PulledWritingRow};

const RECEIVE_PREFIX: &str = "writing_receive:";
const MAX_ERROR_CHARS: usize = 512;

/// Scope copied from persisted session metadata, never supplied by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WritingReceiveScope {
    pub(crate) account_id: String,
    pub(crate) server_url: String,
    pub(crate) device_id: String,
    pub(crate) server_epoch: String,
    pub(crate) session_incarnation: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredReceive {
    scope: WritingReceiveScope,
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

/// One durable queued writing row and the exact stored value required to
/// remove it later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueuedWritingReceive {
    pub(crate) document_id: String,
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
pub(crate) struct WritingReceiveEnqueueError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Immutable queue snapshot safe to carry across an async download.
// `PartialEq` only: `PulledWritingRow` (sync_transport) is not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedWritingReceive {
    document_id: String,
    scope: WritingReceiveScope,
    captured_value: String,
    row: PulledWritingRow,
    download: ReceiveDownloadPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReceiveDownloadPlan {
    Tombstone,
    Install(WritingEnvelopeV1),
}

/// Why settlement did not apply the prepared row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritingReceivePending {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Prepares one queued row without retaining the database across await.
pub(crate) fn prepare_writing_receive(
    conn: &Connection,
    document_id: &str,
) -> Result<Option<PreparedWritingReceive>, WritingReceivePending> {
    let queued = queued_writing_receives(conn)
        .map_err(local_pending)?
        .into_iter()
        .find(|row| row.document_id == document_id);
    let Some(queued) = queued else {
        return Ok(None);
    };
    let scope = current_scope(conn).map_err(enqueue_to_pending)?;
    let stored = parse_stored(&queued.captured_value).map_err(local_pending)?;
    if stored.scope != scope {
        return Err(pending(
            "writing_receive_scope_changed",
            "queued writing row no longer matches the current session",
        ));
    }
    let download = if queued.deleted {
        ReceiveDownloadPlan::Tombstone
    } else {
        let payload = queued.payload.clone().ok_or_else(|| {
            pending(
                "writing_receive_corrupt",
                "queued writing upsert has no payload",
            )
        })?;
        let envelope = WritingEnvelopeV1::from_json(&payload.to_string())
            .map_err(|error| pending("writing_receive_unsupported", error.message))?;
        ReceiveDownloadPlan::Install(envelope)
    };
    Ok(Some(PreparedWritingReceive {
        document_id: queued.document_id.clone(),
        scope,
        captured_value: queued.captured_value,
        row: PulledWritingRow {
            document_id: queued.document_id,
            server_seq: queued.server_seq,
            deleted: queued.deleted,
            changed_at: queued.changed_at,
            device_id: queued.device_id,
            payload: queued.payload,
        },
        download,
    }))
}

/// Downloads blobs for one prepared upsert. Tombstones perform no HTTP.
pub(crate) async fn download_prepared_writing_receive<A: SyncApi>(
    api: &A,
    token: &str,
    data_root: &Path,
    prepared: &PreparedWritingReceive,
    limits: &HealthLimits,
) -> Result<(), super::writing_blobs::WritingBlobPending> {
    let ReceiveDownloadPlan::Install(envelope) = &prepared.download else {
        return Ok(());
    };
    ensure_writing_blobs_installed(api, token, data_root, envelope, limits)
        .await
        .map(|_| ())
}

/// Rechecks session and exact queue identity, then applies and deletes atomically.
pub(crate) fn settle_writing_receive(
    conn: &Connection,
    prepared: &PreparedWritingReceive,
    data_root: &Path,
) -> Result<PullApplyOutcome, WritingReceivePending> {
    let tx = conn.unchecked_transaction().map_err(|error| {
        pending(
            "sql_error",
            format!("failed to start receive settlement: {error}"),
        )
    })?;
    let current = current_scope(&tx).map_err(enqueue_to_pending)?;
    if current != prepared.scope {
        return Err(pending(
            "writing_receive_scope_changed",
            "session changed before writing receive settlement",
        ));
    }
    let stored = read_exact(&tx, &receive_key(&prepared.document_id)).map_err(local_pending)?;
    let Some((_, captured)) = stored else {
        return Err(pending(
            "writing_receive_queue_changed",
            "queued writing row disappeared before settlement",
        ));
    };
    if captured != prepared.captured_value {
        return Err(pending(
            "writing_receive_queue_changed",
            "a newer queued writing row replaced the prepared row",
        ));
    }

    let outcome = apply_pulled_row(&tx, &prepared.scope.device_id, &prepared.row, data_root)
        .map_err(local_pending)?;
    match &outcome {
        PullApplyOutcome::Applied { .. }
        | PullApplyOutcome::NoOp
        | PullApplyOutcome::Stale
        | PullApplyOutcome::OwnChangeObserved => {
            delete_exact(
                &tx,
                &receive_key(&prepared.document_id),
                &prepared.captured_value,
            )
            .map_err(local_pending)?;
        }
        PullApplyOutcome::Deferred { reason } | PullApplyOutcome::Unsupported { reason } => {
            retain_error(&tx, prepared, reason).map_err(local_pending)?;
        }
    }
    tx.commit().map_err(|error| {
        pending(
            "sql_error",
            format!("failed to commit writing receive: {error}"),
        )
    })?;
    Ok(outcome)
}

fn retain_error(
    conn: &Connection,
    prepared: &PreparedWritingReceive,
    reason: &str,
) -> WritingResult<()> {
    let key = receive_key(&prepared.document_id);
    let (mut stored, _) = read_exact(conn, &key)?.ok_or_else(|| {
        WritingError::new(
            "writing_receive_queue_changed",
            "queued writing row disappeared before retention",
        )
    })?;
    stored.attempts = stored.attempts.saturating_add(1);
    stored.last_error = Some(truncate_error(reason));
    let value = serde_json::to_string(&stored)
        .map_err(|error| WritingError::new("writing_receive_corrupt", error.to_string()))?;
    let changed = conn
        .execute(
            "UPDATE sync_meta SET value = ?1 WHERE key = ?2 AND value = ?3",
            params![value, key, prepared.captured_value],
        )
        .map_err(|error| sql_error("Failed to retain writing receive error", error))?;
    if changed != 1 {
        return Err(WritingError::new(
            "writing_receive_queue_changed",
            "queued writing row changed before error retention",
        ));
    }
    Ok(())
}

fn delete_exact(conn: &Connection, key: &str, captured: &str) -> WritingResult<()> {
    let changed = conn
        .execute(
            "DELETE FROM sync_meta WHERE key = ?1 AND value = ?2",
            params![key, captured],
        )
        .map_err(|error| sql_error("Failed to delete settled writing receive", error))?;
    if changed != 1 {
        return Err(WritingError::new(
            "writing_receive_queue_changed",
            "exact queued writing row was not deleted",
        ));
    }
    Ok(())
}

fn parse_stored(value: &str) -> WritingResult<StoredReceive> {
    serde_json::from_str(value).map_err(|error| {
        WritingError::new(
            "writing_receive_corrupt",
            format!("cannot parse writing receive: {error}"),
        )
    })
}

fn enqueue_to_pending(error: WritingReceiveEnqueueError) -> WritingReceivePending {
    WritingReceivePending {
        code: error.code,
        message: error.message,
    }
}

fn local_pending(error: WritingError) -> WritingReceivePending {
    pending("writing_receive_local_state", error.message)
}

fn pending(code: &'static str, message: impl Into<String>) -> WritingReceivePending {
    WritingReceivePending {
        code,
        message: message.into(),
    }
}

/// Durably stores one validated writing row for the current session.
pub(crate) fn enqueue_writing_receive(
    conn: &Connection,
    row: &PullRow,
) -> Result<(), WritingReceiveEnqueueError> {
    let validated = validate_row(row)?;
    let scope = current_scope(conn)?;
    let key = receive_key(&validated.document_id);
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| local_error(format!("failed to start receive enqueue: {error}")))?;
    let existing = read_exact(&tx, &key).map_err(|error| local_error(error.message))?;
    if let Some((stored, _)) = existing {
        if stored.scope != scope {
            return Err(enqueue_error(
                "writing_receive_scope_conflict",
                "queued writing row belongs to another session",
            ));
        }
        if stored.server_seq > validated.server_seq {
            return Ok(());
        }
        if stored.server_seq == validated.server_seq && stored.wire_identity() != validated {
            return Err(enqueue_error(
                "writing_receive_payload_conflict",
                "equal server sequence has conflicting writing payload",
            ));
        }
        if stored.server_seq == validated.server_seq {
            tx.commit().map_err(|error| {
                local_error(format!(
                    "failed to finish idempotent receive enqueue: {error}"
                ))
            })?;
            return Ok(());
        }
    }

    let stored = StoredReceive {
        scope,
        server_seq: validated.server_seq,
        deleted: validated.deleted,
        changed_at: validated.changed_at,
        device_id: validated.device_id,
        payload: validated.payload,
        attempts: 0,
        last_error: None,
    };
    write_exact(&tx, &key, &stored)
        .map_err(|error| local_error(format!("failed to store writing receive: {error}")))?;
    tx.commit()
        .map_err(|error| local_error(format!("failed to commit writing receive: {error}")))
}

/// Reads every well-formed queued row. Corrupt records fail the read.
pub(crate) fn queued_writing_receives(
    conn: &Connection,
) -> WritingResult<Vec<QueuedWritingReceive>> {
    let mut statement = conn
        .prepare(
            "SELECT key, value FROM sync_meta
              WHERE substr(key, 1, length(?1)) = ?1
              ORDER BY key",
        )
        .map_err(|error| sql_error("Failed to read writing receive queue", error))?;
    let rows = statement
        .query_map(params![RECEIVE_PREFIX], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| sql_error("Failed to read writing receive queue", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| sql_error("Failed to read writing receive queue", error))?;

    rows.into_iter()
        .map(|(key, value)| parse_queued(&key, &value))
        .collect()
}

fn parse_queued(key: &str, value: &str) -> WritingResult<QueuedWritingReceive> {
    let document_id = key
        .strip_prefix(RECEIVE_PREFIX)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            WritingError::new(
                "writing_receive_corrupt",
                format!("writing receive key {key:?} has no document id"),
            )
        })?
        .to_string();
    let stored: StoredReceive = serde_json::from_str(value).map_err(|error| {
        WritingError::new(
            "writing_receive_corrupt",
            format!("cannot parse writing receive {key:?}: {error}"),
        )
    })?;
    if stored.server_seq <= 0 || stored.device_id.is_empty() {
        return Err(WritingError::new(
            "writing_receive_corrupt",
            format!("writing receive {key:?} has invalid sequence or device"),
        ));
    }
    Ok(QueuedWritingReceive {
        document_id,
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

fn current_scope(conn: &Connection) -> Result<WritingReceiveScope, WritingReceiveEnqueueError> {
    Ok(WritingReceiveScope {
        account_id: required_meta(conn, "account_id")?,
        server_url: required_meta(conn, "server_url")?,
        device_id: required_meta(conn, "device_id")?,
        server_epoch: required_meta(conn, "server_epoch")?,
        session_incarnation: read_session_incarnation(conn)
            .map_err(local_error)?
            .ok_or_else(|| {
                enqueue_error(
                    "writing_receive_missing_session",
                    format!("sync session metadata {SYNC_SESSION_INCARNATION_KEY:?} is missing"),
                )
            })?,
    })
}

fn required_meta(conn: &Connection, key: &str) -> Result<String, WritingReceiveEnqueueError> {
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
            "writing_receive_missing_session",
            format!("sync session metadata {key:?} is missing"),
        )
    })
}

struct ValidatedRow {
    document_id: String,
    server_seq: i64,
    deleted: bool,
    changed_at: i64,
    device_id: String,
    payload: Option<Value>,
}

impl StoredReceive {
    fn wire_identity(&self) -> ValidatedRow {
        ValidatedRow {
            document_id: String::new(),
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

fn validate_row(row: &PullRow) -> Result<ValidatedRow, WritingReceiveEnqueueError> {
    if row.table != ENVELOPE_TABLE {
        return Err(enqueue_error(
            "writing_receive_wrong_table",
            format!("row table {:?} is not {ENVELOPE_TABLE}", row.table),
        ));
    }
    if row.row_id.is_empty() || row.server_seq <= 0 || row.device_id.is_empty() {
        return Err(enqueue_error(
            "writing_receive_invalid_row",
            "writing row requires nonempty id/device and positive server sequence",
        ));
    }
    if row.deleted == row.payload.is_some() {
        return Err(enqueue_error(
            "writing_receive_invalid_row",
            "writing upsert requires payload and tombstone forbids it",
        ));
    }
    Ok(ValidatedRow {
        document_id: row.row_id.clone(),
        server_seq: row.server_seq,
        deleted: row.deleted,
        changed_at: row.changed_at,
        device_id: row.device_id.clone(),
        payload: row.payload.clone(),
    })
}

fn read_exact(conn: &Connection, key: &str) -> WritingResult<Option<(StoredReceive, String)>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sql_error("Failed to read writing receive", error))?;
    raw.map(|raw| {
        serde_json::from_str(&raw)
            .map(|stored| (stored, raw))
            .map_err(|error| {
                WritingError::new(
                    "writing_receive_corrupt",
                    format!("cannot parse writing receive {key:?}: {error}"),
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

fn receive_key(document_id: &str) -> String {
    format!("{RECEIVE_PREFIX}{document_id}")
}

fn truncate_error(error: &str) -> String {
    error.chars().take(MAX_ERROR_CHARS).collect()
}

fn enqueue_error(code: &'static str, message: impl Into<String>) -> WritingReceiveEnqueueError {
    WritingReceiveEnqueueError {
        code,
        message: message.into(),
    }
}

fn local_error(message: impl Into<String>) -> WritingReceiveEnqueueError {
    enqueue_error("writing_receive_local_state", message)
}

fn sql_error(context: &str, error: rusqlite::Error) -> WritingError {
    WritingError::new("sql_error", format!("{context}: {error}"))
}
