//! Bounded live research pull page staging for `research_envelopes`.
//!
//! This module owns the page unit of the since-zero research catch-up loop:
//! one explicit [`SyncApi::pull_with_research_envelope_v1`] request (never
//! ordinary [`SyncApi::pull`]) followed by the durable staging of the returned
//! research rows into the `research_receive:` queue that the separate settle
//! calls drain later.
//!
//! Contract (PROTOCOL "Negociación de capacidades de sync" and the research
//! aggregate section, mirrored from `writing_pull`):
//!
//! - Before any HTTP, the current session scope (account, server URL, device,
//!   server epoch, and session incarnation) must be complete and the exact
//!   `research-envelope-v1` capability must already have been advertised for
//!   the current server epoch ([`super::research_capture::supports_research`]).
//!   Missing scope, missing incarnation, or missing capability fail locally
//!   without touching the wire.
//! - Research rows are durably enqueued; this module never downloads report
//!   blobs and never applies projections. Download and settlement stay separate
//!   explicit calls, so no HTTP happens inside a database transaction.
//! - Non-research rows are returned untouched as
//!   [`StagedResearchPage::corpus_rows`]: this module neither applies nor
//!   discards them, and it never advances the shared `last_pull_seq` cursor.
//! - The research-only staging cursor ([`PULL_CURSOR_KEY`]) persists only in
//!   the SAME transaction that durably enqueues that page's research rows; a
//!   failed enqueue rolls back both the cursor and every queue change of the
//!   page, so no cursor ever advances across rows the queue does not hold.
//! - Re-staging the same page is idempotent (same-sequence identical rows are
//!   no-ops, the cursor never regresses). Conflicting equal-sequence rows,
//!   foreign-scope rows, and structurally invalid rows fail closed and retain
//!   the previous durable state.
//! - Account/server/device/epoch/incarnation mismatch rejects stale staging and
//!   stages no old rows.
//! - Catch-up ([`super::research_capture::CATCHUP_KEY`], via
//!   [`record_catchup_done`]) is recorded only
//!   when the page reports `has_more: false` and the durable queue holds no
//!   unresolved rows.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::engine::read_schema_tag;
use super::http::{PullResponse, PullRow, SyncApi, SyncError};
use super::research_capture::{
    catchup_needed, record_catchup_done, supports_research, ENVELOPE_TABLE, PULL_CURSOR_KEY,
};
use super::research_envelope::ResearchError;
use super::research_receive::{
    queued_research_receives, stage_research_receive, ResearchReceiveEnqueueError,
    ResearchReceiveScope,
};
use super::session::{read_session_incarnation, SYNC_SESSION_INCARNATION_KEY};

/// PROTOCOL `limit` ceiling for `GET /v1/sync/pull`; requested limits are
/// clamped into `1..=MAX_PAGE_LIMIT` so one staged page stays bounded.
const MAX_PAGE_LIMIT: i64 = 1_000;

const MISSING_SESSION: &str = "research_pull_missing_session";
const CAPABILITY_MISSING: &str = "research_pull_capability_missing";
const STALE_STAGING: &str = "research_pull_stale_staging";
const LOCAL_STATE: &str = "research_pull_local_state";
const TRANSPORT: &str = "research_pull_transport";

/// Why one research pull page could not be prepared, fetched, or staged.
/// Queue-contract codes from `research_receive` pass through unchanged: the
/// durable queue has one semantics no matter which module wrote the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchPullPending {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Immutable identity + request state carried across the async HTTP boundary.
/// Every identity field comes from the persisted session, never from a guess at
/// settlement time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedResearchPull {
    pub(crate) scope: ResearchReceiveScope,
    pub(crate) schema_tag: String,
    pub(crate) since: i64,
    pub(crate) limit: i64,
}

/// What one staged page produced. Corpus rows ride back to the caller untouched.
#[derive(Debug, Clone)]
pub(crate) struct StagedResearchPage {
    /// Non-research rows, in response order: neither applied nor discarded here.
    pub(crate) corpus_rows: Vec<PullRow>,
    /// Research rows durably enqueued with this page's transaction.
    pub(crate) staged_job_ids: Vec<String>,
    pub(crate) has_more: bool,
    /// True only when this page recorded the epoch's research catch-up.
    pub(crate) catchup_recorded: bool,
}

/// The research-only staging cursor: where the research pull stands, bound to
/// the exact session that advanced it. A cursor written by another account,
/// epoch, or incarnation is stale staging and is rejected, never reused.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StagedPullCursor {
    scope: ResearchReceiveScope,
    since: i64,
}

/// Prepares exactly one bounded page request. Fails locally (and reaches no
/// HTTP) unless the session scope with incarnation is complete, the exact
/// `research-envelope-v1` capability was advertised for the current epoch, and
/// any stored staging cursor belongs to this very session. A missing cursor
/// starts the since-zero catch-up at `since=0`.
pub(crate) fn prepare_research_pull(
    conn: &Connection,
    limit: i64,
) -> Result<PreparedResearchPull, ResearchPullPending> {
    let scope = read_scope(conn)?;
    if !supports_research(conn, &scope.server_epoch).map_err(local_from_research)? {
        return Err(pending(
            CAPABILITY_MISSING,
            "research-envelope-v1 was not advertised for the current server epoch",
        ));
    }
    let since = match read_cursor(conn)? {
        Some(cursor) => {
            if cursor.scope != scope {
                return Err(pending(
                    STALE_STAGING,
                    "staged research pull cursor belongs to another account, epoch, or session",
                ));
            }
            cursor.since
        }
        None => 0,
    };
    let schema_tag =
        read_schema_tag(conn).map_err(|message| pending(LOCAL_STATE, message.to_string()))?;
    Ok(PreparedResearchPull {
        scope,
        schema_tag,
        since,
        limit: limit.clamp(1, MAX_PAGE_LIMIT),
    })
}

/// Issues exactly one explicit `research-envelope-v1` opted-in page request. The
/// [`SyncApi`] default for `pull_with_research_envelope_v1` fails locally
/// instead of delegating to ordinary [`SyncApi::pull`], so this call can never
/// turn into a capability-free pull. No database handle crosses this await.
pub(crate) async fn fetch_research_pull_page<A: SyncApi>(
    api: &A,
    token: &str,
    prepared: &PreparedResearchPull,
) -> Result<PullResponse, ResearchPullPending> {
    api.pull_with_research_envelope_v1(token, &prepared.schema_tag, prepared.since, prepared.limit)
        .await
        .map_err(transport_pending)
}

/// Stages one fetched page in a single transaction: enqueue this page's
/// research rows into the durable queue AND persist the research-only staging
/// cursor together. Any failure rolls back the cursor and every queue change of
/// the page. Corpus rows are returned untouched; `last_pull_seq` is never
/// written.
pub(crate) fn stage_research_pull_page(
    conn: &Connection,
    prepared: &PreparedResearchPull,
    response: &PullResponse,
) -> Result<StagedResearchPage, ResearchPullPending> {
    let mut corpus_rows = Vec::new();
    let mut research_rows = Vec::new();
    for row in &response.rows {
        if row.table == ENVELOPE_TABLE {
            research_rows.push(row);
        } else {
            corpus_rows.push(row.clone());
        }
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| sql_pending("Failed to start research pull staging", error))?;

    let current = read_scope(&tx)?;
    if current != prepared.scope {
        return Err(pending(
            STALE_STAGING,
            "session changed since the research pull was prepared; the page is not staged",
        ));
    }
    if response.server_epoch != current.server_epoch {
        return Err(pending(
            STALE_STAGING,
            "pull response server epoch does not match the current session; the page is not staged",
        ));
    }
    if !response.supports_research_envelope_v1() {
        return Err(pending(
            CAPABILITY_MISSING,
            "pull response did not advertise the exact research-envelope-v1 capability",
        ));
    }
    let previous_since = match read_cursor(&tx)? {
        Some(cursor) => {
            if cursor.scope != current {
                return Err(pending(
                    STALE_STAGING,
                    "staged research pull cursor belongs to another account, epoch, or session",
                ));
            }
            cursor.since
        }
        None => 0,
    };

    let mut staged_job_ids = Vec::with_capacity(research_rows.len());
    for row in &research_rows {
        stage_research_receive(&tx, &current, row).map_err(from_enqueue_error)?;
        staged_job_ids.push(row.row_id.clone());
    }

    let next_since = previous_since.max(response.next_since);
    write_cursor(&tx, &current, next_since)?;

    let mut catchup_recorded = false;
    if !response.has_more
        && queued_research_receives(&tx)
            .map_err(local_from_research)?
            .is_empty()
        && catchup_needed(&tx, &current.server_epoch).map_err(local_from_research)?
    {
        record_catchup_done(&tx, &current.server_epoch).map_err(local_from_research)?;
        catchup_recorded = true;
    }

    tx.commit()
        .map_err(|error| sql_pending("Failed to commit research pull staging", error))?;

    Ok(StagedResearchPage {
        corpus_rows,
        staged_job_ids,
        has_more: response.has_more,
        catchup_recorded,
    })
}

/// The bounded orchestration of one page: prepare (scope, incarnation,
/// exact advertised capability), one explicit opted-in fetch, then the
/// transactional staging. The connection is idle across the await and no
/// transaction ever spans HTTP.
pub(crate) async fn pull_research_page<A: SyncApi>(
    conn: &Connection,
    api: &A,
    token: &str,
    limit: i64,
) -> Result<StagedResearchPage, ResearchPullPending> {
    let prepared = prepare_research_pull(conn, limit)?;
    let response = fetch_research_pull_page(api, token, &prepared).await?;
    stage_research_pull_page(conn, &prepared, &response)
}

fn read_scope(conn: &Connection) -> Result<ResearchReceiveScope, ResearchPullPending> {
    Ok(ResearchReceiveScope {
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

fn required_meta(conn: &Connection, key: &str) -> Result<String, ResearchPullPending> {
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

fn read_cursor(conn: &Connection) -> Result<Option<StagedPullCursor>, ResearchPullPending> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![PULL_CURSOR_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sql_pending("Failed to read research staging cursor", error))?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| {
            pending(
                LOCAL_STATE,
                format!("cannot parse research staging cursor: {error}"),
            )
        })
    })
    .transpose()
}

fn write_cursor(
    conn: &Connection,
    scope: &ResearchReceiveScope,
    since: i64,
) -> Result<(), ResearchPullPending> {
    let cursor = StagedPullCursor {
        scope: scope.clone(),
        since,
    };
    let raw = serde_json::to_string(&cursor).map_err(|error| {
        pending(
            LOCAL_STATE,
            format!("cannot serialize research staging cursor: {error}"),
        )
    })?;
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![PULL_CURSOR_KEY, raw],
    )
    .map(|_| ())
    .map_err(|error| sql_pending("Failed to persist research staging cursor", error))
}

fn from_enqueue_error(error: ResearchReceiveEnqueueError) -> ResearchPullPending {
    pending(error.code, error.message)
}

fn transport_pending(error: SyncError) -> ResearchPullPending {
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

fn local_from_research(error: ResearchError) -> ResearchPullPending {
    pending(LOCAL_STATE, error.message)
}

fn sql_pending(context: &str, error: rusqlite::Error) -> ResearchPullPending {
    pending(LOCAL_STATE, format!("{context}: {error}"))
}

fn pending(code: &'static str, message: impl Into<String>) -> ResearchPullPending {
    ResearchPullPending {
        code,
        message: message.into(),
    }
}
