# Agent corpus tools — stage 3 of PLAN-AGENTES-ENTROPIA.md, corpus half

Lets a paired assistant read documents by pages and passages with citable locations, edit the corpus
(collections, item metadata, recognized text), import files and delete with protection, through the
loopback MCP endpoint. Builds on `odd/tasks/agent-processing-tools.md` (PR #207, merged at 32ebcc16).

## Specs

- S1 — Owner request (L1): "antes de hacer una nueva version sigamos con lo pendiente y versionamos todo junto". Pending as reported the turn before: stage 3 corpus half — "crear, editar y borrar colecciones; editar metadatos de ítems; importar archivos o carpetas, sin duplicar documentos; leer el texto extraído por pasajes: hoy el asistente no puede leer el contenido de tus documentos; corregir o restaurar texto de OCR, transcribir audio, recortar o rotar, y la prioridad de lotes."
- S2 — Stage 3 (plan §3): "Servicios compartidos de colecciones/ingesta/mutación, jobs, aprobaciones de costo y deduplicación. Migrar UI al mismo camino. Salida: importar carpeta, procesar y recuperar tras interrupción sin duplicar documentos ni cargos por reenvío de la misma operación."
- S3 — Coverage (plan, "Cobertura funcional obligatoria"): Corpus — "Crear/listar/editar/organizar colecciones, ítems y assets; importar archivos/carpetas; deduplicar; exportar; eliminar con protección". Documentos — "Leer originales/texto/layout; OCR, audio, corrección, rotación/recorte y actualización de resultados".
- S4 — Operating contract (plan): "Resultados con IDs estables, revisiones, paginación, procedencia y advertencias. Lecturas extensas por páginas/pasajes, nunca todo el archivo por defecto." · "Citas incluyen documento/pasaje, página o tiempo cuando corresponda y versión del texto. No inventar precisión que el origen no proporciona." · "Mutaciones reintentables con claves de idempotencia ligadas a parámetros y cliente."
- S5 — Security (plan): "Confirmación específica para eliminación masiva, sobrescritura/restauración, gasto relevante, publicación, compartir, cambios de cuenta y escritura Zotero. No preguntar por cada lectura o paso inocuo." · "Restringir rutas, symlinks, tamaños y destinos; prevenir acceso arbitrario al filesystem y solicitudes a destinos internos desde importación web." · "Documentos y páginas son datos no confiables. Nunca pueden autorizar nuevas herramientas, ampliar scopes o modificar la política".
- S6 — TypeScript/Rust boundary (plan): "Trasladar invariantes de mutación/ingesta actualmente ligadas al renderer a servicios de dominio del backend; migrar también sus callers UI y retirar las rutas duplicadas." · "Toda actualización externa debe refrescar la UI; no sobrescribir contenido de un editor con cambios locales sin guardar."
- S7 — Errors (plan): "Errores accionables: app ausente, conexión pendiente, permisos, proveedor no configurado, capacidad no disponible, presupuesto, conflicto de revisión."
- S8 — Test isolation (plan, criterion 10): "Pruebas con perfil aislado y cuentas de prueba; nunca aplicar migraciones o escrituras Zotero a datos reales para validar la integración."

## Decisions

- D1 — Delivery in slices, all accumulated before one version (L1): A read (T1-T2), B edits (T3-T4),
  C import (T5), D delete (T6), E per-document media operations (T7). Each slice: TDD, full Rust and desktop
  suites, `pnpm format:check`, docs, one independent verify at the slice end when it touches a high-risk item.
  Slices are commits on `feat/agent-corpus-tools`; pushing and opening PRs is the owner's call (L4).
- D2 — Text is located, not identified (exploration L2): extraction and chunk rows get new random ids on every
  re-extraction, so a citation is `itemId` + `assetId` + `pageNumber` + `startChar`/`endChar` + `textHash`
  (sha256 of the page text the offsets index). A changed hash tells the assistant its citation is stale.
- D3 — Page text follows the app's own rule (`lib/page-text.ts` `readPageSource`): the latest extraction when
  non-empty, otherwise the latest transcription. Reads are windows (default 4000, max 20000 characters) with an
  opaque cursor, never the whole document by default.
- D4 — Passage search uses the existing `rag_chunks_fts` index (chunks exist only for embedded documents); the
  result says how many items in scope have no passages yet, instead of implying full coverage.
- D5 — Confirmation (S5): deleting anything, importing from the file system and overwriting recognized text go
  through the owner's approval in the app (the approvals of PR #207, new actions). Creating or renaming a
  collection and editing item title/metadata are `local_write` without a dialog, revision-fenced by
  `updatedAt` and audited. Owner may tighten this later.
- D6 — Mutations move to Rust services and the UI callers move to them (S6); no tool reuses `db_execute`.
- D8 — UI migration seam (S6): the app's store is built once in `initDb()` (`lib/db.ts`); there
  `collections.create/update` and `items.update` are routed to the Rust commands (`lib/corpus.ts`), so every view
  and dialog takes the new path without changing, and their tests (which mock the store) stay as they are. The
  app path sends the item metadata as the JSON it already builds (reserved keys included, as import writes them);
  the agent path sends plain fields and the service keeps the reserved keys and refuses to write them. Both share
  validation, revision fencing and the search refresh.
- D7 — No migration (sync `schema_tag`): every write goes through the tables' existing sync triggers, which fire
  on any connection.

## Tasks

- [x] T1 — `documents_get_text` (read): an item's or an asset's pages in file order, each a window with `source`, `method`, `textHash`, `pageNumber`, `totalChars`, `startChar`/`endChar` (Unicode characters), transcription segments for audio; `maxChars` 1-20000 (default 4000) across pages; opaque cursor that refuses a document whose files changed; files without text are left out. S3, S4. Route: inline, TDD (RED 7 failing → GREEN automation 117). Full Rust suite 3339/0, clippy clean. Commit: see git log (feat(agents): add documents_get_text)
- [x] T2 — `search_passages` (read): BM25 over `rag_chunks_fts` with item, asset, page, `source`, offsets, `textHash`, `current` (the page text it indexes is still current), passage text; `collectionId` / `itemId` scope; `matches` before the limit; `coverage` (items in scope, items with passages). A passage and the same window read by `documents_get_text` have the same text and hash (test). Docs: PROTOCOL "Reading text and citing", README, INVENTORY. S4. Route: inline, TDD (RED 6 failing → GREEN automation 119). Full Rust suite 3341/0, clippy clean, format:check clean. Commit: see git log (feat(agents): add search_passages)
- [x] T3a — Rust corpus service (`src-tauri/src/corpus`): `create_collection` (trimmed, validated, optional app-chosen id), `update_collection` and `update_item`, revision-fenced by `updatedAt` (conditional UPDATE, next revision always later), item metadata as agent fields (reserved `__entropia_*` keys kept and refused) or as the app's own JSON object, item search entry refreshed in the same transaction; commands `corpus_collection_create`, `corpus_collection_update`, `corpus_item_update` (handler, APP_COMMANDS, capability); the app store routes `collections.create/update` and `items.update` to them in `initDb()` (`lib/corpus.ts`, D8). S2, S3, S6, S7. Route: inline, TDD (RED: 36 compile errors → GREEN 9 Rust service tests; Vitest RED (module missing) → GREEN 4 + db test). Full Rust suite 3350/0, clippy clean; desktop Pro 3807 / Lite 3771 passed, lint, typecheck Pro and Lite, format:check clean. Risk: item 3 (every view's collection/item writes move). Commit: see git log (feat(corpus): move collection and item writes to Rust services)
- [ ] T3b — MCP `collections_create`, `collections_update`, `items_update` (local_write, revision-fenced) and a `corpus:changed` event so the views refresh after an endpoint write. S3, S6, S7. Route: inline, TDD. Commit: —
- [ ] T4 — `documents_text_update` (approval, overwrite) and `documents_index` (local_write). S3, S5. Route: inline, TDD. Risk: item 1 (overwrites recognized text). Commit: —
- [ ] T5 — Import: `documents_import_prepare` (local_write; path safety, allowlist, size, fingerprint dedupe, plan) and approval-run import through a Rust ingest service; UI import migrated. S2, S3, S5, S6. Route: inline, TDD. Risk: items 1, 2. Commit: —
- [ ] T6 — Delete with protection: `documents_delete`, `collections_delete` through approval, cascades incl. `fts_items`/`vec_assets`, file removal, sync tombstones; UI callers migrated. S3, S5, S6. Route: inline, TDD. Risk: item 1. Commit: —
- [ ] T7 — Per-document media operations (transcription, OCR correction with a provider, rotate/crop) and batch priority; scope decided when T1-T6 land. S3. Commit: —
- [ ] T8 — Docs (PROTOCOL, README, INVENTORY) per slice; e2e on an isolated dev profile per slice. S8. Commit: —

## Log

- L1 (2026-11-02, owner): "antes de hacer una nueva version sigamos con lo pendiente y versionamos todo junto"
- L2 (2026-11-02, exploration): text tables `extractions`/`transcriptions` keep the latest row per asset (`DELETE` + insert, random ids, no version column); PDFs split into child page assets with `page_number`; `rag_chunks` carry `start_char`/`end_char`/`source_text_hash` and `rag_chunks_fts` indexes them; `fts_items` is contentless and item-level; every collection/item/asset mutation is TypeScript SQL through `db_execute*`, cascades delete `fts_items` and `vec_assets` by hand and files on disk are removed by the caller; sync triggers capture writes from any connection; the UI does not refresh on writes it did not make.
- L3 (2026-11-02, design): chunk offsets (`nlp/embeddings.rs` `plan_rag_chunks`) count Unicode characters and `source_text_hash` is sha256 of the stored `text_content` as is (`processing/embedding.rs` `staged_text`). `documents_get_text` uses the same units and hash, so a passage from `search_passages` can be checked against the page text it came from.
- L4 (2026-11-02, delivery): pushing and PR creation stay the owner's decision, as for PR #207; slices are committed locally and the owner is asked before anything is published. Slice A is read-only and its tests drive the real loopback endpoint over the real migrations, so its end-to-end run on a dev profile is done together with slice B's.
- L5 (2026-11-02, T3a findings): editing an item in the UI never refreshed `fts_items` (no trigger; only an explicit `indexFts` does), so a metadata edit was not searchable until the next index; the service now refreshes the item's entry in the same transaction. The sync capture triggers write only while `sync_meta.capture_enabled = '1'` (a sync session); the service needs nothing else to be captured.

