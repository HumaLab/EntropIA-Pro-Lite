# Tabs and split view

## Objective

Browser-style tabs (up to 4) in the top bar and a Chrome-style split view that
shows two tabs side by side, each with its own navigation history. Design
approved by the user on 2026-09-24.

## Problem

The app has one global navigation history (`lib/navigation.ts` singleton), so
the work area can show only one view at a time.

## Scope

- Tabs between the product title and the global search, right-aligned; max 4;
  new tab opens Inicio; last tab cannot close.
- Back, breadcrumb, sibling arrows and asset delete move to a strip above each
  pane.
- Split toggle (`split` icon, Tabler `layout-columns`) pairs the active tab
  with a new Inicio tab; draggable divider, 320px minimum per pane, double
  click resets 50/50; toggling off ungroups without closing anything.
- Writing lives in one tab at a time; deletions prune every tab.
- Everything responsive (900px breakpoint; panes stack when two 320px panes do
  not fit).
- Out of scope: persistence across restarts, keyboard shortcuts, tab reorder.

## References

- Spec: `docs/superpowers/specs/2026-09-24-tabs-split-view-design.md` (local, docs/ is gitignored)
- Plan: `docs/superpowers/plans/2026-09-24-tabs-split-view.md` (local)

## Execution

- Route: delegated direct, subagent-driven (one implementer + one reviewer per
  task). Trigger evidence: 15 modules import the navigation singleton (4-file
  and writer triggers).
- TDD: strict (session configuration), runner Vitest
  (`pnpm --filter @entropia-pro/desktop test -- <file>`).
- RDD: off (clone_local), so no native review; per-task review is the check.
- Commits on `main` (user preference).

## Tasks

### Stage 1: foundation (no visible change)

- [x] 1.1 `WorkspaceStore` core: tab lifecycle (bf1b224c, review clean)
- [x] 1.2 `pane-context.ts`: Svelte context for pane navigation (4513f630, 05471b22; one fix round: init-only contract)
- [x] 1.3 Cross-tab pruning and the Writing single-tab rule (65a38b31, 54f610e6; one fix round: inactive-tab prune tests)
- [x] 1.4 Wire `App.svelte` to the workspace (545c8483; App.test.ts migrated, $derived freeze avoided)
- [x] 1.5 Migrate the ten pane-scoped views to `getNavigation()` (c88095eb, 7b101e5e; deletions and Writing entries via workspace, Escape-back on the active tab)
- [x] 1.6 Migrate chrome consumers, retire the `navigation` singleton (2e1c2c4e; guard test forbids the singleton)

### Stage 2: location strip and tabs

- [x] 2.1 Tab titles/icons and `TabStrip.svelte` (8a713ba3, 7ceb7d0b; aria-label per tab, no in-component flushSync)
- [x] 2.2 `WorkPane.svelte`: location strip and routed view (b96d2d3a, d1b023fd, 9f85fc00; pane isolation proven by mutation check)
- [x] 2.3 `AppShell` renders `WorkPane` (d28de0b3; keyed per tab, chrome follows the active tab)
- [x] 2.4 `TopBar` loses the location strip, hosts `TabStrip` (1b8bb98d, 680def6c; explorer follows the active tab, moved coverage ported to WorkPane)
- [x] 2.5 Pane-suffixed ids in `WritingView` (7b369a61, 506deee6; extended to every view-owned id per the spec)
- [ ] Visual check by the user (wide and narrow window, Lite)

### Stage 3: split view

- [ ] 3.1 Workspace split state
- [ ] 3.2 `SplitDivider.svelte`
- [ ] 3.3 Split rendering in `AppShell` and toggle in `TopBar`
- [ ] 3.4 Responsive stacking
- [ ] 3.5 Drag-drop pane targeting
- [ ] Visual check by the user (wide and narrow window, Lite)

## Progress

Stage 1 complete (0646cdb2..2e1c2c4e), every task reviewed. Plan gaps
resolved during execution: deletion sites and Writing entries route through
the workspace; `lib/keyboard.ts` Escape-back acts on the active tab;
`lib/document-explorer.ts` migrated.

Stage 2 complete (abbf279d..506deee6), every task reviewed; layout suites
166/166 and `pnpm format:check` clean on the final commit. Next: the user's
visual check of tabs and the location strip (wide and narrow window, Lite),
then Stage 3.
