import type { MarkupViewport, PdfRect } from './types'
import { boundingPdfRect, toViewportBox } from './geometry'

/**
 * Previews of area and ink annotations for the annotations panel
 * (docs/zotero-annotations-plan.md, section 10): the region of the page,
 * rendered from the local PDF with pdf.js at thumbnail scale, ink strokes
 * drawn on top. Cached per annotation revision, with a bounded number of
 * renders at once.
 */

/** The parts of a pdf.js document and page a preview needs. */
export interface ThumbnailDocument {
  getPage(pageNumber: number): Promise<ThumbnailPage>
}

export interface ThumbnailPage {
  getViewport(options: { scale: number }): MarkupViewport
  render(options: {
    canvasContext: CanvasRenderingContext2D
    viewport: MarkupViewport
    transform?: number[]
  }): { promise: Promise<unknown> }
}

export interface ThumbnailRequest {
  /** Cache identity: the annotation key and its revision. */
  id: string
  pageIndex: number
  rects?: PdfRect[]
  paths?: number[][]
  color?: string
  width?: number
}

/** Longest side of a preview, in CSS pixels. */
export const THUMBNAIL_MAX_PX = 160

/** The region a preview shows: the rects, or the bounds of the strokes. */
export function thumbnailRegion(request: ThumbnailRequest): PdfRect | null {
  if (request.rects && request.rects.length > 0) return boundingPdfRect(request.rects)
  const rects: PdfRect[] = (request.paths ?? [])
    .filter((path) => path.length >= 2)
    .map((path) => {
      const xs = path.filter((_, index) => index % 2 === 0)
      const ys = path.filter((_, index) => index % 2 === 1)
      return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)]
    })
  const bounds = boundingPdfRect(rects)
  if (!bounds) return null
  // Strokes have a width: keep their edges inside the preview.
  const pad = (request.width ?? 2) * 2
  return [bounds[0] - pad, bounds[1] - pad, bounds[2] + pad, bounds[3] + pad]
}

/** Renders one preview as a PNG data URL; `null` when there is nothing to show. */
export async function renderThumbnail(
  doc: ThumbnailDocument,
  request: ThumbnailRequest,
  maxPx = THUMBNAIL_MAX_PX
): Promise<string | null> {
  const region = thumbnailRegion(request)
  if (!region) return null
  const page = await doc.getPage(request.pageIndex + 1)
  const natural = toViewportBox(page.getViewport({ scale: 1 }), region)
  const longest = Math.max(natural.width, natural.height)
  if (longest <= 0) return null
  const scale = Math.min(4, maxPx / longest)
  const viewport = page.getViewport({ scale })
  const box = toViewportBox(viewport, region)
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(box.width))
  canvas.height = Math.max(1, Math.round(box.height))
  const context = canvas.getContext('2d')
  if (!context) return null
  await page.render({ canvasContext: context, viewport, transform: [1, 0, 0, 1, -box.x, -box.y] })
    .promise
  if (request.paths?.length) {
    context.strokeStyle = request.color ?? '#000000'
    context.lineWidth = (request.width ?? 2) * scale
    context.lineCap = 'round'
    context.lineJoin = 'round'
    for (const path of request.paths) {
      context.beginPath()
      for (let index = 0; index + 1 < path.length; index += 2) {
        const [x = 0, y = 0] = viewport.convertToViewportPoint(
          path[index] ?? 0,
          path[index + 1] ?? 0
        )
        if (index === 0) context.moveTo(x - box.x, y - box.y)
        else context.lineTo(x - box.x, y - box.y)
      }
      context.stroke()
    }
  }
  return canvas.toDataURL('image/png')
}

/**
 * Previews of one document: cached by request id, at most `concurrency`
 * renders at once. A failed render resolves to `null` and is not cached.
 */
export class ThumbnailRenderer {
  private readonly cache = new Map<string, Promise<string | null>>()
  private readonly waiting: Array<() => void> = []
  private running = 0

  constructor(
    private readonly doc: ThumbnailDocument,
    private readonly concurrency = 2,
    private readonly render: typeof renderThumbnail = renderThumbnail
  ) {}

  thumbnail(request: ThumbnailRequest): Promise<string | null> {
    const cached = this.cache.get(request.id)
    if (cached) return cached
    const result = this.schedule(() => this.render(this.doc, request)).catch(() => {
      this.cache.delete(request.id)
      return null
    })
    this.cache.set(request.id, result)
    return result
  }

  private async schedule<T>(task: () => Promise<T>): Promise<T> {
    if (this.running >= this.concurrency) {
      await new Promise<void>((resolve) => this.waiting.push(resolve))
    }
    this.running++
    try {
      return await task()
    } finally {
      this.running--
      this.waiting.shift()?.()
    }
  }
}
