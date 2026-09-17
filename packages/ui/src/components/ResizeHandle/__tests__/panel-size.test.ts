import { describe, expect, it } from 'vitest'
import {
  EDITOR_MIN_WIDTH,
  KEYBOARD_PAGE,
  KEYBOARD_STEP,
  OUTLINE_BOUNDS,
  RESEARCH_BOUNDS,
  clampPanel,
  readPanelWidth,
  widthAfterDrag,
  widthAfterKey,
  type PanelBounds,
} from '../panel-size'

/**
 * Resizing a side panel (plan-editor.md §18).
 *
 * The rule worth testing hardest is the direction. A panel left of the editor
 * and a panel right of it respond to opposite pointer movements, and getting
 * that backwards produces something that fights whoever is dragging it — a bug
 * that survives review because everyone assumes they are holding it wrong.
 */

const bounds: PanelBounds = { min: 100, max: 400, initial: 200, squeeze: 80 }

describe('the bounds', () => {
  /**
   * §18 asks for panels that are resizable *and* foldable. Folding has a button
   * that can undo itself; a drag to zero has nothing — the panel is gone and so
   * is the handle that would bring it back.
   */
  it('never lets a panel be dragged out of existence', () => {
    expect(OUTLINE_BOUNDS.min).toBeGreaterThan(0)
    expect(RESEARCH_BOUNDS.min).toBeGreaterThan(0)
  })

  /** The research panel holds four tabs of results; the outline holds a list. */
  it('starts the research panel wider than the outline', () => {
    expect(RESEARCH_BOUNDS.initial).toBeGreaterThan(OUTLINE_BOUNDS.initial)
  })

  /**
   * The invariant the first attempt at this got wrong, in the direction that
   * matters: the panels yield to the manuscript, never the reverse.
   *
   * Without a floor on the editor, flexbox squeezed *it* first — it carries the
   * `min-width` that lets it take leftover room — so the panels never reached
   * the point of having to give way. Dragging the outline wide left a list of
   * headings holding two thirds of the window while the prose wrapped one word
   * per line.
   */
  it('gives the manuscript a floor of its own', () => {
    expect(EDITOR_MIN_WIDTH).toBeGreaterThan(OUTLINE_BOUNDS.min)
    expect(EDITOR_MIN_WIDTH).toBeGreaterThan(RESEARCH_BOUNDS.min)
  })

  /**
   * And the three floors together have to fit the smallest window the app
   * allows, or the guarantee is an aspiration: 900px is the window's minimum
   * and 1.25 the zoom ceiling, which leaves 720 CSS pixels for two panels, two
   * resize handles and the manuscript between them.
   *
   * Raising any one minimum without checking this would reintroduce the
   * horizontal overflow §18 forbids, and it would only show up on someone
   * else's narrow window.
   */
  it('squeezes no further than the panels are meant to go', () => {
    expect(OUTLINE_BOUNDS.squeeze).toBeGreaterThan(0)
    expect(RESEARCH_BOUNDS.squeeze).toBeLessThan(RESEARCH_BOUNDS.min)
    expect(OUTLINE_BOUNDS.squeeze).toBeLessThan(OUTLINE_BOUNDS.min)
  })

  /**
   * The floors plus **the chrome around them** have to fit the smallest window
   * the app allows. The first version of this test left the chrome out and
   * passed while the workspace was overflowing and the page had grown a
   * horizontal scrollbar — the arithmetic was right about the columns and wrong
   * about the page around them.
   *
   * So the padding, the gaps and the borders are counted here, from the tokens
   * the view actually uses. What is asserted is not a tidy number; it is that
   * at 900px and 125% zoom nothing is pushed past the viewport, because that is
   * the whole of what "ausencia de desbordes" means and a horizontal scrollbar
   * is how it announces itself.
   */
  it('fits its floors and its chrome into the narrowest window the app allows', () => {
    const WINDOW_MIN = 900
    const ZOOM_MAX = 1.25

    const PAGE_PADDING = 20 * 2 // --space-5, both sides
    const COLUMN_GAPS = 12 * 4 // --space-3, between five children
    const PANEL_BORDERS = 1 * 2 * 3
    const HANDLES = 9 * 2

    const available = WINDOW_MIN / ZOOM_MAX
    const needed =
      OUTLINE_BOUNDS.squeeze +
      RESEARCH_BOUNDS.squeeze +
      EDITOR_MIN_WIDTH +
      PAGE_PADDING +
      COLUMN_GAPS +
      PANEL_BORDERS +
      HANDLES

    expect(needed, `${needed}px needed, ${available}px available`).toBeLessThanOrEqual(available)
  })

  it('keeps a width inside its range', () => {
    expect(clampPanel(50, bounds)).toBe(100)
    expect(clampPanel(900, bounds)).toBe(400)
    expect(clampPanel(250, bounds)).toBe(250)
  })

  /**
   * A width that is not a number would collapse the panel and leave nothing on
   * screen to say that anything had gone wrong.
   */
  it('falls back to the initial width rather than to nothing', () => {
    expect(clampPanel(Number.NaN, bounds)).toBe(200)
    // Infinity is not "a very large width" to be clamped down to the maximum —
    // it is a calculation that went wrong, and the honest answer to that is the
    // width the panel started at, not the widest one it is allowed.
    expect(clampPanel(Number.POSITIVE_INFINITY, bounds)).toBe(200)
    expect(clampPanel(Number.NEGATIVE_INFINITY, bounds)).toBe(200)
  })

  it('gives back whole pixels', () => {
    expect(clampPanel(250.4, bounds)).toBe(250)
  })
})

describe('dragging', () => {
  /** Left of the editor: the pointer moving right makes the panel wider. */
  it('widens a panel on the start side when dragged right', () => {
    expect(widthAfterDrag(200, 40, 'start', bounds)).toBe(240)
    expect(widthAfterDrag(200, -40, 'start', bounds)).toBe(160)
  })

  /** Right of the editor: the same gesture makes it narrower. */
  it('narrows a panel on the end side when dragged right', () => {
    expect(widthAfterDrag(200, 40, 'end', bounds)).toBe(160)
    expect(widthAfterDrag(200, -40, 'end', bounds)).toBe(240)
  })

  it('stops at the ends of the range instead of running past them', () => {
    expect(widthAfterDrag(200, 5000, 'start', bounds)).toBe(400)
    expect(widthAfterDrag(200, -5000, 'start', bounds)).toBe(100)
  })

  /**
   * The width is computed from where the drag *started*, not from the last
   * frame. Accumulating frame by frame drifts, and worse, a pointer that runs
   * past the edge and comes back does not return to where it left.
   */
  it('is computed from the width the drag began at', () => {
    const start = 200
    expect(widthAfterDrag(start, 100, 'start', bounds)).toBe(300)
    expect(widthAfterDrag(start, 0, 'start', bounds)).toBe(200)
  })
})

describe('the keyboard', () => {
  /**
   * §18's first requirement. A resizer that only answers a pointer fails it
   * outright, and this is the half that is easy to leave for later and never do.
   */
  it('moves the panel with the arrow keys', () => {
    expect(widthAfterKey('ArrowRight', 200, 'start', bounds)).toBe(200 + KEYBOARD_STEP)
    expect(widthAfterKey('ArrowLeft', 200, 'start', bounds)).toBe(200 - KEYBOARD_STEP)
  })

  /**
   * The arrow that widens a left panel widens a right one too. Someone pressing
   * a key is asking for a direction they can see, not for a sign in a formula.
   */
  it('makes the same arrow mean the same thing on either side', () => {
    const left = widthAfterKey('ArrowRight', 200, 'start', bounds)
    const right = widthAfterKey('ArrowLeft', 200, 'end', bounds)

    expect(left).toBe(216)
    expect(right).toBe(216)
  })

  it('crosses the range faster with the page keys', () => {
    expect(widthAfterKey('PageUp', 200, 'start', bounds)).toBe(200 + KEYBOARD_PAGE)
    expect(widthAfterKey('PageDown', 200, 'start', bounds)).toBe(200 - KEYBOARD_PAGE)
  })

  it('goes to either end with Home and End', () => {
    expect(widthAfterKey('Home', 300, 'start', bounds)).toBe(100)
    expect(widthAfterKey('End', 150, 'start', bounds)).toBe(400)
  })

  /** Without a way back, trying the resizer is a one-way decision. */
  it('returns to where it started on Enter or Space', () => {
    expect(widthAfterKey('Enter', 380, 'start', bounds)).toBe(200)
    expect(widthAfterKey(' ', 110, 'end', bounds)).toBe(200)
  })

  /**
   * `null`, not the unchanged width: the caller has to be able to tell a key
   * that does nothing here from one that hit the end of the range, and let the
   * first through to whatever else was listening.
   */
  it('reports a key it does not handle rather than swallowing it', () => {
    expect(widthAfterKey('a', 200, 'start', bounds)).toBeNull()
    expect(widthAfterKey('Tab', 200, 'start', bounds)).toBeNull()
    expect(widthAfterKey('Escape', 200, 'start', bounds)).toBeNull()
  })

  it('stops at the ends rather than running past them', () => {
    expect(widthAfterKey('ArrowLeft', 100, 'start', bounds)).toBe(100)
    expect(widthAfterKey('ArrowRight', 400, 'start', bounds)).toBe(400)
  })
})

describe('reading a stored width', () => {
  it('reads one back', () => {
    expect(readPanelWidth('260', bounds)).toBe(260)
  })

  /** Nothing stored is the first run, not a failure. */
  it('starts at the initial width when nothing was stored', () => {
    expect(readPanelWidth(null, bounds)).toBe(200)
    expect(readPanelWidth('', bounds)).toBe(200)
    expect(readPanelWidth('   ', bounds)).toBe(200)
  })

  /**
   * A setting is text by the time it comes back, and can be anything: edited by
   * hand, or written by a build whose bounds were different. None of that should
   * produce a panel nobody can see.
   */
  it('does not let a bad setting produce an invisible panel', () => {
    expect(readPanelWidth('no es un numero', bounds)).toBe(200)
    expect(readPanelWidth('0', bounds)).toBe(100)
    expect(readPanelWidth('-40', bounds)).toBe(100)
    expect(readPanelWidth('99999', bounds)).toBe(400)
  })
})
