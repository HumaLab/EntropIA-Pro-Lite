/**
 * Reading a note's words out of its markup (plan-editor.md §13).
 *
 * # Why this exists
 *
 * A research note is written in a rich text editor, so its content is stored as
 * HTML. Everything that *quotes* a note — the results list, the snapshot that
 * travels on a link, the text copied into a manuscript — wants what the note
 * says, not how it is marked up, and without this they showed
 * `<p>Esto es una carta…</p>` to the writer and wrote it into the article.
 *
 * # Why the hash is deliberately not routed through here
 *
 * `resolveNoteLink` compares a stored hash against a hash of the note as it
 * stands now, and both are taken over the raw content. Hashing the extracted
 * text instead would be tidier and would report every existing link as
 * "the note changed" the first time it ran — a warning about nothing, on every
 * link, which is how a warning stops being read. So identity stays with the
 * stored bytes and only what a person sees passes through here.
 */

/** Blocks whose boundary is a paragraph break rather than a word gap. */
const BLOCK = new Set([
  'ADDRESS',
  'ARTICLE',
  'ASIDE',
  'BLOCKQUOTE',
  'DD',
  'DIV',
  'DL',
  'DT',
  'FIGCAPTION',
  'FIGURE',
  'FOOTER',
  'H1',
  'H2',
  'H3',
  'H4',
  'H5',
  'H6',
  'HEADER',
  'HR',
  'LI',
  'MAIN',
  'NAV',
  'OL',
  'P',
  'PRE',
  'SECTION',
  'TABLE',
  'TD',
  'TH',
  'TR',
  'UL',
])

function walk(node: Node, out: string[]): void {
  if (node.nodeType === 3) {
    out.push(node.nodeValue ?? '')
    return
  }
  if (node.nodeType !== 1) return

  const element = node as Element
  if (element.tagName === 'BR') {
    out.push('\n')
    return
  }

  const block = BLOCK.has(element.tagName)
  if (block) out.push('\n\n')
  for (const child of Array.from(element.childNodes)) walk(child, out)
  if (block) out.push('\n\n')
}

/**
 * A note's text, with its block structure kept as blank lines.
 *
 * Parsed rather than stripped with a pattern. A regular expression over markup
 * gets `<p title="a>b">` wrong, leaves `&amp;` as five characters, and joins
 * two paragraphs into one word — and this text goes into someone's article.
 *
 * `DOMParser` on `text/html` builds an inert document: it runs no script and
 * loads nothing, which is what makes parsing content of unknown provenance the
 * safe option here rather than the risky one.
 */
export function plainTextOf(html: string): string {
  if (!html) return ''

  const parser = typeof DOMParser === 'undefined' ? null : new DOMParser()
  if (!parser) {
    // No parser at all. Better a crude strip than markup in a manuscript, and
    // better still that this never happens — every surface that calls it has a
    // DOM.
    return html
      .replace(/<[^>]*>/g, ' ')
      .replace(/\s+/g, ' ')
      .trim()
  }

  const parsed = parser.parseFromString(html, 'text/html')
  const out: string[] = []
  for (const child of Array.from(parsed.body.childNodes)) walk(child, out)

  return (
    out
      .join('')
      // Spaces and tabs collapse; newlines are structure and survive.
      .replace(/[^\S\n]+/g, ' ')
      .replace(/ *\n */g, '\n')
      .replace(/\n{3,}/g, '\n\n')
      .trim()
  )
}

/**
 * The same text on one line, for a row in a list.
 *
 * A result row is one line tall by design, so a note with four paragraphs has
 * to arrive as one — otherwise the break lands mid-row and the list stops
 * lining up.
 */
export function previewTextOf(html: string): string {
  return plainTextOf(html).replace(/\s+/g, ' ').trim()
}
