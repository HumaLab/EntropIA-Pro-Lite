import type { PaneRect } from './pane-rects'

/**
 * Which pane a Tauri drop position lands in, falling back to the active
 * pane when the position is outside every known pane rect — e.g. dropped
 * over the tab strip or the divider (spec, Hazards: "handled only by the
 * pane whose rectangle contains the drop position, falling back to the
 * active pane").
 *
 * `paneRects` is measured with `getBoundingClientRect()` (CSS/logical
 * pixels), while Tauri's own `onDragDropEvent` reports its drop `position`
 * in *physical* pixels (`@tauri-apps/api/webview`'s `DragDropEvent.position`
 * is a `PhysicalPosition`). `devicePixelRatio` converts the incoming
 * `position` down to logical pixels before comparing — the same conversion
 * `writing-image-drop.ts`'s `handleWritingImageDrop` already applies to the
 * in-editor drop target. It defaults to `1` (a no-op) for callers that
 * already pass a logical-pixel position.
 */
export function resolveDropPaneId(
  position: { x: number; y: number },
  paneRects: readonly { paneId: string; rect: PaneRect }[],
  activePaneId: string,
  devicePixelRatio = 1
): string {
  const x = position.x / devicePixelRatio
  const y = position.y / devicePixelRatio
  const hit = paneRects.find(
    ({ rect }) => x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
  )
  return hit?.paneId ?? activePaneId
}
