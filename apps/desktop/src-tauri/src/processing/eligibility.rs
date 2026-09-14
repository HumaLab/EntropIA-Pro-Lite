//! Shared eligibility predicates for batch admission (plan-lote.md §3.2–3.3).
//!
//! One predicate per operation decides whether an asset needs work, and why.
//! The batch classifier (Unidad 2), the repair sweep, and the manual
//! entry points must all ask the same question — that is what keeps a
//! finished asset from being recomputed by a different path. The repair
//! scheduler and the manual commands are migrated onto these predicates in
//! Unidad 4; until then this module is read-only and changes nothing.
//!
//! Conventions:
//! - An existing canonical extraction row means OCR is done, even when its
//!   text is empty: "no text recognized" is not "never executed". A native
//!   extraction (`method = 'native'`) satisfies the text need without
//!   rasterizing a PDF that already has text.
//! - A PDF root with page children is never OCRed itself: the pages are the
//!   work units. A partial import (fewer pages than the document declares) is
//!   not a complete document, so coverage is checked, not assumed.
//! - Embeddings are fresh only when the vector, the full chunk set, the
//!   source revision, and the contract all agree. `summarize`-style metadata
//!   counts are not freshness.

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::nlp::embeddings::{
    CANONICAL_EMBEDDING_CONTRACT_V1, CANONICAL_EMBEDDING_DIMENSIONS, CANONICAL_EMBEDDING_MODEL,
    RAG_CHUNKING_CONTRACT_V1,
};

/// Character window the RAG chunker slides over each source text. Mirrors
/// `RAG_CHUNK_MAX_CHARS` / `RAG_CHUNK_OVERLAP_CHARS` in `nlp::embeddings`;
/// `expected_chunk_count` is pinned against `plan_rag_chunks` by test.
const RAG_CHUNK_MAX_CHARS: usize = 800;
const RAG_CHUNK_OVERLAP_CHARS: usize = 100;

/// How many chunks `plan_rag_chunks` derives from a text of `char_count`
/// Unicode scalars. Empty text yields zero chunks and is never admitted.
pub fn expected_chunk_count(char_count: usize) -> usize {
    if char_count == 0 {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    loop {
        let end = (start + RAG_CHUNK_MAX_CHARS).min(char_count);
        count += 1;
        if end == char_count {
            break;
        }
        start = end - RAG_CHUNK_OVERLAP_CHARS;
    }
    count
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// Effective embedding contract admitted tasks are pinned to. A provider or
/// model change alters this hash, which blocks the remaining tasks with
/// `configuration_changed` instead of silently mixing contracts (Unidad 3+).
pub fn current_embedding_contract_hash() -> String {
    sha256_hex(
        format!(
            "{}|{}|{}|{}",
            CANONICAL_EMBEDDING_MODEL,
            CANONICAL_EMBEDDING_CONTRACT_V1,
            CANONICAL_EMBEDDING_DIMENSIONS,
            RAG_CHUNKING_CONTRACT_V1
        )
        .as_bytes(),
    )
}

/// Verdict for one asset under the OCR rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrDecision {
    /// Image/PDF without any canonical extraction row.
    Eligible,
    /// An extraction row already exists (any method, even empty text).
    AlreadyDone { method: String },
    /// Audio or any non-image/non-PDF type. Batch OCR never adds
    /// transcription; that is a different operation.
    UnsupportedType { asset_type: String },
    /// PDF root with page children: schedule the pages, not the container.
    ParentHasPages { page_count: i64 },
}

/// Verdict for one asset under the embeddings rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingDecision {
    /// Text exists but the stored result is missing or disagrees with it.
    Eligible { reason: EmbeddingStaleReason },
    /// Vector, chunk set, revision, and contract all agree.
    Fresh,
    /// No usable text in any extraction/transcription. Not a provider error.
    NoSourceText,
}

/// Why stored embeddings no longer satisfy the current input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingStaleReason {
    MissingVector,
    ContractMismatch,
    SourceChanged,
    IncompleteChunks,
    RepairMarked,
}

fn asset_type_and_parent(
    conn: &Connection,
    asset_id: &str,
) -> Result<Option<(String, Option<String>)>, String> {
    let mut stmt = conn
        .prepare("SELECT type, parent_asset_id FROM assets WHERE id = ?1")
        .map_err(|e| format!("Failed to read asset {asset_id}: {e}"))?;
    let mut rows = stmt
        .query_map([asset_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| format!("Failed to read asset {asset_id}: {e}"))?;
    match rows.next() {
        None => Ok(None),
        Some(row) => row
            .map(Some)
            .map_err(|e| format!("Failed to read asset {asset_id}: {e}")),
    }
}

/// Applies the OCR rule (§3.2) to one asset. Reads rows only; never opens the
/// file, so classification pages stay short transactions.
pub fn ocr_decision(conn: &Connection, asset_id: &str) -> Result<OcrDecision, String> {
    let Some((asset_type, _parent)) = asset_type_and_parent(conn, asset_id)? else {
        return Err(format!("Asset {asset_id} no longer exists"));
    };
    if asset_type != "image" && asset_type != "pdf" {
        return Ok(OcrDecision::UnsupportedType { asset_type });
    }
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM assets WHERE parent_asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count pages of {asset_id}: {e}"))?;
    if page_count > 0 {
        return Ok(OcrDecision::ParentHasPages { page_count });
    }
    let method: Option<String> = conn
        .query_row(
            "SELECT method FROM extractions WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .map(|method| Some(method))
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read extraction of {asset_id}: {other}")),
        })?;
    match method {
        Some(method) => Ok(OcrDecision::AlreadyDone { method }),
        None => Ok(OcrDecision::Eligible),
    }
}

/// One non-empty text source feeding the embedding input set.
#[derive(Debug, Clone)]
struct TextSource {
    kind: &'static str,
    id: String,
    text: String,
}

fn text_sources(conn: &Connection, asset_id: &str) -> Result<Vec<TextSource>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, 'extraction', text_content FROM extractions
             WHERE asset_id = ?1 AND LENGTH(TRIM(COALESCE(text_content, ''))) > 0
             UNION ALL
             SELECT id, 'transcription', text_content FROM transcriptions
             WHERE asset_id = ?1 AND LENGTH(TRIM(COALESCE(text_content, ''))) > 0
             ORDER BY 2, 1",
        )
        .map_err(|e| format!("Failed to read text sources of {asset_id}: {e}"))?;
    let rows = stmt
        .query_map([asset_id], |row| {
            Ok(TextSource {
                id: row.get(0)?,
                kind: if row.get::<_, String>(1)? == "extraction" {
                    "extraction"
                } else {
                    "transcription"
                },
                text: row.get(2)?,
            })
        })
        .map_err(|e| format!("Failed to read text sources of {asset_id}: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read text sources of {asset_id}: {e}"))
}

/// Stable fingerprint of the embedding input set: every source id, its text
/// hash, and the effective contract. Stored on the task at admission and
/// re-compared before COMMIT, so a result computed from older text can never
/// clear a newer invalidation.
pub fn embedding_input_fingerprint(conn: &Connection, asset_id: &str) -> Result<String, String> {
    let sources = text_sources(conn, asset_id)?;
    let mut parts = Vec::with_capacity(sources.len() + 1);
    for source in &sources {
        parts.push(format!(
            "{}|{}|{}",
            source.kind,
            source.id,
            sha256_hex(source.text.as_bytes())
        ));
    }
    parts.push(current_embedding_contract_hash());
    Ok(sha256_hex(parts.join("\n").as_bytes()))
}

/// Applies the embeddings rule (§3.3) to one asset.
pub fn embedding_decision(conn: &Connection, asset_id: &str) -> Result<EmbeddingDecision, String> {
    let sources = text_sources(conn, asset_id)?;
    if sources.is_empty() {
        return Ok(EmbeddingDecision::NoSourceText);
    }
    // A newer source revision than the last completed embedding means the
    // text moved under us. Assets that predate revision tracking — or whose
    // text was imported but never embedded through the queue — have no
    // completion stamp and keep the contract/chunk checks below, so legacy
    // results are not recomputed just for lacking a marker.
    let revision: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT source_revision, embedding_completed_revision FROM processing_asset_revisions WHERE asset_id = ?1",
            [asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("Failed to read revisions of {asset_id}: {other}")),
        })?;
    if let Some((source_revision, Some(completed))) = revision {
        if completed != source_revision {
            return Ok(EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::SourceChanged,
            });
        }
    }
    let vector_ok: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM vec_assets WHERE asset_id = ?1 AND embedding_model = ?2 AND embedding_contract = ?3 AND dimensions = ?4",
            rusqlite::params![
                asset_id,
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|e| format!("Failed to read vector of {asset_id}: {e}"))?;
    if !vector_ok {
        let legacy: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM vec_assets WHERE asset_id = ?1",
                [asset_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to read vector of {asset_id}: {e}"))?;
        return Ok(EmbeddingDecision::Eligible {
            reason: if legacy > 0 {
                EmbeddingStaleReason::ContractMismatch
            } else {
                EmbeddingStaleReason::MissingVector
            },
        });
    }
    for source in &sources {
        let source_hash = sha256_hex(source.text.as_bytes());
        let matching: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_chunks WHERE asset_id = ?1 AND source_kind = ?2 AND source_id = ?3 AND source_text_hash = ?4 AND embedding_model = ?5 AND embedding_contract = ?6 AND dimensions = ?7 AND chunking_contract = ?8",
                rusqlite::params![
                    asset_id,
                    source.kind,
                    source.id,
                    source_hash,
                    CANONICAL_EMBEDDING_MODEL,
                    CANONICAL_EMBEDDING_CONTRACT_V1,
                    CANONICAL_EMBEDDING_DIMENSIONS as i64,
                    RAG_CHUNKING_CONTRACT_V1,
                ],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to read chunks of {asset_id}: {e}"))?;
        let expected = expected_chunk_count(source.text.chars().count()) as i64;
        if matching != expected {
            return Ok(EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::IncompleteChunks,
            });
        }
    }
    if repair_marked(conn, asset_id)? {
        return Ok(EmbeddingDecision::Eligible {
            reason: EmbeddingStaleReason::RepairMarked,
        });
    }
    Ok(EmbeddingDecision::Fresh)
}

/// True when the legacy repair marker still flags this asset incomplete.
/// Read-only bridge until Unidad 4 migrates the marker into queue state.
fn repair_marked(conn: &Connection, asset_id: &str) -> Result<bool, String> {
    let table: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'rag_asset_embedding_state'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to inspect repair state: {e}"))?;
    if table == 0 {
        return Ok(false);
    }
    conn.query_row(
        "SELECT COUNT(*) FROM rag_asset_embedding_state WHERE asset_id = ?1 AND rag_incomplete = 1",
        [asset_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(|e| format!("Failed to read repair state of {asset_id}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_counter_matches_the_chunker_on_boundaries() {
        for len in [
            0, 1, 799, 800, 801, 899, 900, 901, 1500, 1600, 1601, 2300, 10000,
        ] {
            let text = "a".repeat(len);
            let source = crate::nlp::embeddings::RagChunkSource {
                asset_id: "a".to_string(),
                item_id: "i".to_string(),
                source_kind: crate::nlp::embeddings::RagChunkSourceKind::Extraction,
                source_id: "e".to_string(),
                text: text.clone(),
            };
            let planned = if len == 0 {
                0
            } else {
                crate::nlp::embeddings::plan_rag_chunks(&source)
                    .expect("plan")
                    .len()
            };
            assert_eq!(
                expected_chunk_count(len),
                planned,
                "counter must agree with plan_rag_chunks at len {len}"
            );
        }
    }
}
