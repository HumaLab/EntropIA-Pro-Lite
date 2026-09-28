//! Inactive v1 snapshot adapter for the negotiated `writing_envelopes` row.
//!
//! This module only builds and validates a coherent aggregate from a transaction
//! supplied by its caller. It is not connected to capture, transport, or apply.

use std::collections::HashSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::repository::{
    require_schema, WritingError, WritingResult, DOCUMENT_NOT_FOUND, DOCUMENT_STATUSES,
};

pub(crate) const WRITING_ENVELOPE_VERSION: u32 = 1;
pub(crate) const INVALID_SYNC_ENVELOPE: &str = "invalid_sync_envelope";
pub(crate) const UNSUPPORTED_SYNC_ENVELOPE_VERSION: &str = "unsupported_sync_envelope_version";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WritingEnvelopeV1 {
    pub(crate) id: String,
    pub(crate) envelope_version: u32,
    pub(crate) content_json: Value,
    pub(crate) title: String,
    #[serde(rename = "type")]
    pub(crate) document_type: String,
    pub(crate) status: String,
    pub(crate) schema_version: i64,
    pub(crate) settings: DocumentSettingsV1,
    pub(crate) collection_associations: Vec<CollectionAssociationV1>,
    pub(crate) citation_projections: CitationProjectionsV1,
    pub(crate) attachments_manifest: AttachmentManifestV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DocumentSettingsV1 {
    pub(crate) citation_style_id: Option<String>,
    pub(crate) citation_locale: Option<String>,
    pub(crate) bibliography_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollectionAssociationV1 {
    pub(crate) collection_id: String,
    pub(crate) is_primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CitationProjectionsV1 {
    pub(crate) corpus: Vec<CorpusCitationV1>,
    pub(crate) zotero: Vec<ZoteroCitationV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusCitationV1 {
    pub(crate) id: String,
    pub(crate) citation_node_id: String,
    pub(crate) collection_id: Option<String>,
    pub(crate) item_id: Option<String>,
    pub(crate) asset_id: Option<String>,
    pub(crate) page_number: Option<i64>,
    pub(crate) start_char: Option<i64>,
    pub(crate) end_char: Option<i64>,
    pub(crate) source_region_json: Option<Value>,
    pub(crate) quoted_text: Option<String>,
    pub(crate) source_text_hash: Option<String>,
    pub(crate) locator_json: Option<Value>,
    pub(crate) metadata_snapshot_json: Value,
    pub(crate) integrity_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ZoteroCitationV1 {
    pub(crate) id: String,
    pub(crate) citation_node_id: String,
    pub(crate) citation_cluster_id: String,
    pub(crate) item_position: i64,
    pub(crate) source_origin: String,
    pub(crate) source_instance_id: Option<String>,
    pub(crate) library_type: String,
    pub(crate) library_id: String,
    pub(crate) item_key: String,
    pub(crate) item_version: Option<i64>,
    pub(crate) locator_type: Option<String>,
    pub(crate) locator: Option<String>,
    pub(crate) prefix: Option<String>,
    pub(crate) suffix: Option<String>,
    pub(crate) suppress_author: bool,
    pub(crate) author_only: bool,
    pub(crate) item_csl_json_snapshot: Value,
    pub(crate) integrity_status: String,
}

/// File discovery and hash verification belong to the later attachment slice.
/// A database-only snapshot is therefore never represented as an empty,
/// transfer-ready manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AttachmentManifestV1 {
    PreparationRequired,
    Validated { files: Vec<AttachmentFileV1> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachmentFileV1 {
    pub(crate) sha256: String,
    pub(crate) rel_path: String,
    pub(crate) size: u64,
    pub(crate) media_type: String,
}

impl AttachmentManifestV1 {
    pub(crate) fn is_transfer_ready(&self) -> bool {
        matches!(self, Self::Validated { .. })
    }
}

impl WritingEnvelopeV1 {
    pub(crate) fn from_json(raw: &str) -> WritingResult<Self> {
        let mut envelope: Self = serde_json::from_str(raw).map_err(|error| {
            invalid_envelope(format!("failed to deserialize writing envelope: {error}"))
        })?;
        envelope.canonicalize();
        envelope.validate()?;
        Ok(envelope)
    }

    pub(crate) fn to_canonical_json(&self) -> WritingResult<String> {
        let mut envelope = self.clone();
        envelope.canonicalize();
        envelope.validate()?;
        serde_json::to_string(&envelope).map_err(|error| {
            invalid_envelope(format!("failed to serialize writing envelope: {error}"))
        })
    }

    pub(crate) fn fingerprint_sha256(&self) -> WritingResult<String> {
        let canonical = self.to_canonical_json()?;
        Ok(format!("{:x}", Sha256::digest(canonical.as_bytes())))
    }

    /// Identifies conflict content independently of database-scoped row ids.
    ///
    /// This fingerprint is only for deterministic conflict-copy identity. The
    /// receiver's expected-state guard must keep using [`Self::fingerprint_sha256`]
    /// so storage changes cannot weaken overwrite authorization.
    pub(crate) fn conflict_semantic_fingerprint_sha256(&self) -> WritingResult<String> {
        let mut semantic = Self::from_json(&self.to_canonical_json()?)?;
        if !semantic.attachments_manifest.is_transfer_ready() {
            return Err(invalid_envelope(
                "conflict identity requires a validated attachment manifest",
            ));
        }

        semantic.id = "writing-conflict-semantic-document".to_string();
        for (index, citation) in semantic.citation_projections.corpus.iter_mut().enumerate() {
            citation.id = format!("writing-conflict-semantic-corpus-{index}");
        }
        for (index, citation) in semantic.citation_projections.zotero.iter_mut().enumerate() {
            citation.id = format!("writing-conflict-semantic-zotero-{index}");
        }

        semantic.fingerprint_sha256()
    }

    pub(crate) fn validate(&self) -> WritingResult<()> {
        if self.envelope_version != WRITING_ENVELOPE_VERSION {
            return Err(WritingError::new(
                UNSUPPORTED_SYNC_ENVELOPE_VERSION,
                format!(
                    "unsupported writing envelope version {}; expected {}",
                    self.envelope_version, WRITING_ENVELOPE_VERSION
                ),
            ));
        }
        require_non_empty("id", &self.id)?;
        require_non_empty("type", &self.document_type)?;
        if !DOCUMENT_STATUSES.contains(&self.status.as_str()) {
            return Err(invalid_envelope(format!(
                "unsupported writing document status {:?}",
                self.status
            )));
        }
        if !self.content_json.is_object() {
            return Err(invalid_envelope("content_json must be a JSON object"));
        }
        if self.schema_version <= 0 {
            return Err(invalid_envelope(format!(
                "schema_version must be positive, found {}",
                self.schema_version
            )));
        }
        if self
            .content_json
            .get("schemaVersion")
            .and_then(Value::as_i64)
            != Some(self.schema_version)
        {
            return Err(invalid_envelope(format!(
                "content_json.schemaVersion must equal schema_version {}",
                self.schema_version
            )));
        }
        if !self
            .content_json
            .get("doc")
            .is_some_and(serde_json::Value::is_object)
        {
            return Err(invalid_envelope("content_json.doc must be a JSON object"));
        }

        let mut collection_ids = HashSet::new();
        for association in &self.collection_associations {
            require_non_empty(
                "collection_associations[].collection_id",
                &association.collection_id,
            )?;
            if !collection_ids.insert(&association.collection_id) {
                return Err(invalid_envelope(format!(
                    "duplicate collection association {:?}",
                    association.collection_id
                )));
            }
        }

        let mut corpus_ids = HashSet::new();
        let mut corpus_nodes = HashSet::new();
        for citation in &self.citation_projections.corpus {
            require_non_empty("citation_projections.corpus[].id", &citation.id)?;
            require_non_empty(
                "citation_projections.corpus[].citation_node_id",
                &citation.citation_node_id,
            )?;
            if !corpus_ids.insert(&citation.id) || !corpus_nodes.insert(&citation.citation_node_id)
            {
                return Err(invalid_envelope("duplicate corpus citation identity"));
            }
            if !matches!(
                citation.integrity_status.as_str(),
                "valid" | "source_modified" | "source_missing" | "unverifiable"
            ) {
                return Err(invalid_envelope(format!(
                    "unsupported corpus citation integrity status {:?}",
                    citation.integrity_status
                )));
            }
        }

        let mut zotero_ids = HashSet::new();
        let mut zotero_positions = HashSet::new();
        for citation in &self.citation_projections.zotero {
            require_non_empty("citation_projections.zotero[].id", &citation.id)?;
            require_non_empty(
                "citation_projections.zotero[].citation_node_id",
                &citation.citation_node_id,
            )?;
            require_non_empty(
                "citation_projections.zotero[].citation_cluster_id",
                &citation.citation_cluster_id,
            )?;
            require_non_empty(
                "citation_projections.zotero[].library_type",
                &citation.library_type,
            )?;
            require_non_empty(
                "citation_projections.zotero[].library_id",
                &citation.library_id,
            )?;
            require_non_empty("citation_projections.zotero[].item_key", &citation.item_key)?;
            if !zotero_ids.insert(&citation.id)
                || !zotero_positions.insert((&citation.citation_cluster_id, citation.item_position))
            {
                return Err(invalid_envelope("duplicate Zotero citation identity"));
            }
            if !matches!(citation.source_origin.as_str(), "local" | "web") {
                return Err(invalid_envelope(format!(
                    "unsupported Zotero source origin {:?}",
                    citation.source_origin
                )));
            }
            if !matches!(
                citation.integrity_status.as_str(),
                "valid" | "zotero_unavailable" | "item_missing" | "item_modified"
            ) {
                return Err(invalid_envelope(format!(
                    "unsupported Zotero citation integrity status {:?}",
                    citation.integrity_status
                )));
            }
        }

        if let AttachmentManifestV1::Validated { files } = &self.attachments_manifest {
            let mut paths = HashSet::new();
            for file in files {
                if file.sha256.len() != 64
                    || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(invalid_envelope("attachment sha256 must be 64 hex digits"));
                }
                if !is_portable_relative_path(&file.rel_path) {
                    return Err(invalid_envelope(format!(
                        "attachment path {:?} is not a safe portable relative path",
                        file.rel_path
                    )));
                }
                require_non_empty("attachments_manifest.files[].media_type", &file.media_type)?;
                if !paths.insert(&file.rel_path) {
                    return Err(invalid_envelope(format!(
                        "duplicate attachment path {:?}",
                        file.rel_path
                    )));
                }
            }
        }

        Ok(())
    }

    fn canonicalize(&mut self) {
        canonicalize_json(&mut self.content_json);
        self.collection_associations
            .sort_by(|left, right| left.collection_id.cmp(&right.collection_id));
        self.citation_projections.corpus.sort_by(|left, right| {
            (&left.citation_node_id, &left.id).cmp(&(&right.citation_node_id, &right.id))
        });
        for citation in &mut self.citation_projections.corpus {
            if let Some(region) = &mut citation.source_region_json {
                canonicalize_json(region);
            }
            if let Some(locator) = &mut citation.locator_json {
                canonicalize_json(locator);
            }
            canonicalize_json(&mut citation.metadata_snapshot_json);
        }
        self.citation_projections.zotero.sort_by(|left, right| {
            (&left.citation_cluster_id, left.item_position, &left.id).cmp(&(
                &right.citation_cluster_id,
                right.item_position,
                &right.id,
            ))
        });
        for citation in &mut self.citation_projections.zotero {
            canonicalize_json(&mut citation.item_csl_json_snapshot);
        }
        if let AttachmentManifestV1::Validated { files } = &mut self.attachments_manifest {
            files.sort_by(|left, right| {
                (&left.rel_path, &left.sha256).cmp(&(&right.rel_path, &right.sha256))
            });
        }
    }
}

/// Captures one coherent document aggregate from the caller's transaction.
/// The returned manifest deliberately requires later file preparation.
pub(crate) fn snapshot_document(
    conn: &Connection,
    document_id: &str,
) -> WritingResult<WritingEnvelopeV1> {
    require_schema(conn)?;
    let document = load_document_source(conn, document_id)?;
    let mut envelope = WritingEnvelopeV1 {
        id: document.id,
        envelope_version: WRITING_ENVELOPE_VERSION,
        content_json: parse_json(
            "writing_documents.current_content_json",
            &document.content_json,
        )?,
        title: document.title,
        document_type: document.document_type,
        status: document.status,
        schema_version: document.schema_version,
        settings: DocumentSettingsV1 {
            citation_style_id: document.citation_style_id,
            citation_locale: document.citation_locale,
            bibliography_enabled: bool_from_integer(
                "writing_documents.bibliography_enabled",
                document.bibliography_enabled,
            )?,
        },
        collection_associations: load_collection_associations(conn, document_id)?,
        citation_projections: CitationProjectionsV1 {
            corpus: load_corpus_citations(conn, document_id)?,
            zotero: load_zotero_citations(conn, document_id)?,
        },
        attachments_manifest: AttachmentManifestV1::PreparationRequired,
    };
    envelope.canonicalize();
    envelope.validate()?;
    Ok(envelope)
}

struct DocumentSource {
    id: String,
    title: String,
    document_type: String,
    status: String,
    schema_version: i64,
    content_json: String,
    citation_style_id: Option<String>,
    citation_locale: Option<String>,
    bibliography_enabled: i64,
}

fn load_document_source(conn: &Connection, document_id: &str) -> WritingResult<DocumentSource> {
    conn.query_row(
        "SELECT id, title, document_type, status, schema_version, current_content_json,
                citation_style_id, citation_locale, bibliography_enabled
           FROM writing_documents WHERE id = ?1",
        [document_id],
        |row| {
            Ok(DocumentSource {
                id: row.get(0)?,
                title: row.get(1)?,
                document_type: row.get(2)?,
                status: row.get(3)?,
                schema_version: row.get(4)?,
                content_json: row.get(5)?,
                citation_style_id: row.get(6)?,
                citation_locale: row.get(7)?,
                bibliography_enabled: row.get(8)?,
            })
        },
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => WritingError::new(
            DOCUMENT_NOT_FOUND,
            format!("no document with id {document_id}"),
        ),
        other => WritingError::sql("Failed to read writing snapshot document", other),
    })
}

fn load_collection_associations(
    conn: &Connection,
    document_id: &str,
) -> WritingResult<Vec<CollectionAssociationV1>> {
    let mut statement = conn
        .prepare(
            "SELECT collection_id, is_primary
               FROM writing_document_collections
              WHERE document_id = ?1
              ORDER BY collection_id ASC",
        )
        .map_err(|error| WritingError::sql("Failed to prepare collection snapshot", error))?;
    let rows = statement
        .query_map([document_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| WritingError::sql("Failed to read collection snapshot", error))?;

    let mut associations = Vec::new();
    for row in rows {
        let (collection_id, is_primary) =
            row.map_err(|error| WritingError::sql("Failed to read collection snapshot", error))?;
        associations.push(CollectionAssociationV1 {
            collection_id,
            is_primary: bool_from_integer("writing_document_collections.is_primary", is_primary)?,
        });
    }
    Ok(associations)
}

struct CorpusCitationSource {
    id: String,
    citation_node_id: String,
    collection_id: Option<String>,
    item_id: Option<String>,
    asset_id: Option<String>,
    page_number: Option<i64>,
    start_char: Option<i64>,
    end_char: Option<i64>,
    source_region_json: Option<String>,
    quoted_text: Option<String>,
    source_text_hash: Option<String>,
    locator_json: Option<String>,
    metadata_snapshot_json: String,
    integrity_status: String,
}

fn load_corpus_citations(
    conn: &Connection,
    document_id: &str,
) -> WritingResult<Vec<CorpusCitationV1>> {
    let mut statement = conn
        .prepare(
            "SELECT id, citation_node_id, collection_id, item_id, asset_id, page_number,
                    start_char, end_char, source_region_json, quoted_text, source_text_hash,
                    locator_json, metadata_snapshot_json, integrity_status
               FROM writing_document_citations
              WHERE document_id = ?1
              ORDER BY citation_node_id ASC, id ASC",
        )
        .map_err(|error| WritingError::sql("Failed to prepare corpus citation snapshot", error))?;
    let rows = statement
        .query_map([document_id], |row| {
            Ok(CorpusCitationSource {
                id: row.get(0)?,
                citation_node_id: row.get(1)?,
                collection_id: row.get(2)?,
                item_id: row.get(3)?,
                asset_id: row.get(4)?,
                page_number: row.get(5)?,
                start_char: row.get(6)?,
                end_char: row.get(7)?,
                source_region_json: row.get(8)?,
                quoted_text: row.get(9)?,
                source_text_hash: row.get(10)?,
                locator_json: row.get(11)?,
                metadata_snapshot_json: row.get(12)?,
                integrity_status: row.get(13)?,
            })
        })
        .map_err(|error| WritingError::sql("Failed to read corpus citation snapshot", error))?;

    let mut citations = Vec::new();
    for row in rows {
        let source = row
            .map_err(|error| WritingError::sql("Failed to read corpus citation snapshot", error))?;
        citations.push(CorpusCitationV1 {
            id: source.id,
            citation_node_id: source.citation_node_id,
            collection_id: source.collection_id,
            item_id: source.item_id,
            asset_id: source.asset_id,
            page_number: source.page_number,
            start_char: source.start_char,
            end_char: source.end_char,
            source_region_json: parse_optional_json(
                "writing_document_citations.source_region_json",
                source.source_region_json,
            )?,
            quoted_text: source.quoted_text,
            source_text_hash: source.source_text_hash,
            locator_json: parse_optional_json(
                "writing_document_citations.locator_json",
                source.locator_json,
            )?,
            metadata_snapshot_json: parse_json(
                "writing_document_citations.metadata_snapshot_json",
                &source.metadata_snapshot_json,
            )?,
            integrity_status: source.integrity_status,
        });
    }
    Ok(citations)
}

struct ZoteroCitationSource {
    id: String,
    citation_node_id: String,
    citation_cluster_id: String,
    item_position: i64,
    source_origin: String,
    source_instance_id: Option<String>,
    library_type: String,
    library_id: String,
    item_key: String,
    item_version: Option<i64>,
    locator_type: Option<String>,
    locator: Option<String>,
    prefix: Option<String>,
    suffix: Option<String>,
    suppress_author: i64,
    author_only: i64,
    item_csl_json_snapshot: String,
    integrity_status: String,
}

fn load_zotero_citations(
    conn: &Connection,
    document_id: &str,
) -> WritingResult<Vec<ZoteroCitationV1>> {
    let mut statement = conn
        .prepare(
            "SELECT id, citation_node_id, citation_cluster_id, item_position, source_origin,
                    source_instance_id, library_type, library_id, item_key, item_version,
                    locator_type, locator, prefix, suffix, suppress_author, author_only,
                    item_csl_json_snapshot, integrity_status
               FROM writing_zotero_citations
              WHERE document_id = ?1
              ORDER BY citation_cluster_id ASC, item_position ASC, id ASC",
        )
        .map_err(|error| WritingError::sql("Failed to prepare Zotero citation snapshot", error))?;
    let rows = statement
        .query_map([document_id], |row| {
            Ok(ZoteroCitationSource {
                id: row.get(0)?,
                citation_node_id: row.get(1)?,
                citation_cluster_id: row.get(2)?,
                item_position: row.get(3)?,
                source_origin: row.get(4)?,
                source_instance_id: row.get(5)?,
                library_type: row.get(6)?,
                library_id: row.get(7)?,
                item_key: row.get(8)?,
                item_version: row.get(9)?,
                locator_type: row.get(10)?,
                locator: row.get(11)?,
                prefix: row.get(12)?,
                suffix: row.get(13)?,
                suppress_author: row.get(14)?,
                author_only: row.get(15)?,
                item_csl_json_snapshot: row.get(16)?,
                integrity_status: row.get(17)?,
            })
        })
        .map_err(|error| WritingError::sql("Failed to read Zotero citation snapshot", error))?;

    let mut citations = Vec::new();
    for row in rows {
        let source = row
            .map_err(|error| WritingError::sql("Failed to read Zotero citation snapshot", error))?;
        citations.push(ZoteroCitationV1 {
            id: source.id,
            citation_node_id: source.citation_node_id,
            citation_cluster_id: source.citation_cluster_id,
            item_position: source.item_position,
            source_origin: source.source_origin,
            source_instance_id: source.source_instance_id,
            library_type: source.library_type,
            library_id: source.library_id,
            item_key: source.item_key,
            item_version: source.item_version,
            locator_type: source.locator_type,
            locator: source.locator,
            prefix: source.prefix,
            suffix: source.suffix,
            suppress_author: bool_from_integer(
                "writing_zotero_citations.suppress_author",
                source.suppress_author,
            )?,
            author_only: bool_from_integer(
                "writing_zotero_citations.author_only",
                source.author_only,
            )?,
            item_csl_json_snapshot: parse_json(
                "writing_zotero_citations.item_csl_json_snapshot",
                &source.item_csl_json_snapshot,
            )?,
            integrity_status: source.integrity_status,
        });
    }
    Ok(citations)
}

fn parse_optional_json(field: &str, raw: Option<String>) -> WritingResult<Option<Value>> {
    raw.map(|value| parse_json(field, &value)).transpose()
}

fn parse_json(field: &str, raw: &str) -> WritingResult<Value> {
    serde_json::from_str(raw)
        .map_err(|error| invalid_envelope(format!("{field} is not valid JSON: {error}")))
}

fn bool_from_integer(field: &str, value: i64) -> WritingResult<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid_envelope(format!(
            "{field} must be stored as 0 or 1, found {value}"
        ))),
    }
}

fn require_non_empty(field: &str, value: &str) -> WritingResult<()> {
    if value.trim().is_empty() {
        Err(invalid_envelope(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn invalid_envelope(message: impl Into<String>) -> WritingError {
    WritingError::new(INVALID_SYNC_ENVELOPE, message)
}

fn canonicalize_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = std::mem::take(object).into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            for (key, mut child) in entries {
                canonicalize_json(&mut child);
                object.insert(key, child);
            }
        }
        Value::Array(values) => {
            for value in values {
                canonicalize_json(value);
            }
        }
        _ => {}
    }
}

pub(super) fn is_portable_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}
