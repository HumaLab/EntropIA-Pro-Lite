import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { EDITOR_MIN_WIDTH, OUTLINE_BOUNDS, RESEARCH_BOUNDS } from '@entropia/ui'
import {
  writingLayout,
  writingRowMinWidth,
  watchPaneWidth,
  WORKSPACE_GAP,
  HANDLE_WIDTH,
} from './writing-layout'

/**
 * The three-column row's minimum width, at every combination of the two
 * foldable panels — derived straight from the constants `.writing__workspace`
 * is built from in WritingView.svelte: `var(--space-3)` (12px) between every
 * rendered child, a `ResizeHandle` (9px) beside each open panel, and
 * `var(--space-5)` (20px) of `.writing` padding on both sides.
 */
describe('writingRowMinWidth', () => {
  it('outline and research both open: 5 children, 4 gaps, 2 handles', () => {
    // 40 (inset) + 120 (outline squeeze) + 320 (editor) + 140 (research squeeze)
    // + 2*9 (handles) + 4*12 (gaps) = 686
    expect(writingRowMinWidth(true, true)).toBe(686)
  })

  it('research only: 3 children, 2 gaps, 1 handle', () => {
    // 40 + 320 + 140 + 9 + 2*12 = 533
    expect(writingRowMinWidth(false, true)).toBe(533)
  })

  it('outline only: 3 children, 2 gaps, 1 handle', () => {
    // 40 + 120 + 320 + 9 + 2*12 = 513
    expect(writingRowMinWidth(true, false)).toBe(513)
  })

  it('neither panel: just the editor and the inset', () => {
    expect(writingRowMinWidth(false, false)).toBe(360)
  })

  it('is built from the same tokens the stylesheet uses', () => {
    expect(WORKSPACE_GAP).toBe(12)
    expect(HANDLE_WIDTH).toBe(9)
    expect(OUTLINE_BOUNDS.squeeze).toBe(120)
    expect(RESEARCH_BOUNDS.squeeze).toBe(140)
    expect(EDITOR_MIN_WIDTH).toBe(320)
  })
})

/**
 * The decision itself (plan: "forced outline collapse").
 *
 * The outline is forced shut exactly where opening it would not leave the
 * editor and the (currently visible) research panel their own floors — never
 * by the split ratio, never by the window's own width, only by what this pane
 * actually renders.
 */
describe('writingLayout — forcing the outline shut', () => {
  it('leaves the outline open at its own preference when there is room', () => {
    const layout = writingLayout(800, true, true)
    expect(layout.forceOutlineCollapse).toBe(false)
    expect(layout.effectiveOutlineOpen).toBe(true)
  })

  it('forces it shut one pixel under the threshold, with research open', () => {
    const layout = writingLayout(685, true, true)
    expect(layout.forceOutlineCollapse).toBe(true)
    expect(layout.effectiveOutlineOpen).toBe(false)
  })

  it('does not force it shut exactly at the threshold', () => {
    const layout = writingLayout(686, true, true)
    expect(layout.forceOutlineCollapse).toBe(false)
    expect(layout.effectiveOutlineOpen).toBe(true)
  })

  it('never reports the outline open when the user closed it themselves', () => {
    // Plenty of room, but the preference is closed: forcing is about whether
    // it COULD open, not about opening it against the user's own choice.
    const layout = writingLayout(2000, false, true)
    expect(layout.effectiveOutlineOpen).toBe(false)
  })

  it('uses a lower threshold once the research panel is closed', () => {
    // 513..685 fits the outline with research closed but not with it open.
    expect(writingLayout(600, true, false).forceOutlineCollapse).toBe(false)
    expect(writingLayout(600, true, true).forceOutlineCollapse).toBe(true)
  })

  it('screenshot regression: a ~650px pane with both panels open force-collapses', () => {
    const layout = writingLayout(650, true, true)
    expect(layout.forceOutlineCollapse).toBe(true)
    expect(layout.effectiveOutlineOpen).toBe(false)
  })
})

/**
 * The research panel's floor, at the five widths the manual check covers.
 *
 * It never asks for more than the pane actually has left once the editor (a
 * hard, non-negotiable 320px) and whatever the outline is really showing have
 * taken their share — so the row's total never exceeds `paneWidth`, and
 * nothing is left for `overflow-x: hidden` to clip.
 */
describe('writingLayout — the research panel never asks for more than is left', () => {
  it('500px: outline forced shut, research squeezed below its usual floor', () => {
    const layout = writingLayout(500, true, true)
    expect(layout.effectiveOutlineOpen).toBe(false)
    expect(layout.researchMinWidth).toBe(119)
    // The row's real minimum, given what actually renders, fits exactly.
    expect(
      writingRowMinWidth(false, false) + HANDLE_WIDTH + WORKSPACE_GAP + layout.researchMinWidth
    ).toBe(500)
  })

  it('650px: outline forced shut, research keeps its usual floor', () => {
    const layout = writingLayout(650, true, true)
    expect(layout.researchMinWidth).toBe(RESEARCH_BOUNDS.squeeze)
  })

  it('800px, 1000px and a wide window: outline open, research keeps its usual floor', () => {
    for (const width of [800, 1000, 1624]) {
      const layout = writingLayout(width, true, true)
      expect(layout.effectiveOutlineOpen).toBe(true)
      expect(layout.researchMinWidth).toBe(RESEARCH_BOUNDS.squeeze)
    }
  })

  it('never goes negative, even narrower than the editor alone can fit', () => {
    expect(writingLayout(200, true, true).researchMinWidth).toBe(0)
  })

  it('is irrelevant, but still a sane number, when research is closed', () => {
    expect(writingLayout(300, true, false).researchMinWidth).toBeGreaterThanOrEqual(0)
  })
})

describe('watchPaneWidth', () => {
  type Callback = (entries: { contentRect: { width: number } }[]) => void
  class FakeResizeObserver {
    static instances: FakeResizeObserver[] = []
    callback: Callback
    constructor(callback: Callback) {
      this.callback = callback
      FakeResizeObserver.instances.push(this)
    }
    observe() {}
    disconnect() {}
  }

  beforeEach(() => {
    FakeResizeObserver.instances = []
    vi.stubGlobal('ResizeObserver', FakeResizeObserver)
  })
  afterEach(() => vi.unstubAllGlobals())

  it('reports the observed element width on every callback', () => {
    const onChange = vi.fn()
    const el = document.createElement('div')
    watchPaneWidth(el, onChange)
    const observer = FakeResizeObserver.instances.at(-1)!

    observer.callback([{ contentRect: { width: 650 } }])
    expect(onChange).toHaveBeenCalledWith(650)

    observer.callback([{ contentRect: { width: 1000 } }])
    expect(onChange).toHaveBeenCalledWith(1000)
  })

  it('disconnects on cleanup', () => {
    const el = document.createElement('div')
    const disconnect = vi.spyOn(FakeResizeObserver.prototype, 'disconnect')
    const stop = watchPaneWidth(el, vi.fn())
    stop()
    expect(disconnect).toHaveBeenCalledTimes(1)
  })

  it('is a safe no-op when ResizeObserver is unavailable', () => {
    vi.unstubAllGlobals()
    // @ts-expect-error -- deliberately simulating an environment without it
    delete globalThis.ResizeObserver
    const el = document.createElement('div')
    expect(() => watchPaneWidth(el, vi.fn())()).not.toThrow()
  })
})
