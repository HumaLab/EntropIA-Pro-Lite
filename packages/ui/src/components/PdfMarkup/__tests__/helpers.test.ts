import { describe, expect, it, vi } from 'vitest'
import {
  createMarkupViewport,
  noteRectAt,
  popoverAnchor,
  pageIndexToPage,
  pageToPageIndex,
  selectionToPdfRects,
  toPdfRect,
  toViewportBox,
} from '../geometry'
import {
  charsFromTextItems,
  charsFromTextLayer,
  formatSortIndex,
  getClosestOffset,
  getSortIndex,
  getSortIndexRect,
  rectsDist,
} from '../sortIndex'
import { TextLayerController } from '../textLayerController'
import type { PdfRect } from '../types'

const LETTER = [0, 0, 612, 792]

function close(actual: number[], expected: number[]) {
  expect(actual.length).toBe(expected.length)
  actual.forEach((value, index) => expect(value).toBeCloseTo(expected[index] ?? NaN, 6))
}

describe('page index', () => {
  it('converts between Zotero page indexes and viewer pages', () => {
    expect(pageIndexToPage(0)).toBe(1)
    expect(pageToPageIndex(1)).toBe(0)
  })
})

describe('coordinates', () => {
  it('maps a PDF rect to the top-left viewport box at rotation 0', () => {
    const viewport = createMarkupViewport(LETTER, 2, 0)
    expect(viewport.width).toBe(1224)
    expect(viewport.height).toBe(1584)
    const box = toViewportBox(viewport, [100, 700, 200, 720])
    close([box.x, box.y, box.width, box.height], [200, 144, 200, 40])
  })

  it('round-trips rects for every rotation and a nonzero CropBox origin', () => {
    const rect: PdfRect = [130, 250, 190, 268]
    for (const rotation of [0, 90, 180, 270]) {
      for (const viewBox of [LETTER, [36, 48, 576, 744]]) {
        const viewport = createMarkupViewport(viewBox, 1.5, rotation)
        close(toPdfRect(viewport, toViewportBox(viewport, rect)), rect)
      }
    }
  })

  it('swaps width and height at 90 and 270 degrees', () => {
    const viewport = createMarkupViewport(LETTER, 1, 90)
    expect([viewport.width, viewport.height]).toEqual([792, 612])
    const box = toViewportBox(viewport, [0, 0, 100, 10])
    close([box.width, box.height], [10, 100])
  })

  it('places a CropBox origin at the viewport origin', () => {
    const viewport = createMarkupViewport([36, 48, 576, 744], 1, 0)
    const box = toViewportBox(viewport, [36, 734, 46, 744])
    close([box.x, box.y], [0, 0])
  })

  it('carries UserUnit through the scale', () => {
    const userUnit = 2
    const viewport = createMarkupViewport(LETTER, 1.25 * userUnit, 0)
    const box = toViewportBox(viewport, [0, 782, 10, 792])
    close([box.width], [25])
  })

  it('centers a 22 pt note on the click', () => {
    const viewport = createMarkupViewport(LETTER, 1, 0)
    close(noteRectAt(viewport, 100, 100), [89, 681, 111, 703])
  })

  it('merges selection rects into one PDF rect per line', () => {
    const viewport = createMarkupViewport(LETTER, 1, 0)
    const rects = selectionToPdfRects(viewport, [
      { x: 10, y: 100, width: 50, height: 12 },
      { x: 60, y: 101, width: 40, height: 11 },
      { x: 10, y: 120, width: 30, height: 12 },
      { x: 5, y: 5, width: 0, height: 0 },
    ])
    expect(rects).toHaveLength(2)
    close(rects[0] ?? [], [10, 680, 100, 692])
    close(rects[1] ?? [], [10, 660, 40, 672])
  })
})

describe('sort index (zotero/reader@0b16e731)', () => {
  const chars = charsFromTextItems([
    { str: 'Hello', transform: [10, 0, 0, 10, 100, 700], width: 50, height: 10 },
    { str: '   ', transform: [10, 0, 0, 10, 150, 700], width: 30, height: 10 },
    { str: 'World', transform: [10, 0, 0, 10, 100, 650], width: 50, height: 10 },
  ])

  it('splits text items into character boxes and skips whitespace items', () => {
    expect(chars).toHaveLength(10)
    close(chars[1]?.rect ?? [], [110, 700, 120, 710])
  })

  it('formats page, offset and top with zero padding', () => {
    expect(formatSortIndex(3, 42, 101.9)).toBe('00003|000042|00101')
  })

  it('measures distance like rectsDist: corners by hypot, sides by gap, overlap 0', () => {
    expect(rectsDist([0, 0, 10, 10], [13, 14, 20, 20])).toBe(5)
    expect(rectsDist([0, 0, 10, 10], [15, 2, 20, 8])).toBe(5)
    expect(rectsDist([0, 0, 10, 10], [5, 5, 20, 20])).toBe(0)
  })

  it('gives offset 0 for an annotation nearest the first character, and for an empty page', () => {
    expect(getClosestOffset(chars, [95, 700, 101, 710])).toBe(0)
    expect(getSortIndex(chars, LETTER, { pageIndex: 0, rects: [[95, 700, 101, 710]] })).toBe(
      '00000|000000|00082'
    )
    expect(getSortIndex([], LETTER, { pageIndex: 2, rects: [[95, 700, 101, 710]] })).toBe(
      '00002|000000|00082'
    )
  })

  it('takes the closest character for notes and areas too', () => {
    const noteBesideWorld: PdfRect[] = [[155, 648, 177, 670]]
    expect(getSortIndex(chars, LETTER, { pageIndex: 1, rects: noteBesideWorld })).toBe(
      '00001|000009|00122'
    )
  })

  it('measures top from the page height, so a CropBox origin does not shift it', () => {
    const cropped = [36, 48, 576, 744]
    expect(getSortIndex([], cropped, { pageIndex: 0, rects: [[100, 600, 120, 650]] })).toBe(
      '00000|000000|00046'
    )
    expect(getSortIndex([], cropped, { pageIndex: 0, rects: [[100, 600, 120, 900]] })).toBe(
      '00000|000000|00000'
    )
  })

  it('picks the "top-most" rect by x2, exactly as the reader does', () => {
    const rects: PdfRect[] = [
      [10, 700, 50, 710],
      [10, 680, 300, 690],
    ]
    expect(getSortIndexRect({ pageIndex: 0, rects })).toEqual([10, 680, 300, 690])
  })

  it('uses the bounds of ink strokes', () => {
    expect(getSortIndexRect({ pageIndex: 0, paths: [[10, 20, 30, 40], [5, 50]] })).toEqual([
      5, 20, 30, 50,
    ])
  })

  it('reads glyph boxes from a rendered text layer and gives up without layout', () => {
    const container = document.createElement('div')
    container.innerHTML = '<span>Ab</span>'
    document.body.append(container)
    const viewport = createMarkupViewport(LETTER, 1, 0)
    expect(charsFromTextLayer(container, viewport)).toBeNull()
    const spy = vi.spyOn(Range.prototype, 'getBoundingClientRect')
    let call = 0
    spy.mockImplementation(() => {
      const left = 10 + 8 * call++
      return { left, top: 100, width: 8, height: 12, right: left + 8, bottom: 112, x: left, y: 100, toJSON: () => ({}) } as DOMRect
    })
    const glyphs = charsFromTextLayer(container, viewport)
    expect(glyphs).toHaveLength(2)
    close(glyphs?.[1]?.rect ?? [], [18, 680, 26, 692])
    spy.mockRestore()
    container.remove()
  })
})

describe('text layer lifecycle', () => {
  function handle() {
    return { cancel: vi.fn(), remove: vi.fn() }
  }

  it('rebuilds on a page change even between same-size pages at the same zoom', () => {
    const controller = new TextLayerController()
    const first = handle()
    expect(controller.sync({ document: 'a.pdf', page: 1, scale: 1.5 }, () => first)).toBe('built')
    const second = handle()
    expect(controller.sync({ document: 'a.pdf', page: 2, scale: 1.5 }, () => second)).toBe('built')
    expect(first.cancel).toHaveBeenCalled()
    expect(first.remove).toHaveBeenCalled()
  })

  it('rebuilds on an attachment or zoom change and keeps an unchanged identity', () => {
    const controller = new TextLayerController()
    const first = handle()
    controller.sync({ document: 'a.pdf', page: 1, scale: 1 }, () => first)
    expect(controller.sync({ document: 'a.pdf', page: 1, scale: 1 }, handle)).toBe('kept')
    expect(first.remove).not.toHaveBeenCalled()
    expect(controller.sync({ document: 'b.pdf', page: 1, scale: 1 }, handle)).toBe('built')
    expect(first.remove).toHaveBeenCalled()
    expect(controller.sync({ document: 'b.pdf', page: 1, scale: 2 }, handle)).toBe('built')
  })

  it('clears the current layer', () => {
    const controller = new TextLayerController()
    const first = handle()
    controller.sync({ document: 'a.pdf', page: 1, scale: 1 }, () => first)
    controller.clear()
    expect(first.cancel).toHaveBeenCalled()
    expect(controller.identity).toBeNull()
  })
})

describe('selection popover anchor', () => {
  it('centers under the whole selection, not its last line', () => {
    const anchor = popoverAnchor(
      [
        { x: 100, y: 100, width: 300, height: 12 },
        { x: 100, y: 116, width: 40, height: 12 },
      ],
      800
    )
    expect(anchor).toEqual({ x: 250, y: 136, placement: 'below' })
  })

  it('goes above the selection near the bottom of the page', () => {
    const anchor = popoverAnchor([{ x: 100, y: 770, width: 100, height: 12 }], 800)
    expect(anchor).toEqual({ x: 150, y: 762, placement: 'above' })
  })
})
