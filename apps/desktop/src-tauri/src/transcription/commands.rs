/// Tauri IPC commands for transcription operations.
use super::{TranscriptionJob, TranscriptionQueue};
use crate::db::state::AppDbState;
use crate::nlp::NlpQueue;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn test_assemblyai_connection(
    api_key: String,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    let api_key = {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|error| format!("DB lock error: {error}"))?;
        crate::settings::resolve_api_key_input(
            &conn,
            crate::settings::ASSEMBLYAI_API_KEY,
            &api_key,
        )?
    };
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
    super::ensure_transcription_runtime_ready(&app_handle)?;
    {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {e}"))?;
        super::ensure_selected_cloud_key(&conn)?;
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
    _nlp_queue: State<'_, NlpQueue>,
) -> Result<(), String> {
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
    }

    Ok(())
}

#[tauri::command]
pub async fn transcribe_dictation(
    audio_path: String,
    app_handle: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<String, String> {
    super::ensure_transcription_runtime_ready(&app_handle)?;
    crate::app_logs::info(
        &app_handle,
        "transcription",
        "Transcripción de dictado iniciada",
    );
    let db_path = db.db_path.clone();
    {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {e}"))?;
        super::ensure_selected_cloud_key(&conn)?;
    }

    let audio_path_for_worker = audio_path.clone();
    let app_handle_for_worker = app_handle.clone();
    let transcription_result = tauri::async_runtime::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| format!("Failed to open settings DB for dictation: {e}"))?;

        super::transcribe_with_selected_provider(
            &app_handle_for_worker,
            Some(db_path.as_path()),
            &conn,
            None,
            &audio_path_for_worker,
        )
        .map(|result| result.transcription)
    })
    .await
    .map_err(|e| format!("Dictation task failed: {e}"))?;

    let cleanup_result = super::cleanup_temp_audio_file(&audio_path);

    match (transcription_result, cleanup_result) {
        (Ok(result), Ok(())) => Ok(result.text.trim().to_string()),
        (Ok(result), Err(cleanup_error)) => {
            eprintln!("[transcription] Dictation cleanup warning: {cleanup_error}");
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
