import type { MarkupViewport, PdfPoint, PdfRect } from './types'
import { NOTE_SIZE_PT } from './types'

/** A box in viewport pixels, origin top-left. */
export interface ViewportBox {
  x: number
  y: number
  width: number
  height: number
}

/** Zotero's `pageIndex` is zero-based; the viewer's page is one-based. */
export function pageIndexToPage(pageIndex: number): number {
  return pageIndex + 1
}

export function pageToPageIndex(page: number): number {
  return page - 1
}

/**
 * A viewport with pdf.js `PageViewport` semantics (pdfjs-dist 4.10.38,
 * `display_utils.js`): the page's `view` (CropBox, nonzero origin included),
 * its `rotate` and a scale that already carries UserUnit. The viewer passes
 * pdf.js's own viewport; this one exists for callers without a page proxy and
 * for tests.
 */
export function createMarkupViewport(
  viewBox: number[],
  scale: number,
  rotation: number
): MarkupViewport {
  const [x0 = 0, y0 = 0, x1 = 0, y1 = 0] = viewBox
  const centerX = (x1 + x0) / 2
  const centerY = (y1 + y0) / 2
  const normalized = ((rotation % 360) + 360) % 360
  let a: number, b: number, c: number, d: number
  switch (normalized) {
    case 180:
      ;[a, b, c, d] = [-1, 0, 0, 1]
      break
    case 90:
      ;[a, b, c, d] = [0, 1, 1, 0]
      break
    case 270:
      ;[a, b, c, d] = [0, -1, -1, 0]
      break
    default:
      ;[a, b, c, d] = [1, 0, 0, -1]
  }
  let offsetX: number, offsetY: number, width: number, height: number
  if (a === 0) {
    offsetX = Math.abs(centerY - y0) * scale
    offsetY = Math.abs(centerX - x0) * scale
    width = Math.abs(y1 - y0) * scale
    height = Math.abs(x1 - x0) * scale
  } else {
    offsetX = Math.abs(centerX - x0) * scale
    offsetY = Math.abs(centerY - y0) * scale
    width = Math.abs(x1 - x0) * scale
    height = Math.abs(y1 - y0) * scale
  }
  const m: [number, number, number, number, number, number] = [
    a * scale,
    b * scale,
    c * scale,
    d * scale,
    offsetX - a * scale * centerX - c * scale * centerY,
    offsetY - b * scale * centerX - d * scale * centerY,
  ]
  const apply = (x: number, y: number): [number, number] => [
    x * m[0] + y * m[2] + m[4],
    x * m[1] + y * m[3] + m[5],
  ]
  const det = m[0] * m[3] - m[1] * m[2]
  const inverse = (x: number, y: number): [number, number] => [
    (x * m[3] - y * m[2] + m[2] * m[5] - m[4] * m[3]) / det,
    (-x * m[1] + y * m[0] + m[4] * m[1] - m[5] * m[0]) / det,
  ]
  return {
    width,
    height,
    scale,
    rotation: normalized,
    viewBox: [...viewBox],
    convertToViewportPoint: apply,
    convertToViewportRectangle: (rect: number[]) => {
      const p1 = apply(rect[0] ?? 0, rect[1] ?? 0)
      const p2 = apply(rect[2] ?? 0, rect[3] ?? 0)
      return [p1[0], p1[1], p2[0], p2[1]]
    },
    convertToPdfPoint: inverse,
  }
}

/** A PDF rect as a viewport box, whatever the rotation. */
export function toViewportBox(viewport: MarkupViewport, rect: PdfRect): ViewportBox {
  const [x1 = 0, y1 = 0, x2 = 0, y2 = 0] = viewport.convertToViewportRectangle(rect)
  return {
    x: Math.min(x1, x2),
    y: Math.min(y1, y2),
    width: Math.abs(x2 - x1),
    height: Math.abs(y2 - y1),
  }
}

/** A viewport box back in PDF user space, normalized so x1 < x2 and y1 < y2. */
export function toPdfRect(viewport: MarkupViewport, box: ViewportBox): PdfRect {
  const [ax = 0, ay = 0] = viewport.convertToPdfPoint(box.x, box.y)
  const [bx = 0, by = 0] = viewport.convertToPdfPoint(box.x + box.width, box.y + box.height)
  return [Math.min(ax, bx), Math.min(ay, by), Math.max(ax, bx), Math.max(ay, by)]
}

export function toPdfPoint(viewport: MarkupViewport, x: number, y: number): PdfPoint {
  const [px = 0, py = 0] = viewport.convertToPdfPoint(x, y)
  return [px, py]
}

/** Zotero's note rect: a square of about 22 pt centered on the click. */
export function noteRectAt(viewport: MarkupViewport, x: number, y: number): PdfRect {
  const [px, py] = toPdfPoint(viewport, x, y)
  const half = NOTE_SIZE_PT / 2
  return [px - half, py - half, px + half, py + half]
}

/** Moves a PDF rect by a viewport-pixel delta. */
export function movePdfRect(
  viewport: MarkupViewport,
  rect: PdfRect,
  dx: number,
  dy: number
): PdfRect {
  const box = toViewportBox(viewport, rect)
  return toPdfRect(viewport, { ...box, x: box.x + dx, y: box.y + dy })
}

/** Resizes a PDF rect from its bottom-right viewport corner. */
export function resizePdfRect(
  viewport: MarkupViewport,
  rect: PdfRect,
  dx: number,
  dy: number,
  minPx = 8
): PdfRect {
  const box = toViewportBox(viewport, rect)
  return toPdfRect(viewport, {
    ...box,
    width: Math.max(minPx, box.width + dx),
    height: Math.max(minPx, box.height + dy),
  })
}

/**
 * Selection rects (viewport pixels, relative to the page surface) merged
 * into one PDF rect per line, the way Zotero stores a multi-line highlight.
 */
export function selectionToPdfRects(viewport: MarkupViewport, boxes: ViewportBox[]): PdfRect[] {
  const visible = boxes.filter((box) => box.width > 0.5 && box.height > 0.5)
  const lines: ViewportBox[] = []
  for (const box of [...visible].sort((p, q) => p.y - q.y || p.x - q.x)) {
    const line = lines.find((candidate) => {
      const overlap =
        Math.min(candidate.y + candidate.height, box.y + box.height) - Math.max(candidate.y, box.y)
      return overlap > Math.min(candidate.height, box.height) * 0.5
    })
    if (!line) {
      lines.push({ ...box })
      continue
    }
    const right = Math.max(line.x + line.width, box.x + box.width)
    const bottom = Math.max(line.y + line.height, box.y + box.height)
    line.x = Math.min(line.x, box.x)
    line.y = Math.min(line.y, box.y)
    line.width = right - line.x
    line.height = bottom - line.y
  }
  return lines.map((line) => toPdfRect(viewport, line))
}

/** The rect enclosing every rect. */
export function boundingPdfRect(rects: PdfRect[]): PdfRect | null {
  if (rects.length === 0) return null
  return [
    Math.min(...rects.map((rect) => rect[0])),
    Math.min(...rects.map((rect) => rect[1])),
    Math.max(...rects.map((rect) => rect[2])),
    Math.max(...rects.map((rect) => rect[3])),
  ]
}

/** Height the selection popover needs below or above a selection, in px. */
export const POPOVER_SPACE_PX = 48

/**
 * Where the selection popover goes: centered on the whole selection, below
 * its last line, or above its first one when the page ends too soon.
 */
export function popoverAnchor(
  boxes: ViewportBox[],
  pageHeight: number
): { x: number; y: number; placement: 'below' | 'above' } | null {
  if (boxes.length === 0) return null
  const left = Math.min(...boxes.map((box) => box.x))
  const right = Math.max(...boxes.map((box) => box.x + box.width))
  const top = Math.min(...boxes.map((box) => box.y))
  const bottom = Math.max(...boxes.map((box) => box.y + box.height))
  const x = (left + right) / 2
  if (bottom + 8 + POPOVER_SPACE_PX <= pageHeight) return { x, y: bottom + 8, placement: 'below' }
  return { x, y: top - 8, placement: 'above' }
}
