//! Bounded, inactive push orchestration for `writing_envelopes`.
//!
//! This module deliberately has no engine caller. It separates database-backed
//! preparation, async blob/envelope transport, and database-backed settlement so
//! no SQLite connection or transaction crosses an await. Automatic activation
//! remains unsafe until writing catch-up, receive/own-echo restoration, durable
//! deferrals, backfill, and editor protection are integrated end to end.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::http::{
    HealthLimits, HealthResponse, PullRow, PushChange, PushRequest, PushResponse, PushResult,
    SyncApi, SyncError,
};
use super::writing_blobs::{
    ensure_writing_blobs_uploaded, WritingBlobPending, WritingBlobPendingKind,
};
use crate::writing::sync_capture::{supports_writing, ENVELOPE_TABLE};
use crate::writing::sync_envelope::WritingEnvelopeV1;
use crate::writing::sync_transport::{
    build_push_changes, catchup_needed, recorded_server_seq, settle_applied_push_draft,
    PulledWritingRow, PushDraftSettlement, WritingChangeDraft,
};

const PREPARE_SAVEPOINT: &str = "writing_push_prepare";
const SETTLE_SAVEPOINT: &str = "writing_push_settle";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WritingPushPendingKind {
    MissingSession,
    EpochMismatch,
    CapabilityUnavailable,
    CatchupIncomplete,
    LocalFiles,
    InvalidLimits,
    RequestTooLarge,
    InvalidDraft,
    Blob(WritingBlobPendingKind),
    Network,
    Unauthorized,
    AccessDenied,
    RemoteRejected,
    MalformedResponse,
    SessionChanged,
    LocalState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritingPushPending {
    pub(crate) kind: WritingPushPendingKind,
    pub(crate) document_ids: Vec<String>,
    pub(crate) message: String,
}

impl WritingPushPending {
    fn new(kind: WritingPushPendingKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            document_ids: Vec::new(),
            message: message.into(),
        }
    }

    fn for_document(
        kind: WritingPushPendingKind,
        document_id: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            document_ids: vec![document_id.to_string()],
            message: message.into(),
        }
    }

    fn for_documents(
        kind: WritingPushPendingKind,
        document_ids: Vec<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            document_ids,
            message: message.into(),
        }
    }
}

/// Preparation either yields one bounded single-document request, finds no
/// work, or explains why existing outbox work must remain pending.
#[allow(clippy::large_enum_variant)] // short-lived return value, built once per push; boxing would only add churn
pub(crate) enum WritingPushPreparation {
    Ready(PreparedWritingPush),
    Idle,
    Pending(WritingPushPending),
}

/// Immutable state carried across the async network boundary. Every identity
/// field comes from the persisted session, not from a settlement-time guess.
pub(crate) struct PreparedWritingPush {
    pub(crate) document_id: String,
    pub(crate) serialized_request_bytes: usize,
    binding: SessionBinding,
    schema_tag: String,
    limits: HealthLimits,
    data_root: PathBuf,
    envelope: WritingEnvelopeV1,
    envelope_fingerprint_sha256: String,
    request: PushRequest,
    draft: WritingChangeDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionBinding {
    account_id: String,
    server_url: String,
    device_id: String,
    server_epoch: String,
    session_incarnation: Uuid,
}

#[derive(Debug)]
pub(crate) struct CompletedWritingPush {
    binding: SessionBinding,
    draft: WritingChangeDraft,
    result: ValidatedPushResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WritingPushAcceptedStatus {
    Applied,
    LwwWon,
}

#[derive(Debug)]
enum ValidatedPushResult {
    Accepted {
        status: WritingPushAcceptedStatus,
        server_seq: i64,
    },
    Conflict {
        server_seq: i64,
        winner: PulledWritingRow,
    },
}

impl ValidatedPushResult {
    fn server_seq(&self) -> i64 {
        match self {
            Self::Accepted { server_seq, .. } | Self::Conflict { server_seq, .. } => *server_seq,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WritingPushOutcome {
    Acknowledged {
        document_id: String,
        server_seq: i64,
        status: WritingPushAcceptedStatus,
    },
    NewerGenerationPending {
        document_id: String,
        server_seq: i64,
        status: WritingPushAcceptedStatus,
    },
    AlreadySettled {
        document_id: String,
        server_seq: i64,
        status: WritingPushAcceptedStatus,
    },
    StaleResponse {
        document_id: String,
        response_server_seq: i64,
        recorded_server_seq: i64,
    },
    ConflictPending {
        document_id: String,
        server_seq: i64,
        winner: PulledWritingRow,
    },
}

/// Captures one coherent outbox/document snapshot and preflights the exact JSON
/// request body against the server-advertised byte limit. This helper only
/// reads catch-up evidence; it never marks catch-up complete.
pub(crate) fn prepare_writing_push(
    conn: &Connection,
    data_root: &Path,
    health: &HealthResponse,
) -> WritingPushPreparation {
    match with_local_savepoint(conn, PREPARE_SAVEPOINT, || {
        prepare_writing_push_in_savepoint(conn, data_root, health)
    }) {
        Ok(Some(prepared)) => WritingPushPreparation::Ready(prepared),
        Ok(None) => WritingPushPreparation::Idle,
        Err(pending) => WritingPushPreparation::Pending(pending),
    }
}

fn prepare_writing_push_in_savepoint(
    conn: &Connection,
    data_root: &Path,
    health: &HealthResponse,
) -> Result<Option<PreparedWritingPush>, WritingPushPending> {
    if health.epoch.is_empty() {
        return Err(WritingPushPending::new(
            WritingPushPendingKind::EpochMismatch,
            "health response did not identify a server epoch",
        ));
    }

    let binding = read_session_binding(conn, WritingPushPendingKind::MissingSession)?;
    if binding.server_epoch != health.epoch {
        return Err(WritingPushPending::new(
            WritingPushPendingKind::EpochMismatch,
            "persisted session epoch does not match the current health response",
        ));
    }
    require_writing_gate(conn, &binding.server_epoch)?;

    let max_push_bytes = usize::try_from(health.limits.max_push_bytes)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            WritingPushPending::new(
                WritingPushPendingKind::InvalidLimits,
                "server did not advertise a positive max_push_bytes limit",
            )
        })?;
    let schema_tag = super::engine::read_schema_tag(conn)
        .map_err(|message| WritingPushPending::new(WritingPushPendingKind::LocalState, message))?;
    let clock_offset = super::push::clock_offset(conn)
        .map_err(|message| WritingPushPending::new(WritingPushPendingKind::LocalState, message))?;
    let build = build_push_changes(conn, data_root).map_err(pending_from_writing)?;
    let Some(draft) = build.ready.into_iter().next() else {
        if build.pending_files.is_empty() {
            return Ok(None);
        }
        return Err(WritingPushPending::for_documents(
            WritingPushPendingKind::LocalFiles,
            build.pending_files,
            "writing outbox documents are missing or have unprovable local files",
        ));
    };

    let envelope = WritingEnvelopeV1::from_json(&draft.payload.to_string()).map_err(|error| {
        WritingPushPending::for_document(
            WritingPushPendingKind::InvalidDraft,
            &draft.document_id,
            error.message,
        )
    })?;
    if envelope.id != draft.document_id {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::InvalidDraft,
            &draft.document_id,
            "writing envelope id does not match its outbox document",
        ));
    }
    let op = match draft.op {
        'U' => "upsert".to_string(),
        other => {
            return Err(WritingPushPending::for_document(
                WritingPushPendingKind::InvalidDraft,
                &draft.document_id,
                format!("unsupported writing outbox operation {other:?}"),
            ));
        }
    };
    let request = PushRequest {
        changes: vec![PushChange {
            table: ENVELOPE_TABLE.to_string(),
            row_id: draft.document_id.clone(),
            op,
            changed_at: draft.changed_at.saturating_add(clock_offset),
            base_seq: draft.base_seq,
            payload: Some(draft.payload.clone()),
        }],
    };
    let serialized_request_bytes = serde_json::to_vec(&request)
        .map_err(|error| {
            WritingPushPending::for_document(
                WritingPushPendingKind::InvalidDraft,
                &draft.document_id,
                format!("cannot serialize writing push request: {error}"),
            )
        })?
        .len();
    if serialized_request_bytes > max_push_bytes {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::RequestTooLarge,
            &draft.document_id,
            format!(
                "writing push request is {serialized_request_bytes} bytes; advertised limit is {max_push_bytes} bytes"
            ),
        ));
    }
    let envelope_fingerprint_sha256 = envelope.fingerprint_sha256().map_err(|error| {
        WritingPushPending::for_document(
            WritingPushPendingKind::InvalidDraft,
            &draft.document_id,
            error.message,
        )
    })?;

    Ok(Some(PreparedWritingPush {
        document_id: draft.document_id.clone(),
        serialized_request_bytes,
        binding,
        schema_tag,
        limits: health.limits.clone(),
        data_root: data_root.to_path_buf(),
        envelope,
        envelope_fingerprint_sha256,
        request,
        draft,
    }))
}

/// Uploads every referenced blob, then sends the exact opt-in envelope request.
/// No database handle is accepted, so this async phase cannot hold a SQLite
/// lock or acknowledge local work. The caller must still bind both `api` and
/// `token` to the prepared session; incarnation only rejects stale settlement
/// and cannot prevent disclosure through a mismatched endpoint or credential.
pub(crate) async fn send_prepared_writing_push<A: SyncApi>(
    api: &A,
    token: &str,
    prepared: PreparedWritingPush,
) -> Result<CompletedWritingPush, WritingPushPending> {
    let PreparedWritingPush {
        document_id,
        binding,
        schema_tag,
        limits,
        data_root,
        envelope,
        envelope_fingerprint_sha256,
        request,
        draft,
        ..
    } = prepared;

    let uploaded = ensure_writing_blobs_uploaded(api, token, &data_root, &envelope, &limits)
        .await
        .map_err(|error| pending_from_blob(&document_id, error))?;
    if uploaded.envelope_fingerprint_sha256 != envelope_fingerprint_sha256 {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::InvalidDraft,
            &document_id,
            "blob upload proof does not match the prepared writing envelope",
        ));
    }

    let response = api
        .push_with_writing_envelope_v1(token, &schema_tag, request)
        .await
        .map_err(|error| pending_from_sync(&document_id, error))?;
    let result = validate_push_response(response, &document_id, &binding.server_epoch)?;
    Ok(CompletedWritingPush {
        binding,
        draft,
        result,
    })
}

/// Rechecks the persisted account/server/device/epoch scope, then settles the
/// validated response in a savepoint. `lww_lost` stays pending and returns its
/// winner for a future receive stage; this slice never overwrites local text.
pub(crate) fn settle_writing_push(
    conn: &Connection,
    completed: CompletedWritingPush,
) -> Result<WritingPushOutcome, WritingPushPending> {
    with_local_savepoint(conn, SETTLE_SAVEPOINT, || {
        settle_writing_push_in_savepoint(conn, completed)
    })
}

fn settle_writing_push_in_savepoint(
    conn: &Connection,
    completed: CompletedWritingPush,
) -> Result<WritingPushOutcome, WritingPushPending> {
    let current = read_session_binding(conn, WritingPushPendingKind::SessionChanged)?;
    if current != completed.binding {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::SessionChanged,
            &completed.draft.document_id,
            "sync session changed after the writing request was prepared",
        ));
    }
    require_writing_gate(conn, &completed.binding.server_epoch)?;

    let response_server_seq = completed.result.server_seq();
    let recorded =
        recorded_server_seq(conn, &completed.draft.document_id).map_err(pending_from_writing)?;
    if recorded > response_server_seq {
        return Ok(WritingPushOutcome::StaleResponse {
            document_id: completed.draft.document_id,
            response_server_seq,
            recorded_server_seq: recorded,
        });
    }

    match completed.result {
        ValidatedPushResult::Conflict { server_seq, winner } => {
            Ok(WritingPushOutcome::ConflictPending {
                document_id: completed.draft.document_id,
                server_seq,
                winner,
            })
        }
        ValidatedPushResult::Accepted { status, server_seq } => {
            let document_id = completed.draft.document_id.clone();
            let settlement = settle_applied_push_draft(conn, &completed.draft, server_seq)
                .map_err(pending_from_writing)?;
            Ok(match settlement {
                PushDraftSettlement::Acknowledged => WritingPushOutcome::Acknowledged {
                    document_id,
                    server_seq,
                    status,
                },
                PushDraftSettlement::NewerGenerationPending => {
                    WritingPushOutcome::NewerGenerationPending {
                        document_id,
                        server_seq,
                        status,
                    }
                }
                PushDraftSettlement::AlreadySettled => WritingPushOutcome::AlreadySettled {
                    document_id,
                    server_seq,
                    status,
                },
                PushDraftSettlement::StaleServerSequence {
                    recorded_server_seq,
                    response_server_seq,
                } => WritingPushOutcome::StaleResponse {
                    document_id,
                    response_server_seq,
                    recorded_server_seq,
                },
            })
        }
    }
}

fn validate_push_response(
    response: PushResponse,
    document_id: &str,
    expected_epoch: &str,
) -> Result<ValidatedPushResult, WritingPushPending> {
    if response.server_epoch != expected_epoch {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::EpochMismatch,
            document_id,
            "writing push response epoch does not match the prepared epoch",
        ));
    }
    if !response.supports_writing_envelope_v1() {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::CapabilityUnavailable,
            document_id,
            "writing push response omitted writing-envelope-v1 capability",
        ));
    }
    if response.results.len() != 1 {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::MalformedResponse,
            document_id,
            format!(
                "writing push expected exactly one result, received {}",
                response.results.len()
            ),
        ));
    }

    let result = response.results.into_iter().next().ok_or_else(|| {
        WritingPushPending::for_document(
            WritingPushPendingKind::MalformedResponse,
            document_id,
            "writing push result is absent",
        )
    })?;
    validate_result_identity(&result, document_id)?;
    if result.server_seq <= 0 || response.max_server_seq < result.server_seq {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::MalformedResponse,
            document_id,
            "writing push result has an invalid server sequence",
        ));
    }

    match result.status.as_str() {
        "applied" | "lww_won" => {
            if result.winner.is_some() {
                return Err(WritingPushPending::for_document(
                    WritingPushPendingKind::MalformedResponse,
                    document_id,
                    "successful writing push result unexpectedly included a winner",
                ));
            }
            let status = if result.status == "applied" {
                WritingPushAcceptedStatus::Applied
            } else {
                WritingPushAcceptedStatus::LwwWon
            };
            Ok(ValidatedPushResult::Accepted {
                status,
                server_seq: result.server_seq,
            })
        }
        "lww_lost" => {
            let winner = result.winner.ok_or_else(|| {
                WritingPushPending::for_document(
                    WritingPushPendingKind::MalformedResponse,
                    document_id,
                    "lww_lost writing result omitted its winner",
                )
            })?;
            Ok(ValidatedPushResult::Conflict {
                server_seq: result.server_seq,
                winner: validate_winner(winner, document_id, result.server_seq)?,
            })
        }
        _ => Err(WritingPushPending::for_document(
            WritingPushPendingKind::MalformedResponse,
            document_id,
            format!(
                "writing push returned unsupported status {:?}",
                result.status
            ),
        )),
    }
}

fn validate_result_identity(
    result: &PushResult,
    document_id: &str,
) -> Result<(), WritingPushPending> {
    if result.table != ENVELOPE_TABLE || result.row_id != document_id {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::MalformedResponse,
            document_id,
            "writing push result does not match the requested table and row",
        ));
    }
    Ok(())
}

fn validate_winner(
    winner: PullRow,
    document_id: &str,
    result_server_seq: i64,
) -> Result<PulledWritingRow, WritingPushPending> {
    if winner.table != ENVELOPE_TABLE
        || winner.row_id != document_id
        || winner.server_seq <= 0
        || winner.server_seq != result_server_seq
        || winner.changed_at < 0
        || winner.device_id.is_empty()
    {
        return Err(WritingPushPending::for_document(
            WritingPushPendingKind::MalformedResponse,
            document_id,
            "lww_lost winner does not bind the requested writing row and sequence",
        ));
    }
    if winner.deleted {
        if winner.payload.is_some() {
            return Err(WritingPushPending::for_document(
                WritingPushPendingKind::MalformedResponse,
                document_id,
                "deleted lww_lost winner unexpectedly included a payload",
            ));
        }
    } else {
        let payload = winner.payload.as_ref().ok_or_else(|| {
            WritingPushPending::for_document(
                WritingPushPendingKind::MalformedResponse,
                document_id,
                "lww_lost winner omitted its writing envelope",
            )
        })?;
        let envelope = WritingEnvelopeV1::from_json(&payload.to_string()).map_err(|error| {
            WritingPushPending::for_document(
                WritingPushPendingKind::MalformedResponse,
                document_id,
                format!("lww_lost winner envelope is invalid: {}", error.message),
            )
        })?;
        if envelope.id != document_id {
            return Err(WritingPushPending::for_document(
                WritingPushPendingKind::MalformedResponse,
                document_id,
                "lww_lost winner envelope id does not match the requested row",
            ));
        }
    }

    Ok(PulledWritingRow {
        document_id: winner.row_id,
        server_seq: winner.server_seq,
        deleted: winner.deleted,
        changed_at: winner.changed_at,
        device_id: winner.device_id,
        payload: winner.payload,
    })
}

fn require_writing_gate(conn: &Connection, server_epoch: &str) -> Result<(), WritingPushPending> {
    let supported = supports_writing(conn, server_epoch).map_err(pending_from_writing)?;
    if !supported {
        return Err(WritingPushPending::new(
            WritingPushPendingKind::CapabilityUnavailable,
            "writing-envelope-v1 was not discovered for the current server epoch",
        ));
    }
    if catchup_needed(conn, server_epoch).map_err(pending_from_writing)? {
        return Err(WritingPushPending::new(
            WritingPushPendingKind::CatchupIncomplete,
            "full writing catch-up is incomplete for the current server epoch",
        ));
    }
    Ok(())
}

fn read_session_binding(
    conn: &Connection,
    missing_kind: WritingPushPendingKind,
) -> Result<SessionBinding, WritingPushPending> {
    Ok(SessionBinding {
        account_id: required_meta(conn, "account_id", missing_kind)?,
        server_url: required_meta(conn, "server_url", missing_kind)?,
        device_id: required_meta(conn, "device_id", missing_kind)?,
        server_epoch: required_meta(conn, "server_epoch", missing_kind)?,
        session_incarnation: required_session_incarnation(conn, missing_kind)?,
    })
}

fn required_session_incarnation(
    conn: &Connection,
    missing_kind: WritingPushPendingKind,
) -> Result<Uuid, WritingPushPending> {
    super::session::read_session_incarnation(conn)
        .map_err(|message| WritingPushPending::new(WritingPushPendingKind::LocalState, message))?
        .ok_or_else(|| {
            WritingPushPending::new(
                missing_kind,
                format!(
                    "sync session metadata {:?} is missing",
                    super::session::SYNC_SESSION_INCARNATION_KEY
                ),
            )
        })
}

fn required_meta(
    conn: &Connection,
    key: &str,
    missing_kind: WritingPushPendingKind,
) -> Result<String, WritingPushPending> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            WritingPushPending::new(
                WritingPushPendingKind::LocalState,
                format!("failed to read sync session metadata {key:?}: {error}"),
            )
        })?;
    value.filter(|value| !value.is_empty()).ok_or_else(|| {
        WritingPushPending::new(
            missing_kind,
            format!("sync session metadata {key:?} is missing"),
        )
    })
}

fn pending_from_writing(error: crate::writing::repository::WritingError) -> WritingPushPending {
    WritingPushPending::new(
        WritingPushPendingKind::LocalState,
        format!("{}: {}", error.code, error.message),
    )
}

fn pending_from_blob(document_id: &str, error: WritingBlobPending) -> WritingPushPending {
    WritingPushPending::for_document(
        WritingPushPendingKind::Blob(error.kind),
        document_id,
        error.message,
    )
}

fn pending_from_sync(document_id: &str, error: SyncError) -> WritingPushPending {
    let kind = match &error {
        SyncError::Network(_) => WritingPushPendingKind::Network,
        SyncError::Api { status: 401, .. } => WritingPushPendingKind::Unauthorized,
        SyncError::Api { status: 403, .. } => WritingPushPendingKind::AccessDenied,
        SyncError::Decode(_) => WritingPushPendingKind::MalformedResponse,
        SyncError::InvalidUrl(_) | SyncError::Api { .. } => WritingPushPendingKind::RemoteRejected,
    };
    WritingPushPending::for_document(kind, document_id, error.to_string())
}

fn with_local_savepoint<T>(
    conn: &Connection,
    name: &str,
    operation: impl FnOnce() -> Result<T, WritingPushPending>,
) -> Result<T, WritingPushPending> {
    conn.execute_batch(&format!("SAVEPOINT {name};"))
        .map_err(|error| {
            WritingPushPending::new(
                WritingPushPendingKind::LocalState,
                format!("failed to open {name} savepoint: {error}"),
            )
        })?;

    match operation() {
        Ok(value) => match conn.execute_batch(&format!("RELEASE {name};")) {
            Ok(()) => Ok(value),
            Err(release_error) => {
                let rollback = rollback_local_savepoint(conn, name);
                Err(WritingPushPending::new(
                    WritingPushPendingKind::LocalState,
                    format!(
                        "failed to release {name} savepoint: {release_error}; rollback: {rollback:?}"
                    ),
                ))
            }
        },
        Err(pending) => {
            rollback_local_savepoint(conn, name).map_err(|rollback_error| {
                WritingPushPending::new(
                    WritingPushPendingKind::LocalState,
                    format!(
                        "{}; failed to roll back {name} savepoint: {rollback_error}",
                        pending.message
                    ),
                )
            })?;
            Err(pending)
        }
    }
}

fn rollback_local_savepoint(conn: &Connection, name: &str) -> Result<(), rusqlite::Error> {
    conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name};"))
}
