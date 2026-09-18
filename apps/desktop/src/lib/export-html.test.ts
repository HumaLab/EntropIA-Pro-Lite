import { describe, expect, it } from 'vitest'
import type { ExportContext, Node } from './export-document'
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
