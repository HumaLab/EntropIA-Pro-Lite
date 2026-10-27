import type { PdfRect } from './types'
import { boundingPdfRect } from './geometry'

/**
 * Zotero's `annotationSortIndex`, ported from the Zotero reader
 * (`zotero/reader`, `src/pdf/selection.js`: `getSortIndex`,
 * `getClosestOffset`, and the rect choice of `getSortIndexRect`).
 *
 * TODO(zotero-annotations): pin this port to a specific upstream commit and
 * check it against the live fixtures of docs/zotero-annotations-plan.md
 * section 8 (rotation, CropBox origin, UserUnit, notes and areas beside
 * text). Until then sort indexes equal Zotero's only when the page's
 * character boxes match the reader's.
 *
 * Format: `ppppp|oooooo|yyyyy` — page index, zero-based character offset,
 * floored distance from the top of the page.
 */

/** One page character with its box in PDF user space. */
export interface PageChar {
  rect: PdfRect
}

/** The parts of a pdf.js `TextItem` the port needs. */
export interface TextItemLike {
  str: string
  /** `[a, b, c, d, e, f]`: e/f are the baseline origin in PDF space. */
  transform: number[]
  width: number
  height: number
}

/**
 * Character boxes from pdf.js text content: each item's box split evenly
 * across its characters. Whitespace-only items carry no characters, as in
 * the reader.
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

/** The rect the sort index is computed from: the first rect, or the bounds of the strokes. */
export function getSortIndexRect(position: { rects?: PdfRect[]; paths?: number[][] }): PdfRect | null {
  if (position.rects && position.rects.length > 0) return position.rects[0] ?? null
  if (position.paths && position.paths.length > 0) {
    const rects: PdfRect[] = position.paths
      .filter((path) => path.length >= 2)
      .map((path) => {
        const xs = path.filter((_, index) => index % 2 === 0)
        const ys = path.filter((_, index) => index % 2 === 1)
        return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)]
      })
    return boundingPdfRect(rects)
  }
  return null
}

function rectDistance(a: PdfRect, b: PdfRect): number {
  const dx = Math.max(0, Math.max(a[0], b[0]) - Math.min(a[2], b[2]))
  const dy = Math.max(0, Math.max(a[1], b[1]) - Math.min(a[3], b[3]))
  return Math.hypot(dx, dy)
}

/** Index of the character closest to `rect` (first one on ties); 0 on an empty page. */
export function getClosestOffset(chars: PageChar[], rect: PdfRect): number {
  let best = Infinity
  let offset = 0
  chars.forEach((char, index) => {
    const distance = rectDistance(char.rect, rect)
    if (distance < best) {
      best = distance
      offset = index
    }
  })
  return offset
}

export function formatSortIndex(pageIndex: number, offset: number, top: number): string {
  return [
    String(pageIndex).slice(0, 5).padStart(5, '0'),
    String(offset).slice(0, 6).padStart(6, '0'),
    String(Math.max(0, Math.floor(top))).slice(0, 5).padStart(5, '0'),
  ].join('|')
}

/**
 * The sort index of a markup on a page with `chars`, for every type: notes
 * and areas take the closest character too.
 */
export function getSortIndex(
  pageIndex: number,
  chars: PageChar[],
  viewBox: number[],
  position: { rects?: PdfRect[]; paths?: number[][] }
): string {
  const rect = getSortIndexRect(position)
  if (!rect) return formatSortIndex(pageIndex, 0, 0)
  const offset = chars.length > 0 ? getClosestOffset(chars, rect) : 0
  const pageTop = viewBox[3] ?? 0
  const top = pageTop - rect[3]
  return formatSortIndex(pageIndex, offset, top)
}
