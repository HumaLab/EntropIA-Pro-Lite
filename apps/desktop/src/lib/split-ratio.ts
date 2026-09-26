/**
 * Each split pane keeps a minimum width of 640px: Writing's editor plus a
 * research panel still usable for Zotero and Corpus don't fit in less (user
 * rule, 2026-09-26; was 480px on 2026-09-25 and 320px in the spec). Two
 * panes plus the divider need 1286px, so a half-screen snapped window on a
 * common display shows one pane, never two squeezed ones. Below that there is no split view at all — see `fitsSideBySide`
 * below (vertical stacking was removed; a superseded user rule, also
 * 2026-09-25, used to stack panes here instead).
 */
export const MIN_PANE_PX = 640

/**
 * Each pane keeps at least 40% of the split — the ratio is clamped to
 * `[0.4, 0.6]` — in addition to the 640px-per-pane floor below. The
 * stricter of the two always wins (see `clampSplitRatio`). This is also the
 * coarse fallback used when the container size is not yet known.
 */
const COARSE_MIN_RATIO = 0.4
const COARSE_MAX_RATIO = 0.6

/**
 * Clamp a candidate ratio so neither pane drops below `minPaneSize` pixels
 * in a container of `containerSize` pixels, AND neither pane drops below 40%
 * of the split (`[0.4, 0.6]`) — whichever bound is stricter wins. Falls
 * back to the coarse `[0.4, 0.6]` bound alone when the container size is
 * not yet known (0 or non-finite) — the same bound
 * `WorkspaceStore.setSplitRatio` applies on its own, before any DOM
 * measurement exists.
 */
export function clampSplitRatio(
  ratio: number,
  containerSize: number,
  minPaneSize: number = MIN_PANE_PX
): number {
  if (!Number.isFinite(containerSize) || containerSize <= 0) {
    return Math.min(COARSE_MAX_RATIO, Math.max(COARSE_MIN_RATIO, ratio))
  }
  const pixelMinFraction = Math.min(0.5, minPaneSize / containerSize)
  // The stricter of the two floors/ceilings: the larger minimum, the
  // smaller maximum.
  const minFraction = Math.max(pixelMinFraction, COARSE_MIN_RATIO)
  const maxFraction = Math.min(1 - pixelMinFraction, COARSE_MAX_RATIO)
  return Math.min(maxFraction, Math.max(minFraction, ratio))
}

/**
 * `SplitDivider`'s own fixed width (its CSS: `flex: 0 0 6px`) — not a
 * heuristic, the actual pixels it always occupies between the two panes.
 * Kept here as the single source of truth for the side-by-side threshold
 * below; `SplitDivider.svelte`'s stylesheet must stay in sync with it.
 */
export const SPLIT_DIVIDER_PX = 6

/**
 * Whether the split area can give BOTH panes at least `minPaneSize` side by
 * side, with the real divider width between them (user rule, 2026-09-25:
 * below this there is no split view at all — no vertical-stacking fallback).
 * Below the threshold, callers collapse to the active pane alone
 * (`AppShell.svelte`) and disable the split toggle when split is off
 * (`TopBar.svelte`).
 */
export function fitsSideBySide(
  containerSize: number,
  dividerSize: number = SPLIT_DIVIDER_PX,
  minPaneSize: number = MIN_PANE_PX
): boolean {
  return containerSize >= 2 * minPaneSize + dividerSize
}
