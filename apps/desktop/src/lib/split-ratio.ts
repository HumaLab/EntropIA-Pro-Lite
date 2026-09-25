/** Each split pane keeps a minimum width of 320px (spec, Split view). */
export const MIN_PANE_PX = 320

/**
 * Each pane keeps at least 40% of the split — the ratio is clamped to
 * `[0.4, 0.6]` — in addition to the 320px-per-pane floor below. The
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

/** Whether the content area is too narrow to fit two `minPaneSize` panes
 *  side by side (spec, Responsive: "panes stack vertically"). */
export function shouldStack(containerSize: number, minPaneSize: number = MIN_PANE_PX): boolean {
  return containerSize < 2 * minPaneSize
}
