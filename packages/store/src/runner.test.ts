import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { COLLECTION_ACTIVITY_DDL, runMigrations } from './runner'
import { createMockDbClient } from './__mocks__/db.mock'
import { DatabaseSync, type SQLInputValue } from 'node:sqlite'
import type { DbClient } from './types'

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

    const hasDeleteAll = client._executedSql.some(
      (sql) =>
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

  it('adds independent manual coordinates to geocoded entities', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    expect(client._executedSql.some((sql) => sql.includes('ADD COLUMN manual_lat REAL'))).toBe(true)
    expect(client._executedSql.some((sql) => sql.includes('ADD COLUMN manual_lon REAL'))).toBe(true)
  })

  it('marks existing vec_assets rows as legacy while adding embedding contract metadata', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(
      "ALTER TABLE vec_assets ADD COLUMN embedding_model TEXT NOT NULL DEFAULT 'legacy'"
    )
    expect(migrationSql).toContain(
      "ALTER TABLE vec_assets ADD COLUMN embedding_contract TEXT NOT NULL DEFAULT 'legacy'"
    )
    expect(migrationSql).toContain(
      'ALTER TABLE vec_assets ADD COLUMN dimensions INTEGER NOT NULL DEFAULT 0'
    )

    const mirror = readFileSync(
      resolve(here, 'migrations/0028_vec_assets_embedding_contract.sql'),
      'utf8'
    )
    expect(mirror).not.toMatch(/DELETE\s+FROM\s+vec_assets/i)
  })

  it('installs triggers that propagate asset activity to the parent collection', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_assets_insert')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_assets_update')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_assets_delete')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_extractions_insert')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_transcriptions_insert')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_layouts_insert')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_annotations_insert')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_entities_insert')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_triples_insert')
    expect(migrationSql).toContain('CREATE TRIGGER collection_activity_llm_results_insert')
    expect(migrationSql).toContain('UPDATE collections')

    const mirror = readFileSync(resolve(here, 'migrations/0027_collection_activity.sql'), 'utf8')
    expect(mirror).toBe(
      `-- Generated from COLLECTION_ACTIVITY_DDL in ../runner.ts.\n${COLLECTION_ACTIVITY_DDL}\n`
    )
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

  it('executes 0023_sync_ids and rewrites one-per-asset ids (sync re-added)', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    // Its .sql mirror matches the inline registry entry (the registry is the
    // runtime source of truth; the file is the human-readable mirror per the
    // migrations/ convention).
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

describe('fts contentless delete', () => {
  it('rebuilds the index so replaced text stops matching, keeping the rows it had', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
      async execute(sql, params = []) {
        return { rowsAffected: Number(db.prepare(sql).run(...params as SQLInputValue[]).changes) }
      },
      async executeBatch(sql) { db.exec(sql) },
      async select<T>(sql: string, params: unknown[] = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]) as T[]
      },
      async selectRows(sql, params = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]).map(Object.values)
      },
    }
    try {
      await runMigrations(client)

      const ddl = db.prepare("SELECT sql FROM sqlite_master WHERE name='fts_items'").get() as { sql: string }
      expect(ddl.sql).toContain('contentless_delete=1')

      // Seed one indexed item the way the app does, then correct its text.
      db.exec(`INSERT INTO collections(id, name, created_at, updated_at) VALUES('c1','legajo',1,1);
        INSERT INTO items(id, title, collection_id, created_at, updated_at) VALUES('i1','Acta','c1',1,1);`)
      const rowid = (db.prepare("SELECT rowid AS r FROM items WHERE id='i1'").get() as { r: number }).r
      const index = (text: string) =>
        db.prepare('INSERT OR REPLACE INTO fts_items(rowid, item_id, title, metadata, extracted_text) VALUES (?,?,?,?,?)')
          .run(rowid, 'i1', 'Acta', '', text)
      index('zanahoria del sindicato')
      index('berenjena del sindicato')

      const hits = (term: string) =>
        Number((db.prepare('SELECT COUNT(*) AS n FROM fts_items WHERE fts_items MATCH ?').get(term) as { n: number }).n)
      expect(hits('berenjena')).toBe(1)
      expect(hits('zanahoria')).toBe(0)
      expect(hits('sindicato')).toBe(1)
    } finally {
      db.close()
    }
  })
})

describe('durable queue migration', () => {
  it('rolls back schema when recording the migration fails and can retry', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
      async execute(sql, params = []) {
        return { rowsAffected: Number(db.prepare(sql).run(...params as SQLInputValue[]).changes) }
      },
      async executeBatch(sql) { db.exec(sql) },
      async select<T>(sql: string, params: unknown[] = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]) as T[]
      },
      async selectRows(sql, params = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]).map(Object.values)
      },
    }
    try {
      db.exec(`CREATE TABLE _migrations(id INTEGER PRIMARY KEY, name TEXT UNIQUE, applied_at INTEGER);
        CREATE TRIGGER interrupt_queue_migration BEFORE INSERT ON _migrations
        WHEN NEW.name='0032_batch_processing' BEGIN SELECT RAISE(ABORT,'simulated storage failure'); END;`)
      await expect(runMigrations(client)).rejects.toThrow('simulated storage failure')
      expect(db.prepare("SELECT name FROM sqlite_master WHERE name='processing_batches'").get()).toBeUndefined()
      expect(db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()).toBeUndefined()
      db.exec('DROP TRIGGER interrupt_queue_migration')
      await runMigrations(client)
      expect(db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()?.name).toBe('0032_batch_processing')
      await runMigrations(client)
      db.prepare("INSERT INTO processing_batches(id,request_id,origin,state,desired_state,operations,created_at,updated_at) VALUES('b','r','user','preparing','pause','[]',0,0)").run()
      expect(db.prepare("SELECT state FROM processing_batches WHERE id='b'").get()?.state).toBe('preparing')
    } finally {
      db.close()
    }
  })

  it('repairs a half-applied 0032 (tables without registry row) and still applies 0033', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
      async execute(sql, params = []) {
        return { rowsAffected: Number(db.prepare(sql).run(...params as SQLInputValue[]).changes) }
      },
      async executeBatch(sql) { db.exec(sql) },
      async select<T>(sql: string, params: unknown[] = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]) as T[]
      },
      async selectRows(sql, params = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]).map(Object.values)
      },
    }
    try {
      // Start from a real fully-migrated database, then burn it into the
      // exact shape the earlier non-atomic build left behind: complete 0032
      // tables, no registry rows, no invalidation counter column.
      await runMigrations(client)
      db.exec(`DELETE FROM _migrations WHERE name IN ('0032_batch_processing','0033_processing_source_invalidation')`)
      const hadColumn = (db.prepare("SELECT name FROM pragma_table_info('processing_tasks') WHERE name='source_invalidation_count'").get() as { name: string } | undefined) !== undefined
      if (hadColumn) db.exec('ALTER TABLE processing_tasks DROP COLUMN source_invalidation_count')
      await expect(runMigrations(client)).resolves.toBeUndefined()
      expect(db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()?.name).toBe('0032_batch_processing')
      expect(db.prepare("SELECT name FROM _migrations WHERE name='0033_processing_source_invalidation'").get()?.name).toBe('0033_processing_source_invalidation')
      const columns = db.prepare("SELECT name FROM pragma_table_info('processing_tasks')").all() as Array<{ name: string }>
      expect(columns.map((row: { name: string }) => row.name)).toContain('source_invalidation_count')
    } finally {
      db.close()
    }
  })

  it('repairs a PARTIAL 0032 when every surviving table is empty', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
      async execute(sql, params = []) {
        return { rowsAffected: Number(db.prepare(sql).run(...params as SQLInputValue[]).changes) }
      },
      async executeBatch(sql) { db.exec(sql) },
      async select<T>(sql: string, params: unknown[] = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]) as T[]
      },
      async selectRows(sql, params = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]).map(Object.values)
      },
    }
    try {
      // The shape the FIRST non-atomic build left behind: it split the
      // migration on ';', which shreds the CREATE TRIGGER … BEGIN … END;
      // bodies. Every statement before the first trigger had already
      // autocommitted, so the tables survive without processing_meta, without
      // the triggers, and without a registry row.
      await runMigrations(client)
      db.exec(`DELETE FROM _migrations WHERE name IN ('0032_batch_processing','0033_processing_source_invalidation')`)
      db.exec('DROP TABLE processing_meta')
      for (const trigger of [
        'trg_processing_extractions_ai', 'trg_processing_extractions_au', 'trg_processing_extractions_ad',
        'trg_processing_transcriptions_ai', 'trg_processing_transcriptions_au', 'trg_processing_transcriptions_ad',
      ]) db.exec(`DROP TRIGGER ${trigger}`)
      const hadColumn = (db.prepare("SELECT name FROM pragma_table_info('processing_tasks') WHERE name='source_invalidation_count'").get() as { name: string } | undefined) !== undefined
      if (hadColumn) db.exec('ALTER TABLE processing_tasks DROP COLUMN source_invalidation_count')

      await expect(runMigrations(client)).resolves.toBeUndefined()

      expect(db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()?.name).toBe('0032_batch_processing')
      expect(db.prepare("SELECT name FROM _migrations WHERE name='0033_processing_source_invalidation'").get()?.name).toBe('0033_processing_source_invalidation')
      expect(db.prepare("SELECT name FROM sqlite_master WHERE name='processing_meta'").get()?.name).toBe('processing_meta')
      expect(db.prepare("SELECT name FROM sqlite_master WHERE name='trg_processing_extractions_ai'").get()?.name).toBe('trg_processing_extractions_ai')
      const columns = db.prepare("SELECT name FROM pragma_table_info('processing_tasks')").all() as Array<{ name: string }>
      expect(columns.map((row: { name: string }) => row.name)).toContain('source_invalidation_count')
    } finally {
      db.close()
    }
  })

  it('refuses to drop a partial 0032 whose tables still hold queue rows', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
      async execute(sql, params = []) {
        return { rowsAffected: Number(db.prepare(sql).run(...params as SQLInputValue[]).changes) }
      },
      async executeBatch(sql) { db.exec(sql) },
      async select<T>(sql: string, params: unknown[] = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]) as T[]
      },
      async selectRows(sql, params = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]).map(Object.values)
      },
    }
    try {
      await runMigrations(client)
      db.exec(`DELETE FROM _migrations WHERE name IN ('0032_batch_processing','0033_processing_source_invalidation')`)
      db.exec('DROP TABLE processing_meta')
      db.prepare("INSERT INTO processing_batches(id,request_id,origin,state,desired_state,operations,created_at,updated_at) VALUES('b','r','user','preparing','pause','[]',0,0)").run()

      // Dropping now would destroy queue state the user could still resume.
      await expect(runMigrations(client)).rejects.toThrow('incomplete 0032 state')
      expect(db.prepare("SELECT id FROM processing_batches WHERE id='b'").get()?.id).toBe('b')
    } finally {
      db.close()
    }
  })

  it('a fresh install records 0033 and adds the invalidation counter column', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
      async execute(sql, params = []) {
        return { rowsAffected: Number(db.prepare(sql).run(...params as SQLInputValue[]).changes) }
      },
      async executeBatch(sql) { db.exec(sql) },
      async select<T>(sql: string, params: unknown[] = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]) as T[]
      },
      async selectRows(sql, params = []) {
        return db.prepare(sql).all(...params as SQLInputValue[]).map(Object.values)
      },
    }
    try {
      await runMigrations(client)
      expect(db.prepare("SELECT name FROM _migrations WHERE name='0033_processing_source_invalidation'").get()?.name).toBe('0033_processing_source_invalidation')
      const columns = db.prepare("SELECT name FROM pragma_table_info('processing_tasks')").all() as Array<{ name: string }>
      expect(columns.map((row: { name: string }) => row.name)).toContain('source_invalidation_count')
      // The historical 0032 attempts CHECK stays untouched; source changes
      // close attempts as interrupted with a source_changed error code.
      const attempts = db.prepare("SELECT sql FROM sqlite_master WHERE name='processing_attempts'").get() as { sql: string }
      expect(attempts.sql).not.toContain("'source_changed'")
    } finally {
      db.close()
    }
  })
})

describe('writing workspace migration (0035)', () => {
  const shim = (db: DatabaseSync): DbClient => ({
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
  })

  const seedCorpus = (db: DatabaseSync) => {
    db.exec(`
      INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'Coleccion', 1, 1);
      INSERT INTO items (id, collection_id, title, created_at, updated_at) VALUES ('i1', 'c1', 'Item', 1, 1);
      INSERT INTO assets (id, item_id, path, type, sort_index, created_at) VALUES ('a1', 'i1', 'p.pdf', 'pdf', 0, 1);
      INSERT INTO notes (id, item_id, content, created_at, updated_at) VALUES ('n1', 'i1', 'nota', 1, 1);
    `)
  }

  const insertDocument = (db: DatabaseSync, id = 'd1') => {
    db.prepare(
      `INSERT INTO writing_documents
         (id, title, document_type, status, schema_version, current_content_json, revision, created_at, updated_at)
       VALUES (?, 'Articulo', 'article', 'active', 1, '{"type":"doc"}', 0, 1, 1)`
    ).run(id)
  }

  it('adds the writing tables without disturbing existing corpus data', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      seedCorpus(db)

      // Re-running is idempotent and must not touch user rows.
      await runMigrations(shim(db))

      expect(db.prepare("SELECT title FROM items WHERE id='i1'").get()?.title).toBe('Item')
      expect(db.prepare("SELECT content FROM notes WHERE id='n1'").get()?.content).toBe('nota')

      for (const table of [
        'writing_documents',
        'writing_document_collections',
        'writing_document_versions',
        'writing_document_citations',
        'writing_zotero_citations',
        'writing_provenance_events',
        'writing_agent_suggestions',
      ]) {
        expect(
          db.prepare('SELECT name FROM sqlite_master WHERE type=? AND name=?').get('table', table),
          `missing table: ${table}`
        ).toBeDefined()
      }
    } finally {
      db.close()
    }
  })

  it('rolls the whole migration back when recording it fails, and can retry', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      db.exec(`CREATE TABLE _migrations(id INTEGER PRIMARY KEY, name TEXT UNIQUE, applied_at INTEGER);
        CREATE TRIGGER interrupt_writing BEFORE INSERT ON _migrations
        WHEN NEW.name='0035_writing_workspace' BEGIN SELECT RAISE(ABORT,'simulated storage failure'); END;`)

      await expect(runMigrations(shim(db))).rejects.toThrow('simulated storage failure')
      expect(
        db.prepare("SELECT name FROM sqlite_master WHERE name='writing_documents'").get()
      ).toBeUndefined()

      db.exec('DROP TRIGGER interrupt_writing')
      await runMigrations(shim(db))
      expect(
        db.prepare("SELECT name FROM _migrations WHERE name='0035_writing_workspace'").get()?.name
      ).toBe('0035_writing_workspace')
    } finally {
      db.close()
    }
  })

  it('defaults revision to 0 and rejects an unknown status', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      insertDocument(db)

      expect(db.prepare("SELECT revision FROM writing_documents WHERE id='d1'").get()?.revision).toBe(
        0
      )
      expect(() =>
        db
          .prepare(
            `INSERT INTO writing_documents
               (id, title, document_type, status, schema_version, current_content_json, created_at, updated_at)
             VALUES ('d2','t','article','inventado',1,'{}',1,1)`
          )
          .run()
      ).toThrow()
    } finally {
      db.close()
    }
  })

  it('cascades every projection when a document is deleted', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      seedCorpus(db)
      insertDocument(db)

      db.exec(`
        INSERT INTO writing_document_collections (document_id, collection_id, is_primary, created_at) VALUES ('d1','c1',1,1);
        INSERT INTO writing_document_versions (id, document_id, version_number, content_json, schema_version, document_settings_json, reason, created_at, content_hash)
          VALUES ('v1','d1',1,'{}',1,'{}','checkpoint',1,'h');
        INSERT INTO writing_document_citations (id, document_id, citation_node_id, asset_id, integrity_status, created_at, updated_at)
          VALUES ('dc1','d1','node-1','a1','valid',1,1);
        INSERT INTO writing_zotero_citations (id, document_id, citation_node_id, citation_cluster_id, item_position, source_origin, library_type, library_id, item_key, integrity_status, created_at, updated_at)
          VALUES ('zc1','d1','node-2','cl-1',0,'local','user','0','KEY1','valid',1,1);
        INSERT INTO writing_provenance_events (id, document_id, origin_type, operation_type, created_at)
          VALUES ('pe1','d1','corpus','insert',1);
        INSERT INTO writing_agent_suggestions (id, document_id, source_revision, selected_content_hash, action_type, status, created_at)
          VALUES ('as1','d1',0,'h','rewrite','pending',1);
      `)

      db.exec("DELETE FROM writing_documents WHERE id='d1'")

      for (const table of [
        'writing_document_collections',
        'writing_document_versions',
        'writing_document_citations',
        'writing_zotero_citations',
        'writing_provenance_events',
        'writing_agent_suggestions',
      ]) {
        expect(
          db.prepare(`SELECT COUNT(*) AS n FROM ${table}`).get()?.n,
          `${table} kept orphan rows`
        ).toBe(0)
      }
    } finally {
      db.close()
    }
  })

  it('lets a collection be deleted without blocking, dropping only the association', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      seedCorpus(db)
      insertDocument(db)
      db.exec(
        "INSERT INTO writing_document_collections (document_id, collection_id, is_primary, created_at) VALUES ('d1','c1',1,1)"
      )

      // The corpus cascade is hand-rolled in collection.repo.ts and does not
      // know about writing tables, so the association must never RESTRICT.
      db.exec("DELETE FROM notes WHERE item_id='i1'")
      db.exec("DELETE FROM assets WHERE item_id='i1'")
      db.exec("DELETE FROM items WHERE collection_id='c1'")
      expect(() => db.exec("DELETE FROM collections WHERE id='c1'")).not.toThrow()

      expect(db.prepare('SELECT COUNT(*) AS n FROM writing_document_collections').get()?.n).toBe(0)
      expect(db.prepare("SELECT id FROM writing_documents WHERE id='d1'").get()?.id).toBe('d1')
    } finally {
      db.close()
    }
  })

  it('keeps one citation projection row per node and per cluster position', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      insertDocument(db)

      db.exec(
        `INSERT INTO writing_document_citations (id, document_id, citation_node_id, integrity_status, created_at, updated_at)
         VALUES ('dc1','d1','node-1','valid',1,1)`
      )
      expect(() =>
        db.exec(
          `INSERT INTO writing_document_citations (id, document_id, citation_node_id, integrity_status, created_at, updated_at)
           VALUES ('dc2','d1','node-1','valid',1,1)`
        )
      ).toThrow()

      db.exec(
        `INSERT INTO writing_zotero_citations (id, document_id, citation_node_id, citation_cluster_id, item_position, source_origin, library_type, library_id, item_key, integrity_status, created_at, updated_at)
         VALUES ('zc1','d1','node-2','cl-1',0,'local','user','0','K1','valid',1,1)`
      )
      // A cluster holds several items, so the same cluster with a new position is fine.
      expect(() =>
        db.exec(
          `INSERT INTO writing_zotero_citations (id, document_id, citation_node_id, citation_cluster_id, item_position, source_origin, library_type, library_id, item_key, integrity_status, created_at, updated_at)
           VALUES ('zc2','d1','node-2','cl-1',1,'local','user','0','K2','valid',1,1)`
        )
      ).not.toThrow()
      // The same position twice is not.
      expect(() =>
        db.exec(
          `INSERT INTO writing_zotero_citations (id, document_id, citation_node_id, citation_cluster_id, item_position, source_origin, library_type, library_id, item_key, integrity_status, created_at, updated_at)
           VALUES ('zc3','d1','node-2','cl-1',1,'local','user','0','K3','valid',1,1)`
        )
      ).toThrow()
    } finally {
      db.close()
    }
  })
})
