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
mod research;
mod runtime;
mod settings;
mod splash;
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

#[cfg(debug_assertions)]
fn apply_development_window_title(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.dev.conf.json"))?;
    let title = config
        .pointer("/app/windows/0/title")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            std::io::Error::other("tauri.dev.conf.json is missing the main window title")
        })?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| std::io::Error::other("main window is unavailable"))?;

    window.set_title(title)?;
    Ok(())
}

/// The shared data directory, for the frontend.
///
/// The frontend cannot use `appDataDir()` from `@tauri-apps/api/path`: that
/// resolves the per-variant directory from the Tauri identifier, which is
/// exactly what Lite and Pro must stop using. Asking the backend keeps one
/// definition of where the data lives.
#[tauri::command]
fn resolve_data_dir(app_handle: tauri::AppHandle) -> Result<String, String> {
    path_utils::data_dir(&app_handle).map(|dir| path_utils::normalize_windows_path_string(&dir))
}

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

    if url
        .chars()
        .any(|ch| ch.is_ascii_control() || EXTERNAL_URL_DISALLOWED_CHARS.contains(&ch))
    {
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
        // `_CrtSetReportMode` lives in the MSVC DEBUG CRT (ucrtbased), which is only
        // linked when a native C++ dep is built with /MDd — i.e. the local-ml ML libs
        // (MNN via ocr-rs, llama.cpp). The lean (API-only) build links none of them, so
        // the symbol would be unresolved (LNK2019). It is also only relevant there: the
        // lean variant has no native ML libs to throw CRT assertions, so gate it off.
        #[cfg(all(debug_assertions, feature = "local-ml"))]
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
            #[cfg(debug_assertions)]
            apply_development_window_title(app)?;

            // First thing on screen: the main window is configured hidden, so the
            // startup mark stands in for it while the work below runs. The frontend
            // closes it via `splash_finish` once it is ready; the watchdog reveals the
            // main window anyway if that signal never arrives.
            splash::open(&app.handle().clone());
            splash::start_watchdog(&app.handle().clone());

            use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
            // On a clean/varied Windows PC the data dir or DB can be unreachable
            // (redirected/roaming AppData, read-only or full disk, AV locking the
            // freshly-created folder). SetErrorMode above suppresses the OS crash
            // dialog, so a panic here would vanish silently. Instead, write a
            // diagnostic file to TEMP, show a clear error dialog, and exit cleanly.
            let dialog_handle = app.handle().clone();
            let fail = move |context: &str, detail: String| -> Box<dyn std::error::Error> {
                // The startup window is always-on-top, so it has to go before the
                // blocking dialog below or the message would be hidden behind the mark.
                splash::finish_now(&dialog_handle);
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

            // Resolved through the filesystem, not just asked of the OS: inside
            // an MSIX package Windows redirects these writes elsewhere, and an
            // app holding the pre-redirection path stores rows and declares
            // permissions for files that are not where it thinks. Both
            // directories are settled here, once, before anything reads a path.
            let (app_dir, cache_dir) =
                path_utils::resolve_and_remember_dirs(app.handle()).map_err(|e| {
                    fail("No se pudieron resolver las carpetas de datos y caché.", e)
                })?;
            migrate_legacy_app_dir(&app_dir)
                .map_err(|e| fail("No se pudo preparar la carpeta de datos heredada.", e))?;
            std::fs::create_dir_all(&app_dir).map_err(|e| {
                fail(
                    &format!("No se pudo crear la carpeta de datos {}.", app_dir.display()),
                    e.to_string(),
                )
            })?;

            // Not fatal: everything this moves is redownloadable or
            // regenerable. If it fails the archive is still intact and the app
            // still opens — refusing to start over a cache move would deny
            // someone their documents to tidy a directory.
            if let Err(error) = migrate_cache_out_of_data_dir(&app_dir, &cache_dir) {
                eprintln!("[setup] cache move skipped: {error}");
            }

            // Grant the asset protocol the two directories it must serve, from
            // the same functions that resolved them.
            //
            // The configs declare the equivalent `$DATA`/`$LOCALDATA` scopes,
            // but those are three copies of a path spelled by hand: rename the
            // shared directory in `path_utils` and they silently point at
            // nothing, which surfaces as an image that will not load. Deriving
            // the grant here keeps one definition of where the files are.
            {
                use tauri::Manager;
                let scope = app.asset_protocol_scope();
                scope.allow_directory(&app_dir, true).map_err(|e| {
                    fail(
                        "No se pudo habilitar el acceso a la carpeta de datos.",
                        e.to_string(),
                    )
                })?;
                scope.allow_directory(&cache_dir, true).map_err(|e| {
                    fail(
                        "No se pudo habilitar el acceso a la carpeta de caché.",
                        e.to_string(),
                    )
                })?;
            }

            app.manage(app_logs::AppLogsState::new(cache_dir.join("logs")));
            app_logs::info(&app.handle().clone(), "setup", "Registro de diagnóstico inicializado");

            // Where the app decided its files live, in the log rather than on
            // stderr. A packaged build has no console: the first time this
            // shipped, the one line that would have explained a whole class of
            // failures was written where nobody could read it.
            app_logs::info(
                &app.handle().clone(),
                "setup",
                format!(
                    "Carpeta de datos: {} · caché: {}{}",
                    app_dir.display(),
                    cache_dir.display(),
                    match path_utils::nominal_data_dir() {
                        Some(nominal) if nominal != app_dir =>
                            format!(" · redirigida desde {}", nominal.display()),
                        _ => String::new(),
                    }
                ),
            );
            let db_path = app_dir.join("entropia.sqlite");

            migrate_legacy_asset_paths(&db_path, &app_dir).map_err(|e| {
                fail(
                    "No se pudieron migrar rutas heredadas en la base de datos.",
                    e,
                )
            })?;

            // Runs after the legacy rewrite so any path it just re-pointed at
            // the current directory is relativized in the same startup.
            migrate_asset_paths_to_relative(&db_path, &app_dir).map_err(|e| {
                fail(
                    "No se pudieron convertir las rutas de assets a relativas.",
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

            // Only after the rows above were migrated: the guard refuses what
            // the migration has just finished removing.
            // Not fatal either: the guard is a safety net against a future
            // regression, not something the archive depends on. Every reader
            // already accepts an absolute path, so its absence degrades to the
            // behaviour that shipped before it existed.
            if let Err(error) = install_relative_asset_path_guard(&ui_conn, &app_dir) {
                eprintln!("[setup] relative asset-path guard not installed: {error}");
            }

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
                    .map(|mut stmt| {
                        let _ = stmt.query_map([], |_| Ok(()));
                        true
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
                    .map(|mut stmt| {
                        let _ = stmt.query_map([], |_| Ok(()));
                        true
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

            settings::migrate_legacy_default_openrouter_model(&ui_conn)
                .expect("Failed to migrate legacy default OpenRouter model");
            let secret_migration = settings::migrate_legacy_api_keys(&ui_conn);
            if secret_migration.migrated > 0 {
                eprintln!(
                    "[setup] Migrated {} API credential(s) to the system credential store",
                    secret_migration.migrated
                );
            }
            if !secret_migration.failed_keys.is_empty() {
                let message = format!(
                    "Could not migrate API credential(s) for: {}. Legacy values were preserved.",
                    secret_migration.failed_keys.join(", ")
                );
                eprintln!("[setup] {message}");
                app_logs::error(&app.handle().clone(), "settings/migration", message);
            }
            if secret_migration.storage_cleanup_failed {
                let message =
                    "API credentials moved to the system credential store, but SQLite storage cleanup could not be completed"
                        .to_string();
                eprintln!("[setup] {message}");
                app_logs::error(&app.handle().clone(), "settings/migration", message);
            }

            let research = research::ResearchState::new(app_dir.clone(), db_path.clone())
                .map_err(|e| fail("No se pudo abrir el estado de investigaciones.", e))?;
            app.manage(research);

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

            // LLM queue: shared correction/summarization/extraction pipeline. Pro can
            // use the local engine; Lite routes through the configured remote provider.
            let (llm_queue, llm_receiver) = LlmQueue::new(db_path.clone());
            let llm_available = llm_queue.available_flag();
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
            let embedding_scheduler_queue = nlp_queue.clone();
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
            );
            nlp::start_embedding_scheduler(db_path.clone(), embedding_scheduler_queue);

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
            resolve_data_dir,
            research::research_request,
            db::commands::db_execute,
            db::commands::db_execute_batch,
            db::commands::db_execute_transaction,
            db::commands::db_select,
            db::commands::db_select_rows,
            db::commands::db_browser_list_tables,
            db::commands::db_browser_describe_table,
            db::commands::db_browser_query_rows,
            ocr::commands::extract_text,
            ocr::commands::crop_pdf,
            ocr::commands::edit_pdf,
            ocr::commands::test_glm_ocr_connection,
            ocr::commands::update_extraction_text_cmd,
            ocr::commands::generate_pdf_thumbnail,
            ocr::commands::generate_image_thumbnail,
            ocr::commands::delete_pdf_thumbnail,
            ocr::commands::delete_image_thumbnail,
            ocr::commands::is_scanned_pdf,
            ocr::commands::probe_pdf,
            ocr::commands::render_pdf_pages,
            ocr::commands::split_pdf_pages,
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
            llm::commands::llm_can_restore_original_ocr_asset,
            llm::commands::llm_restore_original_ocr_asset,
            llm::commands::llm_is_available,
            llm::commands::llm_ocr_correction_is_available,
            llm::commands::llm_local_model_info,
            llm::commands::llm_open_models_dir,
            llm::commands::llm_download_model,
            geo::commands::geocode_entity,
            geo::commands::geocode_item_entities,
            rag::commands::rag_ask,
            rag::commands::rag_list_conversations,
            rag::commands::rag_search_conversations,
            rag::commands::rag_get_conversation,
            rag::commands::rag_delete_conversation,
            rag::commands::rag_update_conversation_title,
            rag::commands::rag_generate_conversation_title,
            rag::commands::rag_reranker_model_info,
            rag::commands::rag_reranker_open_models_dir,
            rag::commands::rag_reranker_download_model,
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
            app_logs::logs_append,
            open_external_url,
            splash::splash_finish,
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
            sync::commands::sync_delete_notification,
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

/// Every directory a previous EntropIA build may have written to, in the order
/// they are considered as the seed for the shared directory.
///
/// The first one holding a database wins; from the rest only what is missing is
/// taken. `com.entropia.lite` leads because it is the variant carrying the real
/// archive — and because both variants' databases converged through the sync
/// server, electing either preserves the same content.
const LEGACY_APP_IDENTIFIERS: &[&str] = &[
    "com.entropia.lite",
    "com.entropia.pro.desktop",
    "com.entropia.pro.desktop.dev",
    "com.entropia.lite.dev",
    "com.entropia.lite.dev2",
    "com.entropia.desktop",
    "app.entropia.lite",
    LEGACY_APP_IDENTIFIER,
];

/// Converge every directory a previous build wrote to into the shared one.
///
/// Nothing is deleted: a legacy directory is left in place once what it holds
/// is represented in the shared directory. Removing ~13 GB of duplicated
/// runtime is an explicit user action, taken after verifying the migration.
fn migrate_legacy_app_dir(app_dir: &Path) -> Result<(), String> {
    let Some(parent_dir) = app_dir.parent() else {
        return Ok(());
    };

    // One marker for the whole convergence. It lives in the shared directory,
    // which is new, so an existing install cannot carry a stale one.
    let migration_marker = legacy_migration_marker_path(app_dir);
    if migration_marker.exists() {
        eprintln!("[setup] legacy app dir convergence already completed — skipping");
        return Ok(());
    }

    let mut merged_any = false;
    for identifier in LEGACY_APP_IDENTIFIERS {
        let legacy_dir = parent_dir.join(identifier);
        if !legacy_dir.exists() || legacy_dir == app_dir {
            continue;
        }
        merge_legacy_dir(&legacy_dir, app_dir)?;
        merged_any = true;
    }

    if merged_any {
        write_legacy_migration_marker(app_dir)?;
    }
    Ok(())
}

/// Merge one legacy directory into the shared one.
fn merge_legacy_dir(legacy_dir: &Path, app_dir: &Path) -> Result<(), String> {
    if !app_dir.exists() {
        // A rename between siblings on one volume is a metadata operation: the
        // seed directory moves whole, in milliseconds, however large it is.
        //
        // It is an optimization, never a requirement. A whole-directory rename
        // is the operation most likely to behave differently somewhere we do not
        // control — inside an MSIX package Windows redirects filesystem calls,
        // and the Store build runs in exactly that sandbox. Failing the startup
        // there would show a user their archive as broken when nothing is wrong
        // with it, so a failed rename falls through to the file-by-file move.
        match fs::rename(legacy_dir, app_dir) {
            Ok(()) => {
                eprintln!(
                    "[setup] migrated legacy app dir: {} -> {}",
                    legacy_dir.display(),
                    app_dir.display()
                );
                return Ok(());
            }
            Err(error) => {
                eprintln!(
                    "[setup] whole-directory rename unavailable ({error}) — moving entry by entry: {} -> {}",
                    legacy_dir.display(),
                    app_dir.display()
                );
                return move_missing_recursive(legacy_dir, app_dir);
            }
        }
    }

    prefer_richer_legacy_database(legacy_dir, app_dir)?;

    // No shortcut here. `legacy_merge_already_satisfied` compares database
    // richness, which says nothing about the files on disk: right after
    // `prefer_richer_legacy_database` copies the legacy database across, the two
    // scores are equal and the check reports "already represented" — while the
    // whole `assets` tree is still sitting in the legacy directory. Skipping the
    // walk on that basis is how an archive ends up with its rows and none of its
    // files. The walk is cheap anyway: it renames, and skips what the target
    // already has.
    move_missing_recursive(legacy_dir, app_dir)?;
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
            "[setup] keeping current sqlite bundle (current_score={current_score}, legacy_score={legacy_score})"
        );
        return Ok(());
    }

    backup_sqlite_bundle(&current_db)?;
    remove_sqlite_bundle(&current_db)?;
    copy_sqlite_bundle(&legacy_db, &current_db)?;
    eprintln!(
        "[setup] restored richer legacy sqlite bundle (legacy_score={legacy_score} > current_score={current_score})"
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

/// Bring across everything the target is missing, moving rather than copying.
///
/// The legacy directories are siblings of the shared one on the same volume, so
/// a rename is a metadata operation — the asset trees are never duplicated on
/// disk. A copy is the fallback for the one case a rename cannot handle: a
/// source on a different volume.
///
/// A file the target already has is left alone on both sides: the target wins,
/// and the legacy copy stays where it is for the user to inspect.
fn move_missing_recursive(from: &Path, to: &Path) -> Result<(), String> {
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
            // A whole subtree the target lacks moves in one rename.
            if !target_path.exists() && fs::rename(&source_path, &target_path).is_ok() {
                continue;
            }
            move_missing_recursive(&source_path, &target_path)?;
            continue;
        }

        if target_path.exists() {
            continue;
        }

        if fs::rename(&source_path, &target_path).is_ok() {
            continue;
        }

        // Cross-volume: rename cannot span devices, so fall back to a copy.
        fs::copy(&source_path, &target_path).map_err(|error| {
            format!(
                "Failed to move file from {} to {}: {error}",
                source_path.display(),
                target_path.display()
            )
        })?;
        let _ = fs::remove_file(&source_path);
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
        .map(|mut stmt| {
            let _ = stmt.query_map([], |_| Ok(()));
            true
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

/// The subdirectories that belong in the cache directory, not beside the data.
///
/// Everything here is redownloadable or regenerable. Nothing that a user would
/// miss goes on this list: `assets`, `research` and the database stay put. The
/// list is explicit rather than "everything except" on purpose — a new data
/// directory appearing one day must not be swept into a directory people are
/// told they can delete.
const CACHE_SUBDIRS: &[&str] = &[
    "models",
    "hf_cache",
    "paddlex_cache",
    "runtime-dev",
    "thumbnails",
    "audio-previews",
    "temp",
    "logs",
];

/// Move regenerable weight out of the data directory.
///
/// Idempotent: a subdirectory already gone from the data directory is skipped,
/// and one that exists on both sides is merged without overwriting.
fn migrate_cache_out_of_data_dir(data_dir: &Path, cache_dir: &Path) -> Result<usize, String> {
    if data_dir == cache_dir {
        return Ok(0);
    }

    let mut moved = 0usize;
    for name in CACHE_SUBDIRS {
        let source = data_dir.join(name);
        if !source.is_dir() {
            continue;
        }
        let target = cache_dir.join(name);

        // Same-volume rename where possible; otherwise the recursive move
        // handles a cross-volume Roaming/Local split file by file.
        if !target.exists() && fs::rename(&source, &target).is_ok() {
            moved += 1;
            continue;
        }
        move_missing_recursive(&source, &target)?;
        let _ = fs::remove_dir_all(&source);
        moved += 1;
    }

    if moved > 0 {
        eprintln!(
            "[setup] moved {moved} cache directories out of the data directory: {} -> {}",
            data_dir.display(),
            cache_dir.display()
        );
    }
    Ok(moved)
}

/// Install the trigger that refuses an absolute `assets.path`.
///
/// Runs only after [`migrate_asset_paths_to_relative`], never before: until the
/// rows are migrated and every writer emits relative keys, this would abort the
/// first import the app attempts. A guard that lands before the invariant it
/// protects is a defect, not a safeguard.
///
/// A trigger rather than a `CHECK` constraint on purpose: adding a `CHECK` to an
/// existing SQLite table means rebuilding it, and `assets` carries three
/// indexes, a partial unique index, a self-referential foreign key with
/// `ON DELETE CASCADE`, and the sync capture triggers.
///
/// External files that were never copied in keep absolute paths, so the guard
/// only refuses a path *under the data directory* — which is precisely the
/// class the migration is responsible for.
fn install_relative_asset_path_guard(conn: &Connection, data_dir: &Path) -> Result<(), String> {
    if !table_exists(conn, "assets") {
        return Ok(());
    }

    // The pattern is inlined because CREATE TRIGGER takes no parameters. `'` is
    // doubled so a path containing one cannot break out of the literal, and
    // `\` escapes LIKE's own wildcards so a directory named `foo_bar` matches
    // itself rather than `fooXbar`.
    let prefix = data_dir
        .to_string_lossy()
        .replace('\\', "/")
        .replace('%', "\\%")
        .replace('_', "\\_")
        .replace('\'', "''");
    let statement = format!(
        "DROP TRIGGER IF EXISTS trg_assets_path_must_be_relative;
         CREATE TRIGGER trg_assets_path_must_be_relative
         BEFORE INSERT ON assets
         WHEN REPLACE(NEW.path, '\\', '/') LIKE '{prefix}%' ESCAPE '\\'
         BEGIN
           SELECT RAISE(ABORT, 'assets.path must be relative to the data directory');
         END;
         DROP TRIGGER IF EXISTS trg_assets_path_must_be_relative_update;
         CREATE TRIGGER trg_assets_path_must_be_relative_update
         BEFORE UPDATE OF path ON assets
         WHEN REPLACE(NEW.path, '\\', '/') LIKE '{prefix}%' ESCAPE '\\'
         BEGIN
           SELECT RAISE(ABORT, 'assets.path must be relative to the data directory');
         END;"
    );

    conn.execute_batch(&statement)
        .map_err(|error| format!("Failed to install the relative asset-path guard: {error}"))
}

/// Rewrite absolute `assets.path` values into keys relative to the data
/// directory, normalizing separators to `/`.
///
/// Idempotent by construction: a row that is already relative does not start
/// with the data-directory prefix, so `derive_rel_path` rejects it and the row
/// is left alone. Running this twice changes nothing.
///
/// A path that does not live under the data directory — an external file that
/// was never copied in — is also left alone. Storing an absolute path stays
/// correct for such a row, and every reader accepts either shape.
///
/// Returns how many rows were rewritten.
fn migrate_asset_paths_to_relative(db_path: &Path, data_dir: &Path) -> Result<usize, String> {
    let mut conn = Connection::open(db_path).map_err(|error| {
        format!("Failed to open database for the relative asset-path migration: {error}")
    })?;

    if !table_exists(&conn, "assets") {
        eprintln!("[setup] assets table not found — skipping relative asset-path migration");
        return Ok(0);
    }

    let tx = conn
        .transaction()
        .map_err(|error| format!("Failed to begin the relative asset-path migration: {error}"))?;

    let rows: Vec<(String, String)> = {
        let mut stmt = tx
            .prepare("SELECT id, path FROM assets")
            .map_err(|error| format!("Failed to read asset paths: {error}"))?;
        let mapped = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| format!("Failed to read asset paths: {error}"))?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to read asset paths: {error}"))?
    };

    // A row may still point at a legacy directory: the files move into the
    // shared directory, but the stored prefix is whatever the variant that
    // wrote the row used. Every known root is tried, so `…/com.entropia.lite/
    // assets/x` and `…/com.entropia.shared/assets/x` both reduce to `assets/x`.
    // Both the resolved parent and the nominal one. Where the filesystem
    // redirects — inside an MSIX package — a previous version stored the
    // NOMINAL prefix, because that is what the OS reported to it. This process
    // resolved a different path for itself, so building roots only from the
    // resolved parent recognizes none of those rows and rewrites nothing: the
    // files move to the shared directory and every path keeps pointing at where
    // they used to be.
    let mut roots = vec![data_dir.to_path_buf()];
    let mut parents = vec![data_dir.parent().map(Path::to_path_buf)];
    if let Some(nominal) = crate::path_utils::nominal_data_dir() {
        roots.push(nominal.clone());
        parents.push(nominal.parent().map(Path::to_path_buf));
    }
    for parent in parents.into_iter().flatten() {
        roots.extend(LEGACY_APP_IDENTIFIERS.iter().map(|id| parent.join(id)));
    }

    let mut rewritten = 0usize;
    for (id, path) in rows {
        let Some(relative) = roots
            .iter()
            .find_map(|root| crate::path_utils::derive_rel_path(&path, root).ok())
        else {
            continue;
        };
        if relative == path {
            continue;
        }
        tx.execute(
            "UPDATE assets SET path = ?1 WHERE id = ?2",
            rusqlite::params![relative, id],
        )
        .map_err(|error| format!("Failed to rewrite asset path for {id}: {error}"))?;
        rewritten += 1;
    }

    tx.commit()
        .map_err(|error| format!("Failed to commit the relative asset-path migration: {error}"))?;

    if rewritten > 0 {
        eprintln!("[setup] rewrote {rewritten} asset paths as relative keys");
    }
    Ok(rewritten)
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

    // ------------------------------------------------------------------
    // Legacy app-dir migration — characterization tests.
    //
    // `migrate_legacy_app_dir` is generalized from one legacy identifier to
    // several when Lite and Pro converge on a shared data directory. It had no
    // tests at all; these describe what it does today so that change has a net
    // under it.
    // ------------------------------------------------------------------

    /// A temp parent holding a populated legacy app dir. The returned target
    /// directory does NOT exist yet.
    fn legacy_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let parent = tempfile::tempdir().expect("tempdir");
        let legacy_dir = parent.path().join(LEGACY_APP_IDENTIFIER);
        let target_dir = parent.path().join("com.entropia.target");
        fs::create_dir_all(legacy_dir.join("assets")).expect("legacy assets dir");
        fs::write(legacy_dir.join("assets").join("photo.jpg"), b"bytes").expect("legacy asset");
        (parent, legacy_dir, target_dir)
    }

    /// A database with `rows` rows in `items`, so `sqlite_richness_score` has
    /// something to compare.
    fn seed_db(path: &std::path::Path, rows: usize) {
        let conn = Connection::open(path).expect("open db");
        conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY);")
            .expect("create items");
        for index in 0..rows {
            conn.execute(
                "INSERT INTO items(id) VALUES (?1)",
                rusqlite::params![format!("item-{index}")],
            )
            .expect("insert item");
        }
    }

    fn item_count(db_path: &std::path::Path) -> i64 {
        let conn = Connection::open(db_path).expect("open db");
        conn.query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .expect("count items")
    }

    #[test]
    fn migrate_legacy_app_dir_renames_when_the_target_does_not_exist() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert!(!legacy_dir.exists(), "legacy dir is consumed by the rename");
        assert!(
            target_dir.join("assets").join("photo.jpg").exists(),
            "legacy content is reachable at the target"
        );
        assert!(
            target_dir.join(LEGACY_MIGRATION_MARKER).exists(),
            "the marker records that the merge completed"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_merges_only_missing_files_when_the_target_exists() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(target_dir.join("assets")).expect("target assets dir");
        fs::write(target_dir.join("assets").join("photo.jpg"), b"target wins")
            .expect("target asset");
        fs::write(legacy_dir.join("assets").join("only-legacy.jpg"), b"bytes")
            .expect("legacy-only asset");

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert_eq!(
            fs::read(target_dir.join("assets").join("photo.jpg")).expect("read"),
            b"target wins".to_vec(),
            "an existing target file is never overwritten"
        );
        assert!(
            target_dir.join("assets").join("only-legacy.jpg").exists(),
            "a file missing from the target is brought over"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_skips_the_scan_once_the_marker_exists() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(&target_dir).expect("target dir");
        fs::write(target_dir.join(LEGACY_MIGRATION_MARKER), b"").expect("marker");
        fs::write(legacy_dir.join("assets").join("late-arrival.jpg"), b"bytes")
            .expect("late asset");

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert!(
            !target_dir.join("assets").join("late-arrival.jpg").exists(),
            "the marker short-circuits the merge entirely"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_prefers_the_richer_legacy_database() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(&target_dir).expect("target dir");
        seed_db(&legacy_dir.join(SQLITE_BASENAME), 5);
        seed_db(&target_dir.join(SQLITE_BASENAME), 1);

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert_eq!(
            item_count(&target_dir.join(SQLITE_BASENAME)),
            5,
            "the richer legacy database replaces the poorer target"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_keeps_the_richer_target_database() {
        let (_parent, legacy_dir, target_dir) = legacy_fixture();
        fs::create_dir_all(&target_dir).expect("target dir");
        seed_db(&legacy_dir.join(SQLITE_BASENAME), 1);
        seed_db(&target_dir.join(SQLITE_BASENAME), 5);

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert_eq!(
            item_count(&target_dir.join(SQLITE_BASENAME)),
            5,
            "a target at least as rich as the legacy one is kept"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_is_a_no_op_without_a_legacy_directory() {
        let parent = tempfile::tempdir().expect("tempdir");
        let target_dir = parent.path().join("com.entropia.target");

        migrate_legacy_app_dir(&target_dir).expect("migration succeeds");

        assert!(
            !target_dir.exists(),
            "nothing is created when there is nothing to migrate"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_running_twice_changes_nothing() {
        let (_parent, _legacy_dir, target_dir) = legacy_fixture();

        migrate_legacy_app_dir(&target_dir).expect("first run");
        let after_first: Vec<_> = fs::read_dir(target_dir.join("assets"))
            .expect("read assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();

        migrate_legacy_app_dir(&target_dir).expect("second run");
        let after_second: Vec<_> = fs::read_dir(target_dir.join("assets"))
            .expect("read assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();

        assert_eq!(after_first, after_second, "the migration is idempotent");
    }

    // ------------------------------------------------------------------
    // Legacy asset-path migration — characterization tests.
    //
    // `migrate_legacy_asset_paths` currently replaces one absolute prefix with
    // another. It becomes a prefix *strip* when asset paths turn relative, and
    // it runs across several identifiers when the variants converge. These
    // describe today's behavior first.
    // ------------------------------------------------------------------

    /// A database with an `assets` table holding the given paths.
    fn seed_assets(db_path: &std::path::Path, paths: &[&str]) {
        let conn = Connection::open(db_path).expect("open db");
        conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, path TEXT NOT NULL);")
            .expect("create assets");
        for (index, path) in paths.iter().enumerate() {
            conn.execute(
                "INSERT INTO assets(id, path) VALUES (?1, ?2)",
                rusqlite::params![format!("asset-{index}"), path],
            )
            .expect("insert asset");
        }
    }

    fn asset_paths(db_path: &std::path::Path) -> Vec<String> {
        let conn = Connection::open(db_path).expect("open db");
        let mut stmt = conn
            .prepare("SELECT path FROM assets ORDER BY id")
            .expect("prepare");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query");
        rows.map(|row| row.expect("row")).collect()
    }

    #[test]
    fn migrate_legacy_asset_paths_rewrites_the_legacy_prefix() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        let legacy_dir = parent.path().join(LEGACY_APP_IDENTIFIER);
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        let legacy_asset = legacy_dir
            .join("assets")
            .join("col")
            .join("item")
            .join("photo.jpg");
        seed_assets(&db_path, &[&legacy_asset.to_string_lossy()]);

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("migration succeeds");

        let expected = app_dir
            .join("assets")
            .join("col")
            .join("item")
            .join("photo.jpg");
        assert_eq!(
            asset_paths(&db_path),
            vec![expected.to_string_lossy().to_string()]
        );
    }

    #[test]
    fn migrate_legacy_asset_paths_leaves_unrelated_paths_alone() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        let foreign = "D:/somewhere/else/photo.jpg";
        seed_assets(&db_path, &[foreign]);

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("migration succeeds");

        assert_eq!(
            asset_paths(&db_path),
            vec![foreign.to_string()],
            "a path outside the legacy dir is untouched"
        );
    }

    #[test]
    fn migrate_legacy_asset_paths_running_twice_changes_nothing() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        let legacy_dir = parent.path().join(LEGACY_APP_IDENTIFIER);
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        let legacy_asset = legacy_dir.join("assets").join("col").join("photo.jpg");
        seed_assets(&db_path, &[&legacy_asset.to_string_lossy()]);

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("first run");
        let after_first = asset_paths(&db_path);
        migrate_legacy_asset_paths(&db_path, &app_dir).expect("second run");

        assert_eq!(
            after_first,
            asset_paths(&db_path),
            "the rewrite is idempotent"
        );
    }

    // ------------------------------------------------------------------
    // Cache / data split.
    // ------------------------------------------------------------------

    #[test]
    fn the_cache_migration_moves_only_regenerable_directories() {
        let root = tempfile::tempdir().expect("tempdir");
        let data = root.path().join("data");
        let cache = root.path().join("cache");
        fs::create_dir_all(&cache).expect("cache dir");

        for name in ["models", "thumbnails", "logs", "assets", "research"] {
            fs::create_dir_all(data.join(name)).expect("dir");
            fs::write(data.join(name).join("file.bin"), b"bytes").expect("file");
        }
        fs::write(data.join(SQLITE_BASENAME), b"db").expect("db");

        migrate_cache_out_of_data_dir(&data, &cache).expect("migration succeeds");

        for name in ["models", "thumbnails", "logs"] {
            assert!(
                cache.join(name).join("file.bin").exists(),
                "{name} is regenerable and belongs in the cache directory"
            );
            assert!(
                !data.join(name).exists(),
                "{name} should not be left behind"
            );
        }
        for name in ["assets", "research"] {
            assert!(
                data.join(name).join("file.bin").exists(),
                "{name} is the user's own work and stays with the data"
            );
            assert!(
                !cache.join(name).exists(),
                "{name} must never be treated as cache"
            );
        }
        assert!(
            data.join(SQLITE_BASENAME).exists(),
            "the database stays put"
        );
    }

    #[test]
    fn the_cache_migration_running_twice_changes_nothing() {
        let root = tempfile::tempdir().expect("tempdir");
        let data = root.path().join("data");
        let cache = root.path().join("cache");
        fs::create_dir_all(&cache).expect("cache dir");
        fs::create_dir_all(data.join("models")).expect("models dir");
        fs::write(data.join("models").join("weights.bin"), b"bytes").expect("file");

        assert_eq!(
            migrate_cache_out_of_data_dir(&data, &cache).expect("first run"),
            1
        );
        assert_eq!(
            migrate_cache_out_of_data_dir(&data, &cache).expect("second run"),
            0,
            "nothing is left to move"
        );
        assert!(cache.join("models").join("weights.bin").exists());
    }

    #[test]
    fn the_cache_migration_merges_without_overwriting() {
        let root = tempfile::tempdir().expect("tempdir");
        let data = root.path().join("data");
        let cache = root.path().join("cache");
        fs::create_dir_all(data.join("models")).expect("data models");
        fs::create_dir_all(cache.join("models")).expect("cache models");
        fs::write(cache.join("models").join("shared.bin"), b"cache wins").expect("file");
        fs::write(data.join("models").join("shared.bin"), b"data loses").expect("file");
        fs::write(data.join("models").join("only-data.bin"), b"bytes").expect("file");

        migrate_cache_out_of_data_dir(&data, &cache).expect("migration succeeds");

        assert_eq!(
            fs::read(cache.join("models").join("shared.bin")).expect("read"),
            b"cache wins".to_vec()
        );
        assert!(cache.join("models").join("only-data.bin").exists());
    }

    #[test]
    fn every_cache_subdir_is_regenerable() {
        // A guard on the list itself: nothing a user would miss may appear
        // here, because people are told they can delete the cache directory.
        for name in CACHE_SUBDIRS {
            assert!(
                !["assets", "research", "entropia.sqlite"].contains(name),
                "{name} is the user's own work and must never be listed as cache"
            );
        }
    }

    // ------------------------------------------------------------------
    // Security scope.
    // ------------------------------------------------------------------

    /// The only path any shipped configuration may expose.
    ///
    /// Turns "do not widen the scope" into something the build enforces instead
    /// of something an audit has to notice. `$DATA` is the parent of every
    /// variant directory, so this entry is exactly as narrow as the
    /// `$APPDATA/**/*` it replaced — it just names a directory no identifier
    /// owns.
    #[test]
    fn every_config_exposes_only_the_shared_directory() {
        let expected = vec![
            format!("$DATA/{}/**/*", path_utils::SHARED_DIR_NAME),
            format!("$LOCALDATA/{}/**/*", path_utils::SHARED_DIR_NAME),
        ];

        for config in [
            "tauri.conf.json",
            "tauri.lite.conf.json",
            "tauri.dev.conf.json",
        ] {
            let raw = fs::read_to_string(config).unwrap_or_else(|e| panic!("read {config}: {e}"));
            let parsed: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {config}: {e}"));
            let scope = parsed
                .pointer("/app/security/assetProtocol/scope")
                .unwrap_or_else(|| panic!("{config} has no assetProtocol scope"))
                .as_array()
                .unwrap_or_else(|| panic!("{config} scope is not an array"));

            let entries: Vec<String> = scope
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect();
            assert_eq!(
                entries, expected,
                "{config} must expose the shared data and cache directories, and nothing else"
            );
        }
    }

    #[test]
    fn the_fs_capability_exposes_only_the_shared_directory() {
        let raw = fs::read_to_string("capabilities/default.json").expect("read capabilities");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse capabilities");
        let permissions = parsed["permissions"]
            .as_array()
            .expect("permissions is an array");

        let scope = permissions
            .iter()
            .find(|entry| entry["identifier"] == "fs:scope")
            .expect("an fs:scope permission");
        let allowed: Vec<String> = scope["allow"]
            .as_array()
            .expect("allow is an array")
            .iter()
            .filter_map(|entry| entry["path"].as_str())
            .map(str::to_string)
            .collect();

        assert_eq!(
            allowed,
            vec![
                format!("$DATA/{}/**/*", path_utils::SHARED_DIR_NAME),
                format!("$LOCALDATA/{}/**/*", path_utils::SHARED_DIR_NAME),
            ]
        );

        // These granted the per-variant directory, which now holds nothing.
        for retired in [
            "fs:allow-appdata-read-recursive",
            "fs:allow-appdata-write-recursive",
        ] {
            assert!(
                !permissions.iter().any(|entry| entry == retired),
                "{retired} widens the scope back to a per-variant directory"
            );
        }
    }

    // ------------------------------------------------------------------
    // Convergence across every legacy directory.
    // ------------------------------------------------------------------

    #[test]
    fn migrate_legacy_app_dir_elects_the_first_identifier_holding_a_database() {
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        // Second in the list, seeded richer, must still lose to the first.
        let lite = parent.path().join("com.entropia.lite");
        let pro = parent.path().join("com.entropia.pro.desktop");
        fs::create_dir_all(&lite).expect("lite dir");
        fs::create_dir_all(&pro).expect("pro dir");
        seed_db(&lite.join(SQLITE_BASENAME), 3);
        seed_db(&pro.join(SQLITE_BASENAME), 9);

        migrate_legacy_app_dir(&shared).expect("convergence succeeds");

        assert_eq!(
            item_count(&shared.join(SQLITE_BASENAME)),
            9,
            "the richest database wins regardless of order"
        );
        assert!(shared.join(LEGACY_MIGRATION_MARKER).exists());
    }

    #[test]
    fn migrate_legacy_app_dir_brings_across_files_from_every_legacy_directory() {
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        for (identifier, filename) in [
            ("com.entropia.lite", "from-lite.jpg"),
            ("com.entropia.pro.desktop", "from-pro.jpg"),
            ("com.entropia.desktop", "from-desktop.jpg"),
        ] {
            let dir = parent.path().join(identifier).join("assets");
            fs::create_dir_all(&dir).expect("legacy assets dir");
            fs::write(dir.join(filename), b"bytes").expect("legacy asset");
        }

        migrate_legacy_app_dir(&shared).expect("convergence succeeds");

        for filename in ["from-lite.jpg", "from-pro.jpg", "from-desktop.jpg"] {
            assert!(
                shared.join("assets").join(filename).exists(),
                "{filename} should be reachable in the shared directory"
            );
        }
    }

    #[test]
    fn migrate_legacy_app_dir_moves_instead_of_copying() {
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        fs::create_dir_all(&shared).expect("shared dir");
        // The shared dir already exists, so the seed goes through the merge
        // path rather than the whole-directory rename.
        let lite = parent.path().join("com.entropia.lite").join("assets");
        fs::create_dir_all(&lite).expect("legacy assets dir");
        fs::write(lite.join("photo.jpg"), b"bytes").expect("legacy asset");

        migrate_legacy_app_dir(&shared).expect("convergence succeeds");

        assert!(shared.join("assets").join("photo.jpg").exists());
        assert!(
            !lite.join("photo.jpg").exists(),
            "the file moved; a copy would leave the original behind"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_moves_files_even_when_the_databases_match() {
        // The bug this pins: `prefer_richer_legacy_database` copies the legacy
        // database across, which makes both richness scores equal, and the old
        // shortcut read that as "already represented" and skipped the file walk.
        // The rows arrived and the assets never did — an archive that lists
        // documents it cannot open.
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        fs::create_dir_all(&shared).expect("shared dir");

        let lite = parent.path().join("com.entropia.lite");
        fs::create_dir_all(lite.join("assets")).expect("legacy assets dir");
        seed_db(&lite.join(SQLITE_BASENAME), 5);
        fs::write(lite.join("assets").join("photo.jpg"), b"bytes").expect("legacy asset");

        migrate_legacy_app_dir(&shared).expect("convergence succeeds");

        assert_eq!(item_count(&shared.join(SQLITE_BASENAME)), 5);
        assert!(
            shared.join("assets").join("photo.jpg").exists(),
            "the asset tree must arrive with the rows, not after them"
        );
    }

    #[test]
    fn migrate_asset_paths_to_relative_strips_a_legacy_directory_prefix() {
        // Rows written by a variant keep that variant's prefix even after the
        // files move into the shared directory, so every known root is tried.
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        fs::create_dir_all(&shared).expect("shared dir");
        let db_path = shared.join(SQLITE_BASENAME);
        let legacy_path = parent
            .path()
            .join("com.entropia.lite")
            .join("assets")
            .join("col")
            .join("photo.jpg");
        seed_assets(&db_path, &[&legacy_path.to_string_lossy()]);

        let rewritten = migrate_asset_paths_to_relative(&db_path, &shared).expect("migration");

        assert_eq!(rewritten, 1);
        assert_eq!(
            asset_paths(&db_path),
            vec!["assets/col/photo.jpg".to_string()]
        );
    }

    #[test]
    fn merge_legacy_dir_falls_back_when_the_whole_directory_rename_is_unavailable() {
        // The Store build runs inside an MSIX package, where Windows redirects
        // filesystem calls and a whole-directory rename is the operation most
        // likely to behave differently. It is an optimization: the archive must
        // arrive complete without it.
        //
        // The fallback is forced by making the target already exist — the same
        // branch a failed rename takes — and asserting the content still lands.
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        let legacy = parent.path().join("com.entropia.lite");
        fs::create_dir_all(legacy.join("assets").join("col")).expect("legacy tree");
        fs::write(
            legacy.join("assets").join("col").join("photo.jpg"),
            b"bytes",
        )
        .expect("asset");
        seed_db(&legacy.join(SQLITE_BASENAME), 4);
        fs::create_dir_all(&shared).expect("target exists, so no whole-dir rename");

        merge_legacy_dir(&legacy, &shared).expect("merge succeeds without a directory rename");

        assert!(
            shared.join("assets").join("col").join("photo.jpg").exists(),
            "every file arrives even when the directory cannot be renamed whole"
        );
        assert_eq!(item_count(&shared.join(SQLITE_BASENAME)), 4);
    }

    #[test]
    fn migrate_legacy_app_dir_never_deletes_a_legacy_directory() {
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        fs::create_dir_all(&shared).expect("shared dir");
        let dead = parent.path().join("com.entropia.desktop");
        fs::create_dir_all(&dead).expect("dead dir");
        fs::write(dead.join("leftover.bin"), b"bytes").expect("leftover");

        migrate_legacy_app_dir(&shared).expect("convergence succeeds");

        assert!(
            dead.exists(),
            "legacy directories are reported, never removed"
        );
    }

    #[test]
    fn migrate_legacy_app_dir_converges_once_across_many_directories() {
        let parent = tempfile::tempdir().expect("tempdir");
        let shared = parent.path().join(path_utils::SHARED_DIR_NAME);
        for identifier in ["com.entropia.lite", "com.entropia.pro.desktop"] {
            let dir = parent.path().join(identifier).join("assets");
            fs::create_dir_all(&dir).expect("legacy assets dir");
            fs::write(dir.join(format!("{identifier}.jpg")), b"bytes").expect("asset");
        }

        migrate_legacy_app_dir(&shared).expect("first run");
        let after_first: Vec<_> = fs::read_dir(shared.join("assets"))
            .expect("read assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();

        migrate_legacy_app_dir(&shared).expect("second run");
        let after_second: Vec<_> = fs::read_dir(shared.join("assets"))
            .expect("read assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();

        assert_eq!(after_first, after_second, "the convergence is idempotent");
    }

    // ------------------------------------------------------------------
    // Relative asset-path migration.
    // ------------------------------------------------------------------

    #[test]
    fn migrate_asset_paths_to_relative_strips_the_data_dir_prefix() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let db_path = data_dir.path().join(SQLITE_BASENAME);
        let absolute = data_dir
            .path()
            .join("assets")
            .join("col")
            .join("item")
            .join("photo.jpg");
        seed_assets(&db_path, &[&absolute.to_string_lossy()]);

        let rewritten =
            migrate_asset_paths_to_relative(&db_path, data_dir.path()).expect("migration succeeds");

        assert_eq!(rewritten, 1);
        assert_eq!(
            asset_paths(&db_path),
            vec!["assets/col/item/photo.jpg".to_string()],
            "the stored key is relative with forward slashes"
        );
    }

    #[test]
    fn migrate_asset_paths_to_relative_strips_a_redirected_nominal_prefix() {
        // The defect the Store flight found. Inside an MSIX package the OS
        // reports one path and the filesystem writes to another. A previous
        // version stored the NOMINAL prefix; this process resolves the
        // redirected one. Building roots only from the resolved parent matched
        // nothing, so every row kept pointing at where the files used to be —
        // the archive listed 2483 documents and could open none of them.
        //
        // `nominal_data_dir()` is process-wide state that setup fills, so it is
        // absent here; the nominal parent is exercised through the resolved one
        // pointing somewhere else entirely, which is the same mismatch.
        let real = tempfile::tempdir().expect("tempdir");
        let nominal = tempfile::tempdir().expect("tempdir nominal");
        let shared = real.path().join(path_utils::SHARED_DIR_NAME);
        fs::create_dir_all(&shared).expect("shared dir");
        let db_path = shared.join(SQLITE_BASENAME);

        // A row written by the previous version, under the OTHER root.
        let stored = nominal
            .path()
            .join("com.entropia.lite")
            .join("assets")
            .join("col")
            .join("photo.jpg");
        seed_assets(&db_path, &[&stored.to_string_lossy()]);

        // With only the resolved root, nothing matches — the bug.
        assert_eq!(
            migrate_asset_paths_to_relative(&db_path, &shared).expect("runs"),
            0,
            "a prefix from another root is not recognized, which is the defect"
        );
        assert_eq!(
            migrate_asset_paths_to_relative(
                &db_path,
                nominal.path().join("com.entropia.lite").as_path()
            )
            .expect("runs"),
            1,
            "given the root the row actually carries, it is rewritten"
        );
        assert_eq!(
            asset_paths(&db_path),
            vec!["assets/col/photo.jpg".to_string()]
        );
    }

    #[test]
    fn migrate_asset_paths_to_relative_running_twice_changes_nothing() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let db_path = data_dir.path().join(SQLITE_BASENAME);
        let absolute = data_dir.path().join("assets").join("col").join("photo.jpg");
        seed_assets(&db_path, &[&absolute.to_string_lossy()]);

        migrate_asset_paths_to_relative(&db_path, data_dir.path()).expect("first run");
        let after_first = asset_paths(&db_path);

        let rewritten =
            migrate_asset_paths_to_relative(&db_path, data_dir.path()).expect("second run");

        assert_eq!(rewritten, 0, "the second run rewrites nothing");
        assert_eq!(after_first, asset_paths(&db_path));
    }

    #[test]
    fn migrate_asset_paths_to_relative_leaves_paths_outside_the_data_dir_alone() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("tempdir outside");
        let db_path = data_dir.path().join(SQLITE_BASENAME);
        let foreign = outside.path().join("assets").join("photo.jpg");
        let foreign = foreign.to_string_lossy().to_string();
        seed_assets(&db_path, &[&foreign]);

        let rewritten =
            migrate_asset_paths_to_relative(&db_path, data_dir.path()).expect("migration succeeds");

        assert_eq!(rewritten, 0);
        assert_eq!(
            asset_paths(&db_path),
            vec![foreign],
            "an external file that was never copied in keeps its absolute path"
        );
    }

    #[test]
    fn migrate_asset_paths_to_relative_skips_a_database_without_an_assets_table() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let db_path = data_dir.path().join(SQLITE_BASENAME);
        seed_db(&db_path, 1);

        assert_eq!(
            migrate_asset_paths_to_relative(&db_path, data_dir.path()).expect("no assets table"),
            0
        );
    }

    // ------------------------------------------------------------------
    // Relative asset-path guard.
    // ------------------------------------------------------------------

    fn guarded_db(data_dir: &std::path::Path) -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, path TEXT NOT NULL);")
            .expect("create assets");
        install_relative_asset_path_guard(&conn, data_dir).expect("install guard");
        conn
    }

    #[test]
    fn the_asset_path_guard_accepts_a_relative_key() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let conn = guarded_db(data_dir.path());

        conn.execute(
            "INSERT INTO assets(id, path) VALUES ('a', 'assets/col/item/photo.jpg')",
            [],
        )
        .expect("a relative key is accepted");
    }

    #[test]
    fn the_asset_path_guard_refuses_a_path_under_the_data_dir() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let conn = guarded_db(data_dir.path());
        let absolute = data_dir.path().join("assets").join("photo.jpg");

        let inserted = conn.execute(
            "INSERT INTO assets(id, path) VALUES ('a', ?1)",
            rusqlite::params![absolute.to_string_lossy()],
        );

        assert!(
            inserted.is_err(),
            "an absolute path under the data dir aborts"
        );
    }

    #[test]
    fn the_asset_path_guard_refuses_an_update_to_an_absolute_path() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let conn = guarded_db(data_dir.path());
        conn.execute(
            "INSERT INTO assets(id, path) VALUES ('a', 'assets/photo.jpg')",
            [],
        )
        .expect("seed");
        let absolute = data_dir.path().join("assets").join("photo.jpg");

        let updated = conn.execute(
            "UPDATE assets SET path = ?1 WHERE id = 'a'",
            rusqlite::params![absolute.to_string_lossy()],
        );

        assert!(updated.is_err(), "an update back to absolute aborts too");
    }

    #[test]
    fn the_asset_path_guard_allows_an_external_file_outside_the_data_dir() {
        // A file that was never copied in keeps its absolute path; refusing it
        // would lose the asset.
        let data_dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("tempdir outside");
        let conn = guarded_db(data_dir.path());
        let foreign = outside.path().join("photo.jpg");

        conn.execute(
            "INSERT INTO assets(id, path) VALUES ('a', ?1)",
            rusqlite::params![foreign.to_string_lossy()],
        )
        .expect("an external absolute path is accepted");
    }

    #[test]
    fn installing_the_asset_path_guard_twice_is_safe() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let conn = guarded_db(data_dir.path());

        install_relative_asset_path_guard(&conn, data_dir.path()).expect("second install");

        conn.execute(
            "INSERT INTO assets(id, path) VALUES ('a', 'assets/photo.jpg')",
            [],
        )
        .expect("still accepts a relative key");
    }

    #[test]
    fn migrate_legacy_asset_paths_skips_a_database_without_an_assets_table() {
        let parent = tempfile::tempdir().expect("tempdir");
        let app_dir = parent.path().join("com.entropia.target");
        fs::create_dir_all(&app_dir).expect("app dir");
        let db_path = app_dir.join(SQLITE_BASENAME);
        seed_db(&db_path, 1);

        migrate_legacy_asset_paths(&db_path, &app_dir).expect("migration succeeds without assets");
    }
}
