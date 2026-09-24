import { describe, expect, it } from 'vitest'
import { buildDocumentCountLabel } from './document-count-label'

/**
 * The document card chip is worded by media (Colección → Documento →
 * Página): a PDF's pages are "páginas", a standalone image is "imagen(es)",
 * and an audio file is "audio(s)" — never a media-blind "N assets".
 */

// A translate stub that mirrors the real key -> template shape closely
// enough to assert on the chosen key and the interpolated count, without
// pulling in the whole i18n module.
function fakeT(key: string, params?: Record<string, unknown>): string {
  const count = params?.count
  return `${key}(${count})`
}

describe('buildDocumentCountLabel', () => {
  it('words a PDF document by its pages, singular', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 1, imageCount: 0, audioCount: 0 }, fakeT)
    expect(label).toBe('collection.pipelineCount.assets.one(1)')
  })

  it('words a PDF document by its pages, plural', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 3, imageCount: 0, audioCount: 0 }, fakeT)
    expect(label).toBe('collection.pipelineCount.assets.other(3)')
  })

  it('words an image document, singular', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 0, imageCount: 1, audioCount: 0 }, fakeT)
    expect(label).toBe('collection.pipelineCount.images.one(1)')
  })

  it('words an image document, plural', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 0, imageCount: 2, audioCount: 0 }, fakeT)
    expect(label).toBe('collection.pipelineCount.images.other(2)')
  })

  it('words an audio document, singular', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 0, imageCount: 0, audioCount: 1 }, fakeT)
    expect(label).toBe('collection.pipelineCount.audios.one(1)')
  })

  it('words an audio document, plural', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 0, imageCount: 0, audioCount: 4 }, fakeT)
    expect(label).toBe('collection.pipelineCount.audios.other(4)')
  })

  it('joins every present media when a document mixes them', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 2, imageCount: 0, audioCount: 1 }, fakeT)
    expect(label).toBe(
      'collection.pipelineCount.assets.other(2) · collection.pipelineCount.audios.one(1)'
    )
  })

  it('falls back to the zero page chip when no media is known yet', () => {
    const label = buildDocumentCountLabel({ pdfPageCount: 0, imageCount: 0, audioCount: 0 }, fakeT)
    expect(label).toBe('collection.pipelineCount.assets.other(0)')
  })
})
