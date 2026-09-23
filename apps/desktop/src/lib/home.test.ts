import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { HomeCollectionSource, HomeResearchJobSource, HomeWritingDocumentSource } from './home'

const { getStoreMock, storeRef, writingRef, researchListMock } = vi.hoisted(() => ({
  getStoreMock: vi.fn(),
  storeRef: {
    current: {
      items: { getCorpusStats: vi.fn() },
      collections: { findAll: vi.fn(), countItems: vi.fn() },
    },
  },
  writingRef: {
    listDocuments: vi.fn(),
    snapshot: { documents: [] as Array<{ id: string; title: string; updated_at: number }> },
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

import { mergeRecentActivity, loadHomeSnapshot } from './home'

describe('mergeRecentActivity', () => {
  const collections: HomeCollectionSource[] = [
    { id: 'c1', name: 'Archivo', updatedAt: 1_000, itemCount: 12 },
    { id: 'c2', name: 'Fotos', updatedAt: 5_000, itemCount: 3 },
  ]
  const writingDocs: HomeWritingDocumentSource[] = [
    { id: 'w1', title: 'Borrador', updated_at: 3_000 },
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
      updatedAt: new Date(1_000),
      view: { name: 'collection', id: 'c1', collectionName: 'Archivo' },
    })

    const writingEntry = result.find((entry) => entry.id === 'w1')
    expect(writingEntry).toEqual({
      kind: 'writing',
      id: 'w1',
      title: 'Borrador',
      size: null,
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
    storeRef.current.collections.findAll.mockReset().mockResolvedValue([])
    storeRef.current.collections.countItems.mockReset().mockResolvedValue(0)
    writingRef.listDocuments.mockReset().mockResolvedValue(undefined)
    writingRef.snapshot.documents = []
    researchListMock.mockReset().mockResolvedValue({ jobs: [], collections: [], modalidades: [] })
  })

  it('combines corpus stats, collections, writing documents and research jobs', async () => {
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
    expect(snapshot.recent.map((entry) => entry.id)).toEqual(['w1', 'c1', 'j1'])
    expect(snapshot.isFirstRun).toBe(false)
  })

  it('reports isFirstRun when there are no collections, writing documents or research jobs', async () => {
    const snapshot = await loadHomeSnapshot()

    expect(snapshot.recent).toEqual([])
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

    expect(snapshot.recent.map((entry) => entry.id)).toEqual(['c1'])
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

    expect(snapshot.recent.map((entry) => entry.id)).toEqual(['j1'])
  })
})
