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
| T1 | S2,S5 | inline (explorer out of quota): migration + store + registry + fixture, tests | done | b88ea866 |
| T2 | S3 | inline: prompt, parser, offset resolution, persistence on both insert paths, tests | done | e4306e0c |
| T3 | S4 | inline: frontend highlight from stored evidence with fallback, tests | in review | (T3 commit) |
| T4 | S1-S5 | verify (Risk items 1: migration of synced data; 3: stored/synced format), PR + merge | pending | |

## Log

- **L1** (user decision in feature relation-evidence, verbatim): "Eso + guardar evidencia en extracciones futuras".
- **L2** (user, verbatim, after being told the migration raises the account schema_tag on first push): "tienes mi visto bueno apra T2".
- **L3** Evidence: `triples` schema `packages/store/src/schema.ts:196-206`; last migration `packages/store/src/migrations/0061_asset_process_events.sql`, Rust registry `apps/desktop/src-tauri/src/db/migrations.rs:204-205` (`migration!(name, Atomic)`), generated map via `pnpm --filter @entropia/store generate-migrations`; prompt `llm/prompt.rs` `DEFAULT_TRIPLETS_PROMPT`; parser `llm/mod.rs` `LlmTriple` (~:1038, serde aliases), `parse_triples_json` :1191; inserts `store_triples_for_item` :1264/:1279 and `replace_triples_for_asset` :1354/:1368; `processing/triples.rs` publishes.
- **L4** Sync evidence: capture triggers only record `(table, row_id)` (`sync/capture.rs:100-112`) and push reads the full row, so new columns travel with no sync change; apply intersects with local columns and journals `schema_drift` for unknown ones (`sync/apply.rs:360-384,499-527`); `read_schema_tag` is the max `_migrations` name (`sync/engine.rs:903-916`), so 0062 raises it.
- **L5** Decisions while implementing: offsets are resolved at write time against the asset's current text (`text_provider::get_asset_text`), the text the reader highlights; item-level triples store the sentence without a span. The frontend trusts a span only when its slice still reads as the sentence (folded), else searches the sentence with the whitespace/diacritic-tolerant matcher, else falls back to the PR #255 scoring. Settings stores the default prompt text when saved, so a stored copy of the previous default (`LEGACY_DEFAULT_TRIPLETS_PROMPT`, byte-identical to the old Settings copy) resolves to the new default; a vitest pins the Settings copy to the Rust default. Local generation keeps max 1024 output tokens (n_ctx 4096: raising it would shrink the input), so the prompt asks for a fragment of at most 200 characters; local runs may return fewer triples per page than before — not measured (no model run).
