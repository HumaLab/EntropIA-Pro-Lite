use std::env;
use std::path::{Path, PathBuf};

/// Every command registered with `generate_handler!` in `src/lib.rs`. tauri-build
/// turns each into an `allow-<name>` / `deny-<name>` permission (underscores become
/// dashes), and the presence of this manifest is what makes Tauri check app
/// commands against the capabilities at all. tests/acl_manifest_guard.rs keeps this
/// list, the handler list and `capabilities/default.json` identical.
const APP_COMMANDS: &[&str] = &[
    "resolve_data_dir",
    "assets_check_integrity",
    "research_request",
    "db_execute",
    "db_execute_batch",
    "db_execute_transaction",
    "db_select",
    "db_select_rows",
    "db_browser_list_tables",
    "db_browser_describe_table",
    "db_browser_query_rows",
    "processing_initialize",
    "processing_prepare",
    "extraction_schemas_list",
    "extraction_schema_save",
    "extraction_schema_delete",
    "extraction_records_list",
    "processing_start",
    "processing_control",
    "processing_set_priority",
    "processing_retry",
    "processing_sync_bibliography_library",
    "processing_bibliography_sync_status",
    "processing_latest_bibliography_sync_status",
    "bibliography_search_works",
    "bibliography_open_passage",
    "bibliography_passage_context",
    "bibliography_library_status",
    "bibliography_search_passages",
    "bibliography_list_works",
    "bibliography_work_detail",
    "bibliography_open_work_attachment",
    "bibliography_reprocess_candidates",
    "bibliography_reprocess_candidates_cancel",
    "bibliography_reprocess_preview",
    "bibliography_reprocess_preview_cancel",
    "bibliography_reprocess_confirm",
    "processing_list_batches",
    "processing_get_batch",
    "processing_list_tasks",
    "processing_get_task",
    "writing_is_ready",
    "writing_publish_hlab",
    "writing_create_document",
    "writing_load_document",
    "writing_list_documents",
    "writing_sync_notices",
    "writing_save_document",
    "writing_rename_document",
    "writing_set_status",
    "writing_citations_for_asset",
    "writing_agent_actions",
    "writing_agent_record_suggestion",
    "writing_agent_ask",
    "writing_corpus_retrieve",
    "writing_agent_pending",
    "writing_agent_resolve",
    "writing_zotero_probe",
    "writing_zotero_cached",
    "writing_zotero_sync",
    "writing_zotero_search",
    "writing_zotero_known_libraries",
    "writing_zotero_check_library",
    "writing_zotero_item_detail",
    "writing_zotero_open_item",
    "writing_csl_render",
    "writing_csl_render_document",
    "writing_csl_bibliography",
    "writing_csl_validate_style",
    "writing_duplicate_document",
    "writing_append_journal",
    "writing_recovery_plan",
    "writing_prune_journal",
    "writing_discard_journal",
    "writing_snapshot_version",
    "writing_list_versions",
    "writing_read_version",
    "writing_restore_version",
    "writing_apply_retention",
    "extract_text",
    "crop_pdf",
    "edit_pdf",
    "test_glm_ocr_connection",
    "update_extraction_text_cmd",
    "generate_pdf_thumbnail",
    "generate_image_thumbnail",
    "delete_pdf_thumbnail",
    "delete_image_thumbnail",
    "is_scanned_pdf",
    "probe_pdf",
    "render_pdf_pages",
    "split_pdf_pages",
    "index_fts",
    "embed_asset",
    "backfill_asset_embeddings",
    "extract_entities",
    "extract_entities_for_asset",
    "extract_triples",
    "extract_triples_for_asset",
    "enrich_item",
    "fts_search",
    "similar_assets",
    "embedding_local_model_info",
    "embedding_open_models_dir",
    "embedding_download_model",
    "transcribe_audio",
    "transcribe_dictation",
    "test_assemblyai_connection",
    "zotero_verify_key",
    "update_transcription_text_cmd",
    "prepare_audio_preview",
    "llm_correct_ocr",
    "llm_extract_entities",
    "llm_extract_triples",
    "llm_summarize",
    "llm_classify",
    "llm_ask",
    "llm_correct_ocr_asset",
    "llm_extract_entities_asset",
    "llm_extract_triples_asset",
    "llm_summarize_asset",
    "llm_get_results",
    "llm_get_result",
    "llm_can_restore_original_ocr_asset",
    "llm_restore_original_ocr_asset",
    "llm_is_available",
    "llm_ocr_correction_is_available",
    "llm_local_model_info",
    "llm_open_models_dir",
    "llm_download_model",
    "geocode_entity",
    "geocode_item_entities",
    "rag_ask",
    "rag_list_conversations",
    "rag_search_conversations",
    "rag_get_conversation",
    "rag_delete_conversation",
    "rag_update_conversation_title",
    "rag_generate_conversation_title",
    "rag_reranker_model_info",
    "rag_reranker_open_models_dir",
    "rag_reranker_download_model",
    "crop_image",
    "rotate_image",
    "rotate_image_degrees",
    "erase_region",
    "delete_asset_files",
    "settings_get",
    "settings_set",
    "settings_get_all",
    "settings_delete",
    "test_openrouter_connection",
    "deps_check_all",
    "deps_get_cached_statuses",
    "deps_install_all",
    "deps_install_one",
    "deps_get_uv_status",
    "deps_reset",
    "runtime_get_status",
    "runtime_get_bootstrap_plan",
    "runtime_repair",
    "logs_get",
    "logs_clear",
    "logs_open_dir",
    "logs_append",
    "app_close_flushed",
    "open_external_url",
    "navegador_open",
    "navegador_navigate",
    "navegador_back",
    "navegador_forward",
    "navegador_reload",
    "navegador_new_tab",
    "navegador_activate_tab",
    "navegador_close_tab",
    "navegador_set_bounds",
    "navegador_set_visible",
    "navegador_close",
    "navegador_state",
    "navegador_capture_page",
    "navegador_capture_selection",
    "navegador_download_dir",
    "navegador_set_download_dir",
    "navegador_save_capture",
    "navegador_save_download",
    "navegador_discard_draft",
    "navegador_list_sources",
    "navegador_source_detail",
    "navegador_delete_source",
    "navegador_pdf_file",
    "navegador_copy_ticket",
    "navegador_zotero_copy_request",
    "navegador_zotero_copy_list",
    "navegador_zotero_copy_run",
    "navegador_zotero_copy_cancel",
    "navegador_zotero_launch",
    "navegador_zotero_libraries",
    "navegador_zotero_status",
    "check_microsoft_store_update",
    "splash_finish",
    "sync_ensure_capture",
    "sync_reverify_blobs",
    "sync_register_account",
    "sync_login",
    "sync_logout",
    "sync_status",
    "sync_now",
    "sync_full_resync",
    "sync_get_auto",
    "sync_set_auto",
    "sync_list_devices",
    "sync_revoke_device",
    "sync_list_conflicts",
    "sync_ack_conflict",
    "sync_ack_all_conflicts",
    "sync_get_usage",
    "sync_writing_shares",
    "sync_writing_share",
    "sync_writing_unshare",
    "sync_list_plans",
    "sync_request_plan_change",
    "sync_list_notifications",
    "sync_delete_notification",
    "sync_mark_notification_read",
    "sync_delete_account",
];

fn main() {
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=tauri.windows.conf.json");
    println!("cargo:rerun-if-changed=tauri.linux.conf.json");
    println!("cargo:rerun-if-changed=tauri.lite.conf.json");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/icon.icns");
    println!("cargo:rerun-if-env-changed=ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL");
    println!("cargo:rerun-if-env-changed=ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID");
    println!("cargo:rerun-if-env-changed=ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64");
    println!("cargo:rerun-if-changed=resources/runtime-pack/windows-x86_64/manifest.json");
    println!("cargo:rerun-if-changed=resources/runtime-pack/linux-x86_64/manifest.json");

    embed_common_controls_manifest_for_tests();
    guard_lean_bootstrap_source();
    ensure_windows_vc_runtime_glob_exists();
    stage_windows_vc_runtime();

    // tauri-build 2.7 links the VC runtime statically by default. Keep it
    // dynamic: the VC runtime DLLs are staged next to the binary above and the
    // native ML libraries use the dynamic CRT (see tests/vc_runtime_guard.rs).
    if let Err(error) = tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new().static_vc_runtime(false))
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    ) {
        panic!("tauri-build failed: {error:#}");
    }
}

/// The Tauri runtime imports `TaskDialogIndirect`, which only exists in Common
/// Controls v6. tauri-build embeds that manifest in the app binary, but a test
/// executable that links the runtime (tests/app_acl.rs) has none and dies at
/// load with STATUS_ENTRYPOINT_NOT_FOUND before running a single test. Scoped to
/// test targets so it can never collide with the manifest tauri-build embeds
/// in the binary.
fn embed_common_controls_manifest_for_tests() {
    let is_windows_msvc = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !is_windows_msvc {
        return;
    }
    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' \
         name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
         processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
    );
}

/// Fail a release build that would ship a lean/fixture runtime-pack with no baked
/// download source. Such an installer dead-ends on a clean machine
/// (`RuntimeState::BlockedSourceUnavailable`, no recovery), so it must never be
/// produced — not even by a plain local `cargo build --release` / `tauri build`.
/// Debug builds and macOS (which ships no runtime-pack) are unaffected.
fn guard_lean_bootstrap_source() {
    if env::var("PROFILE").unwrap_or_default() != "release" {
        return;
    }
    let platform = match env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "windows" => "windows-x86_64",
        "linux" => "linux-x86_64",
        _ => return,
    };
    let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    let manifest_path = PathBuf::from(&manifest_dir)
        .join("resources")
        .join("runtime-pack")
        .join(platform)
        .join("manifest.json");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    let compact: String = manifest.chars().filter(|c| !c.is_whitespace()).collect();
    let bundles_real_release = compact.contains("\"payload_profile\":\"release\"")
        && compact.contains("\"release_injection_required\":false");
    if bundles_real_release {
        return;
    }

    let url = env::var("ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL").unwrap_or_default();
    let key = env::var("ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64").unwrap_or_default();
    if url.trim().is_empty() || key.trim().is_empty() {
        panic!(
            "Release build for {platform} bundles a non-release (fixture) runtime-pack, but \
             ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL / _PUBLIC_KEY_BASE64 are not baked in. A lean \
             installer with no signed download source dead-ends on a clean machine. Set the bootstrap \
             env vars (see .github/workflows/release.yml) before a release build, or bundle a real \
             release runtime-pack."
        );
    }
}

fn ensure_windows_vc_runtime_glob_exists() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os != "windows" || target_env != "msvc" {
        return;
    }

    let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") else {
        return;
    };

    let placeholder_dir = PathBuf::from(manifest_dir)
        .join("target")
        .join("release")
        .join("vc-runtime");

    if let Err(error) = std::fs::create_dir_all(&placeholder_dir) {
        println!(
            "cargo:warning=Failed to create VC runtime placeholder dir {}: {error}",
            placeholder_dir.display()
        );
        return;
    }

    let placeholder = placeholder_dir.join(".gitkeep");
    if !placeholder.exists() {
        if let Err(error) = std::fs::write(&placeholder, b"") {
            println!(
                "cargo:warning=Failed to create VC runtime placeholder {}: {error}",
                placeholder.display()
            );
        }
    }
}

fn stage_windows_vc_runtime() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let profile = env::var("PROFILE").unwrap_or_default();

    if target_os != "windows" || target_env != "msvc" || profile != "release" {
        return;
    }

    let Some(target_profile_dir) = target_profile_dir() else {
        println!("cargo:warning=Unable to resolve target profile dir for VC runtime staging");
        return;
    };

    let required = [
        "msvcp140.dll",
        "msvcp140_1.dll",
        "vcomp140.dll",
        "vcruntime140.dll",
        "vcruntime140_1.dll",
    ];
    let optional = ["concrt140.dll"];

    let mut target_vc_runtime_dirs = vec![target_profile_dir.join("vc-runtime")];

    if let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") {
        let manifest_dir = PathBuf::from(manifest_dir);
        target_vc_runtime_dirs.push(
            manifest_dir
                .join("target")
                .join("release")
                .join("vc-runtime"),
        );
        if let Some(desktop_dir) = manifest_dir.parent() {
            target_vc_runtime_dirs.push(
                desktop_dir
                    .join("target")
                    .join("release")
                    .join("vc-runtime"),
            );
        }
        if let Some(repo_root) = manifest_dir.ancestors().nth(3) {
            target_vc_runtime_dirs
                .push(repo_root.join("target").join("release").join("vc-runtime"));
        }
    }

    for dll in required {
        for dir in &target_vc_runtime_dirs {
            stage_vc_runtime_dll(dll, dir, true);
        }
    }
    for dll in optional {
        for dir in &target_vc_runtime_dirs {
            stage_vc_runtime_dll(dll, dir, false);
        }
    }
}

fn target_profile_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").ok()?);
    out_dir.ancestors().nth(3).map(Path::to_path_buf)
}

fn stage_vc_runtime_dll(name: &str, target_vc_runtime_dir: &Path, required: bool) {
    let Some(source) = find_vc_runtime_dll(name) else {
        let message = format!(
            "Required VC runtime DLL {name} was not found; clean Windows installs will fail before EntropIA can start"
        );
        if required {
            panic!("{message}");
        }
        println!("cargo:warning={message}");
        return;
    };

    if let Err(error) = std::fs::create_dir_all(target_vc_runtime_dir) {
        let message = format!(
            "Failed to create VC runtime staging dir {}: {error}",
            target_vc_runtime_dir.display()
        );
        if required {
            panic!("{message}");
        }
        println!("cargo:warning={message}");
        return;
    }

    let destination = target_vc_runtime_dir.join(name);
    if let Err(error) = std::fs::copy(&source, &destination) {
        let message = format!(
            "Failed to stage VC runtime DLL {} from {} to {}: {error}",
            name,
            source.display(),
            destination.display()
        );
        if required {
            panic!("{message}");
        }
        println!("cargo:warning={message}");
    }
}

fn find_vc_runtime_dll(name: &str) -> Option<PathBuf> {
    if let Ok(dir) = env::var("ENTROPIA_VC_RUNTIME_DIR") {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Ok(system_root) = env::var("WINDIR") {
        let candidate = PathBuf::from(system_root).join("System32").join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}
