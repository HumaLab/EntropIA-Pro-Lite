import { describe, expect, it } from 'vitest'
import type { ExportContext, ExportImage, Node } from './export-document'
import { safeHref, toHtml } from './export-html'

/**
 * HTML export (plan-editor.md §17.1).
 *
 * §17.1 asks for *sanitized* HTML. The strongest form of that is not running a
 * sanitizer over generated markup — it is never passing markup through. So what
 * is asserted here is that nothing the writer typed can become a tag, and that
 * the one string the browser will act on, a link target, is checked.
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
const html = (node: Node, extra: Partial<ExportContext> = {}) =>
  toHtml(node, { ...context, ...extra })

describe('nothing the writer typed becomes markup', () => {
  /** The whole sanitization story in one assertion. */
  it('escapes a script the writer typed as if it were prose', () => {
    const out = html(doc(p(text('<script>alert(1)</script>'))))

    expect(out).not.toContain('<script>')
    expect(out).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
  })

  it('escapes quotes and ampersands', () => {
    expect(html(doc(p(text(`Tom & "Jerry"`))))).toContain('Tom &amp; &quot;Jerry&quot;')
  })

  it('escapes the title too', () => {
    expect(html(doc(), { title: '<b>Titulo</b>' })).toContain('&lt;b&gt;Titulo&lt;/b&gt;')
  })
})

describe('the one string the browser acts on', () => {
  it('admits the schemes a citation could legitimately use', () => {
    expect(safeHref('https://example.org')).toBe('https://example.org')
    expect(safeHref('mailto:alguien@example.org')).toBe('mailto:alguien@example.org')
    expect(safeHref('#seccion')).toBe('#seccion')
  })

  it('refuses the schemes that are the attack', () => {
    expect(safeHref('javascript:alert(1)')).toBeNull()
    expect(safeHref('data:text/html;base64,PHNjcmlwdD4=')).toBeNull()
    expect(safeHref('vbscript:msgbox')).toBeNull()
  })

  it('refuses something that is not a target at all', () => {
    expect(safeHref('no es una url')).toBeNull()
    expect(safeHref(42)).toBeNull()
    expect(safeHref('   ')).toBeNull()
  })

  /**
   * A refused target keeps its words. Dropping them would delete prose the
   * writer wrote, which is worse than a link that does nothing.
   */
  it('keeps the words of a link whose target was refused', () => {
    const out = html(
      doc(p(text('pulse aqui', [{ type: 'link', attrs: { href: 'javascript:x' } }])))
    )

    expect(out).not.toContain('javascript:')
    expect(out).toContain('pulse aqui')
  })

  it('writes an admitted link with its target', () => {
    const out = html(doc(p(text('el sitio', [{ type: 'link', attrs: { href: 'https://e.org' } }]))))

    expect(out).toContain('<a href="https://e.org" rel="noopener noreferrer">el sitio</a>')
  })
})

describe('the structures §17.1 requires', () => {
  it('writes headings at their level', () => {
    expect(html(doc({ type: 'heading', attrs: { level: 3 }, content: [text('Tres')] }))).toContain(
      '<h3>Tres</h3>'
    )
  })

  it('writes each mark as its element', () => {
    const marks = p(
      text('n', [{ type: 'bold' }]),
      text('c', [{ type: 'italic' }]),
      text('t', [{ type: 'strike' }]),
      text('s', [{ type: 'underline' }]),
      text('k', [{ type: 'code' }])
    )

    expect(html(doc(marks))).toContain(
      '<p><strong>n</strong><em>c</em><s>t</s><u>s</u><code>k</code></p>'
    )
  })

  it('writes subscript and superscript as their elements', () => {
    const chemistry = p(
      text('H'),
      text('2', [{ type: 'subscript' }]),
      text('O'),
      text('2', [{ type: 'superscript' }])
    )

    expect(html(doc(chemistry))).toContain('<p>H<sub>2</sub>O<sup>2</sup></p>')
  })

  it('writes a relative size as a sized span, in em so it follows the text around it', () => {
    const sized = p(text('grande', [{ type: 'textStyle', attrs: { fontSize: '1.5em' } }]))

    expect(html(doc(sized))).toContain('<p><span style="font-size: 1.5em">grande</span></p>')
  })

  /** The size is an attribute value the file carries; only the scale's own get through. */
  it('refuses a size that is not one of the scale, keeping the words', () => {
    const forged = p(
      text('texto', [{ type: 'textStyle', attrs: { fontSize: '1em" onmouseover="alert(1)' } }])
    )

    const out = html(doc(forged))
    expect(out).toContain('<p>texto</p>')
    expect(out).not.toContain('onmouseover')
  })

  /**
   * A manuscript stores palette names; an export is read on white paper, so
   * each name goes out as its one print colour (PRINT_COLORS).
   */
  it('writes a text colour and a highlight in their print colours', () => {
    const coloured = p(
      text('rojo', [{ type: 'textStyle', attrs: { fontSize: '1.5em', color: 'red' } }]),
      text(' y '),
      text('marcado', [{ type: 'highlight', attrs: { color: 'yellow' } }])
    )

    expect(html(doc(coloured))).toContain(
      '<p><span style="font-size: 1.5em; color: #9f211c">rojo</span> y ' +
        '<mark style="background-color: #f3d265; color: inherit">marcado</mark></p>'
    )
  })

  /** A name this build does not know, or a forged one, never reaches the file. */
  it('writes no colour for a name that is not in the palette, keeping the words', () => {
    const forged = p(
      text('a', [{ type: 'textStyle', attrs: { color: 'red" onmouseover="alert(1)' } }]),
      text('b', [{ type: 'highlight', attrs: { color: 'chartreuse' } }])
    )

    const out = html(doc(forged))
    expect(out).toContain('<p>ab</p>')
    expect(out).not.toContain('onmouseover')
  })

  it('writes an ordered list that starts where it starts', () => {
    const list = {
      type: 'orderedList',
      attrs: { start: 4 },
      content: [{ type: 'listItem', content: [p(text('cuarto'))] }],
    }

    expect(html(doc(list))).toContain('<ol start="4">')
  })

  it('writes a table with header cells', () => {
    const table = {
      type: 'table',
      content: [
        {
          type: 'tableRow',
          content: [
            { type: 'tableHeader', content: [p(text('Año'))] },
            { type: 'tableCell', content: [p(text('1919'))] },
          ],
        },
      ],
    }

    expect(html(doc(table))).toContain('<tr><th><p>Año</p></th><td><p>1919</p></td></tr>')
  })

  it('gives the bibliography a hanging indent', () => {
    const out = html(doc(p(text('cuerpo'))), { bibliography: ['Acha, O. (2015).'] })

    expect(out).toContain('.bibliography p { padding-left: 2em; text-indent: -2em;')
    expect(out).toContain('<p>Acha, O. (2015).</p>')
  })
})

describe('notes', () => {
  const footnoted = doc(
    p(text('afirma'), { type: 'footnoteReference', attrs: { 'data-id': 'f1' } }),
    {
      type: 'footnotes',
      content: [{ type: 'footnote', attrs: { 'data-id': 'f1' }, content: [p(text('aclara'))] }],
    }
  )

  /** A reader who follows a marker down has to be able to get back up. */
  it('links the marker to the note and the note back to the marker', () => {
    const out = html(footnoted)

    expect(out).toContain('<sup class="fn"><a id="fnref-1" href="#fn-1">1</a></sup>')
    expect(out).toContain('<li id="fn-1">aclara <a href="#fnref-1"')
  })

  /**
   * The matrix calls a comment in HTML a fallback: an aside marked as a note,
   * because HTML has no comment a reader sees.
   */
  it('stands in for a comment with a marked aside', () => {
    const cited = doc(
      p({ type: 'documentCitation', attrs: { metadataSnapshot: { title: 'Acta' } } })
    )

    expect(html(cited, { citations: 'comment' })).toContain('role="note"')
  })
})

describe('the file stands on its own', () => {
  it('carries its doctype, charset and style', () => {
    const out = html(doc(p(text('cuerpo'))))

    expect(out.startsWith('<!doctype html>')).toBe(true)
    expect(out).toContain('<meta charset="utf-8">')
    expect(out).toContain('<style>')
  })
})

/**
 * Paragraph formatting goes out as inline style on the block itself, from the
 * fixed sets the editor stores, so nothing the file carries reaches the
 * attribute as written.
 */
describe('paragraph formatting', () => {
  const formatted = (attrs: Record<string, unknown>, value = 'Texto') => ({
    type: 'paragraph',
    attrs,
    content: [text(value)],
  })

  it('styles a paragraph with its alignment, indent and line spacing', () => {
    const out = html(doc(formatted({ textAlign: 'center', indent: 2, lineHeight: '1.5' })))

    expect(out).toContain(
      '<p style="text-align: center; margin-left: 4em; line-height: 1.7">Texto</p>'
    )
  })

  it('styles a heading the same way and keeps it a heading', () => {
    const heading = {
      type: 'heading',
      attrs: { level: 2, textAlign: 'justify', indent: 1, lineHeight: null },
      content: [text('El problema')],
    }

    expect(html(doc(heading))).toContain(
      '<h2 style="text-align: justify; margin-left: 2em">El problema</h2>'
    )
  })

  it('writes a plain block for the defaults and for values no build writes', () => {
    const out = html(
      doc(
        formatted({ textAlign: null, indent: null, lineHeight: null }, 'uno'),
        formatted({ textAlign: 'left', indent: 0, lineHeight: '3' }, 'dos'),
        formatted({ textAlign: 'center"><script>', indent: '2em', lineHeight: '1;x' }, 'tres')
      )
    )

    expect(out).toContain('<p>uno</p>')
    expect(out).toContain('<p>dos</p>')
    expect(out).toContain('<p>tres</p>')
    expect(out).not.toContain('<script>')
  })

  it('keeps them inside a table cell and a list item', () => {
    const table = {
      type: 'table',
      content: [
        {
          type: 'tableRow',
          content: [{ type: 'tableCell', content: [formatted({ textAlign: 'right' }, '1919')] }],
        },
      ],
    }
    const list = {
      type: 'bulletList',
      content: [{ type: 'listItem', content: [formatted({ lineHeight: '2' }, 'item')] }],
    }
    const out = html(doc(table, list))

    expect(out).toContain('<td><p style="text-align: right">1919</p></td>')
    expect(out).toContain('<li><p style="line-height: 2.27">item</p></li>')
  })
})

// A quote keeps the page's line breaks; HTML writes them as <br />.
describe('a quote that spans lines', () => {
  it('writes each break as a line break', () => {
    const cited = doc(
      p({
        type: 'documentCitation',
        attrs: { quotedText: 'SOLICITADA\nA mis compañeros', metadataSnapshot: { title: 'Acta' } },
      })
    )

    expect(html(cited)).toContain('«SOLICITADA<br />A mis compañeros»')
  })
})

// The manuscript sets a long quote off as a block; HTML's block is the
// blockquote, in the place of the paragraph that held only that citation.
describe('a long quote is set off as a block', () => {
  const longQuote = doc(
    p({
      type: 'documentCitation',
      attrs: {
        quotedText: 'SOLICITADA\nA mis compañeros',
        metadataSnapshot: { title: 'Acta' },
      },
    })
  )

  it('writes a blockquote instead of a paragraph', () => {
    const out = html(longQuote, { citations: 'quote_with_note' })

    expect(out).toContain('<blockquote class="cite-block">')
    expect(out).not.toContain('<p><span class="cite">«SOLICITADA')
  })

  it('carries the style that boxes it, indented on both sides', () => {
    expect(html(longQuote, { citations: 'quote_with_note' })).toContain(
      'blockquote.cite-block { margin: 1.5em 10%;'
    )
  })

  it('leaves a short quote in its paragraph', () => {
    const short = doc(
      p({
        type: 'documentCitation',
        attrs: { quotedText: 'no habia ley', metadataSnapshot: { title: 'Acta' } },
      })
    )

    expect(html(short, { citations: 'quote_with_note' })).not.toContain(
      '<blockquote class="cite-block"'
    )
  })
})

describe('a citation that quoted an image', () => {
  const parts = [
    { kind: 'text', text: 'antes' },
    { kind: 'image', source: 'writing-crops/uno.png' },
    { kind: 'text', text: 'después' },
  ]
  const cited = {
    type: 'documentCitation',
    attrs: {
      quotedText: 'antes\ndespués',
      metadataSnapshot: { title: 'Diario' },
      quotedParts: parts,
    },
  }
  const images = {
    'writing-crops/uno.png': {
      bytes: new Uint8Array([1]),
      mediaType: 'image/png',
      dataUrl: 'data:image/png;base64,AAAA',
      width: 400,
      height: 200,
    },
  }

  it('embeds the image where it was quoted', () => {
    const out = html(doc(p(cited)), { citations: 'quote_with_note', images })

    expect(out).toContain('<img src="data:image/png;base64,AAAA"')
    expect(out.indexOf('antes')).toBeLessThan(out.indexOf('<img'))
    expect(out.indexOf('<img')).toBeLessThan(out.indexOf('después'))
    // The words are still the quotation: the guillemets stay around them.
    expect(out).toContain('«antes')
    expect(out).toContain('después»')
  })

  it('puts the image in the footnote, where the quotation is', () => {
    const out = html(doc(p(cited)), { citations: 'footnote', images })

    const note = out.slice(out.indexOf('<section class="footnotes">'))
    expect(note).toContain('<img src="data:image/png;base64,AAAA"')
  })

  it('writes the words alone when the image file could not be read', () => {
    const out = html(doc(p(cited)), { citations: 'quote_with_note' })

    expect(out).not.toContain('<img')
    expect(out).toContain('«antes<br />después»')
  })
})

describe('a manuscript image', () => {
  const sampleImage: ExportImage = {
    bytes: new Uint8Array([1, 2, 3]),
    mediaType: 'image/png',
    dataUrl: 'data:image/png;base64,AQID',
    width: 300,
    height: 150,
  }

  it('emits a figure with the embedded image and its caption', () => {
    const out = html(
      doc({
        type: 'writingImage',
        attrs: {
          src: 'writing-images/abc.png',
          alt: 'Vista',
          title: null,
          width: 300,
          height: 150,
          align: 'center',
        },
        content: [text('Vista del taller.')],
      }),
      { images: { 'writing-images/abc.png': sampleImage } }
    )

    expect(out).toContain('<figure')
    expect(out).toContain(`src="${sampleImage.dataUrl}"`)
    expect(out).toContain('Vista del taller.')
  })
})

describe('a long quotation in HTML', () => {
  /** A point smaller than the body, as academic typesetting sets one off. */
  it('is set one point smaller than the text around it', () => {
    const out = html(
      doc(
        p({
          type: 'documentCitation',
          attrs: { quotedText: 'una cita\ncon un salto', metadataSnapshot: { title: 'Diario' } },
        })
      ),
      { citations: 'quote_with_note' }
    )

    expect(out).toContain('<blockquote class="cite-block">')
    expect(out).toContain('blockquote.cite-block')
    expect(out).toContain('font-size: calc(1em - 1pt)')
  })
})
