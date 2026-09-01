import { DatabaseSync } from 'node:sqlite'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ItemRepo } from './item.repo'
import type { ItemCursor, ItemPage } from './item.repo'
import { FtsRepo, compileCardSearchQuery } from './fts.repo'
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

  describe('getCollectionStats', () => {
    function createStatsSqlite() {
      const db = new DatabaseSync(':memory:')
      db.exec(`
        CREATE TABLE items (
          id TEXT PRIMARY KEY, title TEXT, collection_id TEXT NOT NULL,
          metadata TEXT, created_at INTEGER, updated_at INTEGER
        );
        CREATE TABLE assets (
          id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT,
          type TEXT, sort_index INTEGER, size INTEGER, parent_asset_id TEXT,
          page_number INTEGER, created_at INTEGER
        );
        CREATE TABLE extractions (
          id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL,
          method TEXT NOT NULL, confidence REAL, created_at INTEGER
        );
        CREATE TABLE vec_assets (
          asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL,
          embedding_model TEXT NOT NULL DEFAULT 'legacy',
          embedding_contract TEXT NOT NULL DEFAULT 'legacy',
          dimensions INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE entities (
          id TEXT PRIMARY KEY NOT NULL, item_id TEXT NOT NULL, asset_id TEXT,
          entity_type TEXT NOT NULL, value TEXT NOT NULL,
          start_offset INTEGER NOT NULL DEFAULT 0, end_offset INTEGER NOT NULL DEFAULT 0,
          confidence REAL NOT NULL DEFAULT 1.0, source TEXT, model_name TEXT,
          latitude REAL, longitude REAL, manual_lat REAL, manual_lon REAL,
          geo_status TEXT NOT NULL DEFAULT 'pending', created_at INTEGER NOT NULL
        );
        CREATE TABLE triples (
          id TEXT PRIMARY KEY NOT NULL, item_id TEXT NOT NULL, asset_id TEXT,
          subject TEXT NOT NULL, predicate TEXT NOT NULL, object TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );
      `)
      // col-1: 3 items, 3 VIEWABLE assets (a2 is a PDF parent container and
      // is never counted, even though it owns page child a3 and has its own
      // extraction row e3 — parents are not viewable assets).
      //   i1: a1 (OCR direct + embedding direct + entity direct)
      //   i2: a2 (PDF parent, excluded) + a3 (page child) — item-level
      //       entity covers the leaf a3
      //   i3: a4 (embedding direct + item-level triple covers it)
      // col-2: i4 with a5 — must stay outside the counts.
      db.exec(`
        INSERT INTO items VALUES
          ('i1','A','col-1',NULL,0,0), ('i2','B','col-1',NULL,0,0),
          ('i3','C','col-1',NULL,0,0), ('i4','D','col-2',NULL,0,0);
        INSERT INTO assets (id, item_id, path, type, created_at) VALUES
          ('a1','i1','p','image',0),
          ('a2','i2','p','pdf',0),
          ('a3','i2','p','pdf',0),
          ('a4','i3','p','image',0),
          ('a5','i4','p','image',0);
        UPDATE assets SET parent_asset_id = 'a2', page_number = 1 WHERE id = 'a3';
        INSERT INTO extractions VALUES
          ('e1','a1','text','ocr',0.9,0),
          ('e2','a3','text','native',0.9,0),
          ('e3','a2','text','ocr',0.9,0);
        INSERT INTO vec_assets (asset_id, item_id, embedding) VALUES
          ('a1','i1',X'01'), ('a4','i3',X'02');
        INSERT INTO entities (id, item_id, asset_id, entity_type, value, created_at) VALUES
          ('en1','i1','a1','person','X',0),
          ('en2','i2',NULL,'place','Y',0);
        INSERT INTO triples VALUES
          ('t1','i2','a3','S','P','O',0),
          ('t2','i3',NULL,'S','P','O',0);
      `)
      return db
    }

    it('counts items, all assets, and processed assets across real SQLite rows', async () => {
      const db = createStatsSqlite()
      const rawClient = {
        select: async <T>(sql: string, params: unknown[] = []): Promise<T[]> =>
          db
            .prepare(sql)
            .all(...(params as Array<null | string | number | bigint | Uint8Array>)) as T[],
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo({} as unknown as DrizzleClient, rawClient)

      const result = await repoWithRaw.getCollectionStats('col-1')

      // 3 items, 3 viewable assets — the PDF parent a2 (with its own
      // extraction e3) never counts. OCR: a1 + a3. Embed: a1 + a4.
      // NER: a1 direct + a3 via item-level entity on i2. Triples: a3 direct
      // + a4 via item-level triple on i3.
      expect(result).toEqual({
        items: 3,
        assets: 3,
        ocr: 2,
        embeddings: 2,
        ner: 2,
        triples: 2,
      })

      // Rows outside the collection are never counted.
      const otherCollection = await repoWithRaw.getCollectionStats('col-2')
      expect(otherCollection).toEqual({
        items: 1,
        assets: 1,
        ocr: 0,
        embeddings: 0,
        ner: 0,
        triples: 0,
      })
    })

    it('maps raw row counts into the typed result', async () => {
      const rawSelectMock = vi.fn().mockResolvedValue([
        {
          items_count: 3,
          assets_count: 16,
          ocr_count: 13,
          embed_count: 13,
          ner_count: 8,
          triples_count: 2,
        },
      ])
      const rawClient = {
        execute: vi.fn(),
        select: rawSelectMock,
      } as unknown as DbClient
      const repoWithRaw = new ItemRepo(db.db, rawClient)

      const result = await repoWithRaw.getCollectionStats('col-1')

      expect(rawSelectMock).toHaveBeenCalledWith(
        expect.stringContaining('FROM extractions e'),
        ['col-1', 'col-1', 'col-1', 'col-1', 'col-1', 'col-1']
      )
      const sql = rawSelectMock.mock.calls[0]?.[0] as string
      expect(sql).toContain('vec_assets')
      expect(sql).toContain('entities')
      expect(sql).toContain('triples')
      expect(sql).toContain('en.asset_id IS NULL')
      expect(sql).toContain('tr.asset_id IS NULL')
      // Parent containers that own page children never count as assets.
      expect(sql).toContain('viewable_assets')
      expect(sql).toContain('child.parent_asset_id')
      expect(result).toEqual({
        items: 3,
        assets: 16,
        ocr: 13,
        embeddings: 13,
        ner: 8,
        triples: 2,
      })
    })

    it('falls back to zero stats through Drizzle when no raw client is available', async () => {
      const result = await repo.getCollectionStats('col-1')
      expect(result).toEqual({
        items: 0,
        assets: 0,
        ocr: 0,
        embeddings: 0,
        ner: 0,
        triples: 0,
      })
    })
  })
})

// ============================================================================
// Keyset pagination, paginated search, and sibling navigation.
//
// These run against the real checked-in schema fixture rather than an ad-hoc
// CREATE TABLE, because the whole point of work unit 1 is that the composite
// index exists and is actually chosen by the planner.
// ============================================================================
describe('keyset pagination against the real schema', () => {
  const fixturePath = resolve(
    dirname(fileURLToPath(import.meta.url)),
    '../../../../apps/desktop/src-tauri/tests/fixtures/schema_full.sql'
  )

  type Seed = { id: string; title: string; collectionId?: string; metadata?: string }

  function createRealDb(seeds: Seed[]) {
    const sqlite = new DatabaseSync(':memory:')
    sqlite.exec(readFileSync(fixturePath, 'utf8'))
    sqlite.exec(
      `INSERT INTO collections (id, name, created_at, updated_at)
       VALUES ('col-1', 'One', 0, 0), ('col-2', 'Two', 0, 0)`
    )

    const insertItem = sqlite.prepare(
      'INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?, ?, ?, ?, 0, 0)'
    )
    const insertAsset = sqlite.prepare(
      'INSERT INTO assets (id, item_id, path, type, created_at, sort_index) VALUES (?, ?, ?, ?, 0, 0)'
    )
    for (const seed of seeds) {
      insertItem.run(seed.id, seed.title, seed.collectionId ?? 'col-1', seed.metadata ?? null)
      insertAsset.run(`asset-${seed.id}`, seed.id, `/p/${seed.id}.png`, 'image')
    }

    const executed: string[] = []
    const rawClient = {
      select: async <T>(sql: string, params: unknown[] = []): Promise<T[]> => {
        executed.push(sql)
        return sqlite
          .prepare(sql)
          .all(...(params as Array<null | string | number | bigint | Uint8Array>)) as T[]
      },
      execute: async (sql: string, params: unknown[] = []) => {
        executed.push(sql)
        sqlite.prepare(sql).run(...(params as Array<null | string | number | bigint | Uint8Array>))
        return { rowsAffected: 0 }
      },
      executeBatch: async (sql: string) => {
        sqlite.exec(sql)
      },
    } as unknown as DbClient

    const realRepo = new ItemRepo({} as unknown as DrizzleClient, rawClient)
    return { sqlite, rawClient, repo: realRepo, executed }
  }

  async function indexFts(rawClient: DbClient) {
    await new FtsRepo(rawClient).rebuildIndex()
  }

  const fiveDocs: Seed[] = [
    { id: 'doc-a', title: 'Alpha' },
    { id: 'doc-b', title: 'Bravo' },
    { id: 'doc-09', title: 'Luna' },
    { id: 'doc-10', title: 'Mosaic' },
    { id: 'doc-11', title: 'Nimbus' },
  ]

  it('returns a stable first page and next cursor', async () => {
    const { repo: realRepo } = createRealDb(fiveDocs)

    const page = await realRepo.findCardSummariesPage('col-1', { limit: 2 })

    expect(page.items.map((row) => row.id)).toEqual(['doc-a', 'doc-b'])
    expect(page.nextCursor).toEqual({ title: 'Bravo', id: 'doc-b' })
    expect(page.hasMore).toBe(true)
    expect(page.items[0]).toMatchObject({
      title: 'Alpha',
      collectionId: 'col-1',
      assetCount: 1,
      primaryAssetId: 'asset-doc-a',
      primaryAssetPath: '/p/doc-a.png',
      primaryAssetType: 'image',
    })
  })

  it('walks the whole collection with no duplicates and no skips', async () => {
    const { repo: realRepo } = createRealDb(fiveDocs)

    const seen: string[] = []
    let cursor: ItemCursor | null = null
    let guard = 0
    for (;;) {
      const page: ItemPage = await realRepo.findCardSummariesPage('col-1', { cursor, limit: 2 })
      seen.push(...page.items.map((row) => row.id))
      if (!page.hasMore) break
      cursor = page.nextCursor
      if ((guard += 1) > 10) throw new Error('pagination did not terminate')
    }

    expect(seen).toEqual(['doc-a', 'doc-b', 'doc-09', 'doc-10', 'doc-11'])
    expect(new Set(seen).size).toBe(seen.length)
  })

  it('ends with hasMore false and a null cursor', async () => {
    const { repo: realRepo } = createRealDb(fiveDocs)

    const page = await realRepo.findCardSummariesPage('col-1', {
      cursor: { title: 'Mosaic', id: 'doc-10' },
      limit: 2,
    })

    expect(page.items.map((row) => row.id)).toEqual(['doc-11'])
    expect(page.hasMore).toBe(false)
    expect(page.nextCursor).toBeNull()
  })

  it('never returns rows from another collection', async () => {
    const { repo: realRepo } = createRealDb([
      ...fiveDocs,
      { id: 'other-1', title: 'Alpha', collectionId: 'col-2' },
    ])

    const page = await realRepo.findCardSummariesPage('col-1', { limit: 50 })

    expect(page.items.map((row) => row.id)).toEqual([
      'doc-a',
      'doc-b',
      'doc-09',
      'doc-10',
      'doc-11',
    ])
  })

  it('skips no document when an already-read row is deleted mid-scroll', async () => {
    // This is the property OFFSET cannot provide: the cursor is a data value,
    // so removing an earlier row does not shift the rows still to come.
    const { repo: realRepo, sqlite } = createRealDb(fiveDocs)

    const first = await realRepo.findCardSummariesPage('col-1', { limit: 2 })
    sqlite.exec("DELETE FROM assets WHERE item_id = 'doc-a'; DELETE FROM items WHERE id = 'doc-a'")
    const second = await realRepo.findCardSummariesPage('col-1', {
      cursor: first.nextCursor,
      limit: 2,
    })

    expect(second.items.map((row) => row.id)).toEqual(['doc-09', 'doc-10'])
  })

  it('uses idx_items_collection_title with no temp b-tree sort', async () => {
    const { sqlite, repo: realRepo, executed } = createRealDb(fiveDocs)
    await realRepo.findCardSummariesPage('col-1', { limit: 2 })

    const pageSql = executed.find((sql) => sql.includes('FROM items i'))!
    const plan = sqlite.prepare(`EXPLAIN QUERY PLAN ${pageSql}`).all('col-1', 3) as Array<{
      detail: string
    }>
    const outerSteps = plan
      .map((row) => row.detail)
      .slice(0, 2)
      .join('\n')

    expect(outerSteps).toContain('idx_items_collection_title')
    expect(outerSteps).not.toContain('USE TEMP B-TREE FOR ORDER BY')
    expect(outerSteps).not.toContain('SCAN i')
  })

  describe('paginated search', () => {
    const searchDocs: Seed[] = Array.from({ length: 120 }, (_, index) => ({
      id: `hit-${String(index).padStart(3, '0')}`,
      title: `Cronica roja ${String(index).padStart(3, '0')}`,
    }))

    it('returns FTS matches past the old hardcoded limit of 50', async () => {
      const { repo: realRepo, rawClient } = createRealDb(searchDocs)
      await indexFts(rawClient)

      const seen: string[] = []
      let cursor: ItemCursor | null = null
      for (;;) {
        const page: ItemPage = await realRepo.findCardSummariesPage('col-1', {
          cursor,
          limit: 40,
          search: compileCardSearchQuery('cronica'),
        })
        seen.push(...page.items.map((row) => row.id))
        if (!page.hasMore) break
        cursor = page.nextCursor
      }

      expect(seen).toHaveLength(120)
      expect(new Set(seen).size).toBe(120)
    })

    it('applies the relaxed OR retry when the strict match finds nothing', async () => {
      const { repo: realRepo, rawClient } = createRealDb([
        { id: 'doc-a', title: 'Sindicato Obrero' },
        { id: 'doc-b', title: 'Industria del Pescado' },
        { id: 'doc-c', title: 'Acta capitular' },
      ])
      await indexFts(rawClient)

      const page = await realRepo.findCardSummariesPage('col-1', {
        search: compileCardSearchQuery('Sindicato Pescado'),
      })

      expect(page.items.map((row) => row.id)).toEqual(['doc-b', 'doc-a'])
    })

    it('falls back to LIKE when FTS matches nothing at all', async () => {
      // No rebuildIndex() call: fts_items is empty, so both MATCH branches miss
      // and only the LIKE seam can find the row.
      const { repo: realRepo } = createRealDb([
        { id: 'doc-a', title: 'Alpha' },
        { id: 'doc-b', title: 'Bravo', metadata: '{"fondo":"expediente"}' },
      ])

      const byTitle = await realRepo.findCardSummariesPage('col-1', {
        search: compileCardSearchQuery('Bravo'),
      })
      const byMetadata = await realRepo.findCardSummariesPage('col-1', {
        search: compileCardSearchQuery('expediente'),
      })

      expect(byTitle.items.map((row) => row.id)).toEqual(['doc-b'])
      expect(byMetadata.items.map((row) => row.id)).toEqual(['doc-b'])
    })

    it('returns an empty page for a query that sanitizes away entirely', async () => {
      const { repo: realRepo, rawClient } = createRealDb(fiveDocs)
      await indexFts(rawClient)

      const page = await realRepo.findCardSummariesPage('col-1', {
        search: compileCardSearchQuery('   '),
      })

      expect(page.items).toEqual([])
      expect(page.hasMore).toBe(false)
      expect(page.nextCursor).toBeNull()
    })

    it('never builds one SQL placeholder per matched id', async () => {
      // The old path expanded `i.id IN (?, ?, ...)`, which caps out against
      // SQLITE_MAX_VARIABLE_NUMBER as soon as the FTS limit is raised.
      const { repo: realRepo, rawClient, executed } = createRealDb(searchDocs)
      await indexFts(rawClient)
      executed.length = 0

      await realRepo.findCardSummariesPage('col-1', {
        limit: 120,
        search: compileCardSearchQuery('cronica'),
      })

      for (const sql of executed) expect(sql).not.toMatch(/\?(\s*,\s*\?){10,}/)
    })
  })

  describe('sibling keyset navigation', () => {
    it('returns previous and next siblings with single-row keyset queries', async () => {
      const { repo: realRepo, executed } = createRealDb(fiveDocs)
      executed.length = 0

      const previous = await realRepo.findPreviousCardSummary('col-1', {
        title: 'Mosaic',
        id: 'doc-10',
      })
      const next = await realRepo.findNextCardSummary('col-1', { title: 'Mosaic', id: 'doc-10' })

      expect(previous).toMatchObject({ id: 'doc-09', title: 'Luna', collectionId: 'col-1' })
      expect(next).toMatchObject({ id: 'doc-11', title: 'Nimbus', collectionId: 'col-1' })
      expect(executed).toHaveLength(2)
      for (const sql of executed) expect(sql).toContain('LIMIT 1')
    })

    it('returns null at each edge of the collection', async () => {
      const { repo: realRepo } = createRealDb(fiveDocs)

      await expect(
        realRepo.findPreviousCardSummary('col-1', { title: 'Alpha', id: 'doc-a' })
      ).resolves.toBeNull()
      await expect(
        realRepo.findNextCardSummary('col-1', { title: 'Nimbus', id: 'doc-11' })
      ).resolves.toBeNull()
    })

    it('does not cross into another collection to find a sibling', async () => {
      const { repo: realRepo } = createRealDb([
        { id: 'doc-a', title: 'Alpha' },
        { id: 'other-1', title: 'Bravo', collectionId: 'col-2' },
      ])

      await expect(
        realRepo.findNextCardSummary('col-1', { title: 'Alpha', id: 'doc-a' })
      ).resolves.toBeNull()
    })
  })
})
