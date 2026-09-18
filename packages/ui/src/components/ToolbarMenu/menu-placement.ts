/**
 * Where a menu opened from a trigger goes, in viewport coordinates.
 *
 * Below the trigger by default, lined up with one of its edges. It flips to the
 * other edge before it would leave the window, shifts inside the window when
 * neither edge fits, and opens above only when below has no room and above
 * does. Pure, so the geometry is tested without a layout engine.
 */

export interface Rect {
  top: number
  left: number
  width: number
  height: number
}

export interface Size {
  width: number
  height: number
}

export type MenuAlign = 'start' | 'end'

export interface MenuPlacementInput {
  anchor: Rect
  menu: Size
  viewport: Size
  align?: MenuAlign
  /** Space between the trigger and the menu. */
  offset?: number
  /** Space kept clear at the window's edges. */
  margin?: number
}

export interface MenuPlacement {
  top: number
  left: number
  /** The height the menu may take before it scrolls. */
  maxHeight: number
}

export function placeMenu({
  anchor,
  menu,
  viewport,
  align = 'start',
  offset = 4,
  margin = 8,
}: MenuPlacementInput): MenuPlacement {
  const startLeft = anchor.left
  const endLeft = anchor.left + anchor.width - menu.width
  const fitsAt = (left: number) => left >= margin && left + menu.width <= viewport.width - margin

  let left = align === 'start' ? startLeft : endLeft
  const other = align === 'start' ? endLeft : startLeft
  if (!fitsAt(left) && fitsAt(other)) left = other
  // Neither edge fits: shift inside the window, the left margin winning when
  // the menu is wider than the window itself.
  left = Math.max(margin, Math.min(left, viewport.width - margin - menu.width))

  const below = anchor.top + anchor.height + offset
  const roomBelow = viewport.height - below - margin
  const roomAbove = anchor.top - offset - margin
  if (menu.height > roomBelow && menu.height <= roomAbove) {
    return { top: anchor.top - offset - menu.height, left, maxHeight: roomAbove }
  }
  return { top: below, left, maxHeight: roomBelow }
}
