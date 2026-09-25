import { EDITOR_MIN_WIDTH, OUTLINE_BOUNDS, RESEARCH_BOUNDS } from '@entropia/ui'

/**
 * How narrow the Writing pane may get before its three-column shell — the
 * outline, the manuscript and the research panel — stops fitting, and what to
 * do about it.
 *
 * # Why this is computed rather than a `@container` breakpoint
 *
 * A `@container pane (max-width: …)` rule can hide the outline, but it cannot
 * also disable its toggle, explain why with a tooltip, or leave the user's
 * saved preference untouched for when the pane widens again — that needs a
 * decision in script, not a rule in a stylesheet. So the geometry itself is
 * worked out here, once, from the same numbers the stylesheet uses: the
 * constants below mirror `.writing__workspace`'s CSS (`gap: var(--space-3)`,
 * `.writing`'s `padding: var(--space-5)`) and `ResizeHandle`'s fixed width —
 * restated as numbers because a container query cannot read a CSS custom
 * property either.
 *
 * # Why the research panel is part of the same decision
 *
 * Forcing the outline shut is not enough on its own: at a pane narrow enough,
 * even the manuscript and the research panel together do not fit at their
 * usual floors. The manuscript is what the screen is for and never yields
 * past its own 320px (`EDITOR_MIN_WIDTH`) — so what is left has to come out
 * of the research panel's floor instead. `writingLayout` works out both
 * numbers together, from what is actually rendered, not from the split ratio
 * or the window's own width.
 */

/** `.writing__workspace`'s `gap: var(--space-3)` — between every child. */
export const WORKSPACE_GAP = 12

/** A `ResizeHandle`'s fixed `flex: 0 0 9px`, rendered beside each open panel. */
export const HANDLE_WIDTH = 9

/** `.writing`'s `padding: var(--space-5)` (20px), on both sides. */
export const WRITING_INSET = 40

/**
 * The narrowest `.writing__workspace` may be and still show every rendered
 * column at its own floor — the outline and the research panel at their
 * `squeeze` bound, the editor at `EDITOR_MIN_WIDTH` — with a `ResizeHandle`
 * and a `var(--space-3)` gap beside each open panel, and `.writing`'s own
 * padding on both sides.
 */
export function writingRowMinWidth(outlineOpen: boolean, researchOpen: boolean): number {
  const columns = [EDITOR_MIN_WIDTH]
  let handles = 0
  if (outlineOpen) {
    columns.push(OUTLINE_BOUNDS.squeeze)
    handles += 1
  }
  if (researchOpen) {
    columns.push(RESEARCH_BOUNDS.squeeze)
    handles += 1
  }
  const children = columns.length + handles
  const gaps = Math.max(0, children - 1) * WORKSPACE_GAP
  const sum = columns.reduce((total, width) => total + width, 0)
  return WRITING_INSET + sum + handles * HANDLE_WIDTH + gaps
}

export interface WritingLayout {
  /**
   * Whether the pane is too narrow to fit the outline at its own floor
   * alongside the editor and whatever the research panel is currently
   * showing. Drives both the forced collapse and the toggle's `disabled`.
   */
  forceOutlineCollapse: boolean
  /** What the outline should actually render as: the user's own preference,
   *  unless the pane forces it shut. Never written back to the preference. */
  effectiveOutlineOpen: boolean
  /**
   * The floor the research panel may be squeezed to. Its usual
   * `RESEARCH_BOUNDS.squeeze`, unless even the editor and the research panel
   * together (with the outline already resolved above) would not fit — in
   * which case it is exactly what is left, so the row's total never exceeds
   * `paneWidth` and nothing is left for `overflow-x: hidden` to clip.
   */
  researchMinWidth: number
}

/**
 * Decides the Writing pane's layout for one measured width.
 *
 * @param paneWidth The Writing view's own rendered width (its root element's
 *   `clientWidth`, via `watchPaneWidth` below) — never the split ratio and
 *   never the window's width, both of which a split pane routinely disagrees
 *   with.
 * @param outlineOpen The user's saved preference, exactly as read from
 *   `localStorage` — untouched by this function either way.
 * @param researchOpen Whether the research panel is currently open.
 */
export function writingLayout(
  paneWidth: number,
  outlineOpen: boolean,
  researchOpen: boolean
): WritingLayout {
  const forceOutlineCollapse = paneWidth < writingRowMinWidth(true, researchOpen)
  const effectiveOutlineOpen = outlineOpen && !forceOutlineCollapse

  let researchMinWidth = RESEARCH_BOUNDS.squeeze
  if (researchOpen) {
    const withoutResearch = writingRowMinWidth(effectiveOutlineOpen, false)
    const available = paneWidth - withoutResearch - HANDLE_WIDTH - WORKSPACE_GAP
    researchMinWidth = Math.max(0, Math.min(RESEARCH_BOUNDS.squeeze, Math.round(available)))
  }

  return { forceOutlineCollapse, effectiveOutlineOpen, researchMinWidth }
}

/**
 * Watches `element`'s content width and reports it on every change.
 *
 * A safe no-op — never calls back, returns a no-op cleanup — when
 * `ResizeObserver` is unavailable: the caller's own initial width (see
 * `writingLayout`'s default in WritingView.svelte) stands instead.
 */
export function watchPaneWidth(element: Element, onChange: (width: number) => void): () => void {
  if (typeof ResizeObserver === 'undefined') return () => {}

  const observer = new ResizeObserver((entries) => {
    const width = entries[0]?.contentRect.width
    if (width === undefined) return
    onChange(width)
  })
  observer.observe(element)
  return () => observer.disconnect()
}
