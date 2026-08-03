import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { buildSchemaFixture } from './runner'
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
})
