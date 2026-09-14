//! Embedding engine behind the batch queue (plan-lote.md §3.3, §8).
//!
//! One asset becomes one aggregate BGE-M3 vector plus one chunk set per text
//! source, reusing the canonical chunker, contracts, and validation of
//! `nlp::embeddings` — but split into stageable pieces:
//!
//! - snapshot (short read) → compute outside any transaction → stage one
//!   checkpoint per source → publish vector + full chunk set + receipt in ONE
//!   commit transaction.
//!
//! A resumed run skips sources whose staged checkpoint already matches the
//! pinned fingerprint and contract, so a crash after seven confirmed sources
//! never re-invokes the provider for them.

use std::path::PathBuf;
use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::repository::NewCheckpoint;
use super::scheduler::{ClaimedTask, ExecCtx, ExecOutput, ExecResult, Executor, StopFlag};
use crate::db::open::open_archive_connection;
use crate::nlp::embeddings::{
    self, RagChunkEmbeddingSpec, RagChunkSource, RagChunkSourceKind,
    CANONICAL_EMBEDDING_CONTRACT_V1, CANONICAL_EMBEDDING_DIMENSIONS, CANONICAL_EMBEDDING_MODEL,
};

/// One text source with its computed chunk vectors, in plan order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedSource {
    pub kind: String,
    pub source_id: String,
    pub text_hash: String,
    /// Base64 little-endian f32 blobs, one per planned chunk, in order.
    pub chunk_blobs: Vec<String>,
}

/// Whole-asset embedding result in plain data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingComputeOutput {
    pub item_id: String,
    pub aggregate_blob: String,
    pub provider: String,
    pub sources: Vec<StagedSource>,
}

fn decode_blob(blob_b64: &str) -> Result<Vec<f32>, String> {
    let bytes = BASE64_STANDARD
        .decode(blob_b64)
        .map_err(|e| format!("Stored embedding checkpoint is corrupt: {e}"))?;
    if bytes.len() % 4 != 0 {
        return Err("Stored embedding checkpoint has a truncated vector".to_string());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]]))
        .collect())
}

fn encode_blob(vector: &[f32]) -> String {
    BASE64_STANDARD.encode(embeddings::floats_to_blob(vector))
}

/// Publishes one asset's embeddings inside the commit transaction: aggregate
/// vector UPSERT (UPDATE-only, no sync tombstones), per-source chunk-set
/// replacement replaying the staged vectors positionally, stale-source
/// cleanup, and legacy repair-marker drainage. Never begins its own
/// transaction; never calls the provider.
pub fn publish_embedding_output(
    conn: &Connection,
    asset_id: &str,
    output: &EmbeddingComputeOutput,
) -> Result<(), String> {
    let aggregate = decode_blob(&output.aggregate_blob)?;
    validate_vector(&aggregate)?;
    // Validate every staged set BEFORE writing anything: a short set must
    // refuse the whole publish, not leave a vector without its chunks.
    // (Inside the commit transaction the caller aborts on any error; this
    // ordering also keeps direct callers from persisting half a result.)
    let mut validated = Vec::with_capacity(output.sources.len());
    for staged in &output.sources {
        let kind = match staged.kind.as_str() {
            "transcription" => RagChunkSourceKind::Transcription,
            _ => RagChunkSourceKind::Extraction,
        };
        let source = RagChunkSource {
            asset_id: asset_id.to_string(),
            item_id: output.item_id.clone(),
            source_kind: kind,
            source_id: staged.source_id.clone(),
            text: staged_text(conn, asset_id, kind, &staged.source_id)?,
        };
        let planned = embeddings::plan_rag_chunks(&source)?;
        if planned.len() != staged.chunk_blobs.len() {
            return Err(format!(
                "Staged embedding for {} has {} vectors for {} planned chunks; refusing a partial publish",
                staged.source_id,
                staged.chunk_blobs.len(),
                planned.len()
            ));
        }
        let mut vectors = Vec::with_capacity(staged.chunk_blobs.len());
        for blob_b64 in &staged.chunk_blobs {
            vectors.push(decode_blob(blob_b64)?);
        }
        validated.push((source, vectors));
    }
    embeddings::upsert_vec_asset(
        conn,
        &output.item_id,
        asset_id,
        &embeddings::floats_to_blob(&aggregate),
    )?;
    let spec = RagChunkEmbeddingSpec {
        model: CANONICAL_EMBEDDING_MODEL,
        contract: CANONICAL_EMBEDDING_CONTRACT_V1,
        dimensions: CANONICAL_EMBEDDING_DIMENSIONS,
    };
    for (source, vectors) in &validated {
        let mut replay = vectors.iter();
        embeddings::backfill_rag_chunks(conn, source, spec, |_chunk_text| {
            replay
                .next()
                .cloned()
                .ok_or_else(|| "Staged embedding ran out of vectors mid-publish".to_string())
        })?;
    }
    // Sources deleted while the unit ran leave orphan chunk sets behind:
    // retire them in the same commit so search never sees mixed revisions.
    let staged_ids: std::collections::HashSet<(String, String)> = output
        .sources
        .iter()
        .map(|s| (s.kind.clone(), s.source_id.clone()))
        .collect();
    let mut stmt = conn
        .prepare("SELECT DISTINCT source_kind, source_id FROM rag_chunks WHERE asset_id = ?1")
        .map_err(|e| format!("Failed to scan persisted chunk sources: {e}"))?;
    let persisted = stmt
        .query_map([asset_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Failed to scan persisted chunk sources: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to scan persisted chunk sources: {e}"))?;
    drop(stmt);
    for (kind, source_id) in persisted {
        if !staged_ids.contains(&(kind.clone(), source_id.clone())) {
            conn.execute(
                "DELETE FROM rag_chunks WHERE asset_id = ?1 AND source_kind = ?2 AND source_id = ?3",
                rusqlite::params![asset_id, kind, source_id],
            )
            .map_err(|e| format!("Failed to retire orphan chunk source {source_id}: {e}"))?;
        }
    }
    // Drain the legacy repair marker this result supersedes (Unidad 4
    // cutover): the queue receipt replaces it as the source of truth.
    let marker: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'rag_asset_embedding_state'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to inspect repair state: {e}"))?;
    if marker > 0 {
        conn.execute(
            "DELETE FROM rag_asset_embedding_state WHERE asset_id = ?1",
            [asset_id],
        )
        .map_err(|e| format!("Failed to drain repair marker: {e}"))?;
    }
    Ok(())
}
fn validate_vector(vector: &[f32]) -> Result<(), String> {
    if vector.len() != CANONICAL_EMBEDDING_DIMENSIONS
        || vector.iter().any(|value| !value.is_finite())
    {
        return Err(format!(
            "Computed embedding does not satisfy {} finite dimensions",
            CANONICAL_EMBEDDING_DIMENSIONS
        ));
    }
    Ok(())
}

/// Classifies engine failures. Transport and overload signals retry with
/// backoff; a missing engine parks as configuration (the user fixes
/// settings, then resumes); anything else fails the unit with its cause.
pub(crate) fn map_embedding_error(error: &str) -> ExecOutput {
    let lower = error.to_lowercase();
    for signal in [
        "timeout",
        "timed out",
        "connection",
        "429",
        "rate limit",
        "500",
        "502",
        "503",
        "504",
    ] {
        if lower.contains(signal) {
            return ExecOutput::Retryable {
                code: "provider_transient".to_string(),
                message: error.to_string(),
            };
        }
    }
    if lower.contains("no bge-m3 embedding engine")
        || lower.contains("api credential")
        || lower.contains("unauthorized")
        || lower.contains("401")
        || lower.contains("403")
    {
        return ExecOutput::Blocked {
            code: "configuration_required".to_string(),
            message: error.to_string(),
        };
    }
    ExecOutput::Fatal {
        code: "embedding_failed".to_string(),
        message: error.to_string(),
    }
}

fn staged_text(
    conn: &Connection,
    asset_id: &str,
    kind: RagChunkSourceKind,
    source_id: &str,
) -> Result<String, String> {
    let table = match kind {
        RagChunkSourceKind::Extraction => "extractions",
        RagChunkSourceKind::Transcription => "transcriptions",
    };
    conn.query_row(
        &format!("SELECT text_content FROM {table} WHERE id = ?1 AND asset_id = ?2"),
        rusqlite::params![source_id, asset_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("Source {source_id} vanished before publish: {e}"))
}
struct EngineCache {
    cached: Option<crate::nlp::CachedEmbeddingEngine>,
    last_error: Option<String>,
}

/// Queue-owned embedding executor. The engine initializes lazily from the
/// current settings and re-initializes when they change (same fingerprint
/// rule as the retired worker); exactly one engine exists per process.
pub struct EmbeddingExecutor {
    #[allow(dead_code)]
    app: AppHandle,
    db_path: PathBuf,
    cache: Mutex<EngineCache>,
}

impl EmbeddingExecutor {
    pub fn new(app: AppHandle, db_path: PathBuf) -> Self {
        Self {
            app,
            db_path,
            cache: Mutex::new(EngineCache {
                cached: None,
                last_error: None,
            }),
        }
    }

    fn settings_conn(&self) -> Result<Connection, String> {
        open_archive_connection(&self.db_path)
    }

    fn engine(&self) -> Result<std::sync::Arc<embeddings::EmbeddingEngine>, String> {
        let conn = self.settings_conn()?;
        let mut guard = self
            .cache
            .lock()
            .map_err(|e| format!("Embedding engine lock poisoned: {e}"))?;
        let EngineCache { cached, last_error } = &mut *guard;
        let engine =
            crate::nlp::ensure_embed_engine_for_current_settings(&conn, cached, last_error);
        engine.ok_or_else(|| embeddings::embedding_engine_unavailable_reason(last_error.as_deref()))
    }
}
fn sha256_hex_label(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let digest = sha2::Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn source_unit_key(kind: &str, source_id: &str) -> String {
    format!("src:{kind}:{source_id}")
}
fn stage_checkpoint(task: &ClaimedTask, staged: &StagedSource) -> NewCheckpoint {
    let payload = serde_json::to_string(staged).unwrap_or_else(|_| "{}".to_string());
    let checksum = sha256_hex_label(payload.as_bytes());
    NewCheckpoint {
        unit_key: source_unit_key(&staged.kind, &staged.source_id),
        input_fingerprint: task.input_fingerprint.clone(),
        contract_hash: task.contract_hash.clone(),
        payload,
        payload_checksum: checksum,
    }
}

impl Executor for EmbeddingExecutor {
    fn kinds(&self) -> &[&str] {
        &["embedding"]
    }

    fn run(&self, _ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
        let failed = |output: ExecOutput| ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: None,
            output,
        };
        let fatal = |code: &str, message: String| {
            failed(ExecOutput::Fatal {
                code: code.to_string(),
                message,
            })
        };
        // Snapshot the input set on a short read: texts travel with the
        // run, the commit transaction only verifies and writes.
        let conn = match self.settings_conn() {
            Ok(conn) => conn,
            Err(error) => return fatal("storage_unavailable", error),
        };
        let item_id = match crate::nlp::lookup_item_id_for_asset(&conn, &task.asset_id) {
            Ok(Some(item_id)) => item_id,
            Ok(None) => {
                return fatal(
                    "source_deleted",
                    format!("Asset {} no longer exists", task.asset_id),
                )
            }
            Err(error) => return fatal("storage_unavailable", error),
        };
        let sources = match snapshot_sources(&conn, &task.asset_id, &item_id) {
            Ok(sources) => sources,
            Err(error) => return fatal("storage_unavailable", error),
        };
        if sources.is_empty() {
            // The claim validated text; losing it mid-flight is a stop, not
            // a provider fault — resume revalidates instead of failing.
            return ExecResult {
                checkpoints: Vec::new(),
                progress_total: Some(0),
                engine_output: None,
                output: ExecOutput::Stopped,
            };
        }
        let staged = match read_staged(&conn, task) {
            Ok(staged) => staged,
            Err(error) => return fatal("storage_unavailable", error),
        };
        drop(conn);
        let engine = match self.engine() {
            Ok(engine) => engine,
            Err(error) => return failed(map_embedding_error(&error)),
        };
        let mut new_checkpoints = Vec::new();
        let mut staged_all = Vec::with_capacity(sources.len());
        for (kind, source_id, text) in &sources {
            if stop.stopped() {
                return stopped_with(new_checkpoints, sources.len());
            }
            let key = (kind.clone(), source_id.clone());
            if let Some(resumed) = staged.get(&key) {
                staged_all.push(resumed.clone());
                continue;
            }
            let source = RagChunkSource {
                asset_id: task.asset_id.clone(),
                item_id: item_id.clone(),
                source_kind: if kind == "transcription" {
                    RagChunkSourceKind::Transcription
                } else {
                    RagChunkSourceKind::Extraction
                },
                source_id: source_id.clone(),
                text: text.clone(),
            };
            let planned = match embeddings::plan_rag_chunks(&source) {
                Ok(planned) => planned,
                Err(error) => return fatal("embedding_failed", error),
            };
            let mut blobs = Vec::with_capacity(planned.len());
            for chunk in &planned {
                if stop.stopped() {
                    return stopped_with(new_checkpoints, sources.len());
                }
                match engine.embed_text(&chunk.text_content) {
                    Ok(vector) => {
                        if let Err(error) = validate_vector(&vector) {
                            return fatal("embedding_failed", error);
                        }
                        blobs.push(encode_blob(&vector));
                    }
                    Err(error) => return failed(map_embedding_error(&error)),
                }
            }
            let staged_source = StagedSource {
                kind: kind.clone(),
                source_id: source_id.clone(),
                text_hash: sha256_hex_label(text.as_bytes()),
                chunk_blobs: blobs,
            };
            new_checkpoints.push(stage_checkpoint(task, &staged_source));
            staged_all.push(staged_source);
        }
        if stop.stopped() {
            return stopped_with(new_checkpoints, sources.len());
        }
        // Aggregate vector mirrors the historic compute path: one embed call
        // over the concatenated asset text.
        let full_text = match self
            .settings_conn()
            .and_then(|conn| crate::nlp::text_provider::get_asset_text(&conn, &task.asset_id))
        {
            Ok(text) => text,
            Err(error) => return fatal("storage_unavailable", error),
        };
        let aggregate = match engine.embed_text(&full_text) {
            Ok(vector) => vector,
            Err(error) => return failed(map_embedding_error(&error)),
        };
        if let Err(error) = validate_vector(&aggregate) {
            return fatal("embedding_failed", error);
        }
        let output = EmbeddingComputeOutput {
            item_id,
            aggregate_blob: encode_blob(&aggregate),
            provider: engine.provider_name().to_string(),
            sources: staged_all,
        };
        ExecResult {
            checkpoints: new_checkpoints,
            progress_total: Some(sources.len() as i64),
            engine_output: Some(super::scheduler::EngineOutput::Embedding(output)),
            output: ExecOutput::Success {
                outcome: "embedded".to_string(),
                receipt: "{}".to_string(),
            },
        }
    }
}

fn stopped_with(checkpoints: Vec<NewCheckpoint>, total_sources: usize) -> ExecResult {
    ExecResult {
        checkpoints,
        progress_total: Some(total_sources as i64),
        engine_output: None,
        output: ExecOutput::Stopped,
    }
}

/// (kind, source_id, text) snapshot, same ordering as the historic backfill.
fn snapshot_sources(
    conn: &Connection,
    asset_id: &str,
    _item_id: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, 'extraction', text_content FROM extractions
             WHERE asset_id = ?1 AND LENGTH(TRIM(COALESCE(text_content, ''))) > 0
             UNION ALL
             SELECT id, 'transcription', text_content FROM transcriptions
             WHERE asset_id = ?1 AND LENGTH(TRIM(COALESCE(text_content, ''))) > 0
             ORDER BY 2, 1",
        )
        .map_err(|e| format!("Failed to snapshot sources of {asset_id}: {e}"))?;
    let rows = stmt
        .query_map([asset_id], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(0)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("Failed to snapshot sources of {asset_id}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to snapshot sources of {asset_id}: {e}"))?;
    drop(stmt);
    Ok(rows)
}

/// Previously staged sources for this exact pinned input.
fn read_staged(
    conn: &Connection,
    task: &ClaimedTask,
) -> Result<std::collections::HashMap<(String, String), StagedSource>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT payload FROM processing_checkpoints
             WHERE task_id = ?1 AND unit_key LIKE 'src:%' AND input_fingerprint = ?2 AND contract_hash = ?3",
        )
        .map_err(|e| format!("Failed to read staged sources: {e}"))?;
    let rows = stmt
        .query_map(
            rusqlite::params![task.task_id, task.input_fingerprint, task.contract_hash],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| format!("Failed to read staged sources: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read staged sources: {e}"))?;
    drop(stmt);
    let mut staged = std::collections::HashMap::new();
    for payload in rows {
        match serde_json::from_str::<StagedSource>(&payload) {
            Ok(source) => {
                staged.insert((source.kind.clone(), source.source_id.clone()), source);
            }
            Err(error) => {
                // A corrupt checkpoint is not a failure: the source simply
                // recomputes below and overwrites it.
                eprintln!(
                    "[processing] Stored embedding checkpoint is corrupt, recomputing: {error}"
                );
            }
        }
    }
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, method TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, model TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE vec_assets(asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL, embedding_model TEXT NOT NULL DEFAULT 'legacy', embedding_contract TEXT NOT NULL DEFAULT 'legacy', dimensions INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE rag_chunks(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, item_id TEXT NOT NULL, source_kind TEXT NOT NULL, source_id TEXT NOT NULL, chunk_ordinal INTEGER NOT NULL, text_content TEXT NOT NULL, start_char INTEGER NOT NULL, end_char INTEGER NOT NULL, source_text_hash TEXT NOT NULL, chunking_contract TEXT NOT NULL, embedding BLOB NOT NULL, embedding_model TEXT NOT NULL, embedding_contract TEXT NOT NULL, dimensions INTEGER NOT NULL);
             CREATE VIRTUAL TABLE rag_chunks_fts USING fts5(chunk_id UNINDEXED, text_content, tokenize = 'unicode61 remove_diacritics 1');
             CREATE TRIGGER rag_chunks_fts_insert AFTER INSERT ON rag_chunks BEGIN INSERT INTO rag_chunks_fts(chunk_id, text_content) VALUES (NEW.id, NEW.text_content); END;
             CREATE TRIGGER rag_chunks_fts_delete AFTER DELETE ON rag_chunks BEGIN DELETE FROM rag_chunks_fts WHERE chunk_id = OLD.id; END;",
        )
        .expect("publish fixture");
        conn
    }

    fn staged_fixture(text: &str, chunks: usize) -> StagedSource {
        // Builds staged vectors through the canonical chunker so the test
        // pins positional replay against plan_rag_chunks.
        let source = RagChunkSource {
            asset_id: "a1".to_string(),
            item_id: "i1".to_string(),
            source_kind: RagChunkSourceKind::Extraction,
            source_id: "e1".to_string(),
            text: text.to_string(),
        };
        let planned = embeddings::plan_rag_chunks(&source).expect("plan");
        assert_eq!(planned.len(), chunks);
        let blobs = planned
            .iter()
            .enumerate()
            .map(|(index, _)| {
                encode_blob(&vec![index as f32 + 0.5; CANONICAL_EMBEDDING_DIMENSIONS])
            })
            .collect();
        StagedSource {
            kind: "extraction".to_string(),
            source_id: "e1".to_string(),
            text_hash: "hash".to_string(),
            chunk_blobs: blobs,
        }
    }
    #[test]
    fn publish_writes_vector_chunks_and_drains_the_legacy_marker() {
        let conn = publish_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a1', 'i1', 'a1.png', 'image', 1)",
            [],
        )
        .expect("asset");
        let text = "contenido con peso semántico ".repeat(60);
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('e1', 'a1', ?1, 'ocr', 1)",
            rusqlite::params![text],
        )
        .expect("extraction");
        // Legacy incomplete marker from the retired pipeline.
        conn.execute_batch(
            "CREATE TABLE rag_asset_embedding_state(asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, rag_incomplete INTEGER NOT NULL DEFAULT 0, failure_count INTEGER NOT NULL DEFAULT 0, next_retry_at_ms INTEGER NOT NULL DEFAULT 0, last_error TEXT, updated_at_ms INTEGER NOT NULL DEFAULT 0);
             INSERT INTO rag_asset_embedding_state (asset_id, item_id, rag_incomplete) VALUES ('a1', 'i1', 1);",
        )
        .expect("legacy marker");
        // Stale chunk set from a deleted source: same asset, unknown source.
        conn.execute(
            "INSERT INTO rag_chunks (id, asset_id, item_id, source_kind, source_id, chunk_ordinal, text_content, start_char, end_char, source_text_hash, chunking_contract, embedding, embedding_model, embedding_contract, dimensions)
             VALUES ('stale', 'a1', 'i1', 'extraction', 'gone', 0, 'viejo', 0, 5, 'h', 'c', zeroblob(8), 'm', 'c', 1024)",
            [],
        )
        .expect("stale chunk");
        let staged = staged_fixture(&text, 3);
        let output = EmbeddingComputeOutput {
            item_id: "i1".to_string(),
            aggregate_blob: encode_blob(&vec![0.25; CANONICAL_EMBEDDING_DIMENSIONS]),
            provider: "test".to_string(),
            sources: vec![staged],
        };
        publish_embedding_output(&conn, "a1", &output).expect("publish");
        let (model, dims): (String, i64) = conn
            .query_row(
                "SELECT embedding_model, dimensions FROM vec_assets WHERE asset_id = 'a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("vector published");
        assert_eq!(model, CANONICAL_EMBEDDING_MODEL);
        assert_eq!(dims, CANONICAL_EMBEDDING_DIMENSIONS as i64);
        let chunks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_chunks WHERE asset_id = 'a1' AND source_id = 'e1'",
                [],
                |row| row.get(0),
            )
            .expect("chunks published");
        assert_eq!(chunks, 3);
        let stale: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_chunks WHERE id = 'stale'",
                [],
                |row| row.get(0),
            )
            .expect("stale check");
        assert_eq!(stale, 0, "orphan chunk sets retire in the same commit");
        let marker: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_asset_embedding_state WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("marker check");
        assert_eq!(marker, 0, "the queue receipt replaces the legacy marker");
        // Every published chunk carries the canonical contract.
        let contracts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_chunks WHERE asset_id = 'a1' AND embedding_contract != ?1",
                [CANONICAL_EMBEDDING_CONTRACT_V1],
                |row| row.get(0),
            )
            .expect("contract check");
        assert_eq!(contracts, 0);
    }

    #[test]
    fn publish_rejects_a_short_staged_set() {
        let conn = publish_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a1', 'i1', 'a1.png', 'image', 1)",
            [],
        )
        .expect("asset");
        let text = "texto ".repeat(200);
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('e1', 'a1', ?1, 'ocr', 1)",
            rusqlite::params![text],
        )
        .expect("extraction");
        // Two chunks planned, one staged: publish must refuse rather than
        // confirm half a source set as complete.
        let mut staged = staged_fixture(&text, 2);
        staged.chunk_blobs.truncate(1);
        let output = EmbeddingComputeOutput {
            item_id: "i1".to_string(),
            aggregate_blob: encode_blob(&vec![0.25; CANONICAL_EMBEDDING_DIMENSIONS]),
            provider: "test".to_string(),
            sources: vec![staged],
        };
        let error = publish_embedding_output(&conn, "a1", &output).expect_err("short set refused");
        assert!(error.contains("refusing a partial publish"), "{error}");
        let vectors: i64 = conn
            .query_row("SELECT COUNT(*) FROM vec_assets", [], |row| row.get(0))
            .expect("no partial vector");
        assert_eq!(vectors, 0);
    }

    #[test]
    fn embedding_errors_map_to_retry_block_or_fatal() {
        assert!(matches!(
            map_embedding_error("request timed out after 180s"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_embedding_error("OpenRouter API error (429): slow down"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_embedding_error(
                "No BGE-M3 embedding engine configured. Set OpenRouter API credentials"
            ),
            ExecOutput::Blocked { .. }
        ));
        assert!(matches!(
            map_embedding_error("some programming bug"),
            ExecOutput::Fatal { .. }
        ));
    }
}
