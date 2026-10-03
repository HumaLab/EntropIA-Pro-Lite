import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  bibliographyLibraryStatus,
  bibliographyOpenPassage,
  bibliographyPassageContext,
  bibliographySearchPassages,
  bibliographySearchWorks,
} from './bibliography-search'

const mockInvoke = vi.mocked(invoke)

beforeEach(() => {
  mockInvoke.mockReset()
})

function hybridResponse() {
  return {
    hits: [
      {
        itemId: 'item-1',
        itemKey: 'AAAA1111',
        libraryId: 'lib-1',
        title: 'Obra A',
        method: 'hybrid',
        lexicalScore: -3.2,
        vectorScore: 0.87,
        fusedScore: 0.033,
        contractHash: 'contract-1',
        generationId: 'gen-1',
      },
    ],
    vectorAvailable: true,
    activeGenerationId: 'gen-1',
    contractHash: 'contract-1',
  }
}

describe('bibliographySearchWorks', () => {
  it('forwards the query text with defaults and returns the labeled answer', async () => {
    mockInvoke.mockResolvedValue(hybridResponse())
    const answer = await bibliographySearchWorks('revoluciones')

    expect(mockInvoke).toHaveBeenCalledWith(
      'bibliography_search_works',
      expect.objectContaining({
        request: expect.objectContaining({ text: 'revoluciones', topK: 20 }),
      })
    )
    expect(answer.vectorAvailable).toBe(true)
    expect(answer.hits).toHaveLength(1)
    expect(answer.hits[0]?.method).toBe('hybrid')
  })

  it('forwards topK and catalog filters without inventing defaults', async () => {
    mockInvoke.mockResolvedValue({ ...hybridResponse(), hits: [] })
    await bibliographySearchWorks('común', {
      topK: 5,
      filters: { yearFrom: 2020, itemTypes: ['book'], tags: ['historia'] },
    })

    expect(mockInvoke).toHaveBeenCalledWith(
      'bibliography_search_works',
      expect.objectContaining({
        request: expect.objectContaining({
          topK: 5,
          yearFrom: 2020,
          yearTo: null,
          itemTypes: ['book'],
          tags: ['historia'],
          libraryIds: null,
        }),
      })
    )
  })

  it('propagates backend errors without masking them', async () => {
    mockInvoke.mockRejectedValue(new Error('schema_not_ready'))
    await expect(bibliographySearchWorks('x')).rejects.toThrow('schema_not_ready')
  })
})

describe('chat scope commands', () => {
  it('reads the synced libraries and whether passage search can run', async () => {
    const status = {
      libraries: [
        { libraryType: 'user', libraryId: '0', name: 'Mi biblioteca', works: 12, passages: 340 },
      ],
      vectorReady: true,
    }
    mockInvoke.mockResolvedValue(status)

    await expect(bibliographyLibraryStatus()).resolves.toEqual(status)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_library_status')
  })

  it('reads a passage with its page context without opening anything', async () => {
    const context = {
      chunkId: 'chunk-1',
      itemId: 'item-1',
      itemKey: 'ABCD1234',
      title: 'Apología',
      text: 'texto',
      spans: [[3, 0, 5]],
      pages: [{ pageNumber: 3, text: 'texto de la página', highlights: [[0, 5]] }],
      openedPath: null,
      openError: null,
    }
    mockInvoke.mockResolvedValue(context)

    await expect(bibliographyPassageContext('chunk-1')).resolves.toEqual(context)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_passage_context', {
      chunkId: 'chunk-1',
    })
  })

  it('opens the original through the OS viewer only by its own command', async () => {
    mockInvoke.mockResolvedValue({ openedPath: 'C:/Zotero/storage/x.pdf', openError: null })

    await bibliographyOpenPassage('chunk-1')
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_open_passage', { chunkId: 'chunk-1' })
  })
})

describe('bibliographySearchPassages', () => {
  it('asks for passages with a default size and returns them with the notice', async () => {
    const answer = { passages: [], notice: 'no_embeddings' }
    mockInvoke.mockResolvedValue(answer)

    await expect(bibliographySearchPassages('cabildo')).resolves.toEqual(answer)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_search_passages', {
      request: { text: 'cabildo', topK: 12 },
    })
  })

  it('forwards an explicit size', async () => {
    mockInvoke.mockResolvedValue({ passages: [], notice: null })
    await bibliographySearchPassages('cabildo', { topK: 5 })
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_search_passages', {
      request: { text: 'cabildo', topK: 5 },
    })
  })
})
