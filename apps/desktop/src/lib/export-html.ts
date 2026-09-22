import {
  aroundFragment,
  quotedPartsOf,
  renderCorpusCitation,
  renderNoteLink,
} from './export-citations'
import { isBlockQuoteParagraph } from './export-markdown'
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
 * HTML export (plan-editor.md §17.1).
 *
 * # What "sanitizado" means here
 *
 * §17.1 asks for sanitized HTML, and the strongest form of that is not to run a
 * sanitizer over generated markup — it is never to pass markup through in the
 * first place. Nothing in a manuscript is HTML: every node is built here, and
 * every piece of text is escaped on its way out. So there is no injection
 * surface to clean, with one exception.
 *
 * That exception is the link target, which *is* a string the writer supplied
 * and which the browser will act on. `javascript:` and `data:` in an `href` are
 * the whole attack, so the scheme is checked against a list of what a citation
 * could legitimately point at, and a link that fails it keeps its text and
 * loses its target. Dropping the text too would silently delete words.
 *
 * # Why the file stands alone
 *
 * An export is something the writer sends to someone. A fragment that needs a
 * stylesheet they do not have is not that, so the document carries its own —
 * small, readable, and with the hanging indent §11.6 wants for a bibliography.
 */

const SAFE_SCHEMES = ['http:', 'https:', 'mailto:', 'ftp:']

function escape(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

/** A target the browser may be asked to follow, or null. */
export function safeHref(value: unknown): string | null {
  if (typeof value !== 'string' || !value.trim()) return null
  const href = value.trim()
  // Relative and anchor links carry no scheme, so they cannot carry a dangerous
  // one either.
  if (href.startsWith('#') || href.startsWith('/') || href.startsWith('./')) return href
  try {
    return SAFE_SCHEMES.includes(new URL(href).protocol) ? href : null
  } catch {
    // Not a URL at all. Refused rather than guessed at.
    return null
  }
}

interface Notes {
  collected: string[]
  bodies: Map<string, string>
}

function marker(notes: Notes, body: string): string {
  notes.collected.push(body)
  const at = notes.collected.length
  return `<sup class="fn"><a id="fnref-${at}" href="#fn-${at}">${at}</a></sup>`
}

const TAG_OF_MARK: Record<string, string> = {
  bold: 'strong',
  italic: 'em',
  strike: 's',
  code: 'code',
  underline: 'u',
  subscript: 'sub',
  superscript: 'sup',
}

function withMarks(value: string, node: Node): string {
  let out = value
  for (const mark of node.marks ?? []) {
    if (mark.type === 'link') {
      const href = safeHref(mark.attrs?.href)
      // A refused target keeps its words: dropping them would delete prose the
      // writer wrote, which is a worse outcome than a link that does nothing.
      out = href ? `<a href="${escape(href)}" rel="noopener noreferrer">${out}</a>` : out
      continue
    }
    if (mark.type === 'textStyle') {
      // A size in em, so it stays a proportion of the text around it, and a
      // colour in its print value (textStyleCss says why nothing else fits).
      const css = textStyleCss(mark.attrs)
      if (css) out = `<span style="${css}">${out}</span>`
      continue
    }
    if (mark.type === 'highlight') {
      const css = highlightCss(mark.attrs)
      if (css) out = `<mark style="${css}">${out}</mark>`
      continue
    }
    const tag = TAG_OF_MARK[mark.type ?? '']
    if (tag) out = `<${tag}>${out}</${tag}>`
  }
  return out
}

function inline(nodes: Node[], context: ExportContext, notes: Notes): string {
  return nodes
    .map((node) => {
      switch (node.type) {
        case 'text':
          return withMarks(escape(node.text ?? ''), node)
        case 'hardBreak':
          return '<br>'
        case 'footnoteReference': {
          const id = typeof node.attrs?.['data-id'] === 'string' ? node.attrs['data-id'] : ''
          return marker(notes, notes.bodies.get(id) ?? '')
        }
        case 'documentCitation': {
          const rendered = renderCorpusCitation(node.attrs ?? {}, context.citations)
          // A quote keeps the page's line breaks; in HTML they are <br />.
          const plain = (value: string) => escape(value).replace(/\n/g, '<br />')
          // A quote that took in an image draws the image where it stood, so
          // the words around it keep the sense they had on the page. The file
          // is embedded: an export is one file, and a link into the archive
          // would break the moment the document left this machine.
          const said = (value: string) => {
            const around = aroundFragment(value, rendered.fragment)
            const parts = quotedPartsOf(node.attrs ?? {})
            // All of them or none: with an image missing, the words around the
            // hole would run together and say something the page did not. The
            // quoted text keeps its line breaks and is the honest fallback.
            const drawable =
              parts !== null &&
              parts.every((part) => part.kind === 'text' || context.images?.[part.source])
            if (!around || !parts || !drawable) return plain(value)
            const drawn = parts
              .map((part) => {
                if (part.kind === 'text') return plain(part.text)
                const image = context.images?.[part.source]
                return image ? `<img src="${image.dataUrl}" alt="" />` : ''
              })
              .join('')
            // The guillemets belong to the fragment, so they stay around the
            // whole of it, image included.
            return `${plain(around[0])}«${drawn}»${plain(around[1])}`
          }
          if (context.citations === 'comment') {
            // HTML has no comment a reader sees. The matrix calls this a
            // fallback and this is it: an aside, marked as one, beside the
            // text rather than at the foot of the page.
            return rendered.note
              ? `<span class="cite"><span class="cite-comment" role="note">${said(rendered.note)}</span></span>`
              : ''
          }
          // Escaped like any other text: the note comes from a source title
          // the writer never meant as markup.
          const note = rendered.note ? marker(notes, said(rendered.note)) : ''
          return `<span class="cite">${said(rendered.inline)}</span>${note}`
        }
        case 'zoteroCitation':
          return `<span class="cite">${escape(zoteroTextOf(node, context))}</span>`
        case 'noteLink':
          return `<span class="note-link">${escape(renderNoteLink(node.attrs ?? {}))}</span>`
        default:
          return inline(childrenOf(node), context, notes)
      }
    })
    .join('')
}

/** A block's formatting as its `style` attribute, or nothing. */
function styled(node: Node): string {
  const css = blockCss(node)
  return css ? ` style="${css}"` : ''
}

function block(node: Node, context: ExportContext, notes: Notes): string {
  const kids = childrenOf(node)
  const children = () => kids.map((child) => block(child, context, notes)).join('\n')

  switch (node.type) {
    case 'paragraph':
      // A paragraph that holds nothing but a long quote is that quote's block,
      // as the manuscript shows it (export-markdown.ts, isBlockQuoteParagraph).
      return isBlockQuoteParagraph(node, context)
        ? `<blockquote class="cite-block"${styled(node)}>${inline(kids, context, notes)}</blockquote>`
        : `<p${styled(node)}>${inline(kids, context, notes)}</p>`

    case 'heading': {
      const level = Math.min(
        Math.max(typeof node.attrs?.level === 'number' ? node.attrs.level : 1, 1),
        6
      )
      return `<h${level}${styled(node)}>${inline(kids, context, notes)}</h${level}>`
    }

    case 'bulletList':
      return `<ul>\n${children()}\n</ul>`

    case 'orderedList': {
      const start = typeof node.attrs?.start === 'number' ? node.attrs.start : 1
      return `<ol${start === 1 ? '' : ` start="${start}"`}>\n${children()}\n</ol>`
    }

    case 'listItem':
      return `<li>${children()}</li>`

    case 'blockquote':
      return `<blockquote>\n${children()}\n</blockquote>`

    case 'codeBlock': {
      const language = typeof node.attrs?.language === 'string' ? node.attrs.language : ''
      const opening = language ? `<code class="language-${escape(language)}">` : '<code>'
      return `<pre>${opening}${escape(textOf(node))}</code></pre>`
    }

    case 'horizontalRule':
      return '<hr>'

    case 'table':
      return `<table>\n${children()}\n</table>`

    case 'tableRow':
      // Cells on one line: a row broken across lines reads as several rows to
      // anyone opening the file in an editor, which is where exports get read.
      return `<tr>${kids.map((cell) => block(cell, context, notes)).join('')}</tr>`

    case 'tableHeader':
      return `<th>${children()}</th>`

    case 'tableCell':
      return `<td>${children()}</td>`

    case 'footnotes':
      // Written at the end from the collected markers, in the order the markers
      // appear rather than the order the bodies are stored.
      return ''

    case 'writingImage': {
      const src = typeof node.attrs?.src === 'string' ? node.attrs.src : ''
      const image = context.images?.[src]
      const align = typeof node.attrs?.align === 'string' ? node.attrs.align : 'center'
      const alt = typeof node.attrs?.alt === 'string' ? node.attrs.alt : ''
      // I7: the author's own width and title never reached the exported
      // <img> — only alt did. `max-width: 100%` in STYLE below still caps a
      // width wider than the column, the same discipline the editor's own
      // stylesheet uses (spec, Rendering and Layout).
      const width = typeof node.attrs?.width === 'number' ? ` width="${node.attrs.width}"` : ''
      const title = typeof node.attrs?.title === 'string' && node.attrs.title ? ` title="${escape(node.attrs.title)}"` : ''
      const caption = inline(kids, context, notes)
      const img = image ? `<img src="${image.dataUrl}" alt="${escape(alt)}"${width}${title} />` : ''
      return `<figure class="writing-image" data-align="${align}">${img}<figcaption>${caption}</figcaption></figure>`
    }

    default:
      return children()
  }
}

export const STYLE = `
:root { color-scheme: light dark; }
body { margin: 0 auto; max-width: 42rem; padding: 2rem 1rem;
  font-family: Georgia, 'Times New Roman', serif; font-size: 1rem; line-height: 1.65; }
h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 2em 0 0.5em; }
blockquote { margin: 1.5em 0; padding-left: 1em; border-left: 3px solid currentColor;
  opacity: 0.85; }
/* A long quotation: set off in a box and a point smaller than the body, as
   academic typesetting sets one off. */
blockquote.cite-block { margin: 1.5em 10%; padding: 0.75em 1em; border: 1px solid currentColor;
  border-radius: 0.375em; white-space: pre-line; font-size: calc(1em - 1pt); }
pre { overflow-x: auto; padding: 0.75em; background: rgba(127,127,127,0.12); }
table { border-collapse: collapse; width: 100%; margin: 1.5em 0; }
th, td { border: 1px solid rgba(127,127,127,0.5); padding: 0.4em 0.6em; text-align: left; }
th { background: rgba(127,127,127,0.12); }
sup.fn a { text-decoration: none; }
/* An image a quote took in: on its own line, never wider than the column. */
.cite img { display: block; max-width: 100%; height: auto; margin: 0.75em 0; }
.cite-comment { font-size: 0.85em; opacity: 0.8; }
/* A manuscript image (I7): data-align was written onto the figure with no
   rule anywhere to act on it. max-width clamps an author's width wider than
   the column, the same discipline the editor's own stylesheet uses. */
.writing-image img { display: block; max-width: 100%; height: auto; }
.writing-image[data-align="left"] img { margin: 0 auto 0 0; }
.writing-image[data-align="center"] img { margin: 0 auto; }
.writing-image[data-align="right"] img { margin: 0 0 0 auto; }
.writing-image figcaption { font-size: 0.9em; font-style: italic; text-align: center; opacity: 0.85; }
.writing-image[data-align="left"] figcaption { text-align: left; }
.writing-image[data-align="right"] figcaption { text-align: right; }
.footnotes { margin-top: 3em; padding-top: 1em; border-top: 1px solid rgba(127,127,127,0.4);
  font-size: 0.9em; }
/* §11.6: a bibliography entry hangs, so the author is what the eye finds. */
.bibliography p { padding-left: 2em; text-indent: -2em; margin: 0.6em 0; }
`.trim()

export function toHtml(doc: Node, context: ExportContext): string {
  const flat: Notes = { collected: [], bodies: new Map() }
  const notes: Notes = {
    collected: [],
    bodies: footnoteBodies(doc, (nodes) => inline(nodes, context, flat)),
  }

  const body = childrenOf(doc)
    .filter((node) => node.type !== 'footnotes')
    .map((node) => block(node, context, notes))
    .filter(Boolean)
    .join('\n')

  const title = needsTitleHeading(doc, context.title) ? `<h1>${escape(context.title)}</h1>` : ''
  const sections = [title, body].filter(Boolean)

  if (notes.collected.length > 0) {
    const items = notes.collected
      .map(
        (note, index) =>
          `<li id="fn-${index + 1}">${note} <a href="#fnref-${index + 1}" aria-label="volver">↩</a></li>`
      )
      .join('\n')
    sections.push(`<section class="footnotes">\n<ol>\n${items}\n</ol>\n</section>`)
  }

  if (context.bibliography.length > 0) {
    const entries = context.bibliography.map((entry) => `<p>${escape(entry)}</p>`).join('\n')
    sections.push(
      `<section class="bibliography">\n<h2>${escape(context.bibliographyHeading)}</h2>\n${entries}\n</section>`
    )
  }

  return `<!doctype html>
<html lang="es">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(context.title || 'Documento')}</title>
<style>
${STYLE}
</style>
</head>
<body>
${sections.join('\n')}
</body>
</html>
`
}
