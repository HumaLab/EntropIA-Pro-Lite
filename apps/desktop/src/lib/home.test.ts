import { describe, it, expect, vi, beforeEach } from 'vitest'
import { locale, t } from '$lib/i18n'
import type {
  HomeCollectionSource,
  HomeResearchJobSource,
  HomeWritingDocumentSource,
  HomeRecentlyImportedSource,
  HomeRecentEntry,
} from './home'

const { getStoreMock, storeRef, writingRef, researchListMock } = vi.hoisted(() => ({
  getStoreMock: vi.fn(),
  storeRef: {
    current: {
      items: { getCorpusStats: vi.fn(), findRecentlyImported: vi.fn() },
      collections: { findAll: vi.fn(), countItems: vi.fn() },
    },
  },
  writingRef: {
    listDocuments: vi.fn(),
    snapshot: {
      documents: [] as Array<{
        id: string
        title: string
        updated_at: number
        current_content_json?: string
      }>,
    },
  },
  researchListMock: vi.fn(),
}))

vi.mock('$lib/db', () => ({
  getStore: getStoreMock,
}))

vi.mock('$lib/writing', () => ({
  writing: writingRef,
}))

vi.mock('$lib/research', () => ({
  researchList: researchListMock,
}))

import {
  mergeRecentActivity,
  mapRecentlyImported,
  loadHomeSnapshot,
  countManuscriptWords,
  isUntitledWritingTitle,
  attachContinuarWordCounts,
} from './home'

/** A minimal canonical manuscript envelope whose text is exactly `text`. */
function manuscriptJson(text: string): string {
  return JSON.stringify({
    schemaVersion: 1,
    doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  })
}

describe('mergeRecentActivity', () => {
  const collections: HomeCollectionSource[] = [
    { id: 'c1', name: 'Archivo', updatedAt: 1_000, itemCount: 12 },
    { id: 'c2', name: 'Fotos', updatedAt: 5_000, itemCount: 3 },
  ]
  const writingDocs: HomeWritingDocumentSource[] = [
    { id: 'w1', title: 'Borrador', updated_at: 3_000, current_content_json: manuscriptJson('') },
  ]
  const researchJobs: HomeResearchJobSource[] = [{ id: 'j1', title: 'Pregunta' }]

  it('merges all three sources sorted by modified time, most recent first', () => {
    const result = mergeRecentActivity({ collections, writing: writingDocs, research: [] })

    expect(result.map((entry) => entry.id)).toEqual(['c2', 'w1', 'c1'])
  })

  it('shapes each entry with its kind, size and navigation target', () => {
    const result = mergeRecentActivity({ collections, writing: writingDocs, research: [] })

    const collectionEntry = result.find((entry) => entry.id === 'c1')
    expect(collectionEntry).toEqual({
      kind: 'collection',
      id: 'c1',
      title: 'Archivo',
      size: 12,
      wordCount: null,
      updatedAt: new Date(1_000),
      view: { name: 'collection', id: 'c1', collectionName: 'Archivo' },
    })

    const writingEntry = result.find((entry) => entry.id === 'w1')
    expect(writingEntry).toEqual({
      kind: 'writing',
      id: 'w1',
      title: 'Borrador',
      size: null,
      wordCount: null,
      updatedAt: new Date(3_000),
      view: { name: 'writing', documentId: 'w1', documentTitle: 'Borrador' },
    })
  })

  it('places research jobs, which carry no modified time, after every timestamped entry', () => {
    const result = mergeRecentActivity({
      collections,
      writing: writingDocs,
      research: researchJobs,
    })

    expect(result.map((entry) => entry.id)).toEqual(['c2', 'w1', 'c1', 'j1'])
    const researchEntry = result.find((entry) => entry.id === 'j1')
    expect(researchEntry).toEqual({
      kind: 'research',
      id: 'j1',
      title: 'Pregunta',
      size: null,
      wordCount: null,
      updatedAt: null,
      view: { name: 'investigation', jobId: 'j1', title: 'Pregunta' },
    })
  })

  it('keeps several timestamp-less entries in their original relative order', () => {
    const result = mergeRecentActivity({
      collections: [],
      writing: [],
      research: [
        { id: 'j1', title: 'Primera' },
        { id: 'j2', title: 'Segunda' },
      ],
    })

    expect(result.map((entry) => entry.id)).toEqual(['j1', 'j2'])
  })

  it('returns an empty list when every source is empty', () => {
    expect(mergeRecentActivity({ collections: [], writing: [], research: [] })).toEqual([])
  })
})

describe('mapRecentlyImported', () => {
  it('shapes each recently-imported item with a title, collection name and navigation target', () => {
    const sources: HomeRecentlyImportedSource[] = [
      { id: 'i1', title: 'Acta', collectionId: 'c1', collectionName: 'Archivo', createdAt: 5_000 },
    ]

    expect(mapRecentlyImported(sources)).toEqual([
      {
        id: 'i1',
        title: 'Acta',
        collectionName: 'Archivo',
        createdAt: new Date(5_000),
        view: {
          name: 'item',
          collectionId: 'c1',
          collectionName: 'Archivo',
          itemId: 'i1',
          itemTitle: 'Acta',
        },
      },
    ])
  })

  it('preserves the repository order (already newest first)', () => {
    const sources: HomeRecentlyImportedSource[] = [
      { id: 'i2', title: 'B', collectionId: 'c1', collectionName: 'A', createdAt: 2_000 },
      { id: 'i1', title: 'A', collectionId: 'c1', collectionName: 'A', createdAt: 1_000 },
    ]

    expect(mapRecentlyImported(sources).map((entry) => entry.id)).toEqual(['i2', 'i1'])
  })

  it('returns an empty list for an empty source', () => {
    expect(mapRecentlyImported([])).toEqual([])
  })
})

describe('countManuscriptWords', () => {
  it('counts words across paragraph text nodes', () => {
    const json = JSON.stringify({
      schemaVersion: 1,
      doc: {
        type: 'doc',
        content: [
          { type: 'paragraph', content: [{ type: 'text', text: 'Hola mundo' }] },
          { type: 'paragraph', content: [{ type: 'text', text: 'cómo estás' }] },
        ],
      },
    })

    expect(countManuscriptWords(json)).toBe(4)
  })

  it('counts nested content, such as a heading and a bullet list, recursively', () => {
    const json = JSON.stringify({
      schemaVersion: 1,
      doc: {
        type: 'doc',
        content: [
          { type: 'heading', content: [{ type: 'text', text: 'Título largo' }] },
          {
            type: 'bulletList',
            content: [
              {
                type: 'listItem',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'primer punto' }] }],
              },
            ],
          },
        ],
      },
    })

    expect(countManuscriptWords(json)).toBe(4)
  })

  it('splits on unicode whitespace, not only ascii spaces', () => {
    const json = manuscriptJson('uno dos\ttres\n\ncuatro')

    expect(countManuscriptWords(json)).toBe(4)
  })

  it('ignores an empty text node', () => {
    expect(countManuscriptWords(manuscriptJson(''))).toBe(0)
  })

  it('returns null for invalid JSON', () => {
    expect(countManuscriptWords('{not valid json')).toBeNull()
  })

  it('returns null for a value that is not a canonical document envelope', () => {
    expect(countManuscriptWords('"just a string"')).toBeNull()
    expect(countManuscriptWords('42')).toBeNull()
    expect(countManuscriptWords('null')).toBeNull()
  })

  it('returns null for undefined input', () => {
    expect(countManuscriptWords(undefined)).toBeNull()
  })
})

describe('isUntitledWritingTitle', () => {
  it('treats an empty or whitespace-only title as untitled', () => {
    expect(isUntitledWritingTitle('')).toBe(true)
    expect(isUntitledWritingTitle('   ')).toBe(true)
  })

  it('treats the default stored title as untitled, trimmed, in either locale', () => {
    expect(isUntitledWritingTitle('Sin título')).toBe(true)
    expect(isUntitledWritingTitle('  Sin título  ')).toBe(true)
    expect(isUntitledWritingTitle('Untitled')).toBe(true)
    expect(isUntitledWritingTitle('  Untitled  ')).toBe(true)
  })

  it('treats a real title as not untitled', () => {
    expect(isUntitledWritingTitle('Borrador de tesis')).toBe(false)
  })

  it('matches the actual stored default title in i18n.ts, for both locales', () => {
    const originalLocale = 'es' as const
    locale.set('es')
    expect(isUntitledWritingTitle(t('writing.newDocumentTitle'))).toBe(true)
    locale.set('en')
    expect(isUntitledWritingTitle(t('writing.newDocumentTitle'))).toBe(true)
    locale.set(originalLocale)
  })
})

describe('attachContinuarWordCounts', () => {
  function writingEntry(id: string, title = id): HomeRecentEntry {
    return {
      kind: 'writing',
      id,
      title,
      size: null,
      wordCount: null,
      updatedAt: new Date(1),
      view: { name: 'writing', documentId: id, documentTitle: title },
    }
  }

  it('computes a word count for a writing entry inside the first 3 slots', () => {
    const entries = [writingEntry('w1')]
    const sources: HomeWritingDocumentSource[] = [
      {
        id: 'w1',
        title: 'w1',
        updated_at: 1,
        current_content_json: manuscriptJson('uno dos tres'),
      },
    ]

    const result = attachContinuarWordCounts(entries, sources)

    expect(result[0]!.wordCount).toBe(3)
  })

  it('leaves collection and research entries untouched', () => {
    const collectionEntry: HomeRecentEntry = {
      kind: 'collection',
      id: 'c1',
      title: 'Archivo',
      size: 12,
      wordCount: null,
      updatedAt: new Date(1),
      view: { name: 'collection', id: 'c1', collectionName: 'Archivo' },
    }
    const researchEntry: HomeRecentEntry = {
      kind: 'research',
      id: 'j1',
      title: 'Pregunta',
      size: null,
      wordCount: null,
      updatedAt: null,
      view: { name: 'investigation', jobId: 'j1', title: 'Pregunta' },
    }

    const result = attachContinuarWordCounts([collectionEntry, researchEntry], [])

    expect(result).toEqual([collectionEntry, researchEntry])
  })

  it('does not compute a word count for a writing entry beyond the top 3', () => {
    const entries = [writingEntry('w1'), writingEntry('w2'), writingEntry('w3'), writingEntry('w4')]
    const sources: HomeWritingDocumentSource[] = entries.map((entry) => ({
      id: entry.id,
      title: entry.title,
      updated_at: 1,
      current_content_json: manuscriptJson('una palabra mas'),
    }))

    const result = attachContinuarWordCounts(entries, sources)

    expect(result[0]!.wordCount).toBe(3)
    expect(result[3]!.wordCount).toBeNull()
  })

  it('leaves the word count null when the matching content is missing or invalid', () => {
    const entry = writingEntry('w1')

    expect(attachContinuarWordCounts([entry], [])[0]!.wordCount).toBeNull()

    const invalidSource: HomeWritingDocumentSource[] = [
      { id: 'w1', title: 'w1', updated_at: 1, current_content_json: '{not valid json' },
    ]
    expect(attachContinuarWordCounts([entry], invalidSource)[0]!.wordCount).toBeNull()
  })
})

describe('loadHomeSnapshot', () => {
  beforeEach(() => {
    getStoreMock.mockReset().mockReturnValue(storeRef.current)
    storeRef.current.items.getCorpusStats.mockReset().mockResolvedValue({
      collections: 1,
      items: 5,
      ocr: 3,
      embeddings: 2,
      pendingOcr: 1,
      pendingEmbeddings: 0,
    })
    storeRef.current.items.findRecentlyImported.mockReset().mockResolvedValue([])
    storeRef.current.collections.findAll.mockReset().mockResolvedValue([])
    storeRef.current.collections.countItems.mockReset().mockResolvedValue(0)
    writingRef.listDocuments.mockReset().mockResolvedValue(undefined)
    writingRef.snapshot.documents = []
    researchListMock.mockReset().mockResolvedValue({ jobs: [], collections: [], modalidades: [] })
  })

  it('combines corpus stats, collections, writing documents and research jobs into Continuar', async () => {
    storeRef.current.collections.findAll.mockResolvedValue([
      { id: 'c1', name: 'Archivo', description: null, createdAt: 0, updatedAt: 2_000 },
    ])
    storeRef.current.collections.countItems.mockResolvedValue(7)
    writingRef.snapshot.documents = [
      { id: 'w1', title: 'Borrador', updated_at: 4_000 } as {
        id: string
        title: string
        updated_at: number
      },
    ]
    researchListMock.mockResolvedValue({
      jobs: [{ id: 'j1', title: 'Pregunta' }],
      collections: [],
      modalidades: [],
    })

    const snapshot = await loadHomeSnapshot()

    expect(snapshot.stats).toEqual({
      collections: 1,
      items: 5,
      ocr: 3,
      embeddings: 2,
      pendingOcr: 1,
      pendingEmbeddings: 0,
    })
    expect(snapshot.continuar.map((entry) => entry.id)).toEqual(['w1', 'c1', 'j1'])
    expect(snapshot.isFirstRun).toBe(false)
  })

  it('loads recently imported items into Actividad reciente, distinct from Continuar', async () => {
    storeRef.current.items.findRecentlyImported.mockResolvedValue([
      { id: 'i1', title: 'Acta', collectionId: 'c1', collectionName: 'Archivo', createdAt: 9_000 },
    ])

    const snapshot = await loadHomeSnapshot()

    expect(storeRef.current.items.findRecentlyImported).toHaveBeenCalledWith(5)
    expect(snapshot.activity).toEqual([
      {
        id: 'i1',
        title: 'Acta',
        collectionName: 'Archivo',
        createdAt: new Date(9_000),
        view: {
          name: 'item',
          collectionId: 'c1',
          collectionName: 'Archivo',
          itemId: 'i1',
          itemTitle: 'Acta',
        },
      },
    ])
  })

  it('reports isFirstRun when there are no collections, writing documents or research jobs', async () => {
    const snapshot = await loadHomeSnapshot()

    expect(snapshot.continuar).toEqual([])
    expect(snapshot.activity).toEqual([])
    expect(snapshot.isFirstRun).toBe(true)
  })

  it('is not first-run when only one source has content', async () => {
    storeRef.current.collections.findAll.mockResolvedValue([
      { id: 'c1', name: 'Archivo', description: null, createdAt: 0, updatedAt: 1_000 },
    ])

    const snapshot = await loadHomeSnapshot()

    expect(snapshot.isFirstRun).toBe(false)
  })

  it('tolerates the research source failing without failing the whole snapshot', async () => {
    researchListMock.mockRejectedValue(new Error('research backend unavailable'))
    storeRef.current.collections.findAll.mockResolvedValue([
      { id: 'c1', name: 'Archivo', description: null, createdAt: 0, updatedAt: 1_000 },
    ])

    const snapshot = await loadHomeSnapshot()

    expect(snapshot.continuar.map((entry) => entry.id)).toEqual(['c1'])
    expect(snapshot.isFirstRun).toBe(false)
  })

  it('tolerates the writing source failing without failing the whole snapshot', async () => {
    writingRef.listDocuments.mockRejectedValue(new Error('writing backend unavailable'))
    researchListMock.mockResolvedValue({
      jobs: [{ id: 'j1', title: 'Pregunta' }],
      collections: [],
      modalidades: [],
    })

    const snapshot = await loadHomeSnapshot()

    expect(snapshot.continuar.map((entry) => entry.id)).toEqual(['j1'])
  })

  it('attaches a word count to a Continuar writing entry from its already-loaded content', async () => {
    writingRef.snapshot.documents = [
      {
        id: 'w1',
        title: 'Borrador',
        updated_at: 4_000,
        current_content_json: manuscriptJson('uno dos tres cuatro'),
      },
    ]

    const snapshot = await loadHomeSnapshot()

    const writingEntry = snapshot.continuar.find((entry) => entry.id === 'w1')
    expect(writingEntry?.wordCount).toBe(4)
  })

  it('leaves the word count null when a writing document carries no content', async () => {
    writingRef.snapshot.documents = [{ id: 'w1', title: 'Borrador', updated_at: 4_000 }]

    const snapshot = await loadHomeSnapshot()

    const writingEntry = snapshot.continuar.find((entry) => entry.id === 'w1')
    expect(writingEntry?.wordCount).toBeNull()
  })

  it('tolerates the recently-imported items query failing without failing the whole snapshot', async () => {
    storeRef.current.items.findRecentlyImported.mockRejectedValue(new Error('db unavailable'))
    storeRef.current.collections.findAll.mockResolvedValue([
      { id: 'c1', name: 'Archivo', description: null, createdAt: 0, updatedAt: 1_000 },
    ])

    const snapshot = await loadHomeSnapshot()

    expect(snapshot.activity).toEqual([])
    expect(snapshot.isFirstRun).toBe(false)
  })
})
