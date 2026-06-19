use tauri::State;

#[cfg(feature = "local-ml")]
use super::download::download_model_file;
use super::openrouter::{ModelInfo, OpenRouterClient};
use super::{
    get_local_model_info, resolve_model_path, LlmJob, LlmQueue, LlmResultEntry, LocalModelInfo,
};
#[cfg(feature = "local-ml")]
use super::LlmDownloadErrorPayload;
#[cfg(feature = "local-ml")]
use super::{resolve_local_model_filename, resolve_local_model_source_url};
use crate::db::state::AppDbState;
#[cfg(feature = "local-ml")]
use tauri::Emitter;

/// Returns `true` if the LLM engine loaded successfully and is ready to accept jobs.
#[tauri::command]
pub async fn llm_is_available(llm_queue: State<'_, LlmQueue>) -> Result<bool, String> {
    Ok(llm_queue.is_available())
}

/// Return the current status of the local Gemma GGUF model file.
#[tauri::command]
pub async fn llm_local_model_info(db: State<'_, AppDbState>) -> Result<LocalModelInfo, String> {
    Ok(get_local_model_info(&db.db_path))
}

/// Open the models directory in the system file manager.
#[tauri::command]
pub async fn llm_open_models_dir(db: State<'_, AppDbState>) -> Result<(), String> {
    let db_path = &db.db_path;
    let models_dir = resolve_model_path(db_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| db_path.parent().unwrap_or(db_path).join("models"));
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models dir: {e}"))?;

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

/// Start downloading the local model from the configured (or default) source URL.
/// Emits `llm:download_progress`, `llm:download_complete`, and `llm:download_error` events.
///
/// Without the `local-ml` feature there is no local engine to feed, so the
/// command stays registered but reports that the local model is not needed in
/// this build (OpenRouter-only). Mirrors EntropIA Lite.
#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub async fn llm_download_model(
    _db: State<'_, AppDbState>,
    _app_handle: tauri::AppHandle,
) -> Result<String, String> {
    Err("No requerido en este build (solo OpenRouter). Configurá OpenRouter en Configuración para usar LLM remoto.".to_string())
}

/// Start downloading the local model from the configured (or default) source URL.
/// Emits `llm:download_progress`, `llm:download_complete`, and `llm:download_error` events.
#[cfg(feature = "local-ml")]
#[tauri::command]
pub async fn llm_download_model(
    db: State<'_, AppDbState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let db_path = db.db_path.clone();
    let conn =
        rusqlite::Connection::open(&db_path).map_err(|e| format!("Failed to open DB: {e}"))?;

    let url = resolve_local_model_source_url(Some(&conn));

    let filename = resolve_local_model_filename(Some(&conn));

    let models_dir = db_path
        .parent()
        .map(|p| p.join("models"))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("models"));
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models dir: {e}"))?;

    let dest = models_dir.join(&filename);

    if dest.exists() {
        return Err("Model file already exists at the destination".to_string());
    }

    crate::app_logs::info(
        &app_handle,
        "llm/download",
        format!("Inicio descarga de modelo local: {filename}"),
    );

    let app_handle_clone = app_handle.clone();
    let tmp_path = dest.with_extension("download.tmp");
    tauri::async_runtime::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            download_model_file(&url, &dest, &app_handle_clone)
        })
        .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                crate::app_logs::error(
                    &app_handle,
                    "llm/download",
                    format!("Descarga de modelo local falló: {e}"),
                );
                let _ = app_handle.emit("llm:download_error", LlmDownloadErrorPayload { error: e });
                let _ = std::fs::remove_file(&tmp_path);
            }
            Err(e) => {
                crate::app_logs::error(
                    &app_handle,
                    "llm/download",
                    format!("Tarea de descarga de modelo local paniqueó: {e}"),
                );
                let _ = app_handle.emit(
                    "llm:download_error",
                    LlmDownloadErrorPayload {
                        error: format!("Download task panicked: {e}"),
                    },
                );
                let _ = std::fs::remove_file(&tmp_path);
            }
        }
    });

    Ok("started".to_string())
}

#[tauri::command]
pub async fn llm_correct_ocr(
    item_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::CorrectOcr { item_id })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_extract_entities(
    item_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::ExtractEntities { item_id })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_extract_triples(
    item_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::ExtractTriples { item_id })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_summarize(
    item_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::Summarize { item_id })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_classify(
    item_id: String,
    categories: Vec<String>,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::Classify {
        item_id,
        categories,
    })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_ask(
    collection_id: String,
    question: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::Ask {
        collection_id,
        question,
    })?;
    Ok("queued".to_string())
}

// ── Asset-level LLM commands ──────────────────────────────────────────────────
// These operate on a single asset/page, using get_asset_text() which avoids
// concatenating all pages and prevents context-window overflow on multi-page docs.

#[tauri::command]
pub async fn llm_correct_ocr_asset(
    asset_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::CorrectOcrAsset { asset_id })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_extract_entities_asset(
    asset_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::ExtractEntitiesAsset { asset_id })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_extract_triples_asset(
    asset_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::ExtractTriplesAsset { asset_id })?;
    Ok("queued".to_string())
}

#[tauri::command]
pub async fn llm_summarize_asset(
    asset_id: String,
    llm_queue: State<'_, LlmQueue>,
) -> Result<String, String> {
    llm_queue.submit(LlmJob::SummarizeAsset { asset_id })?;
    Ok("queued".to_string())
}

/// Retrieve all latest LLM results for a given target (item or collection).
/// Returns one result per job_type, ordered by most recent first.
#[tauri::command]
pub async fn llm_get_results(
    target_id: String,
    target_type: Option<String>,
    db: State<'_, AppDbState>,
) -> Result<Vec<LlmResultEntry>, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock error: {e}"))?;
    super::get_all_results_for_target(&conn, target_type.as_deref().unwrap_or("item"), &target_id)
}

/// Test the OpenRouter connection with the given API key.
/// Returns a list of available models on success.
#[tauri::command]
pub async fn test_openrouter_connection(api_key: String) -> Result<Vec<ModelInfo>, String> {
    let client = OpenRouterClient::new(api_key, String::new());
    client.test_connection().await
}

/// Retrieve the latest single LLM result for a target + job_type.
#[tauri::command]
pub async fn llm_get_result(
    target_id: String,
    job_type: String,
    target_type: Option<String>,
    db: State<'_, AppDbState>,
) -> Result<Option<LlmResultEntry>, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock error: {e}"))?;
    super::get_latest_result(
        &conn,
        target_type.as_deref().unwrap_or("item"),
        &target_id,
        Some(&job_type),
    )
}
