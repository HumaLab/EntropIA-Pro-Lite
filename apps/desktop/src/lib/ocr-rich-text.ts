import MarkdownIt from 'markdown-it'
import type { PDFDocumentProxy } from 'pdfjs-dist'

const OCR_REGION_DESTINATION = 'ocr-region:'
const OCR_REGION_MARKDOWN = /!\[\]\(\s*([^)]*)\)/gi
const OCR_REGION_VALUE = /^page\s*=\s*(\d+)\s*,\s*bbox\s*=\s*\[\s*([^,\]]+)\s*,\s*([^,\]]+)\s*,\s*([^,\]]+)\s*,\s*([^\]]+)\s*\]$/i
const OCR_REGION_PLACEHOLDER =
  /<span\s+data-ocr-region-token="([^"]+)"(?:\s+aria-hidden="true")?\s*><\/span>/g
const NUMERIC_VALUE = /^[-+]?(?:\d+(?:\.\d*)?|\.\d+)$/

const ALLOWED_TAGS: Record<string, true> = {
  a: true,
  b: true,
  blockquote: true,
  br: true,
  caption: true,
  code: true,
  div: true,
  em: true,
  h1: true,
  h2: true,
  h3: true,
  h4: true,
  h5: true,
  h6: true,
  i: true,
  img: true,
  li: true,
  ol: true,
  p: true,
  pre: true,
  section: true,
  strong: true,
  table: true,
  tbody: true,
  td: true,
  tfoot: true,
  th: true,
  thead: true,
  tr: true,
  u: true,
  ul: true,
}

const DROP_CONTENT_TAGS: Record<string, true> = {
  embed: true,
  form: true,
  iframe: true,
  object: true,
  script: true,
  style: true,
}
const SAFE_HREF_PROTOCOLS: Record<string, true> = {
  'http:': true,
  'https:': true,
  'mailto:': true,
  'tel:': true,
}
const SAFE_SCOPE_VALUES: Record<string, true> = {
  col: true,
  colgroup: true,
  row: true,
  rowgroup: true,
}
const SAFE_IMAGE_SOURCE = /^data:image\/[a-z0-9.+-]+;base64,[a-z0-9+/]+=*$/i
const MAX_SPAN = 100

export type OcrSourceType = 'image' | 'pdf'

export interface OcrBbox {
  left: number
  top: number
  right: number
  bottom: number
}

export interface OcrRegionReference {
  token: string
  source: string
  page: number
  bbox: OcrBbox
}

export interface OcrRenderContext {
  assetUrl: string
  sourceType: OcrSourceType
  referenceWidth: number
  referenceHeight: number
}

export type OcrRegionResolver = (
  reference: OcrRegionReference,
  context: OcrRenderContext
) => Promise<string>

export interface OcrMarkup {
  html: string
  references: OcrRegionReference[]
}

const markdown = new MarkdownIt({
  breaks: true,
  html: true,
  linkify: false,
  typographer: false,
}).enable('table')

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

function isFiniteNumeric(value: string): boolean {
  return NUMERIC_VALUE.test(value.trim()) && Number.isFinite(Number(value))
}

function isValidBbox(bbox: OcrBbox): boolean {
  return (
    Number.isFinite(bbox.left) &&
    Number.isFinite(bbox.top) &&
    Number.isFinite(bbox.right) &&
    Number.isFinite(bbox.bottom) &&
    bbox.left >= 0 &&
    bbox.top >= 0 &&
    bbox.right > bbox.left &&
    bbox.bottom > bbox.top
  )
}

export function parseOcrRegionReference(source: string): Omit<OcrRegionReference, 'token'> | null {
  const match = source.trim().match(OCR_REGION_VALUE)
  if (!match) return null

  const [, pageValue, leftValue, topValue, rightValue, bottomValue] = match
  const values = [leftValue, topValue, rightValue, bottomValue]
  if (!values.every((value) => value !== undefined && isFiniteNumeric(value))) return null

  const page = Number(pageValue)
  const bbox: OcrBbox = {
    left: Number(leftValue),
    top: Number(topValue),
    right: Number(rightValue),
    bottom: Number(bottomValue),
  }

  if (!Number.isInteger(page) || page < 0 || !isValidBbox(bbox)) return null

  return { source, page, bbox }
}

function protectOcrRegionReferences(source: string): {
  protectedSource: string
  references: OcrRegionReference[]
} {
  const references: OcrRegionReference[] = []
  const protectedSource = source.replace(OCR_REGION_MARKDOWN, (whole, region: string) => {
    const parsed = parseOcrRegionReference(region)
    if (parsed) {
      const token = `region-${references.length}`
      references.push({ ...parsed, source: whole, token })
      return `![](${OCR_REGION_DESTINATION}${token})`
    }

    return /^page\s*=/i.test(region.trim()) ? whole.replace('![]', '!&#91;&#93;') : whole
  })

  return { protectedSource, references }
}

interface MarkdownImageToken {
  content: string
  attrGet(name: string): string | null
}

function renderNonOcrImage(tokens: MarkdownImageToken[], index: number): string {
  const token = tokens[index]
  const alt = token?.content ?? ''
  const source = token?.attrGet('src') ?? ''
  return escapeHtml(`![${alt}](${source})`)
}

markdown.renderer.rules.image = (tokens: MarkdownImageToken[], index: number) => {
  const token = tokens[index]
  const source = token?.attrGet('src') ?? ''
  if (source.startsWith(OCR_REGION_DESTINATION)) {
    const tokenValue = source.slice(OCR_REGION_DESTINATION.length)
    return `<span data-ocr-region-token="${escapeHtml(tokenValue)}" aria-hidden="true"></span>`
  }

  return renderNonOcrImage(tokens, index)
}

export function renderOcrMarkup(source: string): OcrMarkup {
  const normalized = source.replace(/\r\n?/g, '\n')
  if (!normalized) return { html: '', references: [] }

  const { protectedSource, references } = protectOcrRegionReferences(normalized)
  return { html: markdown.render(protectedSource), references }
}

function normalizeSafeHref(value: string): string | null {
  const trimmed = value.trim()
  if (!trimmed) return null
  if (trimmed.startsWith('#')) return trimmed

  const candidate = /^[a-z][a-z\d+.-]*:/i.test(trimmed) ? trimmed : `https://${trimmed}`
  try {
    const url = new URL(candidate, 'https://entropia.local')
    return SAFE_HREF_PROTOCOLS[url.protocol.toLowerCase()] ? url.toString() : null
  } catch {
    return null
  }
}

function appendSanitizedChildren(source: Element | DocumentFragment, target: Node): void {
  for (const child of Array.from(source.childNodes)) {
    const sanitized = sanitizeNode(child)
    if (sanitized) target.appendChild(sanitized)
  }
}


function copyBoundedSpanAttribute(source: Element, target: Element, name: string): void {
  const rawValue = source.getAttribute(name)
  if (!rawValue) return

  const value = Number(rawValue)
  if (Number.isInteger(value) && value > 0 && value <= MAX_SPAN) {
    target.setAttribute(name, String(value))
  }
}

function sanitizeNode(node: Node): Node | null {
  const document = node.ownerDocument ?? globalThis.document
  if (!document) return null

  if (node.nodeType === Node.TEXT_NODE) {
    return document.createTextNode(node.textContent ?? '')
  }

  if (node.nodeType !== Node.ELEMENT_NODE) return null

  const element = node as Element
  const tag = element.tagName.toLowerCase()
  if (DROP_CONTENT_TAGS[tag]) return null

  if (!ALLOWED_TAGS[tag]) {
    const fragment = document.createDocumentFragment()
    appendSanitizedChildren(element, fragment)
    return fragment
  }

  const clean = document.createElement(tag)

  if (tag === 'a') {
    const href = normalizeSafeHref(element.getAttribute('href') ?? '')
    if (!href) {
      const fragment = document.createDocumentFragment()
      appendSanitizedChildren(element, fragment)
      return fragment
    }
    clean.setAttribute('href', href)
    clean.setAttribute('target', '_blank')
    clean.setAttribute('rel', 'noopener noreferrer nofollow')
  }

  if (tag === 'div') {
    const align = element.getAttribute('align')?.toLowerCase()
    if (align === 'left' || align === 'center' || align === 'right') {
      clean.setAttribute('align', align)
    }
  }

  if (tag === 'th') {
    const scope = element.getAttribute('scope')?.toLowerCase()
    if (scope && SAFE_SCOPE_VALUES[scope]) clean.setAttribute('scope', scope)
    copyBoundedSpanAttribute(element, clean, 'colspan')
    copyBoundedSpanAttribute(element, clean, 'rowspan')
  }

  if (tag === 'td') {
    copyBoundedSpanAttribute(element, clean, 'colspan')
    copyBoundedSpanAttribute(element, clean, 'rowspan')
  }

  if (tag === 'img') {
    const source = element.getAttribute('src')?.trim() ?? ''
    if (!SAFE_IMAGE_SOURCE.test(source)) return null
    clean.setAttribute('src', source)
    clean.setAttribute('alt', element.getAttribute('alt')?.trim() ?? '')
  }

  appendSanitizedChildren(element, clean)
  return clean
}

export function sanitizeOcrHtml(html: string): string {
  if (!html) return ''
  if (typeof document === 'undefined') return escapeHtml(html)

  const template = document.createElement('template')
  template.innerHTML = html
  const container = document.createElement('div')
  appendSanitizedChildren(template.content, container)
  return container.innerHTML
}

export function replaceOcrRegionPlaceholders(
  html: string,
  replacements: ReadonlyMap<string, string>
): string {
  return html.replace(OCR_REGION_PLACEHOLDER, (_whole, token: string) => replacements.get(token) ?? '')
}

export { escapeHtml }

export interface OcrCropRect {
  left: number
  top: number
  width: number
  height: number
}

interface OcrRaster {
  canvas: HTMLCanvasElement
  width: number
  height: number
}

const imageRasterCache = new Map<string, Promise<OcrRaster>>()
const pdfDocumentCache = new Map<string, Promise<PDFDocumentProxy>>()
const pdfRasterCache = new Map<string, Promise<OcrRaster>>()

export function scaleOcrBbox(
  bbox: OcrBbox,
  referenceWidth: number,
  referenceHeight: number,
  sourceWidth: number,
  sourceHeight: number
): OcrCropRect {
  if (
    ![referenceWidth, referenceHeight, sourceWidth, sourceHeight].every(Number.isFinite) ||
    referenceWidth <= 0 ||
    referenceHeight <= 0 ||
    sourceWidth <= 0 ||
    sourceHeight <= 0
  ) {
    throw new Error('OCR crop reference dimensions must be positive')
  }

  if (
    !isValidBbox(bbox) ||
    bbox.right > referenceWidth ||
    bbox.bottom > referenceHeight
  ) {
    throw new Error('OCR crop bbox is outside reference bounds')
  }

  const scaleX = sourceWidth / referenceWidth
  const scaleY = sourceHeight / referenceHeight
  const left = Math.floor(bbox.left * scaleX)
  const top = Math.floor(bbox.top * scaleY)
  const right = Math.ceil(bbox.right * scaleX)
  const bottom = Math.ceil(bbox.bottom * scaleY)

  if (left < 0 || top < 0 || right > sourceWidth || bottom > sourceHeight) {
    throw new Error('OCR crop bbox is outside source bounds')
  }

  const width = right - left
  const height = bottom - top
  if (width <= 0 || height <= 0) {
    throw new Error('OCR crop bbox is smaller than one source pixel')
  }

  return { left, top, width, height }
}

function createCanvas(width: number, height: number): HTMLCanvasElement {
  if (typeof document === 'undefined') {
    throw new Error('OCR image crops require a browser document')
  }

  const canvas = document.createElement('canvas')
  canvas.width = Math.ceil(width)
  canvas.height = Math.ceil(height)
  return canvas
}

async function loadImageRaster(assetUrl: string): Promise<OcrRaster> {
  const cached = imageRasterCache.get(assetUrl)
  if (cached) return cached

  const pending = (async () => {
    if (typeof Image === 'undefined') {
      throw new Error('Image loading is unavailable')
    }

    const image = new Image()
    image.decoding = 'async'
    image.src = assetUrl

    if (typeof image.decode === 'function') {
      await image.decode()
    } else {
      await new Promise<void>((resolve, reject) => {
        image.onload = () => resolve()
        image.onerror = () => reject(new Error('OCR source image failed to load'))
      })
    }

    const width = image.naturalWidth || image.width
    const height = image.naturalHeight || image.height
    if (width <= 0 || height <= 0) {
      throw new Error('OCR source image has no dimensions')
    }

    const canvas = createCanvas(width, height)
    const context = canvas.getContext('2d')
    if (!context) throw new Error('OCR crop canvas is unavailable')
    context.drawImage(image, 0, 0, width, height)
    return { canvas, width, height }
  })()

  imageRasterCache.set(assetUrl, pending)
  return pending
}

async function loadPdfDocument(assetUrl: string): Promise<PDFDocumentProxy> {
  const cached = pdfDocumentCache.get(assetUrl)
  if (cached) return cached

  // PDF.js must load only for PDF regions; static loading selects its Node legacy build in Vitest.
  const pdfjs = await import('pdfjs-dist')
  pdfjs.GlobalWorkerOptions.workerSrc = new URL(
    'pdfjs-dist/build/pdf.worker.min.mjs',
    import.meta.url
  ).href
  const pending = pdfjs.getDocument(assetUrl).promise

  pdfDocumentCache.set(assetUrl, pending)
  return pending
}

async function loadPdfRaster(
  assetUrl: string,
  pageIndex: number,
  referenceWidth: number,
  referenceHeight: number
): Promise<OcrRaster> {
  const cacheKey = `${assetUrl}#${pageIndex}:${referenceWidth}:${referenceHeight}`
  const cached = pdfRasterCache.get(cacheKey)
  if (cached) return cached

  const pending = (async () => {
    const pdfDocument = await loadPdfDocument(assetUrl)
    const page = await pdfDocument.getPage(pageIndex + 1)
    const naturalViewport = page.getViewport({ scale: 1 })
    const scale =
      referenceWidth > 0
        ? referenceWidth / naturalViewport.width
        : referenceHeight > 0
          ? referenceHeight / naturalViewport.height
          : 1
    const viewport = page.getViewport({ scale })
    const canvas = createCanvas(viewport.width, viewport.height)
    const context = canvas.getContext('2d')
    if (!context) throw new Error('OCR PDF crop canvas is unavailable')
    await page.render({ canvasContext: context, viewport }).promise
    return { canvas, width: canvas.width, height: canvas.height }
  })()

  pdfRasterCache.set(cacheKey, pending)
  return pending
}

function cropRaster(
  raster: OcrRaster,
  bbox: OcrBbox,
  referenceWidth: number,
  referenceHeight: number
): string {
  const crop = scaleOcrBbox(
    bbox,
    referenceWidth,
    referenceHeight,
    raster.width,
    raster.height
  )
  const canvas = createCanvas(crop.width, crop.height)
  const context = canvas.getContext('2d')
  if (!context) throw new Error('OCR crop canvas is unavailable')

  context.drawImage(
    raster.canvas,
    crop.left,
    crop.top,
    crop.width,
    crop.height,
    0,
    0,
    crop.width,
    crop.height
  )

  const dataUrl = canvas.toDataURL('image/png')
  if (!SAFE_IMAGE_SOURCE.test(dataUrl)) {
    throw new Error('OCR crop canvas returned an invalid image')
  }
  return dataUrl
}

export async function resolveOcrRegion(
  reference: OcrRegionReference,
  context: OcrRenderContext
): Promise<string> {
  if (!Number.isInteger(reference.page) || reference.page < 0) {
    throw new Error('OCR region page must be a non-negative integer')
  }

  if (context.sourceType === 'image') {
    const raster = await loadImageRaster(context.assetUrl)
    const referenceWidth = context.referenceWidth > 0 ? context.referenceWidth : raster.width
    const referenceHeight =
      context.referenceHeight > 0 ? context.referenceHeight : raster.height
    return cropRaster(raster, reference.bbox, referenceWidth, referenceHeight)
  }

  const raster = await loadPdfRaster(
    context.assetUrl,
    reference.page,
    context.referenceWidth,
    context.referenceHeight
  )
  const referenceWidth = context.referenceWidth > 0 ? context.referenceWidth : raster.width
  const referenceHeight =
    context.referenceHeight > 0 ? context.referenceHeight : raster.height
  return cropRaster(raster, reference.bbox, referenceWidth, referenceHeight)
}

export async function renderOcrHtml(
  source: string,
  context: OcrRenderContext,
  resolveRegion: OcrRegionResolver = resolveOcrRegion
): Promise<string> {
  const markup = renderOcrMarkup(source)
  const replacements = new Map<string, string>()

  await Promise.all(
    markup.references.map(async (reference) => {
      try {
        const dataUrl = await resolveRegion(reference, context)
        if (!SAFE_IMAGE_SOURCE.test(dataUrl)) {
          throw new Error('OCR region resolver returned an unsafe image')
        }
        replacements.set(
          reference.token,
          `<img src="${escapeHtml(dataUrl)}" alt="OCR region from page ${reference.page + 1}" />`
        )
      } catch {
        replacements.set(reference.token, escapeHtml(reference.source))
      }
    })
  )

  return sanitizeOcrHtml(replaceOcrRegionPlaceholders(markup.html, replacements))
}
