import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AnnotationRepo } from './annotation.repo'
import type { DbClient, DrizzleClient } from '../types'

function createChainMock(resolveValue: unknown = []) {
  const chain: Record<string, ReturnType<typeof vi.fn>> = {}

  const createProxy = (): unknown =>
    new Proxy(() => {}, {
      apply: () => (resolveValue instanceof Promise ? resolveValue : Promise.resolve(resolveValue)),
      get: (_target, prop) => {
        if (prop === 'then') {
          return (resolve: (value: unknown) => void) => resolve(resolveValue)
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
  const deleteMock = createChainMock([])

  const db = {
    select: vi.fn().mockReturnValue(selectMock.proxy),
    insert: vi.fn().mockReturnValue(insertMock.proxy),
    delete: vi.fn().mockReturnValue(deleteMock.proxy),
  } as unknown as DrizzleClient

  return {
    db,
    mocks: {
      select: selectMock,
      insert: insertMock,
      delete: deleteMock,
    },
  }
}

describe('AnnotationRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: AnnotationRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new AnnotationRepo(db.db)
  })

  describe('create', () => {
    it('creates a rectangle annotation with page 1 geometry', async () => {
      const created = {
        id: 'ann-1',
        assetId: 'asset-1',
        page: 1,
        kind: 'rectangle',
        color: 'var(--color-accent)',
        x: 0.15,
        y: 0.2,
        width: 0.35,
        height: 0.25,
        createdAt: 100,
        updatedAt: 100,
      }

      const returningMock = vi.fn().mockResolvedValue([created])
      const valuesMock = vi.fn().mockReturnValue({ returning: returningMock })
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        assetId: 'asset-1',
        page: 1,
        kind: 'rectangle',
        color: 'var(--color-accent)',
        x: 0.15,
        y: 0.2,
        width: 0.35,
        height: 0.25,
      })

      expect(result).toEqual(created)
      expect(result.page).toBe(1)
      expect(result.kind).toBe('rectangle')
      expect(result.width).toBe(0.35)
    })

    it('creates an underline annotation with normalized geometry', async () => {
      const created = {
        id: 'ann-2',
        assetId: 'asset-1',
        page: 1,
        kind: 'underline',
        color: 'var(--color-warning)',
        x: 0.25,
        y: 0.6,
        width: 0.4,
        height: 0.05,
        createdAt: 200,
        updatedAt: 200,
      }

      const returningMock = vi.fn().mockResolvedValue([created])
      const valuesMock = vi.fn().mockReturnValue({ returning: returningMock })
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        assetId: 'asset-1',
        page: 1,
        kind: 'underline',
        color: 'var(--color-warning)',
        x: 0.25,
        y: 0.6,
        width: 0.4,
        height: 0.05,
      })

      expect(result).toEqual(created)
      expect(result.kind).toBe('underline')
      expect(result.height).toBe(0.05)
    })
  })

  describe('findByAsset', () => {
    it('returns only annotations for the requested asset', async () => {
      const records = [
        {
          id: 'ann-1',
          assetId: 'asset-1',
          page: 1,
          kind: 'rectangle',
          color: 'var(--color-accent)',
          x: 0.1,
          y: 0.2,
          width: 0.3,
          height: 0.2,
          createdAt: 10,
          updatedAt: 10,
        },
      ]

      const selectResult = createChainMock(records)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByAsset('asset-1', 1)

      expect(result).toEqual(records)
      expect(result).toHaveLength(1)
      expect(result[0]?.assetId).toBe('asset-1')
      expect(result[0]?.page).toBe(1)
    })

    it('returns annotations scoped to a different asset independently', async () => {
      const otherAssetRecords = [
        {
          id: 'ann-9',
          assetId: 'asset-2',
          page: 1,
          kind: 'underline',
          color: 'var(--color-success)',
          x: 0.4,
          y: 0.8,
          width: 0.2,
          height: 0.03,
          createdAt: 20,
          updatedAt: 20,
        },
      ]

      const selectResult = createChainMock(otherAssetRecords)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByAsset('asset-2', 1)

      expect(result).toEqual(otherAssetRecords)
      expect(result[0]?.assetId).toBe('asset-2')
    })
  })

  describe('replaceForAssetPage', () => {
    it('replaces all annotations for one asset page with the provided list', async () => {
      const deleteWhereMock = vi.fn().mockResolvedValue(undefined)
      db.mocks.delete.chain['where'] = deleteWhereMock

      const returningMock = vi.fn().mockResolvedValue([
        {
          id: 'ann-new-1',
          assetId: 'asset-1',
          page: 1,
          kind: 'rectangle',
          color: 'var(--color-accent)',
          x: 0.1,
          y: 0.1,
          width: 0.2,
          height: 0.2,
          createdAt: 100,
          updatedAt: 100,
        },
        {
          id: 'ann-new-2',
          assetId: 'asset-1',
          page: 1,
          kind: 'underline',
          color: 'var(--color-warning)',
          x: 0.25,
          y: 0.7,
          width: 0.3,
          height: 0.04,
          createdAt: 100,
          updatedAt: 100,
        },
      ])
      const valuesMock = vi.fn().mockReturnValue({ returning: returningMock })
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.replaceForAssetPage('asset-1', 1, [
        {
          kind: 'rectangle',
          color: 'var(--color-accent)',
          x: 0.1,
          y: 0.1,
          width: 0.2,
          height: 0.2,
        },
        {
          kind: 'underline',
          color: 'var(--color-warning)',
          x: 0.25,
          y: 0.7,
          width: 0.3,
          height: 0.04,
        },
      ])

      expect(deleteWhereMock).toHaveBeenCalledOnce()
      expect(valuesMock).toHaveBeenCalledOnce()
      expect(result).toHaveLength(2)
      expect(result[0]?.assetId).toBe('asset-1')
      expect(result[1]?.page).toBe(1)
    })

    it('deletes existing rows and returns empty list when replacements are empty', async () => {
      const deleteWhereMock = vi.fn().mockResolvedValue(undefined)
      db.mocks.delete.chain['where'] = deleteWhereMock

      const result = await repo.replaceForAssetPage('asset-1', 1, [])

      expect(deleteWhereMock).toHaveBeenCalledOnce()
      expect(result).toEqual([])
      expect(db.db.insert).not.toHaveBeenCalled()
    })

    it('uses one atomic batch and deterministic ids for singleton document edits', async () => {
      const executeTransaction = vi.fn().mockResolvedValue(undefined)
      const rawClient = {
        executeTransaction,
      } as unknown as DbClient
      const atomicRepo = new AnnotationRepo(db.db, rawClient)

      await atomicRepo.replaceForAssetPage('asset-1', 2, [
        { kind: 'crop', color: '#fff', x: 0.1, y: 0.2, width: 0.7, height: 0.6 },
      ])

      expect(executeTransaction).toHaveBeenCalledOnce()
      const statements = executeTransaction.mock.calls[0]?.[0] ?? []
      expect(statements[0]).toEqual({
        sql: 'DELETE FROM annotations WHERE asset_id = ? AND page = ?',
        params: ['asset-1', 2],
      })
      expect(statements[1]?.params).toContain('document-edit:asset-1:2:crop')
    })

    it('rejects malformed numeric geometry before constructing batch SQL', async () => {
      const executeTransaction = vi.fn().mockResolvedValue(undefined)
      const rawClient = {
        executeTransaction,
      } as unknown as DbClient
      const atomicRepo = new AnnotationRepo(db.db, rawClient)

      await expect(
        atomicRepo.replaceForAssetPage('asset-1', 1, [
          {
            kind: 'erase',
            color: '#fff',
            x: '0); DROP TABLE annotations; --' as unknown as number,
            y: 0,
            width: 1,
            height: 1,
          },
        ])
      ).rejects.toThrow('Invalid annotation x')
      expect(executeTransaction).not.toHaveBeenCalled()
    })
  })

  describe('delete', () => {
    it('deletes a single annotation by id', async () => {
      await expect(repo.delete('ann-1')).resolves.toBeUndefined()
    })
  })
})
