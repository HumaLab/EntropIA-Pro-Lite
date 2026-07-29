import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EntityRepo } from './entity.repo'
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
})
