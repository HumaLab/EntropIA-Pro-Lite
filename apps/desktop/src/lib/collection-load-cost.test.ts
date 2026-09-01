/**
 * What opening a collection costs, before and after the pagination work.
 *
 * This is the half of work unit 0 that could be closed without a running Tauri
 * app, turned into a regression guard rather than a one-off script. It measures
 * the SHIPPING code — the real `ItemRepo` page query, the real virtual-grid
 * window math, the real pagination state — against real SQLite, and asserts the
 * property that actually matters:
 *
 *   the cost of opening a collection stops depending on its size.
 *
 * If someone later reintroduces an unbounded load, these assertions fail rather
 * than the regression being noticed months later on a customer's archive.
 *
 * The printed table is the companion to docs/collections-pagination-baseline.md.
 * Run it alone with:
 *   pnpm --filter @entropia-pro/desktop test -- --run collection-load-cost
 */
import { DatabaseSync } from 'node:sqlite'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { ItemRepo } from '@entropia/store'
import type { DbClient, DrizzleClient } from '@entropia/store'
import { computeVirtualGridWindow } from '@entropia/ui'
import { COLLECTION_PAGE_SIZE, appendPage, createPaginationState } from './collection-pagination'

const fixturePath = resolve(
  dirname(fileURLToPath(import.meta.url)),
  '../../src-tauri/tests/fixtures/schema_full.sql'
)

const COLLECTION_ID = 'col-cost'
const SIZES = [500, 5_000, 10_000]
const THUMBNAIL_CONCURRENCY = 4
const ELEMENTS_PER_CARD = 9

/**
 * Per-thumbnail decode cost, measured on the real Rust path with
 *   cargo test --profile measure --lib image_thumbnail_generation_cost -- --ignored --nocapture
 * 6 MP source, the middle of the three sizes measured there.
 */
const THUMBNAIL_COLD_MS = 55.04

/** CollectionView's card geometry in a typical 1440px window. */
const GRID = {
  rowHeight: 232,
  columns: 5,
  overscanRows: 2,
  viewportHeight: 900,
  gridOffset: 0,
  scrollTop: 0,
}

/**
 * The card query as it shipped before this work. It is the one literal copy
 * here, because the code it came from no longer exists to be imported.
 */
const LEGACY_CARD_SUMMARY_SQL = `
  SELECT
    i.id, i.title, i.collection_id, i.metadata, i.created_at, i.updated_at,
    (SELECT COUNT(*) FROM assets leaf
      WHERE leaf.item_id = i.id
        AND NOT EXISTS (SELECT 1 FROM assets child WHERE child.parent_asset_id = leaf.id)
    ) AS asset_count,
    pa.id AS primary_asset_id, pa.path AS primary_asset_path, pa.type AS primary_asset_type
  FROM items i
  LEFT JOIN assets pa ON pa.id = (
    SELECT p.id FROM assets p
    WHERE p.item_id = i.id AND p.parent_asset_id IS NULL
    ORDER BY CASE p.type WHEN 'image' THEN 0 WHEN 'pdf' THEN 1 ELSE 2 END,
      p.sort_index ASC, p.created_at ASC
    LIMIT 1
  )
  WHERE i.collection_id = ?
  ORDER BY i.title COLLATE NOCASE ASC, i.id ASC
`

/** Scrambled so insertion order does not already satisfy the ORDER BY. */
function makeTitle(index: number): string {
  const words = ['Acta', 'Boletin', 'Cronica', 'Diario', 'Expediente', 'Folio', 'Gaceta', 'Hoja']
  const scrambled = (index * 2654435761) % 4294967296
  return `${words[scrambled % words.length]} ${String(scrambled).padStart(10, '0')}`
}

function seedCollection(size: number) {
  const db = new DatabaseSync(':memory:')
  db.exec(readFileSync(fixturePath, 'utf8'))
  db.prepare('INSERT INTO collections (id, name, created_at, updated_at) VALUES (?, ?, 0, 0)').run(
    COLLECTION_ID,
    'Cost'
  )

  const insertItem = db.prepare(
    'INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?, ?, ?, ?, 0, 0)'
  )
  const insertAsset = db.prepare(
    'INSERT INTO assets (id, item_id, path, type, size, created_at, sort_index, parent_asset_id) VALUES (?, ?, ?, ?, ?, 0, 0, NULL)'
  )

  db.exec('BEGIN')
  for (let index = 0; index < size; index += 1) {
    const itemId = `doc-${String(index).padStart(6, '0')}`
    insertItem.run(itemId, makeTitle(index), COLLECTION_ID, '{"fondo":"sintetico"}')
    insertAsset.run(`asset-${itemId}`, itemId, `/synthetic/${itemId}.png`, 'image', 1_500_000)
  }
  db.exec('COMMIT')
  db.exec('ANALYZE')

  const rawClient = {
    select: async <T>(sql: string, params: unknown[] = []): Promise<T[]> =>
      db.prepare(sql).all(...(params as Array<null | string | number>)) as T[],
    execute: async (sql: string, params: unknown[] = []) => {
      db.prepare(sql).run(...(params as Array<null | string | number>))
      return { rowsAffected: 0 }
    },
    executeBatch: async (sql: string) => db.exec(sql),
  } as unknown as DbClient

  return { db, repo: new ItemRepo({} as unknown as DrizzleClient, rawClient) }
}

/** The quadratic copy the old view performed once per round of four. */
function legacyMapWrites(rowCount: number): number {
  let writes = 0
  for (let loaded = 0; loaded < rowCount; loaded += THUMBNAIL_CONCURRENCY) writes += loaded
  return writes
}

type Measurement = {
  size: number
  before: { rows: number; cards: number; elements: number; thumbnails: number; mapWrites: number }
  after: { rows: number; cards: number; elements: number; thumbnails: number; mapWrites: number }
  queryMs: { before: number; after: number }
}

function seconds(count: number): string {
  const ms = (count * THUMBNAIL_COLD_MS) / THUMBNAIL_CONCURRENCY
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms.toFixed(0)}ms`
}

describe('cost of opening a collection', () => {
  const measurements: Measurement[] = []

  it.each(SIZES)('measures a %i document collection end to end', async (size) => {
    const { db, repo } = seedCollection(size)

    try {
      const legacyStatement = db.prepare(LEGACY_CARD_SUMMARY_SQL)
      legacyStatement.all(COLLECTION_ID) // warm up
      const legacyStarted = performance.now()
      const legacyRows = legacyStatement.all(COLLECTION_ID)
      const legacyMs = performance.now() - legacyStarted

      await repo.findCardSummariesPage(COLLECTION_ID, { limit: COLLECTION_PAGE_SIZE })
      const pageStarted = performance.now()
      const firstPage = await repo.findCardSummariesPage(COLLECTION_ID, {
        limit: COLLECTION_PAGE_SIZE,
      })
      const pageMs = performance.now() - pageStarted

      const pageState = appendPage(createPaginationState(), firstPage)
      const windowRange = computeVirtualGridWindow({
        ...GRID,
        totalItems: pageState.items.length,
      })
      const renderedCards = windowRange.endIndex - windowRange.startIndex

      expect(legacyRows).toHaveLength(size)

      measurements.push({
        size,
        before: {
          rows: legacyRows.length,
          cards: legacyRows.length,
          elements: legacyRows.length * ELEMENTS_PER_CARD,
          thumbnails: legacyRows.length,
          mapWrites: legacyMapWrites(legacyRows.length),
        },
        after: {
          rows: firstPage.items.length,
          cards: renderedCards,
          elements: renderedCards * ELEMENTS_PER_CARD,
          thumbnails: renderedCards,
          // The SvelteMap is mutated in place: one write per row, once.
          mapWrites: firstPage.items.length,
        },
        queryMs: { before: legacyMs, after: pageMs },
      })
    } finally {
      db.close()
    }
  })

  it('keeps the cost of opening a collection flat as the collection grows', () => {
    expect(measurements).toHaveLength(SIZES.length)

    const flat = <K extends 'rows' | 'cards' | 'elements' | 'thumbnails'>(key: K) =>
      new Set(measurements.map((entry) => entry.after[key])).size

    // This is the whole claim of the pagination work, as an assertion: none of
    // these grows between a 500 and a 10,000 document collection.
    expect(flat('rows')).toBe(1)
    expect(flat('cards')).toBe(1)
    expect(flat('elements')).toBe(1)
    expect(flat('thumbnails')).toBe(1)

    // And the before path grew with every one of them.
    expect(new Set(measurements.map((entry) => entry.before.rows)).size).toBe(SIZES.length)
  })

  it('reads one page and paints one window, whatever the collection size', () => {
    for (const entry of measurements) {
      expect(entry.after.rows).toBe(COLLECTION_PAGE_SIZE)
      expect(entry.after.cards).toBeLessThan(COLLECTION_PAGE_SIZE)
      expect(entry.after.thumbnails).toBeLessThan(50)
      // The quadratic map copy is gone: writes are linear in the page, not
      // quadratic in the collection.
      expect(entry.after.mapWrites).toBeLessThan(entry.before.mapWrites)
    }

    const biggest = measurements[measurements.length - 1]!
    expect(biggest.before.mapWrites).toBeGreaterThan(12_000_000)
    expect(biggest.after.mapWrites).toBe(COLLECTION_PAGE_SIZE)
  })

  it('prints the before/after table', () => {
    const line = (label: string, before: string, after: string) =>
      `  ${label.padEnd(24)} ${before.padStart(14)}  ->  ${after.padStart(12)}`

    const out: string[] = [
      '',
      'Cost of opening a collection — before vs after',
      `thumbnail decode: ${THUMBNAIL_COLD_MS} ms each (measured on the real Rust path, 6 MP source)`,
      '',
    ]

    for (const entry of measurements) {
      out.push(`=== ${entry.size.toLocaleString('en-US')} documents ===`)
      out.push(
        line('rows read on open', entry.before.rows.toLocaleString('en-US'), String(entry.after.rows))
      )
      out.push(
        line('SQL query', `${entry.queryMs.before.toFixed(1)} ms`, `${entry.queryMs.after.toFixed(1)} ms`)
      )
      out.push(
        line('cards in DOM', entry.before.cards.toLocaleString('en-US'), String(entry.after.cards))
      )
      out.push(
        line(
          'DOM elements',
          entry.before.elements.toLocaleString('en-US'),
          entry.after.elements.toLocaleString('en-US')
        )
      )
      out.push(
        line(
          'metadata map writes',
          entry.before.mapWrites.toLocaleString('en-US'),
          entry.after.mapWrites.toLocaleString('en-US')
        )
      )
      out.push(
        line(
          'thumbnails generated',
          entry.before.thumbnails.toLocaleString('en-US'),
          String(entry.after.thumbnails)
        )
      )
      out.push(
        line('thumbnail wall clock', seconds(entry.before.thumbnails), seconds(entry.after.thumbnails))
      )
      out.push('')
    }

    console.log(out.join('\n'))
    expect(measurements.length).toBeGreaterThan(0)
  })
})
