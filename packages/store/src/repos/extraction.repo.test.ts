import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ExtractionRepo } from './extraction.repo'
import type { DrizzleClient } from '../types'

// Helper: create a chainable mock that resolves with the given value
function createChainMock(resolveValue: unknown = []) {
  const chain: Record<string, ReturnType<typeof vi.fn>> = {}

  const createProxy = (): unknown =>
    new Proxy(() => {}, {
      apply: () => (resolveValue instanceof Promise ? resolveValue : Promise.resolve(resolveValue)),
      get: (_target, prop) => {
        if (prop === 'then') {
          return (resolve: (v: unknown) => void) => resolve(resolveValue)
        }
        if (!chain[prop as string]) {
          chain[prop as string] = vi.fn().mockReturnValue(createProxy())
        }
        return chain[prop as string]
      },
    })

  return { proxy: createProxy(), chain }
}

function createMockDrizzle() {
  const selectMock = createChainMock([])
  const insertMock = createChainMock([])
  const updateMock = createChainMock([])
  const deleteMock = createChainMock([])

  const db = {
    select: vi.fn().mockReturnValue(selectMock.proxy),
    insert: vi.fn().mockReturnValue(insertMock.proxy),
    update: vi.fn().mockReturnValue(updateMock.proxy),
    delete: vi.fn().mockReturnValue(deleteMock.proxy),
  } as unknown as DrizzleClient

  return {
    db,
    mocks: {
      select: selectMock,
      insert: insertMock,
      update: updateMock,
      delete: deleteMock,
    },
  }
}

describe('ExtractionRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: ExtractionRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new ExtractionRepo(db.db)
  })

  describe('upsert', () => {
    it('creates an extraction with correct fields', async () => {
      const now = Date.now()
      const mockExtraction = {
        id: 'ext-1',
        assetId: 'asset-1',
        textContent: 'Acta de nacimiento...',
        method: 'native',
        confidence: null,
        createdAt: now,
      }

      // Mock delete chain (delete existing for assetId)
      const deleteResult = createChainMock(undefined)
      ;(db.db.delete as ReturnType<typeof vi.fn>).mockReturnValue(deleteResult.proxy)

      // Mock insert chain
      const returningMock = vi.fn().mockResolvedValue([mockExtraction])
      const valuesMock = vi.fn().mockReturnValue({ returning: returningMock })
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.upsert({
        assetId: 'asset-1',
        textContent: 'Acta de nacimiento...',
        method: 'native',
      })

      expect(result).toEqual(mockExtraction)
      expect(result.assetId).toBe('asset-1')
      expect(result.textContent).toBe('Acta de nacimiento...')
      expect(result.method).toBe('native')
      expect(result.confidence).toBeNull()
    })

    it('creates an extraction with ocr method and confidence', async () => {
      const now = Date.now()
      const mockExtraction = {
        id: 'ext-2',
        assetId: 'asset-2',
        textContent: 'OCR result text',
        method: 'ocr',
        confidence: 0.87,
        createdAt: now,
      }

      const deleteResult = createChainMock(undefined)
      ;(db.db.delete as ReturnType<typeof vi.fn>).mockReturnValue(deleteResult.proxy)

      const returningMock = vi.fn().mockResolvedValue([mockExtraction])
      const valuesMock = vi.fn().mockReturnValue({ returning: returningMock })
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.upsert({
        assetId: 'asset-2',
        textContent: 'OCR result text',
        method: 'ocr',
        confidence: 0.87,
      })

      expect(result).toEqual(mockExtraction)
      expect(result.method).toBe('ocr')
      expect(result.confidence).toBe(0.87)
    })
  })

  describe('findByAsset', () => {
    it('returns null when no extraction exists', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByAsset('nonexistent-asset')
      expect(result).toBeNull()
    })

    it('returns the latest extraction for the asset', async () => {
      const extraction = {
        id: 'ext-latest',
        assetId: 'asset-1',
        textContent: 'Latest text',
        method: 'ocr',
        confidence: 0.95,
        createdAt: 300,
      }

      const selectResult = createChainMock([extraction])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByAsset('asset-1')
      expect(result).toEqual(extraction)
      expect(result!.id).toBe('ext-latest')
      expect(result!.textContent).toBe('Latest text')
    })
  })

  describe('findAllByAsset', () => {
    it('returns empty array when no extractions exist', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findAllByAsset('nonexistent-asset')
      expect(result).toEqual([])
      expect(result).toHaveLength(0)
    })

    it('returns all extractions for the asset ordered by createdAt DESC', async () => {
      const extractions = [
        {
          id: 'ext-2',
          assetId: 'asset-1',
          textContent: 'Second extraction',
          method: 'ocr',
          confidence: 0.95,
          createdAt: 200,
        },
        {
          id: 'ext-1',
          assetId: 'asset-1',
          textContent: 'First extraction',
          method: 'native',
          confidence: null,
          createdAt: 100,
        },
      ]

      const selectResult = createChainMock(extractions)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findAllByAsset('asset-1')
      expect(result).toEqual(extractions)
      expect(result).toHaveLength(2)
      expect(result[0]!.createdAt).toBe(200)
      expect(result[1]!.createdAt).toBe(100)
    })
  })

  describe('delete', () => {
    it('completes without error', async () => {
      await expect(repo.delete('ext-1')).resolves.toBeUndefined()
    })

    it('invalidates every extraction for an asset', async () => {
      await expect(repo.deleteByAsset('asset-1')).resolves.toBeUndefined()

      expect(db.db.delete).toHaveBeenCalledTimes(1)
      expect(db.mocks.delete.chain['where']).toHaveBeenCalledTimes(1)
    })
  })

  describe('findTextByCollection', () => {
    it('returns empty array when the collection has no extractions', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findTextByCollection('collection-1')
      expect(result).toEqual([])
      expect(db.db.select).toHaveBeenCalledTimes(1)
    })

    it('returns text rows for all assets in the collection', async () => {
      const rows = [
        { assetId: 'asset-1', textContent: 'Pescado del país', createdAt: 100 },
        { assetId: 'asset-2', textContent: 'Obreros en huelga', createdAt: 200 },
      ]

      const selectResult = createChainMock(rows)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findTextByCollection('collection-1')
      expect(result).toEqual(rows)
      expect(result).toHaveLength(2)
      expect(result[0]!.textContent).toBe('Pescado del país')
    })
  })
})
