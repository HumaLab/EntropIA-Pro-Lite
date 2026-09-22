# Inserting images into the manuscript

## Objective

A writer inserts an image from the Escritura toolbar, sees it inside the
document margins, selects it, resizes it, optionally captions it, and finds it
intact after closing and reopening both the document and the application. The
image exports correctly to Markdown, HTML and DOCX.

## Problem

The Escritura editor has no image node at all. A manuscript can hold citations,
footnotes, tables, links and typography, but not a figure. The only images the
editor knows are the ones embedded inside citation quotes, which arrive through
a different path and are not authored by the writer.

An image must also stop depending on where it came from: picking
`D:\Fotos\grafico.png` cannot mean the manuscript breaks when that file is
moved, renamed or deleted.

## Why

Requested directly. The full requirement is in the spec; this document tracks
the work.

## Scope

- **Spec:** `docs/superpowers/specs/2026-09-21-writing-image-node-design.md`
- **Plan:** `docs/superpowers/plans/2026-09-21-writing-image-node.md`
- **Ledger:** `.superpowers/sdd/2026-09-21-writing-image-node/progress.md`

Not in scope: the investigation `assets` table, citation quote images, the
writing journal, document versioning, `WRITING_SCHEMA_VERSION`, and garbage
collection of stored images.

## Constraints

- Accepted formats: PNG, JPEG, GIF. WebP and SVG are refused before anything is
  stored, because `DOCX_IMAGE_TYPES` (`export-docx.ts:393-398`) cannot draw a
  WebP and `drawnImage` fails silently on it.
- `packages/ui` has zero `@tauri-apps/*` dependency and keeps it. Every Tauri
  call lives in `apps/desktop`; the editor receives capabilities as props.
- Stored paths are always relative to the shared data directory.
- No Tauri configuration changes; every needed permission is already granted.
- Conventional commits, no AI attribution.

## Execution

- **Route:** delegated direct, one bounded implementer per task, with a task
  review after each. Not SDD — no SDD artifacts are created.
- **Branch:** `main`, by explicit user consent recorded at setup. No feature
  branch, no worktree, no pull request.
- **Delivery:** direct-to-main work-unit commits. The chained-PR budget does not
  apply because no pull request is opened. Forecast is well above the 400-line
  advisory heuristic across the eight tasks; the tasks are the slices.
- **TDD:** strict, enabled by project configuration. RED observed before GREEN
  on every production step.
- **Runners:** `pnpm --filter @entropia/ui test -- <file>`,
  `pnpm --filter @entropia-pro/desktop test -- <file>`,
  `VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck`.

## Tasks

- [x] **WIN-1** Prove `crypto.subtle` in the real happy-dom environment; ship
      `sha256Hex`, or the declared fallback if the proof fails.
- [x] **WIN-2** `image-dimensions.ts`: extract the PNG/JPEG header decoders out
      of `export-images.ts`, add GIF, leave PNG/JPEG output byte-identical.
- [x] **WIN-3** `writing-images.ts`: content-addressed import to
      `writing-images/{sha256}.{ext}`, refusing anything that is not PNG, JPEG
      or GIF.
- [x] **WIN-4** The `writingImage` node, its keymaps, and joining `TRAPPING` in
      `trailing-paragraph.ts`.
- [x] **WIN-5** The node view — first `addNodeView` in this repository — with a
      pure, directly tested `clampWritingImageWidth`.
- [x] **WIN-6** Toolbar button, `insert-image` icon, and the file picker wired
      from `WritingView.svelte`.
- [x] **WIN-7** Paste and drag-and-drop through the same import path.
- [x] **WIN-8** Exports: collector, three switch cases, `NODE_FIDELITY`, and the
      pattern document.

## Acceptance

The feature is done when an image can be inserted, seen, selected, resized,
saved, recovered after restarting the application, and exported correctly —
not when it first appears on screen.

Verification contracts 1 through 18 are listed in the spec. The editor is a
native Tauri window, so every visual and interaction check is confirmed by the
user and cannot be asserted by an agent.

## Progress

Eight tasks implemented and reviewed, then verified by hand in the running
application. 22 commits on `main` (`1374cb8..dc86c3a`). Suites green in both
packages: `@entropia/ui` 771 tests, `@entropia-pro/desktop` 1889 with 7
pre-existing skips.

Manual verification found four defects the entire test suite was blind to, all
in the node view's interactive surface, and a fifth introduced by their fix:

1. Clicking the image did not select it — `selectClickedLeaf` requires
   `node.isAtom`, and this node has content, so ProseMirror's default click
   path never produced a NodeSelection for it.
2. Resizing stalled under a native HTML5 drag, because `nodeDOM` (the figure)
   is armed `draggable` and the two gestures fought.
3. The resize handle sat at the column's corner, not the image's.
4. The `title` attribute was impersonating the caption — a tooltip the writer
   could type into but never see — while the real caption, the figcaption that
   is the node's own content, had no CSS and no way to be discovered.
5. Then the caret could not reach that caption either: `MouseDown` arms
   `figure.draggable` on almost any mousedown inside the node's range, and only
   the resize gesture was exempt from the resulting drag.

All five fixed and confirmed working by the user (`846c0a5`, `a3bcfd6`). The
align buttons were then swapped for the toolbar's own icons via Svelte 5's
imperative `mount`/`unmount`, keeping the node view plain DOM (`dc86c3a`).

**Status: confirmed working in the running application.**

Still unverified by a human, and worth checking when convenient:

1. Delete a file from `{dataDir}/writing-images/` and reopen the manuscript —
   a placeholder must draw and the node must survive.
2. Resize an image small, export to DOCX and HTML, and confirm the chosen size
   and alignment were honoured in both.
3. Close EntropIA entirely, reopen, and confirm size, alignment, caption and
   alt text all persist.
4. Paste an image from the clipboard, and drag one in from the desktop.
