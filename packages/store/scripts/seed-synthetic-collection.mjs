// Dev-only baseline harness for the collections pagination work (work unit 0
// of docs/collections-pagination-plan.md). It seeds throwaway SQLite databases
// from the checked-in schema fixture and measures the costs that plan section 2
// only projected, so later "this is faster" claims rest on numbers instead of
// on reading the code.
//
// It never ships: it is not referenced from src/, not exported from the
// package, and writes only into a temp directory.
//
// Run with: pnpm --filter @entropia/store run seed-synthetic-collection
//
// Node v24 ships node:sqlite, so this stays dependency-free like
// export-schema.mjs. The fixture carries no vec0 virtual table and node:sqlite
// is compiled with FTS5, so the whole application schema loads as-is.

import { DatabaseSync } from 'node:sqlite'
import { readFileSync, mkdirSync, rmSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve, join } from 'node:path'
import { tmpdir } from 'node:os'

const here = dirname(fileURLToPath(import.meta.url))
const fixturePath = resolve(here, '../../../apps/desktop/src-tauri/tests/fixtures/schema_full.sql')

const SIZES = [500, 5000, 10000]
const COLLECTION_ID = 'col-baseline'
const CHUNK = 4

// The query under audit, copied verbatim from ItemRepo.findCardSummariesByCollection
// (packages/store/src/repos/item.repo.ts). Kept as a literal on purpose: the
// point of the baseline is to measure what ships today, not a paraphrase.
const CARD_SUMMARY_SQL = `
  SELECT
    i.id,
    i.title,
    i.collection_id,
    i.metadata,
    i.created_at,
    i.updated_at,
    (SELECT COUNT(*)
       FROM assets leaf
       WHERE leaf.item_id = i.id
         AND NOT EXISTS (
           SELECT 1 FROM assets child WHERE child.parent_asset_id = leaf.id
         )
    ) AS asset_count,
    pa.id AS primary_asset_id,
    pa.path AS primary_asset_path,
    pa.type AS primary_asset_type
  FROM items i
  LEFT JOIN assets pa ON pa.id = (
    SELECT p.id
    FROM assets p
    WHERE p.item_id = i.id AND p.parent_asset_id IS NULL
    ORDER BY
      CASE p.type
        WHEN 'image' THEN 0
        WHEN 'pdf' THEN 1
        ELSE 2
      END,
      p.sort_index ASC,
      p.created_at ASC
    LIMIT 1
  )
  WHERE i.collection_id = ?
  ORDER BY i.title COLLATE NOCASE ASC, i.id ASC
`

// Deterministic pseudo-random titles: the sort cost is what is being measured,
// so insertion order must not already match title order.
function makeTitle(index) {
  const words = ['Acta', 'Boletin', 'Cronica', 'Diario', 'Expediente', 'Folio', 'Gaceta', 'Hoja']
  const scrambled = (index * 2654435761) % 4294967296
  return `${words[scrambled % words.length]} ${String(scrambled).padStart(10, '0')}`
}

function createDatabase(file) {
  const db = new DatabaseSync(file)
  db.exec('PRAGMA journal_mode = WAL')
  db.exec('PRAGMA foreign_keys = ON')
  db.exec(readFileSync(fixturePath, 'utf8'))
  return db
}

function seed(db, size) {
  const now = Date.now()
  db.prepare('INSERT INTO collections (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)').run(
    COLLECTION_ID,
    'Baseline',
    now,
    now
  )

  const insertItem = db.prepare(
    'INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)'
  )
  const insertAsset = db.prepare(
    'INSERT INTO assets (id, item_id, path, type, size, created_at, sort_index, parent_asset_id) VALUES (?, ?, ?, ?, ?, ?, ?, NULL)'
  )

  db.exec('BEGIN')
  for (let index = 0; index < size; index += 1) {
    const itemId = `doc-${String(index).padStart(6, '0')}`
    insertItem.run(
      itemId,
      makeTitle(index),
      COLLECTION_ID,
      JSON.stringify({ fondo: 'sintetico', caja: index % 97 }),
      now,
      now
    )
    insertAsset.run(
      `asset-${itemId}`,
      itemId,
      `/synthetic/${itemId}.png`,
      'image',
      1_500_000,
      now,
      0
    )
  }
  db.exec('COMMIT')
  db.exec('ANALYZE')
}

function timed(label, fn) {
  const started = performance.now()
  const value = fn()
  return { label, ms: performance.now() - started, value }
}

// Reproduces CollectionView.loadImageThumbnails' `itemAssetMeta = new Map(itemAssetMeta)`
// per chunk of four. No IPC involved: this isolates the copy itself.
function measureQuadraticMapCopy(rows) {
  let map = new Map()
  let writes = 0
  const started = performance.now()
  for (let index = 0; index < rows.length; index += CHUNK) {
    map = new Map(map)
    writes += map.size
    for (const row of rows.slice(index, index + CHUNK)) {
      map.set(row.id, { thumb: `/thumbs/${row.id}.png` })
    }
  }
  return { ms: performance.now() - started, writes, rounds: Math.ceil(rows.length / CHUNK) }
}

// The Rust -> WebView bridge serializes the whole result set to JSON and the
// WebView parses it back. Measured separately from the query itself.
function measureIpcSerialization(rows) {
  const encode = timed('encode', () => JSON.stringify(rows))
  const decode = timed('decode', () => JSON.parse(encode.value))
  return { ms: encode.ms + decode.ms, bytes: encode.value.length, rows: decode.value.length }
}

// EXPLAIN QUERY PLAN rows are a tree: `parent === 0` is a step of the outer
// query, anything else belongs to a nested subquery. The distinction matters
// here because the primary-asset picker has an ORDER BY of its own, so a flat
// text search for "USE TEMP B-TREE FOR ORDER BY" cannot tell the collection's
// full temp sort apart from a per-row LIMIT 1 pick.
function explainQueryPlan(db) {
  const rows = db.prepare(`EXPLAIN QUERY PLAN ${CARD_SUMMARY_SQL}`).all(COLLECTION_ID)
  const isTempSort = (row) => row.detail.includes('USE TEMP B-TREE FOR ORDER BY')

  return {
    lines: rows.map((row) => `${row.parent === 0 ? '' : '  '}${row.detail}`),
    outerTempSort: rows.some((row) => row.parent === 0 && isTempSort(row)),
    nestedTempSorts: rows.filter((row) => row.parent !== 0 && isTempSort(row)).length,
  }
}

function format(ms) {
  return `${ms.toFixed(1)} ms`
}

const workDir = join(tmpdir(), 'entropia-pagination-baseline')
rmSync(workDir, { recursive: true, force: true })
mkdirSync(workDir, { recursive: true })

console.log('Collections pagination — work unit 0 baseline')
console.log(`schema fixture: ${fixturePath}`)
console.log(`scratch dir:    ${workDir}`)

for (const size of SIZES) {
  const file = join(workDir, `baseline-${size}.sqlite`)
  const db = createDatabase(file)

  const seeding = timed('seed', () => seed(db, size))

  const statement = db.prepare(CARD_SUMMARY_SQL)
  // One warm-up so the reported number is steady-state, not page-cache cold.
  statement.all(COLLECTION_ID)
  const query = timed('query', () => statement.all(COLLECTION_ID))
  const rows = query.value

  const mapCopy = measureQuadraticMapCopy(rows)
  const ipc = measureIpcSerialization(rows)
  const plan = explainQueryPlan(db)

  console.log(`\n=== ${size.toLocaleString('en-US')} documents ===`)
  console.log(`  seed                    ${format(seeding.ms)}`)
  console.log(`  SQL query alone         ${format(query.ms)}  (${rows.length} rows)`)
  console.log(
    `  quadratic Map copy      ${format(mapCopy.ms)}  (${mapCopy.writes.toLocaleString('en-US')} writes over ${mapCopy.rounds} rounds)`
  )
  console.log(
    `  IPC serialization       ${format(ipc.ms)}  (${(ipc.bytes / 1_048_576).toFixed(2)} MiB of JSON)`
  )
  console.log(`  thumbnail IPC calls     ${rows.length} invocations in ${mapCopy.rounds} sequential rounds of ${CHUNK}`)
  console.log(`  DOM nodes if unpaginated ~${(rows.length * 9).toLocaleString('en-US')} (9 elements per ItemCard)`)
  console.log('  EXPLAIN QUERY PLAN (indented rows belong to a subquery):')
  for (const line of plan.lines) console.log(`    ${line}`)
  console.log(
    `  outer ORDER BY temp sort: ${plan.outerTempSort ? 'YES — the whole collection is sorted in memory' : 'no'}`
  )
  console.log(
    `  nested temp sorts:        ${plan.nestedTempSorts} (primary-asset picker; not the collection ordering)`
  )

  db.close()
}

console.log(
  '\nNot measurable headlessly: per-thumbnail Rust decode time and real paint cost.\n' +
    'Both need the Tauri app running against a seeded collection; the invocation\n' +
    'and node counts above are the headless half of those two rows.'
)
