/**
 * How wide a side panel may be (plan-editor.md §18).
 *
 * # Why this is a module and not four lines inside a component
 *
 * Because the rules are the interesting part and they are easy to get subtly
 * wrong: which direction makes a panel wider depends on which side of the
 * editor it sits on, and a panel that can be dragged to nothing looks to the
 * writer like a panel that has been lost. Here they can be stated once and
 * tested, and the component is left with the part that genuinely needs a DOM.
 *
 * # Why there is a floor and why it is not zero
 *
 * §18 asks for panels that are both resizable *and* foldable. Folding already
 * has a button, and it is reversible from that same button. A drag to zero
 * width is neither: the panel is gone, nothing marks where it was, and the
 * control that would bring it back is the one that just disappeared. So the
 * floor is a width the panel is still usable at, and disappearing stays the
 * job of the toggle that knows how to undo itself.
 */

export interface PanelBounds {
  /** Narrowest the panel may be **dragged**. Never zero — see above. */
  min: number
  max: number
  /** Where it starts, and what "reset" returns to. */
  initial: number
  /**
   * Narrowest the panel may be **squeezed to by the window**, which is smaller.
   *
   * Two floors, because they exist for two different reasons. `min` protects
   * against a gesture: you dragged the panel and should not be able to lose it
   * that way. This one is not a gesture — the window got small and something
   * has to give — and there the alternative is not a narrower panel but a
   * horizontal overflow, which is worse and which §18 forbids outright.
   *
   * Folding stays available throughout, so nothing here is a trap: a panel
   * squeezed to its uncomfortable width is one button away from being gone and
   * one more from coming back at the width its owner chose.
   */
  squeeze: number
}

/**
 * Which side of the editor a panel sits on.
 *
 * `start` is left of it, so dragging its handle to the right widens it;
 * `end` is right of it, so the same gesture narrows it. Getting this backwards
 * produces a panel that fights the pointer, which is the kind of bug that
 * survives review because everyone assumes they are holding it wrong.
 */
export type PanelSide = 'start' | 'end'

/** The outline: a list of headings, so it needs less room than the sources. */
export const OUTLINE_BOUNDS: PanelBounds = { min: 160, max: 480, initial: 240, squeeze: 120 }

/**
 * The research panel holds four tabs of results, so it starts wider. Its
 * squeeze floor is 240px because a Zotero or Corpus list narrower than that
 * no longer shows a readable title (user rule, 2026-09-26).
 */
export const RESEARCH_BOUNDS: PanelBounds = { min: 260, max: 560, initial: 280, squeeze: 240 }

/**
 * The narrowest the manuscript column is allowed to become.
 *
 * The panels yield to this, not the other way round. Without it the editor —
 * which has to be free to take the leftover room, so it carries `min-width: 0` —
 * is the *first* thing flexbox squeezes, and the panels never reach the floor
 * that would have made them give way. Dragging the outline wide then wraps the
 * manuscript to one word per line while a list of headings keeps two thirds of
 * the window, which inverts what the screen is for.
 *
 * Roughly thirty characters at the editor's measure: cramped, but still prose
 * someone can read a sentence of. Below that it stops being a narrow column and
 * becomes a vertical strip of words.
 *
 * Unlike the panels this has one floor and not two. The manuscript is what the
 * screen is for, so it is the thing everything else yields to — there is no
 * second, smaller number it may be pushed past when the window gets tight.
 */
export const EDITOR_MIN_WIDTH = 320

/**
 * How far one arrow key moves a panel.
 *
 * §18's first requirement is the keyboard, and a resizer that only answers to a
 * pointer fails it outright. Sixteen pixels is large enough to be worth pressing
 * and small enough to land where someone meant.
 */
export const KEYBOARD_STEP = 16

/** A bigger step, for crossing the range without holding a key down. */
export const KEYBOARD_PAGE = 64

export function clampPanel(width: number, bounds: PanelBounds): number {
  // A width that is not a number at all — a corrupt stored setting, a division
  // gone wrong — lands on the initial rather than on NaN, which would collapse
  // the panel and leave no way to see that anything had happened.
  if (!Number.isFinite(width)) return bounds.initial
  return Math.round(Math.min(bounds.max, Math.max(bounds.min, width)))
}

/**
 * The width after dragging the handle by `delta` pixels along the x axis.
 *
 * `delta` is the pointer's movement, not the panel's: the side decides which
 * of those two is the negative of the other.
 */
export function widthAfterDrag(
  startWidth: number,
  delta: number,
  side: PanelSide,
  bounds: PanelBounds
): number {
  return clampPanel(startWidth + (side === 'start' ? delta : -delta), bounds)
}

/**
 * The width after a key press, or `null` when the key is not one of ours.
 *
 * `null` rather than the unchanged width, so the caller can tell "this key does
 * nothing here" from "this key hit the end of the range" — and let the first
 * one through to whatever else was listening.
 */
export function widthAfterKey(
  key: string,
  current: number,
  side: PanelSide,
  bounds: PanelBounds
): number | null {
  // Toward the editor is narrower, away from it is wider, whichever side the
  // panel is on — so the arrow that widens a left panel is the one that widens
  // a right panel too, in the direction the writer sees.
  const outward = side === 'start' ? 1 : -1

  switch (key) {
    case 'ArrowRight':
      return clampPanel(current + KEYBOARD_STEP * outward, bounds)
    case 'ArrowLeft':
      return clampPanel(current - KEYBOARD_STEP * outward, bounds)
    case 'PageUp':
      return clampPanel(current + KEYBOARD_PAGE * outward, bounds)
    case 'PageDown':
      return clampPanel(current - KEYBOARD_PAGE * outward, bounds)
    case 'Home':
      return bounds.min
    case 'End':
      return bounds.max
    // A resizer with no way back to where it started makes experimenting with
    // it a one-way decision.
    case 'Enter':
    case ' ':
      return bounds.initial
    default:
      return null
  }
}

/**
 * Reads a stored width back, falling back to the initial one.
 *
 * Settings are text and can be anything by the time they come back — absent,
 * empty, edited by hand, written by an older build with different bounds. None
 * of those should produce a panel nobody can see.
 */
export function readPanelWidth(stored: string | null, bounds: PanelBounds): number {
  if (stored === null || stored.trim() === '') return bounds.initial
  const parsed = Number(stored)
  return Number.isFinite(parsed) ? clampPanel(parsed, bounds) : bounds.initial
}
