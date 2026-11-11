import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AliasResolverRow } from '@entropia/store'
import {
  getEntityAliasResolver,
  identityEntityAliasResolver,
  invalidateEntityAliases,
  loadEntityAliasResolver,
  refreshEntityAliases,
} from './entity-aliases'

const { listMock, storeState } = vi.hoisted(() => ({
  listMock: vi.fn(),
  storeState: { broken: false },
}))

vi.mock('./db', () => ({
  getStore: () => {
    if (storeState.broken) throw new Error('Store not initialized. Call initDb() first.')
    return { entityAliases: { list: listMock } }
  },
}))

const ROWS: AliasResolverRow[] = [{ aliasKey: 'mdme. cuevas', canonical: 'Carmela Cuevas' }]

beforeEach(() => {
  vi.clearAllMocks()
  storeState.broken = false
  listMock.mockResolvedValue(ROWS)
  invalidateEntityAliases()
})

describe('entity aliases resolver loading (T2)', () => {
  it('is the identity resolver before anything loads', () => {
    expect(identityEntityAliasResolver('Mdme. Cuevas')).toBe('Mdme. Cuevas')
    expect(getEntityAliasResolver()('Mdme. Cuevas')).toBe('Mdme. Cuevas')
  })

  it('resolves an alias to its canonical name and leaves other values untouched', async () => {
    const resolve = await loadEntityAliasResolver()

    expect(resolve('Mdme. Cuevas')).toBe('Carmela Cuevas')
    expect(resolve('  MDME.  cuevas ')).toBe('Carmela Cuevas')
    expect(resolve('Carmela Cuevas')).toBe('Carmela Cuevas')
    expect(resolve('Otra persona')).toBe('Otra persona')
  })

  it('loads once and shares the in-flight request', async () => {
    const [first, second] = await Promise.all([
      loadEntityAliasResolver(),
      loadEntityAliasResolver(),
    ])

    expect(first('Mdme. Cuevas')).toBe('Carmela Cuevas')
    expect(second('Mdme. Cuevas')).toBe('Carmela Cuevas')
    expect(listMock).toHaveBeenCalledTimes(1)
    await loadEntityAliasResolver()
    expect(listMock).toHaveBeenCalledTimes(1)
  })

  it('falls back to identity when the store is unavailable, without throwing', async () => {
    storeState.broken = true

    const resolve = await loadEntityAliasResolver()

    expect(resolve('Mdme. Cuevas')).toBe('Mdme. Cuevas')
    expect(getEntityAliasResolver()('Mdme. Cuevas')).toBe('Mdme. Cuevas')
  })

  it('falls back to identity when listing fails, without throwing, and retries later', async () => {
    listMock.mockRejectedValueOnce(new Error('database is locked'))

    const failed = await loadEntityAliasResolver()
    expect(failed('Mdme. Cuevas')).toBe('Mdme. Cuevas')

    const retried = await loadEntityAliasResolver()
    expect(retried('Mdme. Cuevas')).toBe('Carmela Cuevas')
  })

  it('refreshes the list on demand so alias editing (T3) can reload it', async () => {
    await loadEntityAliasResolver()
    expect(getEntityAliasResolver()('Sra. Gómez')).toBe('Sra. Gómez')

    listMock.mockResolvedValue([{ aliasKey: 'sra. gómez', canonical: 'Juana Gómez' }])
    // The cached list stays until someone asks for a reload.
    expect(getEntityAliasResolver()('Sra. Gómez')).toBe('Sra. Gómez')

    const refreshed = await refreshEntityAliases()
    expect(refreshed('Sra. Gómez')).toBe('Juana Gómez')
    expect(getEntityAliasResolver()('Mdme. Cuevas')).toBe('Mdme. Cuevas')
  })
})
