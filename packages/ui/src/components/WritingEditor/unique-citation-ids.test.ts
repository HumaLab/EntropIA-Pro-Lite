import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { citationsFromDocument, duplicatedCitationIds } from './citations'

/**
 * Citation identity through the operations of Unit 4.
 *
 * Copying, moving, deleting, undo and redo are all settled by deriving the
 * projection from the document — except for identity, which a paste duplicates.
 * These are the cases that proves it: that a copy gets a new identity while
 * still pointing at the same source, and that a move keeps its own.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

const CITATION = {
  type: 'documentCitation',
  attrs: {
    citationNodeId: 'c1',
    collectionId: 'col1',
    itemId: 'it1',
    assetId: 'as1',
    pageNumber: 12,
    startChar: 100,
    endChar: 140,
    quotedText: 'el molino de viento',
    sourceTextHash: 'abc123',
  },
}

function mount(content: unknown[]) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions(),
    content: { type: 'doc', content } as never,
  })
  return editor
}

function ids(instance: Editor): string[] {
  return citationsFromDocument(instance.state.doc).map((row) => row.citation_node_id)
}

describe('citation identity', () => {
  it('leaves a single citation alone', () => {
    const instance = mount([{ type: 'paragraph', content: [CITATION] }])

    expect(ids(instance)).toEqual(['c1'])
  })

  /**
   * A pasted copy arrives carrying the identity of the one it came from. The
   * first keeps it; the copy is reissued, and the projection stays unique.
   */
  it('reissues the identity of a pasted copy', () => {
    const instance = mount([{ type: 'paragraph', content: [CITATION] }])

    instance.chain().focus('end').insertContent(CITATION).run()

    const after = ids(instance)
    expect(after).toHaveLength(2)
    expect(new Set(after).size).toBe(2)
    expect(after).toContain('c1')
    expect(duplicatedCitationIds(instance.state.doc)).toEqual([])
  })

  /** A copy is still the same citation of the same source: only the id changes. */
  it('keeps the copy pointing at the source it was copied from', () => {
    const instance = mount([{ type: 'paragraph', content: [CITATION] }])

    instance.chain().focus('end').insertContent(CITATION).run()

    const rows = citationsFromDocument(instance.state.doc)
    for (const row of rows) {
      expect(row.asset_id).toBe('as1')
      expect(row.page_number).toBe(12)
      expect(row.start_char).toBe(100)
      expect(row.quoted_text).toBe('el molino de viento')
    }
  })

  /**
   * §10.1 asks a move to keep its identity. From the document's side a move and
   * a copy both end in a paste; what tells them apart is that a move leaves
   * nothing behind, so nothing collides and nothing is reissued.
   */
  it('keeps the identity when the citation is moved rather than copied', () => {
    const instance = mount([
      { type: 'paragraph', content: [CITATION] },
      { type: 'paragraph', content: [{ type: 'text', text: 'destino' }] },
    ])
    const before = ids(instance)

    // Cut: remove it from where it was, then put it back somewhere else.
    instance.chain().focus().setTextSelection({ from: 1, to: 2 }).deleteSelection().run()
    instance.chain().focus('end').insertContent(CITATION).run()

    expect(ids(instance)).toEqual(before)
  })

  it('gives an identity to a citation that arrived without one', () => {
    const instance = mount([
      { type: 'paragraph', content: [{ type: 'documentCitation', attrs: { assetId: 'as1' } }] },
    ])

    instance.chain().focus('end').insertContent(' ').run()

    const rows = citationsFromDocument(instance.state.doc)
    expect(rows).toHaveLength(1)
    expect(rows[0]!.citation_node_id).not.toBe('')
    expect(rows[0]!.asset_id).toBe('as1')
  })

  it('drops the row when the citation is deleted, without touching the others', () => {
    const instance = mount([
      { type: 'paragraph', content: [CITATION] },
      {
        type: 'paragraph',
        content: [{ ...CITATION, attrs: { ...CITATION.attrs, citationNodeId: 'c2' } }],
      },
    ])

    instance.chain().focus().setTextSelection({ from: 1, to: 2 }).deleteSelection().run()

    expect(ids(instance)).toEqual(['c2'])
  })

  /** Undo restores the document, and the projection follows because it is derived. */
  it('brings the citation back on undo', () => {
    const instance = mount([{ type: 'paragraph', content: [CITATION] }])
    instance.chain().focus().setTextSelection({ from: 1, to: 2 }).deleteSelection().run()
    expect(ids(instance)).toEqual([])

    instance.commands.undo()

    expect(ids(instance)).toEqual(['c1'])
  })
})
