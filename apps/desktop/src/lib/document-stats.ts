import type { Node } from './export-document'

/**
 * Counts for the Export tab: what a writer checks against a word limit before
 * sending a manuscript out. Informative only; nothing is stored.
 *
 * # What is counted
 *
 * The body, as a word processor counts it. Footnotes are reported as how many
 * there are, not added to the words: a journal's limit is usually on the text,
 * and a count that grows with every note would read as the text growing.
 *
 * What the page shows is what is counted. A citation is an atom with no text
 * of its own, but on screen it reads as its quotation, its note snapshot or its
 * rendering — so it counts as that, and the number matches what the writer
 * sees.
 */

export interface DocumentStats {
  words: number
  /** With spaces, excluding the breaks between paragraphs. */
  characters: number
  charactersNoSpaces: number
  /** Paragraphs and headings with something in them. */
  paragraphs: number
  footnotes: number
}

/** The text an atom shows on the page, by the same attribute its node draws. */
const ATOM_TEXT: Record<string, string> = {
  documentCitation: 'quotedText',
  noteLink: 'contentSnapshot',
  zoteroCitation: 'renderedText',
}

/** A word has at least one letter or digit: a dash on its own is not one. */
const WORD = /[\p{L}\p{N}]/u

function inlineText(node: Node): string {
  if (node.type === 'text') return typeof node.text === 'string' ? node.text : ''
  const attribute = ATOM_TEXT[node.type ?? '']
  if (attribute) {
    const value = node.attrs?.[attribute]
    return typeof value === 'string' ? value : ''
  }
  // A hard break separates words the way a space does.
  if (node.type === 'hardBreak') return ' '
  return (node.content ?? []).map(inlineText).join('')
}

export function documentStats(doc: Node): DocumentStats {
  const stats: DocumentStats = {
    words: 0,
    characters: 0,
    charactersNoSpaces: 0,
    paragraphs: 0,
    footnotes: 0,
  }

  const walk = (node: Node) => {
    if (node.type === 'footnote') {
      stats.footnotes += 1
      return
    }
    if (node.type === 'paragraph' || node.type === 'heading') {
      // Code points, so an accented letter written as one counts as one.
      const chars = [...inlineText(node)]
      const text = chars.join('')
      if (text.trim() === '') return
      stats.paragraphs += 1
      stats.characters += chars.length
      stats.charactersNoSpaces += chars.filter((char) => !/\s/u.test(char)).length
      stats.words += text.split(/\s+/u).filter((token) => WORD.test(token)).length
      return
    }
    for (const child of node.content ?? []) walk(child)
  }

  walk(doc)
  return stats
}
