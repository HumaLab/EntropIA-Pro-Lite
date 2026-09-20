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

Acceptance and checks:

- Every mandatory capability has an exercised path or a named blocker before its implementation stage.
- No private Zotero content, credential, or unapproved write appears in the repository or logs.
- Documentation formatting/readback is clean; product tests are N/A for a docs-only unit.

### ZSB-E1 — Preserve identity and synchronize a trusted catalog

- [ ] E1a: preserve native key plus library/namespace through listing, selection, citation save/edit, and historical-reference compatibility.
- [ ] E1b: add catalog migrations/relations and durable sync for items, collections, tags, and attachments.
- [ ] E1c: add connection/library selection, work details, opening, offline state, and stale-response isolation.

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
- Current blockers: the personal isolated profile must be scheduled because one Zotero instance exposes one personal library; CSL inclusion, collections, attachments/file resolution, opening, and Web API remain unverified. The group treeViewID is runtime-local and must never be hardcoded.
- Evaluation seed: `zsb-eval-v1`, using synthetic or explicitly authorized material, opaque IDs, and human passage-level judgments before E3/E7 tuning.
- Unrelated working-tree path: `.gentle-ai-default-agent.json` (leave untouched and uncommitted).

## Next step

Record and commit the live group evidence without touching product code. E1 may proceed on the verified local group path; schedule the isolated personal-profile run before claiming personal-library compatibility, and keep CSL, attachments, file opening, and Web API as explicit follow-up blockers.
