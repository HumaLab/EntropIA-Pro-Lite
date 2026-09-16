import { describe, expect, it } from 'vitest'
import { writingSchema } from './document-contract'
import { citationProjection, citationsFromDocument, duplicatedCitationIds } from './citations'

/**
 * The citation projection (plan-editor.md §9.4, §10.1).
 *
 * The projection is *derived* from the manuscript on every save, never kept
 * alongside it. `save_document` deletes and reinserts the rows inside the same
 * transaction that writes the content, so a derived projection cannot drift:
 * copying, moving, deleting, undo and redo all stay consistent because there is
 * only one source of truth, and it is the document.
 *
 * That leaves exactly one thing the document cannot settle by itself, which is
 * what the second half of this file is about: pasting a citation duplicates its
 * identity.
 */

const text = (value: string) => ({ type: 'text', text: value })

function citation(attrs: Record<string, unknown>) {
  return { type: 'documentCitation', attrs }
}

function docOf(content: unknown[]) {
  return writingSchema().nodeFromJSON({ type: 'doc', content })
}

const FULL = {
  citationNodeId: 'c1',
  collectionId: 'col1',
  itemId: 'it1',
  assetId: 'as1',
  pageNumber: 12,
  startChar: 100,
  endChar: 140,
  quotedText: 'el molino de viento',
  sourceTextHash: 'abc123',
  metadataSnapshot: { title: 'Molinos' },
}

describe('citationsFromDocument', () => {
  it('projects every attribute the row needs', () => {
    const doc = docOf([{ type: 'paragraph', content: [text('antes '), citation(FULL)] }])

    const [row, ...rest] = citationsFromDocument(doc)

    expect(rest).toHaveLength(0)
    expect(row).toMatchObject({
      citation_node_id: 'c1',
      collection_id: 'col1',
      item_id: 'it1',
      asset_id: 'as1',
      page_number: 12,
      start_char: 100,
      end_char: 140,
      quoted_text: 'el molino de viento',
      source_text_hash: 'abc123',
    })
    expect(JSON.parse(row!.metadata_snapshot_json)).toEqual({ title: 'Molinos' })
  })

  /**
   * The rows are deleted and reinserted on every save, so the row id has no
   * referent outside the document. Deriving it from the node id makes saving
   * the same document twice produce the same projection.
   */
  it('is idempotent: the same document projects the same rows', () => {
    const doc = docOf([{ type: 'paragraph', content: [citation(FULL)] }])

    expect(citationsFromDocument(doc)).toEqual(citationsFromDocument(doc))
  })

  it('finds citations wherever they are, including inside a table cell', () => {
    const doc = docOf([
      { type: 'paragraph', content: [citation({ ...FULL, citationNodeId: 'c1' })] },
      {
        type: 'table',
        content: [
          {
            type: 'tableRow',
            content: [
              {
                type: 'tableCell',
                content: [
                  {
                    type: 'paragraph',
                    content: [citation({ ...FULL, citationNodeId: 'c2' })],
                  },
                ],
              },
            ],
          },
        ],
      },
    ])

    expect(citationsFromDocument(doc).map((row) => row.citation_node_id)).toEqual(['c1', 'c2'])
  })

  /** A citation with no page or range is still a citation, not a broken row. */
  it('keeps the optional columns null rather than inventing values', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [citation({ citationNodeId: 'c1', assetId: 'as1' })],
      },
    ])

    const [row] = citationsFromDocument(doc)
    expect(row).toMatchObject({
      citation_node_id: 'c1',
      asset_id: 'as1',
      page_number: null,
      start_char: null,
      end_char: null,
      quoted_text: null,
    })
    expect(row!.metadata_snapshot_json).toBe('{}')
  })

  /**
   * A node with no identity cannot be projected — the projection is keyed by
   * it. Dropping the row silently would lose the citation from the database
   * while leaving it on the page, so these are reported instead.
   */
  it('skips a citation that has no identity', () => {
    const doc = docOf([
      { type: 'paragraph', content: [citation({ assetId: 'as1' }), citation(FULL)] },
    ])

    expect(citationsFromDocument(doc).map((row) => row.citation_node_id)).toEqual(['c1'])
  })

  it('projects nothing for a document with no citations', () => {
    expect(citationsFromDocument(docOf([{ type: 'paragraph', content: [text('nada')] }]))).toEqual(
      []
    )
  })
})

/**
 * Pasting is the one operation the document cannot settle alone. A copied
 * citation arrives carrying the identity of the one it was copied from, and the
 * projection is unique per `(document, citation_node_id)` — so without this the
 * second copy would either collide or silently replace the first.
 */
describe('duplicatedCitationIds', () => {
  it('finds an id that appears more than once', () => {
    const doc = docOf([
      { type: 'paragraph', content: [citation(FULL)] },
      { type: 'paragraph', content: [citation(FULL)] },
    ])

    expect(duplicatedCitationIds(doc)).toEqual(['c1'])
  })

  /** The first occurrence keeps the identity; only the later ones are reported. */
  it('reports each duplicated id once, however many copies there are', () => {
    const doc = docOf([
      { type: 'paragraph', content: [citation(FULL), citation(FULL), citation(FULL)] },
    ])

    expect(duplicatedCitationIds(doc)).toEqual(['c1'])
  })

  it('finds nothing when every citation is its own', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [citation(FULL), citation({ ...FULL, citationNodeId: 'c2' })],
      },
    ])

    expect(duplicatedCitationIds(doc)).toEqual([])
  })
})

describe('citationProjection', () => {
  it('projects from the stored envelope the autosave loop holds', () => {
    const rows = citationProjection({
      schemaVersion: 1,
      doc: { type: 'doc', content: [{ type: 'paragraph', content: [citation(FULL)] }] },
    })

    expect(rows.map((row) => row.citation_node_id)).toEqual(['c1'])
  })

  /**
   * The save path is not where a malformed manuscript should first be found —
   * `parseCanonical` refuses it long before. Throwing here would turn a bad
   * document into a failed save instead of a reported one.
   */
  it('projects nothing rather than throwing on a document it cannot parse', () => {
    expect(
      citationProjection({
        schemaVersion: 1,
        doc: { type: 'doc', content: [{ type: 'quePasaAca' }] },
      })
    ).toEqual([])
    expect(citationProjection(null)).toEqual([])
  })
})

/**
 * A citation nobody can see is a citation nobody will notice is wrong.
 *
 * The node is an inline atom, so it has no content of its own: an empty
 * `renderHTML` produces an empty span and the fragment vanishes from the page
 * while sitting intact in the database.
 */
describe('a citation is visible in the manuscript', () => {
  it('draws the quoted fragment and its page', async () => {
    const { Editor } = await import('@tiptap/core')
    const { createWritingExtensions } = await import('./extensions')
    const element = document.createElement('div')
    document.body.appendChild(element)
    const editor = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [citation(FULL)] }],
      } as never,
    })

    expect(element.textContent).toContain('el molino de viento')
    expect(element.textContent).toContain('p. 12')
    editor.destroy()
  })

  /** §10.1 allows a reference without the transcription; it still shows. */
  it('draws a marker for a reference with no quoted text', async () => {
    const { Editor } = await import('@tiptap/core')
    const { createWritingExtensions } = await import('./extensions')
    const element = document.createElement('div')
    document.body.appendChild(element)
    const editor = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: {
        type: 'doc',
        content: [
          { type: 'paragraph', content: [citation({ citationNodeId: 'c9', assetId: 'as1' })] },
        ],
      } as never,
    })

    expect(element.textContent?.trim()).not.toBe('')
    editor.destroy()
  })

  /** An object attribute rendered into HTML lands as "[object Object]". */
  it('keeps the metadata snapshot out of the markup', async () => {
    const { Editor } = await import('@tiptap/core')
    const { createWritingExtensions } = await import('./extensions')
    const element = document.createElement('div')
    document.body.appendChild(element)
    const editor = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [citation(FULL)] }],
      } as never,
    })

    expect(element.innerHTML).not.toContain('[object Object]')
    // It survives in the JSON, which is the canonical form.
    expect(citationsFromDocument(editor.state.doc)[0]?.metadata_snapshot_json).toContain('Molinos')
    editor.destroy()
  })
})
