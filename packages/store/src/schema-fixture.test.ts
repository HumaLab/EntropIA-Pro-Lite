import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { buildSchemaFixture } from './runner'
import { getTableConfig } from 'drizzle-orm/sqlite-core'
import * as schema from './schema'

const here = dirname(fileURLToPath(import.meta.url))
const fixturePath = resolve(here, '../../../apps/desktop/src-tauri/tests/fixtures/schema_full.sql')

describe('schema fixture export', () => {
  it('the checked-in Rust fixture is up to date with the migration registry', () => {
    const expected = buildSchemaFixture()
    let actual: string
    try {
      actual = readFileSync(fixturePath, 'utf8')
    } catch {
      throw new Error(
        `Missing schema fixture at ${fixturePath}. Run: pnpm --filter @entropia/store export-schema`
      )
    }

    // Normalize CRLF so the check is stable across platforms / git autocrlf.
    const norm = (s: string) => s.replace(/\r\n/g, '\n')
    expect(norm(actual), 'Stale fixture — run: pnpm --filter @entropia/store export-schema').toBe(
      norm(expected)
    )
  })

  it('keeps the checked-in schema fixture aligned with the new collection/title index', () => {
    const sql = buildSchemaFixture()

    expect(sql).toContain('CREATE INDEX IF NOT EXISTS idx_items_collection_title')
    expect(sql).toContain('ON items (collection_id, title COLLATE NOCASE, id)')
  })

  it('declares the collection/title index on the drizzle items table too', () => {
    // The fixture is generated from the migration registry, so a registry-only
    // index would silently drift from the schema module the repos type against.
    const config = getTableConfig(schema.items)

    expect(config.indexes.map((index) => index.config.name)).toContain('idx_items_collection_title')
  })

  it('builds a fixture that contains every synced base table', () => {
    const sql = buildSchemaFixture()
    for (const table of [
      'collections',
      'items',
      'assets',
      'notes',
      'annotations',
      'extractions',
      'transcriptions',
      'layouts',
      'entities',
      'triples',
      'topics',
      'item_topics',
      'llm_results',
      'rag_conversations',
      'rag_messages',
    ]) {
      expect(sql).toContain(table)
    }
  })

  it('replaces the 0020_layouts no-op marker with the real layouts DDL', () => {
    const sql = buildSchemaFixture()
    expect(sql).toContain('CREATE TABLE IF NOT EXISTS layouts')
    expect(sql).toContain('idx_layouts_asset_id_unique')
    // The no-op marker table must not leak into the fixture.
    expect(sql).not.toContain('__entropia_migration_0020_noop')
  })

  it('includes version metadata for vec_assets with legacy-safe defaults', () => {
    const sql = buildSchemaFixture()
    expect(sql).toContain("embedding_model TEXT NOT NULL DEFAULT 'legacy'")
    expect(sql).toContain("embedding_contract TEXT NOT NULL DEFAULT 'legacy'")
    expect(sql).toContain('dimensions INTEGER NOT NULL DEFAULT 0')
  })

  it('exports canonical rag_chunks and migrates its complete provenance shape', () => {
    expect(schema).toHaveProperty('ragChunks')

    const sql = buildSchemaFixture()
    expect(sql).toContain('CREATE TABLE rag_chunks')
    for (const column of [
      'id TEXT PRIMARY KEY',
      'asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE',
      'item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE',
      'source_kind TEXT NOT NULL',
      'source_id TEXT NOT NULL',
      'chunk_ordinal INTEGER NOT NULL',
      'text_content TEXT NOT NULL',
      'start_char INTEGER NOT NULL',
      'end_char INTEGER NOT NULL',
      'source_text_hash TEXT NOT NULL',
      'chunking_contract TEXT NOT NULL',
      'embedding BLOB NOT NULL',
      'embedding_model TEXT NOT NULL',
      'embedding_contract TEXT NOT NULL',
      'dimensions INTEGER NOT NULL',
    ]) {
      expect(sql).toContain(column)
    }
    expect(sql).toContain('UNIQUE(asset_id, source_kind, source_id, chunk_ordinal)')
    expect(sql).toContain('idx_rag_chunks_asset_id')
    expect(sql).toContain('idx_rag_chunks_item_id')
  })

  it('exports the bibliography catalog foundation with qualified Zotero identity and snapshots', () => {
    for (const table of ['zoteroConnections', 'zoteroLibraries', 'bibliographicItems']) {
      expect(schema).toHaveProperty(table)
    }

    const sql = buildSchemaFixture()
    const normalized = sql.replace(/\s+/g, ' ')
    for (const column of [
      "source_origin TEXT NOT NULL CHECK(source_origin IN ('local', 'web'))",
      'source_instance_id TEXT',
      'connection_id TEXT NOT NULL REFERENCES zotero_connections(id) ON DELETE CASCADE',
      'library_id TEXT NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE',
      'item_key TEXT NOT NULL',
      'item_version INTEGER',
      'native_json_snapshot TEXT NOT NULL',
      'csl_json_snapshot TEXT NOT NULL',
      'created_at INTEGER NOT NULL',
      'updated_at INTEGER NOT NULL',
    ]) {
      expect(normalized).toContain(column)
    }
    expect(normalized).toContain('idx_bibliographic_items_library_key')
    expect(normalized).toContain('ON bibliographic_items(library_id, item_key)')
  })

  it('declares the bibliography catalog relationships in the drizzle schema', () => {
    const connection = getTableConfig(schema.zoteroConnections)
    const library = getTableConfig(schema.zoteroLibraries)
    const item = getTableConfig(schema.bibliographicItems)

    expect(connection.name).toBe('zotero_connections')
    expect(library.name).toBe('zotero_libraries')
    expect(item.name).toBe('bibliographic_items')
    expect(library.indexes.map((index) => index.config.name)).toContain(
      'idx_zotero_libraries_identity'
    )
    expect(item.indexes.map((index) => index.config.name)).toContain(
      'idx_bibliographic_items_library_key'
    )
  })

  it('declares the E1b-1b relational catalog and tombstone schema', () => {
    for (const table of [
      'zoteroCollections',
      'zoteroTags',
      'zoteroAttachments',
      'zoteroItemCollections',
      'zoteroItemTags',
      'zoteroItemTombstones',
      'zoteroCollectionTombstones',
      'zoteroTagTombstones',
      'zoteroAttachmentTombstones',
    ]) {
      expect(schema).toHaveProperty(table)
    }

    const sql = buildSchemaFixture().replace(/\s+/g, ' ')
    for (const column of [
      'collection_key TEXT NOT NULL',
      'parent_collection_key TEXT',
      'native_json_snapshot TEXT NOT NULL',
      'native_version INTEGER',
      'tag_text TEXT NOT NULL',
      'tag_type TEXT',
      'attachment_key TEXT NOT NULL',
      'item_id TEXT NOT NULL REFERENCES bibliographic_items(id) ON DELETE CASCADE',
      'content_type TEXT',
      'link_mode TEXT',
      'filename TEXT',
      'native_path TEXT',
      'url TEXT',
      'md5 TEXT',
      'mtime INTEGER',
      'reason TEXT NOT NULL',
    ]) {
      expect(sql).toContain(column)
    }
    expect(sql).toContain('FOREIGN KEY (item_id, library_id)')
    expect(sql).toContain('FOREIGN KEY (collection_id, library_id)')
    expect(sql).toContain('FOREIGN KEY (tag_id, library_id)')
    expect(sql).toContain('CHECK(length(trim(reason)) > 0)')
  })

  it('keeps composite identity and membership indexes in the drizzle schema', () => {
    const collection = getTableConfig(schema.zoteroCollections)
    const tag = getTableConfig(schema.zoteroTags)
    const attachment = getTableConfig(schema.zoteroAttachments)
    const itemCollections = getTableConfig(schema.zoteroItemCollections)
    const itemTags = getTableConfig(schema.zoteroItemTags)

    expect(collection.indexes.map((index) => index.config.name)).toContain(
      'idx_zotero_collections_library_key'
    )
    expect(tag.indexes.map((index) => index.config.name)).toContain('idx_zotero_tags_library_text')
    expect(attachment.indexes.map((index) => index.config.name)).toContain(
      'idx_zotero_attachments_item_key'
    )
    expect(itemCollections.primaryKeys[0]?.columns.map((column) => column.name)).toEqual(
      expect.arrayContaining(['library_id', 'item_id', 'collection_id'])
    )
    expect(itemTags.primaryKeys[0]?.columns.map((column) => column.name)).toEqual(
      expect.arrayContaining(['library_id', 'item_id', 'tag_id'])
    )
  })

  it('declares durable reconciliation state and normalized seen-set tables', () => {
    for (const table of ['zoteroReconciliationRuns', 'zoteroReconciliationSeen']) {
      expect(schema).toHaveProperty(table)
    }

    const sql = buildSchemaFixture().replace(/\s+/g, ' ')
    for (const column of [
      'library_id TEXT PRIMARY KEY NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE',
      'run_id TEXT NOT NULL',
      'connection_revision INTEGER NOT NULL',
      'cursor_start INTEGER NOT NULL DEFAULT 0',
      'cursor_limit INTEGER NOT NULL',
      'remote_total INTEGER',
      'target_version INTEGER',
      'checkpoint_version INTEGER',
      'retry_count INTEGER NOT NULL DEFAULT 0',
      'attempt_count INTEGER NOT NULL DEFAULT 0',
      'latest_error_message TEXT',
      'latest_error_retryable INTEGER',
      'revision INTEGER NOT NULL DEFAULT 0',
      'checkpointed_at INTEGER',
      'completed_at INTEGER',
      'run_id TEXT NOT NULL',
      'entity_kind TEXT NOT NULL',
      'entity_key TEXT NOT NULL CHECK(length(trim(entity_key)) > 0)',
      "parent_key TEXT NOT NULL DEFAULT ''",
      'remote_version INTEGER',
      'observed_at INTEGER NOT NULL',
    ]) {
      expect(sql).toContain(column)
    }
    expect(sql).toContain(
      "CHECK(state IN ('running', 'retry_wait', 'interrupted', 'blocked', 'failed', 'completed'))"
    )
    expect(sql).toContain("CHECK(phase IN ('versions', 'catalog', 'finalize'))")
    expect(sql).toContain("CHECK(entity_kind IN ('item', 'collection', 'tag', 'attachment'))")
    expect(sql).toContain('ON DELETE CASCADE')
  })

  it('keeps reconciliation composite identity and foreign-key alignment in drizzle', () => {
    const runs = getTableConfig(schema.zoteroReconciliationRuns)
    const seen = getTableConfig(schema.zoteroReconciliationSeen)

    expect(runs.name).toBe('zotero_reconciliation_runs')
    expect(seen.name).toBe('zotero_reconciliation_seen')
    expect(runs.indexes.map((index) => index.config.name)).toContain(
      'idx_zotero_reconciliation_runs_library_run'
    )
    expect(runs.indexes.map((index) => index.config.name)).toContain(
      'idx_zotero_reconciliation_runs_run_id_unique'
    )
    expect(seen.primaryKeys[0]?.columns.map((column) => column.name)).toEqual(
      expect.arrayContaining(['library_id', 'run_id', 'entity_kind', 'entity_key', 'parent_key'])
    )
    expect(
      seen.foreignKeys.some(
        (foreignKey) => foreignKey.getName() === 'zotero_reconciliation_seen_run_fkey'
      )
    ).toBe(true)
  })

  it('exports the E2a-2 subject cutover: the composite is the sole single-flight authority', () => {
    const sql = buildSchemaFixture()

    for (const fragment of [
      "domain TEXT NOT NULL DEFAULT 'corpus'",
      "subject_kind TEXT NOT NULL DEFAULT 'asset'",
      "subject_id TEXT NOT NULL DEFAULT ''",
      'idx_processing_tasks_subject_active_unique',
      'ON processing_tasks(domain, subject_kind, subject_id, kind)',
    ]) {
      expect(sql, `fixture is missing: ${fragment}`).toContain(fragment)
    }
    // The 0032 section still carries the historical CREATE (migrations are
    // never rewritten); E2a-2 drops it, so the 0042 section must carry the
    // DROP and no later section may recreate it.
    expect(sql).toContain('DROP INDEX IF EXISTS idx_processing_tasks_active_unique')
    const cutoverAt = sql.indexOf('-- 0042_processing_task_subject_cutover')
    expect(cutoverAt).toBeGreaterThanOrEqual(0)
    expect(sql.slice(cutoverAt)).not.toContain(
      'CREATE UNIQUE INDEX idx_processing_tasks_active_unique'
    )
  })

  it('declares the subject-identity columns on the drizzle processing tables', () => {
    const tasks = getTableConfig(schema.processingTasks)
    const batchTasks = getTableConfig(schema.processingBatchTasks)

    for (const config of [tasks, batchTasks]) {
      expect(config.columns.map((column) => column.name)).toEqual(
        expect.arrayContaining(['domain', 'subject_kind', 'subject_id'])
      )
    }
    expect(tasks.name).toBe('processing_tasks')
    expect(batchTasks.name).toBe('processing_batch_tasks')
  })

  it('exports E2c per-batch priority in SQL and drizzle', () => {
    const sql = buildSchemaFixture()
    expect(sql).toContain('priority INTEGER NOT NULL DEFAULT 0')
    expect(sql).toContain('CHECK(priority IN (0, 1, 2))')
    expect(sql).toContain('idx_processing_batches_priority')
    expect(sql).toContain('ON processing_batches(priority, created_at, id)')
    const batches = getTableConfig(schema.processingBatches)
    expect(batches.columns.map((column) => column.name)).toContain('priority')
  })
})
