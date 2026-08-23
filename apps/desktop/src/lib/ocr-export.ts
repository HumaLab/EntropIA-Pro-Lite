import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import htmlDocxBundleUrl from 'html-docx-js/dist/html-docx.js?url'

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

export interface OcrExportGenerators {
  pdf: (html: string) => Promise<Uint8Array>
  docx: (html: string) => Promise<Uint8Array>
}

export interface OcrExportRuntime {
  resolveRegion?: OcrRegionResolver
  generators?: Partial<OcrExportGenerators>
}

interface HtmlDocxBrowserApi {
  asBlob(html: string, options?: Record<string, unknown>): Blob
}

const OCR_EXPORT_FALLBACK_MARKDOWN = '*[Imagen OCR no disponible]*'
const OCR_EXPORT_FALLBACK_HTML = '<span>Imagen OCR no disponible</span>'
const OCR_EXPORT_IMAGE_SOURCE = /^data:image\/[a-z0-9.+-]+;base64,[a-z0-9+/]+=*$/i

const OCR_EXPORT_STYLES = `
  :root { color-scheme: light; }
  body { margin: 0; color: #1f2937; background: #ffffff; font: 11pt/1.55 system-ui, sans-serif; }
  h1, h2, h3, h4, h5, h6 { color: #111827; line-height: 1.2; break-after: avoid; }
  p, ul, ol, blockquote, pre, table { margin: 0 0 12pt; }
  ul, ol { padding-inline-start: 24pt; }
  blockquote { border-inline-start: 2pt solid #9ca3af; padding-inline-start: 10pt; color: #4b5563; }
  table { width: 100%; border-collapse: collapse; break-inside: avoid; }
  th, td { border: 0.5pt solid #9ca3af; padding: 5pt; vertical-align: top; }
  pre { padding: 8pt; background: #f3f4f6; white-space: pre-wrap; break-inside: avoid; }
  img { display: block; max-width: 100%; height: auto; break-inside: avoid; }
`

const EXPORT_OPTIONS = {
  markdown: { name: 'Markdown', extension: 'md' },
  pdf: { name: 'PDF', extension: 'pdf' },
  docx: { name: 'Microsoft Word', extension: 'docx' },
} as const

let htmlDocxBundlePromise: Promise<HtmlDocxBrowserApi> | null = null

function buildExportHtml(document: PreparedOcrExport): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>${OCR_EXPORT_STYLES}</style></head><body>${document.html}</body></html>`
}

async function generatePdfBytes(html: string): Promise<Uint8Array> {
  const { default: html2pdf } = await import('html2pdf.js')
  const container = document.createElement('div')
  container.style.position = 'fixed'
  container.style.insetInlineStart = '-100000px'
  container.style.insetBlockStart = '0'
  container.style.width = '180mm'
  container.innerHTML = html
  document.body.append(container)

  try {
    const output = await html2pdf()
      .set({
        margin: [12, 12, 12, 12],
        image: { type: 'png', quality: 1 },
        pagebreak: { mode: ['css', 'legacy'], avoid: ['img', 'tr', 'pre'] },
        html2canvas: { scale: 2, useCORS: false, backgroundColor: '#ffffff' },
        jsPDF: { unit: 'mm', format: 'a4', orientation: 'portrait', compressPDF: true },
      })
      .from(container)
      .outputPdf('arraybuffer')

    return new Uint8Array(output)
  } finally {
    container.remove()
  }
}

async function loadHtmlDocxBrowserApi(): Promise<HtmlDocxBrowserApi> {
  if (typeof window !== 'undefined' && window.htmlDocx?.asBlob) {
    return window.htmlDocx
  }

  if (!htmlDocxBundlePromise) {
    htmlDocxBundlePromise = fetch(htmlDocxBundleUrl)
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(`Failed to load html-docx-js browser bundle: ${response.status}`)
        }

        const source = await response.text()
        new Function(source)()

        const api = window.htmlDocx
        if (!api?.asBlob) {
          throw new Error('html-docx-js browser bundle did not expose window.htmlDocx')
        }

        return api
      })
  }

  return htmlDocxBundlePromise
}

async function generateDocxBytes(html: string): Promise<Uint8Array> {
  const { asBlob } = await loadHtmlDocxBrowserApi()
  const blob = asBlob(html, {
    orientation: 'portrait',
    margins: { top: 720, right: 720, bottom: 720, left: 720 },
  })
  return new Uint8Array(await blob.arrayBuffer())
}

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

export async function generateOcrExportBytes(
  format: OcrExportFormat,
  document: PreparedOcrExport,
  generators: Partial<OcrExportGenerators> = {}
): Promise<Uint8Array> {
  if (format === 'markdown') {
    return new TextEncoder().encode(document.markdown)
  }

  const html = buildExportHtml(document)

  if (format === 'pdf') {
    return (generators.pdf ?? generatePdfBytes)(html)
  }

  return (generators.docx ?? generateDocxBytes)(html)
}

export async function exportOcrText(
  input: OcrExportInput,
  format: OcrExportFormat,
  defaultName: string,
  runtime: OcrExportRuntime = {}
): Promise<string | null> {
  const option = EXPORT_OPTIONS[format]
  const filePath = await save({
    defaultPath: defaultName,
    filters: [{ name: option.name, extensions: [option.extension] }],
  })

  if (!filePath) {
    return null
  }

  const document = await prepareOcrExport(input, runtime.resolveRegion)
  const bytes = await generateOcrExportBytes(format, document, runtime.generators)
  await writeFile(filePath, bytes)

  return filePath
}
