mod app_logs;
mod audio_preview;
mod db;
// `deps` is whole-file swapped by variant: the full managed-Python implementation
// under local-ml, and Lite's self-contained API-only stub otherwise. The module name
// + its command/struct surface are identical in both arms (DependencyId diverges
// enum-vs-String, so the type can't be shared — hence a file swap, not body branching).
#[cfg(feature = "local-ml")]
#[path = "deps/mod.rs"]
pub mod deps;
#[cfg(not(feature = "local-ml"))]
#[path = "deps/mod_lite.rs"]
pub mod deps;
mod geo;
mod image_edit;
mod llm;
mod nlp;
mod ocr;
mod path_utils;
#[cfg(feature = "local-ml")]
mod python_discovery;
mod rag;
mod runtime;
mod settings;
// `pub` so the multi-device E2E integration test (tests/sync_e2e.rs) can drive
// the engine's internal API (run_cycle / ensure_capture / start_engine).
pub mod sync;
mod transcription;

use db::state::AppDbState;
use geo::GeoQueue;
use llm::LlmQueue;
use nlp::NlpQueue;
use ocr::OcrQueue;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use std::fs;
use std::path::Path;
use std::process::Command;
use tauri::Manager;
use transcription::TranscriptionQueue;

const LEGACY_APP_IDENTIFIER: &str = "com.entropia.app";
const LEGACY_MIGRATION_MARKER: &str = ".legacy-app-dir-merged";
const SQLITE_BASENAME: &str = "entropia.sqlite";
const EXTERNAL_URL_DISALLOWED_CHARS: &[char] =
    &['\0', '\n', '\r', '\t', ' ', '"', '\'', '<', '>', '`', '|'];

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    validate_external_url(&url)?;

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = Command::new("rundll32.exe");
        cmd.args(["url.dll,FileProtocolHandler", &url]);
        cmd
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut cmd = Command::new("open");
        cmd.arg(&url);
        cmd
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(&url);
        cmd
    };

    command
        .spawn()
        .map_err(|error| format!("Failed to open URL: {error}"))?;

    Ok(())
}

fn validate_external_url(url: &str) -> Result<(), String> {
    if url.trim() != url {
        return Err("External URL must not contain leading or trailing whitespace".to_string());
    }

    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only HTTP(S) URLs are allowed".to_string());
    }

    if url.chars().any(|ch| {
        ch.is_ascii_control()
            || EXTERNAL_URL_DISALLOWED_CHARS
                .iter()
                .any(|blocked| *blocked == ch)
    }) {
        return Err("External URL contains unsafe characters".to_string());
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Suppress Windows error dialogs and CRT debug assertions that block the
    // process when native libraries crash.
    // This must run before any other initialization.
    #[cfg(target_os = "windows")]
    unsafe {
        const SEM_FAILCRITICALERRORS: u32 = 0x0001;
        const SEM_NOGPFAULTERRORBOX: u32 = 0x0002;
        const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;
        extern "system" {
            fn SetErrorMode(uMode: u32) -> u32;
        }
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);

        // Suppress CRT debug assertions in debug builds.
        // Routes assertion output to stderr instead of a blocking dialog.
        #[cfg(debug_assertions)]
        {
            extern "C" {
                fn _CrtSetReportMode(reportType: i32, reportMode: i32) -> i32;
            }
            const _CRT_ASSERT: i32 = 2;
            const _CRTDBG_MODE_FILE: i32 = 4;
            const _CRTDBG_FILE_STDERR: i32 = 2;
            _CrtSetReportMode(_CRT_ASSERT, _CRTDBG_MODE_FILE);
            _CrtSetReportMode(_CRT_ASSERT, _CRTDBG_FILE_STDERR);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
            // On a clean/varied Windows PC the data dir or DB can be unreachable
            // (redirected/roaming AppData, read-only or full disk, AV locking the
            // freshly-created folder). SetErrorMode above suppresses the OS crash
            // dialog, so a panic here would vanish silently. Instead, write a
            // diagnostic file to TEMP, show a clear error dialog, and exit cleanly.
            let dialog_handle = app.handle().clone();
            let fail = move |context: &str, detail: String| -> Box<dyn std::error::Error> {
                let log_path = std::env::temp_dir().join("entropia-pro-startup-error.log");
                let _ = std::fs::write(&log_path, format!("{context}\n{detail}\n"));
                eprintln!("EntropIA Pro startup error: {context}: {detail}");
                dialog_handle
                    .dialog()
                    .message(format!(
                        "EntropIA Pro no pudo iniciar.\n\n{context}\n\nDetalle técnico: {detail}\n\nRevisá permisos y espacio libre en la carpeta de datos de la aplicación, y volvé a intentar.\nDiagnóstico guardado en: {}",
                        log_path.display()
                    ))
                    .title("Error al iniciar EntropIA Pro")
                    .kind(MessageDialogKind::Error)
                    .blocking_show();
                detail.into()
            };

            let app_dir = app.path().app_data_dir().map_err(|e| {
                fail(
                    "No se pudo resolver la carpeta de datos (AppData).",
                    e.to_string(),
                )
            })?;
            migrate_legacy_app_dir(&app_dir)
                .map_err(|e| fail("No se pudo preparar la carpeta de datos heredada.", e))?;
            std::fs::create_dir_all(&app_dir).map_err(|e| {
                fail(
                    &format!("No se pudo crear la carpeta de datos {}.", app_dir.display()),
                    e.to_string(),
                )
            })?;
            app.manage(app_logs::AppLogsState::new(app_dir.join("logs")));
            app_logs::info(&app.handle().clone(), "setup", "Registro de diagnóstico inicializado");
            let db_path = app_dir.join("entropia.sqlite");

            migrate_legacy_asset_paths(&db_path, &app_dir).map_err(|e| {
                fail(
                    "No se pudieron migrar rutas heredadas en la base de datos.",
                    e,
                )
            })?;

            // UI connection — used by Tauri IPC commands
            let ui_conn = rusqlite::Connection::open(&db_path).map_err(|e| {
                fail(
                    &format!("No se pudo abrir la base de datos {}.", db_path.display()),
                    e.to_string(),
                )
            })?;
            ui_conn
                .execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
                .map_err(|e| {
                    fail(
                        "No se pudieron configurar los PRAGMA de SQLite.",
                        e.to_string(),
                    )
                })?;

            // Normalize legacy duplicates and enforce one-row-per-asset semantics
            // for extractions/transcriptions so Rust workers can use real UPSERT.
            // Fresh installs may not have these tables yet (created later by JS migrations),
            // so this must be conditional.
            let has_extractions_table = table_exists(&ui_conn, "extractions");
            let has_transcriptions_table = table_exists(&ui_conn, "transcriptions");
            let mut legacy_uniques_sql = String::new();

            if has_extractions_table {
                legacy_uniques_sql.push_str(
                    "DELETE FROM extractions
                     WHERE rowid NOT IN (
                       SELECT MAX(rowid) FROM extractions GROUP BY asset_id
                     );
                     CREATE UNIQUE INDEX IF NOT EXISTS idx_extractions_asset_id_unique
                     ON extractions(asset_id);",
                );
            }

            if has_transcriptions_table {
                legacy_uniques_sql.push_str(
                    "DELETE FROM transcriptions
                     WHERE rowid NOT IN (
                       SELECT MAX(rowid) FROM transcriptions GROUP BY asset_id
                     );
                     CREATE UNIQUE INDEX IF NOT EXISTS idx_transcriptions_asset_id_unique
                     ON transcriptions(asset_id);",
                );
            }

            if !legacy_uniques_sql.is_empty() {
                ui_conn
                    .execute_batch(&legacy_uniques_sql)
                    .expect("Failed to enforce unique asset_id indexes for extraction/transcription");
            } else {
                eprintln!(
                    "[setup] extractions/transcriptions tables not found — skipping legacy unique-index enforcement"
                );
            }

            // Migrate extractions.method CHECK constraint: remove the legacy
            // `CHECK(method IN ('native', 'ocr'))` which blocked modern OCR methods
            // like 'paddle', 'paddle_vl', 'pdf_paddle', and 'pdf_paddle_vl'.
            migrate_extractions_method_check(&ui_conn)
                .expect("Failed to migrate extractions method CHECK constraint");
            llm::ensure_llm_results_schema(&ui_conn)
                .expect("Failed to migrate llm_results table");

            ensure_layouts_schema(&ui_conn)
                .map_err(|e| format!("Failed to migrate layouts table: {e}"))
                .expect("Failed to migrate layouts table");
            let modern_schema_bootstrapped =
                migration_applied(&ui_conn, "0017_vec_assets").unwrap_or(false);

            if !modern_schema_bootstrapped && table_exists(&ui_conn, "assets") {
                // Legacy fallback for databases that haven't run JS migrations yet.
                let has_sort_index: bool = ui_conn
                    .prepare("SELECT sort_index FROM assets LIMIT 0")
                    .and_then(|mut stmt| {
                        let _ = stmt.query_map([], |_| Ok(()));
                        Ok(true)
                    })
                    .unwrap_or(false);

                if !has_sort_index {
                    ui_conn
                        .execute_batch(
                            "ALTER TABLE assets ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
                             CREATE INDEX IF NOT EXISTS idx_assets_item_sort ON assets(item_id, sort_index);",
                        )
                        .map_err(|e| format!("Failed to add sort_index column to assets: {e}"))
                        .expect("Failed to add sort_index column");
                    eprintln!("[setup] Added sort_index column to assets table");
                }

                let has_notes_asset_id: bool = ui_conn
                    .prepare("SELECT asset_id FROM notes LIMIT 0")
                    .and_then(|mut stmt| {
                        let _ = stmt.query_map([], |_| Ok(()));
                        Ok(true)
                    })
                    .unwrap_or(false);

                if !has_notes_asset_id {
                    ui_conn
                        .execute_batch(
                            "ALTER TABLE notes ADD COLUMN asset_id TEXT;
                             ALTER TABLE entities ADD COLUMN asset_id TEXT;
                             ALTER TABLE triples ADD COLUMN asset_id TEXT;
                             CREATE INDEX IF NOT EXISTS idx_notes_asset_id ON notes(asset_id);
                             CREATE INDEX IF NOT EXISTS idx_entities_asset_id ON entities(asset_id);
                             CREATE INDEX IF NOT EXISTS idx_triples_asset_id ON triples(asset_id);",
                        )
                        .map_err(|e| format!("Failed to add asset_id columns: {e}"))
                        .expect("Failed to add asset_id columns");
                    eprintln!("[setup] Added asset_id columns to notes, entities, triples");
                }
            } else if !modern_schema_bootstrapped {
                eprintln!(
                    "[setup] assets table not found — skipping legacy fallback schema patching"
                );
            }

            // Create app_settings table for user configuration (API keys, preferences).
            // Keep this outside the legacy fallback so modern-schema databases get it too.
            ui_conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS app_settings (
                        key TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    );",
                )
                .map_err(|e| format!("Failed to create app_settings table: {e}"))
                .expect("Failed to create app_settings table");
            eprintln!("[setup] app_settings table ensured");

            // OCR worker connection
            let worker_conn = rusqlite::Connection::open(&db_path)
                .expect("Failed to open SQLite database (worker)");
            worker_conn
                .execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
                .expect("Failed to configure SQLite pragmas (worker)");

            app.manage(AppDbState::new(ui_conn, worker_conn, db_path.clone()));

            // Dependency manager: tracks Python-package availability (OCR, embeddings, etc.)
            app.manage(deps::DepsState::new());

            // Managed-runtime lifecycle is local-ml only; the lite build has no runtime to validate.
            #[cfg(feature = "local-ml")]
            {
                app.manage(runtime::manager::RuntimeManager::new());

                if let Err(error) = app
                    .state::<runtime::manager::RuntimeManager>()
                    .inner()
                    .validate_startup(&app.handle().clone())
                {
                    eprintln!("[runtime] startup validation failed: {error}");
                    app_logs::error(
                        &app.handle().clone(),
                        "runtime",
                        format!("Validación inicial falló: {error}"),
                    );
                }
            }

            // Background dependency check — runs 2 s after startup so the window is visible first.
            let app_handle_deps = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let db_state = app_handle_deps.state::<AppDbState>();
                let deps_state = app_handle_deps.state::<deps::DepsState>();
                match deps::probe_all_once(deps_state.inner(), db_state.inner()).await {
                    Ok(results) => {
                        deps::emit_probe_complete(&app_handle_deps, &results);
                        eprintln!("[deps] Startup check: {} deps checked", results.len());
                        app_logs::info(
                            &app_handle_deps,
                            "deps",
                            format!("Verificación inicial completada: {} dependencias", results.len()),
                        );
                    }
                    Err(err) => {
                        eprintln!("[deps] Startup check failed: {err}");
                        app_logs::error(
                            &app_handle_deps,
                            "deps",
                            format!("Verificación inicial falló: {err}"),
                        );
                    }
                }
            });

            // OCR queue: create channel, manage the sender half, spawn worker with receiver
            let (ocr_queue, ocr_receiver) = OcrQueue::new();
            app.manage(ocr_queue);

            // PaddleVL and layout engine creation deferred to OCR worker (lazy init).
            // This removes Python probing and ONNX model loading from the critical
            // startup path, which previously blocked app window display by 3-15s.
            OcrQueue::start_worker(db_path.clone(), ocr_receiver, app.handle().clone());

            // LLM queue: local Gemma model via llama.cpp for NER, summarization,
            // OCR correction, Q&A, etc. Degrades gracefully if model not present.
            let (llm_queue, llm_receiver) = LlmQueue::new(db_path.clone());
            let llm_available = llm_queue.available_flag();
            let nlp_llm_queue = llm_queue.clone();
            app.manage(llm_queue);
            LlmQueue::start_worker(
                db_path.clone(),
                llm_receiver,
                app.handle().clone(),
                llm_available,
            );

            // NLP queue: create channel, manage the sender half, spawn worker with receiver
            // The NLP worker opens its own dedicated connection and initializes the
            // embedding engine (Python subprocess) independently from OCR/UI connections.
            let (nlp_queue, nlp_receiver) = NlpQueue::new();
            // Clone the dedup handle before moving nlp_queue into managed state
            let ner_pending = nlp_queue.ner_pending_handle();
            let fts_pending = nlp_queue.fts_pending_handle();
            let asset_ner_pending = nlp_queue.asset_ner_pending_handle();
            let embedding_pending = nlp_queue.embedding_pending_handle();
            app.manage(nlp_queue);
            NlpQueue::start_worker(
                db_path.clone(),
                nlp_receiver,
                app.handle().clone(),
                ner_pending,
                fts_pending,
                asset_ner_pending,
                embedding_pending,
                nlp_llm_queue,
            );

            // Transcription queue: faster-whisper subprocess for audio transcription.
            // Each job spawns a Python process, no persistent state needed.
            let (transcription_queue, transcription_receiver) = TranscriptionQueue::new();
            app.manage(transcription_queue);
            TranscriptionQueue::start_worker(
                db_path.clone(),
                transcription_receiver,
                app.handle().clone(),
            );

            // Geo queue: Nominatim geocoding for place entities.
            let (geo_queue, geo_receiver) = GeoQueue::new();
            app.manage(geo_queue);
            GeoQueue::start_worker(
                db_path.clone(),
                geo_receiver,
                app.handle().clone(),
            );

            // Sync capture bootstrap (DESIGN §6.1): ensure the sync schema and the
            // capture triggers AFTER every ensure_*/migrate_* patch above. On a
            // fresh install the JS migrations haven't run yet, so this only covers
            // tables that already exist; the frontend re-invokes sync_ensure_capture
            // after initStore() to cover the rest. Non-fatal — never block startup.
            match sync::ensure_capture_on_path(&db_path) {
                Ok(()) => eprintln!("[sync] capture triggers ensured at setup"),
                Err(error) => {
                    eprintln!("[sync] capture bootstrap failed (will retry from UI): {error}")
                }
            }

            // Sync blob cleanup (DESIGN §7): remove orphaned `*.part` temp files
            // under assets/ left by a download interrupted before the atomic
            // rename. Best-effort — never blocks startup.
            match sync::blobs::cleanup_orphan_parts(&app_dir) {
                Ok(0) => {}
                Ok(removed) => eprintln!("[sync] cleaned {removed} orphan .part file(s)"),
                Err(error) => eprintln!("[sync] orphan .part cleanup failed: {error}"),
            }

            // Sync engine (DESIGN §3.1): single long-lived task owning its own
            // connection. Spawned PAUSED — it runs no cycle until the gate opens
            // (capture ensured + a session exists). Held in managed state so the
            // sync_now / sync_status commands can reach it.
            let sync_engine = sync::engine::start_engine(app.handle().clone(), db_path.clone());
            app.manage(sync_engine);
            eprintln!("[sync] engine spawned (gated until capture + session)");

            // On Linux, WebKitGTK denies media-device permission requests by default.
            // We must explicitly enable media-stream and auto-approve permission
            // requests so getUserMedia / MediaRecorder work for dictation.
            #[cfg(target_os = "linux")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    if let Err(e) = window.with_webview(|webview| {
                        use webkit2gtk::WebViewExt;
                        use webkit2gtk::PermissionRequestExt;
                        use webkit2gtk::SettingsExt;
                        let gtk_webview = webview.inner();
                        if let Some(settings) = gtk_webview.settings() {
                            settings.set_enable_media_stream(true);
                            settings.set_enable_webrtc(true);
                        }
                        gtk_webview.connect_permission_request(|_webview, request| {
                            request.allow();
                            true
                        });
                    }) {
                        eprintln!("[linux-setup] Failed to configure webview media permissions: {}", e);
                    }
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::commands::db_execute,
            db::commands::db_execute_batch,
            db::commands::db_select,
            db::commands::db_select_rows,
            db::commands::db_browser_list_tables,
            db::commands::db_browser_describe_table,
            db::commands::db_browser_query_rows,
            ocr::commands::extract_text,
            ocr::commands::test_glm_ocr_connection,
            ocr::commands::update_extraction_text_cmd,
            ocr::commands::generate_pdf_thumbnail,
            ocr::commands::generate_image_thumbnail,
            ocr::commands::delete_pdf_thumbnail,
            ocr::commands::delete_image_thumbnail,
            ocr::commands::is_scanned_pdf,
            ocr::commands::probe_pdf,
            ocr::commands::render_pdf_pages,
            nlp::commands::index_fts,
            nlp::commands::embed_asset,
            nlp::commands::backfill_asset_embeddings,
            nlp::commands::extract_entities,
            nlp::commands::extract_entities_for_asset,
            nlp::commands::extract_triples,
            nlp::commands::extract_triples_for_asset,
            nlp::commands::enrich_item,
            nlp::commands::fts_search,
            nlp::commands::similar_assets,
            nlp::commands::embedding_local_model_info,
            nlp::commands::embedding_open_models_dir,
            nlp::commands::embedding_download_model,
            transcription::commands::transcribe_audio,
            transcription::commands::transcribe_dictation,
            transcription::commands::test_assemblyai_connection,
            transcription::commands::update_transcription_text_cmd,
            audio_preview::prepare_audio_preview,
            llm::commands::llm_correct_ocr,
            llm::commands::llm_extract_entities,
            llm::commands::llm_extract_triples,
            llm::commands::llm_summarize,
            llm::commands::llm_classify,
            llm::commands::llm_ask,
            llm::commands::llm_correct_ocr_asset,
            llm::commands::llm_extract_entities_asset,
            llm::commands::llm_extract_triples_asset,
            llm::commands::llm_summarize_asset,
            llm::commands::llm_get_results,
            llm::commands::llm_get_result,
            llm::commands::llm_is_available,
            llm::commands::llm_local_model_info,
            llm::commands::llm_open_models_dir,
            llm::commands::llm_download_model,
            geo::commands::geocode_entity,
            geo::commands::geocode_item_entities,
            rag::commands::rag_ask,
            rag::commands::rag_list_conversations,
            rag::commands::rag_get_conversation,
            rag::commands::rag_delete_conversation,
            image_edit::crop_image,
            image_edit::rotate_image,
            image_edit::rotate_image_degrees,
            image_edit::erase_region,
            image_edit::delete_asset_files,
            settings::settings_get,
            settings::settings_set,
            settings::settings_get_all,
            settings::settings_delete,
            llm::commands::test_openrouter_connection,
            deps::deps_check_all,
            deps::deps_get_cached_statuses,
            deps::deps_install_all,
            deps::deps_install_one,
            deps::deps_get_uv_status,
            deps::deps_reset,
            runtime::runtime_get_status,
            runtime::runtime_get_bootstrap_plan,
            runtime::runtime_repair,
            app_logs::logs_get,
            app_logs::logs_clear,
            app_logs::logs_open_dir,
            open_external_url,
            sync::sync_ensure_capture,
            sync::sync_reverify_blobs,
            sync::session::sync_register_account,
            sync::session::sync_login,
            sync::session::sync_logout,
            sync::commands::sync_status,
            sync::commands::sync_now,
            sync::commands::sync_set_auto,
            sync::commands::sync_list_devices,
            sync::commands::sync_revoke_device,
            sync::commands::sync_list_conflicts,
            sync::commands::sync_ack_conflict,
            sync::commands::sync_get_usage,
            sync::commands::sync_list_plans,
            sync::commands::sync_request_plan_change,
            sync::commands::sync_list_notifications,
            sync::commands::sync_mark_notification_read,
            sync::commands::sync_delete_account,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Signal the sync engine to tear down cleanly on app exit (DESIGN
            // §3.1) instead of being killed mid-cycle.
            if let tauri::RunEvent::Exit = event {
                if let Some(engine) = app_handle.try_state::<sync::engine::SyncEngine>() {
                    engine.shutdown();
                }
            }
        });
}

fn migrate_legacy_app_dir(app_dir: &Path) -> Result<(), String> {
    let Some(parent_dir) = app_dir.parent() else {
        return Ok(());
    };

    let legacy_dir = parent_dir.join(LEGACY_APP_IDENTIFIER);
    if !legacy_dir.exists() || legacy_dir == app_dir {
        return Ok(());
    }

    let migration_marker = legacy_migration_marker_path(app_dir);
    if migration_marker.exists() {
        eprintln!(
            "[setup] legacy app dir merge already completed — skipping recursive scan: {}",
            legacy_dir.display()
        );
        return Ok(());
    }

    if !app_dir.exists() {
        fs::rename(&legacy_dir, app_dir).map_err(|error| {
            format!(
                "Failed to rename legacy app dir from {} to {}: {error}",
                legacy_dir.display(),
                app_dir.display()
            )
        })?;
        eprintln!(
            "[setup] migrated legacy app dir: {} -> {}",
            legacy_dir.display(),
            app_dir.display()
        );
        write_legacy_migration_marker(app_dir)?;
        return Ok(());
    }

    prefer_richer_legacy_database(&legacy_dir, app_dir)?;

    if legacy_merge_already_satisfied(&legacy_dir, app_dir) {
        write_legacy_migration_marker(app_dir)?;
        eprintln!(
            "[setup] legacy app dir already represented in current app dir — skipping recursive scan: {} -> {}",
            legacy_dir.display(),
            app_dir.display()
        );
        return Ok(());
    }

    copy_missing_recursive(&legacy_dir, app_dir)?;
    write_legacy_migration_marker(app_dir)?;
    eprintln!(
        "[setup] merged legacy app dir into current app dir: {} -> {}",
        legacy_dir.display(),
        app_dir.display()
    );
    Ok(())
}

fn legacy_migration_marker_path(app_dir: &Path) -> std::path::PathBuf {
    app_dir.join(LEGACY_MIGRATION_MARKER)
}

fn write_legacy_migration_marker(app_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(app_dir)
        .map_err(|error| format!("Failed to create directory {}: {error}", app_dir.display()))?;
    fs::write(legacy_migration_marker_path(app_dir), "merged\n").map_err(|error| {
        format!(
            "Failed to write legacy migration marker in {}: {error}",
            app_dir.display()
        )
    })
}

fn legacy_merge_already_satisfied(legacy_dir: &Path, app_dir: &Path) -> bool {
    let legacy_db = legacy_dir.join(SQLITE_BASENAME);
    let current_db = app_dir.join(SQLITE_BASENAME);

    if !legacy_db.exists() {
        return current_db.exists();
    }

    if !current_db.exists() {
        return false;
    }

    let legacy_score = sqlite_richness_score(&legacy_db).unwrap_or(0);
    let current_score = sqlite_richness_score(&current_db).unwrap_or(0);
    current_score >= legacy_score
}

fn prefer_richer_legacy_database(legacy_dir: &Path, app_dir: &Path) -> Result<(), String> {
    let legacy_db = legacy_dir.join(SQLITE_BASENAME);
    let current_db = app_dir.join(SQLITE_BASENAME);

    if !legacy_db.exists() {
        return Ok(());
    }

    if !current_db.exists() {
        copy_sqlite_bundle(&legacy_db, &current_db)?;
        eprintln!(
            "[setup] copied legacy sqlite bundle into new app dir: {} -> {}",
            legacy_db.display(),
            current_db.display()
        );
        return Ok(());
    }

    let legacy_score = sqlite_richness_score(&legacy_db).unwrap_or(0);
    let current_score = sqlite_richness_score(&current_db).unwrap_or(0);

    if legacy_score <= current_score {
        eprintln!(
            "[setup] keeping current sqlite bundle (current_score={}, legacy_score={})",
            current_score, legacy_score
        );
        return Ok(());
    }

    backup_sqlite_bundle(&current_db)?;
    remove_sqlite_bundle(&current_db)?;
    copy_sqlite_bundle(&legacy_db, &current_db)?;
    eprintln!(
        "[setup] restored richer legacy sqlite bundle (legacy_score={} > current_score={})",
        legacy_score, current_score
    );
    Ok(())
}

fn sqlite_richness_score(db_path: &Path) -> Option<u64> {
    let conn = Connection::open(db_path).ok()?;
    let mut score = 0_u64;
    for table in [
        "collections",
        "items",
        "assets",
        "notes",
        "extractions",
        "transcriptions",
        "entities",
        "triples",
        "annotations",
    ] {
        score += table_row_count(&conn, table).unwrap_or(0);
    }
    Some(score)
}

fn table_row_count(conn: &Connection, table: &str) -> Option<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
        .ok()
        .map(|count| count.max(0) as u64)
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1 LIMIT 1",
        rusqlite::params![table],
        |_row| Ok(true),
    )
    .unwrap_or(false)
}

fn copy_sqlite_bundle(from_db: &Path, to_db: &Path) -> Result<(), String> {
    let Some(parent) = to_db.parent() else {
        return Err(format!(
            "Target database path has no parent: {}",
            to_db.display()
        ));
    };
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;

    for (source, target) in sqlite_bundle_paths(from_db, to_db) {
        if !source.exists() {
            continue;
        }
        fs::copy(&source, &target).map_err(|error| {
            format!(
                "Failed to copy sqlite bundle file from {} to {}: {error}",
                source.display(),
                target.display()
            )
        })?;
    }
    Ok(())
}

fn remove_sqlite_bundle(db_path: &Path) -> Result<(), String> {
    for path in sqlite_bundle_members(db_path) {
        if !path.exists() {
            continue;
        }
        fs::remove_file(&path)
            .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
    }
    Ok(())
}

fn backup_sqlite_bundle(db_path: &Path) -> Result<(), String> {
    for path in sqlite_bundle_members(db_path) {
        if !path.exists() {
            continue;
        }
        let backup = backup_path(&path);
        if backup.exists() {
            continue;
        }
        fs::copy(&path, &backup).map_err(|error| {
            format!(
                "Failed to backup sqlite bundle file from {} to {}: {error}",
                path.display(),
                backup.display()
            )
        })?;
    }
    Ok(())
}

fn backup_path(path: &Path) -> std::path::PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("entropia.sqlite");
    path.with_file_name(format!("{file_name}.before-legacy-restore.bak"))
}

fn sqlite_bundle_paths(
    from_db: &Path,
    to_db: &Path,
) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    let from = sqlite_bundle_members(from_db);
    let to = sqlite_bundle_members(to_db);
    from.into_iter().zip(to).collect()
}

fn sqlite_bundle_members(db_path: &Path) -> Vec<std::path::PathBuf> {
    vec![
        db_path.to_path_buf(),
        db_path.with_file_name(format!(
            "{}-wal",
            db_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(SQLITE_BASENAME)
        )),
        db_path.with_file_name(format!(
            "{}-shm",
            db_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(SQLITE_BASENAME)
        )),
    ]
}

fn copy_missing_recursive(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to)
        .map_err(|error| format!("Failed to create directory {}: {error}", to.display()))?;

    for entry in fs::read_dir(from)
        .map_err(|error| format!("Failed to read directory {}: {error}", from.display()))?
    {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read directory entry in {}: {error}",
                from.display()
            )
        })?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());

        if source_path.is_dir() {
            copy_missing_recursive(&source_path, &target_path)?;
            continue;
        }

        if target_path.exists() {
            continue;
        }

        fs::copy(&source_path, &target_path).map_err(|error| {
            format!(
                "Failed to copy file from {} to {}: {error}",
                source_path.display(),
                target_path.display()
            )
        })?;
    }

    Ok(())
}

fn migrate_legacy_asset_paths(db_path: &Path, app_dir: &Path) -> Result<(), String> {
    let Some(parent_dir) = app_dir.parent() else {
        return Ok(());
    };

    let legacy_dir = parent_dir.join(LEGACY_APP_IDENTIFIER);
    if legacy_dir == app_dir {
        return Ok(());
    }

    let legacy_prefix = legacy_dir.to_string_lossy().to_string();
    let current_prefix = app_dir.to_string_lossy().to_string();

    let conn = Connection::open(db_path)
        .map_err(|error| format!("Failed to open database for asset-path migration: {error}"))?;

    let has_assets_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='assets' LIMIT 1",
            [],
            |_row| Ok(true),
        )
        .unwrap_or(false);

    if !has_assets_table {
        eprintln!("[setup] assets table not found — skipping legacy asset-path migration");
        return Ok(());
    }

    let has_path_column: bool = conn
        .prepare("SELECT path FROM assets LIMIT 0")
        .and_then(|mut stmt| {
            let _ = stmt.query_map([], |_| Ok(()));
            Ok(true)
        })
        .unwrap_or(false);

    if !has_path_column {
        eprintln!("[setup] assets.path column not found — skipping legacy asset-path migration");
        return Ok(());
    }

    conn.execute(
        "UPDATE assets SET path = REPLACE(path, ?1, ?2) WHERE path LIKE ?3",
        rusqlite::params![
            legacy_prefix,
            current_prefix,
            format!("{}%", legacy_dir.to_string_lossy())
        ],
    )
    .map_err(|error| format!("Failed to migrate asset paths from legacy app dir: {error}"))?;

    Ok(())
}

fn migration_applied(conn: &Connection, name: &str) -> Result<bool, String> {
    let has_migrations_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='_migrations' LIMIT 1",
            [],
            |_row| Ok(true),
        )
        .unwrap_or(false);

    if !has_migrations_table {
        return Ok(false);
    }

    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM _migrations WHERE name = ?1 LIMIT 1",
            rusqlite::params![name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to check migration '{name}': {e}"))?;

    Ok(found.is_some())
}

/// Migrate the `extractions` table to remove the legacy CHECK constraint
/// on the `method` column that only allowed 'native' and 'ocr'.
/// PaddleOCR uses methods like 'paddle', 'paddle_vl', 'pdf_paddle', and 'pdf_paddle_vl'.
/// SQLite doesn't support ALTER TABLE DROP CONSTRAINT, so we recreate the table.
fn migrate_extractions_method_check(conn: &Connection) -> Result<(), String> {
    // Check if the CHECK constraint exists by attempting an insert with a new method value.
    // If it succeeds, no migration needed.
    let has_check: bool = conn
        .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='extractions'")
        .and_then(|mut stmt| {
            stmt.query_row([], |row| {
                let sql: String = row.get(0)?;
                Ok(sql.contains("CHECK(method IN"))
            })
        })
        .unwrap_or(false);

    if !has_check {
        eprintln!(
            "[setup] extractions.method: no legacy CHECK constraint found — skipping migration"
        );
        return Ok(());
    }

    eprintln!("[setup] Migrating extractions table to remove legacy method CHECK constraint...");

    conn.execute_batch(
        "BEGIN TRANSACTION;
         CREATE TABLE extractions_new (
           id TEXT PRIMARY KEY,
           asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
           text_content TEXT NOT NULL,
           method TEXT NOT NULL,
           confidence REAL,
           created_at INTEGER NOT NULL
         );
         INSERT INTO extractions_new SELECT * FROM extractions;
         DROP TABLE extractions;
         ALTER TABLE extractions_new RENAME TO extractions;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_extractions_asset_id_unique ON extractions(asset_id);
         COMMIT;",
    )
    .map_err(|e| format!("Failed to migrate extractions table: {e}"))?;

    eprintln!("[setup] extractions.method CHECK constraint removed successfully");
    Ok(())
}

fn ensure_layouts_schema(conn: &Connection) -> Result<(), String> {
    let has_layouts_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='layouts' LIMIT 1",
            [],
            |_row| Ok(true),
        )
        .unwrap_or(false);

    if !has_layouts_table {
        conn.execute_batch(
            "CREATE TABLE layouts (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                regions TEXT NOT NULL,
                blocks TEXT NOT NULL,
                model TEXT NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id);",
        )
        .map_err(|e| format!("Failed to create layouts table: {e}"))?;
        eprintln!("[setup] layouts table created with blocks column");
        return Ok(());
    }

    let mut stmt = conn
        .prepare("PRAGMA table_info(layouts)")
        .map_err(|e| format!("Failed to inspect layouts schema: {e}"))?;

    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("Failed to read layouts schema: {e}"))?;

    let mut has_blocks = false;
    for column in columns {
        if column.map_err(|e| format!("Failed to read layouts column: {e}"))? == "blocks" {
            has_blocks = true;
            break;
        }
    }
    drop(stmt);

    if !has_blocks {
        eprintln!("[setup] Migrating legacy layouts table to add blocks column...");
        conn.execute_batch(
            "BEGIN TRANSACTION;
             CREATE TABLE layouts_new (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                regions TEXT NOT NULL,
                blocks TEXT NOT NULL,
                model TEXT NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
             );
             INSERT INTO layouts_new (
                id, asset_id, regions, blocks, model, image_width, image_height, created_at
             )
             SELECT
                id,
                asset_id,
                regions,
                '[]' AS blocks,
                model,
                image_width,
                image_height,
                created_at
             FROM layouts;
             DROP TABLE layouts;
             ALTER TABLE layouts_new RENAME TO layouts;
             COMMIT;",
        )
        .map_err(|e| format!("Failed to migrate layouts table: {e}"))?;
    }

    conn.execute_batch(
        "DELETE FROM layouts
         WHERE rowid NOT IN (
           SELECT MAX(rowid) FROM layouts GROUP BY asset_id
         );
         CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id);",
    )
    .map_err(|e| format!("Failed to finalize layouts schema: {e}"))?;

    eprintln!("[setup] layouts schema ensured");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_external_url_accepts_http_and_https_urls() {
        validate_external_url("https://github.com/HumaLab/EntropIA-Pro?tab=readme").unwrap();
        validate_external_url("https://example.com/search?q=one&lang=es#results").unwrap();
        validate_external_url("http://localhost:1420/docs").unwrap();
    }

    #[test]
    fn validate_external_url_rejects_non_http_schemes() {
        assert!(validate_external_url("file:///C:/Users/user/secrets.txt").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn validate_external_url_rejects_shell_metacharacters_and_whitespace() {
        assert!(validate_external_url("https://example.com|calc.exe").is_err());
        assert!(validate_external_url(" https://example.com").is_err());
        assert!(validate_external_url("https://example.com/path with spaces").is_err());
    }

    #[test]
    fn migrate_extractions_method_check_removes_legacy_check_and_preserves_upsert_target() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY);
             INSERT INTO assets(id) VALUES ('asset-1');
             CREATE TABLE extractions (
               id TEXT PRIMARY KEY,
               asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
               text_content TEXT NOT NULL,
               method TEXT NOT NULL CHECK(method IN ('native', 'ocr')),
               confidence REAL,
               created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_extractions_asset_id ON extractions(asset_id);",
        )
        .expect("create legacy schema");

        migrate_extractions_method_check(&conn).expect("migrate extractions schema");

        let create_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='extractions'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated table sql");
        assert!(!create_sql.contains("CHECK(method IN"));

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
             VALUES ('ext-1', 'asset-1', 'first', 'paddle_vl', NULL, 1)
             ON CONFLICT(asset_id) DO UPDATE SET
               text_content = excluded.text_content,
               method = excluded.method,
               confidence = excluded.confidence,
               created_at = excluded.created_at",
            [],
        )
        .expect("insert modern OCR method");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
             VALUES ('ext-2', 'asset-1', 'updated', 'pdf_paddle_vl', NULL, 2)
             ON CONFLICT(asset_id) DO UPDATE SET
               text_content = excluded.text_content,
               method = excluded.method,
               confidence = excluded.confidence,
               created_at = excluded.created_at",
            [],
        )
        .expect("upsert by asset_id");

        let (text, method, count): (String, String, i64) = conn
            .query_row(
                "SELECT text_content, method, (SELECT COUNT(*) FROM extractions) FROM extractions WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read upserted extraction");
        assert_eq!(text, "updated");
        assert_eq!(method, "pdf_paddle_vl");
        assert_eq!(count, 1);
    }
}
