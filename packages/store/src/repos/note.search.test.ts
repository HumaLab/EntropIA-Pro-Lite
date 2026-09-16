import { DatabaseSync } from 'node:sqlite'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { NoteRepo } from './note.repo'
import type { DbClient, DrizzleClient } from '../types'

/**
 * Searching notes across the corpus (plan-editor.md G11, Unit 5).
 *
 * Against the real schema, because the whole question is one of scope: a search
 * that returns a note from the wrong collection is not a slower search, it is a
 * wrong one. A mocked query builder cannot tell the difference.
 *
 * `notes.item_id` stays `NOT NULL` throughout. §13.1 is explicit that it must
 * not be relaxed and that no fictitious items may be minted to get around it.
 */

const fixturePath = resolve(
  dirname(fileURLToPath(import.meta.url)),
  '../../../../apps/desktop/src-tauri/tests/fixtures/schema_full.sql'
)

interface Seed {
  id: string
  content: string
  itemId: string
  assetId?: string | null
}

function createRealDb(seeds: Seed[]) {
  const sqlite = new DatabaseSync(':memory:')
  sqlite.exec(readFileSync(fixturePath, 'utf8'))
  sqlite.exec(
    `INSERT INTO collections (id, name, created_at, updated_at)
       VALUES ('col-1', 'Pesca', 0, 0), ('col-2', 'Molinos', 0, 0);
     INSERT INTO items (id, title, collection_id, created_at, updated_at)
       VALUES ('it-1', 'Acta del gremio', 'col-1', 0, 0),
              ('it-2', 'Diario de campo', 'col-1', 0, 0),
              ('it-3', 'Padron rural', 'col-2', 0, 0);
     INSERT INTO assets (id, item_id, path, type, created_at, sort_index)
       VALUES ('as-1', 'it-1', '/p/1.png', 'image', 0, 0)`
  )

  const insert = sqlite.prepare(
    'INSERT INTO notes (id, item_id, asset_id, content, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)'
  )
  seeds.forEach((seed, index) => {
    insert.run(seed.id, seed.itemId, seed.assetId ?? null, seed.content, index, index)
  })

  const rawClient = {
    select: async <T>(sql: string, params: unknown[] = []): Promise<T[]> =>
      sqlite
        .prepare(sql)
        .all(...(params as Array<null | string | number | bigint | Uint8Array>)) as T[],
    execute: async () => ({ rowsAffected: 0 }),
    executeBatch: async (sql: string) => {
      sqlite.exec(sql)
    },
  } as unknown as DbClient

  return new NoteRepo({} as unknown as DrizzleClient, rawClient)
}

const SEEDS: Seed[] = [
  { id: 'n1', itemId: 'it-1', content: 'los obreros del filet reanudaron', assetId: 'as-1' },
  { id: 'n2', itemId: 'it-2', content: 'nota sobre los molineros' },
  { id: 'n3', itemId: 'it-3', content: 'padron de molineros rurales' },
]

describe('searching notes across the corpus', () => {
  it('finds a note by its text wherever it lives', async () => {
    const repo = createRealDb(SEEDS)

    const found = await repo.search({ query: 'molineros' })

    expect(found.map((note) => note.id).sort()).toEqual(['n2', 'n3'])
  })

  /** A note is unreadable without knowing what it is attached to. */
  it('reports the item and collection each note belongs to', async () => {
    const repo = createRealDb(SEEDS)

    const [found] = await repo.search({ query: 'filet' })

    expect(found).toMatchObject({
      id: 'n1',
      itemId: 'it-1',
      itemTitle: 'Acta del gremio',
      collectionId: 'col-1',
      assetId: 'as-1',
    })
  })

  /**
   * The scope is the point. A manuscript associated with one collection must
   * not be offered notes from another.
   */
  it('narrows to a collection', async () => {
    const repo = createRealDb(SEEDS)

    const found = await repo.search({ query: 'molineros', collectionIds: ['col-1'] })

    expect(found.map((note) => note.id)).toEqual(['n2'])
  })

  it('narrows to several collections at once', async () => {
    const repo = createRealDb(SEEDS)

    const found = await repo.search({ query: 'molineros', collectionIds: ['col-1', 'col-2'] })

    expect(found.map((note) => note.id).sort()).toEqual(['n2', 'n3'])
  })

  it('narrows to one item', async () => {
    const repo = createRealDb(SEEDS)

    const found = await repo.search({ itemId: 'it-3' })

    expect(found.map((note) => note.id)).toEqual(['n3'])
  })

  /** An empty scope list is not "every collection"; it is no collection. */
  it('finds nothing when narrowed to an empty set of collections', async () => {
    const repo = createRealDb(SEEDS)

    expect(await repo.search({ query: 'molineros', collectionIds: [] })).toEqual([])
  })

  it('lists everything in scope when no text was asked for', async () => {
    const repo = createRealDb(SEEDS)

    const found = await repo.search({ collectionIds: ['col-1'] })

    expect(found.map((note) => note.id).sort()).toEqual(['n1', 'n2'])
  })

  /** The query is text someone typed, not a pattern. */
  it('treats LIKE wildcards in the query as literal characters', async () => {
    const repo = createRealDb([
      { id: 'n1', itemId: 'it-1', content: 'cien por ciento' },
      { id: 'n2', itemId: 'it-1', content: 'un 100% seguro' },
    ])

    expect((await repo.search({ query: '100%' })).map((note) => note.id)).toEqual(['n2'])
    // A lone `%` finds the note that actually contains one, not every note.
    // Unescaped it would match all of them, which is the bug this guards.
    expect((await repo.search({ query: '%' })).map((note) => note.id)).toEqual(['n2'])
    expect((await repo.search({ query: '_' })).map((note) => note.id)).toEqual([])
  })

  it('is case-insensitive, like every other search in the app', async () => {
    const repo = createRealDb(SEEDS)

    expect((await repo.search({ query: 'MOLINEROS' })).map((note) => note.id).sort()).toEqual([
      'n2',
      'n3',
    ])
  })

  it('honours a limit', async () => {
    const repo = createRealDb(SEEDS)

    expect(await repo.search({ limit: 1 })).toHaveLength(1)
  })
})
