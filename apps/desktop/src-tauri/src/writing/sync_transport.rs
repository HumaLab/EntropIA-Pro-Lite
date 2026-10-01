//! Glue between the writing domain and the sync transport.
//!
//! The transport engine owns HTTP, batching and cursor bookkeeping; this
//! module owns the writing-specific semantics both directions need:
//!
//! * [`build_push_changes`] turns outbox entries into wire drafts for the
//!   virtual `writing_envelopes` table. A draft is only produced when every
//!   referenced attachment can be proven from local bytes. Nothing is ever
//!   silently dropped — unprovable documents stay in the outbox as
//!   `pending_files` until the file layer can prove them.
//! * [`apply_pulled_row`] applies one pulled envelope row with the offline
//!   primitives: guarded receive, deterministic conflict copies for divergent
//!   local edits, skip-if-stale, and echo suppression for our own pushes.
//! * [`apply_lww_lost_winner`] settles one validated `lww_lost` push result.
//!   The push response — never a wire timestamp — proves the pending
//!   generation lost against the winner, so the loser is preserved as a
//!   visible conflict copy before the winner lands, and only the exact
//!   adjudicated outbox generation is ever cleared.
//! * [`pending_local_writes`] is the W-GUARD2 backend dirty barrier: it tells a
//!   future writing-receive caller when one document still holds durable local
//!   work (recovery journal deltas or an outbox entry) so automatic application
//!   can defer. It is a durable-state rule only — see its docs for why the
//!   W-GUARD1 manual flush is still required before activation.
//!
//! Activation itself (sending `X-Sync-Capabilities`) is the engine's job and
//! stays gated on [`catchup_needed`] being resolved first: a client whose
//! cursor already passed hidden writing rows must run one full writing
//! catch-up before incremental pulls are meaningful.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::repository::{now_ms, WritingError, WritingResult};
use super::sync_capture::{
    acknowledge_outbox_entry, discard_outbox_for_authoritative_receive, enqueue_document,
    outbox_entries, OutboxAcknowledgment, CATCHUP_EPOCH_KEY, ENVELOPE_TABLE, MANIFEST_PREFIX,
    PENDING_ASSETS_PREFIX,
};
use super::sync_conflict::{
    preserve_conflict_copy, preserve_loser_then_receive_winner, ConflictSourceMetadata,
    ConflictThenReceiveOutcome,
};
use super::sync_envelope::{snapshot_document, AttachmentManifestV1, WritingEnvelopeV1};
use super::sync_files::{scan_document_attachments, verify_attachment_manifest};
use super::sync_receive::{
    pending_journal_count, receive_envelope, require_receive_schema, AttachmentInstallReceipt,
    ReceiveAuthorization, ReceiveDeferred, ReceiveOutcome,
};

pub(crate) const ENVELOPE_UNSUPPORTED: &str = "writing_envelope_unsupported";
const TRANSPORT_SAVEPOINT: &str = "writing_sync_transport";

/// One pushable document change for the wire.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WritingChangeDraft {
    pub(crate) document_id: String,
    pub(crate) op: char,
    pub(crate) changed_at: i64,
    pub(crate) base_seq: i64,
    pub(crate) payload: Value,
    pub(crate) acknowledgment: OutboxAcknowledgment,
}

/// What the outbox could and could not prove right now.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct PushBuild {
    pub(crate) ready: Vec<WritingChangeDraft>,
    pub(crate) pending_files: Vec<String>,
}

/// One pulled `writing_envelopes` row handed to the writing layer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PulledWritingRow {
    pub(crate) document_id: String,
    pub(crate) server_seq: i64,
    pub(crate) deleted: bool,
    pub(crate) changed_at: i64,
    pub(crate) device_id: String,
    pub(crate) payload: Option<Value>,
}

/// What happened to one pulled row. `Deferred`/`Unsupported` leave the row
/// unapplied so the transport can park or journal it; nothing partial is ever
/// written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PullApplyOutcome {
    Applied {
        created: bool,
        conflict_document_id: Option<String>,
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

/// What settling one validated `lww_lost` push result did locally.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LwwLostSettlement {
    /// The validated winner went through the pull-apply machinery. With local
    /// pending edits that is the preserve-loser-then-receive path.
    Routed(PullApplyOutcome),
    /// A newer local generation replaced the adjudicated one mid-flight: this
    /// result may not clear it, so it keeps its outbox entry and waits for its
    /// own push to be adjudicated.
    NewerGenerationPending,
}

// ──────────────────── automatic-apply dirty barrier ────────────────────

/// Durable local work that must drain before automatic writing application
/// may touch one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingLocalWrites {
    /// Recovery journal deltas not yet folded into a canonical revision.
    pub(crate) pending_journal_entries: i64,
    /// A writing outbox entry still awaiting its push acknowledgment.
    pub(crate) pending_outbox_entry: bool,
}

impl PendingLocalWrites {
    /// The W-GUARD2 rule for automatic cycles: defer while EITHER durable
    /// source still holds local work for this document.
    pub(crate) fn blocks_automatic_apply(&self) -> bool {
        self.pending_journal_entries > 0 || self.pending_outbox_entry
    }
}

/// The W-GUARD2 backend dirty barrier: reports whether one document has
/// pending recovery-journal entries or a pending writing outbox entry, so a
/// future writing-receive caller can defer automatic application of a pulled
/// row or a queued receive instead of touching a document with durable local
/// work.
///
/// "Pending journal" follows the journal's own pending semantics (deltas on
/// top of the document's current revision or later). Deltas already folded
/// into a canonical revision are content, and the save that folded them owns
/// an outbox entry until its push is acknowledged — that is the outbox side
/// of this barrier. Scope is exactly one document: another document's journal
/// or outbox entry never blocks this one.
///
/// This reads durable state only. It cannot see frontend keystrokes the editor
/// has not journaled yet, so it does NOT by itself capture unsaved frontend
/// state: activating automatic writing sync requires BOTH the W-GUARD1 manual
/// flush before a sync AND this durable barrier inside automatic cycles.
/// Neither one alone is sufficient.
pub(crate) fn pending_local_writes(
    conn: &Connection,
    document_id: &str,
) -> WritingResult<PendingLocalWrites> {
    require_receive_schema(conn)?;
    let revision = conn
        .query_row(
            "SELECT revision FROM writing_documents WHERE id = ?1",
            params![document_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| WritingError::sql("Failed to read writing revision", error))?
        .unwrap_or(0);
    Ok(PendingLocalWrites {
        pending_journal_entries: pending_journal_count(conn, document_id, revision)?,
        pending_outbox_entry: outbox_entries(conn)?
            .iter()
            .any(|entry| entry.document_id == document_id),
    })
}

// ─────────────────────────────── push side ───────────────────────────────

/// Builds one wire draft per ready outbox entry. Documents whose attachments
/// cannot be proven stay listed as `pending_files` and keep their outbox entry.
/// The savepoint pins the outbox and document snapshot to one database view, so
/// every returned acknowledgment identity describes the exact state encoded by
/// its draft.
pub(crate) fn build_push_changes(conn: &Connection, data_root: &Path) -> WritingResult<PushBuild> {
    with_transport_savepoint(conn, || build_push_changes_in_savepoint(conn, data_root))
}

fn build_push_changes_in_savepoint(
    conn: &Connection,
    data_root: &Path,
) -> WritingResult<PushBuild> {
    let mut build = PushBuild::default();
    for entry in outbox_entries(conn)? {
        let outgoing = match build_outgoing(conn, &entry.document_id, data_root) {
            Ok(outgoing) => outgoing,
            Err(error) if error.code == super::repository::DOCUMENT_NOT_FOUND => {
                // An outbox entry whose document vanished is reported as
                // pending work, never silently dropped.
                build.pending_files.push(entry.document_id.clone());
                continue;
            }
            Err(error) => return Err(error),
        };
        match outgoing {
            OutgoingEnvelope::Ready {
                mut envelope,
                manifest,
            } => {
                envelope.attachments_manifest = manifest;
                let payload: Value =
                    serde_json::from_str(&envelope.to_canonical_json()?).map_err(|error| {
                        WritingError::new(
                            ENVELOPE_UNSUPPORTED,
                            format!("cannot encode envelope {}: {error}", entry.document_id),
                        )
                    })?;
                build.ready.push(WritingChangeDraft {
                    document_id: entry.document_id.clone(),
                    op: entry.op,
                    changed_at: entry.changed_at,
                    base_seq: recorded_server_seq(conn, &entry.document_id)?,
                    payload,
                    acknowledgment: entry.acknowledgment,
                });
            }
            OutgoingEnvelope::PendingFiles => build.pending_files.push(entry.document_id.clone()),
        }
    }
    Ok(build)
}

/// Conditionally acknowledges the exact outbox generation captured by a push
/// draft. Future engine wiring should call this only after that draft receives
/// a successful server acknowledgment; pulled own-device rows are not proof.
#[cfg(test)]
pub(crate) fn acknowledge_push_draft(
    conn: &Connection,
    draft: &WritingChangeDraft,
) -> WritingResult<bool> {
    acknowledge_outbox_entry(conn, &draft.acknowledgment)
}

/// Atomic result of settling an `applied` or `lww_won` response for one exact
/// writing draft.
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

/// Records a monotonic row version and acknowledges only the generation that
/// produced `draft`. The savepoint composes with a caller-owned transaction;
/// any bookkeeping failure restores both the row version and the outbox entry.
pub(crate) fn settle_applied_push_draft(
    conn: &Connection,
    draft: &WritingChangeDraft,
    server_seq: i64,
) -> WritingResult<PushDraftSettlement> {
    if server_seq <= 0 {
        return Err(WritingError::new(
            ENVELOPE_UNSUPPORTED,
            format!("writing push returned invalid server_seq {server_seq}"),
        ));
    }
    if draft.document_id != draft.acknowledgment.document_id {
        return Err(WritingError::new(
            ENVELOPE_UNSUPPORTED,
            "writing draft acknowledgment does not match its document",
        ));
    }

    with_transport_savepoint(conn, || {
        let recorded = recorded_server_seq(conn, &draft.document_id)?;
        if recorded > server_seq {
            return Ok(PushDraftSettlement::StaleServerSequence {
                recorded_server_seq: recorded,
                response_server_seq: server_seq,
            });
        }

        let current = outbox_entries(conn)?
            .into_iter()
            .find(|entry| entry.document_id == draft.document_id);
        let Some(current) = current else {
            return Ok(PushDraftSettlement::AlreadySettled);
        };

        record_server_seq_monotonic(conn, &draft.document_id, server_seq)?;
        if current.acknowledgment != draft.acknowledgment {
            return Ok(PushDraftSettlement::NewerGenerationPending);
        }

        if !acknowledge_outbox_entry(conn, &draft.acknowledgment)? {
            return Err(WritingError::new(
                "writing_ack_raced",
                format!(
                    "writing outbox generation changed while settling {}",
                    draft.document_id
                ),
            ));
        }
        Ok(PushDraftSettlement::Acknowledged)
    })
}

#[allow(clippy::large_enum_variant)] // short-lived per-document value; boxing would only add churn
enum OutgoingEnvelope {
    Ready {
        envelope: WritingEnvelopeV1,
        manifest: AttachmentManifestV1,
    },
    PendingFiles,
}

/// Only a scan of the current local bytes can prove an outgoing manifest.
/// Accepted peer metadata is not evidence that the server still has a blob.
fn build_outgoing(
    conn: &Connection,
    document_id: &str,
    data_root: &Path,
) -> WritingResult<OutgoingEnvelope> {
    let envelope = snapshot_document(conn, document_id)?;
    let regions = attachment_source_regions(&envelope);
    let scan = scan_document_attachments(&envelope.content_json, &regions, data_root);
    if scan.is_transfer_ready() {
        Ok(OutgoingEnvelope::Ready {
            envelope,
            manifest: scan.manifest,
        })
    } else {
        Ok(OutgoingEnvelope::PendingFiles)
    }
}

// ─────────────────────────────── pull side ───────────────────────────────

/// Applies one pulled writing row. The caller owns the outer transaction; a
/// transport savepoint keeps each authoritative receive, outbox decision, and
/// row-version update atomic inside it.
pub(crate) fn apply_pulled_row(
    conn: &Connection,
    own_device_id: &str,
    row: &PulledWritingRow,
    data_root: &Path,
) -> WritingResult<PullApplyOutcome> {
    let recorded = recorded_server_seq(conn, &row.document_id)?;
    if recorded >= row.server_seq {
        return Ok(PullApplyOutcome::Stale);
    }

    if row.device_id == own_device_id && document_exists(conn, &row.document_id)? {
        // Device identity and a wire timestamp cannot identify which local
        // generation was pushed. The exact draft acknowledgment API is the
        // only path that may remove pending work. A missing local document
        // is not an echo: it must be restored through the verified receive.
        record_server_seq(conn, &row.document_id, row.server_seq)?;
        return Ok(PullApplyOutcome::OwnChangeObserved);
    }

    with_transport_savepoint(conn, || {
        apply_remote_pulled_row(conn, own_device_id, row, data_root)
    })
}

fn apply_remote_pulled_row(
    conn: &Connection,
    own_device_id: &str,
    row: &PulledWritingRow,
    data_root: &Path,
) -> WritingResult<PullApplyOutcome> {
    if row.deleted {
        return apply_tombstone(conn, own_device_id, row, data_root);
    }

    let Some(payload) = &row.payload else {
        return Ok(PullApplyOutcome::Unsupported {
            reason: "upsert row without payload".to_string(),
        });
    };
    let envelope = match WritingEnvelopeV1::from_json(&payload.to_string()) {
        Ok(envelope) => envelope,
        Err(error) => {
            return Ok(PullApplyOutcome::Unsupported {
                reason: error.message,
            })
        }
    };
    if envelope.id != row.document_id {
        return Ok(PullApplyOutcome::Unsupported {
            reason: format!(
                "envelope id {} does not match row {}",
                envelope.id, row.document_id
            ),
        });
    }

    let incoming_receipt = match attachment_receipt(&envelope, data_root)? {
        AttachmentReceiptProof::Verified(receipt) => receipt,
        AttachmentReceiptProof::Unproven(reason) => {
            return Ok(PullApplyOutcome::Deferred {
                reason: format!("incoming attachments are not locally verified: {reason}"),
            })
        }
    };
    let incoming_manifest = envelope.attachments_manifest.clone();
    let local_exists = document_exists(conn, &row.document_id)?;
    if !local_exists {
        let outcome = receive_envelope(
            conn,
            &row.document_id,
            &envelope,
            &ReceiveAuthorization::CreateOnly,
            &incoming_receipt,
        )?;
        return finish_receive(conn, row, outcome, None, &incoming_manifest);
    }

    // Divergent local edits (or a pending journal, which receive also checks)
    // mean the local manuscript must survive as a conflict copy first.
    let local_edits_pending = outbox_entries(conn)?
        .iter()
        .any(|entry| entry.document_id == row.document_id);
    if local_edits_pending {
        return apply_winner_with_conflict(
            conn,
            own_device_id,
            row,
            &envelope,
            &incoming_receipt,
            &incoming_manifest,
            data_root,
        );
    }

    let existing = snapshot_document(conn, &row.document_id)?;
    let local_revision = current_revision(conn, &row.document_id)?;
    let local_fingerprint = existing.fingerprint_sha256()?;
    let outcome = receive_envelope(
        conn,
        &row.document_id,
        &envelope,
        &ReceiveAuthorization::Replace {
            expected_local_revision: local_revision,
            expected_local_fingerprint_sha256: local_fingerprint,
        },
        &incoming_receipt,
    )?;
    finish_receive(conn, row, outcome, None, &incoming_manifest)
}

/// Settles one validated `lww_lost` push result: the server adjudicated the
/// pushed generation as a loser against `winner`. This is the ONLY automatic
/// path allowed to settle a remote row while a local outbox entry exists — the
/// push response proves the pending generation lost, so `winner` can never be
/// that generation's echo and the loser must survive as a visible conflict
/// copy instead of being retried into the same deadlock.
///
/// `adjudicated_generation` is the exact outbox generation the settled draft
/// captured. A save during the send round trip replaces it, and the newer
/// generation is NEVER cleared here: it keeps its outbox entry and is
/// adjudicated by its own push. A pending recovery journal still defers
/// through the guarded receive — nothing unsaved is ever overwritten — and a
/// replay of an already-settled winner is a stale no-op.
pub(crate) fn apply_lww_lost_winner(
    conn: &Connection,
    own_device_id: &str,
    winner: &PulledWritingRow,
    adjudicated_generation: Option<&OutboxAcknowledgment>,
    data_root: &Path,
) -> WritingResult<LwwLostSettlement> {
    with_transport_savepoint(conn, || {
        let recorded = recorded_server_seq(conn, &winner.document_id)?;
        if recorded >= winner.server_seq {
            return Ok(LwwLostSettlement::Routed(PullApplyOutcome::Stale));
        }

        // Only the exact adjudicated generation may be preserved-and-cleared.
        // The savepoint pins this comparison to the receive below, so a
        // generation that replaced the adjudicated one is never dropped.
        let current = outbox_entries(conn)?
            .into_iter()
            .find(|entry| entry.document_id == winner.document_id)
            .map(|entry| entry.acknowledgment);
        if current.is_some() && current.as_ref() != adjudicated_generation {
            return Ok(LwwLostSettlement::NewerGenerationPending);
        }

        // Unlike [`apply_pulled_row`] there is no own-echo shortcut: the
        // server explicitly named this row the winner that beat the
        // adjudicated generation, so device identity proves nothing here and
        // the guarded receive below owns every overwrite decision.
        Ok(LwwLostSettlement::Routed(apply_remote_pulled_row(
            conn,
            own_device_id,
            winner,
            data_root,
        )?))
    })
}

/// The divergent-edit path: preserve the local manuscript, then let the
/// guarded receiver decide. A retry of the same pair re-creates nothing.
fn apply_winner_with_conflict(
    conn: &Connection,
    own_device_id: &str,
    row: &PulledWritingRow,
    winner: &WritingEnvelopeV1,
    winner_receipt: &AttachmentInstallReceipt,
    winner_manifest: &AttachmentManifestV1,
    data_root: &Path,
) -> WritingResult<PullApplyOutcome> {
    // Divergence preservation needs a manifest-complete loser, but the
    // overwrite guard must bind to the RAW local snapshot fingerprint — that
    // is exactly what the receiver recomputes before replacing.
    let expected_fingerprint = snapshot_document(conn, &row.document_id)?.fingerprint_sha256()?;
    let expected_revision = current_revision(conn, &row.document_id)?;
    let loser = match proven_local_envelope(conn, &row.document_id, data_root)? {
        ProvenEnvelope::Ready(envelope) => envelope,
        ProvenEnvelope::Unproven => {
            // Never overwrite text we cannot yet snapshot completely.
            return Ok(PullApplyOutcome::Deferred {
                reason: "local attachments not proven for conflict preservation".to_string(),
            });
        }
    };
    let loser_receipt = match attachment_receipt(&loser, data_root)? {
        AttachmentReceiptProof::Verified(receipt) => receipt,
        AttachmentReceiptProof::Unproven(reason) => {
            return Ok(PullApplyOutcome::Deferred {
                reason: format!("local attachments are not verified for preservation: {reason}"),
            })
        }
    };
    let source = ConflictSourceMetadata {
        source_document_id: row.document_id.clone(),
        source_device_id: own_device_id.to_string(),
        source_device_label: own_device_id.to_string(),
        source_captured_at_ms: now_ms(),
    };
    let outcome = preserve_loser_then_receive_winner(
        conn,
        &loser,
        &source,
        &loser_receipt,
        winner,
        expected_revision,
        &expected_fingerprint,
        winner_receipt,
    )?;
    match outcome {
        ConflictThenReceiveOutcome::Applied {
            conflict_document_id,
            winner,
            ..
        } => {
            enqueue_document(conn, &conflict_document_id)?;
            finish_receive(
                conn,
                row,
                winner,
                Some(conflict_document_id),
                winner_manifest,
            )
        }
        ConflictThenReceiveOutcome::Deferred { reason, .. } => Ok(PullApplyOutcome::Deferred {
            reason: format!("{reason:?}"),
        }),
        ConflictThenReceiveOutcome::WinnerNotApplied { winner } => match winner {
            ReceiveOutcome::Deferred { reason, .. } => Ok(PullApplyOutcome::Deferred {
                reason: format!("{reason:?}"),
            }),
            ReceiveOutcome::ConflictRequired { .. } => Ok(PullApplyOutcome::Deferred {
                reason: "local state moved during conflict preservation".to_string(),
            }),
            other => finish_receive(conn, row, other, None, winner_manifest),
        },
    }
}

fn apply_tombstone(
    conn: &Connection,
    own_device_id: &str,
    row: &PulledWritingRow,
    data_root: &Path,
) -> WritingResult<PullApplyOutcome> {
    require_receive_schema(conn)?;
    if !document_exists(conn, &row.document_id)? {
        discard_outbox_for_authoritative_receive(conn, &row.document_id)?;
        clear_manifest_state(conn, &row.document_id)?;
        record_server_seq(conn, &row.document_id, row.server_seq)?;
        return Ok(PullApplyOutcome::NoOp);
    }

    let local_revision = current_revision(conn, &row.document_id)?;
    let pending_journal = pending_journal_count(conn, &row.document_id, local_revision)?;
    if pending_journal > 0 {
        return Ok(PullApplyOutcome::Deferred {
            reason: format!(
                "{:?}",
                ReceiveDeferred::PendingJournal {
                    entries: pending_journal,
                }
            ),
        });
    }

    let local_edits_pending = outbox_entries(conn)?
        .iter()
        .any(|entry| entry.document_id == row.document_id);
    let mut conflict_document_id = None;
    if local_edits_pending {
        // The deletion is the winner; the local manuscript survives as a copy.
        let loser = match proven_local_envelope(conn, &row.document_id, data_root)? {
            ProvenEnvelope::Ready(envelope) => envelope,
            ProvenEnvelope::Unproven => {
                return Ok(PullApplyOutcome::Deferred {
                    reason: "local attachments not proven for tombstone preservation".to_string(),
                });
            }
        };
        let loser_receipt = match attachment_receipt(&loser, data_root)? {
            AttachmentReceiptProof::Verified(receipt) => receipt,
            AttachmentReceiptProof::Unproven(reason) => {
                return Ok(PullApplyOutcome::Deferred {
                    reason: format!(
                        "local attachments are not verified for tombstone preservation: {reason}"
                    ),
                })
            }
        };
        let source = ConflictSourceMetadata {
            source_document_id: row.document_id.clone(),
            source_device_id: own_device_id.to_string(),
            source_device_label: own_device_id.to_string(),
            source_captured_at_ms: now_ms(),
        };
        match preserve_conflict_copy(conn, &loser, &source, &loser_receipt)? {
            super::sync_conflict::ConflictCopyOutcome::Preserved { document_id, .. } => {
                enqueue_document(conn, &document_id)?;
                conflict_document_id = Some(document_id);
            }
            super::sync_conflict::ConflictCopyOutcome::Deferred { reason, .. } => {
                return Ok(PullApplyOutcome::Deferred {
                    reason: format!("{reason:?}"),
                });
            }
        }
    }

    conn.execute(
        "DELETE FROM writing_documents WHERE id = ?1",
        params![row.document_id],
    )
    .map_err(|error| WritingError::sql("Failed to apply writing tombstone", error))?;
    discard_outbox_for_authoritative_receive(conn, &row.document_id)?;
    clear_manifest_state(conn, &row.document_id)?;
    record_server_seq(conn, &row.document_id, row.server_seq)?;
    Ok(PullApplyOutcome::Applied {
        created: false,
        conflict_document_id,
    })
}

/// Completes authoritative receive bookkeeping inside the transport savepoint
/// opened by [`apply_pulled_row`]. The unconditional outbox discard is safe only
/// because a bookkeeping failure rolls the document mutation back with it.
fn finish_receive(
    conn: &Connection,
    row: &PulledWritingRow,
    outcome: ReceiveOutcome,
    conflict_document_id: Option<String>,
    accepted_manifest: &AttachmentManifestV1,
) -> WritingResult<PullApplyOutcome> {
    match outcome {
        ReceiveOutcome::Applied { created, .. } => {
            record_server_seq(conn, &row.document_id, row.server_seq)?;
            discard_outbox_for_authoritative_receive(conn, &row.document_id)?;
            track_incoming_files(conn, &row.document_id, accepted_manifest)?;
            Ok(PullApplyOutcome::Applied {
                created,
                conflict_document_id,
            })
        }
        ReceiveOutcome::NoOp { .. } => {
            record_server_seq(conn, &row.document_id, row.server_seq)?;
            discard_outbox_for_authoritative_receive(conn, &row.document_id)?;
            track_incoming_files(conn, &row.document_id, accepted_manifest)?;
            Ok(PullApplyOutcome::NoOp)
        }
        ReceiveOutcome::Deferred { reason, .. } => Ok(PullApplyOutcome::Deferred {
            reason: format!("{reason:?}"),
        }),
        ReceiveOutcome::ConflictRequired { conflict, .. } => Ok(PullApplyOutcome::Deferred {
            reason: format!("{:?}", conflict.reason),
        }),
    }
}

// ─────────────────────────────── helpers ───────────────────────────────

fn with_transport_savepoint<T>(
    conn: &Connection,
    operation: impl FnOnce() -> WritingResult<T>,
) -> WritingResult<T> {
    conn.execute_batch(&format!("SAVEPOINT {TRANSPORT_SAVEPOINT};"))
        .map_err(|error| WritingError::sql("Failed to open writing transport savepoint", error))?;

    match operation() {
        Ok(value) => match conn.execute_batch(&format!("RELEASE {TRANSPORT_SAVEPOINT};")) {
            Ok(()) => Ok(value),
            Err(release_error) => {
                rollback_transport_savepoint(conn).map_err(|rollback_error| {
                    WritingError::new(
                        "sql_error",
                        format!(
                            "Failed to release writing transport savepoint: {release_error}; \
                             rollback also failed: {rollback_error}"
                        ),
                    )
                })?;
                Err(WritingError::sql(
                    "Failed to release writing transport savepoint",
                    release_error,
                ))
            }
        },
        Err(error) => {
            rollback_transport_savepoint(conn).map_err(|rollback_error| {
                WritingError::new(
                    "sql_error",
                    format!(
                        "{}; failed to roll back writing transport savepoint: {rollback_error}",
                        error.message
                    ),
                )
            })?;
            Err(error)
        }
    }
}

fn rollback_transport_savepoint(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(&format!(
        "ROLLBACK TO {TRANSPORT_SAVEPOINT}; RELEASE {TRANSPORT_SAVEPOINT};"
    ))
}

#[allow(clippy::large_enum_variant)] // short-lived per-document value; boxing would only add churn
enum ProvenEnvelope {
    Ready(WritingEnvelopeV1),
    Unproven,
}

/// The local document as a fully provable envelope, or `Unproven` when its
/// current referenced files cannot be attested from local bytes.
fn proven_local_envelope(
    conn: &Connection,
    document_id: &str,
    data_root: &Path,
) -> WritingResult<ProvenEnvelope> {
    let envelope = snapshot_document(conn, document_id)?;
    let regions = attachment_source_regions(&envelope);
    let scan = scan_document_attachments(&envelope.content_json, &regions, data_root);
    if scan.is_transfer_ready() {
        Ok(ProvenEnvelope::Ready(with_manifest(
            envelope,
            scan.manifest,
        )))
    } else {
        Ok(ProvenEnvelope::Unproven)
    }
}

fn with_manifest(
    mut envelope: WritingEnvelopeV1,
    manifest: AttachmentManifestV1,
) -> WritingEnvelopeV1 {
    envelope.attachments_manifest = manifest;
    envelope
}

fn current_revision(conn: &Connection, document_id: &str) -> WritingResult<i64> {
    conn.query_row(
        "SELECT revision FROM writing_documents WHERE id = ?1",
        params![document_id],
        |row| row.get(0),
    )
    .map_err(|error| WritingError::sql("Failed to read writing revision", error))
}

enum AttachmentReceiptProof {
    Verified(AttachmentInstallReceipt),
    Unproven(String),
}

fn attachment_source_regions(envelope: &WritingEnvelopeV1) -> Vec<Value> {
    envelope
        .citation_projections
        .corpus
        .iter()
        .filter_map(|citation| citation.source_region_json.clone())
        .collect()
}

/// Constructs a receipt only after the manifest exactly covers the envelope's
/// references and every claimed local file has been re-read and verified.
fn attachment_receipt(
    envelope: &WritingEnvelopeV1,
    data_root: &Path,
) -> WritingResult<AttachmentReceiptProof> {
    let regions = attachment_source_regions(envelope);
    if let Err(issue) = verify_attachment_manifest(
        &envelope.content_json,
        &regions,
        data_root,
        &envelope.attachments_manifest,
    ) {
        return Ok(AttachmentReceiptProof::Unproven(issue.to_string()));
    }

    Ok(AttachmentReceiptProof::Verified(
        AttachmentInstallReceipt::caller_asserts_installed(envelope.fingerprint_sha256()?),
    ))
}

/// Stores only a manifest accepted by a successful receive and clears any old
/// pending marker. Every database error escapes to the transport savepoint.
fn track_incoming_files(
    conn: &Connection,
    document_id: &str,
    manifest: &AttachmentManifestV1,
) -> WritingResult<()> {
    store_manifest(conn, document_id, manifest)?;
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1",
        params![format!("{PENDING_ASSETS_PREFIX}{document_id}")],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to clear writing pending assets", error))
}

fn store_manifest(
    conn: &Connection,
    document_id: &str,
    manifest: &AttachmentManifestV1,
) -> WritingResult<()> {
    let raw = serde_json::to_string(manifest).map_err(|error| {
        WritingError::new(
            ENVELOPE_UNSUPPORTED,
            format!("cannot store manifest for {document_id}: {error}"),
        )
    })?;
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![format!("{MANIFEST_PREFIX}{document_id}"), raw],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to store writing manifest", error))
}

fn clear_manifest_state(conn: &Connection, document_id: &str) -> WritingResult<()> {
    for key in [
        format!("{MANIFEST_PREFIX}{document_id}"),
        format!("{PENDING_ASSETS_PREFIX}{document_id}"),
    ] {
        conn.execute("DELETE FROM sync_meta WHERE key = ?1", params![key])
            .map_err(|error| WritingError::sql("Failed to clear writing manifest state", error))?;
    }
    Ok(())
}

fn document_exists(conn: &Connection, document_id: &str) -> WritingResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM writing_documents WHERE id = ?1)",
        params![document_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
    .map_err(|error| WritingError::sql("Failed to inspect writing document", error))
}

pub(crate) fn recorded_server_seq(conn: &Connection, document_id: &str) -> WritingResult<i64> {
    conn.query_row(
        "SELECT server_seq FROM sync_row_versions WHERE table_name = ?1 AND row_id = ?2",
        params![ENVELOPE_TABLE, document_id],
        |row| row.get(0),
    )
    .optional()
    .map(|value| value.unwrap_or(0))
    .map_err(|error| WritingError::sql("Failed to read writing row version", error))
}

fn record_server_seq_monotonic(
    conn: &Connection,
    document_id: &str,
    server_seq: i64,
) -> WritingResult<()> {
    conn.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq) VALUES (?1, ?2, ?3)
         ON CONFLICT(table_name, row_id) DO UPDATE SET server_seq = excluded.server_seq
          WHERE excluded.server_seq > sync_row_versions.server_seq",
        params![ENVELOPE_TABLE, document_id, server_seq],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to record writing row version", error))
}

fn record_server_seq(conn: &Connection, document_id: &str, server_seq: i64) -> WritingResult<()> {
    conn.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq) VALUES (?1, ?2, ?3)
         ON CONFLICT(table_name, row_id) DO UPDATE SET server_seq = excluded.server_seq",
        params![ENVELOPE_TABLE, document_id, server_seq],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to record writing row version", error))
}

// ─────────────────────────── activation catch-up ───────────────────────────

/// True until this server epoch's one-time full writing catch-up has run.
/// A client that pulled past hidden writing rows cannot go incremental until
/// it has seen the whole account once.
pub(crate) fn catchup_needed(conn: &Connection, server_epoch: &str) -> WritingResult<bool> {
    let recorded: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![CATCHUP_EPOCH_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| WritingError::sql("Failed to read writing catch-up state", error))?;
    Ok(recorded.as_deref() != Some(server_epoch))
}

/// Marks this server epoch's writing catch-up complete.
pub(crate) fn record_catchup_done(conn: &Connection, server_epoch: &str) -> WritingResult<()> {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CATCHUP_EPOCH_KEY, server_epoch],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to record writing catch-up", error))
}
