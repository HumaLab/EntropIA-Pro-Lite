import { describe, expect, it } from 'vitest'
import { writingSchema } from './document-contract'
import {
  citationProjection,
  citationsFromDocument,
  duplicatedCitationIds,
  zoteroCitationsFromDocument,
} from './citations'

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
/**
 * A long quote is set off as a block, the way academic prose sets off long
 * quotations; a short one stays in the sentence it was written into.
 */
describe('a long citation is set off as a block', () => {
  async function renderQuote(quotedText: string) {
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
          {
            type: 'paragraph',
            content: [text('como dijo '), citation({ citationNodeId: 'c1', quotedText })],
          },
        ],
      } as never,
    })
    const node = element.querySelector('[data-document-citation]')
    const block = node?.hasAttribute('data-block-quote') ?? false
    editor.destroy()
    return block
  }

  it('sets off a quote that keeps line breaks from the page', async () => {
    expect(await renderQuote('Convenio Laboral\n\nLa aplicación del convenio')).toBe(true)
  })

  it('sets off a quote of forty words or more', async () => {
    expect(await renderQuote(Array.from({ length: 40 }, () => 'palabra').join(' '))).toBe(true)
  })

  it('leaves a short quote inside the sentence', async () => {
    expect(await renderQuote('no existía una legislación')).toBe(false)
    expect(await renderQuote(Array.from({ length: 39 }, () => 'palabra').join(' '))).toBe(false)
  })
})

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

/**
 * The bibliographic projection (§9.5), derived exactly like the corpus one and
 * for the same reason: `save_document` replaces these rows inside the
 * transaction that writes the content, so deriving them makes drift impossible.
 */
describe('zoteroCitationsFromDocument', () => {
  const WORK = {
    itemKey: 'ABCD1234',
    libraryType: 'user',
    libraryId: '0',
    itemVersion: 140,
    locator: '45',
    locatorType: 'page',
    suppressAuthor: true,
    metadataSnapshot: { id: 'ABCD1234', title: 'Il formaggio e i vermi' },
  }

  const SECOND = {
    itemKey: 'EFGH5678',
    metadataSnapshot: { id: 'EFGH5678', title: 'The Great Cat Massacre' },
  }

  function cluster(attrs: Record<string, unknown>) {
    return { type: 'zoteroCitation', attrs }
  }

  it('projects the CSL data a row needs', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [cluster({ citationNodeId: 'z1', items: [WORK], prefix: 'ver', suffix: 'y ss.' })],
      },
    ])

    const [row] = zoteroCitationsFromDocument(doc)

    expect(row).toMatchObject({
      citation_node_id: 'z1',
      citation_cluster_id: 'z1',
      item_position: 0,
      item_key: 'ABCD1234',
      item_version: 140,
      locator: '45',
      locator_type: 'page',
      prefix: 'ver',
      suffix: 'y ss.',
      suppress_author: true,
    })
    expect(JSON.parse(row!.item_csl_json_snapshot).title).toBe('Il formaggio e i vermi')
  })

  /**
   * The point of the whole cluster shape: `(Acha, 2015; Acha, 2008)` is one
   * citation of two works, and the table models it as two rows sharing a
   * cluster and differing by position — which is the unique key it declares.
   */
  it('projects one row per work, sharing the cluster', () => {
    const doc = docOf([
      { type: 'paragraph', content: [cluster({ citationNodeId: 'z1', items: [WORK, SECOND] })] },
    ])

    const rows = zoteroCitationsFromDocument(doc)

    expect(rows).toHaveLength(2)
    expect(rows.map((row) => row.citation_cluster_id)).toEqual(['z1', 'z1'])
    expect(rows.map((row) => row.item_position)).toEqual([0, 1])
    expect(rows.map((row) => row.item_key)).toEqual(['ABCD1234', 'EFGH5678'])
  })

  /**
   * The affixes belong to the citation, not to each of its works: repeating
   * them per row would print "see" once per source.
   */
  it('gives the affixes to the cluster rather than to every work', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [
          cluster({ citationNodeId: 'z1', items: [WORK, SECOND], prefix: 'ver', suffix: 'y ss.' }),
        ],
      },
    ])

    const rows = zoteroCitationsFromDocument(doc)

    expect(rows[0]).toMatchObject({ prefix: 'ver', suffix: 'y ss.' })
    expect(rows[1]).toMatchObject({ prefix: null, suffix: null })
  })

  /** A locator belongs to its work: two sources can be cited at two pages. */
  it('keeps each work its own locator', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [
          cluster({
            citationNodeId: 'z1',
            items: [WORK, { ...SECOND, locator: '12', locatorType: 'chapter' }],
          }),
        ],
      },
    ])

    const rows = zoteroCitationsFromDocument(doc)

    expect(rows.map((row) => row.locator)).toEqual(['45', '12'])
    expect(rows.map((row) => row.locator_type)).toEqual(['page', 'chapter'])
  })

  /**
   * §11.5 forbids storing the rendered string. It is what would leave stale
   * text in the database when someone changes citation style.
   */
  it('does not project the rendered text', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [
          cluster({ citationNodeId: 'z1', items: [WORK], renderedText: '(Ginzburg, 1976)' }),
        ],
      },
    ])

    const [row] = zoteroCitationsFromDocument(doc)

    expect(JSON.stringify(row)).not.toContain('(Ginzburg, 1976)')
  })

  /** `item_key` is NOT NULL; a work without one would be a citation of nothing. */
  it('skips a work with no item key, and a cluster with no identity', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [
          cluster({ citationNodeId: 'z1', items: [{ metadataSnapshot: {} }, WORK] }),
          cluster({ items: [WORK] }),
        ],
      },
    ])

    const rows = zoteroCitationsFromDocument(doc)

    expect(rows).toHaveLength(1)
    expect(rows[0]!.item_key).toBe('ABCD1234')
    // The surviving work is the cluster's only row, so it sits at position 0.
    expect(rows[0]!.item_position).toBe(1)
  })

  it('projects nothing for a manuscript that cites no bibliography', () => {
    expect(zoteroCitationsFromDocument(docOf([{ type: 'paragraph' }]))).toEqual([])
  })
})

/**
 * Citations written before a citation could hold more than one work.
 *
 * Changing the node's shape left every manuscript already on disk behind: their
 * citations carried `itemKey` on the node itself, so reading only `items` found
 * nothing and they rendered as `[cita]` — while the backend, handed an empty
 * cluster, brought down its worker thread.
 *
 * The old shape is understood as what it always meant: a cluster of one.
 */
describe('a citation written in the older shape', () => {
  const LEGACY = {
    type: 'zoteroCitation',
    attrs: {
      citationNodeId: 'z-old',
      itemKey: 'ABCD1234',
      libraryType: 'user',
      libraryId: '0',
      itemVersion: 140,
      locator: '45',
      locatorType: 'page',
      suppressAuthor: true,
      metadataSnapshot: { id: 'ABCD1234', title: 'Lucha y organización' },
    },
  }

  it('is still projected, as a cluster of one', () => {
    const doc = docOf([{ type: 'paragraph', content: [LEGACY] }])

    const rows = zoteroCitationsFromDocument(doc)

    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({
      citation_node_id: 'z-old',
      item_position: 0,
      item_key: 'ABCD1234',
      locator: '45',
      locator_type: 'page',
      suppress_author: true,
    })
  })

  /** Its snapshot is what lets it render with Zotero closed; it must survive. */
  it('keeps the snapshot it recorded', () => {
    const doc = docOf([{ type: 'paragraph', content: [LEGACY] }])

    const [row] = zoteroCitationsFromDocument(doc)

    expect(JSON.parse(row!.item_csl_json_snapshot).title).toBe('Lucha y organización')
  })

  /** A citation with neither shape has nothing to project, and must not crash. */
  it('projects nothing for a citation that names no work at all', () => {
    const doc = docOf([
      { type: 'paragraph', content: [{ type: 'zoteroCitation', attrs: { citationNodeId: 'z' } }] },
    ])

    expect(zoteroCitationsFromDocument(doc)).toEqual([])
  })

  /** Once it has an items array, that is what counts. */
  it('prefers the current shape when the citation has one', () => {
    const doc = docOf([
      {
        type: 'paragraph',
        content: [
          {
            ...LEGACY,
            attrs: {
              ...LEGACY.attrs,
              items: [{ itemKey: 'NUEVO999', metadataSnapshot: { id: 'NUEVO999' } }],
            },
          },
        ],
      },
    ])

    const rows = zoteroCitationsFromDocument(doc)

    expect(rows).toHaveLength(1)
    expect(rows[0]!.item_key).toBe('NUEVO999')
  })
})
