# Batch: OCR correction and page summaries

Adds two operations to Settings › Lotes, next to Tripletes: LLM OCR correction and summaries.
Both already exist per document (ItemView); this runs them over a batch's pages.
Branch `feat/batch-ocr-correction-summary`.

## Specs

- S1 — Goal (L1): "quiero agregar junto a triplete la posbilidad de hacer por lotes Correccion OCR y Resumenes". The
  Operaciones row of Settings › Lotes gains two checkboxes after Tripletes: «Corrección OCR» and «Resúmenes».
- S2 — Summary unit (L2, owner: "Por página"): the batch writes one summary per page (asset), the same result the
  per-page summary of ItemView reads and shows (`llm_results` `summarize` for the asset). No per-document summary in
  this batch.
- S3 — Correction semantics (L2, owner: "Solo corregir"): a page's corrected text replaces its OCR text exactly as the
  per-document correction does (original kept in `ocr_correction_backups`, «Restaurar OCR original» keeps working).
  Embeddings, entities and triples that already exist are left as they are; the batch does not redo them.
- S4 — Order inside one batch (L3, parent's reading of L2, to confirm): when the same batch also runs Embeddings,
  Entidades or Tripletes for pages that do not have them yet, those wait for that page's correction and read the
  corrected text. A page without OCR text waits for OCR, as the existing ops do.
- S5 — Done once: a page already corrected (correction result present for its current extraction) is not corrected
  again by a batch, and a page with a summary is not summarized again, so re-running a flow never pays twice. Retry of a
  failed task works as for the other ops.
- S6 — Same configuration as per document: correction uses the OCR correction prompt and model of Settings › Prompts
  (always remote, also in Pro); summaries use the summary prompt and model. Both need the OpenRouter key and are gated
  like Entidades/Tripletes when it is missing.
- S7 — Saved flows (Flujo guardado) store and restore both new checkboxes; flows saved before this change still load.
- S8 — Progress and counts show the new operations like the existing ones (draft counts, tasks by kind, retries).
- S9 — Assistants: the MCP `processing_prepare` tool accepts the two new operations, documented, with the same
  approval for paid work as the other remote operations.
- S10 — Docs: the user manual stops saying the app does not make summaries by batch
  (`manual/manual-usuario/inventario-funciones-manual.md:120,156`) and documents both operations in the Lotes chapter.

## Tasks

- [x] T1 — S2-S6, S8 (backend) — worker: Rust contracts, executors, eligibility, plan wiring, migration of task kinds,
  tests — route: done — commit: 28c69149
- [x] T2 — S1, S7, S8 (frontend) — worker: Lotes tab checkboxes, ops array, flows, i18n, counts, OpenRouter gate,
  tests — route: done — commit: 885ef8c8
- [x] T3 — S9, S10 — worker: MCP/assistant operations and docs, manual — route: done — commit: 51994d09
- [x] T4 — S1-S10 — gentle-ai-verify (Risk: item 1 correction rewrites stored OCR text in a batch; item 3 new task
  kinds in a synced schema) — commit: none (fixes in the T4 follow-up commit)

## Log

- L1 — Owner request (verbatim): "quiero agregar junto a triplete la posbilidad de hacer por lotes Correccion OCR y
  Resumenes" (with a screenshot of Settings › Lotes: Operaciones OCR, Embeddings, Entidades, Tripletes).
- L2 — Owner answers: summary unit «Por página»; on pages whose embeddings/entities/triples were made from the old
  text, «Solo corregir» ("Corrige el texto y deja lo derivado como está hasta que el usuario lo rehaga").
- L3 — Parent's inference stated to the owner before work: inside one batch, ops that are created for the first time
  for a page run after its correction and use the corrected text; existing derived data is untouched.
- L4 — Exploration (gentle-ai-explore): batch tab `apps/desktop/src/views/BatchProcessingTab.svelte:72-99,387-397,
  1227-1243`; flows `apps/desktop/src/lib/batch-flows.ts:13-48`; Rust `processing/commands.rs:518-545,974-992`,
  `processing/repository.rs:162-183,5940+,6148`, `processing/eligibility.rs`, `processing/scheduler.rs:96-104,241`,
  engines `processing/{ner,triples,schema_extract}.rs`, registry `lib.rs:1216-1235`; per-document correction
  `llm/commands.rs:225` → `llm/ocr_correction.rs:788` `commit_asset_correction_if_current` (overwrites
  `extractions.text_content`, backup, `llm_results llr-asset-{id}-correct_ocr`), `force_remote_ocr_correction`
  (`llm/mod.rs:1581`); per-page summary `llm/commands.rs:252` → `persist_result` `llm_results summarize`; assistant
  ops `automation/jobs.rs:200,216`, `automation/mcp.rs:247-256`.
- L5 — T1 backend done (route: done, committed as `28c69149`). New kinds `ocr_correction` and `summary`:
  `packages/store/src/migrations/0060_processing_ocr_correction_summary_tasks.sql` widens the kind CHECK on
  `processing_tasks`/`processing_batch_tasks` (0058 rebuild shape, nothing else; mirror `include_str!` in
  `processing/repository.rs`); `migrations.generated.ts` regenerated with
  `pnpm --filter @entropia/store generate-migrations`, `runner.ts` gained the 0060 atomic-list entry,
  `db/migrations.rs` one registry line, `tests/fixtures/schema_canonical.json` + `schema_full.sql` regenerated with
  `export-schema-canonical` / `export-schema`, `schema-canonical.test.ts` 59→60 and the db count assertions 59→60 /
  atomic 28→29. Shared code instead of forks: `llm::correct_ocr_for_asset_blocking` (always remote,
  `require_openrouter_key` before any request) and `llm::summarize_for_asset_blocking` (settings-chosen engine) call the
  same `prepare_remote_job_request`/`run_asset_job_blocking` the per-document commands use;
  `llm::ocr_correction::commit_asset_correction_in_owner_transaction` runs the *same* correction body inside the queue's
  commit transaction (the old entry point began its own, which SQLite refuses to nest) and
  `llm::persist_asset_summary` is `persist_result(asset, summarize)`. Eligibility: `correction_decision` (done = stored
  `correct_ocr` result equals the page's current extraction; `Restaurar OCR original` clears both) and
  `summary_decision` (done = a `summarize` row for the asset; an item-level summary is not the page's). Planning chain
  OCR → correction → embeddings/ner/triples/summary via `TextDependency`, and **S4 refined by the owner**: a summary of
  a page the same batch corrects waits for that correction too, so it reads the corrected text. Read filter (kind list)
  in `list_tasks`, claim `validate_corpus_claim_input`, commit `corpus_route`, admission whitelist,
  `publish_engine_output`, `lib.rs` observers — all six enumerations. Sync: nothing to change — `processing_tasks` never
  syncs, and `llm_results` already syncs (`target_type=asset`); the correction backup stays local, as it does for the
  per-document command. Tests: `cargo test --lib processing::` 200 passed (RED first: 11 failures, including the
  "correction is not an embedding" claim pin), `llm::` 89, `sync::` 499, `db::` 121 (2 ignored), clippy/fmt clean,
  `pnpm --filter @entropia/store test` 457.
- L6 — T2 frontend done (route: done, committed as `885ef8c8`). `BatchProcessingTab.svelte` gained `runOcrCorrection`/
  `runSummary` (both off by default), two hinted toggles after Tripletes in that order («Corrección OCR»,
  «Resúmenes»), `'ocr_correction'`/`'summary'` in the `operations` array (after triples, before `schema:<id>`) and in
  both "at least one op" guards; `missingOpenrouterKey` includes both, so under Lite they block «Analizar selección»
  exactly like Entidades/Tripletes. `extractionCounts` now carries four kinds with readable count lines, and the paid
  notice for correction shows also in Pro (`alwaysPaid`) because that call is always remote. T1 exposed no draft-count
  fields for the new kinds — the snapshot only has `tasksByKind` — so the counts are read from it; task lists and the
  history keep rendering `task.kind` raw, as they do for every kind (`ocr`, `ner`, `triples`), and `batchProgress`
  already keeps non OCR/embedding units out of the page/chunk aggregate, so a correction/summary batch reports by
  settled tasks. `batch-flows.ts` `BatchFlowSteps` gained `ocrCorrection`/`summary` (`=== true`, so flows stored before
  this change load with both off); i18n got `batch.opOcrCorrection(+Hint)`, `batch.ocrCorrectionCount`,
  `batch.opSummary(+Hint)`, `batch.summaryCount` in es and en. Tests (RED first: 7 failures): `batch-flows.test.ts` +2
  (round-trip, legacy), `batch-processing.test.ts` +1 (new kinds stay out of the unit aggregate), `BatchProcessingTab
  .test.ts` +6 (order and ids, tooltip hints, key gate, flow save/restore, legacy flow, kind names in the task list).
  Desktop suite 275 files / 3865 passed (10 skipped), Lite tab 44 passed, lint and both typechecks clean,
  `pnpm format:check` clean.
- L7 — T3 assistants and manual done (route: done, committed as `51994d09`). S9: `automation/jobs.rs:201` `needs_openrouter` and
  `:219` `is_known_operation` take `ocr_correction` and `summary` (the exact strings the tab sends); the unknown-op
  refusal (`:287`), the `provider_not_configured` gate (`:304`) and the `costNote` (`:323`) name them, the note saying
  each is one paid call per page; `automation/mcp.rs:255` documents the param, `:236` adds the two task kinds to
  `jobs_get`'s `taskKind` and `:960` the `processing_prepare` description names both and says they are paid remote calls
  the person approves, like entities and triples. No tools/list snapshot exists; the published schema only gained doc
  text, so `every_tool_publishes_a_strict_input_schema` still passes unchanged. Approval flow untouched (correction and
  summaries reach the same `processing_start` approval). Docs:
  `integrations/agents/README.md:6`, `PROTOCOL.md:501-513` (operations list and Lite key list), `INVENTORY.md:88`
  (correction/summaries now "as a batch"; `documents_transform_start` stays planned). Tests (RED first: both new tests
  failed, `invalid_argument` for `ocr_correction`, before the change): `mcp_tests.rs:1156` prepares with both ops and
  asserts the job keeps them plus the description naming them as paid-and-approved; `:1245` (Lite) refuses each with
  `provider_not_configured` and creates no batch. `cargo test --lib automation::` 146 passed (144 before, +2), clippy
  `--lib --tests` clean, `cargo fmt --check` clean. No `tests/*mcp*` target exists: the endpoint tests are the unit
  module `automation::mcp_tests`, already covered by that filter.
- L8 — T3 manual (S10). Verified in code before rewording: the batch has no audio/STT operation (kinds are `ocr`,
  `embedding`, `ner`, `triples`, `schema_extract`, `ocr_correction`, `summary`), so "no transcribe audio" stays.
  `inventario-funciones-manual.md:120` now lists corrección and resúmenes and says the summaries are per page, not a
  collection-wide one; `:156` changed from "Resumir muchos documentos a la vez con los lotes" (a denial that is no
  longer true) to the one claim that still holds (`Un resumen general de toda la colección`). `manual-usuario.md` ch. 14: intro `:968`, step 2
  and the key gate `:973`, Lite/paid paragraph `:991`; new §14.1 «Corregir el texto o resumir por lote» (`:993-1002`)
  documents «Corrección OCR» (prompt in Prompts, model in Model Params, replaces each page's text, original restorable
  with «Restaurar OCR original», once per page, existing embeddings/entities/triples left alone, same-batch ops wait and
  read the corrected text) and «Resúmenes» (one per page, in the «Resumen» section of «Texto extraído», once per page,
  waits for the same batch's correction); both paid per page and needing the OpenRouter key. Later sections renumbered
  14.2/14.3 (anchors unchanged, no numeric cross-references existed); §18.4 `:1272` stops denying batch summaries.
- L9 — T4 independent verification returned two should-fix findings; both fixed here, no other behaviour changed.
  **Finding 1 (S6): correction planned without the OpenRouter key in Pro.** Correction is always remote
  (`llm/mod.rs:1658-1662` `force_remote_ocr_correction`; `correct_ocr_for_asset_blocking` calls
  `require_openrouter_key` before any request), but the UI gate skipped every check under Pro
  (`missingOpenrouterKey` began with `!LOCAL_ML`) and so did the MCP gate (`!cfg!(feature = "local-ml")`), so Pro
  without a key planned correction tasks that could only park as blocked. Fix: `BatchProcessingTab.svelte:82-94`
  gates `runOcrCorrection` in both variants and only the other ops under `!LOCAL_ML`; `:718-726` now reads the key
  on mount in Pro too (an unread key is still "not blocked"). `automation/jobs.rs:198-218` splits the old
  `needs_openrouter` into `is_always_remote` (correction), `runs_on_openrouter_in_lite` (embeddings, ner, triples,
  summary, `schema:`) and `needs_openrouter_key(operations, local_ml)`, a pure helper taking the variant explicitly so
  the Lite test build can assert the Pro branch; `:308` calls it with `cfg!(feature = "local-ml")`. Message and refusal
  code unchanged. **Summary in Pro verified read-only and left as is:** `llm::summarize_for_asset_blocking` goes
  through `run_asset_job_blocking` (`llm/mod.rs:1441-1481`), which reads `llm_mode` (default `local`) and tries the
  local engine first, falling back to OpenRouter only for `auto` with a key — exactly what the per-page summary button
  and the queue worker do. So a Pro summary needs no key (it can fail for lack of a local model, a real error, not a
  missing key), Pro keeps skipping the gate for it, and only correction became variant-independent.
  **Finding 2 (migration 0060 store test gap):** `packages/store/src/runner.test.ts:3079` adds a 0060 block modeled on
  0058 — a `.sql` mirror check, and a rebuild test that reaches the 0059 shape with `upTo0059()` (a shim that reports
  0060 as already recorded so the applier skips it, then verifies its `_migrations` row is absent), seeds
  `processing_batches`/`processing_tasks`/`processing_batch_tasks`/`processing_attempts`/`processing_checkpoints`, runs
  the real 0060, and asserts every row survived unchanged, `ocr_correction` and `summary` insert in both tables, an
  unknown kind is still rejected on both, a batch-task with a ghost batch is refused (FK survived), the restored
  `processing_tasks_settle_dependents` trigger still promotes a blocked dependent to pending and
  `idx_processing_tasks_state_id` exists, and 0060 is recorded once. The fake future migration at `:2919` is renamed
  `0060_from_the_future` → `9999_from_the_future` so it cannot collide with a real 0060. Tests: `BatchProcessingTab
  .test.ts:1307` (Pro) asserts correction alone disables «Analizar selección» with the key tooltip and never calls
  `processing_prepare`, that unchecking frees it, and that summaries alone do not block in Pro;
  `jobs.rs:540` covers the helper for both variants (correction yes, ocr never, the rest only in Lite);
  `mcp_tests.rs:1268` asserts `processing_prepare` with `ocr_correction` and no key answers `provider_not_configured`
  and creates no batch (runs in both variants). RED first: the Pro tab test failed with the button still enabled, and
  the Rust tests failed to compile (`needs_openrouter_key` missing). GREEN: desktop tab 43 passed / 2 skipped (Pro),
  44 / 1 skipped (Lite); `cargo test --lib automation::` 148 passed (146 before); store 459 passed (457 before);
  clippy `--lib --tests` and `cargo fmt --check` clean, lint and Pro typecheck clean, `pnpm format:check` clean.
