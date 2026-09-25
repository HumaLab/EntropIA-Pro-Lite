import { shouldStack } from './split-ratio'

/**
 * Watches `element`'s content width and calls `onChange(stacked)` only when
 * crossing the two-pane-fits threshold (spec, Responsive: "panes stack
 * vertically" below it) — never on every resize tick, so a divider drag
 * that hovers around the boundary does not spam re-layout.
 *
 * A no-op (never calls back, returns a no-op cleanup) when `ResizeObserver`
 * is unavailable in this environment — the caller's initial `stacked` state
 * (side-by-side) stands.
 */
export function watchStacking(element: Element, onChange: (stacked: boolean) => void): () => void {
  if (typeof ResizeObserver === 'undefined') return () => {}

  // Starts at `false` (side-by-side) rather than `null`: `ResizeObserver`
  // fires an initial callback as soon as `observe()` runs, and that first
  // report should only reach `onChange` when it actually diverges from the
  // caller's assumed starting state — matching it exactly (the common case)
  // must not fire a redundant call.
  let last = false
  const observer = new ResizeObserver((entries) => {
    const width = entries[0]?.contentRect.width
    if (width === undefined) return
    const next = shouldStack(width)
    if (next === last) return
    last = next
    onChange(next)
  })
  observer.observe(element)
  return () => observer.disconnect()
}
