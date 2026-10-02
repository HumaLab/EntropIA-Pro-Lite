import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { bibliographySearchWorks } from './bibliography-search'

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
