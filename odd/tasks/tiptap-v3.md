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
      Checks on Tiptap 2: `pnpm --filter @entropia/ui test` 874/874, ui typecheck 0/0.
- [ ] T2 Dependency bump: catalog to ^3.32.0, add `@tiptap/extensions`, tiptap-footnotes ^3.0.1,
      single prosemirror copy.
- [ ] T3 Import fixes: named `Table*` and `{ TextStyle }` imports.
- [ ] T4 StarterKit de-dup in both editors (link, underline, trailingNode).
- [ ] T5 `setContent(x, false)` -> `{ emitUpdate: false }` (4 sites) + test that external loads
      do not fire updates.
- [ ] T6 TextStyle `mergeNestedSpanStyles` vs `WritingTextStyle.parseHTML`; insertContent
      regression (dictation, suggestions, search-replace, citations).
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
