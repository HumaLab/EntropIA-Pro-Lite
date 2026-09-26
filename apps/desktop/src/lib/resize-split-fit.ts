import { fitsSideBySide } from './split-ratio'

/**
 * Watches `element`'s content width and calls `onChange(fits)` only when
 * crossing the side-by-side threshold (`fitsSideBySide`, split-ratio.ts;
 * user rule, 2026-09-25: below it there is no split view at all) — never on
 * every resize tick, so a divider drag that hovers around the boundary does
 * not spam re-layout.
 *
 * A no-op (never calls back, returns a no-op cleanup) when `ResizeObserver`
 * is unavailable in this environment — the caller's initial assumption
 * (both panes fit) stands.
 */
export function watchSplitFit(element: Element, onChange: (fits: boolean) => void): () => void {
  if (typeof ResizeObserver === 'undefined') return () => {}

  // Starts at `true` (fits, side-by-side) rather than `null`: `ResizeObserver`
  // fires an initial callback as soon as `observe()` runs, and that first
  // report should only reach `onChange` when it actually diverges from the
  // caller's assumed starting state — matching it exactly (the common case)
  // must not fire a redundant call.
  let last = true
  const observer = new ResizeObserver((entries) => {
    const width = entries[0]?.contentRect.width
    if (width === undefined) return
    const next = fitsSideBySide(width)
    if (next === last) return
    last = next
    onChange(next)
  })
  observer.observe(element)
  return () => observer.disconnect()
}
