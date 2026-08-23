import {
  escapeHtml,
  renderOcrMarkup,
  replaceOcrRegionPlaceholders,
  replaceOcrRegionReferences,
  resolveOcrRegion,
  sanitizeOcrHtml,
  type OcrRegionResolver,
  type OcrRenderContext,
} from './ocr-rich-text'

export type OcrExportFormat = 'markdown' | 'pdf' | 'docx'

export interface OcrExportInput extends OcrRenderContext {
  source: string
}

export interface PreparedOcrExport {
  markdown: string
  html: string
}

const OCR_EXPORT_FALLBACK_MARKDOWN = '*[Imagen OCR no disponible]*'
const OCR_EXPORT_FALLBACK_HTML = '<span>Imagen OCR no disponible</span>'
const OCR_EXPORT_IMAGE_SOURCE = /^data:image\/[a-z0-9.+-]+;base64,[a-z0-9+/]+=*$/i

export async function prepareOcrExport(
  input: OcrExportInput,
  resolveRegion: OcrRegionResolver = resolveOcrRegion
): Promise<PreparedOcrExport> {
  const normalizedSource = input.source.replace(/\r\n?/g, '\n')
  const markup = renderOcrMarkup(normalizedSource)
  const htmlReplacements = new Map<string, string>()
  const markdownReplacements = new Map<string, string>()

  await Promise.all(
    markup.references.map(async (reference) => {
      try {
        const dataUrl = await resolveRegion(reference, input)
        if (!OCR_EXPORT_IMAGE_SOURCE.test(dataUrl)) {
          throw new Error('OCR export resolver returned an unsafe image')
        }

        const alt = `OCR region from page ${reference.page + 1}`
        htmlReplacements.set(
          reference.token,
          `<img src="${escapeHtml(dataUrl)}" alt="${escapeHtml(alt)}" />`
        )
        markdownReplacements.set(reference.token, `![${alt}](${dataUrl})`)
      } catch {
        htmlReplacements.set(reference.token, OCR_EXPORT_FALLBACK_HTML)
        markdownReplacements.set(reference.token, OCR_EXPORT_FALLBACK_MARKDOWN)
      }
    })
  )

  const markdown = replaceOcrRegionReferences(normalizedSource, (reference) => {
    return markdownReplacements.get(reference.token) ?? OCR_EXPORT_FALLBACK_MARKDOWN
  })

  const html = sanitizeOcrHtml(
    replaceOcrRegionPlaceholders(
      markup.html,
      new Map(
        markup.references.map((reference) => [
          reference.token,
          htmlReplacements.get(reference.token) ?? OCR_EXPORT_FALLBACK_HTML,
        ])
      )
    )
  )

  return { markdown, html }
}
