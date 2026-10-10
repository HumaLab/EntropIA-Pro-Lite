import { toDocx } from './export-docx'
import type { ExportContext, ExportImage, Node as DocNode } from './export-document'
import { imageSize } from './image-dimensions'

/**
 * The OCR HTML as the DOCX object model (audit D-01, plan-editor.md §17.1).
 *
 * The OCR export used to hand its printable HTML to `html-docx-js`, which
 * wrapped it in an `altChunk`: the file carried no document model, so Word —
 * not the exporter — decided what the reader saw, and nothing inside the file
 * could be asserted. The vocabulary is bounded (`ALLOWED_TAGS`,
 * ocr-rich-text.ts), so it is read here into the same `Node` tree the
 * manuscript exporter takes, and built with the same `docx` package.
 *
 * The conversion is one-way and lossless against that vocabulary: a construct
 * the HTML may carry but the model cannot express is dropped, never guessed
 * at. Images are the one thing that needs a second channel: a `.docx` is a zip
 * with its own media, so each `data:` URL is decoded into an `ExportImage` and
 * the `writingImage` node refers to it by id.
 */

const TEXT_NODE = 3
const ELEMENT_NODE = 1

/** Tags that carry no document text, in or out of the sanitizer's vocabulary. */
const IGNORED_TAGS: Record<string, true> = {
  embed: true,
  form: true,
  head: true,
  iframe: true,
  link: true,
  meta: true,
  noscript: true,
  object: true,
  script: true,
  style: true,
  template: true,
  title: true,
}

/** The vocabulary's inline elements (`ALLOWED_TAGS`). Everything else inline
 *  is a wrapper the conversion walks through. */
const INLINE_TAGS: Record<string, true> = {
  a: true,
  b: true,
  br: true,
  code: true,
  em: true,
  i: true,
  img: true,
  span: true,
  strong: true,
  u: true,
}

/** The vocabulary's block elements. `div` and `section` are listed because
 *  they open a block context, but the flow treats them as transparent. */
const BLOCK_TAGS: Record<string, true> = {
  blockquote: true,
  caption: true,
  div: true,
  h1: true,
  h2: true,
  h3: true,
  h4: true,
  h5: true,
  h6: true,
  li: true,
  ol: true,
  p: true,
  pre: true,
  section: true,
  table: true,
  tbody: true,
  tfoot: true,
  thead: true,
  td: true,
  th: true,
  tr: true,
  ul: true,
}

const MARK_TAGS: Record<string, string> = {
  b: 'bold',
  strong: 'bold',
  em: 'italic',
  i: 'italic',
  u: 'underline',
  code: 'code',
}

/** The page margins the printable HTML used, in twips: half an inch. */
const OCR_PAGE_MARGINS_TWIPS = 720

const DATA_IMAGE = /^data:(image\/[a-z0-9.+-]+);base64,([a-z0-9+/=\s]+)$/i

type Mark = { type: string; attrs?: Record<string, unknown> }
type Wrap = (content: DocNode[]) => DocNode[]

interface ImageStore {
  images: Record<string, ExportImage>
  take(element: Element): DocNode | null
}

/**
 * A `data:image/...;base64,...` URL as bytes, or null when it is not one or
 * `atob` refuses it. The sanitizer and the export preparer already bound the
 * shape (ocr-rich-text.ts, ocr-export.ts); this is the gate for any other
 * caller of `generateDocxBytes`.
 */
function decodeDataImage(source: string): { mediaType: string; bytes: Uint8Array } | null {
  const match = DATA_IMAGE.exec(source)
  if (!match) return null

  try {
    const binary = atob(match[2]!.replace(/\s+/g, ''))
    const bytes = new Uint8Array(binary.length)
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index)
    }
    return { mediaType: match[1]!.toLowerCase(), bytes }
  } catch {
    return null
  }
}

function createImageStore(): ImageStore {
  const images: Record<string, ExportImage> = {}
  let count = 0

  return {
    images,
    take(element) {
      const source = element.getAttribute('src')?.trim() ?? ''
      const decoded = decodeDataImage(source)
      if (!decoded) return null

      const id = `ocr-image-${count++}`
      const size = imageSize(decoded.bytes)
      images[id] = {
        bytes: decoded.bytes,
        mediaType: decoded.mediaType,
        dataUrl: source,
        width: size?.width ?? 0,
        height: size?.height ?? 0,
      }

      return {
        type: 'writingImage',
        attrs: {
          src: id,
          alt: element.getAttribute('alt')?.trim() ?? '',
          title: null,
          width: size?.width ?? null,
          height: size?.height ?? null,
          align: 'center',
        },
        content: [],
      }
    },
  }
}

/** HTML collapses runs of spaces and newlines to one; so does Word. */
function collapseWhitespace(value: string): string {
  return value.replace(/[\t\n\r ]+/g, ' ')
}

/** One text node, with the marks its ancestors put on it. */
function textNode(text: string, marks: Mark[]): DocNode {
  return marks.length === 0 ? { type: 'text', text } : { type: 'text', text, marks: marks.slice() }
}

/**
 * Leading and trailing spaces are not content the page showed, and a space
 * right after a break would indent the line Word starts. The runs are fresh
 * objects, so trimming them here cannot touch the caller's.
 */
function tidyRuns(runs: DocNode[]): DocNode[] {
  const out: DocNode[] = []
  let atLineStart = true

  for (const run of runs) {
    if (run.type === 'text') {
      const text = atLineStart ? (run.text ?? '').replace(/^ +/, '') : (run.text ?? '')
      if (text === '') continue
      out.push({ ...run, text })
      atLineStart = false
      continue
    }

    if (run.type === 'hardBreak') {
      const previous = out[out.length - 1]
      if (previous?.type === 'text') previous.text = (previous.text ?? '').replace(/ +$/, '')
      out.push(run)
      atLineStart = true
      continue
    }

    out.push(run)
    atLineStart = false
  }

  const last = out[out.length - 1]
  if (last?.type === 'text') last.text = (last.text ?? '').replace(/ +$/, '')

  return out.filter((run) => run.type !== 'text' || (run.text ?? '') !== '')
}

/**
 * The inline content of an element, appended to `out`. An image is not inline
 * in the model, so it is handed to `emitImage`, which closes the run before it
 * and opens a new one after — what the page showed as words, picture, words.
 */
function collectInline(
  element: Element,
  marks: Mark[],
  out: DocNode[],
  emitImage: (element: Element) => void
): void {
  for (const child of Array.from(element.childNodes)) {
    if (child.nodeType === TEXT_NODE) {
      const value = collapseWhitespace(child.textContent ?? '')
      if (value !== '') out.push(textNode(value, marks))
      continue
    }
    if (child.nodeType !== ELEMENT_NODE) continue
    collectInlineElement(child as Element, marks, out, emitImage)
  }
}

/** One inline element: its own mark, if it carries one, plus its content. */
function collectInlineElement(
  element: Element,
  marks: Mark[],
  out: DocNode[],
  emitImage: (element: Element) => void
): void {
  const tag = element.tagName.toLowerCase()
  if (IGNORED_TAGS[tag]) return

  if (tag === 'br') {
    out.push({ type: 'hardBreak' })
    return
  }
  if (tag === 'img') {
    emitImage(element)
    return
  }
  if (tag === 'a') {
    const href = element.getAttribute('href')?.trim() ?? ''
    collectInline(
      element,
      href === '' ? marks : [...marks, { type: 'link', attrs: { href } }],
      out,
      emitImage
    )
    return
  }

  const mark = MARK_TAGS[tag]
  collectInline(element, mark === undefined ? marks : [...marks, { type: mark }], out, emitImage)
}

const paragraphWrap: Wrap = (content) => [{ type: 'paragraph', content }]

function headingWrap(level: number): Wrap {
  return (content) => [{ type: 'heading', attrs: { level }, content }]
}

function emptyParagraph(): DocNode {
  return { type: 'paragraph', content: [] }
}

/**
 * A run of block content. Loose inline text becomes a paragraph through
 * `wrap`; a block element closes the run and contributes its own blocks; a
 * `div` or `section` is walked through without costing a paragraph of its own.
 */
function flow(nodes: Iterable<ChildNode>, store: ImageStore, wrap: Wrap): DocNode[] {
  const blocks: DocNode[] = []
  let pending: DocNode[] = []

  const flush = (): void => {
    if (pending.length === 0) return
    const content = tidyRuns(pending)
    pending = []
    if (content.length > 0) blocks.push(...wrap(content))
  }

  const emitImage = (element: Element): void => {
    const image = store.take(element)
    if (!image) return
    flush()
    blocks.push(image)
  }

  const visit = (list: Iterable<ChildNode>): void => {
    for (const child of Array.from(list)) {
      if (child.nodeType === TEXT_NODE) {
        const value = collapseWhitespace(child.textContent ?? '')
        if (value !== '') pending.push({ type: 'text', text: value })
        continue
      }
      if (child.nodeType !== ELEMENT_NODE) continue

      const el = child as Element
      const tag = el.tagName.toLowerCase()
      if (IGNORED_TAGS[tag]) continue
      if (tag === 'div' || tag === 'section') {
        visit(el.childNodes)
        continue
      }
      if (INLINE_TAGS[tag] || !BLOCK_TAGS[tag]) {
        collectInlineElement(el, [], pending, emitImage)
        continue
      }

      flush()
      blocks.push(...blockOf(el, tag, store))
    }
  }

  visit(nodes)
  flush()
  return blocks
}

function listOf(element: Element, ordered: boolean, store: ImageStore): DocNode {
  const items: DocNode[] = []

  for (const child of Array.from(element.children)) {
    if (child.tagName.toLowerCase() !== 'li') continue
    const content = flow(child.childNodes, store, paragraphWrap)
    items.push({ type: 'listItem', content: content.length > 0 ? content : [emptyParagraph()] })
  }

  return { type: ordered ? 'orderedList' : 'bulletList', content: items }
}

/** `colspan`/`rowspan` as a merge count; 1 and the absurd read as no merge. */
function spanAttributes(element: Element): Record<string, number> {
  const attrs: Record<string, number> = {}
  const colspan = spanValue(element.getAttribute('colspan'))
  const rowspan = spanValue(element.getAttribute('rowspan'))
  if (colspan !== null) attrs.colspan = colspan
  if (rowspan !== null) attrs.rowspan = rowspan
  return attrs
}

function spanValue(value: string | null): number | null {
  if (value === null) return null
  const parsed = Number(value)
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null
}

function rowOf(element: Element, store: ImageStore): DocNode {
  const cells: DocNode[] = []

  for (const child of Array.from(element.children)) {
    const tag = child.tagName.toLowerCase()
    if (tag !== 'td' && tag !== 'th') continue
    const content = flow(child.childNodes, store, paragraphWrap)
    cells.push({
      type: tag === 'th' ? 'tableHeader' : 'tableCell',
      attrs: spanAttributes(child),
      content: content.length > 0 ? content : [emptyParagraph()],
    })
  }

  return { type: 'tableRow', content: cells }
}

/**
 * A table's rows, with its caption — if it has one — as a paragraph before the
 * table: the model has no caption node, and a caption that stood after the
 * rows would read as a note about whatever follows instead.
 */
function tableOf(element: Element, store: ImageStore): DocNode[] {
  const blocks: DocNode[] = []
  const rows: DocNode[] = []

  const collect = (parent: Element): void => {
    for (const child of Array.from(parent.children)) {
      const tag = child.tagName.toLowerCase()
      if (tag === 'caption') {
        blocks.push(...flow(child.childNodes, store, paragraphWrap))
        continue
      }
      if (tag === 'thead' || tag === 'tbody' || tag === 'tfoot') {
        collect(child)
        continue
      }
      if (tag === 'tr') rows.push(rowOf(child, store))
    }
  }

  collect(element)
  if (rows.length > 0) blocks.push({ type: 'table', content: rows })
  return blocks
}

function blockOf(element: Element, tag: string, store: ImageStore): DocNode[] {
  if (tag === 'p') return flow(element.childNodes, store, paragraphWrap)

  const heading = /^h([1-6])$/.exec(tag)
  if (heading) return flow(element.childNodes, store, headingWrap(Number(heading[1])))

  switch (tag) {
    case 'blockquote':
      return [{ type: 'blockquote', content: flow(element.childNodes, store, paragraphWrap) }]
    case 'pre':
      // The whole point of `pre` is that its whitespace was laid out by hand,
      // so the text goes in uncollapsed.
      return [{ type: 'codeBlock', content: [{ type: 'text', text: element.textContent ?? '' }] }]
    case 'ul':
      return [listOf(element, false, store)]
    case 'ol':
      return [listOf(element, true, store)]
    case 'table':
      return tableOf(element, store)
    default:
      return flow(element.childNodes, store, paragraphWrap)
  }
}

/**
 * The document model and the images its `writingImage` nodes refer to.
 * Separated so a test can look at the tree without opening the package.
 */
export function htmlToDocxDocument(html: string): {
  node: DocNode
  images: Record<string, ExportImage>
} {
  const parsed = new DOMParser().parseFromString(html, 'text/html')
  const container = parsed.querySelector('.ocr-export-document') ?? parsed.body
  const store = createImageStore()
  const content = container ? flow(container.childNodes, store, paragraphWrap) : []
  return { node: { type: 'doc', content }, images: store.images }
}

/**
 * Word bytes from the OCR export's printable HTML.
 *
 * The context is deliberately empty: the OCR document has no manuscript title,
 * citations, Zotero fields or bibliography, and an empty one is what keeps the
 * exporter from adding a heading for any of them.
 */
export async function generateDocxBytes(html: string): Promise<Uint8Array> {
  const { node, images } = htmlToDocxDocument(html)
  const context: ExportContext = {
    title: '',
    citations: 'footnote',
    zotero: {},
    bibliography: [],
    bibliographyHeading: '',
    images,
  }

  return toDocx(node, context, { margins: OCR_PAGE_MARGINS_TWIPS })
}
