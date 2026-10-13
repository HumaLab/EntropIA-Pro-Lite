const ALLOWED_TAGS = new Set([
  'a',
  'blockquote',
  'br',
  'code',
  'em',
  'h1',
  'h2',
  'h3',
  'i',
  'li',
  'ol',
  'p',
  'pre',
  'strong',
  'u',
  'ul',
])

const DROP_CONTENT_TAGS = new Set(['iframe', 'object', 'script', 'style'])
const PARAGRAPH_LIKE_TAGS = new Set(['article', 'div', 'section'])
const BLOCK_TAG_PATTERN = /<(?:p|h1|h2|h3|ul|ol|blockquote|pre|li)\b/i

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

function normalizeLineEndings(value: string): string {
  return value.replace(/\r\n?/g, '\n')
}

export function normalizeNoteLinkHref(href: string | null): string | null {
  if (!href) return null

  const trimmed = href.trim()
  if (!trimmed) return null

  if (trimmed.startsWith('#')) return trimmed

  const candidate = /^[a-zA-Z][a-zA-Z\d+.-]*:/.test(trimmed) ? trimmed : `https://${trimmed}`

  try {
    const url = new URL(candidate, 'https://entropia.local')
    const protocol = url.protocol.toLowerCase()

    if (
      protocol === 'http:' ||
      protocol === 'https:' ||
      protocol === 'mailto:' ||
      protocol === 'tel:'
    ) {
      return url.toString()
    }
  } catch {
    return null
  }

  return null
}

function appendSanitizedChildren(source: Element | DocumentFragment, target: Node) {
  for (const child of Array.from(source.childNodes)) {
    const sanitized = sanitizeNode(child)
    if (!sanitized) continue
    target.appendChild(sanitized)
  }
}

function sanitizeNode(node: Node): Node | null {
  const doc = node.ownerDocument ?? document

  if (node.nodeType === Node.TEXT_NODE) {
    return doc.createTextNode(node.textContent ?? '')
  }

  if (node.nodeType !== Node.ELEMENT_NODE) {
    return null
  }

  const element = node as Element
  const tag = element.tagName.toLowerCase()

  if (DROP_CONTENT_TAGS.has(tag)) {
    return null
  }

  const normalizedTag = PARAGRAPH_LIKE_TAGS.has(tag) ? 'p' : tag

  if (!ALLOWED_TAGS.has(normalizedTag)) {
    const fragment = doc.createDocumentFragment()
    appendSanitizedChildren(element, fragment)
    return fragment
  }

  const clean = doc.createElement(normalizedTag)

  if (normalizedTag === 'a') {
    const href = normalizeNoteLinkHref(element.getAttribute('href'))
    if (!href) {
      const fragment = doc.createDocumentFragment()
      appendSanitizedChildren(element, fragment)
      return fragment
    }

    clean.setAttribute('href', href)
    clean.setAttribute('target', '_blank')
    clean.setAttribute('rel', 'noopener noreferrer nofollow')
  }

  appendSanitizedChildren(element, clean)

  if (normalizedTag !== 'br' && !clean.textContent?.trim() && clean.children.length === 0) {
    return null
  }

  return clean
}

function wrapInlineOnlyHtml(html: string): string {
  const trimmed = html.trim()
  if (!trimmed) return ''
  if (BLOCK_TAG_PATTERN.test(trimmed)) return trimmed
  return `<p>${trimmed}</p>`
}

function createFragmentFromHtml(html: string): DocumentFragment | null {
  if (typeof document === 'undefined') return null

  const template = document.createElement('template')
  template.innerHTML = html
  return template.content
}

export function isLegacyPlainTextNoteContent(content: string | null | undefined): boolean {
  if (!content) return true

  const trimmed = content.trim()
  if (!trimmed) return true

  return !/<\/?[a-z][\w:-]*\b[^>]*>/i.test(trimmed)
}

export function convertLegacyNoteTextToHtml(content: string | null | undefined): string {
  const normalized = normalizeLineEndings(content ?? '').trim()
  if (!normalized) return ''

  return normalized
    .split(/\n{2,}/)
    .map((paragraph) => `<p>${escapeHtml(paragraph).replace(/\n/g, '<br>')}</p>`)
    .join('')
}

export function sanitizeNoteHtml(content: string | null | undefined): string {
  const raw = content?.trim() ?? ''
  if (!raw) return ''

  // Without a DOM the allowlist below cannot run, so markup is shown as the
  // text it is: escaped and paragraphed, never passed through raw.
  if (typeof document === 'undefined') return convertLegacyNoteTextToHtml(raw)

  const source = createFragmentFromHtml(raw)
  if (!source) return ''

  const container = document.createElement('div')
  appendSanitizedChildren(source, container)
  return wrapInlineOnlyHtml(container.innerHTML)
}

export function normalizeNoteContentForEditor(content: string | null | undefined): string {
  const normalized = isLegacyPlainTextNoteContent(content)
    ? convertLegacyNoteTextToHtml(content)
    : sanitizeNoteHtml(content)

  return normalized || '<p></p>'
}

export function normalizeNoteContentForRender(content: string | null | undefined): string {
  if (!content?.trim()) return ''

  return isLegacyPlainTextNoteContent(content)
    ? convertLegacyNoteTextToHtml(content)
    : sanitizeNoteHtml(content)
}

interface NoteEditorChangeOptions {
  originalContent: string | null | undefined
  currentContent: string | null | undefined
}

interface NoteEditorSaveStateOptions extends NoteEditorChangeOptions {
  isEditing: boolean
}

export function hasNoteEditorMeaningfulChanges({
  originalContent,
  currentContent,
}: NoteEditorChangeOptions): boolean {
  return (
    normalizeNoteContentForRender(originalContent) !== normalizeNoteContentForRender(currentContent)
  )
}

export function shouldDisableNoteEditorSave({
  currentContent,
  originalContent,
  isEditing,
}: NoteEditorSaveStateOptions): boolean {
  if (isNoteHtmlEffectivelyEmpty(currentContent)) {
    return true
  }

  if (!isEditing) {
    return false
  }

  return !hasNoteEditorMeaningfulChanges({ originalContent, currentContent })
}

export function isNoteHtmlEffectivelyEmpty(content: string | null | undefined): boolean {
  const normalized = normalizeNoteContentForRender(content)
  if (!normalized) return true

  if (typeof document === 'undefined') {
    return normalized.replace(/<[^>]+>/g, '').trim().length === 0
  }

  const container = document.createElement('div')
  container.innerHTML = normalized
  return (container.textContent ?? '').trim().length === 0
}
