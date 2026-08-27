import type {
  Alignment,
  Content,
  ContentImage,
  ContentOrderedList,
  ContentStack,
  ContentText,
  ContentUnorderedList,
  StyleDictionary,
  TDocumentDefinitions,
} from 'pdfmake/interfaces'

const PAGE_MARGIN_PT = 34.02
const WRITABLE_PAGE_WIDTH_PT = 527.24
const MAX_IMAGE_HEIGHT_PT = 700
const BLOCK_TAGS: Record<string, true> = {
  blockquote: true,
  div: true,
  h1: true,
  h2: true,
  h3: true,
  h4: true,
  h5: true,
  h6: true,
  img: true,
  ol: true,
  p: true,
  pre: true,
  section: true,
  table: true,
  ul: true,
}
const BLOCK_SELECTOR = Object.keys(BLOCK_TAGS).join(',')

type InlineStyle = {
  bold?: boolean
  italics?: boolean
  decoration?: 'underline'
  link?: string
  background?: string
}

const DOCUMENT_STYLES: StyleDictionary = {
  paragraph: { margin: [0, 0, 0, 9] },
  listItem: { margin: [0, 0, 0, 3] },
  list: { margin: [0, 0, 0, 9] },
  h1: { fontSize: 22, bold: true, color: '#111827', margin: [0, 0, 0, 10] },
  h2: { fontSize: 19, bold: true, color: '#111827', margin: [0, 0, 0, 9] },
  h3: { fontSize: 16, bold: true, color: '#111827', margin: [0, 0, 0, 8] },
  h4: { fontSize: 14, bold: true, color: '#111827', margin: [0, 0, 0, 7] },
  h5: { fontSize: 12, bold: true, color: '#111827', margin: [0, 0, 0, 6] },
  h6: { fontSize: 11, bold: true, color: '#111827', margin: [0, 0, 0, 5] },
  codeBlock: {
    background: '#f3f4f6',
    color: '#111827',
    fontSize: 9.5,
    margin: [0, 0, 0, 9],
  },
}

function normalizedInlineText(value: string): string {
  return value.replace(/\s+/g, ' ')
}

function inlineStyleFor(element: Element, inherited: InlineStyle): InlineStyle {
  const tag = element.tagName.toLowerCase()
  if (tag === 'strong' || tag === 'b') return { ...inherited, bold: true }
  if (tag === 'em' || tag === 'i') return { ...inherited, italics: true }
  if (tag === 'u') return { ...inherited, decoration: 'underline' }
  if (tag === 'code') return { ...inherited, background: '#f3f4f6' }
  if (tag === 'a') {
    const link = element.getAttribute('href')?.trim()
    return link ? { ...inherited, link } : inherited
  }
  return inherited
}

function trimInlineRuns(runs: ContentText[]): ContentText[] {
  const trimmed = runs.map((run) => ({ ...run, text: String(run.text) }))
  while (trimmed.length > 0 && String(trimmed[0]!.text).trim() === '') trimmed.shift()
  while (trimmed.length > 0 && String(trimmed.at(-1)!.text).trim() === '') trimmed.pop()
  if (trimmed.length === 0) return []

  trimmed[0] = { ...trimmed[0]!, text: String(trimmed[0]!.text).replace(/^\s+/, '') }
  const last = trimmed.length - 1
  trimmed[last] = { ...trimmed[last]!, text: String(trimmed[last]!.text).replace(/\s+$/, '') }
  return trimmed.filter((run) => String(run.text).length > 0)
}

function inlineRuns(nodes: Iterable<Node>, inherited: InlineStyle = {}): ContentText[] {
  const runs: ContentText[] = []

  for (const node of nodes) {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = normalizedInlineText(node.textContent ?? '')
      if (text) runs.push({ text, ...inherited })
      continue
    }
    if (node.nodeType !== Node.ELEMENT_NODE) continue

    const element = node as Element
    if (element.tagName.toLowerCase() === 'br') {
      runs.push({ text: '\n', ...inherited })
      continue
    }
    runs.push(...inlineRuns(element.childNodes, inlineStyleFor(element, inherited)))
  }

  return runs
}

function paragraphFromNodes(nodes: Node[], style: string): ContentText | null {
  const text = trimInlineRuns(inlineRuns(nodes))
  return text.length > 0 ? { text, style } : null
}

function paragraphFromText(value: string, style = 'paragraph'): ContentText | null {
  const text = value.trim()
  return text ? { text, style } : null
}

function alignmentFor(element: Element): Alignment | undefined {
  const align = element.getAttribute('align')?.toLowerCase()
  return align === 'left' || align === 'center' || align === 'right' ? align : undefined
}

function isBlockNode(node: Node): node is Element {
  if (node.nodeType !== Node.ELEMENT_NODE) return false

  const element = node as Element
  return (
    BLOCK_TAGS[element.tagName.toLowerCase()] === true ||
    element.querySelector(BLOCK_SELECTOR) !== null
  )
}

function convertChildren(element: Element, paragraphStyle = 'paragraph'): Content[] {
  const content: Content[] = []
  let pendingInline: Node[] = []

  const flushInline = () => {
    const paragraph = paragraphFromNodes(pendingInline, paragraphStyle)
    if (paragraph) content.push(paragraph)
    pendingInline = []
  }

  for (const child of Array.from(element.childNodes)) {
    if (isBlockNode(child)) {
      flushInline()
      content.push(...convertElement(child))
    } else {
      pendingInline.push(child)
    }
  }
  flushInline()

  return content
}

function convertListItem(element: Element): Content {
  const content = convertChildren(element, 'listItem')
  if (content.length === 0) return { text: '' }
  return content.length === 1 ? content[0]! : ({ stack: content } satisfies ContentStack)
}

function convertList(element: Element): ContentOrderedList | ContentUnorderedList {
  const items = Array.from(element.children)
    .filter((child) => child.tagName.toLowerCase() === 'li')
    .map(convertListItem)

  return element.tagName.toLowerCase() === 'ol'
    ? { ol: items as ContentOrderedList['ol'], style: 'list' }
    : { ul: items as ContentUnorderedList['ul'], style: 'list' }
}

function convertImage(element: Element): ContentImage[] {
  const source = element.getAttribute('src')?.trim()
  if (!source) return []
  return [
    {
      image: source,
      fit: [WRITABLE_PAGE_WIDTH_PT, MAX_IMAGE_HEIGHT_PT],
      margin: [0, 0, 0, 9],
    },
  ]
}

function convertBlockquote(element: Element): Content[] {
  const stack = convertChildren(element)
  if (stack.length === 0) return []
  return [
    {
      table: {
        widths: ['*'],
        body: [
          [
            {
              stack,
              border: [true, false, false, false],
              borderColor: ['#9ca3af', '#9ca3af', '#9ca3af', '#9ca3af'],
              color: '#4b5563',
              margin: [10, 0, 0, 0],
            },
          ],
        ],
      },
      margin: [0, 0, 0, 9],
    },
  ]
}

function convertElement(element: Element): Content[] {
  const tag = element.tagName.toLowerCase()

  if (/^h[1-6]$/.test(tag)) {
    const heading = paragraphFromNodes(Array.from(element.childNodes), tag)
    if (!heading) return []
    return [{ ...heading, headlineLevel: Number(tag[1]) }]
  }
  if (tag === 'p') return convertChildren(element)
  if (tag === 'div' || tag === 'section') {
    const stack = convertChildren(element)
    const alignment = alignmentFor(element)
    return alignment && stack.length > 0 ? [{ stack, alignment }] : stack
  }
  if (tag === 'ul' || tag === 'ol') return [convertList(element)]
  if (tag === 'blockquote') return convertBlockquote(element)
  if (tag === 'pre') {
    const text = element.textContent ?? ''
    return text
      ? [
          {
            text,
            style: 'codeBlock',
            preserveLeadingSpaces: true,
            preserveTrailingSpaces: true,
          },
        ]
      : []
  }
  if (tag === 'img') return convertImage(element)
  if (tag === 'table') {
    const fallback = paragraphFromText(element.textContent ?? '')
    return fallback ? [fallback] : []
  }

  return convertChildren(element)
}

export function buildOcrPdfDefinition(html: string): TDocumentDefinitions {
  const parsed = new DOMParser().parseFromString(html, 'text/html')
  const root = parsed.querySelector('.ocr-export-document') ?? parsed.body

  return {
    pageSize: 'A4',
    pageOrientation: 'portrait',
    pageMargins: [PAGE_MARGIN_PT, PAGE_MARGIN_PT, PAGE_MARGIN_PT, PAGE_MARGIN_PT],
    compress: true,
    defaultStyle: {
      font: 'Roboto',
      fontSize: 11,
      lineHeight: 1.55,
      color: '#1f2937',
    },
    styles: DOCUMENT_STYLES,
    content: convertChildren(root),
    pageBreakBefore(currentNode, followingNodesOnPage) {
      return Boolean(
        currentNode.headlineLevel && followingNodesOnPage.getFollowingNodesOnPage().length === 0
      )
    },
  }
}
