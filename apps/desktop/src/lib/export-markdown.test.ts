import { describe, expect, it } from 'vitest'
import type { ExportContext, Node } from './export-document'
import { toMarkdown } from './export-markdown'

/**
 * Markdown export (plan-editor.md §17.1, §17.4).
 *
 * The pattern document of §17.4 is exercised in `export-pattern.test.ts`, which
 * holds all three formats to the same manuscript. What is asserted here is each
 * construct on its own, so a failure names what broke instead of printing a
 * whole document and leaving the reading to whoever is on call.
 */

const context: ExportContext = {
  title: '',
  citations: 'footnote',
  zotero: {},
  bibliography: [],
  bibliographyHeading: 'Bibliografía',
}

const text = (value: string, marks?: { type: string; attrs?: Record<string, unknown> }[]) => ({
  type: 'text',
  text: value,
  ...(marks ? { marks } : {}),
})
const p = (...content: Node[]) => ({ type: 'paragraph', content })
const doc = (...content: Node[]) => ({ type: 'doc', content })

const md = (node: Node, extra: Partial<ExportContext> = {}) =>
  toMarkdown(node, { ...context, ...extra }).trimEnd()

describe('the hierarchy and the prose', () => {
  it('writes headings at their level', () => {
    expect(md(doc({ type: 'heading', attrs: { level: 2 }, content: [text('El problema')] }))).toBe(
      '## El problema'
    )
  })

  /** Six is as deep as Markdown goes; a deeper heading clamps rather than breaks. */
  it('clamps a heading deeper than markdown admits', () => {
    expect(md(doc({ type: 'heading', attrs: { level: 9 }, content: [text('hondo')] }))).toBe(
      '###### hondo'
    )
  })

  it('carries every mark the format has', () => {
    const marked = p(
      text('negrita', [{ type: 'bold' }]),
      text(' '),
      text('cursiva', [{ type: 'italic' }]),
      text(' '),
      text('tachado', [{ type: 'strike' }]),
      text(' '),
      text('codigo', [{ type: 'code' }])
    )

    expect(md(doc(marked))).toBe('**negrita** *cursiva* ~~tachado~~ `codigo`')
  })

  /**
   * The matrix declares underline a fallback in Markdown. Dropping it would be
   * a silent edit of the manuscript, so it goes out as the tag that every
   * reader allowing HTML will render.
   */
  it('stands in for underline rather than dropping it', () => {
    expect(md(doc(p(text('subrayado', [{ type: 'underline' }]))))).toBe('<u>subrayado</u>')
  })

  /** The same stand-in as underline, for the same reason. */
  it('stands in for subscript and superscript with their tags', () => {
    const chemistry = p(
      text('H'),
      text('2', [{ type: 'subscript' }]),
      text('O, m'),
      text('2', [{ type: 'superscript' }])
    )

    expect(md(doc(chemistry))).toBe('H<sub>2</sub>O, m<sup>2</sup>')
  })

  it('stands in for a relative size with a sized span', () => {
    const sized = p(text('grande', [{ type: 'textStyle', attrs: { fontSize: '1.5em' } }]))

    expect(md(doc(sized))).toBe('<span style="font-size: 1.5em">grande</span>')
  })

  /** A size is written only when it is one of the scale's, so nothing else rides in on it. */
  it('keeps the words of a size it does not recognise, and drops the size', () => {
    const forged = p(
      text('texto', [{ type: 'textStyle', attrs: { fontSize: '1em"><script>x</script>' } }])
    )

    expect(md(doc(forged))).toBe('texto')
  })

  /** GFM has no colour: the same inline-HTML stand-in, in the print colours. */
  it('stands in for a text colour and a highlight with inline HTML', () => {
    const coloured = p(
      text('rojo', [{ type: 'textStyle', attrs: { color: 'red' } }]),
      text(' y '),
      text('marcado', [{ type: 'highlight', attrs: { color: 'yellow' } }])
    )

    expect(md(doc(coloured))).toBe(
      '<span style="color: #9f211c">rojo</span> y ' +
        '<mark style="background-color: #f3d265; color: inherit">marcado</mark>'
    )
  })

  it('keeps the words of a colour it does not know, and drops the colour', () => {
    const forged = p(
      text('a', [{ type: 'textStyle', attrs: { color: '"><script>x</script>' } }]),
      text('b', [{ type: 'highlight', attrs: { color: 'chartreuse' } }])
    )

    expect(md(doc(forged))).toBe('ab')
  })

  it('writes a link with its target', () => {
    const link = [{ type: 'link', attrs: { href: 'https://example.org' } }]

    expect(md(doc(p(text('el sitio', link))))).toBe('[el sitio](https://example.org)')
  })

  /**
   * A literal asterisk in the prose is not emphasis. Escaping is what keeps a
   * manuscript about regular expressions from exporting as italics.
   */
  it('escapes text that would otherwise be syntax', () => {
    expect(md(doc(p(text('2 * 3 y un _guion_'))))).toBe('2 \\* 3 y un \\_guion\\_')
  })
})

describe('the structures', () => {
  it('writes an unordered list', () => {
    const list = {
      type: 'bulletList',
      content: [
        { type: 'listItem', content: [p(text('uno'))] },
        { type: 'listItem', content: [p(text('dos'))] },
      ],
    }

    expect(md(doc(list))).toBe('- uno\n- dos')
  })

  it('numbers an ordered list from where it starts', () => {
    const list = {
      type: 'orderedList',
      attrs: { start: 3 },
      content: [{ type: 'listItem', content: [p(text('tercero'))] }],
    }

    expect(md(doc(list))).toBe('3. tercero')
  })

  /** A nested list stays nested: without the indent it closes the outer one. */
  it('indents a nested list under its bullet', () => {
    const inner = {
      type: 'bulletList',
      content: [{ type: 'listItem', content: [p(text('interno'))] }],
    }
    const outer = {
      type: 'bulletList',
      content: [{ type: 'listItem', content: [p(text('externo')), inner] }],
    }

    expect(md(doc(outer))).toBe('- externo\n\n  - interno')
  })

  it('writes a blockquote across its lines', () => {
    const quote = { type: 'blockquote', content: [p(text('primera')), p(text('segunda'))] }

    expect(md(doc(quote))).toBe('> primera\n>\n> segunda')
  })

  it('fences a code block with its language', () => {
    const code = { type: 'codeBlock', attrs: { language: 'rust' }, content: [text('fn main() {}')] }

    expect(md(doc(code))).toBe('```rust\nfn main() {}\n```')
  })

  it('writes a horizontal rule', () => {
    expect(md(doc({ type: 'horizontalRule' }))).toBe('---')
  })
})

describe('tables', () => {
  const cell = (value: string) => ({ type: 'tableCell', content: [p(text(value))] })
  const header = (value: string) => ({ type: 'tableHeader', content: [p(text(value))] })

  it('writes a table with its separator row', () => {
    const table = {
      type: 'table',
      content: [
        { type: 'tableRow', content: [header('Año'), header('Hecho')] },
        { type: 'tableRow', content: [cell('1919'), cell('Semana Trágica')] },
      ],
    }

    expect(md(doc(table))).toBe('| Año | Hecho |\n| --- | --- |\n| 1919 | Semana Trágica |')
  })

  /**
   * A newline inside a GFM cell ends the table. Flattening the cell keeps the
   * structure around it intact, which matters more than the break inside it.
   */
  it('flattens a cell rather than letting it break the table', () => {
    const table = {
      type: 'table',
      content: [
        {
          type: 'tableRow',
          content: [{ type: 'tableCell', content: [p(text('una')), p(text('otra'))] }],
        },
      ],
    }

    expect(md(doc(table))).toBe('| una otra |\n| --- |')
  })
})

describe('notes and citations', () => {
  const footnoted = doc(
    p(text('una afirmacion'), { type: 'footnoteReference', attrs: { 'data-id': 'f1' } }),
    {
      type: 'footnotes',
      content: [
        { type: 'footnote', attrs: { 'data-id': 'f1' }, content: [p(text('la aclaracion'))] },
      ],
    }
  )

  it('writes a footnote marker where it stands and its body at the end', () => {
    expect(md(footnoted)).toBe('una afirmacion[^1]\n\n[^1]: la aclaracion')
  })

  /**
   * Footnotes and citation notes share one counter. Numbering them separately
   * would print `[^1]` twice on the same page.
   */
  it('numbers real footnotes and citation notes in one sequence', () => {
    const mixed = doc(
      p(
        text('afirma'),
        { type: 'footnoteReference', attrs: { 'data-id': 'f1' } },
        text(' y cita'),
        {
          type: 'documentCitation',
          attrs: { quotedText: 'lo dicho', metadataSnapshot: { title: 'Acta' } },
        }
      ),
      {
        type: 'footnotes',
        content: [{ type: 'footnote', attrs: { 'data-id': 'f1' }, content: [p(text('aclara'))] }],
      }
    )

    expect(md(mixed)).toBe('afirma[^1] y cita[^2]\n\n[^1]: aclara\n[^2]: Acta\\. «lo dicho»')
  })

  it('writes a corpus citation inline when that is what was chosen', () => {
    const cited = doc(
      p({ type: 'documentCitation', attrs: { metadataSnapshot: { title: 'Acta' }, pageNumber: 4 } })
    )

    expect(md(cited, { citations: 'inline' })).toBe('\\(Acta, p\\. 4\\)')
  })

  /**
   * §11.5: the cached `renderedText` is never the source of truth. An export
   * that trusted it would put yesterday's citation style in today's document.
   */
  it('prefers the freshly rendered citation over the one cached on the node', () => {
    const cited = doc(
      p({
        type: 'zoteroCitation',
        attrs: { citationNodeId: 'z1', renderedText: '(Viejo, 1999)' },
      })
    )

    expect(md(cited, { zotero: { z1: '(Acha, 2015)' } })).toBe('\\(Acha, 2015\\)')
  })

  /** A citation that exports as nothing disappears; one that exports as a marker can be found. */
  it('leaves a marker when a citation could not be rendered at all', () => {
    const cited = doc(p({ type: 'zoteroCitation', attrs: { citationNodeId: 'z1' } }))

    expect(md(cited)).toBe('\\[cita\\]')
  })

  it('writes the bibliography under its own heading', () => {
    expect(md(doc(p(text('cuerpo'))), { bibliography: ['Acha, O. (2015). Un libro.'] })).toBe(
      'cuerpo\n\n## Bibliografía\n\nAcha, O\\. \\(2015\\)\\. Un libro\\.'
    )
  })

  it('says nothing about a bibliography the document does not have', () => {
    expect(md(doc(p(text('cuerpo'))))).toBe('cuerpo')
  })
})

describe('what is left of a note link', () => {
  it('reads as the snapshot, with no marker added to the prose', () => {
    const linked = doc(p({ type: 'noteLink', attrs: { contentSnapshot: 'lo anotado' } }))

    expect(md(linked)).toBe('«lo anotado»')
  })
})

/**
 * Markdown has no paragraph formatting. A `<div style>` wraps the block, with
 * blank lines inside it so what it holds is still read as Markdown — the marks
 * stay marks and a heading stays a heading — and the export says so.
 */
describe('paragraph formatting', () => {
  const formatted = (attrs: Record<string, unknown>, ...content: Node[]) => ({
    type: 'paragraph',
    attrs,
    content: content.length > 0 ? content : [text('Texto')],
  })

  it('wraps a formatted paragraph in a styled div, keeping its Markdown', () => {
    const out = md(
      doc(
        formatted(
          { textAlign: 'center', indent: 2, lineHeight: '1.5' },
          text('Hola '),
          text('mundo', [{ type: 'bold' }])
        )
      )
    )

    expect(out).toBe(
      '<div style="text-align: center; margin-left: 4em; line-height: 1.7">\n\n' +
        'Hola **mundo**\n\n</div>'
    )
  })

  it('keeps a heading a Markdown heading inside the div', () => {
    const heading = {
      type: 'heading',
      attrs: { level: 2, textAlign: 'right', indent: null, lineHeight: null },
      content: [text('El problema')],
    }

    expect(md(doc(heading))).toBe('<div style="text-align: right">\n\n## El problema\n\n</div>')
  })

  it('leaves a paragraph with only defaults as it was', () => {
    expect(md(doc(formatted({ textAlign: null, indent: null, lineHeight: '3' })))).toBe('Texto')
  })

  it('wraps a list item’s paragraph inside the item, under its bullet', () => {
    const list = {
      type: 'bulletList',
      content: [
        { type: 'listItem', content: [formatted({ lineHeight: '2' }, text('uno'))] },
        { type: 'listItem', content: [p(text('dos'))] },
      ],
    }

    expect(md(doc(list))).toBe('- <div style="line-height: 2.27">\n\n  uno\n\n  </div>\n- dos')
  })

  /** A GFM cell is one line of inline text; there is no block to wrap. */
  it('drops them in a table cell rather than breaking the table', () => {
    const table = {
      type: 'table',
      content: [
        {
          type: 'tableRow',
          content: [
            { type: 'tableCell', content: [formatted({ textAlign: 'center' }, text('a'))] },
          ],
        },
      ],
    }

    expect(md(doc(table))).toBe('| a |\n| --- |')
  })
})
