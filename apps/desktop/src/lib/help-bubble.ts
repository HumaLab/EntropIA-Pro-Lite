/**
 * Floating help bubble (S13, T16): position helpers.
 *
 * The bubble lives in one place for the whole app. It starts at the default
 * bottom-right offsets on every launch (T16, superseding the persisted
 * position of D9): nothing about the position is written to localStorage, a
 * drag lasts for the session only, and any stale stored value is removed
 * once on mount. The panel's maximized state is still persisted per device
 * (T14). Every helper here is pure and takes its inputs as arguments, so the
 * component stays thin and these are unit-testable without a window.
 *
 * Anchoring (T14): the offsets are the bubble's distance to the window's
 * RIGHT and BOTTOM edges, not an absolute left/top. The bubble is drawn at
 * `innerWidth - right - size`, `innerHeight - bottom - size`, so by default
 * it follows the bottom-right corner across resizes and maximize/restore,
 * and a drag keeps its distance to that corner for the session.
 */

export interface BubbleViewport {
  width: number
  height: number
}

export interface BubblePosition {
  x: number
  y: number
}

/** Distance from the window's right/bottom edges, in px (T14). */
export interface BubbleOffsets {
  right: number
  bottom: number
}

/** The bubble button's own diameter, in px (matches the stylesheet). */
export const HELP_BUBBLE_SIZE_PX = 56
/** Minimum air between the bubble and any window edge when clamping. */
export const HELP_BUBBLE_EDGE_MARGIN_PX = 8
/** Default bottom-right air: 24px from the right edge. */
export const HELP_BUBBLE_DEFAULT_RIGHT_PX = 24
/** Default bottom clearance: the 30px status bar plus 16px of air. */
export const HELP_BUBBLE_DEFAULT_BOTTOM_PX = 46
/** A press that moves more than this is a drag; at most this is a click. */
export const HELP_BUBBLE_DRAG_THRESHOLD_PX = 4
/** Per-device storage for the bubble position (D9). */
export const HELP_BUBBLE_STORAGE_KEY = 'entropia.help-bubble.position'
/** Per-device storage for the panel's maximized state (T14). */
export const HELP_BUBBLE_MAXIMIZED_KEY = 'entropia.help-bubble.maximized'

type BubbleStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null | undefined

function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback
}

/** Bottom-right by default (S13), as offsets from that corner. */
export function defaultBubbleOffsets(): BubbleOffsets {
  return { right: HELP_BUBBLE_DEFAULT_RIGHT_PX, bottom: HELP_BUBBLE_DEFAULT_BOTTOM_PX }
}

/** Keeps the whole bubble inside the window, with edge air (re-clamped on resize). */
export function clampBubblePosition(
  position: BubblePosition,
  viewport: BubbleViewport
): BubblePosition {
  const width = finiteOr(viewport.width, 0)
  const height = finiteOr(viewport.height, 0)
  const margin = HELP_BUBBLE_EDGE_MARGIN_PX
  const maxX = Math.max(margin, width - HELP_BUBBLE_SIZE_PX - margin)
  const maxY = Math.max(margin, height - HELP_BUBBLE_SIZE_PX - margin)
  return {
    x: Math.min(Math.max(finiteOr(position.x, margin), margin), maxX),
    y: Math.min(Math.max(finiteOr(position.y, margin), margin), maxY),
  }
}

/** Draws the bubble at its offsets from the right/bottom edges, clamped into the window. */
export function offsetsToPosition(
  offsets: BubbleOffsets,
  viewport: BubbleViewport
): BubblePosition {
  const width = finiteOr(viewport.width, 0)
  const height = finiteOr(viewport.height, 0)
  return clampBubblePosition(
    {
      x: width - HELP_BUBBLE_SIZE_PX - finiteOr(offsets.right, HELP_BUBBLE_DEFAULT_RIGHT_PX),
      y: height - HELP_BUBBLE_SIZE_PX - finiteOr(offsets.bottom, HELP_BUBBLE_DEFAULT_BOTTOM_PX),
    },
    viewport
  )
}

/** Reads the offsets back from an absolute position in the current window. */
export function positionToOffsets(
  position: BubblePosition,
  viewport: BubbleViewport
): BubbleOffsets {
  const width = finiteOr(viewport.width, 0)
  const height = finiteOr(viewport.height, 0)
  return {
    right: width - HELP_BUBBLE_SIZE_PX - finiteOr(position.x, HELP_BUBBLE_EDGE_MARGIN_PX),
    bottom: height - HELP_BUBBLE_SIZE_PX - finiteOr(position.y, HELP_BUBBLE_EDGE_MARGIN_PX),
  }
}

/** Keeps the offsets drawing the bubble inside the window (re-clamped on resize). */
export function clampBubbleOffsets(
  offsets: BubbleOffsets,
  viewport: BubbleViewport
): BubbleOffsets {
  const raw: BubblePosition = {
    x: finiteOr(viewport.width, 0) - HELP_BUBBLE_SIZE_PX - finiteOr(offsets.right, 0),
    y: finiteOr(viewport.height, 0) - HELP_BUBBLE_SIZE_PX - finiteOr(offsets.bottom, 0),
  }
  return positionToOffsets(clampBubblePosition(raw, viewport), viewport)
}

/** True once the pointer moved far enough that the press is a drag, not a click. */
export function exceedsDragThreshold(from: BubblePosition, to: BubblePosition): boolean {
  const dx = to.x - from.x
  const dy = to.y - from.y
  return Math.hypot(dx, dy) > HELP_BUBBLE_DRAG_THRESHOLD_PX
}

/**
 * Removes any stale stored bubble position once (T16). Both the legacy
 * absolute `{x, y}` shape and the T14 `{right, bottom}` shape lived under
 * HELP_BUBBLE_STORAGE_KEY, so one removal clears either. The bubble always
 * starts at the default bottom-right offsets; a drag lasts for the session
 * only. Never throws. The panel's maximized key is left alone (the owner did
 * not ask to change it).
 */
export function clearStoredBubblePosition(storage: BubbleStorage): void {
  if (!storage) return
  try {
    storage.removeItem(HELP_BUBBLE_STORAGE_KEY)
  } catch {
    // Best effort: without storage access there is simply nothing to clear.
  }
}

/** The stored maximized state, or false (normal size) when nothing valid was stored. */
export function loadBubbleMaximized(storage: BubbleStorage): boolean {
  if (!storage) return false
  try {
    return storage.getItem(HELP_BUBBLE_MAXIMIZED_KEY) === 'true'
  } catch {
    return false
  }
}

/** Persists the maximized state; never throws. */
export function saveBubbleMaximized(storage: BubbleStorage, maximized: boolean): void {
  if (!storage) return
  try {
    storage.setItem(HELP_BUBBLE_MAXIMIZED_KEY, maximized ? 'true' : 'false')
  } catch {
    // Best effort: the panel keeps working for the session without persistence.
  }
}
