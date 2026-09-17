import type { Node } from '@tiptap/pm/model'

/**
 * Finding text in the manuscript (plan-editor.md §26, "búsqueda y reemplazo").
 *
 * # Why this is not a walk over text nodes
 *
 * A bold word in the middle of a sentence splits it into three text nodes, so
 * searching each node on its own cannot find "los molineros rurales" — the
 * phrase exists only once the nodes are read together. Marks are invisible to a
 * reader and have to be invisible to a search.
 *
 * So the document is flattened into one string first, with each text node
 * recorded next to the document position it came from, and matches found in
 * that string are mapped back. Blocks are separated by a character that cannot
 * be typed into a paragraph, so two paragraphs never concatenate into a match
 * that is not on the page.
 *
 * Pure over a ProseMirror node: no editor, no DOM, no state.
 */

export interface SearchMatch {
  /** Document position where the match starts. */
  from: number
  /** Document position where it ends. */
  to: number
}

export interface SearchOptions {
  caseSensitive?: boolean
}

/**
 * Separates blocks in the flattened text. A record separator: there is no key
 * for it, so no query can contain one and match across a block boundary.
 */
const BLOCK_BREAK = ''

interface Chunk {
  /** Offset of this chunk's first character in the flattened text. */
  offset: number
  /** Document position of that same character. */
  pos: number
  length: number
}

/**
 * The document as one string, plus what each stretch of it came from.
 *
 * An inline atom — a corpus citation — contributes no text, so it is skipped
 * rather than counted: its size shifts document positions but not offsets,
 * which is exactly the difference the chunk table records.
 */
function flatten(doc: Node): { text: string; chunks: Chunk[] } {
  const parts: string[] = []
  const chunks: Chunk[] = []
  let offset = 0
  let pendingBreak = false

  doc.descendants((node, pos) => {
    if (node.isText && node.text) {
      if (pendingBreak) {
        parts.push(BLOCK_BREAK)
        offset += BLOCK_BREAK.length
        pendingBreak = false
      }
      parts.push(node.text)
      chunks.push({ offset, pos, length: node.text.length })
      offset += node.text.length
      return false
    }
    // A block that is about to hold text closes whatever came before it. The
    // break is written lazily, so a document never opens or ends with one.
    if (node.isBlock && chunks.length > 0) pendingBreak = true
    return true
  })

  return { text: parts.join(''), chunks }
}

/** The document position of a flattened offset, or null if it lands in a break. */
function toPosition(chunks: Chunk[], offset: number): number | null {
  for (const chunk of chunks) {
    if (offset >= chunk.offset && offset <= chunk.offset + chunk.length) {
      return chunk.pos + (offset - chunk.offset)
    }
  }
  return null
}

/**
 * Every occurrence of `query`, in document order and never overlapping.
 *
 * The query is text someone typed, not a pattern: it is searched literally, so
 * `(` finds a parenthesis instead of opening a group. An empty query finds
 * nothing — matching everywhere is not a useful answer to asking for nothing.
 */
export function findMatches(doc: Node, query: string, options: SearchOptions = {}): SearchMatch[] {
  if (query.length === 0) return []

  const { text, chunks } = flatten(doc)
  const haystack = options.caseSensitive ? text : text.toLowerCase()
  const needle = options.caseSensitive ? query : query.toLowerCase()

  const matches: SearchMatch[] = []
  let cursor = 0
  for (;;) {
    const start = haystack.indexOf(needle, cursor)
    if (start === -1) break
    const from = toPosition(chunks, start)
    const to = toPosition(chunks, start + needle.length)
    if (from !== null && to !== null) matches.push({ from, to })
    // Past the whole match: occurrences are reported side by side rather than
    // overlapping, so `aa` finds two in `aaaa` and not three.
    cursor = start + needle.length
  }

  return matches
}

/** Steps through the matches, wrapping at either end. */
export function nextMatchIndex(matches: SearchMatch[], current: number, direction: 1 | -1): number {
  if (matches.length === 0) return -1
  return (current + direction + matches.length) % matches.length
}

/** The first match at or after `pos`, so a search starts where the caret is. */
export function matchIndexAfter(matches: SearchMatch[], pos: number): number {
  if (matches.length === 0) return -1
  const found = matches.findIndex((match) => match.from >= pos)
  return found === -1 ? 0 : found
}
