import { DatabaseSync, type SQLInputValue } from 'node:sqlite'
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { applyMigrations } from '../runner'
import { createDrizzleClient } from '../client'
import type { DbClient, DrizzleClient } from '../types'
import {
  EntityAliasRepo,
  buildAliasResolver,
  normalizeEntityAliasKey,
  type EntityAlias,
} from './entity-alias.repo'

function shim(db: DatabaseSync): DbClient {
  return {
    async execute(sql, params = []) {
      return { rowsAffected: Number(db.prepare(sql).run(...(params as SQLInputValue[])).changes) }
    },
    async executeBatch(sql) {
      db.exec(sql)
    },
    async select<T>(sql: string, params: unknown[] = []) {
      return db.prepare(sql).all(...(params as SQLInputValue[])) as T[]
    },
    async selectRows(sql, params = []) {
      return db
        .prepare(sql)
        .all(...(params as SQLInputValue[]))
        .map(Object.values)
    },
  }
}

describe('normalizeEntityAliasKey', () => {
  it('trims, collapses whitespace and lowercases like normalize_entity_value', () => {
    expect(normalizeEntityAliasKey('Mdme. Cuevas')).toBe('mdme. cuevas')
    expect(normalizeEntityAliasKey('  mdme.   cuevas ')).toBe('mdme. cuevas')
    expect(normalizeEntityAliasKey('MDME.\tCUEVAS')).toBe('mdme. cuevas')
  })
})

describe('EntityAliasRepo', () => {
  let sqlite: DatabaseSync
  let repo: EntityAliasRepo

  beforeEach(async () => {
    sqlite = new DatabaseSync(':memory:')
    sqlite.exec('PRAGMA foreign_keys = ON;')
    await applyMigrations(shim(sqlite))
    const drizzle = createDrizzleClient(shim(sqlite)) as DrizzleClient
    repo = new EntityAliasRepo(drizzle)
  })

  afterEach(() => {
    sqlite.close()
  })

  async function rowCount(): Promise<number> {
    return Number(sqlite.prepare('SELECT COUNT(*) AS n FROM entity_aliases').get()?.n ?? 0)
  }

  async function allRows(): Promise<EntityAlias[]> {
    return repo.list()
  }

  it('starts empty and lists rows ordered by key', async () => {
    expect(await repo.list()).toEqual([])

    await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })
    await repo.upsert({ alias: 'Ana', canonical: 'Ana Beltrán' })

    expect((await repo.list()).map((row) => row.aliasKey)).toEqual(['ana', 'mdme. cuevas'])
  })

  it('stores the normalized key, the typed alias and the canonical name', async () => {
    const row = await repo.upsert({ alias: '  Mdme.   Cuevas ', canonical: 'Carmela Cuevas' })

    expect(row.aliasKey).toBe('mdme. cuevas')
    expect(row.alias).toBe('  Mdme.   Cuevas ')
    expect(row.canonical).toBe('Carmela Cuevas')
    expect(row.createdAt).toBeGreaterThan(0)
    expect(row.updatedAt).toBe(row.createdAt)
  })

  it('new rows get a deterministic id from the normalized key (S6 convergence)', async () => {
    const row = await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })

    expect(row.id).toBe('alias:mdme. cuevas')
  })

  it('two independent upserts of the same alias (different spacing/case) produce the same id', async () => {
    const first = await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })
    const second = await repo.upsert({ alias: '  MDME.   cuevas', canonical: 'Carmela Vidal' })

    expect(second.id).toBe(first.id)
    expect(second.id).toBe('alias:mdme. cuevas')
    expect(await rowCount()).toBe(1)
  })

  it('upserting an existing alias key updates its canonical and updated_at', async () => {
    const first = await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })
    const second = await repo.upsert({ alias: '  mdme. cuevas', canonical: 'Carmela Vidal' })

    expect(second.id).toBe(first.id)
    expect(second.canonical).toBe('Carmela Vidal')
    expect(second.updatedAt).toBeGreaterThanOrEqual(first.updatedAt)
    expect(await rowCount()).toBe(1)
  })

  it('deletes by id', async () => {
    const row = await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })
    await repo.upsert({ alias: 'Ana', canonical: 'Ana Beltrán' })

    await repo.delete(row.id)

    expect((await repo.list()).map((r) => r.aliasKey)).toEqual(['ana'])
  })

  it('buildAliasResolver maps variants to the canonical and leaves unknown values unchanged', async () => {
    await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })
    const resolve = buildAliasResolver(await allRows())

    expect(resolve('Mdme. Cuevas')).toBe('Carmela Cuevas')
    expect(resolve('  mdme.   cuevas ')).toBe('Carmela Cuevas')
    expect(resolve('Carmela Cuevas')).toBe('Carmela Cuevas')
    expect(resolve('Otra Persona')).toBe('Otra Persona')
  })

  it('resolution is single-hop: a canonical is never resolved again', () => {
    const resolve = buildAliasResolver([
      { aliasKey: 'mdme. cuevas', canonical: 'Carmela Cuevas' } as EntityAlias,
    ])

    expect(resolve('Carmela Cuevas')).toBe('Carmela Cuevas')
  })

  it.each([
    ['empty alias', { alias: '', canonical: 'Carmela Cuevas' }],
    ['blank alias', { alias: '   ', canonical: 'Carmela Cuevas' }],
    ['empty canonical', { alias: 'Mdme. Cuevas', canonical: '' }],
    ['blank canonical', { alias: 'Mdme. Cuevas', canonical: '  ' }],
    ['alias equal to canonical', { alias: 'Carmela Cuevas', canonical: 'carmela  cuevas' }],
  ])('rejects %s and leaves the table unchanged', async (_label, input) => {
    await repo.upsert({ alias: 'Sra. Pardo', canonical: 'Luisa Pardo' })
    const before = await allRows()

    await expect(repo.upsert(input)).rejects.toThrow()

    expect(await rowCount()).toBe(1)
    expect(await allRows()).toEqual(before)
  })

  it('rejects a canonical that is itself an alias key (no chains)', async () => {
    await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })
    const before = await allRows()

    await expect(
      repo.upsert({ alias: 'La Se\u00f1ora', canonical: 'Mdme. Cuevas' })
    ).rejects.toThrow(/itself an alias/i)

    expect(await rowCount()).toBe(1)
    expect(await allRows()).toEqual(before)
  })

  it('rejects an alias whose key is already used as a canonical by another row', async () => {
    await repo.upsert({ alias: 'Madame', canonical: 'Mdme. Cuevas' })
    const before = await allRows()

    await expect(
      repo.upsert({ alias: 'mdme. cuevas', canonical: 'Carmela Cuevas' })
    ).rejects.toThrow(/already used as a canonical/i)

    expect(await rowCount()).toBe(1)
    expect(await allRows()).toEqual(before)
  })

  it('rejects retargeting an existing alias at another alias key', async () => {
    await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })
    await repo.upsert({ alias: 'Sra. Pardo', canonical: 'Luisa Pardo' })
    const before = await allRows()

    await expect(repo.upsert({ alias: 'Sra. Pardo', canonical: 'Mdme. Cuevas' })).rejects.toThrow(
      /itself an alias/i
    )

    expect(await rowCount()).toBe(2)
    expect(await allRows()).toEqual(before)
  })

  it('never touches entities rows (S2)', async () => {
    sqlite.exec(
      `INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'C', 0, 0);
       INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES ('i1', 'I', 'c1', 0, 0);
       INSERT INTO entities (id, item_id, entity_type, value, created_at)
         VALUES ('e1', 'i1', 'person', 'Mdme. Cuevas', 0);`
    )

    await repo.upsert({ alias: 'Mdme. Cuevas', canonical: 'Carmela Cuevas' })

    expect(sqlite.prepare('SELECT value FROM entities WHERE id = ?').get('e1')).toEqual({
      value: 'Mdme. Cuevas',
    })
  })
})
