/**
 * Which pages of the open document "OCR every page" sends to the queue.
 *
 * Each page then goes through the same admission a single page's OCRL or
 * OCRH button uses (`extract_text`), so the queue runs them one after another
 * and every page shows its own progress. What this decides is only which
 * pages: the ones with no text yet. A page with text is left alone — above
 * all one corrected by hand, which a bulk action must never overwrite without
 * the question the single-page button asks — and so is one already queued.
 *
 * A page whose state cannot be read is left out too: not knowing is no reason
 * to replace someone's text.
 */

export interface PageOcrProbe {
  hasText: (assetId: string) => Promise<boolean>
  editedByHand: (assetId: string) => Promise<boolean>
  inQueue: (assetId: string) => Promise<boolean>
}

export interface OcrAllPagesPlan<T> {
  /** The pages to send, in the document's order. */
  queue: T[]
  /** Pages left out: with text, corrected by hand, queued, or unreadable. */
  skipped: number
}

export async function planOcrAllPages<T extends { id: string; type: string }>(
  assets: T[],
  probe: PageOcrProbe
): Promise<OcrAllPagesPlan<T>> {
  // Audio has a transcription, not an OCR.
  const pages = assets.filter((asset) => asset.type !== 'audio')
  const needed = await Promise.all(
    pages.map(async (asset) => {
      try {
        const [text, edited, queued] = await Promise.all([
          probe.hasText(asset.id),
          probe.editedByHand(asset.id),
          probe.inQueue(asset.id),
        ])
        return !text && !edited && !queued
      } catch {
        return false
      }
    })
  )
  const queue = pages.filter((_, index) => needed[index])
  return { queue, skipped: pages.length - queue.length }
}
