/**
 * Floating help bubble (S13, D9): position helpers.
 *
 * The bubble lives in one place for the whole app and remembers where the
 * user left it. Position is persisted per device in localStorage (D9), so it
 * is naturally per-device: nothing syncs it. Every helper here is pure and
 * takes its inputs as arguments, so the component stays thin and these are
 * unit-testable without a window.
 */

export interface BubbleViewport {
  width: number
  height: number
}

export interface BubblePosition {
  x: number
  y: number
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

type BubbleStorage = Pick<Storage, 'getItem' | 'setItem'> | null | undefined

function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback
}

/**
 * Bottom-right by default (S13), with room for the status bar below it.
 * Degenerate viewports still answer an inside-the-window position.
 */
export function defaultBubblePosition(viewport: BubbleViewport): BubblePosition {
  const width = finiteOr(viewport.width, 0)
  const height = finiteOr(viewport.height, 0)
  const x = width - HELP_BUBBLE_SIZE_PX - HELP_BUBBLE_DEFAULT_RIGHT_PX
  const y = height - HELP_BUBBLE_SIZE_PX - HELP_BUBBLE_DEFAULT_BOTTOM_PX
  return clampBubblePosition({ x, y }, viewport)
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

/** True once the pointer moved far enough that the press is a drag, not a click. */
export function exceedsDragThreshold(from: BubblePosition, to: BubblePosition): boolean {
  const dx = to.x - from.x
  const dy = to.y - from.y
  return Math.hypot(dx, dy) > HELP_BUBBLE_DRAG_THRESHOLD_PX
}

function isValidPosition(value: unknown): value is BubblePosition {
  if (typeof value !== 'object' || value === null) return false
  const { x, y } = value as Record<string, unknown>
  return typeof x === 'number' && Number.isFinite(x) && typeof y === 'number' && Number.isFinite(y)
}

/** The stored position, or null when nothing valid was stored (corrupt storage falls back to default). */
export function loadBubblePosition(storage: BubbleStorage): BubblePosition | null {
  if (!storage) return null
  let raw: string | null
  try {
    raw = storage.getItem(HELP_BUBBLE_STORAGE_KEY)
  } catch {
    return null
  }
  if (raw === null) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  return isValidPosition(parsed) ? { x: parsed.x, y: parsed.y } : null
}

/** Persists the position; never throws (a full or denied storage must not break the bubble). */
export function saveBubblePosition(storage: BubbleStorage, position: BubblePosition): void {
  if (!storage || !isValidPosition(position)) return
  try {
    storage.setItem(HELP_BUBBLE_STORAGE_KEY, JSON.stringify({ x: position.x, y: position.y }))
  } catch {
    // Best effort: the bubble keeps working for the session without persistence.
  }
}
