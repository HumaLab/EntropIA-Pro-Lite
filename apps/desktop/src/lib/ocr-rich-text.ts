import MarkdownIt from 'markdown-it'

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

function renderNonOcrImage(tokens: MarkdownIt.Token[], index: number): string {
  const token = tokens[index]
  const alt = token?.content ?? ''
  const source = token?.attrGet('src') ?? ''
  return escapeHtml(`![${alt}](${source})`)
}

markdown.renderer.rules.image = (tokens, index) => {
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
