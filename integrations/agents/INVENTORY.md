# Product inventory: what an assistant can and cannot reach

Stage 1 of `PLAN-AGENTES-ENTROPIA.md`: every product operation is mapped to where
it lives today, to the tool that would expose it, and to its status. The exit
condition is "no product function without a mapping and no promise of a
capability that does not exist", so this table is deliberately pessimistic: a
row is `implemented` only when the tool exists in
`apps/desktop/src-tauri/src/automation/mcp.rs` today.

Facts this table is built from:

- the registered Tauri commands in the `generate_handler!` block of
  `apps/desktop/src-tauri/src/lib.rs`;
- the TypeScript domain operations in `packages/store/src/repos/*` and
  `packages/store/src/index.ts`, reached through `apps/desktop/src/lib/db.ts`;
- the user-visible features described in
  `manual/manual-usuario/inventario-funciones-manual.md`.

## How to read it

- **Where it lives today**: Rust paths are relative to
  `apps/desktop/src-tauri/src/`; TypeScript paths are repository-relative.
- **Status**: `implemented` = a working MCP tool exists. `planned (stage N)` =
  designed, not built; the stage is the plan's stage that owns it.
  `not exposed` = deliberately stays out of MCP, with the reason.
- **Scope**: the target scope from `PROTOCOL.md` (`read`, `local_write`,
  `remote_spend`, `external_write`, `admin`, `web`). `—` for rows that are not exposed.
- **Planned tool names** beyond the nine the plan itself proposes
  (`entropia_status`, `collections_create`, `documents_import_prepare`,
  `processing_start`, `jobs_get`, `search_passages`, `writing_apply_patch`,
  `writing_export`, `approvals_status`) are naming proposals, not commitments.
- **Pro/Lite**: both variants register the same commands; where a capability
  depends on Pro's local models the row says so.

Two facts explain most of the "where it lives" column and shape stages 3-5:

1. Reads the tools already serve go straight to SQLite through a dedicated
   `query_only` connection (`automation/read.rs`).
2. Most **mutations** have no Rust domain service yet: the renderer calls
   `CollectionRepo`, `ItemRepo`, `AssetRepo`, `NoteRepo`, …, and those reach the
   archive through the generic `db::commands::db_execute*` commands wired by
   `apps/desktop/src/lib/tauri-db-client.ts`. An MCP tool
   must not reuse that path (it would be the `execute_sql` the plan forbids), so
   every mutation row below is blocked on moving its invariant into a domain
   service first. That is exactly what the plan's stage 3 is for.

## Inicio / configuración

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Instance identity, variant, active profile | `entropia_status` (`automation/mcp.rs`) | `entropia_status` | read | Both | implemented |
| How much the archive holds | `automation/read.rs` (`counts`) | `entropia_status.archive` | read | Both | implemented |
| Dependency and runtime diagnosis | `deps/*` (`deps_check_all`, `deps_get_cached_statuses`, `deps_install_all`, `deps_install_one`, `deps_get_uv_status`, `deps_reset`), `runtime/*` (`runtime_get_status`, `runtime_get_bootstrap_plan`, `runtime_repair`, `runtime_bootstrap_start`, `runtime_cancel_download`) | `runtime_status` | read | Both (Pro runtime pack only in Pro) | planned (stage 5) |
| Guided repair | `runtime::runtime_repair` | `runtime_repair_prepare` → approval → job | admin | Both | planned (stage 5), approval per D13 |
| Providers configured / verified / failing | `settings.rs` (`settings_get`, `settings_get_all`, `settings_set`, `settings_delete`), `llm::test_openrouter_connection`, `llm::test_vercel_connection`, `transcription::test_assemblyai_connection`, `ocr::test_glm_ocr_connection`, `zotero_web::zotero_verify_key`, `llm_is_available`, `llm_ocr_correction_is_available` | `providers_status` (never a key, only configured/verified/error) | read | Both | planned (stage 4) |
| Storing or removing a provider key | same `settings` commands, keyring-backed | not exposed | — | Both | not exposed (owner decision D13): key entry stays a human step in Settings; agents see `providers_status` only |
| Local models and runtime (download, open folder, info) | `llm_local_model_info`, `llm_download_model`, `llm_open_models_dir`, `embedding_local_model_info`, `embedding_download_model`, `embedding_open_models_dir`, `rag_reranker_model_info`, `rag_reranker_download_model`, `rag_reranker_open_models_dir` | `models_download_prepare` → approval → job | admin | Pro only | planned (stage 5): plan with size and free space, human approval in the app, then a queryable job (D13) |
| Appearance, zoom, typography, interface language | `apps/desktop/src/lib/appearance.ts`, `i18n.ts` | not exposed | — | Both | not exposed: presentation only, no domain effect |
| Recent activity shown on Inicio | `apps/desktop/src/views/HomeView.svelte` (recent items through `ItemRepo`) | `activity_recent` | read | Both | planned (stage 5) |
| App update (Microsoft Store) | `store_updates::check_microsoft_store_update` | not exposed | — | Both | not exposed: the platform updater owns it |
| App logs | `app_logs.rs` (`logs_get`, `logs_clear`, `logs_open_dir`, `logs_append`) | `logs_recent` | admin | Both | planned (stage 5); needs redaction first |

## Corpus

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| List collections with item counts | `automation/read.rs` (`collections`) | `collections_list` | read | Both | implemented |
| Create / rename / edit a collection | `corpus::create_collection`, `corpus::update_collection` (the app's views use them through `lib/corpus.ts`) | `collections_create`, `collections_update` | local_write | Both | implemented |
| Delete a collection and its contents | `corpus::delete::delete_collection` (the app's Colecciones view uses it through `lib/corpus.ts`) | `collections_delete` (owner approval with the impact; nothing is deleted before the answer) | local_write + owner approval | Both | implemented |
| List items, optionally by collection | `automation/read.rs` (`items`) | `items_list` | read | Both | implemented |
| One item: metadata and file list | `automation/read.rs` (`item`) | `items_get` | read | Both | implemented |
| Search titles, metadata and indexed text | `automation/read.rs` (`search`), `nlp::commands::fts_search`, `packages/store/src/repos/fts.repo.ts`, `apps/desktop/src/lib/collection-search-plan.ts` | `search_items` | read | Both | implemented |
| Edit item metadata / custom fields | `corpus::update_item` (the app's views use it through `lib/corpus.ts`) | `items_update` | local_write | Both | implemented |
| Import files (picker or drag and drop) | agent: `corpus::ingest` from owner-enabled folders; app: `lib/file-import.ts`, `collection-import.ts`, Tauri dialog/fs plugins | `import_sources_list`, `documents_import` (owner approval with the file list) | local_write + owner approval | Both | implemented for the agent; the app's own import moves to `corpus::ingest` later |
| Import deduplication | `corpus::ingest::find_duplicate` (the app's rule, inside the import transaction); app: `item.repo.ts` `findImportedFromSource` | part of `documents_import` (a duplicate is skipped and reported) | local_write | Both | implemented |
| Delete a document | `corpus::delete::delete_items` (the app's CollectionView uses it through `lib/corpus.ts`) | `documents_delete` (owner approval with the impact and the titles; nothing is deleted before the answer) | local_write + owner approval | Both | implemented |
| Delete a page (asset) | `packages/store/src/repos/asset.repo.ts`, `image_edit::delete_asset_files`, `navegador_delete_source` | not exposed | — | Both | not exposed: page deletion stays in the renderer for now (D11 d) |
| Export a collection as JSON | `apps/desktop/src/lib/export.ts` | `collections_export` | read | Both | planned (stage 5) |
| Asset integrity check | `asset_integrity::assets_check_integrity` | `archive_check` | admin | Both | planned (stage 5) |
| Move items between collections, reorder pages, add a page to an existing document | no code (the manual lists these as unavailable) | not exposed | — | Both | not exposed: no product operation exists to expose |

## Documentos

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Remote OCR (OCRH / GLM-OCR) | `ocr/commands.rs` (`extract_text`, `test_glm_ocr_connection`, `is_scanned_pdf`, `probe_pdf`) | as a batch: `processing_prepare` with `ocr`, then `processing_start`; a per-document `documents_ocr_start` stays planned | remote_spend | Both | implemented as a batch (stage 3) |
| Local OCR | `ocr/pdf.rs`, gated by `local-ml` | `documents_ocr_start` | local_write | Pro only | planned (stage 3) |
| OCR correction and summaries with a provider | `llm/commands.rs` (`llm_correct_ocr`, `llm_correct_ocr_asset`, `llm_summarize`, `llm_summarize_asset`, `llm_get_results`, `llm_get_result`, `llm_can_restore_original_ocr_asset`, `llm_restore_original_ocr_asset`) | as a batch: `processing_prepare` with `ocr_correction` or `summary`; a per-document `documents_transform_start` stays planned | remote_spend | Both | implemented as a batch (stage 3) |
| Edit or restore recognized text | `corpus::replace_page_text` (agent; owner approval, backup kept), `ocr::update_extraction_text_cmd` (app), `llm_restore_original_ocr_asset` (undo) | `documents_text_update` (recognized text; transcriptions stay planned) | local_write + owner approval | Both | implemented for recognized text |
| Read extracted text, layout or transcript | `automation::read::document_text` (same rule as `lib/page-text.ts`), `layout.repo.ts`, `ocr::render_pdf_pages` | `documents_get_text` (windows, never the whole file); `documents_get_layout` stays planned | read | Both | implemented (text and transcript); layout planned (stage 3) |
| Crop, rotate, cover a region, split pages | `ocr::crop_pdf`, `edit_pdf`, `split_pdf_pages`, `image_edit::crop_image`, `rotate_image`, `rotate_image_degrees`, `erase_region` | `documents_edit` | local_write | Both | planned (stage 3) |
| Transcription and dictation | `transcription::transcribe_audio`, `transcribe_dictation`, `update_transcription_text_cmd`, `test_assemblyai_connection` | `documents_transcribe_start` | remote_spend | Both | planned (stage 3) |
| Audio playback | `audio_preview::prepare_audio_preview` | not exposed | — | Both | not exposed: playback is a viewer concern |
| Thumbnails | `ocr::generate_pdf_thumbnail`, `generate_image_thumbnail`, `delete_pdf_thumbnail`, `delete_image_thumbnail` | not exposed | — | Both | not exposed: cache artefact, no domain meaning |

## Análisis

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| INDEX: prepare text for lexical search | `corpus::reindex_item` (`nlp::fts::index_item_from_db`), `nlp::commands::index_fts` | `documents_index` | local_write | Both | implemented |
| EMBED: prepare embeddings | `nlp::commands::embed_asset`, `backfill_asset_embeddings`, `packages/store/src/repos/rag-chunk.repo.ts` | as a batch: `processing_prepare` with `embeddings`; a per-document `documents_embed` stays planned | remote_spend | Both (local embeddings in Pro) | implemented as a batch (stage 3) |
| Similar assets | `nlp::commands::similar_assets` | `search_similar` | read | Both | planned (stage 4) |
| NER: entities | `nlp::commands::extract_entities`, `extract_entities_for_asset`, `llm_extract_entities`, `llm_extract_entities_asset`, `enrich_item`, `packages/store/src/repos/entity.repo.ts` | `analysis_entities_start` | remote_spend | Both | planned (stage 4) |
| TRIPLET: relations | `nlp::commands::extract_triples`, `extract_triples_for_asset`, `llm_extract_triples`, `llm_extract_triples_asset`, `packages/store/src/repos/triple.repo.ts` | `analysis_triples_start` | remote_spend | Both | planned (stage 4) |
| Relations graph | `triple.repo.ts`, `apps/desktop/src/lib/relation-graph.ts`, `apps/desktop/src/components/RelationGraph3D.svelte` | `analysis_relations_get` | read | Both | planned (stage 4) |
| Notes and topics, with confirmation before delete | `packages/store/src/repos/note.repo.ts`, `topic.repo.ts` | `notes_list`, `notes_create`, `notes_update`, `notes_delete` | local_write | Both | planned (stage 4) |
| Annotations | `packages/store/src/repos/annotation.repo.ts` | `annotations_list`, `annotations_create` | local_write | Both | planned (stage 4) |
| Word analysis of a collection | `apps/desktop/src/views/CollectionAnalysisPanel.svelte`, `analysis-settings.ts`, extractions | `analysis_words_get` | read | Both | planned (stage 4) |
| Map: geocoding and places | `geo::geocode_entity`, `geocode_item_entities`, `entity.repo.ts` | `analysis_geocode` | remote_spend | Both | planned (stage 4) |

## Procesamiento

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Prepare a batch: scope, operations, estimated cost | `processing::commands::prepare_batch` (shared with the tab's `processing_prepare` command), `apps/desktop/src/lib/batch-processing.ts` | `processing_prepare` (collections and/or items; no cost before planning) | local_write | Both | implemented |
| Start a batch | `processing::commands::start_batch` | `processing_start` → owner approval in the app (`automation/approvals.rs`) | remote_spend | Both | implemented |
| Batch, task and progress read | `processing::commands::{list_batches_page, batch_snapshot, list_batch_tasks}`, `apps/desktop/src/lib/batch-pagination.ts` | `jobs_get`, `jobs_list` | read | Both | implemented |
| Pause, resume, cancel | `processing::commands::control_batches` | `processing_control` (pause/cancel: local_write, at once; resume: remote_spend + owner approval) | local_write, remote_spend | Both | implemented |
| Priority | `processing::commands::set_priority` | `processing_set_priority` | local_write | Both | planned (stage 3): the service is shared, the tool is not exposed yet |
| Retry failed work | `processing::commands::retry_batch` | `processing_control` action `retry` (owner approval) | remote_spend | Both | implemented |
| Recovery after closing the app | `batch-processing.ts`, `processing_*` state machines | `jobs_get` (jobs persist; approvals do not survive a restart) | read | Both | implemented |
| Custom schema extraction and CSV | `processing::schema_extract::extraction_schemas_list`, `extraction_schema_save`, `extraction_schema_delete`, `extraction_records_list`, `packages/store/src/repos/extraction.repo.ts` | `schemas_list`, `schemas_save`, `schemas_extract_start` | remote_spend | Both | planned (stage 4) |
| Zotero library sync queued as batches | `processing_sync_bibliography_library`, `processing_bibliography_sync_status`, `processing_latest_bibliography_sync_status` | `library_sync_start`, `library_sync_status` | remote_spend | Both | planned (stage 4) |
| Saved batch flows | `apps/desktop/src/lib/batch-flows.ts` | not exposed | — | Both | not exposed: a UI preference that names operations, not a domain operation |

## Consulta / investigación

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Chat over documents and/or Library | `rag/commands.rs` (`rag_ask`), `rag-chunk.repo.ts` | `chat_ask` | remote_spend | Both | planned (stage 4) |
| Conversations: list, read, search, rename, delete | `rag_list_conversations`, `rag_get_conversation`, `rag_search_conversations`, `rag_update_conversation_title`, `rag_delete_conversation`, `rag_generate_conversation_title` | `conversations_list`, `conversations_get`, `conversations_delete` | read / local_write | Both | planned (stage 4) |
| Staged research with plan, sources and report | `research::research_request`, `apps/desktop/src/lib/research.ts` | `research_start`, `research_get` | remote_spend | Both | planned (stage 4) |
| Citable passages | corpus: `automation::read::passages` over `rag_chunks_fts`; library: `bibliography::bibliography_search_passages`, `bibliography_passage_context`, `bibliography_open_passage`, `writing::writing_corpus_retrieve` | `search_passages` | read | Both | implemented for Colecciones; Biblioteca passages planned (stage 4) |
| Report export (Markdown, HTML, DOCX) | `research::research_request` output, `apps/desktop/src/lib/export.ts` | `research_export` | read | Both | planned (stage 5) |
| Generic LLM actions (ask, summarize, classify) | `llm::commands::llm_ask`, `llm_summarize`, `llm_classify`, `llm_get_results` | not exposed | — | Both | not exposed (owner decision D13): the agent uses its own model; EntropIA spends provider credit only through named business operations |
| Chunk coverage diagnostics | `rag::commands::rag_chunk_coverage` | not exposed | — | Both | not exposed: diagnostic, not a product intent |

## Biblioteca

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Library status and Zotero connection | `bibliography_library_status`, `zotero_web::zotero_verify_key`, `bibliography::zotero_data_dir_grant`, `apps/desktop/src/lib/bibliography-library.ts` | `library_status` | read | Both | planned (stage 4) |
| Browse and search works | `bibliography_list_works`, `bibliography_search_works`, `bibliography_work_detail`, `apps/desktop/src/lib/bibliography-search.ts` | `library_works_list`, `library_works_get` | read | Both | planned (stage 4) |
| Open an attachment | `bibliography_open_work_attachment` | `library_open_attachment` | read | Both | planned (stage 4) |
| Annotations: list, create, update, delete, restore, resolve, sync | `bibliography::annotations::commands::bibliography_annotations_*`, `apps/desktop/src/lib/bibliography-annotations.ts`, `bibliography-annotation-markups.ts` | `library_annotations_list`, `library_annotations_create`, `library_annotations_sync` | external_write (Web API) | Both | planned (stage 4) |
| Reprocess damaged text | `bibliography_reprocess_candidates`, `bibliography_reprocess_candidates_cancel`, `bibliography_reprocess_preview`, `bibliography_reprocess_preview_cancel`, `bibliography_reprocess_confirm`, `apps/desktop/src/lib/bibliography-reprocess.ts` | `library_reprocess_prepare`, `library_reprocess_start` | remote_spend | Both | planned (stage 4) |
| Library sync (catalogue + text preparation) | see Procesamiento | `library_sync_start` | remote_spend | Both | planned (stage 4) |

## Navegador / Internet Archive

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Navigate: open, tabs, back, forward, reload | `navegador::commands::navegador_open`, `navigate`, `back`, `forward`, `reload`, `new_tab`, `activate_tab`, `close_tab`, `close`, `state` | `browser_navigate`, `browser_state` | web (page content is untrusted data; D13) | Both | planned (stage 4) |
| Capture a page or a selection | `navegador_capture_page`, `navegador_capture_selection`, `navegador_save_capture`, `navegador_pdf_file` | `browser_capture` | local_write | Both | planned (stage 4) |
| Downloads | `navegador_save_download`, `navegador_download_dir`, `navegador_set_download_dir`, `navegador_discard_draft` | `browser_download` | local_write | Both | planned (stage 4) |
| Saved sources | `navegador_list_sources`, `navegador_source_detail`, `navegador_delete_source` | `browser_sources_list`, `browser_sources_delete` | read / local_write | Both | planned (stage 4) |
| Copy a captured page into a collection | `navegador_save_capture` plus the import path | `browser_save_to_collection` | local_write | Both | planned (stage 4) |
| Copy to Zotero (request, list, run, cancel, launch, libraries, status) | `navegador_zotero_copy_request`, `navegador_zotero_copy_list`, `navegador_zotero_copy_run`, `navegador_zotero_copy_cancel`, `navegador_zotero_launch`, `navegador_zotero_libraries`, `navegador_zotero_status` | `browser_zotero_copy` (approval required) | external_write | Both | planned (stage 4) |
| Internet Archive: search, browse, resolve | `internet_archive::commands::ia_search`, `ia_list_collection`, `ia_get_item`, `ia_resolve_identifier` | `ia_search`, `ia_item_get` | read | Both | planned (stage 4) |
| Internet Archive: import a PDF | `ia_prepare_import`, `ia_start_import`, `ia_cancel_import`, `ia_get_import_status`, `ia_list_jobs`, `ia_list_verified_files`, `ia_discard_draft`, `internet_archive::import::ia_begin_file_import`, `ia_confirm_file_imported`, `ia_fail_file_import`, `ia_list_reconcile`, `ia_finish_reconcile` | `ia_import_prepare`, `ia_import_start` | local_write + read | Both | planned (stage 4) |

## Escritura

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Create, list, load, delete a manuscript | `writing::commands::writing_create_document`, `writing_list_documents`, `writing_load_document`, `writing_is_ready`, `writing_duplicate_document` | `writing_documents_list`, `writing_documents_get`, `writing_documents_create` | read / local_write | Both | planned (stage 5) |
| Save with revision guard | `writing::commands::writing_save_document`, `writing_append_journal` | `writing_apply_patch` (revision conflict is its own error) | local_write | Both | planned (stage 5) |
| Rename, change status | `writing_rename_document`, `writing_set_status` | `writing_documents_update` | local_write | Both | planned (stage 5) |
| Versions: snapshot, list, read, restore, retention | `writing_snapshot_version`, `writing_list_versions`, `writing_read_version`, `writing_restore_version`, `writing_apply_retention` | `writing_versions_list`, `writing_versions_restore` (confirmation required) | local_write | Both | planned (stage 5) |
| Recovery journal and sync notices | `writing_recovery_plan`, `writing_prune_journal`, `writing_discard_journal`, `writing_sync_notices` | `writing_recovery_status` | local_write | Both | planned (stage 5) |
| Citations and CSL rendering | `writing_citations_for_asset`, `writing_csl_render`, `writing_csl_render_document`, `writing_csl_bibliography`, `writing_csl_validate_style`, `apps/desktop/src/lib/citation-clusters.ts`, `citation-target.ts` | `writing_citations_get` | read | Both | planned (stage 5) |
| Zotero inside Writing: probe, cache, sync, search, libraries, item detail, open | `writing_zotero_probe`, `writing_zotero_cached`, `writing_zotero_sync`, `writing_zotero_search`, `writing_zotero_known_libraries`, `writing_zotero_check_library`, `writing_zotero_item_detail`, `writing_zotero_open_item` | `writing_zotero_search`, `writing_zotero_cite` | external_write | Both | planned (stage 5) |
| Writing agent: 14 actions and reviewable proposals | `writing_agent_actions`, `writing_agent_record_suggestion`, `writing_agent_ask`, `writing_agent_pending`, `writing_agent_resolve`, `writing/agent_actions.rs` (the action matrix) | `writing_agent_propose` (proposals only), `approvals_status` | remote_spend + local_write | Both | planned (stage 5): reviewable proposals the person accepts or discards in the editor; never applied by the agent (D13) |
| Corpus retrieval inside Writing | `writing_corpus_retrieve` | `search_passages` | read | Both | planned (stage 4) |
| Export to Markdown, HTML, DOCX with citations | `apps/desktop/src/lib/writing-export.ts` | `writing_export` | local_write | Both | planned (stage 5) |
| Publish to hlab.com.ar | `writing::publish::writing_publish_hlab` | `writing_publish` (separate, explicit approval) | external_write | Both | planned (stage 5) |

## Colaboración

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Account: register, log in, log out, delete account | `sync::session::sync_register_account`, `sync_login`, `sync_logout`, `sync_delete_account` | `sync_account` | admin | Both | planned (stage 5) |
| Sync status, usage, plan | `sync::commands::sync_status`, `sync_get_usage`, `sync_list_plans`, `sync_request_plan_change` | `sync_status` | read | Both | planned (stage 5) |
| Sync now, automatic mode, full resync | `sync_now`, `sync_get_auto`, `sync_set_auto`, `sync_full_resync`, `sync_ensure_capture`, `sync_reverify_blobs` | `sync_run`, `sync_configure` | local_write (egress) | Both | planned (stage 5) |
| Devices and device revocation | `sync_list_devices`, `sync_revoke_device` | `sync_devices_list`, `sync_device_revoke` | admin | Both | planned (stage 5) |
| Conflicts | `sync_list_conflicts`, `sync_ack_conflict`, `sync_ack_all_conflicts` | `sync_conflicts_list`, `sync_conflicts_ack` | local_write | Both | planned (stage 5) |
| Share a manuscript, remove a share | `sync_writing_shares`, `sync_writing_share`, `sync_writing_unshare` | `writing_share`, `writing_unshare` | external_write (explicit approval) | Both | planned (stage 5) |
| Notifications | `sync_list_notifications`, `sync_delete_notification`, `sync_mark_notification_read` | `sync_notifications_list` | read | Both | planned (stage 5) |

## Mantenimiento

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Backup: folder, schedule, run now | `backup::backup_dir_grant`, `backup::backup_run_now` | `backup_configure`, `backup_run` | admin | Both | planned (stage 5) |
| Restore a backup | no code (the manual documents copying the folder back with the app closed) | not exposed | — | Both | not exposed (owner decision D13): the person restores in the app; agents may create and list backups and explain the steps |
| Logs | `app_logs.rs` | see Inicio / configuración | admin | Both | planned (stage 5) |
| Database browser (Query DB) | `db::commands::db_select`, `db_select_rows`, `db_browser_list_tables`, `db_browser_describe_table`, `db_browser_query_rows`, `apps/desktop/src/lib/db-browser.ts` | not exposed | — | Both | not exposed: a generic query surface is exactly the `execute_sql` the plan forbids |
| Asset integrity check | `asset_integrity::assets_check_integrity` | see Corpus | admin | Both | planned (stage 5) |
| Archive compaction on close | `apps/desktop/src/lib/archive-compaction.ts`, `app_close_flushed` | not exposed | — | Both | not exposed: lifecycle of the app, not a user intent |
| App update | `store_updates::check_microsoft_store_update` | not exposed | — | Both | not exposed: platform updater |

## Interfaz

| Product operation | Where it lives today | Planned tool | Scope | Pro/Lite | Status |
|---|---|---|---|---|---|
| Open a result, document or page in the app | frontend routing in `apps/desktop/src/App.svelte`, view state | `ui_open_item` | read | Both | planned (stage 5) |
| Select a document, page or region for a later action | view state in `apps/desktop/src/views/*` | folded into each tool's explicit ids; no separate tool | read | Both | planned (stage 5) |
| Tabs and split view | `apps/desktop/src/views/*` | not exposed | — | Both | not exposed: window arrangement, no domain effect |
| Viewer geometry for crops and annotations | coordinates passed to `image_edit::*`, `ocr::crop_pdf` | arguments of `documents_edit` | local_write | Both | planned (stage 3) |
| Open an external URL | `open_external_url` | not exposed | — | Both | not exposed: opens a browser, no domain effect |
| Splash / close handshake | `splash::splash_finish`, `app_close_flushed` | not exposed | — | Both | not exposed: app lifecycle |
| Browser window geometry | `navegador_set_bounds`, `navegador_set_visible` | not exposed | — | Both | not exposed: viewer arrangement |

## Owner decisions (2026-11-01)

These were open at stage 1 and are now settled (feature document D13):

- **Approval model.** Every action that spends, writes outside EntropIA or
  deletes is prepared first (impact, cost, destinations) and approved by the
  person in an app dialog. The approval is bound to that plan and that client,
  is single-use and expires after 10 minutes. A model-produced
  `approved: true` is never consent. Design and implementation: stage 3.
- **Provider keys.** Never through an agent. Agents see configured / verified /
  error only and guide the person to Settings.
- **Backup restore.** The person restores in the app. Agents may create and
  list backups and explain how to restore.
- **Local models and runtime.** Plan with size and free space → approval in the
  app → queryable job.
- **Navegador.** A sixth scope, `web`, separate from `read`: a read-only client
  cannot browse or send requests to external sites. Internet Archive search
  stays `read` (one fixed API host, no arbitrary navigation).
- **Generic LLM actions (`llm_run`).** Not exposed.
- **Writing agent actions.** Only as reviewable proposals the person accepts or
  discards in the editor.
