# Triple evidence: store the source sentence of each extracted relation

Future relation extractions keep the sentence each triple was built from, so
Relaciones 3D highlights it exactly. Branch `feat/triple-evidence`.

## Specs

- **S1 Store evidence for future extractions.** "Eso + guardar evidencia en extracciones futuras" (L1). The triple prompt also returns the source sentence; it is stored (nullable, migration) and used for exact highlighting; existing triples keep using the current sentence scoring (PR #255).
- **S2 Schema.** Migration `0062` adds nullable `evidence_text`, `evidence_start`, `evidence_end` to `triples` (UTF-16 offsets into the text the extraction read). Registered in the store (`migrations.generated.ts`, `schema.ts`), the Rust registry (`db/migrations.rs`) and the canonical schema fixture. Rows without evidence stay valid.
- **S3 Prompt and parser.** The default triples prompt asks for a fourth key with the verbatim sentence. The parser accepts it (and its Spanish alias) but never requires it: custom prompts and older outputs still yield triples with no evidence. Offsets are resolved in Rust against the exact text sent to the model; evidence that is not found verbatim (after whitespace normalization) is stored as text without offsets, never with wrong offsets.
- **S4 Highlight.** When a triple carries evidence with offsets that still match the current text, the passage view highlights exactly that span; otherwise it falls back to the PR #255 sentence scoring.
- **S5 Sync.** Approved by the user knowing the first push after the migration raises the account `schema_tag`, so other devices must update to keep syncing (L2).

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S2,S5 | explore (sync/consumers map) then inline: migration + store + registry + fixture, tests | pending | |
| T2 | S3 | inline: prompt, parser, offset resolution, persistence on both insert paths, tests | pending | |
| T3 | S4 | inline: frontend highlight from stored evidence with fallback, tests | pending | |
| T4 | S1-S5 | verify (Risk items 1: migration of synced data; 3: stored/synced format), PR + merge | pending | |

## Log

- **L1** (user decision in feature relation-evidence, verbatim): "Eso + guardar evidencia en extracciones futuras".
- **L2** (user, verbatim, after being told the migration raises the account schema_tag on first push): "tienes mi visto bueno apra T2".
- **L3** Evidence: `triples` schema `packages/store/src/schema.ts:196-206`; last migration `packages/store/src/migrations/0061_asset_process_events.sql`, Rust registry `apps/desktop/src-tauri/src/db/migrations.rs:204-205` (`migration!(name, Atomic)`), generated map via `pnpm --filter @entropia/store generate-migrations`; prompt `llm/prompt.rs` `DEFAULT_TRIPLETS_PROMPT`; parser `llm/mod.rs` `LlmTriple` (~:1038, serde aliases), `parse_triples_json` :1191; inserts `store_triples_for_item` :1264/:1279 and `replace_triples_for_asset` :1354/:1368; `processing/triples.rs` publishes.
