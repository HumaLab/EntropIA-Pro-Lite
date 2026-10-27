import { describe, expect, it, vi } from 'vitest'
import {
  createMarkupViewport,
  noteRectAt,
  pageIndexToPage,
  pageToPageIndex,
  selectionToPdfRects,
  toPdfRect,
  toViewportBox,
} from '../geometry'
import {
  charsFromTextItems,
  formatSortIndex,
  getClosestOffset,
  getSortIndex,
  getSortIndexRect,
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

describe('sort index', () => {
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
    expect(formatSortIndex(0, 0, -4)).toBe('00000|000000|00000')
  })

  it('gives offset 0 for an annotation nearest the first character, and for an empty page', () => {
    expect(getClosestOffset(chars, [95, 700, 101, 710])).toBe(0)
    expect(getSortIndex(0, chars, LETTER, { rects: [[95, 700, 101, 710]] })).toBe('00000|000000|00082')
    expect(getSortIndex(2, [], LETTER, { rects: [[95, 700, 101, 710]] })).toBe('00002|000000|00082')
  })

  it('takes the closest character for notes and areas too', () => {
    const noteBesideWorld: PdfRect[] = [[155, 648, 177, 670]]
    expect(getSortIndex(1, chars, LETTER, { rects: noteBesideWorld })).toBe('00001|000009|00122')
  })

  it('uses the bounds of ink strokes', () => {
    expect(getSortIndexRect({ paths: [[10, 20, 30, 40], [5, 50]] })).toEqual([5, 20, 30, 50])
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
