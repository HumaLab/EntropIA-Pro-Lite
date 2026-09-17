import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { citeWork, worksOf } from './citation-cluster'
import { createWritingExtensions } from './extensions'
import { parseCanonical, WRITING_SCHEMA_VERSION } from './document-contract'

/**
 * Documents written by earlier development builds (plan-editor.md §9, Unit 9).
 *
 * # What this is protecting
 *
 * The manuscripts written *while* the section was being built. They are the
 * only ones in existence, they belong to the person who built it with us, and
 * every one of them predates at least one change of node shape.
 *
 * The schema version is not the risk: it was never bumped, so they all read as
 * version 1 and the guard in `validateCanonical` never fires. The risk is the
 * shapes underneath it — a citation that held one work before it held a
 * cluster, a document saved before `footnotes` existed — which the version
 * number says nothing about.
 *
 * So these are the real shapes, from the real changes this codebase made, and
 * what is asserted is that each still opens and still knows what it cited.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

const text = (value: string) => ({ type: 'text', text: value })
const p = (body: string) => ({ type: 'paragraph', content: [text(body)] })

function envelope(content: unknown[]) {
  return { schemaVersion: WRITING_SCHEMA_VERSION, doc: { type: 'doc', content } }
}

function mount(doc: unknown) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions(),
    content: doc as never,
  })
  return editor
}

describe('a citation written before a citation could hold several works', () => {
  /**
   * The first shape: one work, its fields on the node itself, no `items`. Every
   * citation in every manuscript written before the cluster change looks like
   * this, and reading only the array renders them as nothing — which is exactly
   * how they showed up as `[cita]` once.
   */
  const legacy = {
    type: 'zoteroCitation',
    attrs: {
      citationNodeId: 'z1',
      itemKey: 'ABCD1234',
      libraryType: 'user',
      libraryId: '1',
      metadataSnapshot: '{"id":"acha2015","title":"Un libro"}',
      locator: '112',
      locatorType: 'page',
      renderedText: '(Acha, 2015)',
    },
  }

  it('opens', () => {
    const parsed = parseCanonical(envelope([{ type: 'paragraph', content: [legacy] }]))

    expect(parsed.ok).toBe(true)
  })

  /** And still knows what it cited, which is what keeps it from rendering blank. */
  it('still reports the work it cites', () => {
    const works = worksOf(legacy)

    expect(works).toHaveLength(1)
    expect((works[0] as { itemKey?: string }).itemKey).toBe('ABCD1234')
  })

  it('mounts and draws the rendering it was saved with', () => {
    const instance = mount(envelope([{ type: 'paragraph', content: [legacy] }]).doc)

    // The rendered DOM, not `doc.textContent`: a citation is an atom, so its
    // text comes from `renderHTML` and never appears in the document's own text
    // — which is also why an atom with an empty `renderHTML` is invisible on
    // the page while sitting intact in the database.
    expect(instance.view.dom.textContent).toContain('(Acha, 2015)')
  })

  /**
   * And a work added to it today joins the one already there rather than
   * replacing it — the migration is the read, so nothing has to be rewritten
   * before the manuscript is usable again.
   */
  it('takes a second work without losing the first', () => {
    const instance = mount(envelope([{ type: 'paragraph', content: [legacy] }]).doc)
    instance.commands.setTextSelection(instance.state.doc.content.size - 1)

    citeWork(instance, { itemKey: 'EFGH5678', metadataSnapshot: '{"id":"nieto2021"}' })

    const node = instance.state.doc.nodeAt(1)
    expect(node?.type.name).toBe('zoteroCitation')
    expect(worksOf(node!)).toHaveLength(2)
  })
})

describe('a document saved before the footnote nodes existed', () => {
  it('opens, because nothing requires them', () => {
    const parsed = parseCanonical(
      envelope([
        { type: 'heading', attrs: { level: 1 }, content: [text('La huelga')] },
        p('un parrafo sin notas'),
      ])
    )

    expect(parsed.ok).toBe(true)
  })
})

describe('a note link written before its snapshot was cleaned of markup', () => {
  /**
   * Snapshots taken before the extraction went in still hold `<p>…</p>`. They
   * are stored text, so they read back exactly as saved: what changed is what
   * *new* links carry. The old ones must still open — a manuscript that refuses
   * to load because of how a note was quoted would be a far worse outcome than
   * a stray tag on screen.
   */
  it('opens with its markup intact rather than refusing', () => {
    const linked = {
      type: 'noteLink',
      attrs: {
        noteLinkNodeId: 'n1',
        noteId: 'note-1',
        itemId: 'it1',
        contentSnapshot: '<p>Esto es una carta enviada a la Union Obrera Local.</p>',
        contentHash: 'abc',
      },
    }

    const parsed = parseCanonical(envelope([{ type: 'paragraph', content: [linked] }]))

    expect(parsed.ok).toBe(true)
  })
})

describe('the version guard itself', () => {
  /**
   * §8.3: a document from a build that knows more than this one is reported,
   * never opened and never overwritten. The version was never bumped during
   * development, so this has protected nothing yet — which is why it is worth a
   * test rather than an assumption.
   */
  it('refuses a document from a newer build without destroying it', () => {
    const parsed = parseCanonical({
      schemaVersion: WRITING_SCHEMA_VERSION + 1,
      doc: { type: 'doc', content: [p('escrito por una version posterior')] },
    })

    expect(parsed.ok).toBe(false)
    expect(parsed.ok === false && parsed.code).toBe('unsupported-schema-version')
  })

  it('opens a document from an older build', () => {
    const parsed = parseCanonical({
      schemaVersion: 0,
      doc: { type: 'doc', content: [p('escrito por una version anterior')] },
    })

    expect(parsed.ok).toBe(true)
  })
})

describe('a document carrying every node the schema grew', () => {
  /**
   * The nodes added over the units — table, footnotes, the three citation
   * kinds. Adding a node cannot break an old document, but it can break a new
   * one if the schema and the extension list ever drift, and this is the
   * cheapest place to notice.
   */
  it('opens and mounts whole', () => {
    const doc = envelope([
      { type: 'heading', attrs: { level: 2 }, content: [text('El conflicto')] },
      {
        type: 'paragraph',
        content: [
          text('cita '),
          { type: 'documentCitation', attrs: { citationNodeId: 'd1', quotedText: 'lo dicho' } },
          { type: 'zoteroCitation', attrs: { citationNodeId: 'z1', items: [{ id: 'a' }] } },
          { type: 'noteLink', attrs: { noteLinkNodeId: 'n1', contentSnapshot: 'anotado' } },
          { type: 'footnoteReference', attrs: { 'data-id': 'f1', referenceNumber: '1' } },
        ],
      },
      {
        type: 'table',
        content: [
          {
            type: 'tableRow',
            content: [{ type: 'tableHeader', content: [p('Año')] }],
          },
        ],
      },
      {
        type: 'footnotes',
        content: [{ type: 'footnote', attrs: { 'data-id': 'f1' }, content: [p('la nota')] }],
      },
    ])

    const parsed = parseCanonical(doc)
    expect(parsed.ok).toBe(true)

    const instance = mount(parsed.ok ? parsed.document.doc : doc.doc)
    expect(instance.state.doc.textContent).toContain('El conflicto')
    expect(instance.state.doc.textContent).toContain('la nota')
  })
})
