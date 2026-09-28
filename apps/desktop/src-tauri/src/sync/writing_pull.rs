//! Bounded, inactive writing pull page staging for `writing_envelopes`.
//!
//! This module has NO engine caller and never activates the transport. It owns
//! the page unit of the since-zero writing catch-up loop: one explicit
//! [`SyncApi::pull_with_writing_envelope_v1`] request (never ordinary
//! [`SyncApi::pull`]) followed by the durable staging of the returned writing
//! rows into the `writing_receive:` queue that the separate, explicit
//! prepare/download/settle calls drain later.
//!
//! Contract (PROTOCOL "Negociación de capacidades de sync" and the writing
//! aggregate section):
//!
//! - Before any HTTP, the current session scope (account, server URL, device,
//!   server epoch, and session incarnation) must be complete and the exact
//!   `writing-envelope-v1` capability must already have been advertised for the
//!   current server epoch ([`supports_writing`]). Missing scope, missing
//!   incarnation, or missing capability fail locally without touching the wire.
//! - Writing rows are durably enqueued; this module never downloads blobs and
//!   never applies documents. Download and settlement stay separate explicit
//!   calls, so no HTTP happens inside a database transaction.
//! - Non-writing rows are returned untouched as
//!   [`StagedWritingPage::corpus_rows`]: this module neither applies nor
//!   discards them, and it never advances the shared `last_pull_seq` cursor.
//! - The writing-only staging cursor ([`PULL_CURSOR_KEY`]) persists only in the
//!   SAME transaction that durably enqueues that page's writing rows; a failed
//!   enqueue rolls back both the cursor and every queue change of the page.
//! - Re-staging the same page is idempotent (same-sequence identical rows are
//!   no-ops, the cursor never regresses).
//! - Account/server/device/epoch/incarnation mismatch rejects stale staging and
//!   stages no old rows.
//! - Catch-up (`writing_catchup_epoch`, via [`record_catchup_done`]) is recorded
//!   only when the page reports `has_more: false` and the durable queue holds no
//!   deferred or otherwise unresolved rows.

#![allow(dead_code)]

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::engine::read_schema_tag;
use super::http::{PullResponse, PullRow, SyncApi, SyncError};
use super::session::{read_session_incarnation, SYNC_SESSION_INCARNATION_KEY};
use super::writing_receive::{queued_writing_receives, WritingReceiveScope};
use crate::writing::sync_capture::{
    supports_writing, ENVELOPE_TABLE, PULL_CURSOR_KEY, RECEIVE_PREFIX,
};
use crate::writing::sync_transport::{catchup_needed, record_catchup_done};

/// PROTOCOL `limit` ceiling for `GET /v1/sync/pull`; requested limits are
/// clamped into `1..=MAX_PAGE_LIMIT` so one staged page stays bounded.
const MAX_PAGE_LIMIT: i64 = 1_000;

const MISSING_SESSION: &str = "writing_pull_missing_session";
const CAPABILITY_MISSING: &str = "writing_pull_capability_missing";
const STALE_STAGING: &str = "writing_pull_stale_staging";
const LOCAL_STATE: &str = "writing_pull_local_state";
const TRANSPORT: &str = "writing_pull_transport";

// Queue-contract codes shared with `writing_receive`: the durable queue has one
// semantics no matter which module wrote the entry.
const QUEUE_WRONG_TABLE: &str = "writing_receive_wrong_table";
const QUEUE_INVALID_ROW: &str = "writing_receive_invalid_row";
const QUEUE_SCOPE_CONFLICT: &str = "writing_receive_scope_conflict";
const QUEUE_PAYLOAD_CONFLICT: &str = "writing_receive_payload_conflict";

/// Why one writing pull page could not be prepared, fetched, or staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritingPullPending {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Immutable identity + request state carried across the async HTTP boundary.
/// Every identity field comes from the persisted session, never from a guess at
/// settlement time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedWritingPull {
    pub(crate) scope: WritingReceiveScope,
    pub(crate) schema_tag: String,
    pub(crate) since: i64,
    pub(crate) limit: i64,
}

/// What one staged page produced. Corpus rows ride back to the caller untouched.
#[derive(Debug, Clone)]
pub(crate) struct StagedWritingPage {
    /// Non-writing rows, in response order: neither applied nor discarded here.
    pub(crate) corpus_rows: Vec<PullRow>,
    /// Writing rows durably enqueued with this page's transaction.
    pub(crate) staged_document_ids: Vec<String>,
    /// The effective writing-only staging cursor after this page (monotonic).
    pub(crate) next_since: i64,
    pub(crate) has_more: bool,
    /// True only when this page recorded the epoch's writing catch-up.
    pub(crate) catchup_recorded: bool,
}

/// Byte-compatible mirror of `writing_receive::StoredReceive` (identical serde
/// field names, order, and optionality). The page transaction must enqueue rows
/// and advance the staging cursor in ONE SQLite transaction, while
/// `enqueue_writing_receive` owns its own `BEGIN` and can never run inside a
/// caller transaction. This module therefore writes the identical durable queue
/// value shape; `writing_pull_tests` locks the contract by reading staged rows
/// back through `queued_writing_receives`/`prepare_writing_receive` and by
/// replaying rows first enqueued through `enqueue_writing_receive`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StagedReceiveValue {
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

/// The writing-only staging cursor: where the writing pull stands, bound to the
/// exact session that advanced it. A cursor written by another account, epoch,
/// or incarnation is stale staging and is rejected, never reused.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StagedPullCursor {
    scope: WritingReceiveScope,
    since: i64,
}

/// Prepares exactly one bounded page request. Fails locally (and reaches no
/// HTTP) unless the session scope with incarnation is complete, the exact
/// `writing-envelope-v1` capability was advertised for the current epoch, and
/// any stored staging cursor belongs to this very session.
pub(crate) fn prepare_writing_pull(
    conn: &Connection,
    limit: i64,
) -> Result<PreparedWritingPull, WritingPullPending> {
    let scope = read_scope(conn)?;
    if !supports_writing(conn, &scope.server_epoch).map_err(local_from_writing)? {
        return Err(pending(
            CAPABILITY_MISSING,
            "writing-envelope-v1 was not advertised for the current server epoch",
        ));
    }
    let since = match read_cursor(conn)? {
        Some(cursor) => {
            if cursor.scope != scope {
                return Err(pending(
                    STALE_STAGING,
                    "staged writing pull cursor belongs to another account, epoch, or session",
                ));
            }
            cursor.since
        }
        None => 0,
    };
    let schema_tag =
        read_schema_tag(conn).map_err(|message| pending(LOCAL_STATE, message.to_string()))?;
    Ok(PreparedWritingPull {
        scope,
        schema_tag,
        since,
        limit: limit.clamp(1, MAX_PAGE_LIMIT),
    })
}

/// Issues exactly one explicit `writing-envelope-v1` opted-in page request. The
/// [`SyncApi`] default for `pull_with_writing_envelope_v1` fails locally instead
/// of delegating to ordinary [`SyncApi::pull`], so this call can never turn into
/// a capability-free pull. No database handle crosses this await.
pub(crate) async fn fetch_writing_pull_page<A: SyncApi>(
    api: &A,
    token: &str,
    prepared: &PreparedWritingPull,
) -> Result<PullResponse, WritingPullPending> {
    api.pull_with_writing_envelope_v1(token, &prepared.schema_tag, prepared.since, prepared.limit)
        .await
        .map_err(transport_pending)
}

/// Stages one fetched page in a single transaction: enqueue this page's writing
/// rows into the durable queue AND persist the writing-only staging cursor
/// together. Any failure rolls back the cursor and every queue change of the
/// page. Corpus rows are returned untouched; `last_pull_seq` is never written.
pub(crate) fn stage_writing_pull_page(
    conn: &Connection,
    prepared: &PreparedWritingPull,
    response: &PullResponse,
) -> Result<StagedWritingPage, WritingPullPending> {
    let mut corpus_rows = Vec::new();
    let mut writing_rows = Vec::new();
    for row in &response.rows {
        if row.table == ENVELOPE_TABLE {
            writing_rows.push(row);
        } else {
            corpus_rows.push(row.clone());
        }
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| sql_pending("Failed to start writing pull staging", error))?;

    let current = read_scope(&tx)?;
    if current != prepared.scope {
        return Err(pending(
            STALE_STAGING,
            "session changed since the writing pull was prepared; the page is not staged",
        ));
    }
    if response.server_epoch != current.server_epoch {
        return Err(pending(
            STALE_STAGING,
            "pull response server epoch does not match the current session; the page is not staged",
        ));
    }
    if !response.supports_writing_envelope_v1() {
        return Err(pending(
            CAPABILITY_MISSING,
            "pull response did not advertise the exact writing-envelope-v1 capability",
        ));
    }
    let previous_since = match read_cursor(&tx)? {
        Some(cursor) => {
            if cursor.scope != current {
                return Err(pending(
                    STALE_STAGING,
                    "staged writing pull cursor belongs to another account, epoch, or session",
                ));
            }
            cursor.since
        }
        None => 0,
    };

    let mut staged_document_ids = Vec::with_capacity(writing_rows.len());
    for row in &writing_rows {
        staged_document_ids.push(stage_row(&tx, &current, row)?);
    }

    let next_since = previous_since.max(response.next_since);
    write_cursor(&tx, &current, next_since)?;

    let mut catchup_recorded = false;
    if !response.has_more
        && queued_writing_receives(&tx)
            .map_err(local_from_writing)?
            .is_empty()
        && catchup_needed(&tx, &current.server_epoch).map_err(local_from_writing)?
    {
        record_catchup_done(&tx, &current.server_epoch).map_err(local_from_writing)?;
        catchup_recorded = true;
    }

    tx.commit()
        .map_err(|error| sql_pending("Failed to commit writing pull staging", error))?;

    Ok(StagedWritingPage {
        corpus_rows,
        staged_document_ids,
        next_since,
        has_more: response.has_more,
        catchup_recorded,
    })
}

/// The inactive orchestration of one bounded page: prepare (scope, incarnation,
/// exact advertised capability), one explicit opted-in fetch, then the
/// transactional staging. The connection is idle across the await and no
/// transaction ever spans HTTP.
pub(crate) async fn pull_writing_page<A: SyncApi>(
    conn: &Connection,
    api: &A,
    token: &str,
    limit: i64,
) -> Result<StagedWritingPage, WritingPullPending> {
    let prepared = prepare_writing_pull(conn, limit)?;
    let response = fetch_writing_pull_page(api, token, &prepared).await?;
    stage_writing_pull_page(conn, &prepared, &response)
}

/// Durably enqueues one validated writing row for the current session inside the
/// caller's page transaction. Mirrors `enqueue_writing_receive`'s compare-and-set
/// semantics: newer sequences replace, identical same-sequence rows are no-ops,
/// conflicting same-sequence rows and foreign-session rows are rejected, and
/// older sequences never overwrite newer staged state.
fn stage_row(
    conn: &Connection,
    scope: &WritingReceiveScope,
    row: &PullRow,
) -> Result<String, WritingPullPending> {
    validate_writing_row(row)?;
    let key = receive_key(&row.row_id);
    if let Some(stored) = read_staged(conn, &key)? {
        if stored.scope != *scope {
            return Err(pending(
                QUEUE_SCOPE_CONFLICT,
                format!(
                    "queued writing row {:?} belongs to another session",
                    row.row_id
                ),
            ));
        }
        if stored.server_seq > row.server_seq {
            return Ok(row.row_id.clone());
        }
        if stored.server_seq == row.server_seq {
            if same_wire_identity(&stored, row) {
                return Ok(row.row_id.clone());
            }
            return Err(pending(
                QUEUE_PAYLOAD_CONFLICT,
                format!(
                    "equal server sequence has conflicting writing payload for {:?}",
                    row.row_id
                ),
            ));
        }
    }

    let value = StagedReceiveValue {
        scope: scope.clone(),
        server_seq: row.server_seq,
        deleted: row.deleted,
        changed_at: row.changed_at,
        device_id: row.device_id.clone(),
        payload: row.payload.clone(),
        attempts: 0,
        last_error: None,
    };
    let raw = serde_json::to_string(&value).map_err(|error| {
        pending(
            LOCAL_STATE,
            format!(
                "cannot serialize queued writing row {:?}: {error}",
                row.row_id
            ),
        )
    })?;
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, raw],
    )
    .map_err(|error| sql_pending("Failed to enqueue staged writing row", error))?;
    Ok(row.row_id.clone())
}

/// Same row contract as `writing_receive`: a well-formed writing row is an
/// upsert with payload or a tombstone without one, with positive sequence and
/// nonempty identity.
fn validate_writing_row(row: &PullRow) -> Result<(), WritingPullPending> {
    if row.table != ENVELOPE_TABLE {
        return Err(pending(
            QUEUE_WRONG_TABLE,
            format!("row table {:?} is not {ENVELOPE_TABLE}", row.table),
        ));
    }
    if row.row_id.is_empty() || row.server_seq <= 0 || row.device_id.is_empty() {
        return Err(pending(
            QUEUE_INVALID_ROW,
            "writing row requires nonempty id/device and positive server sequence",
        ));
    }
    if row.deleted == row.payload.is_some() {
        return Err(pending(
            QUEUE_INVALID_ROW,
            "writing upsert requires payload and tombstone forbids it",
        ));
    }
    Ok(())
}

/// Wire identity of one queued row: everything the server may change at the
/// same sequence (document id excluded, as in `enqueue_writing_receive`).
fn same_wire_identity(stored: &StagedReceiveValue, row: &PullRow) -> bool {
    stored.server_seq == row.server_seq
        && stored.deleted == row.deleted
        && stored.changed_at == row.changed_at
        && stored.device_id == row.device_id
        && stored.payload == row.payload
}

fn read_scope(conn: &Connection) -> Result<WritingReceiveScope, WritingPullPending> {
    Ok(WritingReceiveScope {
        account_id: required_meta(conn, "account_id")?,
        server_url: required_meta(conn, "server_url")?,
        device_id: required_meta(conn, "device_id")?,
        server_epoch: required_meta(conn, "server_epoch")?,
        session_incarnation: read_session_incarnation(conn)
            .map_err(|message| pending(LOCAL_STATE, message))?
            .ok_or_else(|| {
                pending(
                    MISSING_SESSION,
                    format!("sync session metadata {SYNC_SESSION_INCARNATION_KEY:?} is missing"),
                )
            })?,
    })
}

fn required_meta(conn: &Connection, key: &str) -> Result<String, WritingPullPending> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sql_pending("Failed to read sync session metadata", error))?
        .filter(|value: &String| !value.is_empty());
    value.ok_or_else(|| {
        pending(
            MISSING_SESSION,
            format!("sync session metadata {key:?} is missing"),
        )
    })
}

fn read_cursor(conn: &Connection) -> Result<Option<StagedPullCursor>, WritingPullPending> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![PULL_CURSOR_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sql_pending("Failed to read writing staging cursor", error))?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| {
            pending(
                LOCAL_STATE,
                format!("cannot parse writing staging cursor: {error}"),
            )
        })
    })
    .transpose()
}

fn write_cursor(
    conn: &Connection,
    scope: &WritingReceiveScope,
    since: i64,
) -> Result<(), WritingPullPending> {
    let cursor = StagedPullCursor {
        scope: scope.clone(),
        since,
    };
    let raw = serde_json::to_string(&cursor).map_err(|error| {
        pending(
            LOCAL_STATE,
            format!("cannot serialize writing staging cursor: {error}"),
        )
    })?;
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![PULL_CURSOR_KEY, raw],
    )
    .map(|_| ())
    .map_err(|error| sql_pending("Failed to persist writing staging cursor", error))
}

fn read_staged(
    conn: &Connection,
    key: &str,
) -> Result<Option<StagedReceiveValue>, WritingPullPending> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sql_pending("Failed to read staged writing row", error))?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| {
            pending(
                LOCAL_STATE,
                format!("cannot parse staged writing row {key:?}: {error}"),
            )
        })
    })
    .transpose()
}

fn receive_key(document_id: &str) -> String {
    format!("{RECEIVE_PREFIX}{document_id}")
}

fn transport_pending(error: SyncError) -> WritingPullPending {
    let message = match &error {
        SyncError::InvalidUrl(message)
        | SyncError::Network(message)
        | SyncError::Decode(message) => message.clone(),
        SyncError::Api {
            status,
            code,
            message,
        } => format!("{status} {code}: {message}"),
    };
    pending(TRANSPORT, message)
}

fn local_from_writing(error: crate::writing::repository::WritingError) -> WritingPullPending {
    pending(LOCAL_STATE, error.message)
}

fn sql_pending(context: &str, error: rusqlite::Error) -> WritingPullPending {
    pending(LOCAL_STATE, format!("{context}: {error}"))
}

fn pending(code: &'static str, message: impl Into<String>) -> WritingPullPending {
    WritingPullPending {
        code,
        message: message.into(),
    }
}
