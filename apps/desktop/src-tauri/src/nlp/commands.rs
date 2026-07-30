/// Tauri IPC commands for NLP operations.
///
/// Each command pushes a job to the `NlpQueue` and returns immediately with
/// `Ok("queued")`. The worker emits `nlp:progress`, `nlp:complete`, or
/// `nlp:error` events asynchronously.
use super::vector::{cosine_distance, decode_embedding_blob};
use super::{NlpJob, NlpQueue};
use crate::db::state::AppDbState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

type SimilarAssetRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Vec<u8>,
);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetEmbeddingBackfillFailure {
    pub asset_id: String,
    pub item_id: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetEmbeddingBackfillReport {
    pub force: bool,
    pub limit: Option<usize>,
    pub total_assets: i64,
    pub assets_with_text: i64,
    pub assets_with_embedding: i64,
    pub assets_missing_embedding: i64,
    pub requested: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub failures: Vec<AssetEmbeddingBackfillFailure>,
}

fn enqueue(nlp_queue: &NlpQueue, job: NlpJob) -> Result<String, String> {
    nlp_queue.submit(job)?;
    Ok("queued".to_string())
}

fn triples_retired_error(target: &str) -> String {
    format!(
        "NLP triples extraction was retired for {target}. Use the LLM triples commands instead."
    )
}

fn local_embedding_model_dir(db: &AppDbState) -> std::path::PathBuf {
    let app_data_dir = db.db_path.parent();
    let configured = db.ui_conn.lock().ok().and_then(|conn| {
        crate::settings::get_setting(
            &conn,
            super::embeddings::LOCAL_EMBEDDING_MODEL_DIR_SETTING_KEY,
        )
    });
    super::embeddings::resolve_local_embedding_model_dir(configured.as_deref(), app_data_dir)
}

#[tauri::command]
pub async fn embedding_local_model_info(
    db: State<'_, AppDbState>,
) -> Result<super::embeddings::LocalEmbeddingModelInfo, String> {
    Ok(super::embeddings::get_local_embedding_model_info(Some(
        local_embedding_model_dir(&db),
    )))
}

#[tauri::command]
pub async fn embedding_open_models_dir(db: State<'_, AppDbState>) -> Result<(), String> {
    let models_dir = local_embedding_model_dir(&db);
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create local BGE-M3 model dir: {e}"))?;

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&models_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&models_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&models_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn embedding_download_model(
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let model_dir = local_embedding_model_dir(&db);
    crate::app_logs::info(
        &app_handle,
        "embedding/download",
        format!(
            "Inicio instalación de BGE-M3 local en {}",
            model_dir.display()
        ),
    );

    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            super::embeddings::download_local_embedding_model_files(&model_dir, &app_handle_clone)
        })
        .await;

        match result {
            Ok(Ok(())) => {
                crate::app_logs::info(
                    &app_handle,
                    "embedding/download",
                    "Instalación BGE-M3 completada",
                );
            }
            Ok(Err(error)) => {
                crate::app_logs::error(
                    &app_handle,
                    "embedding/download",
                    format!("Instalación BGE-M3 falló: {error}"),
                );
                let _ = app_handle.emit(
                    "embedding:download_error",
                    super::embeddings::EmbeddingDownloadErrorPayload { error },
                );
            }
            Err(error) => {
                let message = format!("BGE-M3 download task panicked: {error}");
                crate::app_logs::error(&app_handle, "embedding/download", message.clone());
                let _ = app_handle.emit(
                    "embedding:download_error",
                    super::embeddings::EmbeddingDownloadErrorPayload { error: message },
                );
            }
        }
    });

    Ok("started".to_string())
}

/// Submit an FTS5 indexing job for `item_id`.
///
/// The worker will fetch the item's title + extracted text and upsert into
/// the `fts_items` virtual table.
#[tauri::command]
pub async fn index_fts(item_id: String, nlp_queue: State<'_, NlpQueue>) -> Result<String, String> {
    enqueue(&nlp_queue, NlpJob::IndexFts { item_id })
}

/// Submit a NER extraction job for `item_id`.
///
/// Uses the shared dedup gate so repeated manual requests do not overlap.
#[tauri::command]
pub async fn extract_entities(
    item_id: String,
    app_handle: AppHandle,
    nlp_queue: State<'_, NlpQueue>,
) -> Result<String, String> {
    super::ensure_nlp_runtime_ready(&app_handle)?;
    super::enqueue_entity_refresh_for_item(&nlp_queue, &item_id).map(|_| "queued".to_string())
}

/// Submit a semantic triples extraction job for `item_id`.
#[tauri::command]
pub async fn extract_triples(
    item_id: String,
    _nlp_queue: State<'_, NlpQueue>,
) -> Result<String, String> {
    Err(triples_retired_error(&format!("item '{item_id}'")))
}

/// Submit the item-level automatic enrichment job (FTS only) for `item_id`.
/// NER and semantic triples are intentionally manual-only.
#[tauri::command]
pub async fn enrich_item(
    item_id: String,
    nlp_queue: State<'_, NlpQueue>,
) -> Result<String, String> {
    enqueue(&nlp_queue, NlpJob::IndexFts { item_id })
}

// ── Asset-level commands ────────────────────────────────────────────────────
// These process only the selected asset/page text, not the entire item.
// Results are stored with both item_id (ownership) and asset_id (filtering).

/// Submit an embedding computation job for a specific asset.
///
/// The worker will extract the asset's text, compute a 1024-dim BGE-M3 vector via the
/// configured embedding provider, and upsert into `vec_assets` keyed by `asset_id`.
#[tauri::command]
pub async fn embed_asset(
    item_id: String,
    asset_id: String,
    _app_handle: AppHandle,
    nlp_queue: State<'_, NlpQueue>,
) -> Result<String, String> {
    enqueue(
        &nlp_queue,
        NlpJob::ComputeAssetEmbedding { item_id, asset_id },
    )
}

/// Batch backfill asset-level embeddings into `vec_assets`.
///
/// Walks every asset that already has OCR/transcription text and persists
/// embeddings keyed by `asset_id`. By default it skips assets that already
/// have a `vec_assets` row; pass `force=true` to recompute them.
#[tauri::command]
pub async fn backfill_asset_embeddings(
    force: Option<bool>,
    limit: Option<usize>,
    app_handle: tauri::AppHandle,
) -> Result<AssetEmbeddingBackfillReport, String> {
    let force = force.unwrap_or(false);
    let limit = limit.filter(|value| *value > 0);

    tokio::task::spawn_blocking(move || {
        use tauri::Manager;

        let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
            format!("Failed to resolve app data dir for asset embedding backfill: {e}")
        })?;
        let db_path = app_data_dir.join("entropia.sqlite");

        let conn = rusqlite::Connection::open(&db_path).map_err(|e| {
            format!("Failed to open SQLite database for asset embedding backfill: {e}")
        })?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;\
              CREATE TABLE IF NOT EXISTS vec_assets(\
                  asset_id TEXT PRIMARY KEY,\
                  item_id TEXT NOT NULL,\
                  embedding BLOB NOT NULL,\
                  embedding_model TEXT NOT NULL DEFAULT 'legacy',\
                  embedding_contract TEXT NOT NULL DEFAULT 'legacy',\
                  dimensions INTEGER NOT NULL DEFAULT 0\
              );\
             CREATE INDEX IF NOT EXISTS idx_vec_assets_item_id ON vec_assets(item_id);",
        )
        .map_err(|e| format!("Failed to ensure embedding tables for backfill: {e}"))?;

        let coverage = super::embeddings::summarize_asset_embedding_coverage(&conn)?;
        let candidates = super::embeddings::list_asset_embedding_candidates(&conn, force, limit)?;

        eprintln!(
            "[nlp/embeddings] EMBED backfill scan force={force} limit={limit:?} total_assets={} assets_with_text={} assets_with_embedding={} assets_missing_embedding={} candidates={}",
            coverage.total_assets,
            coverage.assets_with_text,
            coverage.assets_with_embedding,
            coverage.assets_missing_embedding,
            candidates.len()
        );

        if candidates.is_empty() {
            eprintln!(
                "[nlp/embeddings] EMBED backfill skipped: no candidates (use force=true to recompute existing embeddings)"
            );
            return Ok(AssetEmbeddingBackfillReport {
                force,
                limit,
                total_assets: coverage.total_assets,
                assets_with_text: coverage.assets_with_text,
                assets_with_embedding: coverage.assets_with_embedding,
                assets_missing_embedding: coverage.assets_missing_embedding,
                requested: 0,
                succeeded: 0,
                failed: 0,
                failures: Vec::new(),
            });
        }

        let engine = super::embeddings::EmbeddingEngine::init(
            super::embeddings::config_from_settings(&conn)?,
        )?;
        eprintln!(
            "[nlp/embeddings] EMBED backfill using provider={} requested={}",
            engine.provider_name(),
            candidates.len()
        );

        let mut succeeded = 0_usize;
        let mut failures = Vec::new();

        for candidate in candidates.iter() {
            match super::embeddings::compute_and_store_for_asset(
                Some(&engine),
                &conn,
                &candidate.item_id,
                &candidate.asset_id,
            ) {
                Ok(()) => succeeded += 1,
                Err(error) => failures.push(AssetEmbeddingBackfillFailure {
                    asset_id: candidate.asset_id.clone(),
                    item_id: candidate.item_id.clone(),
                    error,
                }),
            }
        }

        let updated_coverage = super::embeddings::summarize_asset_embedding_coverage(&conn)?;
        eprintln!(
            "[nlp/embeddings] EMBED backfill complete provider={} requested={} succeeded={} failed={} assets_with_embedding={} assets_missing_embedding={}",
            engine.provider_name(),
            candidates.len(),
            succeeded,
            failures.len(),
            updated_coverage.assets_with_embedding,
            updated_coverage.assets_missing_embedding
        );

        Ok(AssetEmbeddingBackfillReport {
            force,
            limit,
            total_assets: updated_coverage.total_assets,
            assets_with_text: updated_coverage.assets_with_text,
            assets_with_embedding: updated_coverage.assets_with_embedding,
            assets_missing_embedding: updated_coverage.assets_missing_embedding,
            requested: candidates.len(),
            succeeded,
            failed: failures.len(),
            failures,
        })
    })
    .await
    .map_err(|e| format!("Asset embedding backfill task panicked: {e}"))?
}

/// Submit a NER extraction job for a specific asset.
///
/// The worker will extract entities from only the selected asset's text.
/// Entities are stored with both `item_id` and `asset_id` for filtering.
#[tauri::command]
pub async fn extract_entities_for_asset(
    item_id: String,
    asset_id: String,
    app_handle: AppHandle,
    nlp_queue: State<'_, NlpQueue>,
) -> Result<String, String> {
    super::ensure_nlp_runtime_ready(&app_handle)?;
    enqueue(
        &nlp_queue,
        NlpJob::ExtractEntitiesForAsset { item_id, asset_id },
    )
}

/// Submit a semantic triples extraction job for a specific asset.
///
/// Semantic triples are Gemma-only. This legacy NLP endpoint is intentionally disabled
/// to prevent rule-based output from overwriting LLM triples in the shared table.
#[tauri::command]
pub async fn extract_triples_for_asset(
    item_id: String,
    asset_id: String,
    _nlp_queue: State<'_, NlpQueue>,
) -> Result<String, String> {
    Err(triples_retired_error(&format!(
        "asset '{asset_id}' (item '{item_id}')"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedding_blob(values: [f32; 2]) -> Vec<u8> {
        values.into_iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn setup_similar_assets_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db should open");
        conn.execute_batch(
            r#"
            CREATE TABLE items (
                id TEXT PRIMARY KEY,
                collection_id TEXT,
                title TEXT NOT NULL
            );

            CREATE TABLE assets (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                path TEXT NOT NULL,
                type TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE vec_assets (
                asset_id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                embedding BLOB NOT NULL
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
                created_at INTEGER NOT NULL
            );
            "#,
        )
        .expect("test schema should be created");

        conn
    }

    #[test]
    fn enqueue_accepts_index_job_when_queue_has_capacity() {
        let (queue, _receiver) = NlpQueue::new();
        let result = enqueue(
            &queue,
            NlpJob::IndexFts {
                item_id: "item-1".to_string(),
            },
        );

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "queued");
    }

    #[test]
    fn enqueue_propagates_queue_error_when_channel_is_full() {
        let (queue, _receiver) = NlpQueue::new();

        for i in 0..64 {
            let ok = enqueue(
                &queue,
                NlpJob::IndexFts {
                    item_id: format!("item-{i}"),
                },
            );
            assert!(ok.is_ok());
        }

        let result = enqueue(
            &queue,
            NlpJob::IndexFts {
                item_id: "overflow".to_string(),
            },
        );

        assert!(result.is_err());
        assert!(
            result.err().unwrap().contains("Failed to enqueue NLP job"),
            "Expected queue error to be propagated"
        );
    }

    #[test]
    fn enqueue_keeps_non_embedding_jobs_stable() {
        let (queue, _receiver) = NlpQueue::new();

        let fts = enqueue(
            &queue,
            NlpJob::IndexFts {
                item_id: "item-fts".to_string(),
            },
        );
        let ner = enqueue(
            &queue,
            NlpJob::ExtractEntities {
                item_id: "item-ner".to_string(),
            },
        );
        assert_eq!(fts.unwrap(), "queued");
        assert_eq!(ner.unwrap(), "queued");
    }

    #[test]
    fn triples_retired_error_points_callers_to_llm() {
        let message = triples_retired_error("item 'item-7'");

        assert!(message.contains("retired"));
        assert!(message.contains("LLM triples commands"));
    }

    #[test]
    fn cosine_distance_identical_vectors_is_zero() {
        let a = vec![1.0_f32, 2.0_f32, 3.0_f32];
        let b = vec![1.0_f32, 2.0_f32, 3.0_f32];
        let dist = super::cosine_distance(&a, &b).unwrap();
        assert!(
            (dist - 0.0).abs() < 1e-10,
            "identical vectors should have distance 0"
        );
    }

    #[test]
    fn cosine_distance_opposite_vectors_is_two() {
        let a = vec![1.0_f32, 0.0_f32, 0.0_f32];
        let b = vec![-1.0_f32, 0.0_f32, 0.0_f32];
        let dist = super::cosine_distance(&a, &b).unwrap();
        assert!(
            (dist - 2.0).abs() < 1e-10,
            "opposite vectors should have distance 2"
        );
    }

    #[test]
    fn cosine_distance_orthogonal_vectors_is_one() {
        let a = vec![1.0_f32, 0.0_f32, 0.0_f32];
        let b = vec![0.0_f32, 1.0_f32, 0.0_f32];
        let dist = super::cosine_distance(&a, &b).unwrap();
        assert!(
            (dist - 1.0).abs() < 1e-10,
            "orthogonal vectors should have distance 1"
        );
    }

    #[test]
    fn cosine_distance_different_lengths_returns_none() {
        let a = vec![1.0_f32, 2.0_f32];
        let b = vec![1.0_f32, 2.0_f32, 3.0_f32];
        assert!(super::cosine_distance(&a, &b).is_none());
    }

    #[test]
    fn cosine_distance_empty_vectors_returns_none() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        assert!(super::cosine_distance(&a, &b).is_none());
    }

    #[test]
    fn cosine_distance_zero_magnitude_returns_none() {
        let a = vec![0.0_f32, 0.0_f32, 0.0_f32];
        let b = vec![1.0_f32, 2.0_f32, 3.0_f32];
        assert!(super::cosine_distance(&a, &b).is_none());
    }

    #[test]
    fn cosine_distance_proportional_vectors_is_zero() {
        let a = vec![1.0_f32, 2.0_f32, 3.0_f32];
        let b = vec![2.0_f32, 4.0_f32, 6.0_f32];
        let dist = super::cosine_distance(&a, &b).unwrap();
        assert!(
            (dist - 0.0).abs() < 1e-10,
            "proportional vectors should have distance 0"
        );
    }

    #[test]
    fn floats_to_blob_round_trips_for_similarity_search() {
        let original = vec![0.5_f32, -0.3_f32, 0.8_f32, 1.0_f32];
        let blob: Vec<u8> = original.iter().flat_map(|f| f.to_le_bytes()).collect();
        let recovered: Vec<f32> = blob
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(
            recovered, original,
            "blob round-trip should preserve values"
        );
    }

    #[test]
    fn truncate_preview_caps_length_on_char_boundary() {
        let short = "hola";
        assert_eq!(super::truncate_preview(short, 500), "hola");

        let long: String = "á".repeat(600);
        let truncated = super::truncate_preview(&long, super::PREVIEW_MAX_CHARS);
        assert_eq!(
            truncated.chars().count(),
            super::PREVIEW_MAX_CHARS,
            "preview should be capped to PREVIEW_MAX_CHARS characters"
        );
        assert!(
            truncated.is_char_boundary(truncated.len()),
            "truncation must land on a char boundary"
        );
    }

    #[test]
    fn rank_similar_asset_rows_keeps_asset_context() {
        let target_blob = embedding_blob([1.0_f32, 0.0_f32]);
        let same_item_blob = embedding_blob([0.85_f32, 0.15_f32]);
        let other_collection_blob = embedding_blob([0.95_f32, 0.05_f32]);

        let results = super::rank_similar_asset_rows(
            &target_blob,
            vec![
                (
                    "asset-same".to_string(),
                    "item-same".to_string(),
                    "Página hermana".to_string(),
                    "col-1".to_string(),
                    "same.pdf".to_string(),
                    "pdf".to_string(),
                    "Texto OCR de la hermana".to_string(),
                    same_item_blob,
                ),
                (
                    "asset-other".to_string(),
                    "item-other".to_string(),
                    "Página externa".to_string(),
                    "col-2".to_string(),
                    "other.wav".to_string(),
                    "audio".to_string(),
                    "Transcripción externa".to_string(),
                    other_collection_blob,
                ),
            ],
            5,
        )
        .expect("ranking should succeed");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["assetId"], "asset-other");
        assert_eq!(results[0]["assetPath"], "other.wav");
        assert_eq!(results[0]["assetType"], "audio");
        assert_eq!(results[0]["textPreview"], "Transcripción externa");
        assert_eq!(results[1]["assetId"], "asset-same");
        assert_eq!(results[1]["textPreview"], "Texto OCR de la hermana");
    }

    #[test]
    fn similar_assets_queries_across_collections_and_excludes_self() {
        let conn = setup_similar_assets_test_db();

        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            rusqlite::params!["item-target", "col-1", "Documento base"],
        )
        .expect("target item should insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            rusqlite::params!["item-same", "col-1", "Página hermana"],
        )
        .expect("same item should insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            rusqlite::params!["item-other", "col-2", "Página externa"],
        )
        .expect("other item should insert");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["asset-target", "item-target", "target.pdf", "pdf", 1_i64],
        )
        .expect("target asset should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["asset-same", "item-same", "same.pdf", "pdf", 2_i64],
        )
        .expect("same asset should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["asset-other", "item-other", "other.wav", "audio", 3_i64],
        )
        .expect("other asset should insert");

        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "asset-target",
                "item-target",
                embedding_blob([1.0_f32, 0.0_f32])
            ],
        )
        .expect("target embedding should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "asset-same",
                "item-same",
                embedding_blob([0.8_f32, 0.2_f32])
            ],
        )
        .expect("same asset embedding should insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "asset-other",
                "item-other",
                embedding_blob([0.95_f32, 0.05_f32])
            ],
        )
        .expect("other asset embedding should insert");

        // Seed OCR text for the sibling asset and transcription text for the
        // cross-collection asset so the preview covers both source tables.
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["ext-same", "asset-same", "Texto OCR de la página hermana", 1_i64],
        )
        .expect("same asset extraction should insert");
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["tr-other", "asset-other", "Transcripción del audio externo", 1_i64],
        )
        .expect("other asset transcription should insert");

        let results = super::similar_assets_from_conn(&conn, "asset-target", 5)
            .expect("similar assets query should succeed");
        let results = results.as_array().expect("result should be a JSON array");

        assert_eq!(results.len(), 2, "should exclude the target asset itself");
        assert_eq!(results[0]["assetId"], "asset-other");
        assert_eq!(results[0]["collectionId"], "col-2");
        assert_eq!(
            results[0]["textPreview"], "Transcripción del audio externo",
            "transcription text should surface as the preview"
        );
        assert_eq!(results[1]["assetId"], "asset-same");
        assert_eq!(results[1]["collectionId"], "col-1");
        assert_eq!(
            results[1]["textPreview"], "Texto OCR de la página hermana",
            "extraction text should surface as the preview"
        );
    }
}

/// Search `fts_items` using full-text search.
///
/// Returns a JSON array of `{ item_id, title, rank }` objects, ordered by
/// BM25 relevance. `query` is sanitized internally.
///
/// # Arguments
/// * `query`         — user search query (will be sanitized)
/// * `collection_id` — optional UUID to scope results to a single collection
#[tauri::command]
pub async fn fts_search(
    query: String,
    collection_id: Option<String>,
    nlp_queue: State<'_, NlpQueue>,
) -> Result<serde_json::Value, String> {
    // FTS search runs on the worker connection via a blocking task.
    // We use the NlpQueue's channel to serialize DB access — here we send
    // a search that returns results synchronously via a oneshot channel.
    //
    // For simplicity in MVP: use the db::commands::db_select pathway instead
    // of duplicating the connection. The frontend calls db_select directly for
    // search — this command is provided for completeness and future use.
    let _ = nlp_queue; // state injected but search uses db_select from frontend
    let sanitized = crate::nlp::fts::sanitize_fts5_query(&query);
    Ok(serde_json::json!({
        "query": sanitized,
        "collection_id": collection_id,
        "note": "Use db_select with FTS5 MATCH for direct search"
    }))
}

/// Find assets similar to `asset_id` using asset-level kNN vector search.
/// Returns up to `limit` (default 5) similar assets ordered by cosine similarity.
#[tauri::command]
pub async fn similar_assets(
    asset_id: String,
    limit: Option<u8>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<serde_json::Value, String> {
    let limit = limit.unwrap_or(5) as usize;
    let conn = db.ui_conn.lock().map_err(|e| e.to_string())?;

    similar_assets_from_conn(&conn, &asset_id, limit)
}

fn similar_assets_from_conn(
    conn: &rusqlite::Connection,
    asset_id: &str,
    limit: usize,
) -> Result<serde_json::Value, String> {
    const MAX_CANDIDATES: usize = 2000;

    let target_blob: Vec<u8> = conn
        .query_row(
            "SELECT embedding FROM vec_assets WHERE asset_id = ?1",
            rusqlite::params![asset_id],
            |row| row.get(0),
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                format!("No embedding found for asset '{asset_id}'")
            } else {
                format!("Failed to read embedding for asset '{asset_id}': {e}")
            }
        })?;

    let mut stmt = conn
        .prepare(
            "SELECT v.asset_id, v.item_id, i.title, i.collection_id, a.path, a.type,
                    COALESCE(e.text_content, t.text_content, '') AS text_preview, v.embedding
             FROM vec_assets v
             LEFT JOIN assets a ON a.id = v.asset_id
             LEFT JOIN items i ON i.id = v.item_id
             LEFT JOIN extractions e ON e.asset_id = v.asset_id
             LEFT JOIN transcriptions t ON t.asset_id = v.asset_id
             WHERE v.asset_id != ?1
             LIMIT ?2",
        )
        .map_err(|e| format!("Failed to prepare asset similarity query: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![asset_id, MAX_CANDIDATES as i64], |row| {
            let asset_id: String = row.get(0)?;
            let item_id: String = row.get(1)?;
            let title: Option<String> = row.get(2)?;
            let collection_id: Option<String> = row.get(3)?;
            let asset_path: Option<String> = row.get(4)?;
            let asset_type: Option<String> = row.get(5)?;
            let text_preview: Option<String> = row.get(6)?;
            let blob: Vec<u8> = row.get(7)?;
            Ok((
                asset_id,
                item_id,
                title.unwrap_or_default(),
                collection_id.unwrap_or_default(),
                asset_path.unwrap_or_default(),
                asset_type.unwrap_or_default(),
                text_preview.unwrap_or_default(),
                blob,
            ))
        })
        .map_err(|e| format!("Failed to execute asset similarity query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read asset similarity rows: {e}"))?;

    Ok(serde_json::Value::Array(rank_similar_asset_rows(
        &target_blob,
        rows,
        limit,
    )?))
}

/// Max number of characters surfaced in a similar-asset text preview. The
/// frontend trims to 3 lines, so a short slice keeps the IPC payload small
/// while still giving enough context to preview.
const PREVIEW_MAX_CHARS: usize = 500;

/// Truncate `text` to at most `max_chars` characters on a char boundary so the
/// preview payload stays small without panicking on multibyte text.
fn truncate_preview(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => text[..byte_idx].to_string(),
        None => text.to_string(),
    }
}

fn rank_similar_asset_rows(
    target_blob: &[u8],
    rows: Vec<SimilarAssetRow>,
    limit: usize,
) -> Result<Vec<serde_json::Value>, String> {
    rank_similarity_rows(
        target_blob,
        rows.into_iter()
            .map(
                |(
                    asset_id,
                    item_id,
                    title,
                    collection_id,
                    asset_path,
                    asset_type,
                    text_preview,
                    blob,
                )| {
                    (
                        (
                            asset_id,
                            item_id,
                            title,
                            collection_id,
                            asset_path,
                            asset_type,
                            text_preview,
                        ),
                        blob,
                    )
                },
            )
            .collect(),
        limit,
        |(asset_id, item_id, title, collection_id, asset_path, asset_type, text_preview),
         similarity| {
            serde_json::json!({
                "assetId": asset_id,
                "itemId": item_id,
                "title": title,
                "collectionId": collection_id,
                "assetPath": asset_path,
                "assetType": asset_type,
                "textPreview": truncate_preview(&text_preview, PREVIEW_MAX_CHARS),
                "similarity": similarity,
            })
        },
    )
}

fn rank_similarity_rows<T, F>(
    target_blob: &[u8],
    rows: Vec<(T, Vec<u8>)>,
    limit: usize,
    to_json: F,
) -> Result<Vec<serde_json::Value>, String>
where
    F: Fn(T, f64) -> serde_json::Value,
{
    let target = decode_embedding_blob(target_blob)?;

    let mut results: Vec<(T, f64)> = rows
        .into_iter()
        .filter_map(|(meta, blob)| {
            if blob.len() != target_blob.len() {
                return None;
            }

            let other = decode_embedding_blob(&blob).ok()?;
            let distance = cosine_distance(&target, &other)?;
            Some((meta, 1.0 - distance))
        })
        .collect();

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    Ok(results
        .into_iter()
        .take(limit)
        .map(|(meta, similarity)| to_json(meta, similarity))
        .collect())
}

// `decode_embedding_blob` y `cosine_distance` viven en `nlp::vector` para que
// el pipeline RAG pueda reutilizarlos sin duplicar lógica.
