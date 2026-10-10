import { describe, it, expect, vi, beforeEach } from 'vitest'
import { CollectionRepo } from './collection.repo'
import type { DrizzleClient, DbClient } from '../types'

/**
 * CollectionRepo tests.
 * Strategy: mock the DrizzleClient's Drizzle query-builder chain.
 * We build a chainable mock that captures method calls and returns
 * predictable data, then assert the repo returns the correct shape.
 */

// Helper: create a chainable mock that resolves with the given value
function createChainMock(resolveValue: unknown = []) {
  const chain: Record<string, ReturnType<typeof vi.fn>> = {}

  const createProxy = (): unknown =>
    new Proxy(() => {}, {
      apply: () => (resolveValue instanceof Promise ? resolveValue : Promise.resolve(resolveValue)),
      get: (_target, prop) => {
        if (prop === 'then') {
          // Make the proxy thenable so `await chain` resolves
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

function createMockRawClient() {
  return {
    execute: vi.fn().mockResolvedValue({ rowsAffected: 0 }),
    executeBatch: vi.fn().mockResolvedValue(undefined),
    executeTransaction: vi.fn().mockResolvedValue(undefined),
    select: vi.fn().mockResolvedValue([]),
  } as unknown as DbClient
}

describe('CollectionRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: CollectionRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new CollectionRepo(db.db)
  })

  describe('create', () => {
    it('returns a collection with generated id and timestamps', async () => {
      const valuesMock = vi.fn().mockResolvedValue(undefined)
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        name: 'Research Papers',
        description: 'My papers',
      })

      expect(result.name).toBe('Research Papers')
      expect(result.description).toBe('My papers')
      expect(result.id).toBeTruthy()
      expect(typeof result.id).toBe('string')
      expect(result.createdAt).toBeTypeOf('number')
      expect(result.updatedAt).toBeTypeOf('number')
      expect(result.updatedAt).toBe(result.createdAt)

      expect(valuesMock).toHaveBeenCalledTimes(1)
      const insertedCollection = valuesMock.mock.calls[0]![0]
      expect(insertedCollection).toEqual(result)
    })

    it('generates unique ids for different collections', async () => {
      const ids: string[] = []
      const valuesMock = vi.fn().mockImplementation(async () => {
        // Extract the id from the call to values()
        const data = valuesMock.mock.calls[valuesMock.mock.calls.length - 1]![0]
        ids.push(data.id)
      })
      db.mocks.insert.chain['values'] = valuesMock

      await repo.create({ name: 'Collection A' })
      await repo.create({ name: 'Collection B' })

      expect(ids).toHaveLength(2)
      expect(ids[0]).not.toBe(ids[1])
      // IDs should be non-empty strings
      expect(ids[0]!.length).toBeGreaterThan(0)
      expect(ids[1]!.length).toBeGreaterThan(0)
    })
  })

  describe('findAll', () => {
    it('returns empty array when no collections exist', async () => {
      // Default mock resolves with []
      const result = await repo.findAll()
      expect(result).toEqual([])
    })

    it('returns collections sorted by updated_at desc', async () => {
      const collections = [
        { id: '1', name: 'Newer', description: null, createdAt: 100, updatedAt: 200 },
        { id: '2', name: 'Older', description: null, createdAt: 50, updatedAt: 100 },
      ]

      // Reconfigure select mock to return collections
      const selectResult = createChainMock(collections)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findAll()
      expect(result).toEqual(collections)
      expect(result).toHaveLength(2)
      expect(result[0]!.name).toBe('Newer')
    })
  })

  describe('findAllNonEmpty', () => {
    it('uses rawClient when available', async () => {
      const rawClient = createMockRawClient()
      const collections = [
        { id: '1', name: 'Has Items', description: null, createdAt: 100, updatedAt: 200 },
      ]
      ;(rawClient.select as ReturnType<typeof vi.fn>).mockResolvedValue(collections)

      const repoWithRaw = new CollectionRepo(db.db, rawClient)
      const result = await repoWithRaw.findAllNonEmpty()

      expect(result).toEqual(collections)
      expect(rawClient.select).toHaveBeenCalledWith(expect.stringContaining('INNER JOIN items'))
    })

    it('falls back to Drizzle when rawClient is not available', async () => {
      const collections = [
        { id: '1', name: 'Has Items', description: null, createdAt: 100, updatedAt: 200 },
      ]
      const selectResult = createChainMock(collections)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findAllNonEmpty()

      expect(result).toEqual(collections)
    })
  })

  describe('findById', () => {
    it('returns null when collection not found', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findById('non-existent')
      expect(result).toBeNull()
    })

    it('returns the collection when found', async () => {
      const collection = { id: 'abc', name: 'Found', description: null, createdAt: 1, updatedAt: 2 }
      const selectResult = createChainMock([collection])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findById('abc')
      expect(result).toEqual(collection)
      expect(result!.id).toBe('abc')
    })
  })

  describe('update', () => {
    it('returns updated collection with new updatedAt', async () => {
      const updated = {
        id: 'u1',
        name: 'Updated Name',
        description: null,
        createdAt: 100,
        updatedAt: 999,
      }

      const returningMock = vi.fn().mockResolvedValue([updated])
      const whereMock = vi.fn().mockReturnValue({ returning: returningMock })
      const setMock = vi.fn().mockReturnValue({ where: whereMock })
      db.mocks.update.chain['set'] = setMock

      const result = await repo.update('u1', { name: 'Updated Name' })
      expect(result).toEqual(updated)
      expect(result.name).toBe('Updated Name')
    })
  })

  describe('delete', () => {
    it('throws when rawClient is not available', async () => {
      await expect(repo.delete('col-1')).rejects.toThrow(
        'delete requires a rawClient for transactional execution'
      )
    })

    it('executes one parameterized transaction for core tables', async () => {
      const rawClient = createMockRawClient()
      const repoWithRaw = new CollectionRepo(db.db, rawClient)

      await repoWithRaw.delete('col-1')

      const executeTransaction = rawClient.executeTransaction as ReturnType<typeof vi.fn>
      expect(executeTransaction).toHaveBeenCalledOnce()
      const statements = executeTransaction.mock.calls[0]![0] as Array<{
        sql: string
        params: unknown[]
      }>
      const sql = statements.map((statement) => statement.sql).join('\n')
      // The command owns the transaction; every entry is a single statement.
      expect(sql).not.toContain('BEGIN')
      expect(sql).not.toContain('COMMIT')
      expect(statements.every((statement) => !statement.sql.includes(';'))).toBe(true)
      expect(sql).toContain('DELETE FROM extractions')
      expect(sql).toContain('DELETE FROM layouts')
      expect(sql).toContain('DELETE FROM llm_results')
      expect(sql).toContain('DELETE FROM assets')
      expect(sql).toContain('DELETE FROM entities')
      expect(sql).toContain('DELETE FROM triples')
      expect(sql).toContain('DELETE FROM notes')
      expect(sql).toContain('DELETE FROM items')
      expect(sql).toContain('DELETE FROM collections')
      // Optional tables should NOT be in the transaction
      expect(sql).not.toContain('DELETE FROM vec_items')
      expect(sql).not.toContain('DELETE FROM embeddings_fallback')
      expect(sql).not.toContain('DELETE FROM fts_index')
      // The item's own index row leaves by rowid inside the same transaction.
      expect(sql).toContain(
        'DELETE FROM fts_items WHERE rowid IN (SELECT rowid FROM items WHERE'
      )
      // Ids travel bound, never interpolated into the SQL.
      expect(statements.some((statement) => statement.sql.includes('col-1'))).toBe(false)
      expect(
        statements.every((statement) => statement.params.includes('col-1'))
      ).toBe(true)
    })

    it('cleans up optional vec_assets after the transaction with bound ids', async () => {
      const rawExecuteMock = vi.fn().mockResolvedValue({ rowsAffected: 0 })
      const rawClient = {
        execute: rawExecuteMock,
        executeBatch: vi.fn().mockResolvedValue(undefined),
        executeTransaction: vi.fn().mockResolvedValue(undefined),
        select: vi.fn().mockResolvedValue([{ id: 'item-1' }]),
      } as unknown as DbClient
      const repoWithRaw = new CollectionRepo(db.db, rawClient)

      await repoWithRaw.delete('col-1')

      // The embedding cleanup stays best-effort: its table may not exist.
      expect(rawExecuteMock).toHaveBeenCalledOnce()
      expect(rawExecuteMock).toHaveBeenCalledWith(
        'DELETE FROM vec_assets WHERE item_id IN (?)',
        ['item-1']
      )
    })

    it('skips the optional cleanup when the collection has no items', async () => {
      const rawExecuteMock = vi.fn().mockResolvedValue({ rowsAffected: 0 })
      const rawClient = {
        execute: rawExecuteMock,
        executeBatch: vi.fn().mockResolvedValue(undefined),
        executeTransaction: vi.fn().mockResolvedValue(undefined),
        select: vi.fn().mockResolvedValue([]),
      } as unknown as DbClient
      const repoWithRaw = new CollectionRepo(db.db, rawClient)

      await repoWithRaw.delete('col-1')

      expect(rawClient.executeTransaction).toHaveBeenCalledOnce()
      expect(rawExecuteMock).not.toHaveBeenCalled()
    })

    it('binds a collection ID with single quotes instead of interpolating it', async () => {
      const rawClient = createMockRawClient()
      const repoWithRaw = new CollectionRepo(db.db, rawClient)

      await repoWithRaw.delete("col'; DROP TABLE collections;--")

      const statements = (rawClient.executeTransaction as ReturnType<typeof vi.fn>).mock
        .calls[0]![0] as Array<{ sql: string; params: unknown[] }>
      expect(
        statements.some((statement) => statement.sql.includes("col'; DROP TABLE"))
      ).toBe(false)
      expect(
        statements.every((statement) =>
          statement.params.includes("col'; DROP TABLE collections;--")
        )
      ).toBe(true)
    })

    it('wraps errors with context message', async () => {
      const rawClient = createMockRawClient()
      ;(rawClient.executeTransaction as ReturnType<typeof vi.fn>).mockRejectedValue(
        new Error('constraint violation')
      )
      const repoWithRaw = new CollectionRepo(db.db, rawClient)

      await expect(repoWithRaw.delete('col-1')).rejects.toThrow(
        'Failed to delete collection col-1: constraint violation'
      )
      // The command is atomic and rolls back server-side; the repo sends no ROLLBACK.
      expect(rawClient.executeBatch).not.toHaveBeenCalled()
    })
  })

  describe('deleteIfEmpty', () => {
    it('throws when rawClient is not available', async () => {
      await expect(repo.deleteIfEmpty('col-1')).rejects.toThrow(
        'deleteIfEmpty requires a rawClient for transactional execution'
      )
    })

    it('returns true when collection is deleted (had 0 items)', async () => {
      const rawClient = createMockRawClient()
      // After delete, the collection no longer exists
      ;(rawClient.select as ReturnType<typeof vi.fn>).mockResolvedValue([])
      const repoWithRaw = new CollectionRepo(db.db, rawClient)

      const result = await repoWithRaw.deleteIfEmpty('col-1')

      expect(result).toBe(true)
      expect(rawClient.executeBatch).toHaveBeenCalledWith(
        expect.stringContaining('DELETE FROM collections')
      )
    })

    it('returns false when collection still exists (had items)', async () => {
      const rawClient = createMockRawClient()
      // After delete attempt, the collection still exists (had items)
      ;(rawClient.select as ReturnType<typeof vi.fn>).mockResolvedValue([{ id: 'col-1' }])
      const repoWithRaw = new CollectionRepo(db.db, rawClient)

      const result = await repoWithRaw.deleteIfEmpty('col-1')

      expect(result).toBe(false)
    })
  })

  describe('countItems', () => {
    it('returns 0 when collection has no items', async () => {
      const selectResult = createChainMock([{ count: 0 }])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.countItems('empty-col')
      expect(result).toBe(0)
    })

    it('returns correct count when collection has items', async () => {
      const selectResult = createChainMock([{ count: 5 }])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.countItems('col-with-items')
      expect(result).toBe(5)
    })
  })
})
