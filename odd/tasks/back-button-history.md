# Back button: the immediately previous screen, always

## Objective

"← Volver" in the TopBar always returns to the screen shown immediately
before, never skipping one, in every combination of navigation paths.

## Problem (audit 2026-09-24)

`lib/navigation.ts` has four operations; three of them make Back skip screens:

- `navigate` pushes — correct.
- `openRootSection` (TopBar section icons, Inicio actions and quick access,
  batch/sync indicators, Chat → Investigación) rebuilds the history as
  origin + section, dropping any trailing sections: Inicio → Chat →
  Investigación → Back lands on Inicio.
- `resetToPath` (Colecciones icon, breadcrumb clicks, some sidebar flows,
  after deleting an asset) replaces the whole history: Chat → Colecciones →
  Back lands on Inicio.
- `replace` (previous/next document arrows, sidebar selections, Escritura
  list/document) overwrites the current screen: document A → next (B) →
  Back lands on the collection.

## Decisions (user request 2026-09-24)

- History behaves like a browser's: every screen change pushes; Back pops
  exactly one. Supersedes the earlier "Back climbs the hierarchy" model; the
  breadcrumb still shows the hierarchy, and clicking a crumb also pushes.
- Pushing a view equal to the current one is a no-op (tapping the section you
  are in adds nothing).
- `replace` remains only where there is no other screen to return to: a
  different page (asset) of the same document, a rename of the open
  document/writing, and after deleting the screen's own subject (it no
  longer exists). Every such call site carries a comment saying why.

## Tasks

- [x] T1 Browser-like history across every navigation call site, with a test per combination (delegated)
  - Commits `f8337ae8` (store: `navigate` pushes, no-op on an equal view, 200-entry cap keeping the root; `openRootSection` = navigate; `originPath` removed), `8091049c` (TopBar: sibling arrows, breadcrumb, Colecciones icon push; deleting the last asset replaces with its collection), `56c4b5ce` (DocumentExplorer pushes, `replace` only for another page of the same document), `06fb6058` (ItemView full-text jumps push). Remaining `replace`: asset paging, rename, deleted subject, WritingView no-history fallback — each commented. `resetToPath` kept only as test scaffolding.
  - RED 6 + 6; GREEN desktop 165 files / 2179; typecheck Pro+Lite, lint, format:check clean. Parent spot check: navigation/TopBar/DocumentExplorer 108 passed.
- [x] T2 Prune deleted subjects from history (regression opened by T1): deleting a collection, document, page (asset), writing or research job removes every history entry pointing at it, collapsing consecutive duplicates, so Back never reaches a screen that no longer exists
  - Delegated: `136a7a24` (`navigation.forget(predicate)` + `forgetCollection/Item/Asset/Writing/Research`: removes matches anywhere, collapses consecutive duplicates, keeps the root, one emit), `b6128bdf` (wired after a successful delete in CollectionsView, CollectionView, TopBar asset delete — after its `replace`, WritingStore.trashDocument, ResearchView; research.ts untouched). RED 11 + 5 sites; GREEN desktop 165 files / 2200; typecheck Pro+Lite, lint, format:check clean. Parent spot check: 7 suites, 194 passed.

## Checks

TDD strict. `pnpm --filter @entropia-pro/desktop test`, typecheck (Pro and
`VITE_LOCAL_ML=0`), lint, `pnpm format:check`. Visual check by the user.
Delivery: commits on `main`; the user decides the push.

## Progress

- T1, T2 done 2026-09-24. Not pushed; waiting for the user's manual check.
