/**
 * Geometry of a layout box being moved or resized over the viewer, in layout
 * reference coordinates (the space `ViewerLayoutRegion` boxes live in).
 */
export interface LayoutEditBox {
  x: number
  y: number
  width: number
  height: number
}

export type LayoutEditHandle = 'move' | 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw'

export const LAYOUT_RESIZE_HANDLES: readonly Exclude<LayoutEditHandle, 'move'>[] = [
  'nw',
  'n',
  'ne',
  'e',
  'se',
  's',
  'sw',
  'w',
]

/** Smallest side a box can be resized to, in layout units. */
export const MIN_LAYOUT_EDIT_SIDE = 8

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}

/**
 * Applies a drag of (`dx`, `dy`) on `handle` to `start`, keeping the box inside
 * `[0, maxW] × [0, maxH]` and never thinner than {@link MIN_LAYOUT_EDIT_SIDE}.
 * A non-positive bound leaves that axis unclamped.
 */
export function dragLayoutBox(
  start: LayoutEditBox,
  handle: LayoutEditHandle,
  dx: number,
  dy: number,
  maxW: number,
  maxH: number
): LayoutEditBox {
  const limitW = maxW > 0 ? maxW : Number.POSITIVE_INFINITY
  const limitH = maxH > 0 ? maxH : Number.POSITIVE_INFINITY

  if (handle === 'move') {
    const x = clamp(start.x + dx, 0, Math.max(0, limitW - start.width))
    const y = clamp(start.y + dy, 0, Math.max(0, limitH - start.height))
    return { x, y, width: start.width, height: start.height }
  }

  let left = start.x
  let top = start.y
  let right = start.x + start.width
  let bottom = start.y + start.height

  if (handle.includes('w')) left = clamp(left + dx, 0, right - MIN_LAYOUT_EDIT_SIDE)
  if (handle.includes('e')) right = clamp(right + dx, left + MIN_LAYOUT_EDIT_SIDE, limitW)
  if (handle.includes('n')) top = clamp(top + dy, 0, bottom - MIN_LAYOUT_EDIT_SIDE)
  if (handle.includes('s')) bottom = clamp(bottom + dy, top + MIN_LAYOUT_EDIT_SIDE, limitH)

  return { x: left, y: top, width: right - left, height: bottom - top }
}

/** Rounds a box to whole layout pixels (the stored layout keeps integers). */
export function roundLayoutBox(box: LayoutEditBox): LayoutEditBox {
  const x = Math.round(box.x)
  const y = Math.round(box.y)
  return {
    x,
    y,
    width: Math.max(1, Math.round(box.x + box.width) - x),
    height: Math.max(1, Math.round(box.y + box.height) - y),
  }
}

/** Where a resize handle sits on `box`, as its center point. */
export function layoutHandleCenter(box: LayoutEditBox, handle: Exclude<LayoutEditHandle, 'move'>) {
  const x = handle.includes('w')
    ? box.x
    : handle.includes('e')
      ? box.x + box.width
      : box.x + box.width / 2
  const y = handle.includes('n')
    ? box.y
    : handle.includes('s')
      ? box.y + box.height
      : box.y + box.height / 2
  return { x, y }
}

export function layoutHandleCursor(handle: LayoutEditHandle) {
  switch (handle) {
    case 'n':
    case 's':
      return 'ns-resize'
    case 'e':
    case 'w':
      return 'ew-resize'
    case 'ne':
    case 'sw':
      return 'nesw-resize'
    case 'nw':
    case 'se':
      return 'nwse-resize'
    default:
      return 'move'
  }
}
