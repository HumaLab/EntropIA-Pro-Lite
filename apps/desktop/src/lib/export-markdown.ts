import { isLongQuote } from '@entropia/ui'
import {
  aroundFragment,
  quotedPartsOf,
  renderCorpusCitation,
  renderNoteLink,
} from './export-citations'
import type { ExportContext, Node } from './export-document'
import {
  blockCss,
  childrenOf,
  footnoteBodies,
  highlightCss,
  needsTitleHeading,
  textOf,
  textStyleCss,
  zoteroTextOf,
} from './export-document'

/**
 * Markdown export (plan-editor.md §17.1).
 *
 * # Which Markdown
 *
 * GitHub-flavoured: it is the dialect with tables and footnotes, which §17.1
 * puts on the obligatory list, and the one every reader the writer is likely to
 * open the file in understands. Underline has no GFM form at all, so it goes
 * out as an inline `<u>` and the fidelity matrix calls that a fallback out
 * loud — the alternative, dropping it, would be a silent edit of the
 * manuscript.
 *
 * # Why the notes are numbered here and not counted per kind
 *
 * A manuscript can carry two things that become notes: real footnotes, and
 * corpus citations when the writer chose the footnote representation. Numbering
 * them separately would print `[^1]` twice on one page. One counter, in
 * document order, is what a reader expects and what every other exporter here
 * does too.
 */

/** Characters that would otherwise start a construct where text was meant. */
function escape(value: string): string {
  return value.replace(/([\\`*_{}[\]()#+\-.!|>])/g, '\\$1')
}

interface Notes {
  /** The note bodies, in the order their markers appear. */
  collected: string[]
  /** `data-id` → the footnote's own body, for a real footnote reference. */
  bodies: Map<string, string>
}

function marker(notes: Notes, body: string): string {
  notes.collected.push(body)
  return `[^${notes.collected.length}]`
}

function withMarks(value: string, node: Node): string {
  let out = value
  for (const mark of node.marks ?? []) {
    switch (mark.type) {
      case 'bold':
        out = `**${out}**`
        break
      case 'italic':
        out = `*${out}*`
        break
      case 'strike':
        out = `~~${out}~~`
        break
      case 'code':
        out = `\`${out}\``
        break
      case 'underline':
        // No GFM equivalent. The matrix declares this a fallback; dropping it
        // would be a silent edit of the manuscript.
        out = `<u>${out}</u>`
        break
      // The same stand-in as underline, declared the same way in the matrix.
      case 'subscript':
        out = `<sub>${out}</sub>`
        break
      case 'superscript':
        out = `<sup>${out}</sup>`
        break
      // Sizes and colours have no GFM form either: the HTML export's own
      // inline style stands in, declared in the matrix like underline.
      case 'textStyle': {
        const css = textStyleCss(mark.attrs)
        if (css) out = `<span style="${css}">${out}</span>`
        break
      }
      case 'highlight': {
        const css = highlightCss(mark.attrs)
        if (css) out = `<mark style="${css}">${out}</mark>`
        break
      }
      case 'link': {
        const href = typeof mark.attrs?.href === 'string' ? mark.attrs.href : ''
        out = href ? `[${out}](${href})` : out
        break
      }
      default:
        break
    }
  }
  return out
}

function inline(nodes: Node[], context: ExportContext, notes: Notes): string {
  return nodes
    .map((node) => {
      switch (node.type) {
        case 'text':
          // Escaped before the marks are applied, so the marks' own syntax is
          // not escaped along with the words.
          return withMarks(escape(node.text ?? ''), node)
        case 'hardBreak':
          // Two trailing spaces: invisible in the source, but the real GFM
          // line break, and the only one that survives a round trip.
          return '  \n'
        case 'footnoteReference': {
          const id = typeof node.attrs?.['data-id'] === 'string' ? node.attrs['data-id'] : ''
          return marker(notes, notes.bodies.get(id) ?? '')
        }
        case 'documentCitation': {
          const rendered = renderCorpusCitation(node.attrs ?? {}, context.citations)
          // A quote keeps the page's line breaks. In Markdown they are hard
          // breaks — two trailing spaces — and never blank lines: a blank line
          // would end the paragraph the citation sits in, or the footnote it
          // is written into.
          const plain = (value: string) =>
            escape(value)
              .split(/\n{2,}/)
              .map((block) => block.replace(/\n/g, '  \n'))
              .join('\n\n')
          // A quote that took in an image draws it where it stood, embedded as
          // the OCR export embeds its regions: a Markdown file is one file, and
          // a link into the archive would break the moment it left this
          // machine. All the images or none — with one missing, the words
          // around the hole would run together.
          const said = (value: string) => {
            const around = aroundFragment(value, rendered.fragment)
            const parts = quotedPartsOf(node.attrs ?? {})
            const drawable =
              parts !== null &&
              parts.every((part) => part.kind === 'text' || context.images?.[part.source])
            if (!around || !parts || !drawable) return plain(value)
            const drawn = parts
              .map((part) =>
                part.kind === 'text'
                  ? plain(part.text)
                  : `![](${context.images?.[part.source]?.dataUrl ?? ''})`
              )
              .join('')
            return `${plain(around[0])}«${drawn}»${plain(around[1])}`
          }
          // Escaped like any other text: a source whose title holds an
          // asterisk would otherwise open emphasis inside the note.
          const note = rendered.note ? marker(notes, said(rendered.note)) : ''
          return `${said(rendered.inline)}${note}`
        }
        case 'zoteroCitation':
          return escape(zoteroTextOf(node, context))
        case 'noteLink':
          return escape(renderNoteLink(node.attrs ?? {}))
        default:
          return inline(childrenOf(node), context, notes)
      }
    })
    .join('')
}

function tableRow(row: Node, context: ExportContext, notes: Notes): string {
  const cells = childrenOf(row).map((cell) =>
    childrenOf(cell)
      .map((block) => inline(childrenOf(block), context, notes))
      .join(' ')
      // A newline inside a GFM cell ends the table, so the cell is flattened
      // rather than allowed to break the structure around it.
      .replace(/\n+/g, ' ')
      .trim()
  )
  return `| ${cells.join(' | ')} |`
}

function table(node: Node, context: ExportContext, notes: Notes): string {
  const rows = childrenOf(node)
  if (rows.length === 0) return ''

  const lines = rows.map((row) => tableRow(row, context, notes))
  const width = childrenOf(rows[0]!).length
  // GFM has no table without a header row. A manuscript table whose first row
  // is ordinary cells still has to become one, so the first row serves.
  lines.splice(1, 0, `|${' --- |'.repeat(width)}`)
  return lines.join('\n')
}

/**
 * Whether this paragraph is a long quote standing on its own, which every
 * format sets off as a block (isLongQuote, shared with the manuscript). Only
 * where the representation writes the quote into the text: as a footnote or a
 * comment the quote is in the note, and the paragraph holds a marker.
 */
export function isBlockQuoteParagraph(node: Node, context: ExportContext): boolean {
  if (context.citations !== 'quote_with_note') return false
  const kids = childrenOf(node)
  if (kids.length !== 1 || kids[0]?.type !== 'documentCitation') return false
  const quoted = kids[0]?.attrs?.quotedText
  return typeof quoted === 'string' && isLongQuote(quoted)
}

/**
 * A formatted block inside a `<div style>`. GFM has no paragraph formatting;
 * the blank lines are what keep the content Markdown (an HTML block ends at a
 * blank line), so the marks stay marks and a heading stays a heading. The
 * matrix calls this a fallback, and the export warns.
 */
function wrapped(node: Node, markdown: string): string {
  const css = blockCss(node)
  return css ? `<div style="${css}">\n\n${markdown}\n\n</div>` : markdown
}

function block(node: Node, context: ExportContext, notes: Notes, depth = 0): string {
  const kids = childrenOf(node)

  switch (node.type) {
    case 'paragraph': {
      const written = wrapped(node, inline(kids, context, notes))
      // A paragraph that holds nothing but a long quote is that quote's block,
      // as the manuscript shows it: Markdown's block is the blockquote.
      return isBlockQuoteParagraph(node, context)
        ? written
            .split('\n')
            // Not trimmed: a hard break inside the quote is two trailing
            // spaces, and trimming the line would undo it.
            .map((line) => (line.trim() ? `> ${line}` : '>'))
            .join('\n')
        : written
    }

    case 'heading': {
      const level = typeof node.attrs?.level === 'number' ? node.attrs.level : 1
      return wrapped(
        node,
        `${'#'.repeat(Math.min(Math.max(level, 1), 6))} ${inline(kids, context, notes)}`
      )
    }

    case 'bulletList':
    case 'orderedList': {
      const ordered = node.type === 'orderedList'
      const start = typeof node.attrs?.start === 'number' ? node.attrs.start : 1
      return kids
        .map((item, index) => {
          const bullet = ordered ? `${start + index}. ` : '- '
          const body = childrenOf(item)
            .map((child) => block(child, context, notes, depth + 1))
            .join('\n\n')
          // Continuation lines are indented under the bullet, which is what
          // keeps a nested list nested instead of closing the outer one.
          const indented = body
            .split('\n')
            // A blank line stays blank: indenting it leaves trailing spaces,
            // which some readers take for a hard line break.
            .map((line, at) => (at === 0 || !line ? line : `${' '.repeat(bullet.length)}${line}`))
            .join('\n')
          return `${bullet}${indented}`
        })
        .join('\n')
    }

    case 'blockquote':
      return kids
        .map((child) => block(child, context, notes, depth))
        .join('\n\n')
        .split('\n')
        .map((line) => `> ${line}`.trimEnd())
        .join('\n')

    case 'codeBlock': {
      const language = typeof node.attrs?.language === 'string' ? node.attrs.language : ''
      return `\`\`\`${language}\n${textOf(node)}\n\`\`\``
    }

    case 'horizontalRule':
      return '---'

    case 'table':
      return table(node, context, notes)

    case 'footnotes':
      // Rendered at the end from the collected markers, never here: the block
      // sits at the end of the document but its notes belong to wherever their
      // references are.
      return ''

    case 'writingImage': {
      const src = typeof node.attrs?.src === 'string' ? node.attrs.src : ''
      const image = context.images?.[src]
      const alt = typeof node.attrs?.alt === 'string' ? node.attrs.alt : ''
      const caption = inline(kids, context, notes)
      const img = image ? `![${escape(alt)}](${image.dataUrl})` : ''
      // I7: the editor's figcaption is italic (WritingEditor.svelte), so the
      // caption goes out wrapped in `*…*` — guarded by the same truthiness
      // check as the line below, so an empty caption never wraps into a
      // stray `**`. `escape()` already neutralises any `*` the writer typed,
      // so these two extra ones are always the markers, never ambiguous.
      //
      // A blank line here, not a hard break, would leave the image alone in
      // its own paragraph, and a paragraph holding only an image is widely
      // promoted to a figure whose visible caption is the *alt* text (e.g.
      // Pandoc's implicit_figures) — alt is accessibility metadata and must
      // never be visible. A hard break — two trailing spaces — keeps the
      // image and caption in one paragraph instead, same as the quote line
      // breaks above.
      //
      // The image's own alignment (`data-align`) has no equivalent here:
      // plain Markdown carries no per-element alignment, and this exporter's
      // standing policy is a single self-contained file, so no HTML fallback
      // is emitted for it either.
      return caption ? `${img}  \n*${caption}*` : img
    }

    default:
      return kids.map((child) => block(child, context, notes, depth)).join('\n\n')
  }
}

export function toMarkdown(doc: Node, context: ExportContext): string {
  // A note inside a note is not a thing, so the bodies are rendered against a
  // counter of their own that is then thrown away: anything inside a footnote
  // that would have become a marker reads as its text instead.
  const flat: Notes = { collected: [], bodies: new Map() }
  const notes: Notes = {
    collected: [],
    bodies: footnoteBodies(doc, (nodes) => inline(nodes, context, flat)),
  }

  const body = childrenOf(doc)
    .filter((node) => node.type !== 'footnotes')
    .map((node) => block(node, context, notes))
    .filter((chunk) => chunk.length > 0)
    .join('\n\n')

  const title = needsTitleHeading(doc, context.title) ? `# ${escape(context.title)}` : ''
  const parts = [title, body].filter(Boolean)

  if (notes.collected.length > 0) {
    parts.push(
      notes.collected
        // A note that runs to several lines continues indented: without it the
        // second line is prose again and the note ends after the first.
        .map((note, index) => `[^${index + 1}]: ${note.replace(/\n/g, '\n    ')}`)
        .join('\n')
    )
  }

  if (context.bibliography.length > 0) {
    parts.push(
      [
        `## ${context.bibliographyHeading}`,
        ...context.bibliography.map((entry) => escape(entry)),
      ].join('\n\n')
    )
  }

  return `${parts.join('\n\n')}\n`
}
