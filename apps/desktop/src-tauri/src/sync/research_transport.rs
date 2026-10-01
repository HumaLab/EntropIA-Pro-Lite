//! Glue between the Investigations research domain and the sync transport.
//!
//! The transport engine owns HTTP, batching and cursor bookkeeping; this
//! module owns the research-specific semantics both directions need:
//!
//! * [`build_push_changes`] turns outbox entries into wire drafts for the
//!   virtual `research_envelopes` table: terminal snapshots only, a proven
//!   report-file upload plan when the manifest is present, and the exact
//!   outbox entry listed as `pending` — never dropped, never cleared —
//!   whenever the snapshot or the file proof cannot be completed right now.
//! * [`settle_applied_push_draft`] acknowledges only the exact captured
//!   outbox generation, only after the server reported `applied`/`lww_won`,
//!   with invalid/stale server sequences rejected and newer generations
//!   preserved for their own push.
//! * [`apply_lww_lost_winner`] settles one validated `lww_lost` push result:
//!   the server-provided winner runs through the same receive/conflict
//!   machinery with no own-echo shortcut, the losing local envelope survives
//!   in `sync_conflicts`, and only the exact adjudicated generation is
//!   cleared after an accepted terminal outcome.
//! * [`apply_pulled_row`] applies one pulled `research_envelopes` row with the
//!   offline primitives only — no HTTP, so future engine code can call it
//!   directly. It skips stale sequences, suppresses own-device echoes while
//!   the local job exists (a missing local job is restored), defers rows that
//!   collide with durable local research work, and journals deterministic
//!   loser conflicts before any authoritative overwrite or remote delete.
//! * The applied terminal projection lives in the SEPARATE research state
//!   database (`research/estado.sqlite`): the job summary plus deterministic
//!   `request`/`report`/`archive` artifacts, sufficient for the existing
//!   `research_request get/list` UI. Replacement is explicit and ordered
//!   (children before parents); billable `llm_calls`, running checkpoints,
//!   human gates, global memories and transient events are never recreated.
//! * The local losing envelope is preserved in the generic `sync_conflicts`
//!   journal (opaque `loser_payload`) with a deterministic id and reason, so
//!   the existing conflict UI can surface it and a replay never duplicates it.
//!
//! A declared `report_file` manifest is never claimed locally present: the
//! file download/install slice owns transfer, and this module only keeps a
//! locally verified file or removes a stale one.
//!
//! Crash safety across the two databases is ordering-based: the losing
//! conflict is journaled durably BEFORE any deletion, the staged queue entry
//! (owned by `research_receive`) is removed only after the research state
//! write succeeded, and every projection write is idempotent so a replay after
//! a crash converges without duplicating conflicts.

// Inactive slice: the future engine calls these helpers after catch-up.
#![allow(dead_code)]

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::http::{PullRow, PushChange, PushResult};
use super::research_blobs::{report_upload_plan, ResearchBlobPending, ResearchReportUploadPlan};
use super::research_capture::{
    acknowledge_outbox_entry, has_pending_outbox_entry, now_ms, outbox_entries,
    OutboxAcknowledgment, OutboxEntry, ENVELOPE_TABLE,
};
use super::research_envelope::{
    is_safe_path_component, snapshot_job, snapshot_job_conn, verify_report_file,
    ResearchEnvelopeV1, ResearchError, ResearchFileManifestV1, ResearchResult,
    ResearchSourceEntryV1, INVALID_RESEARCH_ENVELOPE, RESEARCH_JOB_MODO, RESEARCH_JOB_NOT_FOUND,
    RESEARCH_JOB_NOT_TERMINAL, RESEARCH_REPORT_FILE_MISMATCH, RESEARCH_SNAPSHOT_UNREADABLE,
};

/// Reason code for the loser preserved when a remote tombstone deletes an
/// existing local terminal projection.
pub(crate) const RESEARCH_CONFLICT_REMOTE_DELETE: &str = "research_remote_delete";
/// Reason code for the loser preserved when a server winner replaces a job
/// with pending local research outbox work.
pub(crate) const RESEARCH_CONFLICT_DIVERGENT_UPSERT: &str = "research_divergent_upsert";
/// Error code for a wire result or draft that cannot settle one exact outbox
/// generation (wrong table/row, unsupported status, invalid sequence).
pub(crate) const RESEARCH_ENVELOPE_UNSUPPORTED: &str = "research_envelope_unsupported";

const TERMINAL_JOB_STATUSES: [&str; 2] = ["done", "failed"];
const SYNC_SAVEPOINT: &str = "research_sync_transport";

/// One pulled `research_envelopes` row handed to the research layer. Typed and
/// transport-free: the receive queue and future pull code both build this.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PulledResearchRow {
    pub(crate) job_id: String,
    pub(crate) server_seq: i64,
    pub(crate) deleted: bool,
    pub(crate) changed_at: i64,
    pub(crate) device_id: String,
    pub(crate) payload: Option<serde_json::Value>,
}

/// What happened to one pulled row. `Deferred`/`Unsupported` leave the row
/// unapplied so the receive queue can retain it with a bounded error; nothing
/// partial is ever written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PullApplyOutcome {
    Applied {
        created: bool,
        conflict_id: Option<String>,
    },
    NoOp,
    Stale,
    OwnChangeObserved,
    Deferred {
        reason: String,
    },
    Unsupported {
        reason: String,
    },
}

impl PulledResearchRow {
    /// Adapts one wire row — a pull page row or the `winner` of an `lww_lost`
    /// push result — to the research receive contract. The wire table is
    /// checked so a foreign table can never reach the research machinery.
    pub(crate) fn from_wire_row(row: &PullRow) -> ResearchResult<Self> {
        if row.table != ENVELOPE_TABLE {
            return Err(ResearchError::new(
                RESEARCH_ENVELOPE_UNSUPPORTED,
                format!(
                    "wire row {} carries table {:?}; expected {ENVELOPE_TABLE}",
                    row.row_id, row.table
                ),
            ));
        }
        Ok(Self {
            job_id: row.row_id.clone(),
            server_seq: row.server_seq,
            deleted: row.deleted,
            changed_at: row.changed_at,
            device_id: row.device_id.clone(),
            payload: row.payload.clone(),
        })
    }
}

// ─────────────────────────────── push side ───────────────────────────────

/// One pushable research change for the virtual `research_envelopes` table.
/// A `D` draft carries a null payload and no report upload; a `U` draft
/// carries the canonical envelope payload and, when the manifest is present,
/// the proven local report-file upload plan. No absolute path travels in any
/// payload.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResearchChangeDraft {
    pub(crate) job_id: String,
    pub(crate) op: char,
    pub(crate) changed_at: i64,
    pub(crate) base_seq: i64,
    pub(crate) payload: Option<Value>,
    pub(crate) report_upload: Option<ResearchReportUploadPlan>,
    pub(crate) acknowledgment: OutboxAcknowledgment,
}

impl ResearchChangeDraft {
    /// The wire change for `POST /v1/sync/push`. A `D` draft serializes with
    /// no payload; `base_seq` is the recorded `sync_row_versions` sequence
    /// and nothing else.
    pub(crate) fn to_push_change(&self) -> PushChange {
        PushChange {
            table: ENVELOPE_TABLE.to_string(),
            row_id: self.job_id.clone(),
            op: if self.op == 'D' { "delete" } else { "upsert" }.to_string(),
            changed_at: self.changed_at,
            base_seq: self.base_seq,
            payload: self.payload.clone(),
        }
    }
}

/// What the outbox could and could not prove right now. Unprovable entries
/// keep their exact outbox row and are listed as `pending`; building a push
/// never acknowledges, clears or rewrites anything.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct PushBuild {
    pub(crate) ready: Vec<ResearchChangeDraft>,
    pub(crate) pending: Vec<OutboxEntry>,
}

/// Atomic result of settling one `applied`/`lww_won` result for one exact
/// research draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PushDraftSettlement {
    Acknowledged,
    NewerGenerationPending,
    AlreadySettled,
    StaleServerSequence {
        recorded_server_seq: i64,
        response_server_seq: i64,
    },
}

/// The report-file proof boundary: validates the manifest and proves the
/// local bytes against it. Production uses [`report_upload_plan`]; the
/// `#[cfg(test)]` seam injects a prover so the pending routing can be tested
/// deterministically for the states only a filesystem race can produce
/// between the snapshot read and the proof read (the same house pattern as
/// `writing::sync_files`'s `before_publish` seam).
type ReportFileProver<'a> = &'a dyn Fn(
    &Path,
    &str,
    &ResearchFileManifestV1,
) -> Result<ResearchReportUploadPlan, ResearchBlobPending>;

/// Builds one wire draft per ready outbox entry. `U` drafts snapshot only
/// terminal research jobs and prove their declared `report.md` at build time;
/// `D` drafts emit a null payload without ever opening or fabricating the
/// research state database. A missing, changed or unreadable file — or a
/// vanished job — keeps the EXACT outbox entry in `pending` instead of
/// silently dropping it. The base sequence comes only from
/// `sync_row_versions`, and nothing here clears the outbox.
pub(crate) fn build_push_changes(
    sync: &Connection,
    state_db_path: &Path,
    artifacts_root: &Path,
) -> ResearchResult<PushBuild> {
    build_push_changes_inner(
        sync,
        state_db_path,
        artifacts_root,
        &|artifacts_root, job_id, manifest| report_upload_plan(artifacts_root, job_id, manifest),
    )
}

/// Test-only seam for deterministically exercising report-file proof
/// failures that a filesystem race can only produce between the snapshot read
/// and the proof read.
#[cfg(test)]
pub(crate) fn build_push_changes_with_report_prover(
    sync: &Connection,
    state_db_path: &Path,
    artifacts_root: &Path,
    prover: ReportFileProver<'_>,
) -> ResearchResult<PushBuild> {
    build_push_changes_inner(sync, state_db_path, artifacts_root, prover)
}

fn build_push_changes_inner(
    sync: &Connection,
    state_db_path: &Path,
    artifacts_root: &Path,
    prover: ReportFileProver<'_>,
) -> ResearchResult<PushBuild> {
    let mut build = PushBuild::default();
    for entry in outbox_entries(sync)? {
        match entry.op {
            'D' => build.ready.push(ResearchChangeDraft {
                job_id: entry.job_id.clone(),
                op: 'D',
                changed_at: entry.changed_at,
                base_seq: recorded_server_seq(sync, &entry.job_id)?,
                payload: None,
                report_upload: None,
                acknowledgment: entry.acknowledgment.clone(),
            }),
            'U' => match build_upsert_draft(sync, state_db_path, artifacts_root, &entry, prover) {
                Ok(draft) => build.ready.push(draft),
                Err(DraftBuildFailure::Pending) => build.pending.push(entry),
                Err(DraftBuildFailure::Fatal(error)) => return Err(error),
            },
            other => {
                return Err(ResearchError::new(
                    "research_outbox_corrupt",
                    format!("outbox entry {} has unsupported op {other:?}", entry.job_id),
                ))
            }
        }
    }
    Ok(build)
}

/// Why one `U` outbox entry produced no draft.
enum DraftBuildFailure {
    /// The entry cannot be proven right now: keep the exact outbox entry.
    Pending,
    /// A real sync-state error: fail the whole build.
    Fatal(ResearchError),
}

fn build_upsert_draft(
    sync: &Connection,
    state_db_path: &Path,
    artifacts_root: &Path,
    entry: &OutboxEntry,
    prover: ReportFileProver<'_>,
) -> Result<ResearchChangeDraft, DraftBuildFailure> {
    let envelope =
        snapshot_job(state_db_path, artifacts_root, &entry.job_id).map_err(map_snapshot_error)?;
    let canonical = envelope.to_canonical_json().map_err(map_snapshot_error)?;
    let payload: Value = serde_json::from_str(&canonical).map_err(|error| {
        DraftBuildFailure::Fatal(ResearchError::new(
            INVALID_RESEARCH_ENVELOPE,
            format!("cannot encode research envelope {}: {error}", entry.job_id),
        ))
    })?;

    // A declared manifest must be provable at build time: a missing, changed
    // or unreadable file keeps the exact outbox entry pending instead of
    // silently dropping the manifest or the entry.
    let report_upload = match &envelope.report_file {
        Some(manifest) => Some(
            prover(artifacts_root, &entry.job_id, manifest)
                .map_err(|_pending| DraftBuildFailure::Pending)?,
        ),
        None => None,
    };

    Ok(ResearchChangeDraft {
        job_id: entry.job_id.clone(),
        op: 'U',
        changed_at: entry.changed_at,
        base_seq: recorded_server_seq(sync, &entry.job_id).map_err(DraftBuildFailure::Fatal)?,
        payload: Some(payload),
        report_upload,
        acknowledgment: entry.acknowledgment.clone(),
    })
}

/// Snapshot failures that mean "this entry is not provable right now" keep
/// the exact outbox entry as durable pending work. SQL errors are real
/// sync-state errors and fail the build.
fn map_snapshot_error(error: ResearchError) -> DraftBuildFailure {
    match error.code.as_str() {
        RESEARCH_JOB_NOT_FOUND
        | RESEARCH_JOB_NOT_TERMINAL
        | RESEARCH_SNAPSHOT_UNREADABLE
        | RESEARCH_REPORT_FILE_MISMATCH
        | INVALID_RESEARCH_ENVELOPE => DraftBuildFailure::Pending,
        _ => DraftBuildFailure::Fatal(error),
    }
}

/// Conditionally acknowledges the exact outbox generation a push draft
/// captured. Callers must run this only after that draft's `applied`/
/// `lww_won` settlement (or its `lww_lost` settlement); pulled own-device
/// rows are not proof.
pub(crate) fn acknowledge_push_draft(
    sync: &Connection,
    draft: &ResearchChangeDraft,
) -> ResearchResult<bool> {
    if draft.job_id != draft.acknowledgment.job_id {
        return Err(ResearchError::new(
            RESEARCH_ENVELOPE_UNSUPPORTED,
            "research draft acknowledgment does not match its job".to_string(),
        ));
    }
    acknowledge_outbox_entry(sync, &draft.acknowledgment)
}

/// Records the monotonic row version and acknowledges only the captured
/// generation, and only for a well-formed `applied`/`lww_won` result of the
/// same row. `lww_lost` results settle through [`apply_lww_lost_winner`].
/// Invalid or stale server sequences are rejected without touching the
/// outbox, a newer generation keeps its entry, and replaying a settled draft
/// is a harmless [`PushDraftSettlement::AlreadySettled`].
pub(crate) fn settle_applied_push_draft(
    sync: &Connection,
    draft: &ResearchChangeDraft,
    result: &PushResult,
) -> ResearchResult<PushDraftSettlement> {
    if result.table != ENVELOPE_TABLE || result.row_id != draft.job_id {
        return Err(ResearchError::new(
            RESEARCH_ENVELOPE_UNSUPPORTED,
            format!(
                "push result {}:{} does not match research draft {}",
                result.table, result.row_id, draft.job_id
            ),
        ));
    }
    if !matches!(result.status.as_str(), "applied" | "lww_won") {
        return Err(ResearchError::new(
            RESEARCH_ENVELOPE_UNSUPPORTED,
            format!(
                "research push result {} settles only after applied/lww_won, got {:?}",
                draft.job_id, result.status
            ),
        ));
    }
    if result.server_seq <= 0 {
        return Err(ResearchError::new(
            RESEARCH_ENVELOPE_UNSUPPORTED,
            format!(
                "research push returned invalid server_seq {}",
                result.server_seq
            ),
        ));
    }

    with_sync_savepoint(sync, || {
        let recorded = recorded_server_seq(sync, &draft.job_id)?;
        if recorded > result.server_seq {
            return Ok(PushDraftSettlement::StaleServerSequence {
                recorded_server_seq: recorded,
                response_server_seq: result.server_seq,
            });
        }

        let current = outbox_entries(sync)?
            .into_iter()
            .find(|entry| entry.job_id == draft.job_id);
        let Some(current) = current else {
            return Ok(PushDraftSettlement::AlreadySettled);
        };

        record_server_seq(sync, &draft.job_id, result.server_seq)?;
        if current.acknowledgment != draft.acknowledgment {
            // A newer generation replaced the adjudicated one mid-flight: it
            // keeps its outbox entry and is settled by its own push.
            return Ok(PushDraftSettlement::NewerGenerationPending);
        }

        if !acknowledge_push_draft(sync, draft)? {
            return Err(ResearchError::new(
                "research_ack_raced",
                format!(
                    "research outbox generation changed while settling {}",
                    draft.job_id
                ),
            ));
        }
        Ok(PushDraftSettlement::Acknowledged)
    })
}

/// Wraps the sync-side settlement bookkeeping in one savepoint so a failed
/// release restores both the row version and the outbox entry. The receive
/// path deliberately does NOT use it: its two-database ordering is crash-safe
/// by commit order (loser journaled before any deletion), not by one
/// transaction.
fn with_sync_savepoint<T>(
    sync: &Connection,
    operation: impl FnOnce() -> ResearchResult<T>,
) -> ResearchResult<T> {
    sync.execute_batch(&format!("SAVEPOINT {SYNC_SAVEPOINT};"))
        .map_err(|error| ResearchError::sql("Failed to open research sync savepoint", error))?;

    match operation() {
        Ok(value) => match sync.execute_batch(&format!("RELEASE {SYNC_SAVEPOINT};")) {
            Ok(()) => Ok(value),
            Err(release_error) => {
                rollback_sync_savepoint(sync).map_err(|rollback_error| {
                    ResearchError::new(
                        "sql_error",
                        format!(
                            "Failed to release research sync savepoint: {release_error}; \
                             rollback also failed: {rollback_error}"
                        ),
                    )
                })?;
                Err(ResearchError::sql(
                    "Failed to release research sync savepoint",
                    release_error,
                ))
            }
        },
        Err(error) => {
            rollback_sync_savepoint(sync).map_err(|rollback_error| {
                ResearchError::new(
                    "sql_error",
                    format!(
                        "{}; failed to roll back research sync savepoint: {rollback_error}",
                        error.message
                    ),
                )
            })?;
            Err(error)
        }
    }
}

fn rollback_sync_savepoint(sync: &Connection) -> Result<(), rusqlite::Error> {
    sync.execute_batch(&format!(
        "ROLLBACK TO {SYNC_SAVEPOINT}; RELEASE {SYNC_SAVEPOINT};"
    ))
}

/// Applies one pulled research row. The caller must hold NO open transaction
/// on either connection: each durable step commits its own transaction so the
/// ordering in the module docs holds across crashes.
///
/// `sync` is the app database carrying the sync schema (row versions, the
/// conflict journal and the pending outbox); `state` is the separate research
/// state database that receives the readable terminal projection.
pub(crate) fn apply_pulled_row(
    sync: &Connection,
    state: &Connection,
    artifacts_root: &Path,
    own_device_id: &str,
    row: &PulledResearchRow,
) -> ResearchResult<PullApplyOutcome> {
    let recorded = recorded_server_seq(sync, &row.job_id)?;
    if recorded >= row.server_seq {
        return Ok(PullApplyOutcome::Stale);
    }

    if row.device_id == own_device_id && research_job_present(state, &row.job_id)? {
        // Device identity and wire timestamps cannot identify which local
        // generation was pushed. Only the exact outbox-generation settlement
        // may remove pending work. A missing local job is NOT an echo: it is
        // restored (or idempotently re-deleted) through the paths below.
        record_server_seq(sync, &row.job_id, row.server_seq)?;
        return Ok(PullApplyOutcome::OwnChangeObserved);
    }

    dispatch_remote_row(sync, state, artifacts_root, row)
}

/// The remote-row dispatch shared by pulled rows and `lww_lost` settlement, so
/// both run exactly the same receive and conflict machinery. The own-echo
/// shortcut deliberately lives only in [`apply_pulled_row`] above.
fn dispatch_remote_row(
    sync: &Connection,
    state: &Connection,
    artifacts_root: &Path,
    row: &PulledResearchRow,
) -> ResearchResult<PullApplyOutcome> {
    if row.deleted {
        apply_tombstone(sync, state, artifacts_root, row)
    } else {
        apply_upsert(sync, state, artifacts_root, row)
    }
}

/// What settling one validated `lww_lost` push result did locally.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LwwLostSettlement {
    /// The server-named winner went through the receive/conflict machinery.
    /// Only `Applied`/`NoOp` are accepted terminal outcomes: only then was
    /// the exact adjudicated generation cleared.
    Routed(PullApplyOutcome),
    /// A newer local generation replaced the adjudicated one mid-flight: this
    /// result may not clear it, so it keeps its outbox entry and waits for its
    /// own push to be adjudicated.
    NewerGenerationPending,
}

/// Settles one validated `lww_lost` push result: the server adjudicated the
/// pushed generation as a loser against `winner`. This is the ONLY automatic
/// path allowed to settle a remote row while a local outbox entry exists —
/// the push response proves the pending generation lost, so `winner` can
/// never be that generation's echo.
///
/// `adjudicated_generation` is the exact outbox generation the settled draft
/// captured. A save during the send round trip replaces it, and the newer
/// generation is NEVER cleared here: it keeps its outbox entry and is
/// adjudicated by its own push. Only an accepted terminal outcome of the
/// winner application clears the adjudicated generation (compare-and-delete,
/// never a timestamp), and the losing local envelope survives in
/// `sync_conflicts` before any overwrite or delete.
pub(crate) fn apply_lww_lost_winner(
    sync: &Connection,
    state: &Connection,
    artifacts_root: &Path,
    winner: &PulledResearchRow,
    adjudicated_generation: Option<&OutboxAcknowledgment>,
) -> ResearchResult<LwwLostSettlement> {
    if let Some(acknowledgment) = adjudicated_generation {
        if acknowledgment.job_id != winner.job_id {
            return Err(ResearchError::new(
                RESEARCH_ENVELOPE_UNSUPPORTED,
                format!(
                    "adjudicated research generation {} does not belong to winner row {}",
                    acknowledgment.job_id, winner.job_id
                ),
            ));
        }
    }

    let recorded = recorded_server_seq(sync, &winner.job_id)?;
    if recorded >= winner.server_seq {
        return Ok(LwwLostSettlement::Routed(PullApplyOutcome::Stale));
    }

    // Only the exact adjudicated generation may be settled. The comparison
    // pins this decision to the receive below: a generation that replaced the
    // adjudicated one is never dropped, and a wire timestamp never clears
    // work.
    let current = outbox_entries(sync)?
        .into_iter()
        .find(|entry| entry.job_id == winner.job_id)
        .map(|entry| entry.acknowledgment);
    if current.is_some() && current.as_ref() != adjudicated_generation {
        return Ok(LwwLostSettlement::NewerGenerationPending);
    }

    // Unlike [`apply_pulled_row`] there is no own-echo shortcut: the server
    // explicitly named this row the winner that beat the adjudicated
    // generation, so device identity proves nothing here and the receive and
    // conflict machinery owns every overwrite decision.
    let outcome = dispatch_remote_row(sync, state, artifacts_root, winner)?;

    match outcome {
        PullApplyOutcome::Applied { .. } | PullApplyOutcome::NoOp => {
            // Accepted terminal outcome: clear ONLY the exact adjudicated
            // generation. The compare-and-delete keeps a replaced generation
            // pending by construction, and a replay clears nothing.
            if let Some(acknowledgment) = adjudicated_generation {
                acknowledge_outbox_entry(sync, acknowledgment)?;
            }
            Ok(LwwLostSettlement::Routed(outcome))
        }
        other => Ok(LwwLostSettlement::Routed(other)),
    }
}

fn apply_upsert(
    sync: &Connection,
    state: &Connection,
    artifacts_root: &Path,
    row: &PulledResearchRow,
) -> ResearchResult<PullApplyOutcome> {
    let Some(payload) = &row.payload else {
        return Ok(PullApplyOutcome::Unsupported {
            reason: "research upsert row without payload".to_string(),
        });
    };
    let envelope = match ResearchEnvelopeV1::from_json(&payload.to_string()) {
        Ok(envelope) => envelope,
        Err(error) => {
            return Ok(PullApplyOutcome::Unsupported {
                reason: error.message,
            })
        }
    };
    if envelope.id != row.job_id {
        return Ok(PullApplyOutcome::Unsupported {
            reason: format!(
                "research envelope id {} does not match row {}",
                envelope.id, row.job_id
            ),
        });
    }

    match inspect_local_projection(state, artifacts_root, &row.job_id)? {
        LocalProjection::Foreign => Ok(PullApplyOutcome::Unsupported {
            reason: format!("row {} collides with a non-research local job", row.job_id),
        }),
        LocalProjection::Active => Ok(PullApplyOutcome::Deferred {
            reason: "local research job is not terminal; active execution is never replaced"
                .to_string(),
        }),
        LocalProjection::Missing => {
            install_projection(state, artifacts_root, &envelope)?;
            record_server_seq(sync, &row.job_id, row.server_seq)?;
            Ok(PullApplyOutcome::Applied {
                created: true,
                conflict_id: None,
            })
        }
        LocalProjection::Valid(local, local_fingerprint) => {
            let incoming_fingerprint = envelope.fingerprint_sha256()?;
            if local_fingerprint == incoming_fingerprint {
                // Same aggregate: nothing changes but the row version.
                record_server_seq(sync, &row.job_id, row.server_seq)?;
                return Ok(PullApplyOutcome::NoOp);
            }
            if has_pending_outbox_entry(sync, &row.job_id)? {
                // Divergent local outbox state: the guarded local snapshot must
                // be valid to lose anything at all. The loser survives in the
                // conflict journal before the winner lands.
                let conflict_id = journal_loser(
                    sync,
                    &row.job_id,
                    RESEARCH_CONFLICT_DIVERGENT_UPSERT,
                    &local,
                    &format!("server winner (server_seq {})", row.server_seq),
                )?;
                if let Some(reason) = install_projection_guarded(
                    state,
                    artifacts_root,
                    &envelope,
                    &local_fingerprint,
                )? {
                    return Ok(PullApplyOutcome::Deferred { reason });
                }
                record_server_seq(sync, &row.job_id, row.server_seq)?;
                return Ok(PullApplyOutcome::Applied {
                    created: false,
                    conflict_id: Some(conflict_id),
                });
            }
            install_projection(state, artifacts_root, &envelope)?;
            record_server_seq(sync, &row.job_id, row.server_seq)?;
            Ok(PullApplyOutcome::Applied {
                created: false,
                conflict_id: None,
            })
        }
        LocalProjection::Invalid(reason) => {
            if has_pending_outbox_entry(sync, &row.job_id)? {
                Ok(PullApplyOutcome::Deferred {
                    reason: format!(
                        "pending local research work and the local snapshot is not valid: {reason}"
                    ),
                })
            } else {
                // Nothing local is pending, so the server winner is
                // authoritative even over an incoherent local projection.
                install_projection(state, artifacts_root, &envelope)?;
                record_server_seq(sync, &row.job_id, row.server_seq)?;
                Ok(PullApplyOutcome::Applied {
                    created: false,
                    conflict_id: None,
                })
            }
        }
    }
}

fn apply_tombstone(
    sync: &Connection,
    state: &Connection,
    artifacts_root: &Path,
    row: &PulledResearchRow,
) -> ResearchResult<PullApplyOutcome> {
    if row.payload.is_some() {
        return Ok(PullApplyOutcome::Unsupported {
            reason: "research tombstone row must not carry a payload".to_string(),
        });
    }
    if has_pending_outbox_entry(sync, &row.job_id)? {
        // Pending local research work owns this job until its exact
        // generation is settled; the queued row is retained for replay.
        return Ok(PullApplyOutcome::Deferred {
            reason: "pending local research outbox work defers the remote delete".to_string(),
        });
    }
    match inspect_local_projection(state, artifacts_root, &row.job_id)? {
        LocalProjection::Foreign => Ok(PullApplyOutcome::Unsupported {
            reason: format!("row {} collides with a non-research local job", row.job_id),
        }),
        LocalProjection::Active => Ok(PullApplyOutcome::Deferred {
            reason: "local research job is not terminal; active execution is never deleted"
                .to_string(),
        }),
        LocalProjection::Invalid(reason) => Ok(PullApplyOutcome::Deferred {
            reason: format!("the local losing envelope is not valid: {reason}"),
        }),
        LocalProjection::Missing => {
            // Idempotent delete: nothing is left to remove but a possible
            // stale report file, and the row version still advances.
            remove_report_file(artifacts_root, &row.job_id)?;
            record_server_seq(sync, &row.job_id, row.server_seq)?;
            Ok(PullApplyOutcome::NoOp)
        }
        LocalProjection::Valid(local, _) => {
            // The remote delete is authoritative: the losing envelope is
            // journaled durably BEFORE the deletion removes it.
            let conflict_id = journal_loser(
                sync,
                &row.job_id,
                RESEARCH_CONFLICT_REMOTE_DELETE,
                &local,
                &format!("remote delete (server_seq {})", row.server_seq),
            )?;
            delete_projection(state, artifacts_root, &row.job_id)?;
            record_server_seq(sync, &row.job_id, row.server_seq)?;
            Ok(PullApplyOutcome::Applied {
                created: false,
                conflict_id: Some(conflict_id),
            })
        }
    }
}

/// What the local research state holds for one row id.
// Short-lived, one per received row: boxing the large variant buys nothing.
#[allow(clippy::large_enum_variant)]
enum LocalProjection {
    /// No local job with this id.
    Missing,
    /// The id is taken by another job kind: never overwritten or deleted.
    Foreign,
    /// A research job that is not terminal: active execution is out of the
    /// terminal upsert snapshot contract and must never be replaced.
    Active,
    /// A coherent terminal projection with its snapshot fingerprint.
    Valid(ResearchEnvelopeV1, String),
    /// A terminal research job whose snapshot cannot be built coherently.
    Invalid(String),
}

fn inspect_local_projection(
    state: &Connection,
    artifacts_root: &Path,
    job_id: &str,
) -> ResearchResult<LocalProjection> {
    let modo: Option<String> = state
        .query_row(
            "SELECT modo FROM jobs WHERE id = ?1",
            params![job_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| ResearchError::sql("Failed to inspect local research job", error))?;
    let Some(modo) = modo else {
        return Ok(LocalProjection::Missing);
    };
    if modo != RESEARCH_JOB_MODO {
        return Ok(LocalProjection::Foreign);
    }
    let status: Option<String> = state
        .query_row(
            "SELECT status FROM jobs WHERE id = ?1",
            params![job_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| ResearchError::sql("Failed to inspect local research job", error))?;
    if !status
        .as_deref()
        .is_some_and(|status| TERMINAL_JOB_STATUSES.contains(&status))
    {
        return Ok(LocalProjection::Active);
    }
    match snapshot_job_conn(state, artifacts_root, job_id) {
        Ok(envelope) => {
            let fingerprint = envelope.fingerprint_sha256()?;
            Ok(LocalProjection::Valid(envelope, fingerprint))
        }
        Err(error) => Ok(LocalProjection::Invalid(error.message)),
    }
}

fn research_job_present(state: &Connection, job_id: &str) -> ResearchResult<bool> {
    state
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE id = ?1 AND modo = ?2)",
            params![job_id, RESEARCH_JOB_MODO],
            |row| row.get::<_, i64>(0),
        )
        .map(|present| present != 0)
        .map_err(|error| ResearchError::sql("Failed to inspect local research job", error))
}

/// Creates or replaces the readable terminal projection in one explicit
/// transaction: safe deletion ordering over the existing job-owned rows, then
/// the deterministic job summary and `request`/`report`/`archive` artifacts.
/// Nothing billable or transient is recreated. The report file is reconciled
/// AFTER the row transaction: only a locally verified file is kept, so the
/// projection never silently claims an uninstalled manifest is present.
fn install_projection(
    state: &Connection,
    artifacts_root: &Path,
    envelope: &ResearchEnvelopeV1,
) -> ResearchResult<()> {
    let tx = state
        .unchecked_transaction()
        .map_err(|error| ResearchError::sql("Failed to start research projection", error))?;
    write_projection(&tx, envelope)?;
    tx.commit()
        .map_err(|error| ResearchError::sql("Failed to commit research projection", error))?;
    reconcile_report_file(artifacts_root, &envelope.id, envelope.report_file.as_ref())
}

/// Same replacement behind a fingerprint guard: the guarded local snapshot is
/// re-verified inside the transaction, so a concurrent local change is never
/// overwritten. Returns `Some(reason)` — and writes nothing — when the guard
/// trips.
fn install_projection_guarded(
    state: &Connection,
    artifacts_root: &Path,
    envelope: &ResearchEnvelopeV1,
    expected_local_fingerprint: &str,
) -> ResearchResult<Option<String>> {
    let tx = state
        .unchecked_transaction()
        .map_err(|error| ResearchError::sql("Failed to start research projection", error))?;
    match snapshot_job_conn(&tx, artifacts_root, &envelope.id) {
        Err(error) => return Ok(Some(error.message)),
        Ok(current) => {
            if current.fingerprint_sha256()? != expected_local_fingerprint {
                return Ok(Some(
                    "local research state changed under the guarded snapshot".to_string(),
                ));
            }
        }
    }
    write_projection(&tx, envelope)?;
    tx.commit()
        .map_err(|error| ResearchError::sql("Failed to commit research projection", error))?;
    reconcile_report_file(artifacts_root, &envelope.id, envelope.report_file.as_ref())?;
    Ok(None)
}

/// Deletes the job-owned terminal projection and its report file. The rows
/// commit first so a crashed replay re-runs the idempotent file removal
/// instead of orphaning it.
fn delete_projection(
    state: &Connection,
    artifacts_root: &Path,
    job_id: &str,
) -> ResearchResult<()> {
    let tx = state
        .unchecked_transaction()
        .map_err(|error| ResearchError::sql("Failed to start research projection delete", error))?;
    delete_job_owned_rows(&tx, job_id)?;
    tx.commit().map_err(|error| {
        ResearchError::sql("Failed to commit research projection delete", error)
    })?;
    remove_report_file(artifacts_root, job_id)
}

/// Safe deletion ordering for every row one job owns: claim children first,
/// then the job-level rows, then the stages that the job row references.
/// Global memories and corpus sources/evidence are not job-owned and stay.
fn delete_job_owned_rows(conn: &Connection, job_id: &str) -> ResearchResult<()> {
    for statement in [
        "DELETE FROM verification_runs WHERE claim_id IN (SELECT id FROM claims WHERE job_id = ?1)",
        "DELETE FROM claim_evidence WHERE claim_id IN (SELECT id FROM claims WHERE job_id = ?1)",
        "DELETE FROM claims WHERE job_id = ?1",
        "DELETE FROM human_decisions WHERE job_id = ?1",
        "DELETE FROM artifacts WHERE job_id = ?1",
        "DELETE FROM job_events WHERE job_id = ?1",
        "DELETE FROM llm_calls WHERE job_id = ?1",
        "DELETE FROM queries WHERE job_id = ?1",
        "DELETE FROM stage_dependencies WHERE stage_id IN (SELECT id FROM stages WHERE job_id = ?1)",
        "DELETE FROM stages WHERE job_id = ?1",
        "DELETE FROM jobs WHERE id = ?1",
    ] {
        conn.execute(statement, params![job_id])
            .map_err(|error| ResearchError::sql("Failed to clear research job rows", error))?;
    }
    Ok(())
}

/// Writes the deterministic terminal projection rows. Caller owns the
/// transaction. Artifact ids derive from `(job_id, kind)` so a replay writes
/// byte-identical rows.
fn write_projection(conn: &Connection, envelope: &ResearchEnvelopeV1) -> ResearchResult<()> {
    let job = &envelope.job;
    delete_job_owned_rows(conn, &job.id)?;
    conn.execute(
        "INSERT INTO jobs (id, modo, pregunta, plan_json, status, close_reason,
                           costo_acumulado, max_cost, max_llm_calls, config_snapshot,
                           corpus_snapshot_id, project, corpus, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, NULL, '{}', ?7, ?8, 'desktop', ?9, ?10)",
        params![
            job.id,
            RESEARCH_JOB_MODO,
            job.question,
            TERMINAL_PLAN_JSON,
            job.status,
            job.close_reason,
            job.corpus_snapshot_id,
            job.project,
            job.created_at,
            job.updated_at,
        ],
    )
    .map_err(|error| ResearchError::sql("Failed to write research job projection", error))?;

    let request = serde_json::json!({
        "title": job.title,
        "question": job.question,
        "project": job.project,
    });
    insert_artifact(conn, &job.id, "request", 1, &request, job.updated_at)?;

    if let Some(report) = &envelope.report {
        insert_artifact(
            conn,
            &job.id,
            "report",
            report.version,
            &report.content_json,
            job.updated_at,
        )?;
    }

    let mut evidence = Vec::with_capacity(envelope.sources.len());
    for entry in &envelope.sources {
        evidence.push(manifest_evidence_entry(entry));
    }
    let archive = serde_json::json!({ "evidence": evidence });
    insert_artifact(conn, &job.id, "archive", 1, &archive, job.updated_at)?;
    Ok(())
}

/// Rebuilds one archive evidence entry from the de-duplicated source manifest
/// so a later snapshot re-derives the exact same entry: a `chunk_id` travels
/// as the entry's `chunk_id`, and a source that never had a corpus chunk
/// travels with a non-corpus provenance marker instead.
fn manifest_evidence_entry(entry: &ResearchSourceEntryV1) -> serde_json::Value {
    let mut value = serde_json::json!({ "id": entry.evidence_id });
    let object = value.as_object_mut().expect("evidence object");
    if let Some(item_id) = &entry.item_id {
        object.insert("item_id".to_string(), serde_json::json!(item_id));
    }
    if let Some(text_hash) = &entry.text_hash {
        object.insert("text_hash".to_string(), serde_json::json!(text_hash));
    }
    if let Some(title) = &entry.title {
        object.insert("title".to_string(), serde_json::json!(title));
    }
    match &entry.chunk_id {
        Some(chunk_id) if *chunk_id == entry.source_id => {
            object.insert("chunk_id".to_string(), serde_json::json!(chunk_id));
        }
        Some(chunk_id) => {
            // Crafted shape the engine never derives: keep both identifiers
            // verbatim; the stable `source_id` wins the re-derivation.
            object.insert("chunk_id".to_string(), serde_json::json!(entry.source_id));
            object.insert("source_id".to_string(), serde_json::json!(entry.source_id));
            object.insert("manifest_chunk_id".to_string(), serde_json::json!(chunk_id));
        }
        None => {
            // No corpus chunk: any non-corpus provenance keeps `chunk_id`
            // empty on re-derivation.
            object.insert("provenance".to_string(), serde_json::json!("external"));
            if entry.source_id != entry.evidence_id {
                object.insert("source_id".to_string(), serde_json::json!(entry.source_id));
            }
        }
    }
    value
}

fn insert_artifact(
    conn: &Connection,
    job_id: &str,
    kind: &str,
    version: i64,
    content: &serde_json::Value,
    created_at: i64,
) -> ResearchResult<()> {
    let content = serde_json::to_string(content).map_err(|error| {
        ResearchError::new(
            "research_projection_corrupt",
            format!("cannot encode research artifact: {error}"),
        )
    })?;
    conn.execute(
        "INSERT INTO artifacts (id, job_id, tipo, path, padre, version, created_at, content_json)
         VALUES (?1, ?2, ?3, '', NULL, ?4, ?5, ?6)",
        params![
            artifact_id(job_id, kind),
            job_id,
            kind,
            version,
            created_at,
            content,
        ],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to write research artifact", error))
}

/// Deterministic artifact id per `(job_id, kind)`, so replaying the same
/// envelope writes the same rows and the replace is idempotent.
fn artifact_id(job_id: &str, kind: &str) -> String {
    format!("sync-art-{job_id}-{kind}")
}

/// A readable terminal workflow state for the `research_request get/list` UI:
/// the plan is closed at the report phase and never executed again.
const TERMINAL_PLAN_JSON: &str = r#"{"step":7,"collections":[],"model":null,"modalidad":null}"#;

/// Keeps a locally verified `report.md` and removes any other one. A declared
/// manifest with no verified local bytes must never look present: that claim
/// belongs to the report-file download slice.
fn reconcile_report_file(
    artifacts_root: &Path,
    job_id: &str,
    manifest: Option<&super::research_envelope::ResearchFileManifestV1>,
) -> ResearchResult<()> {
    if !is_safe_path_component(job_id) {
        return Err(ResearchError::new(
            "invalid_research_envelope",
            format!("job id {job_id:?} is not a safe path component"),
        ));
    }
    let verified = manifest
        .map(|manifest| verify_report_file(artifacts_root, job_id, manifest).is_ok())
        .unwrap_or(false);
    if verified {
        return Ok(());
    }
    remove_report_file(artifacts_root, job_id)
}

/// Idempotent removal of one job's report file. Other files under the job's
/// artifacts directory belong to the download slice and are untouched.
fn remove_report_file(artifacts_root: &Path, job_id: &str) -> ResearchResult<()> {
    if !is_safe_path_component(job_id) {
        return Ok(());
    }
    let path = artifacts_root
        .join(job_id)
        .join(super::research_envelope::REPORT_FILE_REL_PATH);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ResearchError::new(
            "research_snapshot_unreadable",
            format!("cannot remove {}: {error}", path.display()),
        )),
    }
}

/// Journals one losing local envelope in the generic `sync_conflicts` table
/// BEFORE its rows can be deleted. The conflict id derives from `(reason,
/// job_id, loser fingerprint)`: deterministic, replay-idempotent and never
/// colliding with a different loser. The `loser_payload` is the opaque
/// canonical envelope the generic conflict UI can surface.
fn journal_loser(
    sync: &Connection,
    job_id: &str,
    reason: &str,
    loser: &ResearchEnvelopeV1,
    winner_summary: &str,
) -> ResearchResult<String> {
    if !is_safe_path_component(job_id) {
        return Err(ResearchError::new(
            "invalid_research_envelope",
            format!("job id {job_id:?} is not a safe path component"),
        ));
    }
    let payload = loser.to_canonical_json()?;
    let fingerprint = loser.fingerprint_sha256()?;
    let conflict_id = format!("{reason}-{job_id}-{fingerprint}");
    sync.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, loser_payload,
                                    winner_summary, created_at, acknowledged)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
         ON CONFLICT(id) DO NOTHING",
        params![
            conflict_id,
            ENVELOPE_TABLE,
            job_id,
            reason,
            payload,
            winner_summary,
            now_ms(),
        ],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to journal research conflict", error))?;
    Ok(conflict_id)
}

/// The recorded row version of one research row (`0` when never recorded).
fn recorded_server_seq(sync: &Connection, job_id: &str) -> ResearchResult<i64> {
    sync.query_row(
        "SELECT server_seq FROM sync_row_versions
          WHERE table_name = ?1 AND row_id = ?2",
        params![ENVELOPE_TABLE, job_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| ResearchError::sql("Failed to read research row version", error))
    .map(|recorded| recorded.unwrap_or(0))
}

/// Monotonic row-version update. A replayed or stale row never moves it back.
fn record_server_seq(sync: &Connection, job_id: &str, server_seq: i64) -> ResearchResult<()> {
    sync.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq) VALUES (?1, ?2, ?3)
         ON CONFLICT(table_name, row_id) DO UPDATE SET server_seq = excluded.server_seq
          WHERE excluded.server_seq > sync_row_versions.server_seq",
        params![ENVELOPE_TABLE, job_id, server_seq],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to record research row version", error))
}
