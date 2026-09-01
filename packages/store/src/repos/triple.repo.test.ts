import { describe, it, expect, vi, beforeEach } from 'vitest'
import { TripleRepo } from './triple.repo'
import type { DrizzleClient } from '../types'

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

describe('TripleRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: TripleRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new TripleRepo(db.db)
  })

  it('findByItemId returns only triples for the requested item', async () => {
    const rows = [
      {
        id: 't-1',
        itemId: 'item-a',
        subject: 'San Martín',
        predicate: 'cruza',
        object: 'Los Andes',
        createdAt: 1,
      },
    ]
    const selectResult = createChainMock(rows)
    ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

    const result = await repo.findByItemId('item-a')
    expect(result).toHaveLength(1)
    expect(result[0]?.itemId).toBe('item-a')
    expect(result[0]?.subject).toBe('San Martín')
  })

  it('replaceByItemId replaces only target item triples', async () => {
    const valuesMock = vi.fn().mockReturnValue({ returning: vi.fn().mockResolvedValue([]) })
    db.mocks.insert.chain['values'] = valuesMock

    await repo.replaceByItemId('item-a', [
      { subject: 'Belgrano', predicate: 'lidera', object: 'Ejército del Norte' },
      { subject: 'Belgrano', predicate: 'crea', object: 'Bandera' },
    ])

    expect(db.db.delete).toHaveBeenCalledOnce()
    expect(db.db.insert).toHaveBeenCalledOnce()
    expect(valuesMock).toHaveBeenCalledOnce()

    const insertedRows = valuesMock.mock.calls[0]?.[0] as Array<{ itemId: string; subject: string }>
    expect(insertedRows).toHaveLength(2)
    expect(insertedRows[0]?.itemId).toBe('item-a')
    expect(insertedRows[1]?.subject).toBe('Belgrano')
  })

  it('replaceByItemId deletes existing rows when new set is empty', async () => {
    await repo.replaceByItemId('item-empty', [])

    expect(db.db.delete).toHaveBeenCalledOnce()
    expect(db.db.insert).not.toHaveBeenCalled()
  })

  it('create inserts one row shaped exactly like an extracted triple', async () => {
    const valuesMock = vi.fn().mockReturnValue({
      returning: vi.fn().mockResolvedValue([{ id: 'generated' }]),
    })
    db.mocks.insert.chain['values'] = valuesMock

    await repo.create({
      itemId: 'item-a',
      assetId: 'asset-1',
      subject: 'los tripulantes',
      predicate: 'se reintegraron',
      object: 'a sus tareas',
    })

    expect(db.db.insert).toHaveBeenCalledOnce()
    // Sin DELETE previo: agregar una tripleta no puede tocar las existentes.
    expect(db.db.delete).not.toHaveBeenCalled()

    const row = valuesMock.mock.calls[0]?.[0] as Record<string, unknown>
    expect(row).toMatchObject({
      itemId: 'item-a',
      assetId: 'asset-1',
      subject: 'los tripulantes',
      predicate: 'se reintegraron',
      object: 'a sus tareas',
    })
    expect(typeof row['id']).toBe('string')
    expect(typeof row['createdAt']).toBe('number')
  })

  it('create defaults a missing assetId to null, like the extraction path', async () => {
    const valuesMock = vi.fn().mockReturnValue({
      returning: vi.fn().mockResolvedValue([{ id: 'generated' }]),
    })
    db.mocks.insert.chain['values'] = valuesMock

    await repo.create({
      itemId: 'item-a',
      subject: 'la actividad',
      predicate: 'quedó',
      object: 'reiniciada',
    })

    const row = valuesMock.mock.calls[0]?.[0] as Record<string, unknown>
    expect(row['assetId']).toBeNull()
  })

  it('update writes only the S|P|O fields it was given, scoped to one id', async () => {
    const updated = {
      id: 't-1',
      itemId: 'item-a',
      assetId: 'asset-1',
      subject: 'San Martín',
      predicate: 'cruzó',
      object: 'Los Andes',
      createdAt: 1,
    }
    const setMock = vi.fn().mockReturnValue({
      where: vi.fn().mockReturnValue({ returning: vi.fn().mockResolvedValue([updated]) }),
    })
    db.mocks.update.chain['set'] = setMock

    const result = await repo.update('t-1', { predicate: 'cruzó' })

    expect(db.db.update).toHaveBeenCalledOnce()
    // Solo el predicado viaja: itemId, assetId y createdAt identifican la fila
    // y no pueden moverse con una edición manual.
    expect(setMock).toHaveBeenCalledWith({ predicate: 'cruzó' })
    expect(result).toEqual(updated)
  })

  it('update can rewrite the three fields at once', async () => {
    const setMock = vi.fn().mockReturnValue({
      where: vi.fn().mockReturnValue({ returning: vi.fn().mockResolvedValue([{ id: 't-1' }]) }),
    })
    db.mocks.update.chain['set'] = setMock

    await repo.update('t-1', { subject: 'Belgrano', predicate: 'creó', object: 'la Bandera' })

    expect(setMock).toHaveBeenCalledWith({
      subject: 'Belgrano',
      predicate: 'creó',
      object: 'la Bandera',
    })
  })

  it('update ignores an empty patch instead of clearing columns', async () => {
    const setMock = vi.fn().mockReturnValue({
      where: vi.fn().mockReturnValue({ returning: vi.fn().mockResolvedValue([{ id: 't-1' }]) }),
    })
    db.mocks.update.chain['set'] = setMock

    await repo.update('t-1', {})

    expect(setMock).toHaveBeenCalledWith({})
  })

  it('delete removes a single triple and never touches the item as a whole', async () => {
    const whereMock = vi.fn().mockResolvedValue(undefined)
    db.mocks.delete.chain['where'] = whereMock

    await repo.delete('t-1')

    expect(db.db.delete).toHaveBeenCalledOnce()
    expect(whereMock).toHaveBeenCalledOnce()
    expect(db.db.update).not.toHaveBeenCalled()
  })
})
