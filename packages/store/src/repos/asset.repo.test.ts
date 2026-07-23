import { describe, it, expect, vi, beforeEach } from 'vitest'
import { AssetRepo } from './asset.repo'
import type { DrizzleClient } from '../types'
import type { DbClient } from '../types'

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

describe('AssetRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: AssetRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new AssetRepo(db.db)
  })

  describe('create', () => {
    it('returns a locally-constructed asset and inserts it without returning()', async () => {
      const valuesMock = vi.fn().mockResolvedValue(undefined)
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        itemId: 'item-1',
        path: '/data/files/paper.pdf',
        type: 'pdf',
        size: 1024,
      })

      expect(valuesMock).toHaveBeenCalledOnce()
      expect(valuesMock.mock.calls[0]?.[0]).toEqual(result)
      expect(typeof result.id).toBe('string')
      expect(result.itemId).toBe('item-1')
      expect(result.path).toBe('/data/files/paper.pdf')
      expect(result.type).toBe('pdf')
      expect(result.size).toBe(1024)
      expect(typeof result.createdAt).toBe('number')
    })

    it('creates asset without size (optional field)', async () => {
      const valuesMock = vi.fn().mockResolvedValue(undefined)
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        itemId: 'item-1',
        path: '/data/files/photo.jpg',
        type: 'image',
      })

      expect(valuesMock).toHaveBeenCalledOnce()
      expect(valuesMock.mock.calls[0]?.[0]).toEqual(result)
      expect(result.size).toBeNull()
    })

    it('uses raw client INSERT when provided', async () => {
      const rawExecuteMock = vi.fn().mockResolvedValue({ rowsAffected: 1 })
      const rawClient = {
        execute: rawExecuteMock,
        select: vi.fn().mockResolvedValue([{ id: 'item-raw-1' }]),
      } as unknown as DbClient
      const repo2 = new AssetRepo(db.db, rawClient)

      const result = await repo2.create({
        itemId: 'item-raw-1',
        path: '/raw/path/file.pdf',
        type: 'pdf',
        size: 42,
      })

      expect(rawExecuteMock).toHaveBeenCalledOnce()
      expect(rawExecuteMock).toHaveBeenCalledWith(
        'INSERT INTO assets (id, item_id, path, type, sort_index, size, parent_asset_id, page_number, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)',
        [
          result.id,
          result.itemId,
          result.path,
          result.type,
          result.sortIndex,
          result.size,
          result.parentAssetId,
          result.pageNumber,
          result.createdAt,
        ]
      )
      expect(db.db.insert).not.toHaveBeenCalled()
    })

    it('throws when parent item does not exist (raw client)', async () => {
      const rawExecuteMock = vi.fn().mockResolvedValue({ rowsAffected: 1 })
      const rawClient = {
        execute: rawExecuteMock,
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repo2 = new AssetRepo(db.db, rawClient)

      await expect(
        repo2.create({
          itemId: 'non-existent-item',
          path: '/raw/path/file.pdf',
          type: 'pdf',
        })
      ).rejects.toThrow('item "non-existent-item" does not exist')

      expect(rawExecuteMock).not.toHaveBeenCalled()
    })
  })

  describe('findByItem', () => {
    it('returns empty array when item has no assets', async () => {
      const result = await repo.findByItem('no-assets-item')
      expect(result).toEqual([])
    })

    it('returns assets for a specific item', async () => {
      const assets = [
        { id: 'a1', itemId: 'item-1', path: '/a.pdf', type: 'pdf', sortIndex: 0, size: 100, createdAt: 10 },
        { id: 'a2', itemId: 'item-1', path: '/b.jpg', type: 'image', sortIndex: 0, size: 200, createdAt: 20 },
      ]

      const selectResult = createChainMock(assets)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByItem('item-1')
      expect(result).toEqual(assets)
      expect(result).toHaveLength(2)
      expect(selectResult.chain['orderBy']).toHaveBeenCalledOnce()
      expect(result[0]!.type).toBe('pdf')
      expect(result[1]!.type).toBe('image')
    })

    it('keeps A-Z path ordering for multi-asset items without page sort indexes', async () => {
      const unorderedAssets = [
        { id: 'b', itemId: 'item-1', path: '/Zeta.jpg', type: 'image', sortIndex: 0, size: 100, createdAt: 10 },
        { id: 'a', itemId: 'item-1', path: '/alpha.jpg', type: 'image', sortIndex: 0, size: 100, createdAt: 20 },
      ]

      const selectResult = createChainMock(unorderedAssets)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByItem('item-1')

      expect(result.map((asset) => asset.path)).toEqual(['/alpha.jpg', '/Zeta.jpg'])
    })

    it('preserves original PDF page order for multi-page assets with sort indexes', async () => {
      const lexicographicTrapAssets = [
        { id: 'page-10', itemId: 'item-1', path: '/scan_page_10.png', type: 'image', sortIndex: 9, size: 100, createdAt: 10 },
        { id: 'page-2', itemId: 'item-1', path: '/scan_page_2.png', type: 'image', sortIndex: 1, size: 100, createdAt: 20 },
        { id: 'page-1', itemId: 'item-1', path: '/scan_page_1.png', type: 'image', sortIndex: 0, size: 100, createdAt: 30 },
      ]

      const selectResult = createChainMock(lexicographicTrapAssets)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByItem('item-1')

      expect(result.map((asset) => asset.id)).toEqual(['page-1', 'page-2', 'page-10'])
    })
  })

  describe('findById', () => {
    it('returns null when asset not found', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findById('non-existent')
      expect(result).toBeNull()
    })

    it('returns the asset when found', async () => {
      const asset = {
        id: 'found-1',
        itemId: 'item-1',
        path: '/doc.pdf',
        type: 'pdf',
        size: 512,
        createdAt: 1,
      }
      const selectResult = createChainMock([asset])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findById('found-1')
      expect(result).toEqual(asset)
      expect(result!.id).toBe('found-1')
      expect(result!.size).toBe(512)
    })
  })

  describe('findByParentAssetId', () => {
    it('returns page children in deterministic page order', async () => {
      const rawClient = {
        execute: vi.fn(),
        executeBatch: vi.fn(),
        select: vi.fn().mockResolvedValue([
          { id: 'page-1', item_id: 'item-1', path: '/pages/0001.png', type: 'image', sort_index: 1, size: 10, parent_asset_id: 'pdf-1', page_number: 1, created_at: 1 },
          { id: 'page-2', item_id: 'item-1', path: '/pages/0002.png', type: 'image', sort_index: 2, size: 10, parent_asset_id: 'pdf-1', page_number: 2, created_at: 2 },
        ]),
      } as unknown as DbClient
      const repoWithRaw = new AssetRepo(db.db, rawClient)

      const children = await repoWithRaw.findByParentAssetId('pdf-1')

      expect(children.map((asset) => asset.pageNumber)).toEqual([1, 2])
      expect(children.every((asset) => asset.parentAssetId === 'pdf-1')).toBe(true)
    })
  })

  describe('delete', () => {
    it('completes without error', async () => {
      await expect(repo.delete('del-1')).resolves.toBeUndefined()
    })
  })

  describe('deleteWithCascade', () => {
    it('throws when rawClient is not provided', async () => {
      const repoNoRaw = new AssetRepo(db.db)
      await expect(repoNoRaw.deleteWithCascade('asset-1')).rejects.toThrow(
        'deleteWithCascade requires a rawClient'
      )
    })

    it('throws when asset is not found', async () => {
      // Mock Drizzle select for findById to return empty
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: vi.fn().mockResolvedValue(undefined),
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repoWithRaw = new AssetRepo(db.db, rawClient)

      await expect(repoWithRaw.deleteWithCascade('non-existent')).rejects.toThrow(
        'Asset not found: non-existent'
      )
    })

    it('returns the deleted asset and executes batch delete', async () => {
      const asset = {
        id: 'asset-1',
        itemId: 'item-1',
        path: '/app-data/assets/coll-1/item-1/uuid_file.pdf',
        type: 'pdf',
        size: 1024,
        createdAt: 100,
      }

      // Mock Drizzle select for findById (used by deleteWithCascade)
      const selectResult = createChainMock([asset])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: vi.fn().mockResolvedValue(undefined),
        select: vi.fn().mockResolvedValue([asset]),
      } as unknown as DbClient & { executeBatch: ReturnType<typeof vi.fn> }
      const repoWithRaw = new AssetRepo(db.db, rawClient)

      const result = await repoWithRaw.deleteWithCascade('asset-1')

      expect(result).toEqual(asset)
      expect(rawClient.executeBatch).toHaveBeenCalledOnce()
      const batchSql = rawClient.executeBatch.mock.calls[0]?.[0] as string
      expect(batchSql).toContain('BEGIN;')
      expect(batchSql).toContain('DELETE FROM extractions')
      expect(batchSql).toContain('DELETE FROM layouts')
      expect(batchSql).toContain('DELETE FROM transcriptions')
      expect(batchSql).toContain('DELETE FROM llm_results')
      expect(batchSql).toContain('DELETE FROM annotations')
      expect(batchSql).toContain('DELETE FROM entities')
      expect(batchSql).toContain('DELETE FROM triples')
      expect(batchSql).toContain('DELETE FROM vec_assets')
      expect(batchSql).toContain(
        "DELETE FROM entities WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = 'asset-1')"
      )
      expect(batchSql).toContain(
        "DELETE FROM llm_results WHERE target_id IN (SELECT id FROM assets WHERE parent_asset_id = 'asset-1')"
      )
      expect(batchSql).toContain('DELETE FROM assets WHERE parent_asset_id')
      expect(batchSql).toContain('DELETE FROM assets')
      expect(batchSql).toContain('COMMIT;')
      expect(batchSql).toContain('asset-1')
    })

    it('removes only deleted asset scoped derived data', async () => {
      const asset = {
        id: 'asset-1',
        itemId: 'item-1',
        path: '/app-data/assets/coll-1/item-1/asset-1.pdf',
        type: 'pdf',
        size: 1024,
        createdAt: 100,
      }
      const otherAsset = {
        id: 'asset-2',
        itemId: 'item-1',
        path: '/app-data/assets/coll-1/item-1/asset-2.pdf',
        type: 'pdf',
        size: 2048,
        createdAt: 101,
      }
      const itemLevelEntity = { id: 'entity-item', item_id: 'item-1', asset_id: null }
      const itemLevelTriple = { id: 'triple-item', item_id: 'item-1', asset_id: null }
      const tables = {
        assets: [asset, otherAsset],
        extractions: [{ id: 'extraction-1', asset_id: 'asset-1' }, { id: 'extraction-2', asset_id: 'asset-2' }],
        layouts: [{ id: 'layout-1', asset_id: 'asset-1' }, { id: 'layout-2', asset_id: 'asset-2' }],
        transcriptions: [{ id: 'transcription-1', asset_id: 'asset-1' }, { id: 'transcription-2', asset_id: 'asset-2' }],
        llm_results: [
          { id: 'llm-1', target_id: 'asset-1', target_type: 'asset' },
          { id: 'llm-2', target_id: 'asset-2', target_type: 'asset' },
          { id: 'llm-item', target_id: 'item-1', target_type: 'item' },
        ],
        annotations: [{ id: 'annotation-1', asset_id: 'asset-1' }, { id: 'annotation-2', asset_id: 'asset-2' }],
        entities: [{ id: 'entity-1', asset_id: 'asset-1' }, { id: 'entity-2', asset_id: 'asset-2' }, itemLevelEntity],
        triples: [{ id: 'triple-1', asset_id: 'asset-1' }, { id: 'triple-2', asset_id: 'asset-2' }, itemLevelTriple],
        vec_assets: [{ asset_id: 'asset-1' }, { asset_id: 'asset-2' }],
      }

      const selectResult = createChainMock([asset])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: vi.fn().mockImplementation(async (sql: string) => {
          expect(sql).toContain('BEGIN')
          expect(sql).toContain('COMMIT')
          tables.extractions = tables.extractions.filter((row) => row.asset_id !== 'asset-1')
          tables.layouts = tables.layouts.filter((row) => row.asset_id !== 'asset-1')
          tables.transcriptions = tables.transcriptions.filter((row) => row.asset_id !== 'asset-1')
          tables.llm_results = tables.llm_results.filter(
            (row) => !(row.target_id === 'asset-1' && (row.target_type === 'asset' || row.target_type === 'unknown'))
          )
          tables.annotations = tables.annotations.filter((row) => row.asset_id !== 'asset-1')
          tables.entities = tables.entities.filter((row) => row.asset_id !== 'asset-1')
          tables.triples = tables.triples.filter((row) => row.asset_id !== 'asset-1')
          tables.vec_assets = tables.vec_assets.filter((row) => row.asset_id !== 'asset-1')
          tables.assets = tables.assets.filter((row) => row.id !== 'asset-1')
        }),
        select: vi.fn().mockResolvedValue([asset]),
      } as unknown as DbClient
      const repoWithRaw = new AssetRepo(db.db, rawClient)

      const result = await repoWithRaw.deleteWithCascade('asset-1')

      expect(result).toEqual(asset)
      expect(tables.assets).toEqual([otherAsset])
      expect(tables.extractions).toEqual([{ id: 'extraction-2', asset_id: 'asset-2' }])
      expect(tables.layouts).toEqual([{ id: 'layout-2', asset_id: 'asset-2' }])
      expect(tables.transcriptions).toEqual([{ id: 'transcription-2', asset_id: 'asset-2' }])
      expect(tables.annotations).toEqual([{ id: 'annotation-2', asset_id: 'asset-2' }])
      expect(tables.entities).toEqual([{ id: 'entity-2', asset_id: 'asset-2' }, itemLevelEntity])
      expect(tables.triples).toEqual([{ id: 'triple-2', asset_id: 'asset-2' }, itemLevelTriple])
      expect(tables.vec_assets).toEqual([{ asset_id: 'asset-2' }])
      expect(tables.llm_results).toEqual([
        { id: 'llm-2', target_id: 'asset-2', target_type: 'asset' },
        { id: 'llm-item', target_id: 'item-1', target_type: 'item' },
      ])
    })

    it('escapes asset ids inside the transactional batch', async () => {
      const asset = {
        id: "asset-'quoted",
        itemId: 'item-1',
        path: '/app-data/assets/coll-1/item-1/quoted.pdf',
        type: 'pdf',
        size: 1024,
        createdAt: 100,
      }

      const selectResult = createChainMock([asset])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: vi.fn().mockResolvedValue(undefined),
        select: vi.fn().mockResolvedValue([asset]),
      } as unknown as DbClient & { executeBatch: ReturnType<typeof vi.fn> }
      const repoWithRaw = new AssetRepo(db.db, rawClient)

      await repoWithRaw.deleteWithCascade("asset-'quoted")

      const batchSql = rawClient.executeBatch.mock.calls[0]?.[0] as string
      expect(batchSql).toContain("asset-''quoted")
      expect(batchSql).not.toContain("asset-'quoted';")
    })

    it('rethrows error when batch execution fails', async () => {
      const asset = {
        id: 'asset-1',
        itemId: 'item-1',
        path: '/app-data/assets/coll-1/item-1/uuid_file.pdf',
        type: 'pdf',
        size: 1024,
        createdAt: 100,
      }

      // Mock Drizzle select for findById (used by deleteWithCascade)
      const selectResult = createChainMock([asset])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: vi.fn().mockRejectedValue(new Error('constraint violation')),
        select: vi.fn().mockResolvedValue([asset]),
      } as unknown as DbClient
      const repoWithRaw = new AssetRepo(db.db, rawClient)

      await expect(repoWithRaw.deleteWithCascade('asset-1')).rejects.toThrow(
        'Failed to delete asset cascade for asset-1: constraint violation'
      )
      expect(rawClient.executeBatch).toHaveBeenCalledWith('ROLLBACK')
    })
  })
})
