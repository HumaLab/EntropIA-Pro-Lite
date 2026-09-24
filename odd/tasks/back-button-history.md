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

- [ ] T1 Browser-like history across every navigation call site, with a test per combination (delegated)

## Checks

TDD strict. `pnpm --filter @entropia-pro/desktop test`, typecheck (Pro and
`VITE_LOCAL_ML=0`), lint, `pnpm format:check`. Visual check by the user.
Delivery: commits on `main`; the user decides the push.

## Progress

- Created 2026-09-24. T1 delegated.
