//! Offline conflict-copy preservation for validated writing-envelope snapshots.
//!
//! This module is intentionally transport-free. It creates an ordinary visible
//! writing document plus one immutable provenance marker, all inside the
//! caller's transaction and a bounded savepoint.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use super::repository::{WritingError, WritingResult};
use super::sync_capture::{OUTBOX_PREFIX, PENDING_ASSETS_PREFIX, RECEIVE_PREFIX};
use super::sync_envelope::{snapshot_document, AttachmentManifestV1, WritingEnvelopeV1};
use super::sync_receive::{
    apply_receive_plan, plan_receive_inside_savepoint, require_receive_schema,
    validate_envelope_and_attachments, validate_receive_authorization, with_receive_savepoint,
    AttachmentInstallReceipt, ReceiveAuthorization, ReceiveDeferred, ReceiveOutcome, ReceivePlan,
    ValidatedReceiveEnvelope,
};

const CONFLICT_MARKER_KIND: &str = "writing_sync_conflict_copy";
const CONFLICT_MARKER_VERSION: u32 = 1;

pub(crate) const INVALID_CONFLICT_SOURCE_METADATA: &str =
    "invalid_writing_conflict_source_metadata";
pub(crate) const CONFLICT_COPY_COLLISION: &str = "writing_conflict_copy_collision";
pub(crate) const CONFLICT_LOSER_STATE_MISMATCH: &str = "writing_conflict_loser_state_mismatch";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConflictSourceMetadata {
    pub(crate) source_document_id: String,
    pub(crate) source_device_id: String,
    pub(crate) source_device_label: String,
    pub(crate) source_captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ConflictCopyOutcome {
    Preserved {
        document_id: String,
        semantic_fingerprint_sha256: String,
        created: bool,
    },
    Deferred {
        document_id: String,
        reason: ReceiveDeferred,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ConflictThenReceiveOutcome {
    Applied {
        conflict_document_id: String,
        conflict_semantic_fingerprint_sha256: String,
        conflict_created: bool,
        winner: ReceiveOutcome,
    },
    Deferred {
        conflict_document_id: String,
        reason: ReceiveDeferred,
    },
    WinnerNotApplied {
        winner: ReceiveOutcome,
    },
}

#[derive(Debug, Clone)]
struct ValidatedConflictSource {
    source: ConflictSourceMetadata,
    envelope: WritingEnvelopeV1,
    envelope_fingerprint_sha256: String,
    semantic_fingerprint_sha256: String,
    conflict_document_id: String,
    conflict_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictOriginMarkerV1 {
    kind: String,
    version: u32,
    source_document_id: String,
    source_device_id: String,
    source_device_label: String,
    source_captured_at_ms: i64,
    source_title: String,
    semantic_fingerprint_sha256: String,
    attachment_receipt_envelope_fingerprint_sha256: String,
    attachments_manifest: AttachmentManifestV1,
}

struct StoredMarkerRow {
    document_id: String,
    version_id: Option<String>,
    range_anchor_json: Option<String>,
    origin_type: String,
    operation_type: String,
    source_reference_json: Option<String>,
    model_provider: Option<String>,
    model_name: Option<String>,
    prompt_template_id: Option<String>,
    created_at: i64,
}

/// Preserves one full losing version as an ordinary document without changing
/// the source document. Replays of the same semantic version return the first
/// copy and never append another provenance marker.
pub(crate) fn preserve_conflict_copy(
    conn: &Connection,
    losing_envelope: &WritingEnvelopeV1,
    source: &ConflictSourceMetadata,
    attachments: &AttachmentInstallReceipt,
) -> WritingResult<ConflictCopyOutcome> {
    let validated = validate_conflict_source(losing_envelope, source, attachments)?;
    require_receive_schema(conn)?;
    with_receive_savepoint(conn, || {
        preserve_validated_inside_savepoint(conn, &validated)
    })
}

/// Atomically preserves the current losing document before applying a guarded
/// winner. All stale-state, journal, and collection checks run before the copy
/// is written. A non-applying receiver outcome leaves no conflict copy behind.
#[expect(
    clippy::too_many_arguments,
    reason = "the offline composition keeps both envelope receipts and overwrite guards explicit"
)]
pub(crate) fn preserve_loser_then_receive_winner(
    conn: &Connection,
    losing_envelope: &WritingEnvelopeV1,
    source: &ConflictSourceMetadata,
    losing_attachments: &AttachmentInstallReceipt,
    winner_envelope: &WritingEnvelopeV1,
    expected_local_revision: i64,
    expected_local_fingerprint_sha256: &str,
    winner_attachments: &AttachmentInstallReceipt,
) -> WritingResult<ConflictThenReceiveOutcome> {
    let loser = validate_conflict_source(losing_envelope, source, losing_attachments)?;
    let winner = validate_envelope_and_attachments(
        &source.source_document_id,
        winner_envelope,
        winner_attachments,
    )?;
    let authorization = ReceiveAuthorization::Replace {
        expected_local_revision,
        expected_local_fingerprint_sha256: expected_local_fingerprint_sha256.to_string(),
    };
    validate_receive_authorization(&authorization)?;
    require_receive_schema(conn)?;

    with_receive_savepoint(conn, || {
        let plan = plan_receive_inside_savepoint(
            conn,
            &source.source_document_id,
            &winner.envelope,
            &authorization,
        )?;

        match plan {
            ReceivePlan::Outcome(winner) => {
                Ok(ConflictThenReceiveOutcome::WinnerNotApplied { winner })
            }
            apply_plan @ ReceivePlan::Apply { .. } => {
                validate_loser_matches_current(conn, &loser)?;
                match preserve_validated_inside_savepoint(conn, &loser)? {
                    ConflictCopyOutcome::Deferred {
                        document_id,
                        reason,
                    } => Ok(ConflictThenReceiveOutcome::Deferred {
                        conflict_document_id: document_id,
                        reason,
                    }),
                    ConflictCopyOutcome::Preserved {
                        document_id,
                        semantic_fingerprint_sha256,
                        created,
                    } => {
                        let winner =
                            apply_receive_plan(conn, &source.source_document_id, apply_plan)?;
                        Ok(ConflictThenReceiveOutcome::Applied {
                            conflict_document_id: document_id,
                            conflict_semantic_fingerprint_sha256: semantic_fingerprint_sha256,
                            conflict_created: created,
                            winner,
                        })
                    }
                }
            }
        }
    })
}

fn validate_conflict_source(
    losing_envelope: &WritingEnvelopeV1,
    source: &ConflictSourceMetadata,
    attachments: &AttachmentInstallReceipt,
) -> WritingResult<ValidatedConflictSource> {
    validate_source_metadata(source)?;
    let ValidatedReceiveEnvelope {
        envelope,
        envelope_fingerprint_sha256,
    } = validate_envelope_and_attachments(
        &source.source_document_id,
        losing_envelope,
        attachments,
    )?;
    let semantic_fingerprint_sha256 = envelope.conflict_semantic_fingerprint_sha256()?;
    let conflict_document_id =
        conflict_copy_document_id(&source.source_document_id, &semantic_fingerprint_sha256);
    let conflict_title = conflict_copy_title(&envelope.title, &semantic_fingerprint_sha256);

    Ok(ValidatedConflictSource {
        source: source.clone(),
        envelope,
        envelope_fingerprint_sha256,
        semantic_fingerprint_sha256,
        conflict_document_id,
        conflict_title,
    })
}

fn validate_source_metadata(source: &ConflictSourceMetadata) -> WritingResult<()> {
    for (field, value) in [
        ("source_document_id", source.source_document_id.as_str()),
        ("source_device_id", source.source_device_id.as_str()),
        ("source_device_label", source.source_device_label.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(WritingError::new(
                INVALID_CONFLICT_SOURCE_METADATA,
                format!("{field} must not be empty"),
            ));
        }
    }
    if source.source_captured_at_ms < 0 {
        return Err(WritingError::new(
            INVALID_CONFLICT_SOURCE_METADATA,
            format!(
                "source_captured_at_ms must be non-negative, found {}",
                source.source_captured_at_ms
            ),
        ));
    }
    Ok(())
}

fn preserve_validated_inside_savepoint(
    conn: &Connection,
    validated: &ValidatedConflictSource,
) -> WritingResult<ConflictCopyOutcome> {
    if document_exists(conn, &validated.conflict_document_id)? {
        validate_existing_copy(conn, validated)?;
        return Ok(ConflictCopyOutcome::Preserved {
            document_id: validated.conflict_document_id.clone(),
            semantic_fingerprint_sha256: validated.semantic_fingerprint_sha256.clone(),
            created: false,
        });
    }

    if provenance_marker_exists(conn, &conflict_marker_id(&validated.conflict_document_id))? {
        return Err(collision_error(
            &validated.conflict_document_id,
            "the deterministic provenance marker is already owned by another document",
        ));
    }

    let mut copy = validated.envelope.clone();
    copy.id.clone_from(&validated.conflict_document_id);
    copy.title.clone_from(&validated.conflict_title);
    let plan = plan_receive_inside_savepoint(
        conn,
        &validated.conflict_document_id,
        &copy,
        &ReceiveAuthorization::CreateOnly,
    )?;

    match plan {
        ReceivePlan::Outcome(ReceiveOutcome::Deferred { reason, .. }) => {
            Ok(ConflictCopyOutcome::Deferred {
                document_id: validated.conflict_document_id.clone(),
                reason,
            })
        }
        ReceivePlan::Outcome(_) => Err(collision_error(
            &validated.conflict_document_id,
            "the deterministic document id became occupied before insertion",
        )),
        apply_plan @ ReceivePlan::Apply { .. } => {
            let applied = apply_receive_plan(conn, &validated.conflict_document_id, apply_plan)?;
            if !matches!(
                applied,
                ReceiveOutcome::Applied {
                    local_revision: 0,
                    created: true,
                    ..
                }
            ) {
                return Err(collision_error(
                    &validated.conflict_document_id,
                    "the receiver did not create the expected revision-zero copy",
                ));
            }
            insert_origin_marker(conn, validated)?;
            Ok(ConflictCopyOutcome::Preserved {
                document_id: validated.conflict_document_id.clone(),
                semantic_fingerprint_sha256: validated.semantic_fingerprint_sha256.clone(),
                created: true,
            })
        }
    }
}

fn validate_loser_matches_current(
    conn: &Connection,
    loser: &ValidatedConflictSource,
) -> WritingResult<()> {
    let mut current = snapshot_document(conn, &loser.source.source_document_id)?;
    current.attachments_manifest = loser.envelope.attachments_manifest.clone();
    let current_semantic = current.conflict_semantic_fingerprint_sha256()?;
    if current_semantic != loser.semantic_fingerprint_sha256 {
        return Err(WritingError::new(
            CONFLICT_LOSER_STATE_MISMATCH,
            format!(
                "losing envelope does not match current document {}",
                loser.source.source_document_id
            ),
        ));
    }
    Ok(())
}

fn validate_existing_copy(
    conn: &Connection,
    expected: &ValidatedConflictSource,
) -> WritingResult<()> {
    let marker_id = conflict_marker_id(&expected.conflict_document_id);
    let Some((row, marker)) = load_origin_marker(conn, &marker_id)? else {
        return Err(collision_error(
            &expected.conflict_document_id,
            "an existing document has no matching conflict-origin marker",
        ));
    };

    let marker_shape_matches = row.document_id == expected.conflict_document_id
        && row.version_id.is_none()
        && row.range_anchor_json.is_none()
        && row.origin_type == "import"
        && row.operation_type == "other"
        && row.model_provider.is_none()
        && row.model_name.is_none()
        && row.prompt_template_id.is_none()
        && row.created_at == marker.source_captured_at_ms;
    let marker_identity_matches = marker.kind == CONFLICT_MARKER_KIND
        && marker.version == CONFLICT_MARKER_VERSION
        && marker.source_document_id == expected.source.source_document_id
        && !marker.source_device_id.trim().is_empty()
        && !marker.source_device_label.trim().is_empty()
        && marker.source_captured_at_ms >= 0
        && marker.semantic_fingerprint_sha256 == expected.semantic_fingerprint_sha256
        && marker.attachments_manifest.is_transfer_ready()
        && is_sha256(&marker.attachment_receipt_envelope_fingerprint_sha256);
    if !marker_shape_matches || !marker_identity_matches {
        return Err(collision_error(
            &expected.conflict_document_id,
            "the existing provenance marker does not match the expected conflict origin",
        ));
    }

    let mut existing = snapshot_document(conn, &expected.conflict_document_id)?;
    if existing.title != expected.conflict_title {
        return Err(collision_error(
            &expected.conflict_document_id,
            "the existing conflict title has changed",
        ));
    }
    existing.title = marker.source_title;
    existing.attachments_manifest = marker.attachments_manifest;
    let existing_semantic = existing.conflict_semantic_fingerprint_sha256()?;
    if existing_semantic != expected.semantic_fingerprint_sha256 {
        return Err(collision_error(
            &expected.conflict_document_id,
            "the existing document content does not match the expected conflict version",
        ));
    }

    Ok(())
}

fn insert_origin_marker(
    conn: &Connection,
    validated: &ValidatedConflictSource,
) -> WritingResult<()> {
    let marker = ConflictOriginMarkerV1 {
        kind: CONFLICT_MARKER_KIND.to_string(),
        version: CONFLICT_MARKER_VERSION,
        source_document_id: validated.source.source_document_id.clone(),
        source_device_id: validated.source.source_device_id.clone(),
        source_device_label: validated.source.source_device_label.clone(),
        source_captured_at_ms: validated.source.source_captured_at_ms,
        source_title: validated.envelope.title.clone(),
        semantic_fingerprint_sha256: validated.semantic_fingerprint_sha256.clone(),
        attachment_receipt_envelope_fingerprint_sha256: validated
            .envelope_fingerprint_sha256
            .clone(),
        attachments_manifest: validated.envelope.attachments_manifest.clone(),
    };
    let source_reference_json = serde_json::to_string(&marker).map_err(|error| {
        WritingError::new(
            INVALID_CONFLICT_SOURCE_METADATA,
            format!("failed to serialize conflict source metadata: {error}"),
        )
    })?;

    conn.execute(
        "INSERT INTO writing_provenance_events
           (id, document_id, version_id, range_anchor_json, origin_type,
            operation_type, source_reference_json, model_provider, model_name,
            prompt_template_id, created_at)
         VALUES (?1, ?2, NULL, NULL, 'import', 'other', ?3, NULL, NULL, NULL, ?4)",
        params![
            conflict_marker_id(&validated.conflict_document_id),
            &validated.conflict_document_id,
            source_reference_json,
            validated.source.source_captured_at_ms,
        ],
    )
    .map_err(|error| WritingError::sql("Failed to insert conflict origin marker", error))?;
    Ok(())
}

fn load_origin_marker(
    conn: &Connection,
    marker_id: &str,
) -> WritingResult<Option<(StoredMarkerRow, ConflictOriginMarkerV1)>> {
    let row = conn
        .query_row(
            "SELECT document_id, version_id, range_anchor_json, origin_type,
                    operation_type, source_reference_json, model_provider, model_name,
                    prompt_template_id, created_at
               FROM writing_provenance_events WHERE id = ?1",
            [marker_id],
            |row| {
                Ok(StoredMarkerRow {
                    document_id: row.get(0)?,
                    version_id: row.get(1)?,
                    range_anchor_json: row.get(2)?,
                    origin_type: row.get(3)?,
                    operation_type: row.get(4)?,
                    source_reference_json: row.get(5)?,
                    model_provider: row.get(6)?,
                    model_name: row.get(7)?,
                    prompt_template_id: row.get(8)?,
                    created_at: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|error| WritingError::sql("Failed to read conflict origin marker", error))?;

    let Some(row) = row else {
        return Ok(None);
    };
    let Some(raw) = row.source_reference_json.as_deref() else {
        return Err(WritingError::new(
            CONFLICT_COPY_COLLISION,
            "conflict origin marker has no source metadata",
        ));
    };
    let marker = serde_json::from_str(raw).map_err(|error| {
        WritingError::new(
            CONFLICT_COPY_COLLISION,
            format!("conflict origin marker is invalid: {error}"),
        )
    })?;
    Ok(Some((row, marker)))
}

fn document_exists(conn: &Connection, document_id: &str) -> WritingResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM writing_documents WHERE id = ?1)",
        [document_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
    .map_err(|error| WritingError::sql("Failed to inspect conflict document identity", error))
}

fn provenance_marker_exists(conn: &Connection, marker_id: &str) -> WritingResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM writing_provenance_events WHERE id = ?1)",
        [marker_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
    .map_err(|error| WritingError::sql("Failed to inspect conflict marker identity", error))
}

pub(crate) fn conflict_copy_document_id(
    source_document_id: &str,
    semantic_fingerprint_sha256: &str,
) -> String {
    let mut digest = Sha256::new();
    update_hash_field(&mut digest, b"writing-conflict-copy-v1");
    update_hash_field(&mut digest, source_document_id.as_bytes());
    update_hash_field(&mut digest, semantic_fingerprint_sha256.as_bytes());
    format!("writing-conflict-{:x}", digest.finalize())
}

fn conflict_copy_title(source_title: &str, semantic_fingerprint_sha256: &str) -> String {
    format!(
        "{source_title} (Conflict copy {})",
        &semantic_fingerprint_sha256[..8]
    )
}

fn conflict_marker_id(conflict_document_id: &str) -> String {
    format!("{conflict_document_id}-origin")
}

fn update_hash_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

fn collision_error(document_id: &str, reason: &str) -> WritingError {
    WritingError::new(
        CONFLICT_COPY_COLLISION,
        format!("conflict document {document_id} cannot be reused: {reason}"),
    )
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

// ─────────────────────────── read-only sync notices ───────────────────────────

/// How much of a stored error a notice may show. The receive queue already
/// bounds what it stores; this bounds what leaves the database.
pub(crate) const NOTICE_ERROR_CHARS: usize = 200;

/// One writing-sync notice row for the manuscript list: a document that is a
/// conflict copy, or that still has sync work moving. Identity is the document
/// id — provenance markers and durable sync keys, never a title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WritingSyncNotice {
    pub document_id: String,
    pub conflict_copy: bool,
    pub pending_outbox: bool,
    pub pending_assets: bool,
    pub queued_receive: bool,
    pub last_error: Option<String>,
}

/// Reports, per existing manuscript, whether provenance marks it as a writing
/// sync conflict copy and whether a writing outbox entry, pending assets, or a
/// queued receive is outstanding, plus the error already stored with a queued
/// receive. Only manuscripts with something to report get a row, so a quiet
/// writer gets no notice at all. Read-only: this creates no documents and
/// mutates no sync state.
pub(crate) fn sync_notices(conn: &Connection) -> WritingResult<Vec<WritingSyncNotice>> {
    let mut outbox: HashSet<String> = HashSet::new();
    let mut pending_assets: HashSet<String> = HashSet::new();
    let mut queued: HashMap<String, Option<String>> = HashMap::new();

    let rows = conn
        .prepare(
            "SELECT key, value FROM sync_meta
              WHERE substr(key, 1, length(?1)) = ?1
                 OR substr(key, 1, length(?2)) = ?2
                 OR substr(key, 1, length(?3)) = ?3
              ORDER BY key",
        )
        .and_then(|mut statement| {
            let rows = statement
                .query_map(
                    params![OUTBOX_PREFIX, PENDING_ASSETS_PREFIX, RECEIVE_PREFIX],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .map_err(|error| WritingError::sql("Failed to read writing sync notices", error))?;

    for (key, value) in rows {
        if let Some(document_id) = key.strip_prefix(OUTBOX_PREFIX).filter(|id| !id.is_empty()) {
            outbox.insert(document_id.to_string());
        } else if let Some(document_id) = key
            .strip_prefix(PENDING_ASSETS_PREFIX)
            .filter(|id| !id.is_empty())
        {
            pending_assets.insert(document_id.to_string());
        } else if let Some(document_id) =
            key.strip_prefix(RECEIVE_PREFIX).filter(|id| !id.is_empty())
        {
            queued.insert(document_id.to_string(), stored_notice_error(&value));
        }
    }

    let document_ids = conn
        .prepare("SELECT id FROM writing_documents ORDER BY id")
        .and_then(|mut statement| {
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })
        .map_err(|error| WritingError::sql("Failed to read writing documents", error))?;

    let mut notices = Vec::new();
    for document_id in document_ids {
        let conflict_copy = is_conflict_copy_marker(conn, &document_id)?;
        let has_outbox = outbox.remove(&document_id);
        let has_pending_assets = pending_assets.remove(&document_id);
        let queued_error = queued.remove(&document_id);
        let queued_receive = queued_error.is_some();
        if !(conflict_copy || has_outbox || has_pending_assets || queued_receive) {
            continue;
        }
        notices.push(WritingSyncNotice {
            document_id,
            conflict_copy,
            pending_outbox: has_outbox,
            pending_assets: has_pending_assets,
            queued_receive,
            last_error: queued_error.flatten(),
        });
    }
    Ok(notices)
}

/// Whether the deterministic origin marker of `document_id` is the one this
/// module writes for a conflict copy. Both halves of the identity must match:
/// the marker id and the marker kind. Deliberately lenient in the payload — a
/// passive notice must not fail the whole list over one marker it cannot
/// parse, and showing a copy as a copy is the safe direction.
fn is_conflict_copy_marker(conn: &Connection, document_id: &str) -> WritingResult<bool> {
    let raw: Option<Option<String>> = conn
        .query_row(
            "SELECT source_reference_json FROM writing_provenance_events WHERE id = ?1",
            params![conflict_marker_id(document_id)],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| WritingError::sql("Failed to read conflict origin marker", error))?;
    let Some(raw) = raw.flatten() else {
        return Ok(false);
    };
    let kind = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|marker| {
            marker
                .get("kind")
                .and_then(|kind| kind.as_str())
                .map(str::to_owned)
        });
    Ok(kind.as_deref() == Some(CONFLICT_MARKER_KIND))
}

/// The error already stored with a queue record, bounded for display. A record
/// without one is not an error to show.
fn stored_notice_error(value: &str) -> Option<String> {
    let raw = serde_json::from_str::<serde_json::Value>(value)
        .ok()?
        .get("last_error")?
        .as_str()?
        .trim()
        .chars()
        .take(NOTICE_ERROR_CHARS)
        .collect::<String>();
    (!raw.is_empty()).then_some(raw)
}
