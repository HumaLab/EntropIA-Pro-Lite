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

- Inicio is the startup view. The TopBar title is plain text with the EntropIA
  mark (the user removed its link, T3o); Volver leads back to Inicio.
- "Importar fuentes" opens a dialog: pick an existing collection or create a
  new one, then the file picker (user choice, 2026-09-23).
- Embeddings are counted in both variants: Lite embeds through OpenRouter
  (`src-tauri/src/nlp/embeddings.rs`); only the local model is Pro-only.
- Animated constellation: only on Inicio (user decision, T6); other views keep
  the static field.
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
- [x] T3i Corpus fixes from the user's review (2026-09-23): OCR universe = documents with a scanned PDF (no native text layer) or an image; STT universe = documents with audio; numerators are subsets of their universe; show real denominators and %. Embeddings was clipped by the fixed 250px top row: the row takes its content height, nothing overflows the panel
  - Delegated: `7bc4be0` (universes on `assets.type` 'image'|'pdf'|'audio', set by `classifyFileType`; OCR universe = image, or pdf without non-empty native extraction; STT universe = audio; numerators joined to their universe; `ocrUniverse`/`sttUniverse` in `CorpusStats`, still one statement; empty universe shows "—"), `b6ccd29` (top row sized by content, corpus panel `overflow: visible`, CSS guards). RED store 8/67, desktop 4/52; GREEN store 294, desktop 155 files / 2016; typecheck Pro+Lite, lint, format:check clean. Parent spot check: item.repo tests passed.
- [x] T3j Bug: Inicio failed with "db_select/db_select_rows accept only a single SQL statement" (inline, one file + test)
  - Cause: T3i put a ';' inside an SQL comment of `getCorpusStats`; `validate_sql_row_query` (src-tauri/src/db/commands.rs:556) rejects any ';' in the normalized text. Store tests ran the SQL straight on SQLite, so nothing caught it.
  - Fix: comment reworded; new test mirrors the Rust rule on the SQL actually sent (one statement, no ';', starts with SELECT/WITH). RED: 1 failed on the ';'. GREEN: store 295; typecheck, lint, format:check clean.
  - Noted: a failing stats query blanked the whole page; fixed in T3l.
- [x] T3k Polish from the user's screenshot (2026-09-23): Continuar rows stay compact (no stretching to the corpus height; spare space below); Continuar type for a collection reads "Colección" (singular); quick-access arrows more visible at rest, still sober
- [x] T3l Per-panel degradation: a failing source (stats, Continuar sources, activity) only affects its own panel; the rest of the page renders
  - Delegated: `4691cd92` (rows `flex: 0 0 auto`, padded, spare space below), `2d1f9dca` (`home.continuar.type.collection`), `d2aa9b5e` (arrow at full opacity in text-muted, secondary on hover), `ad5bd3d3` (`stats: CorpusStats | null`, `errors: { stats?, continuar?, activity? }`, each source loads on its own, `isFirstRun` false when a Continuar source failed, inline error per panel). GREEN desktop 155 files / 2028; typecheck Pro+Lite, lint, format:check clean. Parent spot check: home + HomeView tests 96 passed.
- [x] T3m The Inicio crumb on the home page was a link to Colecciones (`getBreadcrumbPath` treated every non-collections view's first crumb as a link). Home is now a root like Colecciones: plain text, `aria-current="page"` (inline, one file + test). RED 1 failed; GREEN desktop 155 files / 2029; typecheck Pro+Lite, lint, format:check clean.
- [x] T3n Header copy and crumb (user request, inline): page header is eyebrow "Inicio", heading "Espacio de trabajo", line "Investigar, analizar y escribir." (new key `home.heading`, `home.eyebrow` removed). The home page has no top-bar crumb at all (`breadcrumbForView` returns []); the user did not want "Inicio" there even as plain text. RED: header tests 2, navigation 2; GREEN desktop 155 files / 2029; typecheck Pro+Lite, lint, format:check clean.
- [x] T3o Top-bar title (user request, inline): "ENTROPIA LITE" is plain text again (no link; drag region restored), with the EntropIA "e" mark on its left. The mark is `src/assets/hlab-mark.png` (black on transparent, same file as `public/splash-mark.png`) used as an inline `mask-image` over `currentColor`, so it follows the title colour on every theme. `topbar.homeAria` removed. Home stays reachable through Volver (navigation tests: back from any root section returns to home). RED 1 failed; GREEN desktop 155 files / 2029; typecheck Pro+Lite, lint, format:check clean.
  - Note: `design-tokens.test.ts` rejects a `var(--x)` without fallback unless it is a published token or declared in the component's CSS; a value set through `style:--x` does not count.
- [x] T3p Mark shape bug + header line (inline). The title mark showed a solid disc: hlab-mark.png is a white disc behind a black 'e', and a CSS mask only reads opacity. New `src/assets/entropia-mark.png` (derived: alpha = darkness, cropped to the 'e', 161 x 210, shown at 11 x 14) + source guard test (`50e1c35f`). Header line now "Para organizar, procesar, explorar, analizar y escribir con trazabilidad." (`home.description`, en mirrored). GREEN desktop 155 files / 2030; typecheck Pro+Lite, lint, format:check clean.
- [x] T3q Quick-access arrows at `--font-size-lg` (18px, was xs 12px), `line-height: 1` (`2ad64481`, inline). T3k had only made them readable in color; the user expected them bigger too. GREEN desktop 2031.
- [x] T4 Import dialog: choose/create collection, then pick and import files (delegated)
  - `d10a435f` extracts CollectionView's pipeline into `lib/collection-import.ts` (`importClassifiedPathsIntoCollection`, progress through `onProgress`); CollectionView's 45 tests unchanged and green. `c15d58b4` adds `ImportSourcesDialog` (radio list of collections + "Nueva colección"; files are picked BEFORE a new collection is created, so cancelling the picker leaves nothing behind; duplicate names allowed, as the existing create flow allows them). GREEN desktop 157 files / 2050; typecheck Pro+Lite, lint, format:check clean.
  - Parent review found gaps: the import result was discarded (no summary for rejected/errors/duplicates, silent navigation when nothing imported), no progress in the dialog, explorer sidebar not notified; dialog tests never seen RED. Reopened as T4b.
- [x] T4b Import dialog: show the import summary (stay open on problems), progress while importing, notify the explorer, mutation-check the dialog tests (delegated, `ba223ed3`)
  - Shared `buildImportSummary` (collection-import.ts) and `notifyDocumentExplorerCollectionChanged` (document-explorer.ts), used by CollectionView too (its 45 tests unchanged). Dialog phases choosing → importing → summary; clean import navigates, anything else (incl. nothing imported) stops on the summary with "Ir a la colección" / "Cerrar"; Cancel/Escape inert while importing (the engine cannot cancel).
  - RED 7 failing. Mutation check: disabled rule and picker-cancel rule each caught by 2 tests; the chosen-collection id was caught by none, so a test choosing the second collection was added (RED against the mutation, GREEN after). GREEN desktop 157 files / 2061; typecheck Pro+Lite, lint, format:check clean. Parent spot check: dialog + import + CollectionView tests green.
- [x] T4c Bug from the user's check: re-importing a duplicate from Inicio showed no summary (the collection view did). Cause (most likely, matches every observation): ConfirmDialog cancels on any overlay click, and in the summary phase cancel = close. Picking a file by double click in the OS picker closes it on the first click, and the rest of the gesture lands on the webview overlay; a duplicate imports instantly, so the summary was up and got closed unseen. Fix: `dismissOnOverlay` prop on ConfirmDialog (default true, buttons and Escape still cancel), set false on ImportSourcesDialog (inline; ui prop + dialog). RED: 1 ui + 1 dialog test reproducing the stray click. GREEN ui 782, desktop 157 files / 2062; typecheck Pro+Lite, lint, format:check clean. Commits `5ec6dc8e`, `e3be2cc3`. Confirmed by the user in the running app (2026-09-23).
  - Seen in the same screenshot, out of this feature's scope: CollectionView still shows "7 items", "37 assets", "1 asset" (terminology rule: Documento / Página).
- [x] T5 Actions: Nueva investigación, Nuevo documento, Crear colección, Ver guía (delegated, `2116c496`)
  - Nuevo documento: `writing.createDocument(t('writing.newDocumentTitle'))` (the same store call as WritingView) then opens it; inline alert on failure. Nueva investigación: unchanged, ResearchView's create form is always visible, so the section already lands on it. Crear colección: shared `requestCreateCollection` (document-explorer.ts) extracted from AppShell's sidebar flow (navigate + `entropia:create-collection` event that CollectionsView already handles); AppShell and Inicio both use it. Ver guía de inicio: omitted, there is no in-app help anywhere (the TopBar has no help button; the parent had assumed one from a screenshot icon, wrongly).
  - RED 2 + 2; GREEN desktop 158 files / 2066; mutation checks caught by the new tests; typecheck Pro+Lite, lint, format:check clean. Parent spot check: 82 passed across the four touched suites.
- [x] T6 Constellation animation: animated ONLY on Inicio (user decision 2026-09-23), hlab.com.ar style (points drifting in a 3D box, distance links, slight pointer parallax); every other view keeps today's static field; honours prefers-reduced-motion; stops when leaving Inicio or when the window is hidden
  - Delegated `e84c7253`: `constellation-motion.ts` (pure port of hlab decor.js: 60 points, 3D box 16×10×4, bounce, perspective f = scale / max(6 − z, 0.1), link when squared distance < 6, camera easing 0.015), `animated` prop on EntropicConstellation, AppShell passes `animated={$navigation.current.name === 'home'}`; point alpha 0.14, link alpha 0.06 (theme colors via readThemeColor). RED 5/9, GREEN; mutation checks caught. GREEN desktop 160 files / 2094.
  - Parent review: the writer had kept the bootstrap guard `not.toContain('requestAnimationFrame')` green by moving the rAF call into another module, which dodged the rule instead of restating it. Guard rewritten (`955d3e13`): the static render never schedules frames, the loop starts only behind `animated`, and AppShell turns it on for home only. Mutation-checked: animating everywhere and looping the static render each fail it.
  - Known: svelte-check warns `reducedMotion` is not `$state` (the same declaration existed before T6).

## Checks

TDD: strict (session config, CLAUDE.md). Runner:
`pnpm --filter @entropia-pro/desktop test -- <files>`,
`pnpm --filter @entropia/store test -- <files>`.
Per task gates: tests, `pnpm --filter <pkg> typecheck` (desktop also with
`VITE_LOCAL_ML=0`), `pnpm --filter <pkg> lint`, `pnpm format:check`.
Visual verification: only the user can see the Tauri window.
RDD: off (clone_local) — ordinary checks only.

## Progress

- All tasks done (T1–T6). Waiting for the user's visual check of T6. next T5 (Nueva investigación, Nuevo documento, Crear colección, Ver guía), then T6 (constellation, needs a decision).
