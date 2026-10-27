import type { PdfRect } from './types'

/**
 * Zotero's `annotationSortIndex`, ported from the Zotero reader at
 * zotero/reader@0b16e731cf7d880e74faaf8808921678cac38aae:
 * `src/pdf/selection.js` (`rectsDist`, `getClosestOffset`,
 * `getTopMostRectFromPosition`, `getSortIndexRect`, `formatSortIndex`,
 * `getSortIndex`) and `src/pdf/lib/utilities.js` (`getPositionBoundingRect`).
 * Keep the port literal, quirks included: the reader picks the "top-most"
 * rect by sorting on `x2` (`b[2] - a[2]`), not `y2`, and sort indexes only
 * match Zotero's if this does the same.
 *
 * The one input that is not Zotero's is the page characters: the reader gets
 * them from its pdf-worker. Here they come from the rendered pdf.js text layer
 * (one box per glyph, see `charsFromTextLayer`), or, without it, from pdf.js
 * text items split evenly per glyph. Offsets equal Zotero's when both list the
 * page's characters in the same order with boxes close enough that the same
 * one is nearest.
 *
 * Format: `ppppp|oooooo|yyyyy`: page index, zero-based character offset and
 * floored distance from the page top, each clipped and zero-padded.
 */

/** One page character with its box in PDF user space. */
export interface PageChar {
  rect: PdfRect
}

/** The parts of a pdf.js `TextItem` the fallback needs. */
export interface TextItemLike {
  str: string
  /** `[a, b, c, d, e, f]`: e/f are the baseline origin in PDF space. */
  transform: number[]
  width: number
  height: number
}

/** The position fields the sort index reads. */
export interface SortPosition {
  pageIndex: number
  rects?: PdfRect[]
  paths?: number[][]
}

/**
 * Fallback character boxes from pdf.js text content: each item's box split
 * evenly across its glyphs. Whitespace-only items carry no characters.
 */
export function charsFromTextItems(items: TextItemLike[]): PageChar[] {
  const chars: PageChar[] = []
  for (const item of items) {
    const glyphs = Array.from(item.str)
    if (glyphs.length === 0 || item.str.trim() === '') continue
    const x = item.transform[4] ?? 0
    const y = item.transform[5] ?? 0
    const height = item.height || Math.hypot(item.transform[2] ?? 0, item.transform[3] ?? 0)
    const step = item.width / glyphs.length
    glyphs.forEach((_, index) => {
      chars.push({ rect: [x + index * step, y, x + (index + 1) * step, y + height] })
    })
  }
  return chars
}

/** `rectsDist` of the reader: 0 when the rects touch or overlap. */
export function rectsDist(a: PdfRect, b: PdfRect): number {
  const [ax1, ay1, ax2, ay2] = a
  const [bx1, by1, bx2, by2] = b
  const left = bx2 < ax1
  const right = ax2 < bx1
  const bottom = by2 < ay1
  const top = ay2 < by1
  if (top && left) return Math.hypot(ax1 - bx2, ay2 - by1)
  if (left && bottom) return Math.hypot(ax1 - bx2, ay1 - by2)
  if (bottom && right) return Math.hypot(ax2 - bx1, ay1 - by2)
  if (right && top) return Math.hypot(ax2 - bx1, ay2 - by1)
  if (left) return ax1 - bx2
  if (right) return bx1 - ax2
  if (bottom) return ay1 - by2
  if (top) return by1 - ay2
  return 0
}

/** Index of the character nearest to `rect`; the first one on ties. */
export function getClosestOffset(chars: PageChar[], rect: PdfRect): number {
  let dist = Infinity
  let idx = 0
  for (let i = 0; i < chars.length; i++) {
    const char = chars[i]
    if (!char) continue
    const distance = rectsDist(char.rect, rect)
    if (distance < dist) {
      dist = distance
      idx = i
    }
  }
  return idx
}

/** `getPositionBoundingRect` for the current page, rects or ink paths. */
function getPositionBoundingRect(position: SortPosition): PdfRect | undefined {
  const rects = position.rects
  if (rects && rects.length > 0) {
    return [
      Math.min(...rects.map((rect) => rect[0])),
      Math.min(...rects.map((rect) => rect[1])),
      Math.max(...rects.map((rect) => rect[2])),
      Math.max(...rects.map((rect) => rect[3])),
    ]
  }
  const first = position.paths?.[0]
  if (!position.paths || !first || first.length < 2) return undefined
  const rect: PdfRect = [first[0] ?? 0, first[1] ?? 0, first[0] ?? 0, first[1] ?? 0]
  for (const path of position.paths) {
    for (let i = 0; i < path.length - 1; i += 2) {
      const x = path[i] ?? 0
      const y = path[i + 1] ?? 0
      rect[0] = Math.min(rect[0], x)
      rect[1] = Math.min(rect[1], y)
      rect[2] = Math.max(rect[2], x)
      rect[3] = Math.max(rect[3], y)
    }
  }
  return rect
}

/** The rect whose top edge determines an annotation's sort index. */
export function getSortIndexRect(position: SortPosition): PdfRect | undefined {
  const topMost = position.rects?.slice().sort((a, b) => b[2] - a[2])[0]
  return topMost ?? getPositionBoundingRect(position)
}

export function formatSortIndex(pageIndex: number, offset: number, top: number): string {
  return [
    pageIndex.toString().slice(0, 5).padStart(5, '0'),
    offset.toString().slice(0, 6).padStart(6, '0'),
    Math.floor(top).toString().slice(0, 5).padStart(5, '0'),
  ].join('|')
}

/**
 * The reader's `getSortIndex` for one page: `chars` and `viewBox` are that
 * page's. Without a usable rect the offset and top stay 0, as in the reader
 * when the page is not loaded.
 */
export function getSortIndex(chars: PageChar[], viewBox: number[], position: SortPosition): string {
  const rect = getSortIndexRect(position)
  if (!rect) return formatSortIndex(position.pageIndex, 0, 0)
  const offset = (chars.length && getClosestOffset(chars, rect)) || 0
  const pageHeight = (viewBox[3] ?? 0) - (viewBox[1] ?? 0)
  let top = pageHeight - rect[3]
  if (top < 0) top = 0
  return formatSortIndex(position.pageIndex, offset, top)
}

/** The PDF-space point of a viewport point, as pdf.js `PageViewport` gives it. */
export interface ViewportToPdf {
  convertToPdfPoint(x: number, y: number): number[]
}

/**
 * One box per glyph of a rendered pdf.js text layer, in reading order, in PDF
 * user space. pdf.js scales each span to its glyph run, so a DOM range around
 * one character measures where that glyph is drawn. Returns `null` when the
 * layer cannot be measured (no layout, as in tests), so the caller falls back
 * to the text items.
 */
export function charsFromTextLayer(
  container: HTMLElement,
  viewport: ViewportToPdf
): PageChar[] | null {
  const doc = container.ownerDocument
  if (!doc || typeof doc.createRange !== 'function') return null
  const origin = container.getBoundingClientRect()
  const walker = doc.createTreeWalker(container, NodeFilter.SHOW_TEXT)
  const chars: PageChar[] = []
  let measured = false
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent ?? ''
    let offset = 0
    for (const glyph of Array.from(text)) {
      const length = glyph.length
      if (glyph.trim() !== '') {
        const range = doc.createRange()
        range.setStart(node, offset)
        range.setEnd(node, offset + length)
        const box = range.getBoundingClientRect()
        if (box.width > 0 || box.height > 0) measured = true
        const x1 = box.left - origin.left
        const y1 = box.top - origin.top
        const [ax = 0, ay = 0] = viewport.convertToPdfPoint(x1, y1)
        const [bx = 0, by = 0] = viewport.convertToPdfPoint(x1 + box.width, y1 + box.height)
        chars.push({
          rect: [Math.min(ax, bx), Math.min(ay, by), Math.max(ax, bx), Math.max(ay, by)],
        })
      }
      offset += length
    }
  }
  return measured ? chars : null
}
