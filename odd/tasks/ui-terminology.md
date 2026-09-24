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

- [x] T1 Inventory and rewrite every visible string, updating tests first (delegated)
  - Commits `df648920` (ItemCard fallback page/pages), `61a46506` (CollectionCard `countLabel`, CollectionsView passes `collections.cardDocumentCount.*`), `fc77b3fc` (document view `item.*`), `b03f3be7` (sidebar `explorer.*`), `80549c40` (Escritura), `c1a5c8a9` (settings, investigation.sourceUnavailable, collections.deleteMessage…), `50594100` (PDF split error). RED: ItemCard 2, CollectionCard 3, DocumentViewer 1, ItemView 6+1. GREEN desktop 2102, ui 784; typecheck Pro+Lite, lint, format:check clean. Parent spot check: 171 + 91 passed; independent scan of i18n values (scratchpad scan_terms.py) and svelte markup/attributes finds nothing else.
  - Judgment calls: audio assets read "páginas de audio" (the rule makes Página the general term); "assets ONNX" became "archivos ONNX" (model files, not the domain); the dev-only FTS debug panel and the collapsible technical meta line were translated too; `file-import.ts` 'Failed to delete asset file' stays (console only).
  - Left on purpose: `investigation.report.items` ("Items") in InvestigationView, which another session owns.
- [x] T2 Audio is "audio", never "página de audio" (user decision 2026-09-24): `settings.assemblyAiSpeakerLabelsHint` and `item.layoutUnavailableForAudio` (es/en). New guard `lib/i18n-terminology.test.ts` reads every i18n value and fails on visible item/ítem/asset (allowlist: `investigation.report.items`) and on "página(s) de audio"/"audio page(s)". RED: audio 4 offenders; item/asset guard mutation-checked (an injected "asset" fails it). GREEN desktop 161 files / 2104; typecheck, lint, format:check clean.

## Checks

TDD strict. `pnpm --filter @entropia-pro/desktop test`, `pnpm --filter @entropia/ui test`,
typecheck (Pro and `VITE_LOCAL_ML=0`), lint, `pnpm format:check`.
Delivery: commits on `main`; the user decides the push.

## Progress

- T1 done 2026-09-24. Not pushed; the user decides.
