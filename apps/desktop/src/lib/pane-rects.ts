/** A live registry of each mounted pane's bounding rect, so a webview-wide
 *  Tauri drop event (`onDragDropEvent`) can be attributed to the one pane
 *  the pointer was actually over (spec, Hazards). */
export interface PaneRect {
  left: number
  top: number
  right: number
  bottom: number
}

const measurers = new Map<string, () => PaneRect>()

export function registerPaneRect(paneId: string, measure: () => PaneRect): void {
  measurers.set(paneId, measure)
}

export function unregisterPaneRect(paneId: string): void {
  measurers.delete(paneId)
}

/** Re-measures every registered pane on each call — panes resize (split
 *  drag, window resize) far more often than drops happen, so nothing here
 *  caches a rect across calls. */
export function currentPaneRects(): { paneId: string; rect: PaneRect }[] {
  return [...measurers.entries()].map(([paneId, measure]) => ({ paneId, rect: measure() }))
}
