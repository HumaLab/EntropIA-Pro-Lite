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
- [ ] T3a Terminology and polish: Colección → Documento → Página in every visible string (item = Documento, asset = Página); intro text kept; `Importar fuentes` slightly higher hierarchy without a new hue; quick-access cards read as navigation (hover background/border, trailing →); Continuar/Estado 60/40, stacking at the existing breakpoints
- [ ] T3b Continuar vs Actividad reciente: Continuar = up to 3 resumable workspaces (title, type, last modified, contextual datum when it exists: words, sources, documents), whole row clickable, untitled → "Documento sin título"; "Actividad reciente" shows different entries (e.g. recently imported documents), fewer rows rather than duplicates
- [ ] T3c Estado del corpus: Colecciones, Documentos, OCR and embeddings as `n / total · %` with a very subtle bar; pending counts link to Lotes (existing navigation, no duplicated logic); one aggregate query
- [ ] T3d Active-process band: shown only while OCR/embedding/import/sync work is running (e.g. "OCR · <colección> · 428 / 1.244 páginas · 34 % · Ver lote →"), takes no space otherwise; if wiring needs a large refactor, ship the component and document what is missing
- [ ] T3e First run: no empty Continuar/Actividad panels; one compact "Empezá con EntropIA" block (Importar fuentes, Crear colección) without duplicating the header's buttons
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

- T1–T3 done. Second pass requested by the user: T3a–T3e next, then the user's visual check, then T4.
