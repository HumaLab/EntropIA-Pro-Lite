import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { emptyDocument } from './document-contract'

/**
 * Golden serialization of the manuscript schema.
 *
 * A manuscript is stored as the editor's JSON and synced between devices, so
 * the JSON an editor writes must not change when the editor library does: a
 * different shape is a new revision on every device that opens the document,
 * and a dropped mark or link is lost writing. These snapshots were recorded on
 * Tiptap 2 and must hold, unchanged, on whatever version comes next.
 */

const editors: Editor[] = []

afterEach(() => {
  editors.splice(0).forEach((instance) => instance.destroy())
})

function mount(content: JSONContent | string) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  const instance = new Editor({ element, extensions: createWritingExtensions(), content })
  editors.push(instance)
  return instance
}

/** Everything the toolbar and a paste can put into a manuscript, as HTML. */
const RICH_HTML = [
  '<h1>Título</h1>',
  '<h4>Cuarto nivel</h4>',
  '<p style="text-align: center">Centrado <strong>negrita</strong> <em>itálica</em> ',
  '<u>subrayado</u> <s>tachado</s> <code>código</code> H<sub>2</sub>O x<sup>3</sup></p>',
  '<p><a href="https://example.com/a?b=1">https</a> <a href="http://example.com">http</a> ',
  '<a href="mailto:a@b.c">mail</a> <a href="/relative/path">relativo</a> ',
  '<a href="#ancla">ancla</a> <a href="www.example.com">sin esquema</a> ',
  '<a href="javascript:alert(1)">script</a></p>',
  '<p><span style="font-size: 1.5em">grande</span> ',
  '<span style="color: var(--writing-text-green)">verde</span> ',
  '<span style="font-size: 1.25em"><span style="color: var(--writing-text-red)">anidado</span></span> ',
  '<span style="font-size: 12pt">absoluto</span> <mark>resaltado</mark></p>',
  '<ul><li><p>uno</p></li><li><p>dos</p></li></ul>',
  '<ol start="3"><li><p>tres</p></li></ol>',
  '<blockquote><p>cita</p></blockquote>',
  '<pre><code>bloque</code></pre>',
  '<hr>',
  '<table><tbody><tr><th><p>Encabezado</p></th><th><p>Otro</p></th></tr>',
  '<tr><td><p>Celda</p></td><td><p></p></td></tr></tbody></table>',
  '<p>fin<br>línea</p>',
].join('')

/** The atoms and blocks that only ever arrive as stored JSON. */
const ATOMS_JSON: JSONContent = {
  type: 'doc',
  content: [
    {
      type: 'paragraph',
      content: [
        { type: 'text', text: 'Una afirmación' },
        {
          type: 'footnoteReference',
          attrs: { 'data-id': 'ref-1', referenceNumber: '1' },
        },
        { type: 'text', text: ' citada ' },
        {
          type: 'documentCitation',
          attrs: {
            citationNodeId: 'cit-1',
            collectionId: 'col-1',
            itemId: 'item-1',
            assetId: 'asset-1',
            pageNumber: 4,
            startChar: 10,
            endChar: 42,
            sourceTextHash: 'hash',
            quotedText: 'el texto citado',
            metadataSnapshot: { title: 'Fuente' },
            quotedParts: [{ kind: 'text', text: 'el texto citado' }],
          },
        },
        { type: 'text', text: ' y ' },
        {
          type: 'zoteroCitation',
          attrs: {
            citationNodeId: 'zot-1',
            items: [{ itemKey: 'ABCD1234', locator: '12', metadataSnapshot: { id: 'acha2015' } }],
            prefix: 'ver',
            suffix: null,
            renderedText: '(ver Acha, 2015, p. 12)',
          },
        },
        { type: 'text', text: ' con ' },
        {
          type: 'noteLink',
          attrs: {
            noteLinkNodeId: 'nl-1',
            noteId: 'note-1',
            itemId: 'item-1',
            contentSnapshot: 'una nota',
            contentHash: 'nhash',
          },
        },
      ],
    },
    {
      type: 'writingImage',
      attrs: {
        src: 'writing-images/a.png',
        alt: 'Mapa',
        title: null,
        width: 320,
        height: 200,
        align: 'left',
      },
      content: [{ type: 'text', text: 'Leyenda' }],
    },
    { type: 'paragraph', content: [{ type: 'text', text: 'Cierre' }] },
    {
      type: 'footnotes',
      content: [
        {
          type: 'footnote',
          attrs: { id: 'fn:1', 'data-id': 'ref-1' },
          content: [{ type: 'paragraph', content: [{ type: 'text', text: 'La nota.' }] }],
        },
      ],
    },
  ],
}

describe('manuscript JSON (golden)', () => {
  it('parses the rich HTML into the recorded JSON', () => {
    expect(mount(RICH_HTML).getJSON()).toMatchSnapshot()
  })

  it('writes the recorded JSON for citations, notes, images and footnotes', () => {
    expect(mount(ATOMS_JSON).getJSON()).toMatchSnapshot()
  })

  it('mounts its own output without changing a byte', () => {
    for (const source of [RICH_HTML, ATOMS_JSON]) {
      const first = mount(source).getJSON()
      expect(mount(first).getJSON()).toEqual(first)
    }
  })

  /**
   * Copying within or between manuscripts goes through the clipboard's HTML.
   * Everything the rich document holds is drawn into that HTML, so it must
   * come back as the very same JSON.
   */
  it('round-trips the rich document through its own HTML', () => {
    const original = mount(RICH_HTML).getJSON()
    const html = mount(original).getHTML()
    expect(mount(html).getJSON()).toEqual(original)
  })

  it('stores the empty document as recorded', () => {
    expect(mount(emptyDocument().doc).getJSON()).toMatchSnapshot()
  })

  it('adds no paragraph after a document that already ends in one', () => {
    const instance = mount('<blockquote><p>a</p></blockquote><p>x</p>')
    // A trailing-node plugin acts on transactions, so give it one.
    instance.commands.insertContentAt(2, 'z')
    expect(instance.getJSON().content?.map((node) => node.type)).toEqual([
      'blockquote',
      'paragraph',
    ])
  })

  it('appends exactly one paragraph after a trailing table', () => {
    const instance = mount('<p>a</p><table><tbody><tr><td><p>b</p></td></tr></tbody></table>')
    // The trailing paragraph lands on the first transaction, as it always has.
    instance.commands.insertContentAt(1, 'z')
    expect(instance.getJSON().content?.map((node) => node.type)).toEqual([
      'paragraph',
      'table',
      'paragraph',
    ])
    expect(instance.getJSON()).toMatchSnapshot()
  })
})
