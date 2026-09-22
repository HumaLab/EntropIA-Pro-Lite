import { unzipSync, strFromU8 } from 'fflate'
import { describe, expect, it } from 'vitest'
import type { ExportContext, Node } from './export-document'
import { quotedImageSize, toDocx } from './export-docx'

/**
 * DOCX export (plan-editor.md §17.1, §17.4).
 *
 * # Why these tests open the zip
 *
 * Because that is the test that would have caught the incumbent. Spike S4 found
 * `html-docx-js` producing a file Word opens and renders — while containing no
 * document model at all: its content is an MHTML blob in `afchunk.mht` and
 * there is no `footnotes.xml` anywhere. Any test that only asked "did we get a
 * file" would have passed on it.
 *
 * So what is asserted here is the parts inside: that a footnote is a real
 * `w:footnoteReference` into a real `footnotes.xml`, that a table is a `w:tbl`,
 * that a link is a `w:hyperlink`. §17.4 forbids a warning from standing in for
 * any of these, which means they are not allowed to quietly become italic text.
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

/** The parts of the package, as XML text. */
async function parts(node: Node, extra: Partial<ExportContext> = {}) {
  const bytes = await toDocx(node, { ...context, ...extra })
  const files = unzipSync(bytes)
  const read = (path: string) => (files[path] ? strFromU8(files[path]!) : null)
  return { names: Object.keys(files), read }
}

describe('the package has the parts a DOCX has', () => {
  /**
   * The one that settles it. `html-docx-js` has no `footnotes.xml`, which is
   * why S4 replaced it; a regression to any wrapper approach fails here first.
   */
  it('has a real footnotes part', async () => {
    const { names } = await parts(doc(p(text('cuerpo'))))

    expect(names).toContain('word/footnotes.xml')
    expect(names).toContain('word/document.xml')
    expect(names).toContain('word/styles.xml')
    expect(names).toContain('word/numbering.xml')
  })

  /** And no `altChunk`, which is how the incumbent smuggled HTML through. */
  it('carries no altChunk', async () => {
    const { read } = await parts(doc(p(text('cuerpo'))))

    expect(read('word/document.xml')).not.toContain('altChunk')
    expect(read('word/document.xml')).toContain('cuerpo')
  })
})

describe('the obligatory elements of §17.1, as real OOXML', () => {
  it('writes a footnote as a reference into the footnotes part', async () => {
    const footnoted = doc(
      p(text('afirma'), { type: 'footnoteReference', attrs: { 'data-id': 'f1' } }),
      {
        type: 'footnotes',
        content: [
          { type: 'footnote', attrs: { 'data-id': 'f1' }, content: [p(text('la aclaracion'))] },
        ],
      }
    )

    const { read } = await parts(footnoted)

    expect(read('word/document.xml')).toContain('w:footnoteReference')
    expect(read('word/footnotes.xml')).toContain('la aclaracion')
  })

  it('writes a table as a table and not as tabbed text', async () => {
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

    const body = (await parts(doc(table))).read('word/document.xml')!

    expect(body).toContain('<w:tbl>')
    expect(body).toContain('<w:tc>')
    expect(body).toContain('1919')
  })

  it('writes a link as a hyperlink', async () => {
    const linked = doc(p(text('el sitio', [{ type: 'link', attrs: { href: 'https://e.org' } }])))

    const { read, names } = await parts(linked)

    expect(read('word/document.xml')).toContain('w:hyperlink')
    expect(names).toContain('word/_rels/document.xml.rels')
    expect(read('word/_rels/document.xml.rels')).toContain('https://e.org')
  })

  /**
   * A live link that looks like body text is a link nobody clicks. `docx` emits
   * the `w:hyperlink` with a plain run unless the `Hyperlink` character style is
   * asked for by name, and S4 verified this element as "blue, underlined, live"
   * — so the styling is part of what was verified, not a decoration.
   */
  it('makes the hyperlink look like one', async () => {
    const linked = doc(p(text('el sitio', [{ type: 'link', attrs: { href: 'https://e.org' } }])))

    const body = (await parts(linked)).read('word/document.xml')!

    expect(body).toContain('<w:rStyle w:val="Hyperlink"/>')
  })

  /** The same refusal as in HTML, in a format where the link is also live. */
  it('does not write a dangerous target as a hyperlink', async () => {
    const linked = doc(p(text('pulse', [{ type: 'link', attrs: { href: 'javascript:alert(1)' } }])))

    const { read } = await parts(linked)

    expect(read('word/_rels/document.xml.rels') ?? '').not.toContain('javascript:')
    expect(read('word/document.xml')).toContain('pulse')
  })

  it('writes subscript and superscript as vertical alignment', async () => {
    const chemistry = doc(
      p(
        text('H'),
        text('2', [{ type: 'subscript' }]),
        text('O'),
        text('3', [{ type: 'superscript' }])
      )
    )

    const body = (await parts(chemistry)).read('word/document.xml')!

    expect(body).toContain('<w:vertAlign w:val="subscript"/>')
    expect(body).toContain('<w:vertAlign w:val="superscript"/>')
  })

  /**
   * An em is a proportion of the text around it: the body's 12 pt, or the
   * heading style's own size.
   */
  it('writes a relative size against the size of the paragraph it is in', async () => {
    const sized = [{ type: 'textStyle', attrs: { fontSize: '1.5em' } }]
    const body = (
      await parts(
        doc(p(text('cuerpo', sized)), {
          type: 'heading',
          attrs: { level: 1 },
          content: [text('titulo', sized)],
        })
      )
    ).read('word/document.xml')!

    // 12 pt × 1.5 = 18 pt, and 16 pt × 1.5 = 24 pt; OOXML counts half-points.
    expect(body).toContain('<w:sz w:val="36"/>')
    expect(body).toContain('<w:sz w:val="48"/>')
  })

  /**
   * Word's named highlights are a fixed set of sixteen loud colours, none of
   * them the palette's; shading takes any fill, so the print colour goes out
   * exactly as the HTML export writes it.
   */
  it('writes a text colour as run colour and a highlight as shading, in print colours', async () => {
    const body = (
      await parts(
        doc(
          p(
            text('rojo', [{ type: 'textStyle', attrs: { color: 'red' } }]),
            text('marcado', [{ type: 'highlight', attrs: { color: 'yellow' } }])
          )
        )
      )
    ).read('word/document.xml')!

    expect(body).toContain('<w:color w:val="9F211C"/>')
    expect(body).toMatch(/<w:shd [^>]*w:fill="F3D265"/)
  })

  it('writes no colour for a name that is not in the palette', async () => {
    const forged = doc(
      p(
        text('a', [{ type: 'textStyle', attrs: { color: 'chartreuse' } }]),
        text('b', [{ type: 'highlight', attrs: { color: 'ultraviolet' } }])
      )
    )
    const body = (await parts(forged)).read('word/document.xml')!

    expect(body).not.toContain('<w:color ')
    expect(body).not.toContain('<w:shd ')
  })

  it('writes no size for a size it does not recognise', async () => {
    const forged = doc(p(text('texto', [{ type: 'textStyle', attrs: { fontSize: '12pt' } }])))

    expect((await parts(forged)).read('word/document.xml')).not.toContain('<w:sz ')
  })

  it('writes headings with heading styles', async () => {
    const headed = doc({ type: 'heading', attrs: { level: 2 }, content: [text('El problema')] })

    expect((await parts(headed)).read('word/document.xml')).toContain('Heading2')
  })

  it('numbers an ordered list through the numbering part', async () => {
    const list = {
      type: 'orderedList',
      content: [{ type: 'listItem', content: [p(text('primero'))] }],
    }

    expect((await parts(doc(list))).read('word/document.xml')).toContain('w:numPr')
  })

  it('marks a bullet list as a list', async () => {
    const list = {
      type: 'bulletList',
      content: [{ type: 'listItem', content: [p(text('uno'))] }],
    }

    expect((await parts(doc(list))).read('word/document.xml')).toContain('w:numPr')
  })
})

describe('citations and bibliography', () => {
  it('writes a corpus citation as a footnote when that was chosen', async () => {
    const cited = doc(
      p({
        type: 'documentCitation',
        attrs: { quotedText: 'lo dicho', metadataSnapshot: { title: 'Acta' } },
      })
    )

    const { read } = await parts(cited)

    expect(read('word/document.xml')).toContain('w:footnoteReference')
    expect(read('word/footnotes.xml')).toContain('Acta. «lo dicho»')
  })

  /**
   * §17.2's comment option is only offered where the format has one. S4
   * confirmed `docx` emits `comments.xml`, which is what makes it reachable.
   */
  it('writes a corpus citation as a real comment when that was chosen', async () => {
    const cited = doc(
      p({ type: 'documentCitation', attrs: { metadataSnapshot: { title: 'Acta' } } })
    )

    const { names, read } = await parts(cited, { citations: 'comment' })

    expect(names).toContain('word/comments.xml')
    expect(read('word/comments.xml')).toContain('Acta')
    expect(read('word/document.xml')).toContain('commentRangeStart')
  })

  /** §17.3: the CSL rendering as text, which is what the MVP promises. */
  it('writes the Zotero citation the style produced, not the one cached', async () => {
    const cited = doc(
      p({ type: 'zoteroCitation', attrs: { citationNodeId: 'z1', renderedText: '(Viejo, 1999)' } })
    )

    const body = (await parts(cited, { zotero: { z1: '(Acha, 2015)' } })).read('word/document.xml')!

    expect(body).toContain('(Acha, 2015)')
    expect(body).not.toContain('(Viejo, 1999)')
  })

  /** §11.6: the entry hangs, so the author is what the eye finds. */
  it('gives each bibliography entry a hanging indent', async () => {
    const body = (
      await parts(doc(p(text('cuerpo'))), { bibliography: ['Acha, O. (2015). Un libro.'] })
    ).read('word/document.xml')!

    expect(body).toContain('w:hanging')
    expect(body).toContain('Acha, O. (2015). Un libro.')
  })
})

describe('what a note link becomes', () => {
  it('keeps the snapshot and says it was a note', async () => {
    const linked = doc(p({ type: 'noteLink', attrs: { contentSnapshot: 'lo anotado' } }))

    expect((await parts(linked)).read('word/document.xml')).toContain('lo anotado')
  })
})

/**
 * Paragraph formatting as the paragraph's own properties: `w:jc` for the
 * alignment, a left indent in twips per level, and line spacing as a multiple
 * of single (240) with the automatic rule.
 */
describe('paragraph formatting', () => {
  const formatted = (attrs: Record<string, unknown>, value = 'Texto') => ({
    type: 'paragraph',
    attrs,
    content: [text(value)],
  })
  const body = async (...content: Node[]) =>
    (await parts(doc(...content))).read('word/document.xml')!

  it('writes each alignment, justify as both sides', async () => {
    const xml = await body(
      formatted({ textAlign: 'center' }),
      formatted({ textAlign: 'right' }),
      formatted({ textAlign: 'justify' })
    )

    expect(xml).toContain('<w:jc w:val="center"/>')
    expect(xml).toContain('<w:jc w:val="right"/>')
    expect(xml).toContain('<w:jc w:val="both"/>')
  })

  it('indents 720 twips a level', async () => {
    const xml = await body(formatted({ indent: 1 }), formatted({ indent: 3 }))

    expect(xml).toMatch(/<w:ind w:left="720"\/>/)
    expect(xml).toMatch(/<w:ind w:left="2160"\/>/)
  })

  it('writes line spacing as a multiple of single, 240 being one', async () => {
    const xml = await body(
      formatted({ lineHeight: '1' }),
      formatted({ lineHeight: '1.15' }),
      formatted({ lineHeight: '1.5' }),
      formatted({ lineHeight: '2' })
    )
    const lines = [...xml.matchAll(/<w:spacing ([^>]*)\/>/g)].map(([, attrs]) => attrs)

    expect(lines).toEqual([
      'w:line="240" w:lineRule="auto"',
      'w:line="276" w:lineRule="auto"',
      'w:line="360" w:lineRule="auto"',
      'w:line="480" w:lineRule="auto"',
    ])
  })

  it('formats a heading and keeps its heading style', async () => {
    const xml = await body({
      type: 'heading',
      attrs: { level: 1, textAlign: 'center', indent: 1, lineHeight: '2' },
      content: [text('Título')],
    })

    expect(xml).toContain('<w:pStyle w:val="Heading1"/>')
    expect(xml).toContain('<w:jc w:val="center"/>')
    expect(xml).toContain('w:left="720"')
    expect(xml).toContain('w:line="480"')
  })

  it('adds the indent to a quotation’s own, and aligns a list item', async () => {
    const quote = { type: 'blockquote', content: [formatted({ indent: 1 })] }
    const list = {
      type: 'orderedList',
      content: [{ type: 'listItem', content: [formatted({ textAlign: 'right' }, 'uno')] }],
    }
    const xml = await body(quote, list)

    expect(xml).toContain('w:left="1287"')
    expect(xml).toMatch(/<w:numPr>[\s\S]*?<w:jc w:val="right"\/>/)
  })

  it('writes nothing for the defaults', async () => {
    const xml = await body(formatted({ textAlign: null, indent: null, lineHeight: '3' }))

    expect(xml).not.toContain('<w:jc ')
    expect(xml).not.toContain('<w:ind ')
    expect(xml).not.toContain('w:lineRule')
  })
})

/**
 * The package ships no body size (Word then shows 10 pt) and its heading
 * styles stop at H3 = body. A manuscript needs a scale where each level is a
 * visible step: body 12, H1 16, H2 14, H3 13, H4 12 in bold, notes 10.
 */
describe('the type scale', () => {
  function styleBlock(styles: string, id: string): string {
    const at = styles.indexOf(`w:styleId="${id}"`)
    expect(at, `${id} is not in styles.xml`).toBeGreaterThan(-1)
    return styles.slice(at, styles.indexOf('</w:style>', at))
  }

  it('sets the body, each heading level and the notes', async () => {
    const styles = (await parts(doc(p(text('cuerpo'))))).read('word/styles.xml')!
    const defaults = styles.slice(
      styles.indexOf('<w:docDefaults>'),
      styles.indexOf('</w:docDefaults>')
    )

    expect(defaults).toContain('<w:sz w:val="24"/>')
    expect(styleBlock(styles, 'Heading1')).toContain('<w:sz w:val="32"/>')
    expect(styleBlock(styles, 'Heading2')).toContain('<w:sz w:val="28"/>')
    expect(styleBlock(styles, 'Heading3')).toContain('<w:sz w:val="26"/>')
    expect(styleBlock(styles, 'Heading4')).toContain('<w:sz w:val="24"/>')
    expect(styleBlock(styles, 'Heading4')).toContain('<w:b/>')
    expect(styleBlock(styles, 'FootnoteText')).toContain('<w:sz w:val="20"/>')
  })

  it('keeps the heading colours the package gives them', async () => {
    const styles = (await parts(doc(p(text('cuerpo'))))).read('word/styles.xml')!
    expect(styleBlock(styles, 'Heading1')).toContain('<w:color w:val="2E74B5"/>')
    expect(styleBlock(styles, 'Heading3')).toContain('<w:color w:val="1F4D78"/>')
  })
})

/**
 * Spacing around blocks, in twentieths of a point: 6 pt before and after a
 * paragraph, 12 pt before and 6 pt after a heading. Notes stay tight — the
 * package gives them none after, and they must not inherit the body's 6 before.
 */
describe('the spacing between blocks', () => {
  function styleBlock(styles: string, id: string): string {
    const at = styles.indexOf(`w:styleId="${id}"`)
    expect(at, `${id} is not in styles.xml`).toBeGreaterThan(-1)
    return styles.slice(at, styles.indexOf('</w:style>', at))
  }
  const spacingOf = (xml: string) => /<w:spacing [^>]*\/>/.exec(xml)?.[0] ?? ''

  it('sets 6 pt before and after a paragraph', async () => {
    const styles = (await parts(doc(p(text('cuerpo'))))).read('word/styles.xml')!
    const defaults = styles.slice(
      styles.indexOf('<w:docDefaults>'),
      styles.indexOf('</w:docDefaults>')
    )
    expect(spacingOf(defaults)).toContain('w:before="120"')
    expect(spacingOf(defaults)).toContain('w:after="120"')
  })

  it('sets 12 pt before and 6 pt after every heading level', async () => {
    const styles = (await parts(doc(p(text('cuerpo'))))).read('word/styles.xml')!
    for (const id of ['Heading1', 'Heading2', 'Heading3', 'Heading4']) {
      expect(spacingOf(styleBlock(styles, id)), id).toContain('w:before="240"')
      expect(spacingOf(styleBlock(styles, id)), id).toContain('w:after="120"')
    }
  })

  it('keeps footnotes without space before or after', async () => {
    const styles = (await parts(doc(p(text('cuerpo'))))).read('word/styles.xml')!
    expect(spacingOf(styleBlock(styles, 'FootnoteText'))).toContain('w:before="0"')
    expect(spacingOf(styleBlock(styles, 'FootnoteText'))).toContain('w:after="0"')
  })

  it('leaves a paragraph with its own line spacing to inherit the space around it', async () => {
    const body = (
      await parts(doc({ type: 'paragraph', attrs: { lineHeight: '1.5' }, content: [text('x')] }))
    ).read('word/document.xml')!
    const spacing = spacingOf(body)
    expect(spacing).toContain('w:line="360"')
    expect(spacing).not.toContain('w:before')
    expect(spacing).not.toContain('w:after')
  })
})

// A quote keeps the page's line breaks; DOCX writes them as <w:br/>.
describe('a quote that spans lines', () => {
  it('breaks the line inside the note', async () => {
    const cited = doc(
      p({
        type: 'documentCitation',
        attrs: { quotedText: 'SOLICITADA\nA mis compañeros', metadataSnapshot: { title: 'Acta' } },
      })
    )

    const { read } = await parts(cited)
    const notes = read('word/footnotes.xml')

    expect(notes).toContain('SOLICITADA')
    expect(notes).toContain('A mis compañeros')
    expect(notes).toContain('<w:br/>')
  })
})

// The manuscript sets a long quote off as a block; in DOCX that is the same
// indented, bordered paragraph a blockquote gets.
describe('a long quote is set off as a block', () => {
  it('indents the paragraph that holds only a long quote', async () => {
    const cited = doc(
      p({
        type: 'documentCitation',
        attrs: {
          quotedText: 'SOLICITADA\nA mis compañeros',
          metadataSnapshot: { title: 'Acta' },
        },
      })
    )

    const { read } = await parts(cited, { citations: 'quote_with_note' })
    const document = read('word/document.xml')

    expect(document).toContain('<w:ind ')
    expect(document).toContain('<w:pBdr>')
  })
})

describe('a citation that quoted an image', () => {
  const parts_ = [
    { kind: 'text', text: 'antes' },
    { kind: 'image', source: 'writing-crops/uno.png' },
    { kind: 'text', text: 'después' },
  ]
  const cited = {
    type: 'documentCitation',
    attrs: {
      quotedText: 'antes\ndespués',
      metadataSnapshot: { title: 'Diario' },
      quotedParts: parts_,
    },
  }
  /** Four bytes of nothing: Word never opens this, the packer never looks. */
  const image = {
    bytes: new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
    mediaType: 'image/png',
    dataUrl: 'data:image/png;base64,AAAA',
    width: 800,
    height: 400,
  }
  const images = { 'writing-crops/uno.png': image }

  it('puts the image in the package and draws it inside the quotation', async () => {
    const { names, read } = await parts(doc(p(cited)), { citations: 'quote_with_note', images })

    expect(names.some((name) => name.startsWith('word/media/'))).toBe(true)
    const body = read('word/document.xml') ?? ''
    expect(body).toContain('<w:drawing>')
    expect(body.indexOf('antes')).toBeLessThan(body.indexOf('<w:drawing>'))
    expect(body.indexOf('<w:drawing>')).toBeLessThan(body.indexOf('después'))
  })

  /**
   * Academic typesetting sets a long quotation a point smaller than the body,
   * which is what tells the eye it is quoted before it reads a word of it. The
   * body is 12 pt, so the quotation is 11 pt — 22 half-points.
   */
  it('sets a long quotation one point smaller than the body', async () => {
    const { read } = await parts(doc(p(cited)), { citations: 'quote_with_note', images })

    expect(read('word/document.xml') ?? '').toContain('w:sz w:val="22"')
  })

  it('leaves a short quotation at the size of the text around it', async () => {
    const brief = {
      type: 'documentCitation',
      attrs: { quotedText: 'dos palabras', metadataSnapshot: { title: 'Diario' } },
    }

    const { read } = await parts(doc(p(brief)), { citations: 'quote_with_note' })

    expect(read('word/document.xml') ?? '').not.toContain('w:sz w:val="22"')
  })

  /**
   * In a footnote the quotation is one paragraph — a note is not a block on
   * the page — so there the image stays a run in the line, and a break on each
   * side is what keeps the words from sitting alongside the picture.
   */
  it('gives the image a line of its own inside a footnote', async () => {
    const { read } = await parts(doc(p(cited)), { citations: 'footnote', images })

    const note = read('word/footnotes.xml') ?? ''
    const before = note.slice(note.indexOf('antes'), note.indexOf('<w:drawing>'))
    const after = note.slice(note.indexOf('<w:drawing>'), note.indexOf('despu'))
    expect(before).toContain('<w:br/>')
    expect(after).toContain('<w:br/>')
  })

  /**
   * Word has no picture inside a paragraph that a writer can move on its own:
   * alignment, spacing and indentation all belong to the paragraph. With the
   * image inside the quotation's paragraph, centring the image centres the
   * words too. So the quotation becomes three paragraphs — words, image, words
   * — which share the quotation's border and indent and are therefore still
   * one block on the page.
   */
  it('gives the image a paragraph of its own, inside the same block', async () => {
    const { read } = await parts(doc(p(cited)), { citations: 'quote_with_note', images })

    const body = (read('word/document.xml') ?? '').split('<w:sectPr')[0] ?? ''
    const paragraphs = body.split('<w:p>').slice(1)
    expect(paragraphs).toHaveLength(3)
    expect(paragraphs[0]).toContain('antes')
    expect(paragraphs[1]).toContain('<w:drawing>')
    // Nothing but the picture: what the writer centres is the picture.
    expect(paragraphs[1]).not.toContain('<w:t')
    expect(paragraphs[2]).toContain('despu')
    // Still one block: every one of them carries the quotation's left border
    // and its indent, which is what keeps Word drawing a single rule beside
    // the three of them.
    for (const paragraph of paragraphs) {
      expect(paragraph).toContain('<w:pBdr>')
      expect(paragraph).toContain('w:ind w:left="567"')
    }
    // The footnote marker stays at the end of the quotation, not in the middle.
    expect(paragraphs[2]).toContain('w:footnoteReference')
  })

  /**
   * The same reason: a blank line in the quotation was a paragraph break on
   * the page, and a break run gives no space between paragraphs. Only inside
   * the block — in a footnote the quotation stays one paragraph.
   */
  it('makes a blank line inside the quotation a paragraph of its own', async () => {
    const twoParagraphs = {
      type: 'documentCitation',
      attrs: {
        quotedText: 'primer párrafo largo de la cita\n\nsegundo párrafo de la misma cita',
        metadataSnapshot: { title: 'Diario' },
      },
    }

    const { read } = await parts(doc(p(twoParagraphs)), { citations: 'quote_with_note' })

    const body = (read('word/document.xml') ?? '').split('<w:sectPr')[0] ?? ''
    const paragraphs = body.split('<w:p>').slice(1)
    expect(paragraphs).toHaveLength(2)
    expect(paragraphs[0]).toContain('primer')
    expect(paragraphs[1]).toContain('segundo')
  })

  it('writes the words alone when the image file could not be read', async () => {
    const { names, read } = await parts(doc(p(cited)), { citations: 'quote_with_note' })

    expect(names.some((name) => name.startsWith('word/media/'))).toBe(false)
    expect(read('word/document.xml') ?? '').toContain('antes')
  })

  /**
   * A crop of a scan is far wider than a column of Word. Scaled to the column,
   * keeping its shape: 800×400 at 540 wide is 270 high.
   */
  it('scales a wide image down to the column, keeping its proportions', () => {
    expect(quotedImageSize(image)).toEqual({ width: 540, height: 270 })
  })

  it('leaves a small image at its own size', () => {
    expect(quotedImageSize({ ...image, width: 200, height: 100 })).toEqual({
      width: 200,
      height: 100,
    })
  })

  it('refuses an image whose size the file never declared', () => {
    expect(quotedImageSize({ ...image, width: 0, height: 0 })).toBeNull()
  })
})

describe('a manuscript image', () => {
  it('embeds the image bytes and emits the caption', async () => {
    const { names, read } = await parts(
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
      {
        images: {
          'writing-images/abc.png': {
            bytes: new Uint8Array([1, 2, 3]),
            mediaType: 'image/png',
            dataUrl: 'data:image/png;base64,AQID',
            width: 300,
            height: 150,
          },
        },
      }
    )

    expect(names.some((name) => name.startsWith('word/media/'))).toBe(true)
    expect(read('word/document.xml')).toContain('Vista del taller.')
  })
})
