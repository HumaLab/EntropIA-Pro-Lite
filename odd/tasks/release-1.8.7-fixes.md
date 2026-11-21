# Release 1.8.7 fixes

Objective: fix the findings from testing Pro 1.8.6. Branch `fix/1.8.7` from `origin/main` (61bf1d5a); delivery `ask-on-risk`, chained PRs to main.
Test runners: Vitest (`pnpm --filter <pkg> test -- <file>`), `cargo test` in `apps/desktop/src-tauri` (Lite default; `--features local-ml` for Pro-only code).

## Specs

Each spec is the item of the same number in `odd/plans/release-1.8.7-findings.md` («To fix»), quoted there verbatim from the testing round. S13 means finding 13, and so on.

- S13 Parakeet translates Spanish audio into English (critical).
- S15 OCRL silently drops a whole line (critical).
- S14 Re-transcribing replaces the existing transcript without warning or history.
- S23 Global search sorts its hits by title, not relevance.
- S28 Motores settings are lost without a warning when not saved.
- S29 AssemblyAI transcripts lose their timestamps and speakers.
- S6 GLM-OCR layout markup is stored, indexed and shown.
- S14b (user, 2026-11-21: «Sí, con migración») Keep a history of replaced transcripts. Scope:
  - a new migration `0066_transcription_versions` adds a table holding each replaced transcript (text with speaker names, segments, model, language, duration, when it was made and when it was replaced);
  - re-transcribing copies the current transcript there in the same transaction as the replacement; an empty transcript is not kept;
  - the transcript panel lists the previous versions and restores one; the restored version swaps places with the current one, so nothing is lost; search and embeddings refresh as after an edit;
  - deleting the asset deletes its versions;
  - the warning before re-transcribing no longer says the text "no se puede recuperar";
  - assumption: the table is local to the device (not synced): syncing it needs EntropIA-Cloud to accept a new table. Like every migration, the first push raises the account schema_tag.
- S8 Status badges are in English and do not reflect stored work.
- S11 Help button covers content.
- S12 OCR log mislabels the engine.
- S21 "Sincronizar ahora" gives no feedback.
- Deferred to 1.8.8 (user, 2026-11-21: «cerramos así»): 1–5, 9, 10, 17–20, 26, 27, 30, 32, the re-chunk part of 6, and the «Improvements» list. The list is in the findings doc, section «Scope of 1.8.7».

## Tasks

| ID | Spec | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S13 | inline | done (unit + archive check; not run end to end in a Pro build) | 71a91376 |
| T2 | S15 | inline | done (unit test; not run on the real page in a Pro build) | 4a7c7577 |
| T3 | S14 | inline | done (confirm + history, see T12) | 8f1b6c2f |
| T4 | S23 | inline | done | 11d3644e |
| T5 | S28 | inline | done | fc28e04d |
| T6 | S29 | inline | done | 38f50ca2 |
| T7 | S6 | inline | done | 71613d6e |
| T8 | S31 | inline | done | 7eb93110 |
| T9 | S7 | inline | done | 8370c13d |
| T10 | S16 | inline | done | bc94d5ec |
| T11 | S24 | inline | done | 5f4d3330 |
| T13 | S12 | inline | done | bb80b0b3 |
| T14 | S21 | inline | done (not seen in the real app) | 06c7b113 |
| T15 | S8 | inline | done (not seen in the real app) | f8f297eb |
| T16 | S11 | — | pending: needs a decision on where the bubble sits | |
| T12 | S14b | inline; independent verifier (Risk item 1): no blockers | done (not seen in the real app) | a0479288 |

## Log

- L1 (2026-11-21) User: «Sí, arrancá» — answering «¿Arranco con las correcciones para 1.8.7?» after the findings document was closed.
- L2 Order: the critical items first (13, 15, 14), then the small high-value ones.
- L3 (2026-11-21) T2 root cause: Paddle on the full page reads all 5 lines (line 3 at 0.991); the layout-aware OCRL read only inside the 4 layout boxes and dropped the line between them. Fix: a full-page detection pass adds uncovered lines as plain-text blocks, before the first box below them; block orders renumbered to text order. Cost: one extra full-page recognition per OCRL page.
- L4 T3: the confirm dialog ships; keeping a history of replaced transcripts needs a schema migration (high risk) and waits for the user's decision.
- L5 Next: T7 (S6, GLM markup in indexing), then the remaining findings.
- L6 (2026-11-21) User: «Sí, con migración» — answering whether to add the transcript history even though it needs a migration. Explorer map: production writes are Rust only (`save_transcription` upsert, `update_transcription_text_cmd`); speaker renames live in `text_content`; `transcriptions` is synced, the new table stays local; schema_tag = MAX(_migrations.name).
- L7 T12 implemented: migration 0066_transcription_versions (local table, cascade on asset delete); `save_transcription` archives the current row in a savepoint before the upsert (`transcription/versions.rs`); commands `list_transcription_versions` / `restore_transcription_version` (restore swaps); panel `TranscriptVersions.svelte` loads on open; ItemView flushes a pending edit before restoring and refreshes analysis; warnings and manual 3.x updated. RED observed: `re_transcribing_keeps_the_replaced_transcript_as_a_version` (0 versions) before wiring.
- L8 Verifier: all S14b bullets PASS, no blockers; fixed advisory A2 (failed RELEASE now rolls back). Open advisories: A1 restore can hit SQLITE_BUSY_SNAPSHOT under a concurrent worker commit (clean rollback, user retries); A3 versions grow without a cap. Environmental failures on base: automation::mcp_tests/server tests (HTTP decode). Next: T7 (S6).
- L9 (2026-11-21) User: «vamos con tu recomendación» — previews and FTS now; chunks/embeddings re-chunk deferred to 1.8.8. User: «cerramos así» — close 1.8.7 with what is done + S6 + findings 8, 11, 12, 21; the rest goes to 1.8.8.
- L10 T7 done (71613d6e): `ocr_plain_text` cleans the FTS text (`index_item_from_db`) and both RagSource snippet sites; press baselines compare the cue as sent (`## La Mujer` → `La Mujer`); one-time background reindex with local marker `fts_index_text_version=plain-v1` (app_settings does not sync). RED observed for the FTS, both previews, and the reindex (compile). rag:: 235 green, nlp::fts 25 green, clippy Lite clean.
- L11 T13 (bb80b0b3): the shared PaddleX engine logs its setup under the mode that asked for it; GLM-OCR line only for OCRH. RED: compile (helpers missing).
- L12 T14 (06c7b113): «Sincronizar ahora» waits for the pass with `syncAndWait`, says «Sincronización completa.», and the card always shows the last sync time. RED observed (no last-sync line).
- L13 T15 (f8f297eb): `jobStatusLabelKey` words every badge (sin hacer / en cola / en curso / hecho / error); a resting badge with a stored result says hecho. RED observed; 6 ItemView tests updated from the English words.
- L14 T16 (S11): the bubble's position went through T14–T17 on purpose (centred on the status-bar anchor, draggable, reset every launch). Moving it is a design decision; asked the user.
