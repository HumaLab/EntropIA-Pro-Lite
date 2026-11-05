/// Tauri IPC commands for NLP operations.
///
/// Each command pushes a job to the `NlpQueue` and returns immediately with
/// `Ok("queued")`. The worker emits `nlp:progress`, `nlp:complete`, or
/// `nlp:error` events asynchronously.
#[cfg(test)]
use super::vector::cosine_distance;
use super::vector::{cosine_distance_to_blob, decode_embedding_blob};
use super::{NlpJob, NlpQueue};
use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

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

/// The configured local embedding model directory, read on the blocking pool:
/// an async command must not lock `ui_conn` on its own thread (A-05b).
async fn local_embedding_model_dir(db: &AppDbState) -> std::path::PathBuf {
    // The model is cache, not data: it no longer lives beside the database.
    let cache_dir = crate::path_utils::remembered_cache_dir();
    let app_data_dir = cache_dir.as_deref().or_else(|| db.db_path.parent());
    let db = db.clone();
    let configured = run_blocking_db_task(move || {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        Ok(crate::settings::get_raw_setting(
            &conn,
            super::embeddings::LOCAL_EMBEDDING_MODEL_DIR_SETTING_KEY,
        ))
    })
    .await
    .unwrap_or_default();
    super::embeddings::resolve_local_embedding_model_dir(configured.as_deref(), app_data_dir)
}

#[tauri::command]
pub async fn embedding_local_model_info(
    db: State<'_, AppDbState>,
) -> Result<super::embeddings::LocalEmbeddingModelInfo, String> {
    Ok(super::embeddings::get_local_embedding_model_info(Some(
        local_embedding_model_dir(&db).await,
    )))
}

#[tauri::command]
pub async fn embedding_open_models_dir(db: State<'_, AppDbState>) -> Result<(), String> {
    let models_dir = local_embedding_model_dir(&db).await;
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
    let model_dir = local_embedding_model_dir(&db).await;
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
/// Admits durably into the manual system batch and returns immediately with
/// `Ok("queued")`: the supervisor owns execution, publishes the vector plus
/// the full chunk set atomically with the receipt, and emits `nlp:complete`
/// afterwards. A deliberate action, so it never applies the batch rule that
/// skips already-satisfied units at admission — the claim revalidates.
#[tauri::command]
pub async fn embed_asset(
    item_id: String,
    asset_id: String,
    _app_handle: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<String, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || admit_manual_embedding(&db_path, &item_id, &asset_id))
        .await
        .map_err(|e| format!("Embedding admission task failed: {e}"))??;
    Ok("queued".to_string())
}

/// Admits one deliberate embedding action. The claim revalidates freshness,
/// so manual admits skip the eligibility pre-check and let the supervisor
/// decide (already-satisfied units close without calling any motor).
fn admit_manual_embedding(
    db_path: &std::path::Path,
    item_id: &str,
    asset_id: &str,
) -> Result<(), String> {
    use crate::processing::repository;
    let conn = crate::db::open::open_archive_connection(db_path)?;
    if !repository::is_schema_ready(&conn)? {
        return Err(format!(
            "{}: the queue schema is still migrating; retry in a moment",
            repository::SCHEMA_NOT_READY
        ));
    }
    let batch = repository::ensure_system_batch(&conn, "manual")?;
    let revision = conn
        .query_row(
            "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = ?1",
            [asset_id],
            |row| row.get::<_, i64>(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(0),
            other => Err(format!("Failed to read revision of {asset_id}: {other}")),
        })?;
    let fingerprint = crate::processing::eligibility::embedding_input_fingerprint(&conn, asset_id)
        .unwrap_or_default();
    repository::admit_or_attach(
        &conn,
        &batch,
        "embedding",
        asset_id,
        revision,
        &fingerprint,
        &crate::processing::eligibility::resolve_effective_embedding_contract(&conn)?.hash,
        None,
    )?;
    let _ = item_id;
    Ok(())
}

/// Batch backfill asset-level embeddings into `vec_assets`.
///
/// Scans text candidates (including fresh results when forced) and ADMITS
/// replacement work without deleting the published vectors
/// instead of computing inline: the command returns after the admission
/// COMMIT with `requested` admitted, and the supervisor computes, publishes,
/// and reports per-unit errors durably. `succeeded`/`failed` stay zero here
/// by design — progress lives in the batch snapshots, not in this return.
#[tauri::command]
pub async fn backfill_asset_embeddings(
    force: Option<bool>,
    limit: Option<usize>,
    app_handle: tauri::AppHandle,
) -> Result<AssetEmbeddingBackfillReport, String> {
    let force = force.unwrap_or(false);
    let limit = limit.filter(|value| *value > 0);

    tokio::task::spawn_blocking(move || {
        let app_data_dir = crate::path_utils::data_dir(&app_handle).map_err(|e| {
            format!("Failed to resolve app data dir for asset embedding backfill: {e}")
        })?;
        let db_path = app_data_dir.join("entropia.sqlite");

        let conn = crate::db::open::open_archive_connection(&db_path).map_err(|e| {
            format!("Failed to open SQLite database for asset embedding backfill: {e}")
        })?;
        admit_backfill(&conn, force, limit)
    })
    .await
    .map_err(|e| format!("Asset embedding backfill task panicked: {e}"))?
}

pub(crate) fn admit_backfill(
    conn: &rusqlite::Connection,
    force: bool,
    limit: Option<usize>,
) -> Result<AssetEmbeddingBackfillReport, String> {
    use crate::processing::repository;
    if !repository::is_schema_ready(conn)? {
        return Err(format!(
            "{}: the queue schema is still migrating; retry in a moment",
            repository::SCHEMA_NOT_READY
        ));
    }
    let coverage = super::embeddings::summarize_asset_embedding_coverage(conn)?;
    let candidates = super::embeddings::scan_text_assets(conn, limit, !force)?;
    let batch = repository::ensure_system_batch(conn, "manual")?;
    let current_contract =
        crate::processing::eligibility::resolve_effective_embedding_contract(conn)?.hash;
    let contract = if force {
        format!("force:{current_contract}")
    } else {
        current_contract
    };
    let mut requested = 0_usize;
    for candidate in &candidates {
        let revision = conn
            .query_row(
                "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = ?1",
                [&candidate.asset_id],
                |row| row.get::<_, i64>(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(0),
                other => Err(format!(
                    "Failed to read revision of {}: {other}",
                    candidate.asset_id
                )),
            })?;
        let fingerprint =
            crate::processing::eligibility::embedding_input_fingerprint(conn, &candidate.asset_id)?;
        repository::admit_or_attach(
            conn,
            &batch,
            "embedding",
            &candidate.asset_id,
            revision,
            &fingerprint,
            &contract,
            None,
        )?;
        requested += 1;
    }
    log::info!(
        target: "nlp/embeddings",
        "EMBED backfill admitted force={force} limit={limit:?} requested={requested}"
    );
    Ok(AssetEmbeddingBackfillReport {
        force,
        limit,
        total_assets: coverage.total_assets,
        assets_with_text: coverage.assets_with_text,
        assets_with_embedding: coverage.assets_with_embedding,
        assets_missing_embedding: coverage.assets_missing_embedding,
        requested,
        succeeded: 0,
        failed: 0,
        failures: Vec::new(),
    })
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

    use crate::nlp::embeddings::{
        CANONICAL_EMBEDDING_CONTRACT_V1, CANONICAL_EMBEDDING_DIMENSIONS, CANONICAL_EMBEDDING_MODEL,
    };

    /// A canonical-width vector whose leading components are `head` and the
    /// rest zero, so cosine similarity is the one of `head` alone.
    fn canonical_blob(head: &[f32]) -> Vec<u8> {
        let mut values = vec![0.0_f32; CANONICAL_EMBEDDING_DIMENSIONS];
        values[..head.len()].copy_from_slice(head);
        values.into_iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn insert_asset_row(conn: &rusqlite::Connection, asset_id: &str, collection_id: &str) {
        let item_id = format!("item-{asset_id}");
        conn.execute(
            "INSERT INTO items(id, collection_id, title) VALUES (?1, ?2, ?3)",
            rusqlite::params![item_id, collection_id, format!("Title {asset_id}")],
        )
        .expect("item should insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, ?3, 'pdf', 1)",
            rusqlite::params![asset_id, item_id, format!("{asset_id}.pdf")],
        )
        .expect("asset should insert");
    }

    fn insert_vector_row(
        conn: &rusqlite::Connection,
        asset_id: &str,
        blob: Vec<u8>,
        model: &str,
        contract: &str,
        dimensions: usize,
    ) {
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                asset_id,
                format!("item-{asset_id}"),
                blob,
                model,
                contract,
                dimensions as i64
            ],
        )
        .expect("vector should insert");
    }

    /// Asset with item, file and a vector stamped with the canonical contract.
    fn insert_canonical_asset(conn: &rusqlite::Connection, asset_id: &str, head: &[f32]) {
        insert_asset_row(conn, asset_id, "col-1");
        insert_vector_row(
            conn,
            asset_id,
            canonical_blob(head),
            CANONICAL_EMBEDDING_MODEL,
            CANONICAL_EMBEDDING_CONTRACT_V1,
            CANONICAL_EMBEDDING_DIMENSIONS,
        );
    }

    fn result_ids(results: &serde_json::Value) -> Vec<String> {
        results
            .as_array()
            .expect("result should be a JSON array")
            .iter()
            .map(|row| row["assetId"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn setup_similar_assets_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db should open");
        create_similar_assets_schema(&conn);
        conn
    }

    fn create_similar_assets_schema(conn: &rusqlite::Connection) {
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
                embedding BLOB NOT NULL,
                embedding_model TEXT NOT NULL DEFAULT 'legacy',
                embedding_contract TEXT NOT NULL DEFAULT 'legacy',
                dimensions INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
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

        for (asset_id, item_id, head) in [
            ("asset-target", "item-target", [1.0_f32, 0.0_f32]),
            ("asset-same", "item-same", [0.8_f32, 0.2_f32]),
            ("asset-other", "item-other", [0.95_f32, 0.05_f32]),
        ] {
            conn.execute(
                "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    asset_id,
                    item_id,
                    canonical_blob(&head),
                    CANONICAL_EMBEDDING_MODEL,
                    CANONICAL_EMBEDDING_CONTRACT_V1,
                    CANONICAL_EMBEDDING_DIMENSIONS as i64
                ],
            )
            .expect("embedding should insert");
        }

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

    #[test]
    fn similar_assets_scans_past_the_first_2000_vectors() {
        let conn = setup_similar_assets_test_db();
        insert_canonical_asset(&conn, "asset-target", &[1.0, 0.0]);
        conn.execute_batch("BEGIN").expect("begin");
        for index in 0..2100 {
            insert_canonical_asset(&conn, &format!("filler-{index:04}"), &[0.5, 0.5]);
        }
        conn.execute_batch("COMMIT").expect("commit");
        insert_canonical_asset(&conn, "zz-winner", &[1.0, 0.01]);

        // The pre-fix query read this capped subset (SQLite walks the asset_id
        // primary key here); prove the winner is outside it so the test pins the bug.
        let capped: Vec<String> = conn
            .prepare("SELECT asset_id FROM vec_assets WHERE asset_id != ?1 LIMIT 2000")
            .expect("prepare capped")
            .query_map(["asset-target"], |row| row.get(0))
            .expect("query capped")
            .collect::<Result<_, _>>()
            .expect("read capped");
        assert!(!capped.contains(&"zz-winner".to_string()));

        let results = super::similar_assets_from_conn(&conn, "asset-target", 5)
            .expect("similar assets query should succeed");

        assert_eq!(
            result_ids(&results),
            vec![
                "zz-winner",
                "filler-0000",
                "filler-0001",
                "filler-0002",
                "filler-0003"
            ],
            "the closest vector wins and equal scores break ties by asset id"
        );
        assert_eq!(results[0]["title"], "Title zz-winner");
        assert_eq!(results[0]["assetPath"], "zz-winner.pdf");
    }

    #[test]
    fn similar_assets_compares_only_vectors_of_the_effective_contract() {
        let conn = setup_similar_assets_test_db();
        insert_canonical_asset(&conn, "asset-target", &[1.0, 0.0]);
        insert_canonical_asset(&conn, "asset-compatible", &[0.5, 0.5]);
        for (asset_id, model, contract, dimensions) in [
            (
                "asset-other-contract",
                CANONICAL_EMBEDDING_MODEL,
                "other-contract",
                CANONICAL_EMBEDDING_DIMENSIONS,
            ),
            (
                "asset-other-model",
                "other/model",
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS,
            ),
            ("asset-legacy", "legacy", "legacy", 0),
        ] {
            insert_asset_row(&conn, asset_id, "col-1");
            insert_vector_row(
                &conn,
                asset_id,
                canonical_blob(&[1.0, 0.0]),
                model,
                contract,
                dimensions,
            );
        }

        let results = super::similar_assets_from_conn(&conn, "asset-target", 5)
            .expect("similar assets query should succeed");

        assert_eq!(result_ids(&results), vec!["asset-compatible"]);
    }

    #[test]
    fn similar_assets_returns_empty_when_the_source_vector_is_not_compatible() {
        let conn = setup_similar_assets_test_db();
        for asset_id in ["asset-target", "asset-legacy-peer"] {
            insert_asset_row(&conn, asset_id, "col-1");
            insert_vector_row(
                &conn,
                asset_id,
                canonical_blob(&[1.0, 0.0]),
                "legacy",
                "legacy",
                0,
            );
        }
        insert_canonical_asset(&conn, "asset-canonical", &[1.0, 0.0]);

        let results = super::similar_assets_from_conn(&conn, "asset-target", 5)
            .expect("an incompatible source is not an error");

        assert_eq!(results, serde_json::json!([]));
    }

    #[test]
    fn similar_assets_reports_a_missing_source_embedding() {
        let conn = setup_similar_assets_test_db();
        insert_canonical_asset(&conn, "asset-peer", &[1.0, 0.0]);

        let error = super::similar_assets_from_conn(&conn, "asset-missing", 5)
            .expect_err("a source without a vector is an error");

        assert_eq!(error, "No embedding found for asset 'asset-missing'");
    }

    #[test]
    fn similar_assets_propagates_an_unknown_embedding_provider() {
        let conn = setup_similar_assets_test_db();
        insert_canonical_asset(&conn, "asset-target", &[1.0, 0.0]);
        insert_canonical_asset(&conn, "asset-peer", &[1.0, 0.0]);
        conn.execute(
            "INSERT INTO app_settings(key, value) VALUES ('embedding_provider', 'mystery')",
            [],
        )
        .expect("setting should insert");

        let error = super::similar_assets_from_conn(&conn, "asset-target", 5)
            .expect_err("an unknown provider names no comparable space");

        assert_eq!(
            error,
            "Proveedor de embeddings no soportado: mystery. Usa api o local."
        );
    }

    #[test]
    fn similar_assets_scores_cosine_similarity_and_skips_zero_vectors() {
        let conn = setup_similar_assets_test_db();
        insert_canonical_asset(&conn, "asset-target", &[1.0, 0.0]);
        insert_canonical_asset(&conn, "asset-identical", &[2.0, 0.0]);
        insert_canonical_asset(&conn, "asset-orthogonal", &[0.0, 3.0]);
        insert_canonical_asset(&conn, "asset-opposite", &[-1.0, 0.0]);
        insert_canonical_asset(&conn, "asset-zero", &[0.0, 0.0]);

        let results = super::similar_assets_from_conn(&conn, "asset-target", 5)
            .expect("similar assets query should succeed");

        assert_eq!(
            result_ids(&results),
            vec!["asset-identical", "asset-orthogonal", "asset-opposite"]
        );
        let scores: Vec<f64> = results
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["similarity"].as_f64().unwrap())
            .collect();
        for (score, expected) in scores.iter().zip([1.0, 0.0, -1.0]) {
            assert!(
                (score - expected).abs() < 1e-9,
                "score {score} should be {expected}"
            );
        }
    }

    #[test]
    fn similar_assets_returns_exactly_limit_hydrated_results() {
        let conn = setup_similar_assets_test_db();
        insert_canonical_asset(&conn, "asset-target", &[1.0, 0.0]);
        for index in 0..8 {
            insert_canonical_asset(&conn, &format!("asset-{index}"), &[1.0, index as f32]);
        }

        for limit in [0_usize, 1, 7] {
            let results = super::similar_assets_from_conn(&conn, "asset-target", limit)
                .expect("similar assets query should succeed");
            let ids = result_ids(&results);
            let expected: Vec<String> = (0..limit).map(|index| format!("asset-{index}")).collect();
            assert_eq!(ids, expected, "limit {limit}");
            for row in results.as_array().unwrap() {
                let asset_id = row["assetId"].as_str().unwrap();
                assert_eq!(row["title"], format!("Title {asset_id}"));
                assert_eq!(row["assetPath"], format!("{asset_id}.pdf"));
                assert_eq!(row["collectionId"], "col-1");
                assert_eq!(row["itemId"], format!("item-{asset_id}"));
            }
        }
    }

    /// Latency probe for the full scan, not a regression check. Run it in
    /// release, where the app ships:
    /// `cargo test --release --lib similar_assets_scan_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark: 100k canonical vectors; run in release with --ignored --nocapture"]
    fn similar_assets_scan_benchmark() {
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        const ROWS: usize = 100_000;
        let dir = tempfile::tempdir().expect("tempdir should open");
        let path = dir.path().join("entropia.sqlite");
        {
            let conn =
                crate::db::open::open_archive_connection(&path).expect("archive should open");
            create_similar_assets_schema(&conn);
            insert_canonical_asset(&conn, "asset-target", &[1.0, 0.5, 0.25]);
            // Deterministic xorshift values fill every dimension, so each
            // vector costs a full 4 KB read and a full dot product.
            let mut state = 0x2545_f491_4f6c_dd1d_u64;
            let mut values = vec![0.0_f32; CANONICAL_EMBEDDING_DIMENSIONS];
            conn.execute_batch("BEGIN").expect("begin");
            for index in 0..ROWS {
                for value in values.iter_mut() {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    *value = (state >> 40) as f32 / (1_u64 << 24) as f32 - 0.5;
                }
                insert_vector_row(
                    &conn,
                    &format!("asset-{index:06}"),
                    values.iter().flat_map(|f| f.to_le_bytes()).collect(),
                    CANONICAL_EMBEDDING_MODEL,
                    CANONICAL_EMBEDDING_CONTRACT_V1,
                    CANONICAL_EMBEDDING_DIMENSIONS,
                );
            }
            conn.execute_batch("COMMIT").expect("commit");
        }

        // A fresh connection starts with an empty SQLite page cache; the OS
        // page cache stays warm (dropping it needs root), so this is not cold disk.
        let conn = crate::db::open::open_archive_connection(&path).expect("archive should reopen");
        let started = Instant::now();
        let first = similar_assets_from_conn(&conn, "asset-target", 5).expect("scan");
        let first_run = started.elapsed();
        assert_eq!(first.as_array().map(Vec::len), Some(5));

        let mut warm: Vec<Duration> = (0..5)
            .map(|_| {
                let started = Instant::now();
                similar_assets_from_conn(&conn, "asset-target", 5).expect("scan");
                started.elapsed()
            })
            .collect();
        warm.sort();

        // The command holds the shared UI connection for the whole scan; time
        // how long another UI read waits behind it.
        let shared = Arc::new(Mutex::new(conn));
        let scanner = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let conn = shared.lock().expect("lock");
                similar_assets_from_conn(&conn, "asset-target", 5).expect("scan");
            })
        };
        std::thread::sleep(Duration::from_millis(5));
        let waited = Instant::now();
        let one: i64 = shared
            .lock()
            .expect("lock")
            .query_row("SELECT 1", [], |row| row.get(0))
            .expect("ui read");
        let ui_wait = waited.elapsed();
        scanner.join().expect("scanner thread");
        assert_eq!(one, 1);

        eprintln!(
            "similar_assets over {ROWS} vectors: fresh connection {first_run:?}, \
             warm min {:?} / median {:?} / max {:?}, concurrent UI read waited {ui_wait:?}",
            warm[0], warm[2], warm[4]
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
///
/// Scores every vector of the effective embedding contract, keeping only the
/// best `limit` in memory, so no page is skipped by a candidate cap.
#[tauri::command]
pub async fn similar_assets(
    asset_id: String,
    limit: Option<u8>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<serde_json::Value, String> {
    let limit = limit.unwrap_or(5) as usize;
    let db = db.inner().clone();
    run_blocking_db_task(move || {
        let conn = db.ui_conn.lock().map_err(|e| e.to_string())?;
        similar_assets_from_conn(&conn, &asset_id, limit)
    })
    .await
}

fn similar_assets_from_conn(
    conn: &rusqlite::Connection,
    asset_id: &str,
    limit: usize,
) -> Result<serde_json::Value, String> {
    let (target_blob, model, contract, dimensions): (Vec<u8>, String, String, i64) = conn
        .query_row(
            "SELECT embedding, embedding_model, embedding_contract, dimensions
             FROM vec_assets WHERE asset_id = ?1",
            rusqlite::params![asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                format!("No embedding found for asset '{asset_id}'")
            } else {
                format!("Failed to read embedding for asset '{asset_id}': {e}")
            }
        })?;

    // Only vectors of the effective space are comparable. A source stamped
    // with another space (for example a legacy row awaiting backfill) has
    // nothing it can be meaningfully ranked against.
    let effective = crate::processing::eligibility::resolve_effective_embedding_contract(conn)?;
    let effective_dimensions = effective.dimensions as i64;
    if limit == 0
        || model != effective.model
        || contract != effective.contract
        || dimensions != effective_dimensions
    {
        return Ok(serde_json::Value::Array(Vec::new()));
    }

    let target = decode_embedding_blob(&target_blob)?;
    let ranked = top_similar_assets(
        conn,
        asset_id,
        &target,
        &effective.model,
        &effective.contract,
        effective_dimensions,
        limit,
    )?;
    hydrate_similar_assets(conn, ranked)
}

/// A scored candidate. Ordering is "better than": higher similarity first,
/// then the lower asset id, so ties resolve the same way on every run.
#[derive(Debug)]
struct ScoredAsset {
    similarity: f64,
    asset_id: String,
}

impl Ord for ScoredAsset {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.similarity
            .total_cmp(&other.similarity)
            .then_with(|| other.asset_id.cmp(&self.asset_id))
    }
}

impl PartialOrd for ScoredAsset {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ScoredAsset {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for ScoredAsset {}

/// Stream every compatible vector and keep the best `limit`, best first.
fn top_similar_assets(
    conn: &rusqlite::Connection,
    asset_id: &str,
    target: &[f32],
    model: &str,
    contract: &str,
    dimensions: i64,
    limit: usize,
) -> Result<Vec<ScoredAsset>, String> {
    use std::cmp::Reverse;

    let mut stmt = conn
        .prepare(
            "SELECT asset_id, embedding FROM vec_assets
             WHERE asset_id != ?1 AND embedding_model = ?2
               AND embedding_contract = ?3 AND dimensions = ?4",
        )
        .map_err(|e| format!("Failed to prepare asset similarity query: {e}"))?;
    let mut rows = stmt
        .query(rusqlite::params![asset_id, model, contract, dimensions])
        .map_err(|e| format!("Failed to execute asset similarity query: {e}"))?;

    // Min-heap of the current winners: its top is the weakest one kept.
    let mut best: std::collections::BinaryHeap<Reverse<ScoredAsset>> =
        std::collections::BinaryHeap::with_capacity(limit + 1);
    while let Some(row) = rows
        .next()
        .map_err(|e| format!("Failed to read asset similarity rows: {e}"))?
    {
        let blob = row
            .get_ref(1)
            .and_then(|value| value.as_blob().map_err(Into::into))
            .map_err(|e| format!("Failed to read asset similarity rows: {e}"))?;
        let Some(distance) = cosine_distance_to_blob(target, blob) else {
            continue;
        };
        let similarity = 1.0 - distance;
        if !similarity.is_finite() {
            continue;
        }
        if best.len() == limit
            && best
                .peek()
                .is_some_and(|Reverse(weakest)| similarity < weakest.similarity)
        {
            continue;
        }
        let candidate = ScoredAsset {
            similarity,
            asset_id: row
                .get(0)
                .map_err(|e| format!("Failed to read asset similarity rows: {e}"))?,
        };
        if best.len() < limit {
            best.push(Reverse(candidate));
        } else if best
            .peek()
            .is_some_and(|Reverse(weakest)| candidate > *weakest)
        {
            best.pop();
            best.push(Reverse(candidate));
        }
    }

    // Ascending order of `Reverse` is descending similarity: best first.
    Ok(best
        .into_sorted_vec()
        .into_iter()
        .map(|Reverse(scored)| scored)
        .collect())
}

/// Load title, file and text preview for the winners in one query, keeping
/// the ranking order (`IN (...)` returns rows in no particular order).
fn hydrate_similar_assets(
    conn: &rusqlite::Connection,
    ranked: Vec<ScoredAsset>,
) -> Result<serde_json::Value, String> {
    if ranked.is_empty() {
        return Ok(serde_json::Value::Array(Vec::new()));
    }

    let placeholders = (1..=ranked.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT v.asset_id, v.item_id, i.title, i.collection_id, a.path, a.type,
                COALESCE(e.text_content, t.text_content, '') AS text_preview
         FROM vec_assets v
         LEFT JOIN assets a ON a.id = v.asset_id
         LEFT JOIN items i ON i.id = v.item_id
         LEFT JOIN extractions e ON e.asset_id = v.asset_id
         LEFT JOIN transcriptions t ON t.asset_id = v.asset_id
         WHERE v.asset_id IN ({placeholders})"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare similar asset details query: {e}"))?;
    let mut details: std::collections::HashMap<String, [String; 6]> = stmt
        .query_map(
            rusqlite::params_from_iter(ranked.iter().map(|scored| scored.asset_id.as_str())),
            |row| {
                let text = |index: usize| -> rusqlite::Result<String> {
                    Ok(row.get::<_, Option<String>>(index)?.unwrap_or_default())
                };
                Ok((
                    text(0)?,
                    [text(1)?, text(2)?, text(3)?, text(4)?, text(5)?, text(6)?],
                ))
            },
        )
        .map_err(|e| format!("Failed to execute similar asset details query: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Failed to read similar asset details: {e}"))?;

    Ok(serde_json::Value::Array(
        ranked
            .into_iter()
            .filter_map(|scored| {
                let [item_id, title, collection_id, asset_path, asset_type, text_preview] =
                    details.remove(&scored.asset_id)?;
                Some(serde_json::json!({
                    "assetId": scored.asset_id,
                    "itemId": item_id,
                    "title": title,
                    "collectionId": collection_id,
                    "assetPath": asset_path,
                    "assetType": asset_type,
                    "textPreview": truncate_preview(&text_preview, PREVIEW_MAX_CHARS),
                    "similarity": scored.similarity,
                }))
            })
            .collect(),
    ))
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

// `decode_embedding_blob` y `cosine_distance` viven en `nlp::vector` para que
// el pipeline RAG pueda reutilizarlos sin duplicar lógica.
