# Tiptap v3 migration

## Objective

Move the editors (WritingEditor, NoteEditor) from Tiptap 2.27 to Tiptap 3 (>= 3.30.4) so
`pnpm audit` reports zero advisories.

## Problem / why

GHSA-cp6q-959q-f8rh (moderate): `@tiptap/core` `mergeAttributes()` turns an own `__proto__`
key from JSON into inherited DOM attributes (proof executes `onerror`). Fixed only in
`@tiptap/core >= 3.30.4`; no 2.x backport. User decision (2026-10-21): migrate, no debt.

## Scope

- Catalog in `pnpm-workspace.yaml`, `packages/ui/package.json`, `apps/desktop/package.json`
  (incl. `tiptap-footnotes` 2.0.4 -> 3.0.1), `apps/desktop/vite.config.ts` optimizeDeps.
- `packages/ui/src/components/WritingEditor/**`, `NoteEditor/**`, `Dictation/**`.
- AGENTS.md "Dependencies" note if the package list changes.

## Constraints

- Stored HTML/JSON in SQLite must not drift: no trailing empty paragraph, links kept, mark and
  node names unchanged (`WRITING_SCHEMA_VERSION` stays 2).
- One copy of prosemirror-state/view/model (AGENTS.md "Dependencies").
- External content loads must not emit updates (autosave loops).
- Zero-warning lint/typecheck, prettier format check.

## Base

Branch `feat/tiptap-v3`, stacked on `fix/deps-audit-markdown-it` (PR #158).

## Tasks

- [x] T1 Golden tests: pin current serialization (writing round-trip, real document, note HTML)
      before the bump. Route: delegated writer. `WritingEditor/serialization-golden.test.ts`
      (rich HTML parse, atoms/footnotes JSON, idempotent remount, HTML copy round-trip, empty doc,
      trailing paragraph) and `NoteEditor/__tests__/note-serialization-golden.test.ts` (stored
      note HTML; extension list moved to `note-extensions.ts` so test and component share it).
      Checks on Tiptap 2: `pnpm --filter @entropia/ui test` 874/874, ui typecheck 0/0. Commit 8b799276.
- [x] T2 Dependency bump: catalog to ^3.32.0, add `@tiptap/extensions`, tiptap-footnotes ^3.0.1,
      single prosemirror copy. Route: delegated writer. Lock holds one copy each of
      prosemirror-model 1.25.12, -state 1.4.4, -transform 1.12.2, -view 1.42.6; `@tiptap/extensions`
      arrives through StarterKit (not imported directly). T2-T4 and the T6 shape fixes share one
      commit: each alone leaves typecheck or the golden tests red.
- [x] T3 Import fixes: named `Table*` (all four from `@tiptap/extension-table`) and `{ TextStyle }`.
      Also: our command group renamed `fontSize` -> `writingFontSize` (v3 text-style declares
      `fontSize`), `createWritingExtensions(): Extensions`, `getJSON()` typed as `JSONContent` in
      seven test helpers (v3 narrows the return type), `setContent(x, { emitUpdate: false })` at
      the four sites (v3 signature; behaviour proven in T5).
- [x] T4 StarterKit de-dup in both editors: `link`, `underline`, `listKeymap`, `trailingNode`
      off in StarterKit; Link/Underline configured once as before; TrailingParagraph kept.
- [x] T5 `setContent(x, false)` -> `{ emitUpdate: false }` (4 sites) + test that external loads
      do not fire updates. Route: delegated writer. `WritingEditor.externalLoad.test.ts` and
      `NoteEditor/__tests__/NoteEditor.externalLoad.test.ts` count `update` events whose
      transaction changed the document during a prop-driven load. RED observed on Tiptap 3 with
      the old `false` argument (both fail, 1 update each); GREEN with `{ emitUpdate: false }`.
      No `clearContent` call sites exist. Note: `setEditable` emits an `update` too (v2 and v3
      alike, no document change); WritingEditor's `lastEmitted` guard already ignores it.
- [x] T6 TextStyle `mergeNestedSpanStyles` vs `WritingTextStyle.parseHTML`; insertContent
      regression (dictation, suggestions, search-replace, citations). The parseHTML override
      already bypasses the merge (golden: nested span keeps v2 marks). Golden caught three stored
      JSON drifts, fixed without touching the snapshots: v3 Link adds `title`, v3
      TableCell/TableHeader add `align` (both stripped: an older build's `validateCanonical`, via
      ProseMirror `Node.check()`, refuses unknown attributes), `tiptap-footnotes@3` makes footnote `id`/`data-id` required (defaults restored to
      null: stored footnotes without them must still open). insertContent suites green unchanged.
- [ ] T7 Vite optimizeDeps + AGENTS.md; full lint/typecheck (Pro and Lite)/test/format/audit.
- [ ] T8 Visual check by the user in `tauri dev` with `ENTROPIA_DEV_PROFILE`.

## Acceptance

- `pnpm audit` (all deps): 0 advisories.
- Golden tests from T1 unchanged; full suite green; Lite and Pro typecheck green.
- User confirms both editors behave as before.

## Route

T1-T7 delegated to one writer (2+ non-trivial files; evidence mapped by an explorer).
T8 is the user's (Tauri native window).

## Progress

- 2026-10-21: impact map done (explorer). Feature doc created.
- 2026-10-21: T2-T4, T6 on Tiptap 3.32.0 (commit 0e2bb552): ui test 876/876, ui typecheck 0/0, desktop test
  3741 passed / 9 skipped, desktop typecheck 0/0, lint clean, format:check clean.
