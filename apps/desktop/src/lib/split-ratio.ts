/** Each split pane keeps a minimum width of 320px (spec, Split view). */
export const MIN_PANE_PX = 320

const COARSE_MIN_RATIO = 0.15
const COARSE_MAX_RATIO = 0.85

/**
 * Clamp a candidate ratio so neither pane drops below `minPaneSize` pixels
 * in a container of `containerSize` pixels. Falls back to the coarse
 * `[0.15, 0.85]` bound when the container size is not yet known (0 or
 * non-finite) — the same bound `WorkspaceStore.setSplitRatio` applies on its
 * own, before any DOM measurement exists.
 */
export function clampSplitRatio(
  ratio: number,
  containerSize: number,
  minPaneSize: number = MIN_PANE_PX
): number {
  if (!Number.isFinite(containerSize) || containerSize <= 0) {
    return Math.min(COARSE_MAX_RATIO, Math.max(COARSE_MIN_RATIO, ratio))
  }
  const minFraction = Math.min(0.5, minPaneSize / containerSize)
  return Math.min(1 - minFraction, Math.max(minFraction, ratio))
}

/** Whether the content area is too narrow to fit two `minPaneSize` panes
 *  side by side (spec, Responsive: "panes stack vertically"). */
export function shouldStack(containerSize: number, minPaneSize: number = MIN_PANE_PX): boolean {
  return containerSize < 2 * minPaneSize
}
