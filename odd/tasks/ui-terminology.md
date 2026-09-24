# UI terminology: Colección → Documento → Página

## Objective

No user-visible text says "item"/"ítem" or "asset". The UI hierarchy is
Colección → Documento → Página (internal `item` = Documento, `asset` = Página).
Identifiers, types, DB columns, APIs and log messages keep their names.

## Problem

The user set the rule on 2026-09-23. Inicio and the collection view already
follow it (commits `77c15830`, `2e5c4ed0`). The rest of the app still shows
the internal words.

## Scope

- Every `i18n.ts` value (es and en) that shows item/ítem/asset to the user: the
  document view (`item.*`), the sidebar (`explorer.*`), Escritura
  (`writing.*`), Investigación, Lotes and any other key found.
- Hard-coded visible strings in components (markup text, `aria-label`,
  `title`, placeholders, tooltips) in `apps/desktop/src` and `packages/ui/src`.
- Tests that assert those texts are updated first (RED), then the copy changes.

Out of scope: identifiers, i18n key names, DB/API/Rust, console/log text.

## Tasks

- [ ] T1 Inventory and rewrite every visible string, updating tests first (delegated)

## Checks

TDD strict. `pnpm --filter @entropia-pro/desktop test`, `pnpm --filter @entropia/ui test`,
typecheck (Pro and `VITE_LOCAL_ML=0`), lint, `pnpm format:check`.
Delivery: commits on `main`; the user decides the push.

## Progress

- Created 2026-09-24. T1 delegated.
