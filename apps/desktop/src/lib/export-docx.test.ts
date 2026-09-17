import { unzipSync, strFromU8 } from 'fflate'
import { describe, expect, it } from 'vitest'
import type { ExportContext, Node } from './export-document'
import { toDocx } from './export-docx'

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
        content: [{ type: 'footnote', attrs: { 'data-id': 'f1' }, content: [p(text('la aclaracion'))] }],
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
