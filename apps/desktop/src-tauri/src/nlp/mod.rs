pub mod chunking;
pub mod commands;
pub mod embeddings;
pub mod fts;
pub mod ner;
pub mod text_provider;
pub(crate) mod vector;
// NOTE: `triples` module removed — semantic triples are now Gemma-only via the LLM pipeline
// (see llm::LlmJob::ExtractTriples / ExtractTriplesAsset). The old NLP regex route has
// been retired to prevent low-quality triples from overwriting LLM results in the `triples` table.

use rusqlite::OptionalExtension;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
// PathBuf is used by the local-ml runtime-root helpers and by the unit tests
// (which exercise those helpers via the `any(local-ml, test)`-gated `_with`
// variant), so keep it importable in test builds even without local-ml.
#[cfg(any(feature = "local-ml", test))]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::llm::LlmQueue;
#[cfg(feature = "local-ml")]
use crate::runtime::RuntimeManager;
use embeddings::EmbeddingEngine;

struct CachedEmbeddingEngine {
    config_key: String,
    engine: Arc<EmbeddingEngine>,
}

// ── Event payloads ───────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct NlpProgressPayload {
    pub item_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    pub job: String,
    pub pct: u8,
}

#[derive(Clone, Serialize)]
pub struct NlpCompletePayload {
    pub item_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    pub job: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_count: Option<usize>,
}

#[derive(Clone, Serialize)]
pub struct NlpErrorPayload {
    pub item_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    pub job: String,
    pub error: String,
}

// ── Job & Queue ──────────────────────────────────────────────────────────────

/// A single NLP work unit submitted to the background worker.
#[derive(Debug)]
pub enum NlpJob {
    IndexFts { item_id: String },
    ExtractEntities { item_id: String },
    EnrichItem { item_id: String },
    // Asset-level variants: process only the selected asset/page
    ComputeAssetEmbedding { item_id: String, asset_id: String },
    ExtractEntitiesForAsset { item_id: String, asset_id: String },
}

pub fn lookup_item_id_for_asset(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT item_id FROM assets WHERE id = ?1",
        rusqlite::params![asset_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| format!("Failed to resolve item_id for asset {asset_id}: {e}"))
}

pub fn enqueue_entity_refresh_for_item(nlp_queue: &NlpQueue, item_id: &str) -> Result<(), String> {
    // Dedup: if this item is already pending or in-progress for NER, skip.
    if let Ok(mut pending) = nlp_queue.ner_pending.lock() {
        if pending.contains(item_id) {
            eprintln!("[nlp/ner] Skipping duplicate ExtractEntities enqueue for item_id={item_id}");
            return Ok(());
        }
        pending.insert(item_id.to_string());
    }
    let submit_result = nlp_queue.submit(NlpJob::ExtractEntities {
        item_id: item_id.to_string(),
    });

    if submit_result.is_err() {
        if let Ok(mut pending) = nlp_queue.ner_pending.lock() {
            pending.remove(item_id);
        }
    }

    submit_result
}

/// Handle for submitting NLP jobs to the background worker.
///
/// Managed as Tauri state — NLP commands grab this via `State<NlpQueue>`.
/// Includes a dedup set for ExtractEntities jobs to avoid processing the
/// same item_id twice in quick succession.
pub struct NlpQueue {
    sender: mpsc::Sender<NlpJob>,
    /// Set of item_ids currently pending or in-progress for ExtractEntities.
    /// Prevents duplicate NER work when OCR and transcription both trigger
    /// entity extraction for the same item.
    ner_pending: Arc<Mutex<HashSet<String>>>,
    /// Tracks queued/in-progress FTS jobs per item.
    /// `true` means another enqueue arrived while the current one was busy,
    /// so one extra rerun should happen after the current pass completes.
    fts_pending: Arc<Mutex<HashMap<String, bool>>>,
    /// Tracks queued/in-progress asset-level NER jobs per asset.
    asset_ner_pending: Arc<Mutex<HashSet<String>>>,
    /// Tracks queued/in-progress asset-level embedding jobs per asset.
    embedding_pending: Arc<Mutex<HashSet<String>>>,
}

impl NlpQueue {
    /// Create a new queue and return `(NlpQueue, Receiver)`.
    pub fn new() -> (Self, mpsc::Receiver<NlpJob>) {
        let (sender, receiver) = mpsc::channel::<NlpJob>(64);
        (
            Self {
                sender,
                ner_pending: Arc::new(Mutex::new(HashSet::new())),
                fts_pending: Arc::new(Mutex::new(HashMap::new())),
                asset_ner_pending: Arc::new(Mutex::new(HashSet::new())),
                embedding_pending: Arc::new(Mutex::new(HashSet::new())),
            },
            receiver,
        )
    }

    /// Submit a job to the queue. Returns immediately.
    pub fn submit(&self, job: NlpJob) -> Result<(), String> {
        let mut tracked_fts_item = None;
        let mut tracked_asset_ner = None;
        let mut tracked_embedding = None;

        match &job {
            NlpJob::IndexFts { item_id } => {
                if let Ok(mut pending) = self.fts_pending.lock() {
                    if let Some(needs_rerun) = pending.get_mut(item_id) {
                        *needs_rerun = true;
                        eprintln!(
                            "[nlp/fts] Coalescing duplicate IndexFts enqueue for item_id={item_id}"
                        );
                        return Ok(());
                    }
                    pending.insert(item_id.clone(), false);
                }
                tracked_fts_item = Some(item_id.clone());
            }
            NlpJob::ExtractEntitiesForAsset { asset_id, .. } => {
                if let Ok(mut pending) = self.asset_ner_pending.lock() {
                    if !pending.insert(asset_id.clone()) {
                        eprintln!(
                            "[nlp/ner] Coalescing duplicate ExtractEntitiesForAsset enqueue for asset_id={asset_id}"
                        );
                        return Ok(());
                    }
                }
                tracked_asset_ner = Some(asset_id.clone());
            }
            NlpJob::ComputeAssetEmbedding { asset_id, .. } => {
                if let Ok(mut pending) = self.embedding_pending.lock() {
                    if !pending.insert(asset_id.clone()) {
                        eprintln!(
                            "[nlp/embeddings] Coalescing duplicate ComputeAssetEmbedding enqueue for asset_id={asset_id}"
                        );
                        return Ok(());
                    }
                }
                tracked_embedding = Some(asset_id.clone());
            }
            _ => {}
        }

        self.sender.try_send(job).map_err(|e| {
            if let Some(item_id) = tracked_fts_item {
                if let Ok(mut pending) = self.fts_pending.lock() {
                    pending.remove(&item_id);
                }
            }
            if let Some(asset_id) = tracked_asset_ner {
                if let Ok(mut pending) = self.asset_ner_pending.lock() {
                    pending.remove(&asset_id);
                }
            }
            if let Some(asset_id) = tracked_embedding {
                if let Ok(mut pending) = self.embedding_pending.lock() {
                    pending.remove(&asset_id);
                }
            }
            format!("Failed to enqueue NLP job: {e}")
        })
    }

    /// Get a clone of the NER dedup set handle.
    /// Used by the worker to remove item_ids after processing completes.
    pub fn ner_pending_handle(&self) -> Arc<Mutex<HashSet<String>>> {
        Arc::clone(&self.ner_pending)
    }

    pub fn fts_pending_handle(&self) -> Arc<Mutex<HashMap<String, bool>>> {
        Arc::clone(&self.fts_pending)
    }

    pub fn asset_ner_pending_handle(&self) -> Arc<Mutex<HashSet<String>>> {
        Arc::clone(&self.asset_ner_pending)
    }

    pub fn embedding_pending_handle(&self) -> Arc<Mutex<HashSet<String>>> {
        Arc::clone(&self.embedding_pending)
    }

    /// Spawn the background worker loop on the Tokio runtime.
    ///
    /// The worker drains jobs serially and emits `nlp:progress`, `nlp:complete`,
    /// or `nlp:error` events per job.
    pub fn start_worker(
        db_path: std::path::PathBuf,
        mut receiver: mpsc::Receiver<NlpJob>,
        app_handle: AppHandle,
        ner_pending: Arc<Mutex<HashSet<String>>>,
        fts_pending: Arc<Mutex<HashMap<String, bool>>>,
        asset_ner_pending: Arc<Mutex<HashSet<String>>>,
        embedding_pending: Arc<Mutex<HashSet<String>>>,
        _llm_queue: LlmQueue,
    ) {
        tauri::async_runtime::spawn(async move {
            // Open a dedicated SQLite connection for the NLP worker.
            let conn = match rusqlite::Connection::open(&db_path) {
                Ok(c) => {
                    let _ = c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;");
                    c
                }
                Err(e) => {
                    eprintln!("[nlp] Failed to open worker DB connection: {e}");
                    return;
                }
            };

            if table_exists(&conn, "entities") {
                if let Err(e) = ensure_entities_schema(&conn) {
                    eprintln!("[nlp] Failed to migrate entities schema: {e}");
                }
            }

            // Create vec_assets storage for asset-level embeddings.
            if let Err(e) = conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS vec_assets(
                    asset_id TEXT PRIMARY KEY,
                    item_id TEXT NOT NULL,
                    embedding BLOB NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_vec_assets_item_id ON vec_assets(item_id)",
            ) {
                eprintln!("[nlp] Failed to create embedding tables: {e} — embedding storage will be unavailable");
            }

            let mut embed_engine: Option<CachedEmbeddingEngine> = None;
            let mut last_embed_engine_init_error: Option<String> = None;

            while let Some(job) = receiver.recv().await {
                match job {
                    NlpJob::IndexFts { item_id } => {
                        emit_progress(&app_handle, &item_id, None, "fts", 10);
                        let result = run_coalesced_fts_reindex(&conn, &item_id, &fts_pending);
                        match result {
                            Ok(_) => {
                                eprintln!("[nlp/fts] Reindex complete: item_id={}", item_id);
                                emit_progress(&app_handle, &item_id, None, "fts", 100);
                                emit_complete(&app_handle, &item_id, None, "fts", None);
                            }
                            Err(e) => emit_error(&app_handle, &item_id, None, "fts", &e),
                        }
                    }
                    NlpJob::ExtractEntities { item_id } => {
                        emit_progress(&app_handle, &item_id, None, "ner", 10);
                        let prepared = ner::prepare_ner_candidates_for_item(&conn, &item_id);
                        let (text_present, result) = match prepared {
                            Ok(input) if input.text.trim().is_empty() => (false, Ok(Vec::new())),
                            Ok(input) => (
                                true,
                                run_configured_ner_input(
                                    &app_handle,
                                    &db_path,
                                    ner_fallback_config(&conn),
                                    input,
                                )
                                .await,
                            ),
                            Err(error) => (false, Err(format!("NER extraction failed: {error}"))),
                        };
                        // Remove from dedup set so future enqueues for this item are allowed
                        if let Ok(mut pending) = ner_pending.lock() {
                            pending.remove(&item_id);
                        }
                        match result {
                            Ok(final_entities) => {
                                // Data-loss guard: text present but NER found nothing → do NOT
                                // persist (the delete-then-insert would wipe existing Gemma/manual
                                // automatic entities). Preserve what's already stored.
                                if text_present && final_entities.is_empty() {
                                    eprintln!(
                                        "[nlp/ner] NER returned 0 entities for non-empty text — preserving existing entities item_id={item_id}"
                                    );
                                    emit_progress(&app_handle, &item_id, None, "ner", 100);
                                    emit_complete(&app_handle, &item_id, None, "ner", Some(0));
                                    continue;
                                }
                                if let Err(e) = tokio::task::block_in_place(|| {
                                    ner::persist_entities_for_item(&conn, &item_id, &final_entities)
                                }) {
                                    emit_error(&app_handle, &item_id, None, "ner", &e);
                                    continue;
                                }
                                emit_progress(&app_handle, &item_id, None, "ner", 100);
                                emit_complete(
                                    &app_handle,
                                    &item_id,
                                    None,
                                    "ner",
                                    Some(final_entities.len()),
                                );
                                // Auto-trigger geocoding for place entities
                                if let Err(e) = crate::geo::enqueue_geocoding_for_item(
                                    &app_handle.state::<crate::geo::GeoQueue>(),
                                    &item_id,
                                ) {
                                    eprintln!(
                                        "[geo] Failed to auto-enqueue geocoding after NER: {e}"
                                    );
                                }
                            }
                            Err(e) => emit_error(&app_handle, &item_id, None, "ner", &e),
                        }
                    }
                    NlpJob::EnrichItem { item_id } => {
                        // Run FTS first, then continue with NER. Semantic triples are Gemma-only
                        // via the LLM pipeline.
                        emit_progress(&app_handle, &item_id, None, "fts", 10);

                        let db_for_fts = db_path.clone();
                        let item_for_fts = item_id.clone();
                        let fts_handle =
                            tokio::task::spawn_blocking(move || -> Result<(), String> {
                                let c = rusqlite::Connection::open(&db_for_fts)
                                    .map_err(|e| format!("Failed to open FTS connection: {e}"))?;
                                let _ = c.execute_batch(
                                    "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;",
                                );
                                fts::index_item_from_db(&c, &item_for_fts)
                            });

                        match fts_handle.await {
                            Ok(Ok(())) => {
                                emit_progress(&app_handle, &item_id, None, "fts", 100);
                                emit_complete(&app_handle, &item_id, None, "fts", None);
                            }
                            Ok(Err(e)) => emit_error(&app_handle, &item_id, None, "fts", &e),
                            Err(e) => emit_error(
                                &app_handle,
                                &item_id,
                                None,
                                "fts",
                                &format!("FTS task panicked: {e}"),
                            ),
                        }

                        // NER sub-job: check dedup set — if ExtractEntities is already
                        // handling this item, skip NER here to avoid duplicate work.
                        let ner_already_pending = ner_pending
                            .lock()
                            .map(|p| p.contains(&item_id))
                            .unwrap_or(false);
                        if ner_already_pending {
                            eprintln!("[nlp/ner] Skipping NER in EnrichItem for item_id={item_id} — already queued or in progress");
                        } else {
                            // Register in dedup set before starting NER
                            if let Ok(mut pending) = ner_pending.lock() {
                                pending.insert(item_id.clone());
                            }
                            emit_progress(&app_handle, &item_id, None, "ner", 10);
                            let prepared = ner::prepare_ner_candidates_for_item(&conn, &item_id);
                            let (text_present, r) = match prepared {
                                Ok(input) if input.text.trim().is_empty() => {
                                    (false, Ok(Vec::new()))
                                }
                                Ok(input) => (
                                    true,
                                    run_configured_ner_input(
                                        &app_handle,
                                        &db_path,
                                        ner_fallback_config(&conn),
                                        input,
                                    )
                                    .await,
                                ),
                                Err(error) => {
                                    (false, Err(format!("NER extraction failed: {error}")))
                                }
                            };
                            // Remove from dedup set after NER completes
                            if let Ok(mut pending) = ner_pending.lock() {
                                pending.remove(&item_id);
                            }
                            match r {
                                Ok(final_entities) => {
                                    // Data-loss guard: text present but NER empty → preserve
                                    // existing automatic entities instead of wiping them.
                                    if text_present && final_entities.is_empty() {
                                        eprintln!(
                                            "[nlp/ner] NER returned 0 entities for non-empty text — preserving existing entities item_id={item_id}"
                                        );
                                        emit_progress(&app_handle, &item_id, None, "ner", 100);
                                        emit_complete(&app_handle, &item_id, None, "ner", Some(0));
                                        continue;
                                    }
                                    if let Err(e) = tokio::task::block_in_place(|| {
                                        ner::persist_entities_for_item(
                                            &conn,
                                            &item_id,
                                            &final_entities,
                                        )
                                    }) {
                                        emit_error(&app_handle, &item_id, None, "ner", &e);
                                        continue;
                                    }
                                    emit_progress(&app_handle, &item_id, None, "ner", 100);
                                    emit_complete(
                                        &app_handle,
                                        &item_id,
                                        None,
                                        "ner",
                                        Some(final_entities.len()),
                                    );
                                    if let Err(e) = crate::geo::enqueue_geocoding_for_item(
                                        &app_handle.state::<crate::geo::GeoQueue>(),
                                        &item_id,
                                    ) {
                                        eprintln!("[geo] Failed to auto-enqueue geocoding after NER (enrich): {e}");
                                    }
                                }
                                Err(e) => emit_error(&app_handle, &item_id, None, "ner", &e),
                            }
                        }
                    }

                    // ── Asset-level processing ─────────────────────────────────────
                    // These variants process only the selected asset/page text,
                    // not the entire item. Results are stored with both item_id
                    // (for ownership/cascade) and asset_id (for filtering).
                    NlpJob::ComputeAssetEmbedding { item_id, asset_id } => {
                        eprintln!(
                            "[nlp/embeddings] EMBED job queued item_id={item_id} asset_id={asset_id}"
                        );
                        emit_progress(&app_handle, &item_id, Some(&asset_id), "embed", 10);
                        let engine = ensure_embed_engine_for_current_settings(
                            &conn,
                            &mut embed_engine,
                            &mut last_embed_engine_init_error,
                        );
                        match engine.as_deref() {
                            Some(engine) => eprintln!(
                                "[nlp/embeddings] EMBED job using provider={} item_id={item_id} asset_id={asset_id}",
                                engine.provider_name()
                            ),
                            None => eprintln!(
                                "[nlp/embeddings] EMBED job has no engine item_id={item_id} asset_id={asset_id}"
                            ),
                        }
                        let result = tokio::task::block_in_place(|| {
                            embeddings::compute_and_store_for_asset_with_unavailable_reason(
                                engine.as_deref(),
                                &conn,
                                &item_id,
                                &asset_id,
                                last_embed_engine_init_error.as_deref(),
                            )
                        });
                        if let Ok(mut pending) = embedding_pending.lock() {
                            pending.remove(&asset_id);
                        }
                        match result {
                            Ok(_) => match asset_embedding_exists(&conn, &asset_id) {
                                Ok(true) => {
                                    let provider = engine
                                        .as_deref()
                                        .map(|engine| engine.provider_name())
                                        .unwrap_or("none");
                                    eprintln!(
                                        "[nlp/embeddings] EMBED job complete provider={provider} item_id={item_id} asset_id={asset_id}"
                                    );
                                    emit_progress(
                                        &app_handle,
                                        &item_id,
                                        Some(&asset_id),
                                        "embed",
                                        100,
                                    );
                                    emit_complete(
                                        &app_handle,
                                        &item_id,
                                        Some(&asset_id),
                                        "embed",
                                        None,
                                    );
                                }
                                Ok(false) => emit_error(
                                    &app_handle,
                                    &item_id,
                                    Some(&asset_id),
                                    "embed",
                                    "Asset embedding job completed but no vector was persisted",
                                ),
                                Err(e) => {
                                    emit_error(&app_handle, &item_id, Some(&asset_id), "embed", &e)
                                }
                            },
                            Err(e) => {
                                emit_error(&app_handle, &item_id, Some(&asset_id), "embed", &e)
                            }
                        }
                    }

                    NlpJob::ExtractEntitiesForAsset { item_id, asset_id } => {
                        emit_progress(&app_handle, &item_id, Some(&asset_id), "ner", 10);
                        let result =
                            ner::prepare_ner_candidates_for_asset(&conn, &item_id, &asset_id);
                        let result = match result {
                            Ok(input) => {
                                run_configured_ner_batch(
                                    &app_handle,
                                    &db_path,
                                    ner_fallback_config(&conn),
                                    input,
                                )
                                .await
                            }
                            Err(error) => Err(format!("NER extraction for asset failed: {error}")),
                        };
                        // Remove from asset-level dedup set so later OCR/transcription saves can refresh it.
                        if let Ok(mut pending) = asset_ner_pending.lock() {
                            pending.remove(&asset_id);
                        }
                        match result {
                            Ok(batch) => {
                                // NER uses spaCy first, then falls back to Gemma/OpenRouter by mode.
                                let text_present = !batch.text.trim().is_empty();
                                let final_entities = if text_present {
                                    batch.entities
                                } else {
                                    Vec::new()
                                };

                                // Data-loss guard: text present but NER found nothing → do NOT
                                // persist. The delete-then-insert would wipe the asset's existing
                                // Gemma ('llm') entities. Preserve them; only clear when the text
                                // itself is genuinely empty (intentional clear).
                                if text_present && final_entities.is_empty() {
                                    eprintln!(
                                        "[nlp/ner] Asset NER returned 0 entities for non-empty text — preserving existing entities item_id={item_id} asset_id={asset_id}"
                                    );
                                    emit_progress(
                                        &app_handle,
                                        &item_id,
                                        Some(&asset_id),
                                        "ner",
                                        100,
                                    );
                                    emit_complete(
                                        &app_handle,
                                        &item_id,
                                        Some(&asset_id),
                                        "ner",
                                        Some(0),
                                    );
                                    continue;
                                }

                                let entity_count = final_entities.len();
                                if let Err(e) = tokio::task::block_in_place(|| {
                                    ner::persist_entities_for_asset(
                                        &conn,
                                        &item_id,
                                        &asset_id,
                                        &final_entities,
                                    )
                                }) {
                                    emit_error(&app_handle, &item_id, Some(&asset_id), "ner", &e);
                                    continue;
                                }
                                emit_progress(&app_handle, &item_id, Some(&asset_id), "ner", 100);
                                emit_complete(
                                    &app_handle,
                                    &item_id,
                                    Some(&asset_id),
                                    "ner",
                                    Some(entity_count),
                                );
                                if let Err(e) = crate::geo::enqueue_geocoding_for_item(
                                    &app_handle.state::<crate::geo::GeoQueue>(),
                                    &item_id,
                                ) {
                                    eprintln!("[geo] Failed to auto-enqueue geocoding after asset NER: {e}");
                                }
                            }
                            Err(e) => emit_error(&app_handle, &item_id, Some(&asset_id), "ner", &e),
                        }
                    }
                }
            }
        });
    }
}

/// Attempt to initialize the selected embedding engine from app settings.
#[cfg(test)]
pub(crate) fn try_init_embed_engine(conn: &rusqlite::Connection) -> Option<Arc<EmbeddingEngine>> {
    try_init_embed_engine_result(conn).ok()
}

fn ensure_embed_engine_for_current_settings(
    conn: &rusqlite::Connection,
    cached: &mut Option<CachedEmbeddingEngine>,
    last_init_error: &mut Option<String>,
) -> Option<Arc<EmbeddingEngine>> {
    let config = match embeddings::config_from_settings(conn) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("[nlp/embeddings] Engine init blocked: {error}");
            *cached = None;
            *last_init_error = Some(error);
            return None;
        }
    };

    let config_key = embeddings::config_cache_key(&config);
    if let Some(cached_engine) = cached.as_ref() {
        if cached_engine.config_key == config_key {
            return Some(Arc::clone(&cached_engine.engine));
        }

        eprintln!("[nlp/embeddings] Embedding settings changed; reinitializing engine");
    }

    match init_embed_engine_from_config(config) {
        Ok(engine) => {
            *last_init_error = None;
            *cached = Some(CachedEmbeddingEngine {
                config_key,
                engine: Arc::clone(&engine),
            });
            Some(engine)
        }
        Err(error) => {
            eprintln!("[nlp/embeddings] Engine unavailable: {error}");
            *cached = None;
            *last_init_error = Some(error);
            None
        }
    }
}

#[cfg(test)]
pub(crate) fn try_init_embed_engine_result(
    conn: &rusqlite::Connection,
) -> Result<Arc<EmbeddingEngine>, String> {
    let config = match embeddings::config_from_settings(conn) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("[nlp/embeddings] Engine init blocked: {error}");
            return Err(error);
        }
    };

    init_embed_engine_from_config(config)
}

fn init_embed_engine_from_config(
    config: embeddings::EmbeddingConfig,
) -> Result<Arc<EmbeddingEngine>, String> {
    match EmbeddingEngine::init(config) {
        Ok(engine) => {
            eprintln!(
                "[nlp/embeddings] {} engine ready (lazy init)",
                engine.provider_name()
            );
            Ok(Arc::new(engine))
        }
        Err(e) => {
            eprintln!(
                "[nlp/embeddings] Engine init failed: {e} — embedding jobs will degrade gracefully"
            );
            Err(e)
        }
    }
}

pub(crate) fn ensure_nlp_runtime_ready(app_handle: &AppHandle) -> Result<(), String> {
    // Dev fallback is acceptable: Ok(None) means managed runtime is not healthy
    // but callers will fall back to CARGO_MANIFEST_DIR / system Python.
    // Local NLP (spaCy/local-gemma) only exists under local-ml; the lean build has
    // no managed runtime to prepare, so it is a no-op (mirrors Lite).
    #[cfg(feature = "local-ml")]
    {
        managed_runtime_root_for_nlp(app_handle).map(|_| ())
    }
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = app_handle;
        Ok(())
    }
}

#[cfg(feature = "local-ml")]
fn managed_runtime_root_for_nlp(app_handle: &AppHandle) -> Result<Option<PathBuf>, String> {
    managed_runtime_root_for_nlp_with(
        || RuntimeManager::new().ensure_ready_or_bootstrap(app_handle),
        || RuntimeManager::new().hydrated_runtime_root(app_handle),
    )
}

#[cfg(any(feature = "local-ml", test))]
fn managed_runtime_root_for_nlp_with<E, H>(
    ensure_ready_or_bootstrap: E,
    hydrated_runtime_root: H,
) -> Result<Option<PathBuf>, String>
where
    E: FnOnce() -> Result<crate::runtime::status::RuntimeStatus, String>,
    H: FnOnce() -> Result<Option<PathBuf>, String>,
{
    let status = ensure_ready_or_bootstrap()?;
    if status.state != crate::runtime::status::RuntimeState::Healthy {
        // Dev fallback: return None so callers fall back to CARGO_MANIFEST_DIR resources.
        // Honest blocking (e.g. no Python available) is handled at engine init time.
        return Ok(None);
    }

    hydrated_runtime_root()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn emit_progress(
    app_handle: &AppHandle,
    item_id: &str,
    asset_id: Option<&str>,
    job: &str,
    pct: u8,
) {
    if pct == 10 || pct == 100 {
        crate::app_logs::info(
            app_handle,
            "nlp",
            format!("{job} item_id={item_id} progreso={pct}%"),
        );
    }
    let _ = app_handle.emit(
        "nlp:progress",
        NlpProgressPayload {
            item_id: item_id.to_string(),
            asset_id: asset_id.map(str::to_string),
            job: job.to_string(),
            pct,
        },
    );
}

fn emit_complete(
    app_handle: &AppHandle,
    item_id: &str,
    asset_id: Option<&str>,
    job: &str,
    entity_count: Option<usize>,
) {
    crate::app_logs::info(
        app_handle,
        "nlp",
        format!("{job} completado para item_id={item_id}"),
    );
    let _ = app_handle.emit(
        "nlp:complete",
        NlpCompletePayload {
            item_id: item_id.to_string(),
            asset_id: asset_id.map(str::to_string),
            job: job.to_string(),
            entity_count,
        },
    );
}

fn emit_error(
    app_handle: &AppHandle,
    item_id: &str,
    asset_id: Option<&str>,
    job: &str,
    error: &str,
) {
    crate::app_logs::error(
        app_handle,
        "nlp",
        format!("{job} falló para item_id={item_id}: {error}"),
    );
    let _ = app_handle.emit(
        "nlp:error",
        NlpErrorPayload {
            item_id: item_id.to_string(),
            asset_id: asset_id.map(str::to_string),
            job: job.to_string(),
            error: error.to_string(),
        },
    );
}

async fn run_openrouter_ner_input(
    input: ner::OpenRouterExtractionInput,
) -> Result<Vec<ner::types::Entity>, String> {
    if input.text.trim().is_empty() {
        return Ok(Vec::new());
    }

    ner::openrouter::extract_entities_with_openrouter(
        input.api_key,
        input.model_name,
        &input.text,
        &input.protected_entities,
    )
    .await
    .map_err(|error| format!("NER extraction failed: {error}"))
}

async fn run_configured_ner_input(
    app_handle: &AppHandle,
    db_path: &std::path::Path,
    fallback: NerFallbackConfig,
    input: ner::NerExtractionInput,
) -> Result<Vec<ner::types::Entity>, String> {
    if input.text.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Lean build routes NER straight to OpenRouter and never touches the local
    // (spaCy / Gemma) engines, so these params are only consumed under local-ml.
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = app_handle;
        let _ = db_path;
    }

    // Prefer a non-empty spaCy result. An EMPTY spaCy pass (model resolved but
    // found nothing for this text) must NOT short-circuit the LLM fallback — on a
    // re-run that would silently produce zero entities and, combined with the
    // persist step, wipe a richer earlier Gemma result. Fall through so Gemma can
    // re-derive the entities; the call-site empty-guard is the final safety net.
    //
    // Under the lean build (no local-ml) spaCy is compiled out entirely, so this
    // attempt is skipped and control falls straight through to the OpenRouter
    // fallback below — matching Lite's OpenRouter-only NER shape.
    #[cfg(feature = "local-ml")]
    match run_spacy_ner(app_handle, db_path, &input).await {
        Ok(entities) if !entities.is_empty() => return Ok(entities),
        Ok(_) => {
            eprintln!("[nlp/ner] spaCy returned 0 entities; trying configured LLM fallback")
        }
        Err(error) => {
            eprintln!("[nlp/ner] spaCy NER unavailable; using configured LLM fallback: {error}")
        }
    }

    match fallback.mode {
        #[cfg(feature = "local-ml")]
        NerLlmFallbackMode::Local => run_local_gemma_ner(app_handle, db_path, &input).await,
        NerLlmFallbackMode::OpenRouter => {
            let (api_key, model_name) = fallback.openrouter?;
            run_openrouter_ner_input(ner::OpenRouterExtractionInput {
                text: input.text,
                protected_entities: input.protected_entities,
                api_key,
                model_name,
            })
            .await
        }
    }
}

async fn run_configured_ner_batch(
    app_handle: &AppHandle,
    db_path: &std::path::Path,
    fallback: NerFallbackConfig,
    input: ner::NerExtractionInput,
) -> Result<ner::EntityExtractionBatch, String> {
    let text = input.text.clone();
    let entities = run_configured_ner_input(app_handle, db_path, fallback, input)
        .await
        .map_err(|error| format!("NER extraction for asset failed: {error}"))?;
    Ok(ner::EntityExtractionBatch { text, entities })
}

#[derive(Clone, Copy)]
enum NerLlmFallbackMode {
    #[cfg(feature = "local-ml")]
    Local,
    OpenRouter,
}

struct NerFallbackConfig {
    mode: NerLlmFallbackMode,
    openrouter: Result<(String, String), String>,
}

fn ner_fallback_config(conn: &rusqlite::Connection) -> NerFallbackConfig {
    let mode = match crate::settings::get_setting(conn, "llm_mode")
        .unwrap_or_else(|| "local".to_string())
        .as_str()
    {
        "openrouter" | "auto" => NerLlmFallbackMode::OpenRouter,
        #[cfg(feature = "local-ml")]
        _ => NerLlmFallbackMode::Local,
        // Without the local engine the default must resolve to OpenRouter so NER
        // never routes to a non-existent local Gemma engine.
        #[cfg(not(feature = "local-ml"))]
        _ => NerLlmFallbackMode::OpenRouter,
    };
    let openrouter = match mode {
        NerLlmFallbackMode::OpenRouter => ner::openrouter_settings(conn),
        #[cfg(feature = "local-ml")]
        NerLlmFallbackMode::Local => {
            Err("OpenRouter no seleccionado para fallback NER".to_string())
        }
    };
    NerFallbackConfig { mode, openrouter }
}

#[cfg(feature = "local-ml")]
async fn run_spacy_ner(
    app_handle: &AppHandle,
    db_path: &std::path::Path,
    input: &ner::NerExtractionInput,
) -> Result<Vec<ner::types::Entity>, String> {
    let app_handle = app_handle.clone();
    let db_path = db_path.to_path_buf();
    let text = input.text.clone();
    let protected_entities = input.protected_entities.clone();
    tokio::task::spawn_blocking(move || {
        ner::spacy::extract_entities_with_spacy(&app_handle, &db_path, &text, &protected_entities)
    })
    .await
    .map_err(|error| format!("spaCy NER task panicked: {error}"))?
}

#[cfg(feature = "local-ml")]
async fn run_local_gemma_ner(
    app_handle: &AppHandle,
    db_path: &std::path::Path,
    input: &ner::NerExtractionInput,
) -> Result<Vec<ner::types::Entity>, String> {
    let app_handle = app_handle.clone();
    let db_path = db_path.to_path_buf();
    let text = input.text.clone();
    let protected_entities = input.protected_entities.clone();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|error| format!("Failed to open DB for local NER fallback: {error}"))?;
        let engine = crate::llm::get_or_init_local_gemma_engine(&conn, &db_path, &app_handle)?;
        let max_tokens = 1024;
        let engine = engine
            .lock()
            .map_err(|error| format!("Local Gemma engine lock poisoned: {error}"))?;
        let truncated = crate::llm::truncate_text_for_context(engine.n_ctx(), max_tokens, &text);
        let prompt = crate::llm::prompt::extract_entities(&truncated);
        let raw = engine.generate(&prompt, max_tokens, "[nlp/ner][local]")?;
        ner::openrouter::parse_openrouter_entities(
            &text,
            &protected_entities,
            &raw,
            crate::llm::MODEL_FILENAME,
        )
    })
    .await
    .map_err(|error| format!("Local Gemma NER task panicked: {error}"))?
}

fn asset_embedding_exists(conn: &rusqlite::Connection, asset_id: &str) -> Result<bool, String> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM vec_assets WHERE asset_id = ?1 LIMIT 1",
            rusqlite::params![asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to verify persisted asset embedding: {e}"))?;

    Ok(found.is_some())
}

fn run_coalesced_fts_reindex(
    conn: &rusqlite::Connection,
    item_id: &str,
    fts_pending: &Arc<Mutex<HashMap<String, bool>>>,
) -> Result<(), String> {
    loop {
        eprintln!("[nlp/fts] Reindex start: item_id={item_id}");
        if let Err(error) = tokio::task::block_in_place(|| fts::index_item_from_db(conn, item_id)) {
            if let Ok(mut pending) = fts_pending.lock() {
                pending.remove(item_id);
            }
            return Err(error);
        }

        let should_rerun = match fts_pending.lock() {
            Ok(mut pending) => match pending.get_mut(item_id) {
                Some(needs_rerun) if *needs_rerun => {
                    *needs_rerun = false;
                    true
                }
                Some(_) => {
                    pending.remove(item_id);
                    false
                }
                None => false,
            },
            Err(_) => false,
        };

        if should_rerun {
            eprintln!(
                "[nlp/fts] Reindex rerun requested while busy: item_id={item_id} — processing latest state"
            );
            continue;
        }

        return Ok(());
    }
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
        rusqlite::params![table],
        |_| Ok(()),
    )
    .is_ok()
}

fn column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("Failed to inspect {table}: {e}"))?;

    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("Failed to read {table} columns: {e}"))?;

    for existing in columns {
        if existing.map_err(|e| format!("Failed to decode column name: {e}"))? == column {
            return Ok(true);
        }
    }

    Ok(false)
}

fn ensure_entities_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    if !column_exists(conn, "entities", "source")? {
        conn.execute("ALTER TABLE entities ADD COLUMN source TEXT", [])
            .map_err(|e| format!("Failed to add entities.source: {e}"))?;
    }

    if !column_exists(conn, "entities", "model_name")? {
        conn.execute("ALTER TABLE entities ADD COLUMN model_name TEXT", [])
            .map_err(|e| format!("Failed to add entities.model_name: {e}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use rusqlite::{params, Connection};
    use std::cell::RefCell;

    // ── Event payload contract ────────────────────────────────────────────────
    // These pin the JSON shape the frontend (lib/nlp.ts) routes on. Asset-scoped
    // jobs MUST carry asset_id so per-asset chips advance; NER completion MUST
    // carry entity_count. Regression guard for the asset_id/entity_count drift.

    #[test]
    fn complete_payload_carries_asset_id_and_entity_count_for_asset_jobs() {
        let payload = NlpCompletePayload {
            item_id: "item-1".to_string(),
            asset_id: Some("asset-9".to_string()),
            job: "ner".to_string(),
            entity_count: Some(7),
        };
        let value = serde_json::to_value(&payload).expect("payload serializes");
        assert_eq!(value["item_id"], "item-1");
        assert_eq!(value["asset_id"], "asset-9");
        assert_eq!(value["job"], "ner");
        assert_eq!(value["entity_count"], 7);
    }

    #[test]
    fn complete_payload_omits_asset_id_and_entity_count_for_item_jobs() {
        let payload = NlpCompletePayload {
            item_id: "item-1".to_string(),
            asset_id: None,
            job: "fts".to_string(),
            entity_count: None,
        };
        let value = serde_json::to_value(&payload).expect("payload serializes");
        let obj = value.as_object().expect("payload is a JSON object");
        assert!(
            !obj.contains_key("asset_id"),
            "item-level complete must omit asset_id, got: {value}"
        );
        assert!(
            !obj.contains_key("entity_count"),
            "non-NER complete must omit entity_count, got: {value}"
        );
    }

    #[test]
    fn progress_payload_carries_asset_id_when_present() {
        let payload = NlpProgressPayload {
            item_id: "item-1".to_string(),
            asset_id: Some("asset-9".to_string()),
            job: "embed".to_string(),
            pct: 100,
        };
        let value = serde_json::to_value(&payload).expect("payload serializes");
        assert_eq!(value["asset_id"], "asset-9");
        assert_eq!(value["pct"], 100);
    }

    #[test]
    fn error_payload_carries_asset_id_when_present() {
        let payload = NlpErrorPayload {
            item_id: "item-1".to_string(),
            asset_id: Some("asset-9".to_string()),
            job: "ner".to_string(),
            error: "boom".to_string(),
        };
        let value = serde_json::to_value(&payload).expect("payload serializes");
        assert_eq!(value["asset_id"], "asset-9");
        assert_eq!(value["error"], "boom");
    }

    #[test]
    fn submit_coalesces_duplicate_fts_jobs_while_pending() {
        let (queue, mut receiver) = NlpQueue::new();

        queue
            .submit(NlpJob::IndexFts {
                item_id: "item-dup".to_string(),
            })
            .expect("first enqueue should succeed");
        queue
            .submit(NlpJob::IndexFts {
                item_id: "item-dup".to_string(),
            })
            .expect("duplicate enqueue should coalesce");

        let first = receiver.try_recv().expect("one FTS job should be queued");
        assert!(matches!(first, NlpJob::IndexFts { ref item_id } if item_id == "item-dup"));
        assert!(
            receiver.try_recv().is_err(),
            "duplicate should not queue a second FTS job"
        );
        assert_eq!(
            queue
                .fts_pending
                .lock()
                .expect("fts pending lock")
                .get("item-dup")
                .copied(),
            Some(true),
            "duplicate enqueue should mark the item for one rerun"
        );
    }

    #[test]
    fn submit_coalesces_duplicate_asset_ner_jobs_while_pending() {
        let (queue, mut receiver) = NlpQueue::new();

        queue
            .submit(NlpJob::ExtractEntitiesForAsset {
                item_id: "item-1".to_string(),
                asset_id: "asset-dup".to_string(),
            })
            .expect("first asset NER enqueue should succeed");
        queue
            .submit(NlpJob::ExtractEntitiesForAsset {
                item_id: "item-1".to_string(),
                asset_id: "asset-dup".to_string(),
            })
            .expect("duplicate asset NER enqueue should coalesce");

        let first = receiver
            .try_recv()
            .expect("one asset NER job should be queued");
        assert!(
            matches!(first, NlpJob::ExtractEntitiesForAsset { ref asset_id, .. } if asset_id == "asset-dup")
        );
        assert!(
            receiver.try_recv().is_err(),
            "duplicate should not queue a second asset NER job"
        );
        assert!(
            queue
                .asset_ner_pending
                .lock()
                .expect("asset ner pending lock")
                .contains("asset-dup"),
            "duplicate asset NER should keep one pending marker"
        );
    }

    #[test]
    fn submit_coalesces_duplicate_asset_embedding_jobs_while_pending() {
        let (queue, mut receiver) = NlpQueue::new();

        queue
            .submit(NlpJob::ComputeAssetEmbedding {
                item_id: "item-1".to_string(),
                asset_id: "asset-dup".to_string(),
            })
            .expect("first embedding enqueue should succeed");
        queue
            .submit(NlpJob::ComputeAssetEmbedding {
                item_id: "item-1".to_string(),
                asset_id: "asset-dup".to_string(),
            })
            .expect("duplicate embedding enqueue should coalesce");

        let first = receiver
            .try_recv()
            .expect("one embedding job should be queued");
        assert!(
            matches!(first, NlpJob::ComputeAssetEmbedding { ref asset_id, .. } if asset_id == "asset-dup")
        );
        assert!(
            receiver.try_recv().is_err(),
            "duplicate should not queue a second embedding job"
        );
        assert!(
            queue
                .embedding_pending
                .lock()
                .expect("embedding pending lock")
                .contains("asset-dup"),
            "duplicate embedding should keep one pending marker"
        );
    }

    fn run_job_without_events(conn: &Connection, job: &NlpJob) -> Result<(), String> {
        match job {
            NlpJob::IndexFts { item_id } => fts::index_item_from_db(conn, item_id),
            NlpJob::ExtractEntities { .. } => Ok(()),
            NlpJob::ComputeAssetEmbedding { item_id, asset_id } => {
                // No engine in test context → graceful degradation
                embeddings::compute_and_store_for_asset(None, conn, item_id, asset_id)
            }
            NlpJob::ExtractEntitiesForAsset { .. } => Ok(()),
            NlpJob::EnrichItem { item_id } => {
                // Unit-test the local part of EnrichItem without making remote OpenRouter calls.
                fts::index_item_from_db(conn, item_id)
            }
        }
    }

    fn setup_worker_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");

        conn.execute_batch(
            r#"
            CREATE TABLE items (
              id TEXT PRIMARY KEY,
              collection_id TEXT,
              title TEXT NOT NULL,
              metadata TEXT
            );

            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              sort_index INTEGER NOT NULL DEFAULT 0,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT NOT NULL,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE entities (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              entity_type TEXT NOT NULL,
              value TEXT NOT NULL,
              start_offset INTEGER NOT NULL,
              end_offset INTEGER NOT NULL,
              confidence REAL NOT NULL,
              source TEXT,
              model_name TEXT,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE triples (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              subject TEXT NOT NULL,
              predicate TEXT NOT NULL,
              object TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );

            CREATE VIRTUAL TABLE fts_items USING fts5(
              item_id UNINDEXED,
              title,
              metadata,
              extracted_text,
              content = ''
            );
            "#,
        )
        .expect("nlp worker schema should be created");

        ensure_entities_schema(&conn).expect("entities schema migration should succeed");

        conn
    }

    fn seed_item(conn: &Connection, item_id: &str, asset_id: &str, title: &str, text: &str) {
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![item_id, "col-1", title, "{}"],
        )
        .expect("item should be inserted");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![asset_id, item_id, "asset.txt", "txt", 0_i64, 1_i64],
        )
        .expect("asset should be inserted");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![format!("ext-{item_id}"), asset_id, text, 2_i64],
        )
        .expect("extraction should be inserted");
    }

    // ── EnrichItem integration tests ──────────────────────────────────────────

    #[test]
    fn enrich_item_runs_remaining_item_level_jobs() {
        let conn = setup_worker_test_db();
        seed_item(
            &conn,
            "item-enrich",
            "asset-enrich",
            "Acta Colonial",
            "Don Manuel Belgrano creó la Bandera en la ciudad de Buenos Aires.",
        );

        let result = run_job_without_events(
            &conn,
            &NlpJob::EnrichItem {
                item_id: "item-enrich".to_string(),
            },
        );
        assert!(
            result.is_ok(),
            "EnrichItem should succeed for remaining item-level jobs"
        );

        // FTS should have indexed the item
        let fts_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM fts_items", [], |row| row.get(0))
            .expect("fts count should be queryable");
        assert_eq!(fts_rows, 1, "FTS should index the item");

        let entity_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entities WHERE item_id = ?1",
                params!["item-enrich"],
                |row| row.get(0),
            )
            .expect("entity count should be queryable");
        assert_eq!(entity_rows, 0, "test helper should not call remote NER");
    }

    #[test]
    fn enrich_item_continues_after_sub_job_failure() {
        // Run EnrichItem on an item — remaining item-level sub-jobs should still complete.
        let conn = setup_worker_test_db();
        seed_item(
            &conn,
            "item-partial",
            "asset-partial",
            "Acta Colonial",
            "Don Manuel Belgrano creó la Bandera en la ciudad de Buenos Aires.",
        );

        // Run EnrichItem — remaining item-level sub-jobs should still succeed
        let _result = run_job_without_events(
            &conn,
            &NlpJob::EnrichItem {
                item_id: "item-partial".to_string(),
            },
        );

        // FTS should still have indexed
        let fts_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM fts_items", [], |row| row.get(0))
            .expect("fts count should be queryable");
        assert_eq!(
            fts_rows, 1,
            "FTS should still index the item after partial failure"
        );

        let entity_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entities WHERE item_id = ?1",
                params!["item-partial"],
                |row| row.get(0),
            )
            .expect("entity count should be queryable");
        assert_eq!(entity_rows, 0, "test helper should not call remote NER");
    }

    #[test]
    fn enrich_item_handles_item_with_transcription_text() {
        let conn = setup_worker_test_db();

        // Create item and asset with extraction + transcription
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, ?4)",
            params!["item-trans-enrich", "col-1", "Transcription Item", "{}"],
        )
        .expect("item insert");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "asset-trans-enrich",
                "item-trans-enrich",
                "audio.mp3",
                "audio",
                0_i64,
                1_i64
            ],
        )
        .expect("asset insert");

        // Transcription only
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params!["trans-enrich-1", "asset-trans-enrich", "Don San Martín creó el Ejército.", "es", 5000_i64, "base", "[]", 0.9_f64, 10_i64],
        )
        .expect("transcription insert");

        let result = run_job_without_events(
            &conn,
            &NlpJob::EnrichItem {
                item_id: "item-trans-enrich".to_string(),
            },
        );
        assert!(
            result.is_ok(),
            "EnrichItem should complete for transcription-only text"
        );

        // FTS should find the transcription text
        let fts_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM fts_items", [], |row| row.get(0))
            .expect("fts count should be queryable");
        assert_eq!(
            fts_rows, 1,
            "FTS should index the item with transcription text"
        );
    }

    #[test]
    fn nlp_runtime_resolution_bootstraps_before_using_managed_assets() {
        let calls = RefCell::new(Vec::new());
        let expected = PathBuf::from("/tmp/runtime-ready");

        let resolved = managed_runtime_root_for_nlp_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::Healthy,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: true,
                    summary: "Runtime listo".to_string(),
                    blocked_capabilities: vec![],
                    details: vec![],
                    guidance: vec![],
                    bootstrap_eligible: false,
                    bootstrap_required: false,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(expected.clone()))
            },
        )
        .expect("managed runtime should resolve");

        assert_eq!(resolved, Some(expected));
        assert_eq!(calls.into_inner(), vec!["ensure_ready", "hydrated_root"]);
    }

    #[test]
    fn nlp_runtime_resolution_returns_none_when_not_healthy_allowing_dev_fallback() {
        let calls = RefCell::new(Vec::new());

        let resolved = managed_runtime_root_for_nlp_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedOffline,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "Bootstrap offline".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Nlp],
                    details: vec!["offline".to_string()],
                    guidance: vec!["Reintentá".to_string()],
                    bootstrap_eligible: true,
                    bootstrap_required: true,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(PathBuf::from("/tmp/stale-runtime")))
            },
        )
        .expect("non-healthy runtime should not raise transport errors");

        assert_eq!(resolved, None);
        assert_eq!(calls.into_inner(), vec!["ensure_ready"]);
    }

    #[test]
    fn ensure_nlp_runtime_ready_accepts_dev_fallback() {
        // ensure_nlp_runtime_ready should return Ok(()) even when managed runtime is not healthy,
        // because callers fall back to dev paths / system Python.
        let result = managed_runtime_root_for_nlp_with(
            || {
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedSourceUnavailable,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "No hay una fuente confiable disponible para bootstrap".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Nlp],
                    details: vec!["fixture".to_string()],
                    guidance: vec![],
                    bootstrap_eligible: false,
                    bootstrap_required: true,
                    active_operation: None,
                })
            },
            || Ok(None),
        );
        assert!(
            result.is_ok(),
            "ensure_nlp_runtime_ready should accept dev fallback"
        );
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn try_init_embed_engine_returns_none_when_openrouter_key_missing() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        conn.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .expect("settings table should be created");

        let result = try_init_embed_engine(&conn);
        assert!(
            result.is_none(),
            "Engine should be None when OpenRouter API key is missing"
        );
    }

    #[test]
    fn try_init_embed_engine_returns_some_when_openrouter_key_exists() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        // Pro defaults embeddings to local BGE-M3; the API engine is opt-in, so the
        // api provider must be selected explicitly for this OpenRouter-key scenario.
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'api');\
             INSERT INTO app_settings(key, value) VALUES ('openrouter_api_key', 'sk-test');",
        )
        .expect("settings table should be created");

        let result = try_init_embed_engine(&conn);
        assert!(
            result.is_some(),
            "Engine should be Some when OpenRouter API key exists"
        );
    }

    #[test]
    fn embed_engine_retries_after_key_is_configured() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        // Opt into the API embedding provider (Pro defaults to local BGE-M3). With the
        // api provider selected but no key yet, init must fail until the key arrives.
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'api');",
        )
        .expect("settings table should be created");

        let first = try_init_embed_engine(&conn);
        assert!(
            first.is_none(),
            "First init should fail when OpenRouter key is unavailable"
        );

        conn.execute(
            "INSERT INTO app_settings(key, value) VALUES ('openrouter_api_key', 'sk-test')",
            [],
        )
        .expect("setting insert should succeed");

        let second = try_init_embed_engine(&conn);
        assert!(
            second.is_some(),
            "Second init should succeed after OpenRouter key is configured"
        );
    }

    #[test]
    fn try_init_embed_engine_error_remembers_local_missing_asset_details() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp.path().join("entropia.sqlite");
        let conn = Connection::open(&db_path).expect("sqlite file should open");
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'local');",
        )
        .expect("settings table should be created");

        let error = match try_init_embed_engine_result(&conn) {
            Ok(_) => panic!("local provider without required files should fail with diagnostics"),
            Err(error) => error,
        };

        let expected_dir = temp.path().join("models").join("embeddings").join("bge-m3");
        assert!(error.contains("Local BGE-M3 model incomplete"));
        assert!(error.contains(&expected_dir.to_string_lossy().to_string()));
        assert!(error.contains("model.onnx"));
        assert!(error.contains("model.onnx_data"));
        assert!(error.contains("tokenizer.json"));
        assert!(error.contains("Install BGE-M3 from Settings"));
        assert!(error.contains("configured BGE-M3 provider"));
    }

    #[test]
    fn cached_embed_engine_reinitializes_when_settings_change() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        // Opt into the API provider (Pro defaults to local BGE-M3) to exercise the
        // cache-invalidation path without needing local model files on disk.
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'api');\
             INSERT INTO app_settings(key, value) VALUES ('openrouter_api_key', 'sk-test');",
        )
        .expect("settings table should be created");
        let mut cached = None;
        let mut last_error = None;

        let first = ensure_embed_engine_for_current_settings(&conn, &mut cached, &mut last_error)
            .expect("api engine should initialize");
        assert_eq!(first.provider_name(), "api");

        conn.execute(
            "INSERT INTO app_settings(key, value) VALUES ('openrouter_embedding_model', 'custom/model')",
            [],
        )
        .expect("setting insert should succeed");

        let second = ensure_embed_engine_for_current_settings(&conn, &mut cached, &mut last_error)
            .expect("api engine should reinitialize after model change");

        assert!(
            !Arc::ptr_eq(&first, &second),
            "embedding engine cache must be invalidated when settings change"
        );
        assert!(last_error.is_none());
    }

    #[test]
    fn cached_embed_engine_does_not_use_stale_api_after_switching_to_invalid_local() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp.path().join("entropia.sqlite");
        let conn = Connection::open(&db_path).expect("sqlite file should open");
        // Start on the opt-in API provider (Pro defaults to local BGE-M3), then switch
        // to an invalid local config and assert the stale API engine is not reused.
        conn.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'api');\
             INSERT INTO app_settings(key, value) VALUES ('openrouter_api_key', 'sk-test');",
        )
        .expect("settings table should be created");
        let mut cached = None;
        let mut last_error = None;

        let first = ensure_embed_engine_for_current_settings(&conn, &mut cached, &mut last_error)
            .expect("api engine should initialize");
        assert_eq!(first.provider_name(), "api");

        conn.execute(
            "INSERT OR REPLACE INTO app_settings(key, value) VALUES ('embedding_provider', 'local')",
            [],
        )
        .expect("provider update should succeed");

        let switched =
            ensure_embed_engine_for_current_settings(&conn, &mut cached, &mut last_error);

        assert!(
            switched.is_none(),
            "invalid local settings must not silently keep using stale API engine"
        );
        assert!(cached.is_none());
        let error = last_error.expect("local init error should be recorded");
        assert!(error.contains("Local BGE-M3 model incomplete"));
    }
}
