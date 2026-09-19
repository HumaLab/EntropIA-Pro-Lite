import type { RenderedTextMap } from './rendered-text-map'
import { wordRanges } from './text-highlight'

/**
 * The DOM side of citing from rendered OCR (rendered-text-map.ts is the text
 * side). A selection on rendered text spans elements — a heading, a paragraph,
 * a bold word inside it — so its ends are DOM positions, not offsets. They are
 * turned into offsets into the container's visible text, which is the string
 * the map was built from, and then mapped back to the raw extraction.
 */

export interface RenderedChoice {
  /** Offsets into the RAW extraction: what the citation anchors. */
  start: number
  end: number
  /** What the reader saw, laid out as one line: what the citation quotes. */
  quote: string
}

const BLOCKS = new Set([
  'P',
  'H1',
  'H2',
  'H3',
  'H4',
  'H5',
  'H6',
  'DIV',
  'SECTION',
  'BLOCKQUOTE',
  'PRE',
  'UL',
  'OL',
  'LI',
  'TABLE',
  'CAPTION',
  'THEAD',
  'TBODY',
  'TFOOT',
  'TR',
])

/**
 * The words of a selection laid out as the page lays them out: a blank line
 * between blocks, a line break where the text had one, single spaces inside a
 * line. `textContent` cannot say this — it has one newline for a paragraph
 * break and for a `<br>` alike — so the selected nodes are walked instead.
 */
function quoteOf(selected: Range): string {
  const parts: string[] = []
  const walk = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      parts.push((node as Text).data.replace(/\s+/g, ' '))
      return
    }
    const tag = node.nodeType === Node.ELEMENT_NODE ? (node as Element).tagName : ''
    if (tag === 'BR') {
      parts.push('\n')
      return
    }
    const block = BLOCKS.has(tag)
    if (block) parts.push('\n\n')
    node.childNodes.forEach(walk)
    if (tag === 'TD' || tag === 'TH') parts.push(' ')
    if (block) parts.push('\n\n')
  }
  walk(selected.cloneContents())
  return parts
    .join('')
    .replace(/[ \t]*\n[ \t]*/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/[ \t]{2,}/g, ' ')
    .trim()
}

/** How much visible text comes before a DOM position inside `container`. */
function visibleOffset(container: Node, node: Node, offset: number): number {
  const before = document.createRange()
  before.setStart(container, 0)
  before.setEnd(node, offset)
  return before.toString().length
}

/**
 * The selection as a citable choice; `'unmapped'` when it holds text but
 * cannot be anchored in the raw extraction (the caller asks for another one);
 * `null` when there is nothing selected in this container.
 */
export function renderedSelection(
  container: HTMLElement,
  selected: Range | null,
  map: RenderedTextMap
): RenderedChoice | 'unmapped' | null {
  if (!selected || selected.collapsed) return null
  if (!container.contains(selected.startContainer) || !container.contains(selected.endContainer)) {
    return null
  }

  const from = visibleOffset(container, selected.startContainer, selected.startOffset)
  const to = visibleOffset(container, selected.endContainer, selected.endOffset)
  const quote = quoteOf(selected)
  if (!quote) return null

  const raw = map.toRaw(from, to)
  if (!raw) return 'unmapped'
  return { start: raw.start, end: raw.end, quote }
}

/**
 * Ranges over every occurrence of `words` in the rendered text, for the CSS
 * Custom Highlight API. Word by word inside each text node: formatting splits
 * text into nodes, but it never splits a word the index knows as one.
 */
export function highlightRanges(container: HTMLElement, words: string[]): Range[] {
  const ranges: Range[] = []
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node as Text
    for (const [start, end] of wordRanges(text.data, words)) {
      const mark = document.createRange()
      mark.setStart(text, start)
      mark.setEnd(text, end)
      ranges.push(mark)
    }
  }
  return ranges
}
