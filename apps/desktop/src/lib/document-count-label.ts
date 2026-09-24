/**
 * The document card chip, worded by media (Colección → Documento → Página):
 * a PDF's pages are "páginas", a standalone image is "imagen(es)", and an
 * audio file is "audio(s)" — never a media-blind "N assets".
 *
 * Under the current import flow a document holds assets of a single media
 * type (a PDF is always split into PDF-typed pages; an image or an audio
 * file each becomes its own single asset), so in practice exactly one count
 * here is non-zero. The mixed case is still handled exactly, by joining
 * every present media, rather than guessing at a "primary" one — see
 * odd/tasks/ui-terminology.md T3.
 */

export interface DocumentMediaCounts {
  pdfPageCount: number
  imageCount: number
  audioCount: number
}

type Translate = (key: string, params?: Record<string, string | number>) => string

export function buildDocumentCountLabel(counts: DocumentMediaCounts, t: Translate): string {
  const parts: string[] = []

  if (counts.pdfPageCount > 0) {
    parts.push(
      t(
        counts.pdfPageCount === 1
          ? 'collection.pipelineCount.assets.one'
          : 'collection.pipelineCount.assets.other',
        { count: counts.pdfPageCount }
      )
    )
  }
  if (counts.imageCount > 0) {
    parts.push(
      t(
        counts.imageCount === 1
          ? 'collection.pipelineCount.images.one'
          : 'collection.pipelineCount.images.other',
        { count: counts.imageCount }
      )
    )
  }
  if (counts.audioCount > 0) {
    parts.push(
      t(
        counts.audioCount === 1
          ? 'collection.pipelineCount.audios.one'
          : 'collection.pipelineCount.audios.other',
        { count: counts.audioCount }
      )
    )
  }

  if (parts.length > 0) return parts.join(' · ')

  // No known media yet (assets not loaded, or a freshly created document) —
  // the long-standing page chip, worded for zero, is the fallback.
  return t('collection.pipelineCount.assets.other', { count: 0 })
}
