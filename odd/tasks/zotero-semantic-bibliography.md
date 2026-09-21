# Zotero semantic bibliography implementation

## Objective

Implement the approved master plan in `docs/superpowers/plans/plan-capa-semantica-bibliografica-zotero.md` on `feature/zotero-bibliografia-semantica`, preserving Zotero as bibliographic authority and keeping bibliography separate from the documentary corpus.

## Problem and why

EntropIA can currently list local Zotero CSL records and cite them, but it does not preserve verified native Zotero identity end to end, synchronize personal/group catalogs durably, process bibliographic attachments independently, or retrieve passages with verifiable provenance. The implementation must add those capabilities without weakening existing corpus OCR, citation export, offline use, privacy, or Pro/Lite behavior.

## Scope and constraints

- The master plan is the source of truth for functional scope, stage ordering, acceptance, and rollback.
- Work only on `feature/zotero-bibliografia-semantica`; do not merge or publish without user authorization.
- Never commit the unrelated untracked `.gentle-ai-default-agent.json` or Cargo lock drift caused by the local `entropia-agent` patch.
- Zotero writes require explicit authorization and isolated test data; private library content must not be logged or committed.
- E0 is documentation-only. Product code starts only after E0 records exercised capabilities and resolves blockers.
- Pro/Lite variants, existing manuscripts, historical citations, and corpus OCR remain supported.
- Each implementation task closes as a coherent work-unit commit with focused verification and rollback evidence.
- Delivery strategy: work directly in this dedicated worktree and `feature/zotero-bibliografia-semantica`, with one coherent work-unit commit per task. Do not use a Feature Branch Chain or stacked PRs toward `main` unless the user later changes this decision, despite the forecast exceeding 400 authored lines.
- Effective ODD TDD mode: strict TDD from E1 onward (RED → GREEN → REFACTOR), selected by the user in this session. Known runner: root multi-project Vitest (`pnpm test`) with focused package scripts, plus focused Rust `cargo test` commands from `apps/desktop/src-tauri`.
- E0 Zotero access decision: read/write is authorized only for explicitly selected, isolated personal and group test libraries. No private-library access and no write may occur until concrete test destinations and connections are selected.

## Route and workload forecast

The feature is expected to require roughly 5,400–8,500 authored changed lines across all stages. This is a review-load forecast, not a cap and not permission to omit tests or documentation.

| Task | Forecast | Route | Trigger evidence |
| --- | ---: | --- | --- |
| ZSB-E0 | 40–80 | Parent inline after delegated mapping | One documentation target; repository map delegated under the 4-file rule |
| ZSB-E1 | 800–1,200 | Delegated writer, sliced by work unit | Multi-file Rust/TS/SQL/Svelte change |
| ZSB-E2 | 700–1,100 | Delegated writer, sliced by work unit | Shared scheduler, migration, repository, and tests |
| ZSB-E3 | 800–1,200 | Delegated writer, sliced by work unit | Embedding contract, repository, retrieval, UI, and tests |
| ZSB-E4 | 1,200–1,800 | Delegated writer, sliced by work unit | New extraction/layout pipeline across Rust, store, UI, and tests |
| ZSB-E5 | 700–1,100 | Delegated writer, sliced by work unit | Durable Zotero ingestion and upload workflow |
| ZSB-E6 | 700–1,100 | Delegated writer, sliced by work unit | Editor, canonical citation contract, CSL, and exporters |
| ZSB-E7 | 500–900 | Delegated writer, sliced by work unit | Retrieval composition, provenance, agent capability, and evaluation |

## Actionable checklist

### ZSB-E0 — Verify capabilities and freeze implementation decisions

- [x] Record explicit authorization limited to isolated personal/group Zotero test libraries; group `prueba` (`6680944`) is now a confirmed local test destination, while the isolated personal profile remains pending.
- [x] Resolve effective ODD TDD mode as strict RED → GREEN → REFACTOR and record its source.
- [x] Record live local group evidence and every unverified identity/CSL, personal-profile, collection, attachment, file-resolution, opening, and Web API path as an explicit blocker in the master plan.
- [x] Freeze the evaluation seed `zsb-eval-v1` and the timing/mechanism for passage-level judgments before E3/E7 tuning.
- [x] Review the three user decisions; no additional provider or cost change was introduced.
- [x] Commit the documentation work unit with no product changes (`caa7a8d233ce194e0ddc285da7abf506b0f719b8`).
- [x] Commit the live group capability evidence with no product changes (`78cc46f`).

Acceptance and checks:

- Every mandatory capability has an exercised path or a named blocker before its implementation stage.
- No private Zotero content, credential, or unapproved write appears in the repository or logs.
- Documentation formatting/readback is clean; product tests are N/A for a docs-only unit.

### ZSB-E1 — Preserve identity and synchronize a trusted catalog

- [x] E1a: preserve native key plus library/namespace through listing, selection, citation save/edit, and historical-reference compatibility.
  - [x] E1a-1: preserve `key`, `itemVersion`, `libraryType`, `libraryId`, and CSL snapshot through connector/mirror, listing/search, and initial citation insertion on `user/0`; strict-TDD focused checks passed in commit `67140c0b4cba08e005c0f67152f179bf6bb9d33a`.
  - [x] E1a-2: preserve identity through citation editing, projections, clusters, history, and cross-library collision handling.
  - [x] E1a-2a: preserve qualified identity through the citation editor and cluster equality, including unknown-instance non-merging; commit `3333683e6488515e9dcfcb024b416e5df9094639`.
  - [x] E1a-2b: preserve identity through canonical projection/persistence and schema v2 with v1 legacy reads; commit `3c0b9be`.
  - [x] E1a-2c: preserve identity through history, legacy citations, and export collision handling; commit `85cc0ee`.
- [x] E1b: add catalog migrations/relations and durable sync for items, collections, tags, and attachments.
  - [x] E1b-1a: add persistent connections/libraries/items with native+CSL snapshots and idempotent cross-library upsert; commit `c5f90b3`.
  - [x] E1b-1b: add collection/tag/attachment relations, native snapshots, and explicit tombstone metadata without file resolution; commit `352cdc3`.
  - [x] E1b-2: add durable per-library reconciliation cursor, seen-set, errors, and retry state; commit `f011489`.
  - [x] E1b-3: expose confirmed-catalog reads through the existing Zotero seam without E1c selector/opening behavior.
- [ ] E1c: add connection/library selection, work details, opening, offline state, and stale-response isolation.
  - [x] E1c-1: typed library transport (users/groups) in the Rust connector/mirror/commands plus an explicit selection state in the TS store with library-keyed stale-response isolation; no selector UI yet; commit `c5b6939`.
  - [ ] E1c-2: selector UI listing locally known libraries (mirrors + persisted catalog + user/0) with manual add validated by a cheap version probe.
  - [ ] E1c-3: work details (ficha) and offline/lost-link states via new confirmed-catalog detail reads.
  - [ ] E1c-4: `writing_zotero_open_item` with strict qualified identity and a validated OS opener, disabled until one authorized live session verifies `zotero://select` against `prueba`.

### ZSB-E2 — Extend the single scheduler with bibliographic subjects

- [ ] E2a: migrate task identity to domain/subject/revision without changing documentary task meaning.
- [ ] E2b: run bibliographic synchronization through the existing scheduler with durable retry demand.
- [ ] E2c: add interactive priority, progress, cancellation, and stale-publication barriers.

### ZSB-E3 — Add semantic work profiles and hybrid work search

- [ ] E3a: implement effective embedding contracts and scoped consent.
- [ ] E3b: build/version profiles and index works without PDFs.
- [ ] E3c: add lexical/vector/filter retrieval and atomic generation publication, including incremental additions.

### ZSB-E4 — Extract bibliographic PDFs and retrieve passages

- [ ] E4a: resolve attachments and extract native text plus layout without corpus assets or OCR calls.
- [ ] E4b: add measurable selective OCR quality decisions, independent execution, integration, and checkpoints.
- [ ] E4c: add structural chunks, multi-page spans, embeddings, and invalidation.
- [ ] E4d: add hierarchical passage search plus a concrete original-PDF opening/highlight surface.

### ZSB-E5 — Link or create Zotero records before indexing

- [ ] E5a: add the durable pending tray and explicit match/library decision.
- [ ] E5b: add idempotent parent/attachment creation or linking with receipts and conflict handling.
- [ ] E5c: recover/cancel operations and start processing only after verified Zotero completion.

### ZSB-E6 — Connect related search to citations and exports

- [ ] E6a: search from an anchored manuscript selection with scoped consent.
- [ ] E6b: insert/edit simple, multiple, narrative, parenthetical, and note citations with stable identity.
- [ ] E6c: preserve CSL switching, bibliography, export, historical snapshots, and unresolved legacy references.

### ZSB-E7 — Compose retrieval domains and evaluate quality

- [ ] E7a: support corpus-only, bibliography-only, and combined retrieval with separate budgets and provenance.
- [ ] E7b: validate model-proposed references against supplied evidence and domain.
- [ ] E7c: evaluate reranking, expansion, selected notes, and optional summaries before activation.

## Progress and evidence

- Dedicated worktree: `G:/EntropIA-Stack/EntropIA-Pro-Lite-worktrees/zotero-bibliografia-semantica` on `feature/zotero-bibliografia-semantica`, initial implementation boundary `9c85295`.
- The original checkout `G:/EntropIA-Stack/EntropIA-Pro-Lite` was released cleanly to `main` at `a9bdd5d`; no further work is allowed there.
- Worktree environment restored with `pnpm install --frozen-lockfile`; CodeGraph initialized successfully in the dedicated worktree.
- Repository mapping completed by delegated read-only explorer; the E0 live group probe was later executed only against the isolated test group, with no product tests, builds, or application writes.
- Existing capability evidence: local connector and mirror; mocked/unit tests plus one controlled live group probe. The live probe confirmed Zotero 9.0.3, local endpoint `127.0.0.1:23119`, group `prueba` (`6680944`), runtime target `L9`, native key `7EMV3G8H`, and local writes without an API key; frontend still defaults to library `0` and relies on CSL identity.
- E0 decisions recorded: isolated personal/group read-write authorization, strict TDD from E1, and direct delivery in this worktree with work-unit commits.
- E0 documentation work-unit committed as `caa7a8d233ce194e0ddc285da7abf506b0f719b8` (`docs: define verified Zotero bibliography capabilities`); no product files changed.
- E0 live evidence: `saveItems` returned `201`, `updateSession` returned `200`, and group readback returned one controlled fixture (`book`, `version=3`, `Last-Modified-Version=3`). The fixture remains in the isolated group for subsequent tests.
- E0 live evidence commit: `78cc46f` (`docs: record live Zotero group capability`); no product files or credentials were committed.
- E1a-1 implementation is committed as `67140c0b4cba08e005c0f67152f179bf6bb9d33a` (`feat(writing): preserve qualified Zotero item identity`): native identity is preserved through connector/mirror, frontend listing/search, and initial citation insertion; the differing-key fixture `37C8RJP8`/`moore1973` is covered by Rust and Vitest tests.
- E1a-2 decisions fixed before RED: unknown `source_instance_id` remains nullable and is never used to merge otherwise matching works across uncorroborated instances; canonical citation schema advances to v2 while v1 legacy remains readable.
- E1a-2a completed in commit `3333683e6488515e9dcfcb024b416e5df9094639`: strict-TDD RED then GREEN; desktop WritingCitationEditor/WritingZoteroTab tests (6), UI citation-cluster regression tests (61), Pro/Lite typechecks, Svelte autofixer, and diff check passed. The slice preserves `sourceOrigin`, nullable `sourceInstanceId`, library namespace, key, version, and CSL snapshot through editor apply; qualified cluster equality is conservative and retains itemKey-only legacy fallback.
- E1a-2b completed in commit `3c0b9be`: schema v2 with lazy v1 reads, item-level source origin/instance projection, Rust persistence with enum validation/default local origin, and SQL NULL preservation. Verification passed: UI 91 tests, desktop 30 tests, Rust repository 31 tests, Pro/Lite typechecks, and diff check.
- E1a-2c completed in commit `85cc0ee`: derived CSL IDs namespace qualified source identity without changing canonical snapshots, NULL instances use occurrence-safe IDs, legacy unqualified IDs remain unchanged, and v2 canonical Zotero content round-trips through version snapshots. Verification passed: desktop 38 tests, UI 64 tests, Rust repository 31 plus versions 11, desktop typecheck, and diff check.
- E1a is complete; E1b catalog persistence and durable sync is now the next implementation unit.
- E1b-1a completed in commit `c5f90b3` (`feat(bibliography): persist Zotero catalog foundation`): migration `0038_bibliography_catalog`, persistent connections/libraries/items, qualified `(library_id, item_key)` identity, native+CSL snapshots, nullable `source_instance_id`, JSON validation, transactional idempotent upsert, and synthetic RED → GREEN → REFACTOR coverage. Store migration/fixture tests (43), store typecheck/lint, Rust integration runtime tests (5), cargo check, rustfmt, and diff check passed.
- E1b-1a verification note: the Rust integration test passed with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`; no Pro/local-ML build was run and no private/live Zotero data was accessed.
- E1b-1b completed in commit `352cdc3` (`feat(bibliography): persist Zotero relations and tombstones`): migration `0039_bibliography_relations`, collection/tag/attachment tables with lossless native JSON snapshots and nullable native versions, library-safe composite membership FKs, opaque parent keys, raw nullable attachment metadata, and explicit side-table tombstones with live-upsert revival. Store tests (48), Rust integration tests (12), typecheck/lint, cargo check, targeted rustfmt, diff check, and runtime tests passed. No file resolution or catalog read seam was added.
- E1b-2 completed in commit `f011489` (`feat(bibliography): persist reconciliation state`): migration `0040_bibliography_reconciliation`, per-library fenced run/phase state, normalized seen-set, atomic checkpoints, retry/error metadata, explicit interruption/resume/block/finalize, monotonic totals, and idempotent finalization. Store tests (52), Rust catalog/reconciliation tests (25), typecheck/lint, cargo check, targeted rustfmt, diff check, and runtime tests passed.
- E1b-3 completed in commit `40abe01` (`feat(bibliography): expose confirmed catalog reads`): implemented repository-confirmed local-personal catalog projection through the existing Zotero seam; covered `completed`/`finalize` reads plus seen-set/version/verified/tombstone behavior; preserved existing `writing_zotero_cached` precedence and resilient filesystem fallback; made no selector/opening/network/frontend changes. Verification: 22 catalog tests passed with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0` workaround (the ordinary catalog test initially hit Windows `LNK1201`); 13 reconciliation tests passed; 3 `writing::commands` tests passed; targeted cargo checks, targeted rustfmt, and git diff check passed.
- The next slice is E1c; do not introduce E2 scheduling, attachment file opening, or live private-library access.
- E1c contract selected by the user: the selector lists only locally known libraries (mirrors + persisted catalog + user/0) with manual add validated by a cheap version probe (unavailable Zotero leaves the entry as "Copia local sin verificar ahora"); no unverified enumeration endpoints are consumed. Opening is implemented with strict qualified identity and a validated OS opener but stays disabled until one authorized live session verifies `zotero://select` against `prueba`/`7EMV3G8H`; CSL.id concatenation and unvalidated executable URLs are forbidden. E2 scheduling, attachment file resolution/opening, embeddings, and Web API stay out of E1c.
- E1c-1 completed in commit `c5b6939` (`feat(writing): add typed Zotero library transport`): `zotero::Library` seam (user/group, non-empty ids, storage_key `group-{id}` for groups, personal() = user/0), six URL builders and read functions routing to `/api/users/{id}` or `/api/groups/{id}` with no silent mapping of unknown types, probe pinned to user/0, per-library mirror files with user/0 byte-identical (`library-0.json`) and traversal-safe group keys, flat camelCase `{libraryType, libraryId}`(+`query`) command params with user-only confirmed-catalog precedence and mirror fallback for groups, and a TS store selection state that clears on change and discards late cached/sync/search responses from another selection or query by epoch. Verification: 231 Rust `writing::` tests, 37 TS tests (writing-zotero + WritingZoteroTab), Lite typecheck, cargo check, targeted rustfmt (only pre-existing mirror.rs hunk remains), diff check. Implemented across the subagent outage recovery: Rust and TS halves were delegated to gentle-ai-worker on the opencode-go/muse-spark-1.3-contributor fallback model after the user enabled paid endpoints that train on request data.
- Current blockers: the personal isolated profile must be scheduled because one Zotero instance exposes one personal library; Web API and live collection/tag/attachment/deletion shapes remain unverified. The group treeViewID is runtime-local and must never be hardcoded.
- Evaluation seed: `zsb-eval-v1`, using synthetic or explicitly authorized material, opaque IDs, and human passage-level judgments before E3/E7 tuning.
- Unrelated working-tree path: `.gentle-ai-default-agent.json` (leave untouched and uncommitted).

## Next step

Write E1c-2 RED tests for the selector UI: locally known libraries (mirror files + persisted catalog + user/0), manual add validated by a cheap version probe, and "Copia local sin verificar ahora" when Zotero is unavailable.
