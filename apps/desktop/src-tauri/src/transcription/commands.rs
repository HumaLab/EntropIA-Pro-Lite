/// Tauri IPC commands for transcription operations.
use super::{TranscriptionJob, TranscriptionQueue};
use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use tauri::{AppHandle, State};

/// [`super::ensure_selected_cloud_key`] on a connection of its own, inside a
/// blocking task: the async commands must not lock `ui_conn` on their own
/// thread, and the credential store must not be read while it is held (A-05b).
/// The settings rows are already committed, so the fresh connection sees them.
fn ensure_selected_cloud_key_blocking(db: &AppDbState) -> Result<(), String> {
    let conn = crate::db::open::open_archive_connection(&db.db_path)?;
    super::ensure_selected_cloud_key(&conn)
}

/// [`super::ensure_transcription_runtime_ready`] off the async worker thread.
///
/// The readiness check may fetch the remote bootstrap catalog with
/// `reqwest::blocking`, whose internal tokio runtime panics when dropped on an
/// async-context thread — the command's future dies mid-flight and the invoke
/// never resolves. Running it on the blocking pool (the same context the OCR
/// executor already uses) keeps the drop legal.
async fn ensure_transcription_runtime_ready_off_thread(
    app_handle: &AppHandle,
) -> Result<(), String> {
    let app_handle = app_handle.clone();
    tokio::task::spawn_blocking(move || super::ensure_transcription_runtime_ready(&app_handle))
        .await
        .map_err(|e| format!("Transcription runtime readiness task failed: {e}"))?
}

#[tauri::command]
pub async fn test_assemblyai_connection(
    api_key: String,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    let db = db.inner().clone();
    let api_key = run_blocking_db_task(move || {
        crate::settings::resolve_api_key_input_unlocked(
            &db.ui_conn,
            crate::settings::ASSEMBLYAI_API_KEY,
            &api_key,
            &crate::settings::KeyringSecretStore,
        )
    })
    .await?;
    super::assemblyai::AssemblyAiClient::new(api_key)
        .test_connection()
        .await
}

/// Submit a transcription job to the background worker queue.
///
/// Returns immediately with `Ok("queued")`. The worker will process the job
/// asynchronously and emit `transcription:progress`, `transcription:complete`,
/// or `transcription:error` events.
///
/// # Arguments
/// * `asset_id`   — unique ID of the asset in the database
/// * `asset_path` — absolute filesystem path to the audio file
/// * `transcription_queue` — managed state injected by Tauri
#[tauri::command]
pub async fn transcribe_audio(
    asset_id: String,
    asset_path: String,
    app_handle: AppHandle,
    transcription_queue: State<'_, TranscriptionQueue>,
    db: State<'_, AppDbState>,
) -> Result<String, String> {
    // Resolve once here: the value travels into TranscriptionJob and is read raw
    // by the worker and by each provider.
    let asset_path = crate::path_utils::resolve_asset_path_at_boundary(&asset_path, &app_handle)?;

    ensure_transcription_runtime_ready_off_thread(&app_handle).await?;
    {
        let db = db.inner().clone();
        run_blocking_db_task(move || ensure_selected_cloud_key_blocking(&db)).await?;
    }

    crate::app_logs::info(
        &app_handle,
        "transcription",
        format!("Trabajo de transcripción encolado: asset_id={asset_id}"),
    );

    let job = TranscriptionJob {
        asset_id,
        asset_path,
    };

    transcription_queue.submit(job)?;
    Ok("queued".to_string())
}

/// Read the manual-edit signal without changing the transcription.
#[tauri::command]
pub async fn transcription_is_edited_by_hand(
    asset_id: String,
    db: State<'_, AppDbState>,
) -> Result<bool, String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {e}"))?;
        crate::asset_history::transcription_is_edited_by_hand(&conn, &asset_id)
    })
    .await
}

/// Update the text_content of the latest transcription for an asset.
///
/// This allows users to manually correct transcription output.
/// Downstream NLP refresh is debounced in the frontend after a period of
/// user inactivity, so this command only persists the edited text.
#[tauri::command]
pub async fn update_transcription_text_cmd(
    asset_id: String,
    text_content: String,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {e}"))?;

        // Find the latest transcription for this asset
        let mut stmt = conn
            .prepare(
                "SELECT id FROM transcriptions WHERE asset_id = ?1 ORDER BY created_at DESC LIMIT 1",
            )
            .map_err(|e| format!("Failed to prepare query: {e}"))?;

        let transcription_id: Result<String, _> = stmt.query_row([&asset_id], |row| row.get(0));

        drop(stmt); // release borrow before execute

        if let Ok(id) = transcription_id {
            conn.execute(
                "UPDATE transcriptions SET text_content = ?1 WHERE id = ?2",
                rusqlite::params![text_content, id],
            )
            .map_err(|e| format!("Failed to update transcription text: {e}"))?;
            crate::asset_history::record_text_edit(
                &conn,
                &asset_id,
                crate::asset_history::PROCESS_TRANSCRIPTION_EDIT,
                &text_content,
            );
        }

        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn transcribe_dictation(
    audio_path: String,
    app_handle: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<String, String> {
    ensure_transcription_runtime_ready_off_thread(&app_handle).await?;
    crate::app_logs::info(
        &app_handle,
        "transcription",
        "Transcripción de dictado iniciada",
    );
    let db_path = db.db_path.clone();
    {
        let db = db.inner().clone();
        run_blocking_db_task(move || ensure_selected_cloud_key_blocking(&db)).await?;
    }

    let audio_path_for_worker = audio_path.clone();
    let app_handle_for_worker = app_handle.clone();
    let started = std::time::Instant::now();
    let transcription_result = tauri::async_runtime::spawn_blocking(move || {
        let conn = crate::db::open::open_archive_connection(&db_path)
            .map_err(|e| format!("Failed to open settings DB for dictation: {e}"))?;

        super::transcribe_with_selected_provider(
            &app_handle_for_worker,
            Some(db_path.as_path()),
            &conn,
            None,
            &audio_path_for_worker,
        )
    })
    .await
    .map_err(|e| format!("Dictation task failed: {e}"))?
    .map(|result| {
        // R10: a successful dictation used to leave no trace in the log.
        crate::app_logs::info(
            &app_handle,
            "transcription",
            format!(
                "Dictado transcripto: motor={}, audio={:.1}s, tardó={}ms",
                result.model_name,
                result.transcription.duration_ms as f64 / 1000.0,
                started.elapsed().as_millis()
            ),
        );
        result.transcription
    });

    let cleanup_result = super::cleanup_temp_audio_file(&audio_path);

    match (transcription_result, cleanup_result) {
        (Ok(result), Ok(())) => Ok(result.text.trim().to_string()),
        (Ok(result), Err(cleanup_error)) => {
            crate::app_logs::warn(
                &app_handle,
                "transcription",
                format!("Dictado transcripto, pero falló limpieza temporal: {cleanup_error}"),
            );
            Ok(result.text.trim().to_string())
        }
        (Err(error), Ok(())) => {
            crate::app_logs::error(
                &app_handle,
                "transcription",
                format!("Dictado falló: {error}"),
            );
            Err(error)
        }
        (Err(error), Err(cleanup_error)) => {
            crate::app_logs::error(
                &app_handle,
                "transcription",
                format!(
                    "Dictado falló y también falló limpieza temporal: {error}; {cleanup_error}"
                ),
            );
            Err(format!(
                "{error}\nTemporary file cleanup failed: {cleanup_error}"
            ))
        }
    }
}

/// What Configuración shows about the local Parakeet model.
#[derive(Clone, serde::Serialize)]
#[cfg_attr(not(feature = "local-ml"), allow(dead_code))]
pub struct ParakeetModelInfo {
    pub installed: bool,
    pub downloading: bool,
    pub missing_bytes: u64,
    pub total_bytes: u64,
    pub directory: String,
}

#[cfg(feature = "local-ml")]
static PARAKEET_DOWNLOADING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Status of the Parakeet model on disk (R11: dictation no longer downloads
/// it, so Configuración says whether it is there and how much is left).
#[cfg(feature = "local-ml")]
#[tauri::command]
pub async fn stt_parakeet_model_info(
    db: State<'_, AppDbState>,
) -> Result<ParakeetModelInfo, String> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || parakeet_info(&db_path))
        .await
        .map_err(|e| format!("Parakeet model check failed: {e}"))
}

/// [`stt_parakeet_model_info`] for blocking callers (`models_setup`).
#[cfg(feature = "local-ml")]
pub(crate) fn parakeet_info(db_path: &std::path::Path) -> ParakeetModelInfo {
    let dir = super::parakeet::parakeet_model_dir(db_path);
    let (missing_bytes, total_bytes) = super::parakeet::parakeet_download_size(&dir);
    ParakeetModelInfo {
        installed: missing_bytes == 0,
        downloading: PARAKEET_DOWNLOADING.load(std::sync::atomic::Ordering::SeqCst),
        missing_bytes,
        total_bytes,
        directory: dir.display().to_string(),
    }
}

/// Start the Parakeet download in the background: `parakeet:download_progress`
/// while it runs, then `parakeet:download_complete` or
/// `parakeet:download_error`. A partial file resumes where it stopped.
#[cfg(feature = "local-ml")]
#[tauri::command]
pub async fn stt_parakeet_download_model(
    db: State<'_, AppDbState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    start_parakeet_download(&db.db_path, app_handle)
}

/// Shared starter behind the Settings button, Preparar EntropIA Pro and the
/// assistant `models_setup` tool.
#[cfg(feature = "local-ml")]
pub(crate) fn start_parakeet_download(
    db_path: &std::path::Path,
    app_handle: AppHandle,
) -> Result<String, String> {
    use crate::llm::download::{
        ModelDownloadCompletePayload, ModelDownloadErrorPayload, ModelDownloadProgressPayload,
    };
    use std::sync::atomic::Ordering;
    use tauri::Emitter;

    if PARAKEET_DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Ok("in_progress".to_string());
    }
    let db_path = db_path.to_path_buf();
    crate::app_logs::info(
        &app_handle,
        "parakeet/download",
        "Inicio descarga de Parakeet",
    );
    tauri::async_runtime::spawn(async move {
        let worker = app_handle.clone();
        let started = std::time::Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            let dir = super::parakeet::parakeet_model_dir(&db_path);
            let mut last_pct = None;
            super::parakeet::download_parakeet_model(&dir, &mut |done, total| {
                let pct = (done.saturating_mul(100) / total.max(1)).min(100) as u8;
                if last_pct == Some(pct) {
                    return;
                }
                last_pct = Some(pct);
                let _ = worker.emit(
                    "parakeet:download_progress",
                    ModelDownloadProgressPayload {
                        pct,
                        downloaded_bytes: done,
                        total_bytes: Some(total),
                    },
                );
                if pct.is_multiple_of(10) {
                    crate::app_logs::info(
                        &worker,
                        "parakeet/download",
                        format!("Descarga de Parakeet {pct}% ({} MB)", done / 1_000_000),
                    );
                }
            })
            .map(|()| dir)
        })
        .await
        .unwrap_or_else(|e| Err(format!("Parakeet download task panicked: {e}")));
        PARAKEET_DOWNLOADING.store(false, Ordering::SeqCst);
        match result {
            Ok(dir) => {
                crate::app_logs::info(
                    &app_handle,
                    "parakeet/download",
                    format!(
                        "Descarga de Parakeet completada en {}s: {}",
                        started.elapsed().as_secs(),
                        dir.display()
                    ),
                );
                let _ = app_handle.emit(
                    "parakeet:download_complete",
                    ModelDownloadCompletePayload {
                        path: dir.display().to_string(),
                    },
                );
            }
            Err(error) => {
                crate::app_logs::error(
                    &app_handle,
                    "parakeet/download",
                    format!("Descarga de Parakeet falló: {error}"),
                );
                let _ = app_handle.emit(
                    "parakeet:download_error",
                    ModelDownloadErrorPayload { error },
                );
            }
        }
    });
    Ok("started".to_string())
}

/// Lite ships no local transcription engine.
#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub async fn stt_parakeet_model_info(
    _db: State<'_, AppDbState>,
) -> Result<ParakeetModelInfo, String> {
    Err("Parakeet solo está disponible en EntropIA Pro".to_string())
}

#[cfg(not(feature = "local-ml"))]
#[tauri::command]
pub async fn stt_parakeet_download_model(
    _db: State<'_, AppDbState>,
    _app_handle: AppHandle,
) -> Result<String, String> {
    Err("Parakeet solo está disponible en EntropIA Pro".to_string())
}
