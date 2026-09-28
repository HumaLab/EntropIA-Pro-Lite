//! Inactive offline receiver for validated writing-envelope v1 snapshots.
//!
//! The caller owns the connection and any outer transaction. Every mutation is
//! contained in a savepoint so an error cannot leak a partial aggregate into a
//! later caller commit. Transport, conflict preservation, and file installation
//! remain outside this module.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::repository::{
    is_migration_applied, now_ms, require_schema, WritingError, WritingResult, SCHEMA_NOT_READY,
};
use super::sync_envelope::{
    snapshot_document, AttachmentManifestV1, CorpusCitationV1, WritingEnvelopeV1, ZoteroCitationV1,
    INVALID_SYNC_ENVELOPE,
};

const JOURNAL_MIGRATION_NAME: &str = "0036_writing_journal";
const RECEIVE_SAVEPOINT: &str = "writing_sync_receive";
const MAX_SCOPED_ID_ATTEMPTS: u32 = 1_024;

pub(crate) const ATTACHMENTS_NOT_READY: &str = "writing_attachments_not_ready";
pub(crate) const ATTACHMENT_RECEIPT_MISMATCH: &str = "writing_attachment_receipt_mismatch";
pub(crate) const INVALID_RECEIVE_AUTHORIZATION: &str = "invalid_writing_receive_authorization";

/// The only two write authorities the receiver accepts.
///
/// `Replace` binds a destructive replacement to both the local editor revision
/// and a canonical aggregate fingerprint. The fingerprint is required because
/// title/status/settings changes intentionally do not advance editor revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReceiveAuthorization {
    CreateOnly,
    Replace {
        expected_local_revision: i64,
        expected_local_fingerprint_sha256: String,
    },
}

/// Caller assertion that the future attachment stage installed and verified the
/// files for one exact envelope.
///
/// This receipt deliberately does not inspect the filesystem. The receiver only
/// verifies that the assertion is bound to the envelope it is about to apply;
/// the future file stage owns construction after its real hash/install checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentInstallReceipt {
    envelope_fingerprint_sha256: String,
}

impl AttachmentInstallReceipt {
    pub(crate) fn caller_asserts_installed(envelope_fingerprint_sha256: impl Into<String>) -> Self {
        Self {
            envelope_fingerprint_sha256: envelope_fingerprint_sha256.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReceiveDeferred {
    PendingJournal { entries: i64 },
    MissingCollections { collection_ids: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReceiveConflictReason {
    ExistingDocumentRequiresAuthorization,
    ExpectedStateMismatch,
    DocumentMissing,
}

/// State WS4 needs before it can preserve a loser and authorize a retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ReceiveConflict {
    pub(crate) reason: ReceiveConflictReason,
    pub(crate) existing_local_revision: Option<i64>,
    pub(crate) existing_local_fingerprint_sha256: Option<String>,
    pub(crate) existing_snapshot: Option<Box<WritingEnvelopeV1>>,
}

/// Bounded result surface for the later pull/backfill worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReceiveOutcome {
    Applied {
        document_id: String,
        local_revision: i64,
        local_fingerprint_sha256: String,
        created: bool,
    },
    NoOp {
        document_id: String,
        local_revision: i64,
        local_fingerprint_sha256: String,
    },
    Deferred {
        document_id: String,
        reason: ReceiveDeferred,
    },
    ConflictRequired {
        document_id: String,
        conflict: ReceiveConflict,
    },
}

#[derive(Debug, Clone)]
pub(super) struct ValidatedReceiveEnvelope {
    pub(super) envelope: WritingEnvelopeV1,
    pub(super) envelope_fingerprint_sha256: String,
}

/// Validates one full wire aggregate and its attachment-install proof without
/// opening a transaction or mutating database state.
pub(super) fn validate_envelope_and_attachments(
    document_id: &str,
    envelope: &WritingEnvelopeV1,
    attachments: &AttachmentInstallReceipt,
) -> WritingResult<ValidatedReceiveEnvelope> {
    let canonical = WritingEnvelopeV1::from_json(&envelope.to_canonical_json()?)?;
    if canonical.id != document_id {
        return Err(WritingError::new(
            INVALID_SYNC_ENVELOPE,
            format!(
                "writing envelope id {:?} does not match authoritative document id {:?}",
                canonical.id, document_id
            ),
        ));
    }
    if !canonical.attachments_manifest.is_transfer_ready() {
        return Err(WritingError::new(
            ATTACHMENTS_NOT_READY,
            "writing envelope attachments still require preparation",
        ));
    }

    let envelope_fingerprint_sha256 = canonical.fingerprint_sha256()?;
    if attachments.envelope_fingerprint_sha256 != envelope_fingerprint_sha256 {
        return Err(WritingError::new(
            ATTACHMENT_RECEIPT_MISMATCH,
            "attachment install receipt is not bound to this writing envelope",
        ));
    }

    Ok(ValidatedReceiveEnvelope {
        envelope: canonical,
        envelope_fingerprint_sha256,
    })
}

/// Applies one already-downloaded envelope without activating sync transport.
///
/// The caller supplies the authoritative wire `document_id`, overwrite
/// authority, and a receipt from the future attachment installer. Passing a
/// `Transaction` works through its `Connection` deref; this function nests a
/// savepoint and never commits or rolls back the caller's transaction.
pub(crate) fn receive_envelope(
    conn: &Connection,
    document_id: &str,
    envelope: &WritingEnvelopeV1,
    authorization: &ReceiveAuthorization,
    attachments: &AttachmentInstallReceipt,
) -> WritingResult<ReceiveOutcome> {
    let validated = validate_envelope_and_attachments(document_id, envelope, attachments)?;
    validate_receive_authorization(authorization)?;
    require_receive_schema(conn)?;

    with_receive_savepoint(conn, || {
        receive_inside_savepoint(conn, document_id, &validated.envelope, authorization)
    })
}

pub(super) fn validate_receive_authorization(
    authorization: &ReceiveAuthorization,
) -> WritingResult<()> {
    let ReceiveAuthorization::Replace {
        expected_local_revision,
        expected_local_fingerprint_sha256,
    } = authorization
    else {
        return Ok(());
    };

    if *expected_local_revision < 0 {
        return Err(WritingError::new(
            INVALID_RECEIVE_AUTHORIZATION,
            format!(
                "expected local revision must be non-negative, found {expected_local_revision}"
            ),
        ));
    }
    if !is_sha256(expected_local_fingerprint_sha256) {
        return Err(WritingError::new(
            INVALID_RECEIVE_AUTHORIZATION,
            "expected local fingerprint must be 64 lowercase hexadecimal digits",
        ));
    }
    Ok(())
}

pub(super) fn require_receive_schema(conn: &Connection) -> WritingResult<()> {
    require_schema(conn)?;
    if is_migration_applied(conn, JOURNAL_MIGRATION_NAME)? {
        Ok(())
    } else {
        Err(WritingError::new(
            SCHEMA_NOT_READY,
            "the writing recovery journal schema has not been migrated yet",
        ))
    }
}

pub(super) fn with_receive_savepoint<T>(
    conn: &Connection,
    operation: impl FnOnce() -> WritingResult<T>,
) -> WritingResult<T> {
    conn.execute_batch(&format!("SAVEPOINT {RECEIVE_SAVEPOINT};"))
        .map_err(|error| WritingError::sql("Failed to open writing receive savepoint", error))?;

    match operation() {
        Ok(value) => match conn.execute_batch(&format!("RELEASE {RECEIVE_SAVEPOINT};")) {
            Ok(()) => Ok(value),
            Err(release_error) => {
                rollback_receive_savepoint(conn).map_err(|rollback_error| {
                    WritingError::new(
                        "sql_error",
                        format!(
                            "Failed to release writing receive savepoint: {release_error}; \
                             rollback also failed: {rollback_error}"
                        ),
                    )
                })?;
                Err(WritingError::sql(
                    "Failed to release writing receive savepoint",
                    release_error,
                ))
            }
        },
        Err(error) => {
            rollback_receive_savepoint(conn).map_err(|rollback_error| {
                WritingError::new(
                    "sql_error",
                    format!(
                        "{}; failed to roll back writing receive savepoint: {rollback_error}",
                        error.message
                    ),
                )
            })?;
            Err(error)
        }
    }
}

fn rollback_receive_savepoint(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(&format!(
        "ROLLBACK TO {RECEIVE_SAVEPOINT}; RELEASE {RECEIVE_SAVEPOINT};"
    ))
}

pub(super) enum ReceivePlan {
    Outcome(ReceiveOutcome),
    Apply {
        stored_shape: WritingEnvelopeV1,
        prior_local_revision: Option<i64>,
    },
}

fn receive_inside_savepoint(
    conn: &Connection,
    document_id: &str,
    envelope: &WritingEnvelopeV1,
    authorization: &ReceiveAuthorization,
) -> WritingResult<ReceiveOutcome> {
    let plan = plan_receive_inside_savepoint(conn, document_id, envelope, authorization)?;
    apply_receive_plan(conn, document_id, plan)
}

/// Resolves all overwrite guards and dependencies without writing. A caller
/// may perform another bounded mutation before passing an `Apply` plan back to
/// [`apply_receive_plan`] in the same savepoint.
pub(super) fn plan_receive_inside_savepoint(
    conn: &Connection,
    document_id: &str,
    envelope: &WritingEnvelopeV1,
    authorization: &ReceiveAuthorization,
) -> WritingResult<ReceivePlan> {
    let existing_revision = load_local_revision(conn, document_id)?;
    let mut stored_shape = resolve_child_ids(conn, envelope)?;
    stored_shape.attachments_manifest = AttachmentManifestV1::PreparationRequired;
    let incoming_local_fingerprint = stored_shape.fingerprint_sha256()?;

    let Some(local_revision) = existing_revision else {
        let missing = missing_collection_ids(conn, &stored_shape)?;
        if !missing.is_empty() {
            return Ok(ReceivePlan::Outcome(ReceiveOutcome::Deferred {
                document_id: document_id.to_string(),
                reason: ReceiveDeferred::MissingCollections {
                    collection_ids: missing,
                },
            }));
        }
        if !matches!(authorization, ReceiveAuthorization::CreateOnly) {
            return Ok(ReceivePlan::Outcome(ReceiveOutcome::ConflictRequired {
                document_id: document_id.to_string(),
                conflict: ReceiveConflict {
                    reason: ReceiveConflictReason::DocumentMissing,
                    existing_local_revision: None,
                    existing_local_fingerprint_sha256: None,
                    existing_snapshot: None,
                },
            }));
        }

        return Ok(ReceivePlan::Apply {
            stored_shape,
            prior_local_revision: None,
        });
    };

    let existing = snapshot_document(conn, document_id)?;
    let existing_fingerprint = existing.fingerprint_sha256()?;
    if existing_fingerprint == incoming_local_fingerprint {
        return Ok(ReceivePlan::Outcome(ReceiveOutcome::NoOp {
            document_id: document_id.to_string(),
            local_revision,
            local_fingerprint_sha256: existing_fingerprint,
        }));
    }

    let pending_journal = pending_journal_count(conn, document_id, local_revision)?;
    if pending_journal > 0 {
        return Ok(ReceivePlan::Outcome(ReceiveOutcome::Deferred {
            document_id: document_id.to_string(),
            reason: ReceiveDeferred::PendingJournal {
                entries: pending_journal,
            },
        }));
    }

    let missing = missing_collection_ids(conn, &stored_shape)?;
    if !missing.is_empty() {
        return Ok(ReceivePlan::Outcome(ReceiveOutcome::Deferred {
            document_id: document_id.to_string(),
            reason: ReceiveDeferred::MissingCollections {
                collection_ids: missing,
            },
        }));
    }

    let conflict_reason = match authorization {
        ReceiveAuthorization::CreateOnly => {
            Some(ReceiveConflictReason::ExistingDocumentRequiresAuthorization)
        }
        ReceiveAuthorization::Replace {
            expected_local_revision,
            expected_local_fingerprint_sha256,
        } if *expected_local_revision != local_revision
            || expected_local_fingerprint_sha256 != &existing_fingerprint =>
        {
            Some(ReceiveConflictReason::ExpectedStateMismatch)
        }
        ReceiveAuthorization::Replace { .. } => None,
    };

    if let Some(reason) = conflict_reason {
        return Ok(ReceivePlan::Outcome(ReceiveOutcome::ConflictRequired {
            document_id: document_id.to_string(),
            conflict: ReceiveConflict {
                reason,
                existing_local_revision: Some(local_revision),
                existing_local_fingerprint_sha256: Some(existing_fingerprint),
                existing_snapshot: Some(Box::new(existing)),
            },
        }));
    }

    Ok(ReceivePlan::Apply {
        stored_shape,
        prior_local_revision: Some(local_revision),
    })
}

pub(super) fn apply_receive_plan(
    conn: &Connection,
    document_id: &str,
    plan: ReceivePlan,
) -> WritingResult<ReceiveOutcome> {
    match plan {
        ReceivePlan::Outcome(outcome) => Ok(outcome),
        ReceivePlan::Apply {
            stored_shape,
            prior_local_revision: None,
        } => {
            insert_document_aggregate(conn, &stored_shape)?;
            applied_outcome(conn, document_id, 0, true)
        }
        ReceivePlan::Apply {
            stored_shape,
            prior_local_revision: Some(local_revision),
        } => {
            update_document_aggregate(conn, &stored_shape, local_revision)?;
            applied_outcome(conn, document_id, local_revision + 1, false)
        }
    }
}

fn applied_outcome(
    conn: &Connection,
    document_id: &str,
    local_revision: i64,
    created: bool,
) -> WritingResult<ReceiveOutcome> {
    let stored = snapshot_document(conn, document_id)?;
    let local_fingerprint_sha256 = stored.fingerprint_sha256()?;
    Ok(ReceiveOutcome::Applied {
        document_id: document_id.to_string(),
        local_revision,
        local_fingerprint_sha256,
        created,
    })
}

fn load_local_revision(conn: &Connection, document_id: &str) -> WritingResult<Option<i64>> {
    conn.query_row(
        "SELECT revision FROM writing_documents WHERE id = ?1",
        [document_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| WritingError::sql("Failed to read local writing revision", error))
}

pub(super) fn pending_journal_count(
    conn: &Connection,
    document_id: &str,
    local_revision: i64,
) -> WritingResult<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM writing_journal
          WHERE document_id = ?1 AND base_revision >= ?2",
        params![document_id, local_revision],
        |row| row.get(0),
    )
    .map_err(|error| WritingError::sql("Failed to inspect the writing journal", error))
}

fn missing_collection_ids(
    conn: &Connection,
    envelope: &WritingEnvelopeV1,
) -> WritingResult<Vec<String>> {
    let mut statement = conn
        .prepare("SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?1)")
        .map_err(|error| {
            WritingError::sql("Failed to prepare collection dependency check", error)
        })?;
    let mut missing = Vec::new();
    for association in &envelope.collection_associations {
        let exists: i64 = statement
            .query_row([&association.collection_id], |row| row.get(0))
            .map_err(|error| WritingError::sql("Failed to check collection dependency", error))?;
        if exists == 0 {
            missing.push(association.collection_id.clone());
        }
    }
    missing.sort();
    Ok(missing)
}

fn insert_document_aggregate(conn: &Connection, envelope: &WritingEnvelopeV1) -> WritingResult<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO writing_documents
           (id, title, document_type, status, schema_version, current_content_json,
            revision, plain_text_cache, citation_style_id, citation_locale,
            bibliography_enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?8, ?9, ?10, ?10)",
        params![
            &envelope.id,
            &envelope.title,
            &envelope.document_type,
            &envelope.status,
            envelope.schema_version,
            json_text("content_json", &envelope.content_json)?,
            &envelope.settings.citation_style_id,
            &envelope.settings.citation_locale,
            envelope.settings.bibliography_enabled as i64,
            now,
        ],
    )
    .map_err(|error| WritingError::sql("Failed to insert received writing document", error))?;
    replace_children(conn, envelope, now)
}

fn update_document_aggregate(
    conn: &Connection,
    envelope: &WritingEnvelopeV1,
    local_revision: i64,
) -> WritingResult<()> {
    let now = now_ms();
    let changed = conn
        .execute(
            "UPDATE writing_documents
                SET title = ?1,
                    document_type = ?2,
                    status = ?3,
                    schema_version = ?4,
                    current_content_json = ?5,
                    revision = revision + 1,
                    plain_text_cache = NULL,
                    citation_style_id = ?6,
                    citation_locale = ?7,
                    bibliography_enabled = ?8,
                    updated_at = ?9
              WHERE id = ?10 AND revision = ?11",
            params![
                &envelope.title,
                &envelope.document_type,
                &envelope.status,
                envelope.schema_version,
                json_text("content_json", &envelope.content_json)?,
                &envelope.settings.citation_style_id,
                &envelope.settings.citation_locale,
                envelope.settings.bibliography_enabled as i64,
                now,
                &envelope.id,
                local_revision,
            ],
        )
        .map_err(|error| WritingError::sql("Failed to update received writing document", error))?;
    if changed != 1 {
        return Err(WritingError::new(
            "writing_receive_state_changed",
            format!(
                "document {} changed while its received snapshot was being applied",
                envelope.id
            ),
        ));
    }
    replace_children(conn, envelope, now)
}

fn replace_children(
    conn: &Connection,
    envelope: &WritingEnvelopeV1,
    now: i64,
) -> WritingResult<()> {
    conn.execute(
        "DELETE FROM writing_document_collections WHERE document_id = ?1",
        [&envelope.id],
    )
    .map_err(|error| WritingError::sql("Failed to clear collection associations", error))?;
    conn.execute(
        "DELETE FROM writing_document_citations WHERE document_id = ?1",
        [&envelope.id],
    )
    .map_err(|error| WritingError::sql("Failed to clear corpus citation projection", error))?;
    conn.execute(
        "DELETE FROM writing_zotero_citations WHERE document_id = ?1",
        [&envelope.id],
    )
    .map_err(|error| WritingError::sql("Failed to clear Zotero citation projection", error))?;

    for association in &envelope.collection_associations {
        conn.execute(
            "INSERT INTO writing_document_collections
               (document_id, collection_id, is_primary, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                &envelope.id,
                &association.collection_id,
                association.is_primary as i64,
                now,
            ],
        )
        .map_err(|error| WritingError::sql("Failed to insert collection association", error))?;
    }

    for citation in &envelope.citation_projections.corpus {
        insert_corpus_citation(conn, &envelope.id, citation, now)?;
    }
    for citation in &envelope.citation_projections.zotero {
        insert_zotero_citation(conn, &envelope.id, citation, now)?;
    }
    Ok(())
}

fn insert_corpus_citation(
    conn: &Connection,
    document_id: &str,
    citation: &CorpusCitationV1,
    now: i64,
) -> WritingResult<()> {
    conn.execute(
        "INSERT INTO writing_document_citations
           (id, document_id, citation_node_id, collection_id, item_id, asset_id,
            page_number, start_char, end_char, source_region_json, quoted_text,
            source_text_hash, locator_json, metadata_snapshot_json, integrity_status,
            created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?16)",
        params![
            &citation.id,
            document_id,
            &citation.citation_node_id,
            &citation.collection_id,
            &citation.item_id,
            &citation.asset_id,
            citation.page_number,
            citation.start_char,
            citation.end_char,
            optional_json_text("source_region_json", citation.source_region_json.as_ref())?,
            &citation.quoted_text,
            &citation.source_text_hash,
            optional_json_text("locator_json", citation.locator_json.as_ref())?,
            json_text("metadata_snapshot_json", &citation.metadata_snapshot_json)?,
            &citation.integrity_status,
            now,
        ],
    )
    .map_err(|error| WritingError::sql("Failed to insert corpus citation", error))?;
    Ok(())
}

fn insert_zotero_citation(
    conn: &Connection,
    document_id: &str,
    citation: &ZoteroCitationV1,
    now: i64,
) -> WritingResult<()> {
    conn.execute(
        "INSERT INTO writing_zotero_citations
           (id, document_id, citation_node_id, citation_cluster_id, item_position,
            source_origin, source_instance_id, library_type, library_id, item_key,
            item_version, locator_type, locator, prefix, suffix, suppress_author,
            author_only, item_csl_json_snapshot, integrity_status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19, ?20, ?20)",
        params![
            &citation.id,
            document_id,
            &citation.citation_node_id,
            &citation.citation_cluster_id,
            citation.item_position,
            &citation.source_origin,
            &citation.source_instance_id,
            &citation.library_type,
            &citation.library_id,
            &citation.item_key,
            citation.item_version,
            &citation.locator_type,
            &citation.locator,
            &citation.prefix,
            &citation.suffix,
            citation.suppress_author as i64,
            citation.author_only as i64,
            json_text("item_csl_json_snapshot", &citation.item_csl_json_snapshot,)?,
            &citation.integrity_status,
            now,
        ],
    )
    .map_err(|error| WritingError::sql("Failed to insert Zotero citation", error))?;
    Ok(())
}

fn resolve_child_ids(
    conn: &Connection,
    envelope: &WritingEnvelopeV1,
) -> WritingResult<WritingEnvelopeV1> {
    let mut resolved = envelope.clone();
    let mut corpus_ids = HashSet::new();
    for citation in &mut resolved.citation_projections.corpus {
        citation.id = available_child_id(
            conn,
            ChildTable::Corpus,
            &resolved.id,
            &citation.id,
            &citation.citation_node_id,
            &mut corpus_ids,
        )?;
    }

    let mut zotero_ids = HashSet::new();
    for citation in &mut resolved.citation_projections.zotero {
        let semantic_identity = format!(
            "{}\u{0}{}",
            citation.citation_cluster_id, citation.item_position
        );
        citation.id = available_child_id(
            conn,
            ChildTable::Zotero,
            &resolved.id,
            &citation.id,
            &semantic_identity,
            &mut zotero_ids,
        )?;
    }
    Ok(resolved)
}

#[derive(Debug, Clone, Copy)]
enum ChildTable {
    Corpus,
    Zotero,
}

impl ChildTable {
    fn label(self) -> &'static str {
        match self {
            Self::Corpus => "corpus",
            Self::Zotero => "zotero",
        }
    }

    fn owner(self, conn: &Connection, id: &str) -> Result<Option<String>, rusqlite::Error> {
        match self {
            Self::Corpus => conn
                .query_row(
                    "SELECT document_id FROM writing_document_citations WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional(),
            Self::Zotero => conn
                .query_row(
                    "SELECT document_id FROM writing_zotero_citations WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional(),
        }
    }
}

fn available_child_id(
    conn: &Connection,
    table: ChildTable,
    document_id: &str,
    incoming_id: &str,
    semantic_identity: &str,
    selected: &mut HashSet<String>,
) -> WritingResult<String> {
    if child_id_is_available(conn, table, document_id, incoming_id, selected)? {
        selected.insert(incoming_id.to_string());
        return Ok(incoming_id.to_string());
    }

    for salt in 0..MAX_SCOPED_ID_ATTEMPTS {
        let candidate = scoped_child_id(table.label(), document_id, semantic_identity, salt);
        if child_id_is_available(conn, table, document_id, &candidate, selected)? {
            selected.insert(candidate.clone());
            return Ok(candidate);
        }
    }

    Err(WritingError::new(
        "writing_child_identity_exhausted",
        format!(
            "could not allocate a document-scoped {} citation id for document {}",
            table.label(),
            document_id
        ),
    ))
}

fn child_id_is_available(
    conn: &Connection,
    table: ChildTable,
    document_id: &str,
    candidate: &str,
    selected: &HashSet<String>,
) -> WritingResult<bool> {
    if selected.contains(candidate) {
        return Ok(false);
    }
    let owner = table
        .owner(conn, candidate)
        .map_err(|error| WritingError::sql("Failed to resolve citation identity", error))?;
    Ok(owner
        .as_deref()
        .map(|owner| owner == document_id)
        .unwrap_or(true))
}

fn scoped_child_id(kind: &str, document_id: &str, semantic_identity: &str, salt: u32) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(document_id.as_bytes());
    digest.update([0]);
    digest.update(semantic_identity.as_bytes());
    digest.update([0]);
    digest.update(salt.to_le_bytes());
    format!("writing-sync-{kind}-{:x}", digest.finalize())
}

fn json_text(field: &str, value: &Value) -> WritingResult<String> {
    serde_json::to_string(value).map_err(|error| {
        WritingError::new(
            INVALID_SYNC_ENVELOPE,
            format!("failed to serialize {field}: {error}"),
        )
    })
}

fn optional_json_text(field: &str, value: Option<&Value>) -> WritingResult<Option<String>> {
    value.map(|value| json_text(field, value)).transpose()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
