# Home view (Inicio)

## Objective

A new "Inicio" root view that opens at startup and shows the state of the
user's work: what to continue, what to start, how the corpus stands, and where
the main workspaces are. Design approved by the user on 2026-09-23 (canvas
https://claude.ai/artifact/WBhhdi7CRXexc54KNfUfjN, artboards `InicioV3` = in
use, `InicioV3Nuevo` = first run).

## Problem

The app starts on Colecciones and has no overview. Returning to the last
document, research or collection takes several clicks, and the corpus state
(OCR, embeddings, pending work) is only visible per collection.

## Scope (from the approved design)

- Header: eyebrow "Espacio de trabajo", title "Inicio", one-line description,
  actions `Importar fuentes` (primary), `Nueva investigación`, `Nuevo documento`.
- Continuar: up to 3 most recently touched collections / writing documents /
  research jobs, each reopening it.
- Estado del corpus: collections, items, items with OCR, items with
  embeddings, pending OCR, pending embeddings, sync state.
- Acceso rápido: Colecciones, Chat, Investigación, Escritura (Base de datos
  stays in the TopBar only).
- Reciente: 5 entries (name, type, size, relative modified time).
- First run (empty archive): Continuar becomes "Empezá con EntropIA"
  (Importar fuentes, Crear colección, Ver guía de inicio); Reciente hidden.
- No AI actions on the home.

## Decisions

- Inicio is the startup view; clicking the TopBar app title opens it. TopBar
  otherwise unchanged.
- "Importar fuentes" opens a dialog: pick an existing collection or create a
  new one, then the file picker (user choice, 2026-09-23).
- Embeddings are counted in both variants: Lite embeds through OpenRouter
  (`src-tauri/src/nlp/embeddings.rs`); only the local model is Pro-only.
- Animated constellation: deferred to its own task; it changes the background
  of every view, so it needs the user's call first.
- Delivery: commits straight to `main` (standing user rule), no PRs.

## Tasks

- [x] T1 Navigation: `home` view, startup default, breadcrumb, TopBar title opens it (delegated: 10 files)
  - Commit `ce2bb2a`. HomeView mounted eagerly like CollectionsView. RED: navigation.test.ts 14 failing; GREEN: desktop 152 files / 1932 passed.
- [x] T2 Data: corpus-wide stats aggregate + recent-activity loader (collections, writing, research) (delegated: 10 files)
  - Commit `395c231`. `ItemRepo.getCorpusStats()`; pending = `processing_tasks` of kind ocr/embedding in an active state (the definition its unique index already uses). `lib/home.ts` (`mergeRecentActivity`, `loadHomeSnapshot`, tolerant to one failing source). `formatRelativeDate` extracted to packages/ui, CollectionCard output unchanged.
  - GREEN: desktop 153 files / 1942, store 285, ui 781; typecheck Pro+Lite, lint, format:check clean. Parent spot check: home/navigation/HomeView tests 49 passed.
  - Gap: research jobs carry no timestamp (`research_request` goes to the research agent, owned by another session); they sort last with no date. Follow-up: expose `updated_at` from the agent.
- [x] T3 HomeView: layout, panels, first-run state, i18n es/en (delegated: 3 files)
  - Commit `0d842cd`. RED 22/22 failing; GREEN 22/22; desktop 153 files / 1962 passed; typecheck Pro+Lite, lint, format:check clean. Parent spot check: HomeView 22 passed.
  - Deviations: numbers formatted with `es-AR`/`en-US` (bare `es` does not group thousands on this ICU); Reciente rows are keyboard-reachable `role="row"` divs (a button cannot hold cells).
  - Visual check: the user reviewed it and asked for a second pass (2026-09-23), tasks T3a–T3e.
- [x] T3a Terminology and polish: Colección → Documento → Página in every visible string (item = Documento, asset = Página); intro text kept; `Importar fuentes` slightly higher hierarchy without a new hue; quick-access cards read as navigation (hover background/border, trailing →); Continuar/Estado 60/40, stacking at the existing breakpoints
- [x] T3b Continuar vs Actividad reciente: Continuar = up to 3 resumable workspaces (title, type, last modified, contextual datum when it exists: words, sources, documents), whole row clickable, untitled → "Documento sin título"; "Actividad reciente" shows different entries (e.g. recently imported documents), fewer rows rather than duplicates
- [x] T3c Estado del corpus: Colecciones, Documentos, OCR and embeddings as `n / total · %` with a very subtle bar; pending counts link to Lotes (existing navigation, no duplicated logic); one aggregate query
- [x] T3d Active-process band: shown only while OCR/embedding/import/sync work is running (e.g. "OCR · <colección> · 428 / 1.244 páginas · 34 % · Ver lote →"), takes no space otherwise; if wiring needs a large refactor, ship the component and document what is missing
- [x] T3e First run: no empty Continuar/Actividad panels; one compact "Empezá con EntropIA" block (Importar fuentes, Crear colección) without duplicating the header's buttons
  - Delegated (one writer, 5 commits): `dffe684` T3a (grid 3fr/2fr, stacks under 720px; header hierarchy already on the primary ladder), `bf3f514` T3b (`ItemRepo.findRecentlyImported`: Actividad reciente = recently imported documents, never Continuar's entities; hidden when empty), `9a5f02f` T3c (`n / total · %` + hairline bar; pending lines open Lotes through `batchStore.requestFocus` + settings, as BatchStatusIndicator does; `getCorpusStats` is one statement on the production raw path), `5fddd79` T3d (`ActiveProcessBand` fed by `batchStore` active batches), `3b40dfa` T3e (first run: header drops Importar fuentes, the block carries it).
  - GREEN: desktop 155 files / 1982, store 289; typecheck Pro+Lite, lint, format:check clean.
  - T3d gaps: no collection name in the band (`BatchSummary` lacks it; `processing_get_batch` per batch would add a call per poll); imports and sync have no unit progress source, not wired.
- [x] T3f Continuar gaps: "Documento sin título" display and word count for writing entries (from the already-loaded `current_content_json`); research sources omitted (one extra agent call per job)
  - Commit `45be1e8` (delegated). `countManuscriptWords`, `isUntitledWritingTitle` (empty or the stored default "Sin título"/"Untitled"), counts only for the ≤3 Continuar entries; meta order type · time · datum. RED 19+6 failing; GREEN desktop 155 files / 2007.
- [x] T3g Estado del corpus as a text pipeline: Colecciones, Documentos, OCR, STT, Texto, Embeddings (OCR / STT → Texto → Embeddings); embeddings counted over documents with text (embeddings ≤ texto); value, % and subtle bar per stage (final pass requested 2026-09-23 with a reference image)
- [x] T3h Acceso rápido micro-layout: more inner padding, icon/title/description/arrow away from the edges, use the box height, four cards equal height and aligned, whole card clickable, very subtle hover
  - Delegated (one writer): `e0362b7` T3g — definitions per document (item): OCR = non-empty `extractions` with `method <> 'native'`; STT = non-empty `transcriptions`; Texto = any non-empty extraction (native included) or transcription; Embeddings = `vec_assets` ∩ Texto, so ≤ Texto by construction. One aggregate statement (CTEs) on the raw path. New ActionIcon names `scan`, `nodes`. Pending lines and sync kept in the panel footer. `be05b19` T3h — cards min-height 72px, padding space-4/space-5, arrow in flow; document icon per Actividad reciente row.
  - GREEN: store 289, desktop 155 files / 2014, ui ActionIcon; typecheck Pro+Lite, lint, format:check clean. Parent spot check: item.repo 62, HomeView 50 passed.
- [ ] T3i Corpus fixes from the user's review (2026-09-23): OCR universe = documents with a scanned PDF (no native text layer) or an image; STT universe = documents with audio; numerators are subsets of their universe; show real denominators and %. Embeddings was clipped by the fixed 250px top row: the row takes its content height, nothing overflows the panel
- [ ] T4 Import dialog: choose/create collection, then pick and import files
- [ ] T5 Actions: Nueva investigación, Nuevo documento, Crear colección, Ver guía
- [ ] T6 Constellation animation (pending user decision)

## Checks

TDD: strict (session config, CLAUDE.md). Runner:
`pnpm --filter @entropia-pro/desktop test -- <files>`,
`pnpm --filter @entropia/store test -- <files>`.
Per task gates: tests, `pnpm --filter <pkg> typecheck` (desktop also with
`VITE_LOCAL_ML=0`), `pnpm --filter <pkg> lint`, `pnpm format:check`.
Visual verification: only the user can see the Tauri window.
RDD: off (clone_local) — ordinary checks only.

## Progress

- T1–T3h done. T3i (OCR/STT universes, clipped Embeddings) in progress; then the user's visual check; then T4.
