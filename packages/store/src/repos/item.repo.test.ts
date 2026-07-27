import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ItemRepo } from './item.repo'
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

describe('ItemRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: ItemRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new ItemRepo(db.db)
  })

  describe('create', () => {
    it('returns a locally-constructed item and inserts it without returning()', async () => {
      const valuesMock = vi.fn().mockResolvedValue(undefined)
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        title: 'Test Document',
        collectionId: 'col-1',
      })

      expect(valuesMock).toHaveBeenCalledOnce()
      expect(valuesMock.mock.calls[0]?.[0]).toEqual(result)
      expect(typeof result.id).toBe('string')
      expect(result.title).toBe('Test Document')
      expect(result.collectionId).toBe('col-1')
      expect(result.metadata).toBeNull()
      expect(typeof result.createdAt).toBe('number')
      expect(typeof result.updatedAt).toBe('number')
    })

    it('includes metadata when provided', async () => {
      const meta = JSON.stringify({ author: 'Jane' })
      const valuesMock = vi.fn().mockResolvedValue(undefined)
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        title: 'With Metadata',
        collectionId: 'col-1',
        metadata: meta,
      })

      expect(valuesMock).toHaveBeenCalledOnce()
      expect(valuesMock.mock.calls[0]?.[0]).toEqual(result)
      expect(result.metadata).toBe(meta)
    })

    it('uses raw client INSERT when provided', async () => {
      const rawExecuteMock = vi.fn().mockResolvedValue({ rowsAffected: 1 })
      const rawClient = {
        execute: rawExecuteMock,
        select: vi.fn().mockResolvedValue([{ id: 'col-raw' }]),
      } as unknown as DbClient
      const repo2 = new ItemRepo(db.db, rawClient)

      const result = await repo2.create({
        title: 'Raw Insert',
        collectionId: 'col-raw',
        metadata: null,
      })

      expect(rawExecuteMock).toHaveBeenCalledOnce()
      expect(rawExecuteMock).toHaveBeenCalledWith(
        'INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)',
        [
          result.id,
          result.title,
          result.collectionId,
          result.metadata,
          result.createdAt,
          result.updatedAt,
        ]
      )
      expect(db.db.insert).not.toHaveBeenCalled()
    })

    it('throws when collection does not exist (raw client)', async () => {
      const rawExecuteMock = vi.fn().mockResolvedValue({ rowsAffected: 1 })
      const rawClient = {
        execute: rawExecuteMock,
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repo2 = new ItemRepo(db.db, rawClient)

      await expect(
        repo2.create({
          title: 'Orphan Item',
          collectionId: 'non-existent-col',
          metadata: null,
        })
      ).rejects.toThrow('collection "non-existent-col" does not exist')

      expect(rawExecuteMock).not.toHaveBeenCalled()
    })
  })

  describe('findByCollection', () => {
    it('returns empty array when collection has no items', async () => {
      const result = await repo.findByCollection('empty-col')
      expect(result).toEqual([])
    })

    it('returns items for a specific collection', async () => {
      const items = [
        {
          id: 'i1',
          title: 'Doc A',
          collectionId: 'col-1',
          metadata: null,
          createdAt: 100,
          updatedAt: 200,
        },
        {
          id: 'i2',
          title: 'Doc B',
          collectionId: 'col-1',
          metadata: null,
          createdAt: 150,
          updatedAt: 250,
        },
      ]

      const selectResult = createChainMock(items)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByCollection('col-1')
      expect(result).toEqual(items)
      expect(result).toHaveLength(2)
      expect(selectResult.chain['orderBy']).toHaveBeenCalledOnce()
    })
  })

  describe('findCardSummariesByCollection', () => {
    it('uses one raw query for item card summaries with asset counts and primary asset fields', async () => {
      const rawSelectMock = vi.fn().mockResolvedValue([
        {
          id: 'item-1',
          title: 'Doc A',
          collection_id: 'col-1',
          metadata: null,
          created_at: 100,
          updated_at: 200,
          asset_count: 2,
          primary_asset_id: 'asset-image-1',
          primary_asset_path: '/assets/doc-a.jpg',
          primary_asset_type: 'image',
        },
      ])
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
        executeBatch: vi.fn().mockResolvedValue(undefined),
        select: rawSelectMock,
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo(db.db, rawClient)

      const result = await repoWithRaw.findCardSummariesByCollection('col-1')

      expect(rawSelectMock).toHaveBeenCalledOnce()
      expect(rawSelectMock.mock.calls[0]?.[0]).toContain('AS asset_count')
      expect(rawSelectMock.mock.calls[0]?.[0]).toContain('LEFT JOIN assets pa')
      expect(rawSelectMock.mock.calls[0]?.[0]).toContain('NOT EXISTS')
      expect(rawSelectMock.mock.calls[0]?.[0]).toContain('child.parent_asset_id = leaf.id')
      expect(rawSelectMock.mock.calls[0]?.[0]).toContain('p.parent_asset_id IS NULL')
      expect(rawSelectMock.mock.calls[0]?.[0]).toContain(
        'ORDER BY i.title COLLATE NOCASE ASC, i.id ASC'
      )
      expect(rawSelectMock.mock.calls[0]?.[1]).toEqual(['col-1'])
      expect(db.db.select).not.toHaveBeenCalled()
      expect(result).toEqual([
        {
          id: 'item-1',
          title: 'Doc A',
          collectionId: 'col-1',
          metadata: null,
          createdAt: 100,
          updatedAt: 200,
          assetCount: 2,
          primaryAssetId: 'asset-image-1',
          primaryAssetPath: '/assets/doc-a.jpg',
          primaryAssetType: 'image',
        },
      ])
    })

    it('uses existing text search results to filter summary queries when searching', async () => {
      const rawSelectMock = vi.fn().mockResolvedValue([])
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
        executeBatch: vi.fn().mockResolvedValue(undefined),
        select: rawSelectMock,
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo(db.db, rawClient)
      vi.spyOn(repoWithRaw, 'searchByText').mockResolvedValue([
        {
          id: 'item-match',
          title: 'Acta con texto OCR',
          collectionId: 'col-1',
          metadata: null,
          createdAt: 100,
          updatedAt: 200,
        },
      ])

      await repoWithRaw.findCardSummariesByCollection('col-1', 'acta')

      expect(repoWithRaw.searchByText).toHaveBeenCalledWith('col-1', 'acta')
      expect(rawSelectMock.mock.calls[0]?.[0]).toContain('i.id IN (?)')
      expect(rawSelectMock.mock.calls[0]?.[1]).toEqual(['item-match'])
    })
  })

  describe('findById', () => {
    it('returns null when item not found', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findById('non-existent')
      expect(result).toBeNull()
    })

    it('returns the item when found', async () => {
      const item = {
        id: 'found-1',
        title: 'Found Item',
        collectionId: 'col-1',
        metadata: null,
        createdAt: 1,
        updatedAt: 2,
      }
      const selectResult = createChainMock([item])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findById('found-1')
      expect(result).toEqual(item)
      expect(result!.id).toBe('found-1')
    })
  })

  describe('update', () => {
    it('returns updated item with new title and updatedAt', async () => {
      const updated = {
        id: 'u1',
        title: 'New Title',
        collectionId: 'col-1',
        metadata: null,
        createdAt: 100,
        updatedAt: 999,
      }

      const returningMock = vi.fn().mockResolvedValue([updated])
      const whereMock = vi.fn().mockReturnValue({ returning: returningMock })
      const setMock = vi.fn().mockReturnValue({ where: whereMock })
      db.mocks.update.chain['set'] = setMock

      const result = await repo.update('u1', { title: 'New Title' })
      expect(result).toEqual(updated)
      expect(result.title).toBe('New Title')
    })

    it('updates metadata field', async () => {
      const newMeta = JSON.stringify({ tags: ['important'] })
      const updated = {
        id: 'u2',
        title: 'Same',
        collectionId: 'col-1',
        metadata: newMeta,
        createdAt: 100,
        updatedAt: 999,
      }

      const returningMock = vi.fn().mockResolvedValue([updated])
      const whereMock = vi.fn().mockReturnValue({ returning: returningMock })
      const setMock = vi.fn().mockReturnValue({ where: whereMock })
      db.mocks.update.chain['set'] = setMock

      const result = await repo.update('u2', { metadata: newMeta })
      expect(result.metadata).toBe(newMeta)
    })
  })

  describe('delete', () => {
    it('completes without error', async () => {
      await expect(repo.delete('del-1')).resolves.toBeUndefined()
    })
  })

  describe('deleteWithCascade', () => {
    it('throws when rawClient is not provided', async () => {
      const repoNoRaw = new ItemRepo(db.db)
      await expect(repoNoRaw.deleteWithCascade('item-1')).rejects.toThrow(
        'deleteWithCascade requires a rawClient'
      )
    })

    it('executes batch delete for core tables within a transaction', async () => {
      const rawExecuteBatchMock = vi.fn().mockResolvedValue(undefined)
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: rawExecuteBatchMock,
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo(db.db, rawClient)

      await repoWithRaw.deleteWithCascade('item-1')

      expect(rawExecuteBatchMock).toHaveBeenCalledOnce()
      const batchSql = rawExecuteBatchMock.mock.calls[0]?.[0] as string
      // Core tables (always exist) — in atomic transaction
      expect(batchSql).toContain('BEGIN')
      expect(batchSql).toContain('DELETE FROM extractions')
      expect(batchSql).toContain('DELETE FROM layouts')
      expect(batchSql).toContain('DELETE FROM llm_results')
      expect(batchSql).toContain('DELETE FROM assets')
      expect(batchSql).toContain('DELETE FROM entities')
      expect(batchSql).toContain('DELETE FROM triples')
      expect(batchSql).toContain('DELETE FROM notes')
      expect(batchSql).toContain('DELETE FROM items')
      expect(batchSql).toContain('DELETE FROM collections')
      expect(batchSql).toContain('COMMIT')
      expect(batchSql).toContain('item-1')
      // Optional tables should NOT be in the batch
      expect(batchSql).not.toContain('DELETE FROM vec_items')
      expect(batchSql).not.toContain('DELETE FROM embeddings_fallback')
      expect(batchSql).not.toContain('DELETE FROM fts_index')
      expect(batchSql).not.toContain('DELETE FROM fts_items')
    })

    it('cleans up optional tables after core transaction succeeds', async () => {
      const rawExecuteMock = vi.fn().mockResolvedValue({ rowsAffected: 0 })
      const rawClient = {
        execute: rawExecuteMock,
        executeBatch: vi.fn().mockResolvedValue(undefined),
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo(db.db, rawClient)

      await repoWithRaw.deleteWithCascade('item-1')

      // Optional tables are cleaned up with individual execute calls
      const executeCalls = rawExecuteMock.mock.calls.map((c) => c[0] as string)
      expect(
        executeCalls.some((sql) =>
          sql.includes("INSERT INTO fts_items(fts_items) VALUES ('delete-all')")
        )
      ).toBe(true)
      expect(
        executeCalls.some((sql) =>
          sql.includes('INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)')
        )
      ).toBe(true)
      expect(executeCalls.some((sql) => sql.includes('DELETE FROM fts_items WHERE item_id'))).toBe(
        false
      )
      expect(executeCalls.some((sql) => sql.includes('DELETE FROM vec_items'))).toBe(false)
      expect(executeCalls.some((sql) => sql.includes('DELETE FROM embeddings_fallback'))).toBe(
        false
      )
      expect(executeCalls.some((sql) => sql.includes('DELETE FROM vec_assets'))).toBe(true)
    })

    it('rethrows error when batch execution fails', async () => {
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: vi.fn().mockRejectedValue(new Error('constraint violation')),
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo(db.db, rawClient)

      await expect(repoWithRaw.deleteWithCascade('item-1')).rejects.toThrow(
        'Failed to delete item cascade for item-1: constraint violation'
      )
      expect(rawClient.executeBatch).toHaveBeenCalledWith('ROLLBACK')
    })

    it('escapes single quotes in item ID to prevent SQL injection', async () => {
      const rawExecuteBatchMock = vi.fn().mockResolvedValue(undefined)
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 1 }),
        executeBatch: rawExecuteBatchMock,
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo(db.db, rawClient)

      await repoWithRaw.deleteWithCascade("item'; DROP TABLE items;--")

      const batchSql = rawExecuteBatchMock.mock.calls[0]?.[0] as string
      expect(batchSql).toContain("item''; DROP TABLE items;--")
      expect(batchSql).not.toContain("item'; DROP TABLE items;--")
    })
  })

  describe('searchByText', () => {
    it('returns empty when no matches found', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.searchByText('col-1', 'nonexistent')
      expect(result).toEqual([])
    })

    it('returns matching items for the collection', async () => {
      const matchingItems = [
        {
          id: 'i1',
          title: 'Machine Learning Paper',
          collectionId: 'col-1',
          metadata: null,
          createdAt: 100,
          updatedAt: 200,
        },
      ]

      const selectResult = createChainMock(matchingItems)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.searchByText('col-1', 'machine')
      expect(result).toEqual(matchingItems)
      expect(result).toHaveLength(1)
      expect(result[0]!.title).toBe('Machine Learning Paper')
    })

    it('returns items matching metadata field', async () => {
      const matchingItems = [
        {
          id: 'i2',
          title: 'Untitled Document',
          collectionId: 'col-1',
          metadata: JSON.stringify({ author: 'Darwin', year: '1859' }),
          createdAt: 100,
          updatedAt: 200,
        },
      ]

      const selectResult = createChainMock(matchingItems)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.searchByText('col-1', 'Darwin')
      expect(result).toHaveLength(1)
      expect(result[0]!.metadata).toContain('Darwin')
    })
  })

  describe('searchByFts5', () => {
    it('returns FtsResult[] with itemId and rank from FTS5 search', async () => {
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
        select: vi.fn().mockResolvedValue([
          { item_id: 'item-1', rank: -0.5 },
          { item_id: 'item-2', rank: -1.2 },
        ]),
      } as unknown as DbClient

      const repo2 = new ItemRepo(db.db, rawClient)
      const results = await repo2.searchByFts5('cabildo')
      expect(results).toHaveLength(2)
      expect(results[0]!.itemId).toBe('item-1')
      expect(results[1]!.itemId).toBe('item-2')
    })

    it('returns empty array when FTS5 finds no matches', async () => {
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient

      const repo2 = new ItemRepo(db.db, rawClient)
      const results = await repo2.searchByFts5('xyznonexistentterm')
      expect(results).toEqual([])
    })

    it('falls back to LIKE search when rawClient is not provided', async () => {
      const matchingItems = [
        {
          id: 'i1',
          title: 'Acta de cabildo',
          collectionId: 'col-1',
          metadata: null,
          createdAt: 100,
          updatedAt: 200,
        },
      ]

      const selectResult = createChainMock(matchingItems)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      // No rawClient — uses LIKE fallback
      const result = await repo.searchByText('col-1', 'cabildo')
      expect(result).toHaveLength(1)
      expect(result[0]!.title).toBe('Acta de cabildo')
    })

    it('returns empty for empty query in FTS5 path', async () => {
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient

      const repo2 = new ItemRepo(db.db, rawClient)
      const results = await repo2.searchByFts5('')
      expect(results).toEqual([])
    })
  })

  describe('searchByText with FTS5 integration', () => {
    it('uses FTS5 results when rawClient is provided and FTS5 returns matches', async () => {
      // FTS5 returns specific item IDs
      const rawSelectMock = vi.fn().mockResolvedValue([
        { item_id: 'item-fts-1', rank: -0.5 },
        { item_id: 'item-fts-2', rank: -1.0 },
      ])
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
        select: rawSelectMock,
      } as unknown as DbClient

      const ftsItem1 = {
        id: 'item-fts-1',
        title: 'Acta notarial de cabildo',
        collectionId: 'col-1',
        metadata: null,
        createdAt: 100,
        updatedAt: 200,
      }
      const ftsItem2 = {
        id: 'item-fts-2',
        title: 'Documento de cabildo',
        collectionId: 'col-1',
        metadata: null,
        createdAt: 50,
        updatedAt: 150,
      }

      // Drizzle mock returns the two items (for the follow-up findByIds query)
      const selectResult = createChainMock([ftsItem1, ftsItem2])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const repo2 = new ItemRepo(db.db, rawClient)
      const results = await repo2.searchByText('col-1', 'cabildo')

      // FTS5 was called (rawClient.select was invoked)
      expect(rawSelectMock).toHaveBeenCalled()
      // Results contain both FTS5-matched items
      expect(results).toHaveLength(2)
    })

    it('falls back to LIKE when FTS5 returns no results', async () => {
      // FTS5 returns nothing
      const rawSelectMock = vi.fn().mockResolvedValue([])
      const rawClient = {
        execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
        select: rawSelectMock,
      } as unknown as DbClient

      const likeItems = [
        {
          id: 'like-1',
          title: 'Rare Document',
          collectionId: 'col-1',
          metadata: null,
          createdAt: 100,
          updatedAt: 200,
        },
      ]
      const selectResult = createChainMock(likeItems)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const repo2 = new ItemRepo(db.db, rawClient)
      const results = await repo2.searchByText('col-1', 'rare')

      // FTS5 was tried (rawClient.select was invoked)
      expect(rawSelectMock).toHaveBeenCalled()
      // FTS5 returned nothing, so Drizzle LIKE fallback was used
      expect(results).toHaveLength(1)
      expect(results[0]!.id).toBe('like-1')
    })
  })
})
