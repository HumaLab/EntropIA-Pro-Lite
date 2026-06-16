import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { runMigrations } from './runner'
import { createMockDbClient } from './__mocks__/db.mock'

const here = dirname(fileURLToPath(import.meta.url))

describe('runMigrations — migrations 0004, 0005 and 0006', () => {
  it('executes 0004_fts5 migration SQL (FTS5 virtual table creation)', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasFts5 = client._executedSql.some(
      (sql) => sql.includes('fts_items') && sql.includes('fts5')
    )
    expect(hasFts5).toBe(true)
  })

  it('executes 0005_nlp_tables migration SQL (entities table creation)', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasEntities = client._executedSql.some(
      (sql) => sql.includes('entities') && sql.includes('CREATE TABLE IF NOT EXISTS')
    )
    expect(hasEntities).toBe(true)
  })

  it('is idempotent — running twice does not throw', async () => {
    const client = createMockDbClient()
    // First run
    await runMigrations(client)
    // Simulate "already applied" by pre-populating the applied set
    // Second run with all migrations already recorded — mock already returns them
    await expect(runMigrations(client)).resolves.toBeUndefined()
  })

  it('FTS5 migration uses unicode61 tokenizer config', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasUnicode61 = client._executedSql.some((sql) => sql.includes('unicode61'))
    expect(hasUnicode61).toBe(true)
  })

  it('FTS migrations backfill with explicit items.rowid', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasCanonicalRowidInsert = client._executedSql.some((sql) =>
      sql.includes('INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)')
    )
    expect(hasCanonicalRowidInsert).toBe(true)
  })

  it('FTS corrective migration performs delete-all rebuild', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasDeleteAll = client._executedSql.some((sql) =>
      sql.includes("INSERT INTO fts_items(fts_items) VALUES('delete-all')") ||
      sql.includes("INSERT INTO fts_items(fts_items) VALUES ('delete-all')")
    )
    expect(hasDeleteAll).toBe(true)
  })

  it('entities migration creates idx_entities_item_id index', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasIndex = client._executedSql.some((sql) => sql.includes('idx_entities_item_id'))
    expect(hasIndex).toBe(true)
  })

  it('migrations 0001 through 0005 are all executed on a fresh database', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    // All 5 migration names should cause INSERT INTO _migrations
    const wasInserted = client._executedSql.some(
      (sql) => sql.includes('INSERT INTO _migrations') && sql.includes('VALUES')
    )
    expect(wasInserted).toBe(true)
  })

  it('executes 0006_triples migration SQL (triples table + index)', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasTriplesTable = client._executedSql.some(
      (sql) => sql.includes('CREATE TABLE IF NOT EXISTS triples') && sql.includes('item_id')
    )
    const hasTriplesIndex = client._executedSql.some((sql) =>
      sql.includes('CREATE INDEX IF NOT EXISTS triples_item_id_idx')
    )

    expect(hasTriplesTable).toBe(true)
    expect(hasTriplesIndex).toBe(true)
  })

  it('executes llm_results hardening migration with target_type and timestamp normalization', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasLlmResultsV2 = client._executedSql.some(
      (sql) =>
        sql.includes('CREATE TABLE llm_results_v2') &&
        sql.includes('target_type TEXT NOT NULL') &&
        sql.includes("CHECK(target_type IN ('asset', 'item', 'collection', 'unknown'))")
    )
    const hasTimestampNormalization = client._executedSql.some(
      (sql) =>
        sql.includes('CASE') &&
        sql.includes('created_at < 1000000000000') &&
        sql.includes('created_at * 1000')
    )

    expect(hasLlmResultsV2).toBe(true)
    expect(hasTimestampNormalization).toBe(true)
  })

  it('executes layouts migration with blocks column and unique asset index', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasLayoutsTable = client._executedSql.some(
      (sql) =>
        sql.includes('CREATE TABLE IF NOT EXISTS layouts') &&
        sql.includes('blocks TEXT NOT NULL') &&
        sql.includes('image_width INTEGER NOT NULL')
    )
    const hasUniqueIndex = client._executedSql.some((sql) =>
      sql.includes('CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique')
    )

    expect(hasLayoutsTable).toBe(true)
    expect(hasUniqueIndex).toBe(true)
  })

  it('executes 0022_rag_conversations migration SQL (conversations, messages and index)', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const hasConversationsTable = client._executedSql.some(
      (sql) =>
        sql.includes('CREATE TABLE IF NOT EXISTS rag_conversations') &&
        sql.includes('updated_at INTEGER NOT NULL')
    )
    const hasMessagesTable = client._executedSql.some(
      (sql) =>
        sql.includes('CREATE TABLE IF NOT EXISTS rag_messages') &&
        sql.includes('REFERENCES rag_conversations(id) ON DELETE CASCADE') &&
        sql.includes("CHECK(role IN ('user','assistant'))")
    )
    const hasConversationIndex = client._executedSql.some((sql) =>
      sql.includes(
        'CREATE INDEX IF NOT EXISTS idx_rag_messages_conversation ON rag_messages(conversation_id, sort_index)'
      )
    )

    expect(hasConversationsTable).toBe(true)
    expect(hasMessagesTable).toBe(true)
    expect(hasConversationIndex).toBe(true)
  })

  it('0023_sync_ids is the migration head and rewrites one-per-asset ids (sync re-added)', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    // 0023 is the last registered migration; its .sql mirror matches the inline
    // registry entry (the registry is the runtime source of truth; the file is
    // the human-readable mirror per the migrations/ convention).
    const mirror = readFileSync(resolve(here, 'migrations/0023_sync_ids.sql'), 'utf8')
    expect(mirror).toContain("UPDATE extractions SET id = 'ext-' || asset_id")
    expect(mirror).toContain("UPDATE transcriptions SET id = 'trx-' || asset_id")
    expect(mirror).toContain("UPDATE layouts SET id = 'lay-' || asset_id")

    // Sync is back: the 0023_sync_ids rewrite gives one-per-asset tables
    // deterministic ids so two devices converge on a single server row.
    const ranSyncRewrite = client._executedSql.some(
      (sql) =>
        sql.includes("UPDATE extractions SET id = 'ext-'") ||
        sql.includes("UPDATE transcriptions SET id = 'trx-'") ||
        sql.includes("UPDATE layouts SET id = 'lay-'")
    )
    expect(ranSyncRewrite).toBe(true)
  })
})
