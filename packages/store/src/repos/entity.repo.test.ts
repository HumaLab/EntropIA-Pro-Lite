import { DatabaseSync } from 'node:sqlite'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { EntityRepo } from './entity.repo'
import { applyMigrations } from '../runner'
import { createDrizzleClient } from '../client'
import type { DbClient, DrizzleClient } from '../types'

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

describe('EntityRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: EntityRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new EntityRepo(db.db)
  })

  describe('findByItemId', () => {
    it('returns empty array when no entities exist for item', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByItemId('item-1')
      expect(result).toEqual([])
    })

    it('returns entities for a given item', async () => {
      const now = Date.now()
      const entityRows = [
        {
          id: 'e1',
          itemId: 'item-1',
          entityType: 'person',
          value: 'Don Pedro',
          startOffset: 0,
          endOffset: 9,
          confidence: 0.95,
          createdAt: now,
        },
        {
          id: 'e2',
          itemId: 'item-1',
          entityType: 'place',
          value: 'Buenos Aires',
          startOffset: 20,
          endOffset: 32,
          confidence: 1.0,
          createdAt: now,
        },
      ]
      const selectResult = createChainMock(entityRows)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByItemId('item-1')
      expect(result).toHaveLength(2)
      expect(result[0]!.value).toBe('Don Pedro')
      expect(result[1]!.value).toBe('Buenos Aires')
    })
  })

  describe('findByItemIdAndType', () => {
    it('returns only entities matching the specified type', async () => {
      const now = Date.now()
      const entityRows = [
        {
          id: 'e1',
          itemId: 'item-1',
          entityType: 'person',
          value: 'Don Pedro',
          startOffset: 0,
          endOffset: 9,
          confidence: 0.9,
          createdAt: now,
        },
      ]
      const selectResult = createChainMock(entityRows)
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByItemIdAndType('item-1', 'person')
      expect(result).toHaveLength(1)
      expect(result[0]!.entityType).toBe('person')
    })

    it('returns empty array when no entities match type', async () => {
      const selectResult = createChainMock([])
      ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

      const result = await repo.findByItemIdAndType('item-1', 'date')
      expect(result).toEqual([])
    })
  })

  describe('create', () => {
    it('creates and returns a new entity with generated id', async () => {
      const now = Date.now()
      const newEntity = {
        id: 'generated-uuid',
        itemId: 'item-1',
        entityType: 'person',
        value: 'Don Pedro',
        startOffset: 0,
        endOffset: 9,
        confidence: 0.95,
        createdAt: now,
      }

      const returningMock = vi.fn().mockResolvedValue([newEntity])
      const valuesMock = vi.fn().mockReturnValue({ returning: returningMock })
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        itemId: 'item-1',
        entityType: 'person',
        value: 'Don Pedro',
        startOffset: 0,
        endOffset: 9,
        confidence: 0.95,
        createdAt: now,
      })

      expect(result.value).toBe('Don Pedro')
      expect(result.entityType).toBe('person')
      expect(result.itemId).toBe('item-1')
    })

    it('creates entity with default confidence when not provided', async () => {
      const now = Date.now()
      const newEntity = {
        id: 'uuid-2',
        itemId: 'item-2',
        entityType: 'place',
        value: 'Córdoba',
        startOffset: 5,
        endOffset: 12,
        confidence: 1.0,
        createdAt: now,
      }

      const returningMock = vi.fn().mockResolvedValue([newEntity])
      const valuesMock = vi.fn().mockReturnValue({ returning: returningMock })
      db.mocks.insert.chain['values'] = valuesMock

      const result = await repo.create({
        itemId: 'item-2',
        entityType: 'place',
        value: 'Córdoba',
        startOffset: 5,
        endOffset: 12,
        createdAt: now,
      })

      expect(result.value).toBe('Córdoba')
      expect(result.confidence).toBe(1.0)
    })
  })

  describe('update', () => {
    it('invalidates stale coordinates when an entity value changes', async () => {
      await repo.update('place-1', { entityType: 'place', value: 'Mar del Plata' })

      expect(db.mocks.update.chain['set']).toHaveBeenCalledWith(
        expect.objectContaining({
          value: 'Mar del Plata',
          latitude: null,
          longitude: null,
          geoStatus: 'pending',
        })
      )
    })
  })

  describe('manual location', () => {
    it('persists signed coordinates separately from geocoding coordinates', async () => {
      await repo.setManualLocation('place-1', -34.6037, -58.3816)

      expect(db.mocks.update.chain['set']).toHaveBeenCalledWith({
        manualLatitude: -34.6037,
        manualLongitude: -58.3816,
      })
    })

    it('resets the entity to its automatic location', async () => {
      await repo.resetManualLocation('place-1')

      expect(db.mocks.update.chain['set']).toHaveBeenCalledWith({
        manualLatitude: null,
        manualLongitude: null,
      })
    })

    it('rejects coordinates outside Leaflet latitude bounds', async () => {
      await expect(repo.setManualLocation('place-1', 91, -58.3816)).rejects.toThrow(RangeError)
    })
  })

  describe('deleteByItemId', () => {
    it('calls delete for the given item id without error', async () => {
      await expect(repo.deleteByItemId('item-1')).resolves.toBeUndefined()
    })

    it('delete is called on the db (verifies the method executes)', async () => {
      const deleteSpy = vi.fn().mockReturnValue({
        where: vi.fn().mockResolvedValue({ rowsAffected: 3 }),
      })
      ;(db.db.delete as ReturnType<typeof vi.fn>).mockImplementation(deleteSpy)

      await repo.deleteByItemId('item-1')
      expect(deleteSpy).toHaveBeenCalled()
    })
  })

  describe('listDistinctValues (T5 suggestions)', () => {
    let sqlite: DatabaseSync
    let realRepo: EntityRepo

    function shim(source: DatabaseSync): DbClient {
      return {
        async execute(sql, params = []) {
          return {
            rowsAffected: Number(source.prepare(sql).run(...(params as never[])).changes),
          }
        },
        async executeBatch(sql) {
          source.exec(sql)
        },
        async select<T>(sql: string, params: unknown[] = []) {
          return source.prepare(sql).all(...(params as never[])) as T[]
        },
        async selectRows(sql, params = []) {
          return source
            .prepare(sql)
            .all(...(params as never[]))
            .map(Object.values)
        },
      }
    }

    beforeEach(async () => {
      sqlite = new DatabaseSync(':memory:')
      sqlite.exec('PRAGMA foreign_keys = ON;')
      await applyMigrations(shim(sqlite))
      sqlite.exec(
        `INSERT INTO collections (id, name, created_at, updated_at) VALUES ('col-1', 'One', 0, 0)`
      )
      sqlite.exec(
        `INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at)
         VALUES ('item-1', 'Doc', 'col-1', NULL, 0, 0)`
      )
      realRepo = new EntityRepo(createDrizzleClient(shim(sqlite)) as DrizzleClient)
    })

    afterEach(() => {
      sqlite.close()
    })

    function seed(value: string, type: string, source: string | null, count: number): void {
      const insert = sqlite.prepare(
        `INSERT INTO entities (id, item_id, entity_type, value, source, created_at)
         VALUES (?, 'item-1', ?, ?, ?, 0)`
      )
      for (let index = 0; index < count; index += 1) {
        insert.run(`${value}-${type}-${source ?? 'live'}-${index}`, type, value, source)
      }
    }

    it('returns distinct person values with mention counts across the archive', async () => {
      seed('Carmela Cuevas', 'person', null, 2)
      seed('Mdme. Cuevas', 'person', null, 3)
      seed('Buenos Aires', 'place', null, 5)

      expect(await realRepo.listDistinctValues({ types: ['person'] })).toEqual([
        { value: 'Mdme. Cuevas', mentions: 3 },
        { value: 'Carmela Cuevas', mentions: 2 },
      ])
    })

    it('merges rows that normalize to the same alias key', async () => {
      seed('Carmela Cuevas', 'person', null, 2)
      seed('  carmela   cuevas ', 'person', null, 1)

      expect(await realRepo.listDistinctValues({ types: ['person'] })).toEqual([
        { value: 'Carmela Cuevas', mentions: 3 },
      ])
    })

    it('excludes manually deleted mentions', async () => {
      seed('Carmela Cuevas', 'person', null, 2)
      seed('Carmela Cuevas', 'person', 'manual_deleted', 4)
      seed('Juan Cuevas', 'person', 'manual', 1)

      expect(await realRepo.listDistinctValues({ types: ['person'] })).toEqual([
        { value: 'Carmela Cuevas', mentions: 2 },
        { value: 'Juan Cuevas', mentions: 1 },
      ])
    })

    it('returns nothing for an empty type filter', async () => {
      seed('Carmela Cuevas', 'person', null, 1)

      await expect(realRepo.listDistinctValues({ types: [] })).resolves.toEqual([])
    })
  })
})
