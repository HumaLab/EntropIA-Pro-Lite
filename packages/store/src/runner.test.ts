import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { COLLECTION_ACTIVITY_DDL, buildSchemaFixture, runMigrations } from './runner'
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
    try {
      await runMigrations(client)

      const ddl = db.prepare("SELECT sql FROM sqlite_master WHERE name='fts_items'").get() as {
        sql: string
      }
      expect(ddl.sql).toContain('contentless_delete=1')

      // Seed one indexed item the way the app does, then correct its text.
      db.exec(`INSERT INTO collections(id, name, created_at, updated_at) VALUES('c1','legajo',1,1);
        INSERT INTO items(id, title, collection_id, created_at, updated_at) VALUES('i1','Acta','c1',1,1);`)
      const rowid = (
        db.prepare("SELECT rowid AS r FROM items WHERE id='i1'").get() as { r: number }
      ).r
      const index = (text: string) =>
        db
          .prepare(
            'INSERT OR REPLACE INTO fts_items(rowid, item_id, title, metadata, extracted_text) VALUES (?,?,?,?,?)'
          )
          .run(rowid, 'i1', 'Acta', '', text)
      index('zanahoria del sindicato')
      index('berenjena del sindicato')

      const hits = (term: string) =>
        Number(
          (
            db.prepare('SELECT COUNT(*) AS n FROM fts_items WHERE fts_items MATCH ?').get(term) as {
              n: number
            }
          ).n
        )
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
    try {
      db.exec(`CREATE TABLE _migrations(id INTEGER PRIMARY KEY, name TEXT UNIQUE, applied_at INTEGER);
        CREATE TRIGGER interrupt_queue_migration BEFORE INSERT ON _migrations
        WHEN NEW.name='0032_batch_processing' BEGIN SELECT RAISE(ABORT,'simulated storage failure'); END;`)
      await expect(runMigrations(client)).rejects.toThrow('simulated storage failure')
      expect(
        db.prepare("SELECT name FROM sqlite_master WHERE name='processing_batches'").get()
      ).toBeUndefined()
      expect(
        db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()
      ).toBeUndefined()
      db.exec('DROP TRIGGER interrupt_queue_migration')
      await runMigrations(client)
      expect(
        db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()?.name
      ).toBe('0032_batch_processing')
      await runMigrations(client)
      db.prepare(
        "INSERT INTO processing_batches(id,request_id,origin,state,desired_state,operations,created_at,updated_at) VALUES('b','r','user','preparing','pause','[]',0,0)"
      ).run()
      expect(db.prepare("SELECT state FROM processing_batches WHERE id='b'").get()?.state).toBe(
        'preparing'
      )
    } finally {
      db.close()
    }
  })

  it('repairs a half-applied 0032 (tables without registry row) and still applies 0033', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
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
    try {
      // Start from a real fully-migrated database, then burn it into the
      // exact shape the earlier non-atomic build left behind: complete 0032
      // tables, no registry rows, no invalidation counter column.
      await runMigrations(client)
      db.exec(
        `DELETE FROM _migrations WHERE name IN ('0032_batch_processing','0033_processing_source_invalidation')`
      )
      const hadColumn =
        (db
          .prepare(
            "SELECT name FROM pragma_table_info('processing_tasks') WHERE name='source_invalidation_count'"
          )
          .get() as { name: string } | undefined) !== undefined
      if (hadColumn) db.exec('ALTER TABLE processing_tasks DROP COLUMN source_invalidation_count')
      await expect(runMigrations(client)).resolves.toBeUndefined()
      expect(
        db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()?.name
      ).toBe('0032_batch_processing')
      expect(
        db
          .prepare("SELECT name FROM _migrations WHERE name='0033_processing_source_invalidation'")
          .get()?.name
      ).toBe('0033_processing_source_invalidation')
      const columns = db
        .prepare("SELECT name FROM pragma_table_info('processing_tasks')")
        .all() as Array<{ name: string }>
      expect(columns.map((row: { name: string }) => row.name)).toContain(
        'source_invalidation_count'
      )
    } finally {
      db.close()
    }
  })

  it('repairs a PARTIAL 0032 when every surviving table is empty', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
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
    try {
      // The shape the FIRST non-atomic build left behind: it split the
      // migration on ';', which shreds the CREATE TRIGGER … BEGIN … END;
      // bodies. Every statement before the first trigger had already
      // autocommitted, so the tables survive without processing_meta, without
      // the triggers, and without a registry row.
      await runMigrations(client)
      db.exec(
        `DELETE FROM _migrations WHERE name IN ('0032_batch_processing','0033_processing_source_invalidation')`
      )
      db.exec('DROP TABLE processing_meta')
      for (const trigger of [
        'trg_processing_extractions_ai',
        'trg_processing_extractions_au',
        'trg_processing_extractions_ad',
        'trg_processing_transcriptions_ai',
        'trg_processing_transcriptions_au',
        'trg_processing_transcriptions_ad',
      ])
        db.exec(`DROP TRIGGER ${trigger}`)
      const hadColumn =
        (db
          .prepare(
            "SELECT name FROM pragma_table_info('processing_tasks') WHERE name='source_invalidation_count'"
          )
          .get() as { name: string } | undefined) !== undefined
      if (hadColumn) db.exec('ALTER TABLE processing_tasks DROP COLUMN source_invalidation_count')

      await expect(runMigrations(client)).resolves.toBeUndefined()

      expect(
        db.prepare("SELECT name FROM _migrations WHERE name='0032_batch_processing'").get()?.name
      ).toBe('0032_batch_processing')
      expect(
        db
          .prepare("SELECT name FROM _migrations WHERE name='0033_processing_source_invalidation'")
          .get()?.name
      ).toBe('0033_processing_source_invalidation')
      expect(
        db.prepare("SELECT name FROM sqlite_master WHERE name='processing_meta'").get()?.name
      ).toBe('processing_meta')
      expect(
        db
          .prepare("SELECT name FROM sqlite_master WHERE name='trg_processing_extractions_ai'")
          .get()?.name
      ).toBe('trg_processing_extractions_ai')
      const columns = db
        .prepare("SELECT name FROM pragma_table_info('processing_tasks')")
        .all() as Array<{ name: string }>
      expect(columns.map((row: { name: string }) => row.name)).toContain(
        'source_invalidation_count'
      )
    } finally {
      db.close()
    }
  })

  it('refuses to drop a partial 0032 whose tables still hold queue rows', async () => {
    const db = new DatabaseSync(':memory:')
    const client: DbClient = {
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
    try {
      await runMigrations(client)
      db.exec(
        `DELETE FROM _migrations WHERE name IN ('0032_batch_processing','0033_processing_source_invalidation')`
      )
      db.exec('DROP TABLE processing_meta')
      db.prepare(
        "INSERT INTO processing_batches(id,request_id,origin,state,desired_state,operations,created_at,updated_at) VALUES('b','r','user','preparing','pause','[]',0,0)"
      ).run()

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
    try {
      await runMigrations(client)
      expect(
        db
          .prepare("SELECT name FROM _migrations WHERE name='0033_processing_source_invalidation'")
          .get()?.name
      ).toBe('0033_processing_source_invalidation')
      const columns = db
        .prepare("SELECT name FROM pragma_table_info('processing_tasks')")
        .all() as Array<{ name: string }>
      expect(columns.map((row: { name: string }) => row.name)).toContain(
        'source_invalidation_count'
      )
      // The historical 0032 attempts CHECK stays untouched; source changes
      // close attempts as interrupted with a source_changed error code.
      const attempts = db
        .prepare("SELECT sql FROM sqlite_master WHERE name='processing_attempts'")
        .get() as { sql: string }
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

      expect(
        db.prepare("SELECT revision FROM writing_documents WHERE id='d1'").get()?.revision
      ).toBe(0)
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

describe('writing recovery journal migration (0036)', () => {
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

  const withDocument = async (db: DatabaseSync) => {
    db.exec('PRAGMA foreign_keys=ON')
    await runMigrations(shim(db))
    db.exec(
      `INSERT INTO writing_documents
         (id, title, document_type, status, schema_version, current_content_json, revision, created_at, updated_at)
       VALUES ('d1','Articulo','article','active',1,'{"type":"doc"}',0,1,1)`
    )
  }

  const appendEntry = (db: DatabaseSync, seq: number, baseRevision = 0) =>
    db.exec(
      `INSERT INTO writing_journal (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
       VALUES ('d1', ${seq}, ${baseRevision}, 1, '[]', 'h${seq}', ${seq})`
    )

  it('creates the journal table without disturbing the writing tables', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      await withDocument(db)
      await runMigrations(shim(db)) // idempotent

      expect(
        db
          .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='writing_journal'")
          .get()
      ).toBeDefined()
      expect(
        db.prepare("SELECT revision FROM writing_documents WHERE id='d1'").get()?.revision
      ).toBe(0)
    } finally {
      db.close()
    }
  })

  it('keeps one entry per sequence number within a document', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      await withDocument(db)
      appendEntry(db, 1)
      expect(() => appendEntry(db, 1)).toThrow()
      expect(() => appendEntry(db, 2)).not.toThrow()
    } finally {
      db.close()
    }
  })

  it('drops a document journal with the document', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      await withDocument(db)
      appendEntry(db, 1)
      appendEntry(db, 2)

      db.exec("DELETE FROM writing_documents WHERE id='d1'")

      expect(db.prepare('SELECT COUNT(*) AS n FROM writing_journal').get()?.n).toBe(0)
    } finally {
      db.close()
    }
  })

  it('refuses a journal entry for a document that does not exist', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      await withDocument(db)
      expect(() =>
        db.exec(
          `INSERT INTO writing_journal (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
           VALUES ('ghost', 1, 0, 1, '[]', 'h', 1)`
        )
      ).toThrow()
    } finally {
      db.close()
    }
  })
})

describe('bibliography catalog migrations (0038, 0039)', () => {
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

  it('creates the connection, library and item foundation idempotently with qualified identity', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))

      for (const table of ['zotero_connections', 'zotero_libraries', 'bibliographic_items']) {
        expect(
          db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name=?").get(table),
          `missing catalog table: ${table}`
        ).toBeDefined()
      }

      const itemIndex = db
        .prepare(
          "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_bibliographic_items_library_key'"
        )
        .get() as { sql: string } | undefined
      expect(itemIndex?.sql).toContain('(library_id, item_key)')

      await runMigrations(shim(db))
      expect(
        db
          .prepare("SELECT COUNT(*) AS n FROM _migrations WHERE name='0038_bibliography_catalog'")
          .get()?.n
      ).toBe(1)
    } finally {
      db.close()
    }
  })

  it('keeps the checked-in 0038 SQL mirror aligned with the generated fixture', () => {
    const mirror = readFileSync(
      resolve(here, 'migrations/0038_bibliography_catalog.sql'),
      'utf8'
    ).trim()
    expect(buildSchemaFixture()).toContain(`-- 0038_bibliography_catalog\n${mirror}`)
  })

  it('creates the E1b-1b relational catalog and replay-safe tombstone tables', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))

      for (const table of [
        'zotero_collections',
        'zotero_tags',
        'zotero_attachments',
        'zotero_item_collections',
        'zotero_item_tags',
        'zotero_item_tombstones',
        'zotero_collection_tombstones',
        'zotero_tag_tombstones',
        'zotero_attachment_tombstones',
      ]) {
        expect(
          db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name=?").get(table),
          `missing bibliography relation table: ${table}`
        ).toBeDefined()
      }

      expect(
        db
          .prepare("SELECT COUNT(*) AS n FROM _migrations WHERE name='0039_bibliography_relations'")
          .get()?.n
      ).toBe(1)
      await runMigrations(shim(db))
      expect(
        db
          .prepare("SELECT COUNT(*) AS n FROM _migrations WHERE name='0039_bibliography_relations'")
          .get()?.n
      ).toBe(1)
    } finally {
      db.close()
    }
  })

  it('requires lossless native snapshots and nullable native versions on relation entities', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))

      for (const table of ['zotero_collections', 'zotero_tags', 'zotero_attachments']) {
        const columns = db.prepare(`PRAGMA table_info(${table})`).all() as Array<{
          name: string
          notnull: number
        }>
        const snapshot = columns.find((column) => column.name === 'native_json_snapshot')
        const version = columns.find((column) => column.name === 'native_version')
        expect(snapshot, `${table} must store a native JSON snapshot`).toBeDefined()
        expect(snapshot?.notnull, `${table} native snapshot must be required`).toBe(1)
        expect(version, `${table} must store a nullable native version`).toBeDefined()
        expect(version?.notnull, `${table} native version must be nullable`).toBe(0)

        const ddl = db
          .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name=?")
          .get(table) as { sql: string }
        expect(ddl.sql).toContain('CHECK(json_valid(native_json_snapshot))')
      }
    } finally {
      db.close()
    }
  })

  it('keeps the checked-in 0039 SQL mirror aligned with the generated fixture', () => {
    const mirror = readFileSync(
      resolve(here, 'migrations/0039_bibliography_relations.sql'),
      'utf8'
    ).trim()
    expect(buildSchemaFixture()).toContain(`-- 0039_bibliography_relations\n${mirror}`)
  })

  it('creates and replays the durable 0040 reconciliation state', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))

      for (const table of ['zotero_reconciliation_runs', 'zotero_reconciliation_seen']) {
        expect(
          db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name=?").get(table),
          `missing reconciliation table: ${table}`
        ).toBeDefined()
      }
      expect(
        db.prepare("SELECT COUNT(*) AS n FROM _migrations WHERE name='0040_bibliography_reconciliation'").get()?.n
      ).toBe(1)

      await runMigrations(shim(db))
      expect(
        db.prepare("SELECT COUNT(*) AS n FROM _migrations WHERE name='0040_bibliography_reconciliation'").get()?.n
      ).toBe(1)
    } finally {
      db.close()
    }
  })

  it('keeps the checked-in 0040 SQL mirror aligned with the generated fixture', () => {
    const mirror = readFileSync(
      resolve(here, 'migrations/0040_bibliography_reconciliation.sql'),
      'utf8'
    ).trim()
    expect(buildSchemaFixture()).toContain(`-- 0040_bibliography_reconciliation\n${mirror}`)
  })
})

describe('processing task-subject identity migration (0041)', () => {
  const MIGRATION_0041 = '0041_processing_task_subject_identity'
  const mirrorPath = resolve(here, 'migrations/0041_processing_task_subject_identity.sql')

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

  it('registers 0041 and emits the subject-identity DDL through the runner', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain('subject_kind')
    expect(migrationSql).toContain('idx_processing_tasks_subject_active_unique')
  })

  it('keeps the checked-in 0041 SQL mirror exactly equal to the registry copy', () => {
    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    const fixture = buildSchemaFixture()
    const marker = `-- ${MIGRATION_0041}\n`
    const start = fixture.indexOf(marker)
    expect(
      start,
      '0041 missing from the generated fixture — register it in MIGRATIONS'
    ).toBeGreaterThanOrEqual(0)
    const rest = fixture.slice(start + marker.length)
    const next = rest.search(/\n-- \d{4}_/)
    const section = (next === -1 ? rest : rest.slice(0, next)).trim()
    expect(section).toBe(mirror)
  })

  it('replays through the runner as an error-free no-op with a single registry row', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0041}'`).get()?.n
      ).toBe(1)
      for (const table of ['processing_tasks', 'processing_batch_tasks']) {
        const columns = db
          .prepare(`SELECT name FROM pragma_table_info('${table}')`)
          .all() as Array<{ name: string }>
        for (const column of ['domain', 'subject_kind', 'subject_id']) {
          expect(
            columns.map((row) => row.name),
            `${table} is missing ${column}`
          ).toContain(column)
        }
      }
      expect(
        db
          .prepare(
            "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_processing_tasks_subject_active_unique'"
          )
          .get()
      ).toBeDefined()
    } finally {
      db.close()
    }
  })

  it('backfills legacy corpus rows from the snapshot without touching documentary fields', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))

      // Burn the database back into its pre-0041 shape: complete 0032/0033
      // tables, no 0041 registry row, no subject columns, no subject index.
      // DROP COLUMN is confirmed to work on the CHECK-constrained columns
      // once the dependent partial unique is dropped first.
      db.exec(`DELETE FROM _migrations WHERE name='${MIGRATION_0041}'`)
      db.exec('DROP INDEX IF EXISTS idx_processing_tasks_subject_active_unique')
      for (const table of ['processing_tasks', 'processing_batch_tasks']) {
        for (const column of ['domain', 'subject_kind', 'subject_id']) {
          db.exec(`ALTER TABLE ${table} DROP COLUMN ${column}`)
        }
      }

      db.exec(`INSERT INTO processing_batches(id, request_id, origin, state, desired_state, operations, created_at, updated_at)
        VALUES ('b1', 'req-1', 'user', 'running', 'run', '["ocr"]', 1, 1)`)
      db.exec(`INSERT INTO processing_tasks(id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, created_at, updated_at) VALUES
        ('t-pending', 'ocr', 'a1', 3, 'fp-a1', 'ch-a1', 'pending', 10, 11),
        ('t-interrupted', 'embedding', 'a2', 5, 'fp-a2', 'ch-a2', 'interrupted', 12, 13),
        ('t-done', 'ocr', 'a3', 7, 'fp-a3', 'ch-a3', 'succeeded', 14, 15)`)
      db.exec(`INSERT INTO processing_batch_tasks(batch_id, task_id, kind, asset_id_snapshot, request_state) VALUES
        ('b1', 't-pending', 'ocr', 'a1', 'active'),
        ('b1', 't-interrupted', 'embedding', 'a2', 'paused')`)
      db.exec(`INSERT INTO processing_checkpoints(task_id, unit_key, input_fingerprint, contract_hash, payload, created_at)
        VALUES ('t-pending', 'page:1', 'fp-a1', 'ch-a1', '{}', 16)`)

      await runMigrations(shim(db))

      const tasks = db
        .prepare(
          'SELECT id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, input_revision, input_fingerprint, contract_hash FROM processing_tasks ORDER BY id'
        )
        .all() as Array<Record<string, unknown>>
      expect(tasks).toEqual([
        { id: 't-done', kind: 'ocr', asset_id_snapshot: 'a3', domain: 'corpus', subject_kind: 'asset', subject_id: 'a3', state: 'succeeded', input_revision: 7, input_fingerprint: 'fp-a3', contract_hash: 'ch-a3' },
        { id: 't-interrupted', kind: 'embedding', asset_id_snapshot: 'a2', domain: 'corpus', subject_kind: 'asset', subject_id: 'a2', state: 'interrupted', input_revision: 5, input_fingerprint: 'fp-a2', contract_hash: 'ch-a2' },
        { id: 't-pending', kind: 'ocr', asset_id_snapshot: 'a1', domain: 'corpus', subject_kind: 'asset', subject_id: 'a1', state: 'pending', input_revision: 3, input_fingerprint: 'fp-a1', contract_hash: 'ch-a1' },
      ])

      const links = db
        .prepare(
          'SELECT batch_id, task_id, domain, subject_kind, subject_id, request_state FROM processing_batch_tasks ORDER BY task_id'
        )
        .all() as Array<Record<string, unknown>>
      expect(links).toEqual([
        { batch_id: 'b1', task_id: 't-interrupted', domain: 'corpus', subject_kind: 'asset', subject_id: 'a2', request_state: 'paused' },
        { batch_id: 'b1', task_id: 't-pending', domain: 'corpus', subject_kind: 'asset', subject_id: 'a1', request_state: 'active' },
      ])

      const checkpoints = db
        .prepare('SELECT task_id, unit_key, input_fingerprint, contract_hash, payload FROM processing_checkpoints')
        .all() as Array<Record<string, unknown>>
      expect(checkpoints).toEqual([
        { task_id: 't-pending', unit_key: 'page:1', input_fingerprint: 'fp-a1', contract_hash: 'ch-a1', payload: '{}' },
      ])
    } finally {
      db.close()
    }
  })
})

describe('processing task-subject cutover migration (0042)', () => {
  const MIGRATION_0042 = '0042_processing_task_subject_cutover'
  const mirrorPath = resolve(here, 'migrations/0042_processing_task_subject_cutover.sql')

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

  const liveIndexNames = (db: DatabaseSync): string[] =>
    (
      db.prepare("SELECT name FROM pragma_index_list('processing_tasks')").all() as Array<{
        name: string
      }>
    ).map((row) => row.name)

  it('registers 0042 and emits the cutover DDL through the runner', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain('DROP INDEX IF EXISTS idx_processing_tasks_active_unique')
    expect(migrationSql).toContain('idx_processing_tasks_subject_active_unique')
  })

  it('keeps the checked-in 0042 SQL mirror exactly equal to the registry copy', () => {
    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    const fixture = buildSchemaFixture()
    const marker = `-- ${MIGRATION_0042}\n`
    const start = fixture.indexOf(marker)
    expect(
      start,
      '0042 missing from the generated fixture — register it in MIGRATIONS'
    ).toBeGreaterThanOrEqual(0)
    const rest = fixture.slice(start + marker.length)
    const next = rest.search(/\n-- \d{4}_/)
    const section = (next === -1 ? rest : rest.slice(0, next)).trim()
    expect(section).toBe(mirror)
  })

  it('replays through the runner as an error-free no-op with a single registry row', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0042}'`).get()?.n
      ).toBe(1)
      // The composite is now the sole single-flight authority.
      expect(liveIndexNames(db)).toContain('idx_processing_tasks_subject_active_unique')
      expect(liveIndexNames(db)).not.toContain('idx_processing_tasks_active_unique')
    } finally {
      db.close()
    }
  })

  it('drops the old snapshot unique on upgraded databases while keeping the composite', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))

      // Burn the database back into its pre-0042 shape: the full 0041-era
      // index pair, no 0042 registry row. The old partial unique is
      // recreated exactly as 0032 built it.
      db.exec(`DELETE FROM _migrations WHERE name='${MIGRATION_0042}'`)
      db.exec(`CREATE UNIQUE INDEX IF NOT EXISTS idx_processing_tasks_active_unique
        ON processing_tasks(kind, asset_id_snapshot)
        WHERE state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')`)
      expect(liveIndexNames(db)).toContain('idx_processing_tasks_active_unique')

      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0042}'`).get()?.n
      ).toBe(1)
      expect(liveIndexNames(db)).toContain('idx_processing_tasks_subject_active_unique')
      expect(liveIndexNames(db)).not.toContain('idx_processing_tasks_active_unique')
      // Replay stays an error-free no-op.
      await runMigrations(shim(db))
      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0042}'`).get()?.n
      ).toBe(1)
    } finally {
      db.close()
    }
  })
})

describe('bibliography task admission migration (0043)', () => {
  const MIGRATION_0043 = '0043_bibliography_sync_tasks'
  const mirrorPath = resolve(here, 'migrations/0043_bibliography_sync_tasks.sql')

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

  /** Build a database that has every migration before 0043 recorded. */
  const before0043 = (db: DatabaseSync) => {
    const fullFixture = buildSchemaFixture()
    const marker = `-- ${MIGRATION_0043}`
    const cut = fullFixture.indexOf(marker)
    const prefix = cut < 0 ? fullFixture : fullFixture.slice(0, cut)
    db.exec(prefix)
    const names = [...prefix.matchAll(/^-- (\d{4}_[A-Za-z0-9_]+)\s*$/gm)].map(
      (match) => match[1] as string
    )
    for (const name of names) {
      db.prepare('INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?, 1)').run(name)
    }
  }

  const tableRows = (db: DatabaseSync, table: string): unknown[][] =>
    (db.prepare(`SELECT * FROM ${table} ORDER BY 1`).all() as Array<Record<string, unknown>>).map(
      Object.values
    )

  it('registers 0043 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\\n')
    expect(migrationSql).toContain(MIGRATION_0043)
    expect(migrationSql).toContain("'bibliography_sync'")
    expect(migrationSql).toContain("'bibliography'")
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0043}\n${mirror}`)
  })

  it('freshly applies and replays 0043 without duplicating its registry row', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0043}'`).get()?.n
      ).toBe(1)
      db.prepare(
        `INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES ('b-biblio', 'req-biblio', 'bibliography', 'running', 'run', '[]', 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
         VALUES ('t-biblio', 'bibliography_sync', 'library-row-1', 'bibliography', 'library', 'library-row-1', 'pending', 1, 1)`
      ).run()
      expect(
        db.prepare("SELECT kind FROM processing_tasks WHERE id='t-biblio'").get()?.kind
      ).toBe('bibliography_sync')
    } finally {
      db.close()
    }
  })

  it('upgrades pre-0043 queue rows byte-identically and preserves constraints/indexes', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      before0043(db)
      db.prepare(
        `INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES ('b1', 'req-1', 'user', 'running', 'run', '["ocr", "embeddings"]', 1, 1, 1)`
      ).run()

      for (const [id, kind, snapshot, state] of [
        ['t-pending', 'ocr', 'asset-pending', 'pending'],
        ['t-paused', 'embedding', 'asset-paused', 'pending'],
        ['t-interrupted', 'ocr', 'asset-interrupted', 'interrupted'],
        ['t-running', 'embedding', 'asset-running', 'running'],
      ] as Array<[string, string, string, string]>) {
        db.prepare(
          `INSERT INTO processing_tasks
             (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, input_revision,
              input_fingerprint, contract_hash, state, owner_session, lease_epoch, created_at, updated_at)
           VALUES (?, ?, ?, 'corpus', 'asset', ?, ?, ?, ?, ?, ?, ?, 10, 11)`
        ).run(
          id,
          kind,
          snapshot,
          snapshot,
          state === 'pending' ? 3 : 5,
          `fp-${id}`,
          `contract-${id}`,
          state,
          state === 'running' ? 'session-old' : null,
          state === 'running' ? 9 : 0
        )
      }
      for (const [taskId, kind, snapshot, requestState] of [
        ['t-pending', 'ocr', 'asset-pending', 'active'],
        ['t-paused', 'embedding', 'asset-paused', 'paused'],
        ['t-interrupted', 'ocr', 'asset-interrupted', 'active'],
        ['t-running', 'embedding', 'asset-running', 'active'],
      ] as Array<[string, string, string, string]>) {
        db.prepare(
          `INSERT INTO processing_batch_tasks
             (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
           VALUES ('b1', ?, ?, ?, 'corpus', 'asset', ?, ?)`
        ).run(taskId, kind, snapshot, snapshot, requestState)
      }
      db.prepare(
        `INSERT INTO processing_attempts
           (task_id, attempt_number, lease_epoch, started_at, outcome)
         VALUES ('t-interrupted', 1, 8, 20, 'interrupted'),
                ('t-running', 1, 10, 21, 'open')`
      ).run()
      db.prepare(
        `INSERT INTO processing_checkpoints
           (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
         VALUES ('t-interrupted', 'page:1', 'fp-t-interrupted', 'contract-t-interrupted', '{}', 'sum-1', 22),
                ('t-running', 'page:1', 'fp-t-running', 'contract-t-running', '{"ok":true}', 'sum-2', 23)`
      ).run()

      const beforeTasks = tableRows(db, 'processing_tasks')
      const beforeLinks = tableRows(db, 'processing_batch_tasks')
      const beforeBatches = tableRows(db, 'processing_batches')
      const beforeAttempts = tableRows(db, 'processing_attempts')
      const beforeCheckpoints = tableRows(db, 'processing_checkpoints')

      await runMigrations(shim(db))

      expect(tableRows(db, 'processing_tasks')).toEqual(beforeTasks)
      expect(tableRows(db, 'processing_batch_tasks')).toEqual(beforeLinks)
      // 0044 rides the same runMigrations pass: the only batches difference is
      // the additive priority default appended last by ALTER TABLE.
      expect(tableRows(db, 'processing_batches')).toEqual(beforeBatches.map((row) => [...row, 0]))
      expect(tableRows(db, 'processing_attempts')).toEqual(beforeAttempts)
      expect(tableRows(db, 'processing_checkpoints')).toEqual(beforeCheckpoints)

      const taskIndexes = (
        db.prepare("SELECT name FROM pragma_index_list('processing_tasks')").all() as Array<{
          name: string
        }>
      ).map((row) => row.name)
      expect(taskIndexes).toEqual(
        expect.arrayContaining([
          'idx_processing_tasks_claimable',
          'idx_processing_tasks_subject_active_unique',
        ])
      )
      const claimableSql = db
        .prepare(
          "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_processing_tasks_claimable'"
        )
        .get() as { sql: string }
      const subjectSql = db
        .prepare(
          "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_processing_tasks_subject_active_unique'"
        )
        .get() as { sql: string }
      expect(claimableSql.sql).toContain('ON processing_tasks(state, next_retry_at, id)')
      expect(subjectSql.sql).toContain(
        'ON processing_tasks(domain, subject_kind, subject_id, kind)'
      )
      expect(subjectSql.sql).toContain("state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')")

      const foreignKeys = db
        .prepare("SELECT \"table\" AS \"table\", \"from\" AS \"from\", \"to\" AS \"to\" FROM pragma_foreign_key_list('processing_batch_tasks')")
        .all() as Array<{ table: string; from: string; to: string }>
      expect(foreignKeys).toEqual(
        expect.arrayContaining([
          { table: 'processing_batches', from: 'batch_id', to: 'id' },
          { table: 'processing_tasks', from: 'task_id', to: 'id' },
          { table: 'processing_tasks', from: 'dependency_task_id', to: 'id' },
        ])
      )

      db.prepare(
        `INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES ('b-biblio', 'req-biblio', 'bibliography', 'running', 'run', '[]', 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
         VALUES ('t-biblio', 'bibliography_sync', 'library-row-1', 'bibliography', 'library', 'library-row-1', 'pending', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO processing_batch_tasks
           (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
         VALUES ('b-biblio', 't-biblio', 'bibliography_sync', 'library-row-1', 'bibliography', 'library', 'library-row-1', 'active')`
      ).run()
      expect(() =>
        db.prepare(
          `INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at)
           VALUES ('t-invalid', 'not-a-kind', 'asset-invalid', 'pending', 1, 1)`
        ).run()
      ).toThrow()
      expect(
        db.prepare("SELECT kind FROM processing_tasks WHERE id IN ('t-pending','t-paused','t-interrupted','t-running') ORDER BY id").all()
      ).toEqual([
        { kind: 'ocr' },
        { kind: 'embedding' },
        { kind: 'ocr' },
        { kind: 'embedding' },
      ])
    } finally {
      db.close()
    }
  })
})

describe('batch priority migration (0044)', () => {
  const MIGRATION_0044 = '0044_processing_priority'
  const mirrorPath = resolve(here, 'migrations/0044_processing_priority.sql')

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

  /** Build a database that has every migration before 0044 recorded. */
  const before0044 = (db: DatabaseSync) => {
    const fullFixture = buildSchemaFixture()
    const marker = `-- ${MIGRATION_0044}`
    const cut = fullFixture.indexOf(marker)
    const prefix = cut < 0 ? fullFixture : fullFixture.slice(0, cut)
    db.exec(prefix)
    const names = [...prefix.matchAll(/^-- (\d{4}_[A-Za-z0-9_]+)\s*$/gm)].map(
      (match) => match[1] as string
    )
    for (const name of names) {
      db.prepare('INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?, 1)').run(name)
    }
  }

  it('registers 0044 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0044)
    expect(migrationSql).toContain('ADD COLUMN priority')
    expect(migrationSql).toContain('idx_processing_batches_priority')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0044}\n${mirror}`)
  })

  it('freshly applies and replays 0044 without duplicating its registry row', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0044}'`).get()?.n
      ).toBe(1)
      const columns = (
        db.prepare("SELECT name FROM pragma_table_info('processing_batches')").all() as Array<{
          name: string
        }>
      ).map((row) => row.name)
      expect(columns).toContain('priority')
      const indexes = (
        db.prepare("SELECT name FROM pragma_index_list('processing_batches')").all() as Array<{
          name: string
        }>
      ).map((row) => row.name)
      expect(indexes).toContain('idx_processing_batches_priority')
      // The default is background; only 0/1/2 are admitted.
      db.prepare(
        `INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES ('b-default', 'req-default', 'user', 'running', 'run', '[]', 1, 1, 1)`
      ).run()
      expect(
        db.prepare("SELECT priority FROM processing_batches WHERE id='b-default'").get()?.priority
      ).toBe(0)
      for (const priority of [1, 2]) {
        db.prepare(
          `INSERT INTO processing_batches
             (id, request_id, origin, state, desired_state, operations, planning_done, priority, created_at, updated_at)
           VALUES (?, ?, 'user', 'running', 'run', '[]', 1, ?, 1, 1)`
        ).run(`b-p${priority}`, `req-p${priority}`, priority)
      }
      expect(() =>
        db.prepare(
          `INSERT INTO processing_batches
             (id, request_id, origin, state, desired_state, operations, planning_done, priority, created_at, updated_at)
           VALUES ('b-bad', 'req-bad', 'user', 'running', 'run', '[]', 1, 3, 1, 1)`
        ).run()
      ).toThrow()
    } finally {
      db.close()
    }
  })

  it('upgrades pre-0044 batch rows with a background default and touches nothing else', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      before0044(db)
      db.prepare(
        `INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES ('b1', 'req-1', 'user', 'running', 'run', '["ocr"]', 1, 1, 1)`
      ).run()
      const before = db
        .prepare(
          'SELECT id, request_id, origin, state, desired_state, operations, planning_done, revision, created_at, updated_at FROM processing_batches ORDER BY id'
        )
        .all() as Array<Record<string, unknown>>

      await runMigrations(shim(db))

      const after = db
        .prepare(
          'SELECT id, request_id, origin, state, desired_state, operations, planning_done, revision, priority, created_at, updated_at FROM processing_batches ORDER BY id'
        )
        .all() as Array<Record<string, unknown>>
      expect(after).toEqual(before.map((row) => ({ ...row, priority: 0 })))
      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0044}'`).get()?.n
      ).toBe(1)
    } finally {
      db.close()
    }
  })
})

describe('bibliographic semantic profiles migration (0045)', () => {
  const MIGRATION_0045 = '0045_bibliographic_semantic_profiles'
  const mirrorPath = resolve(here, 'migrations/0045_bibliographic_semantic_profiles.sql')

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

  it('registers 0045 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0045)
    expect(migrationSql).toContain('REFERENCES bibliographic_items(id) ON DELETE CASCADE')
    expect(migrationSql).toContain('idx_bibliographic_semantic_profiles_hash')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0045}\n${mirror}`)
  })

  it('freshly applies and replays 0045 without duplicating its registry row', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0045}'`).get()?.n
      ).toBe(1)
      db.prepare(
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_semantic_profiles
           (item_id, profile_revision, template_version, canonical_text, input_hash, field_provenance_json, created_at, updated_at)
         VALUES ('item-1', 1, 'bibliography-profile-v1', 'Título: Obra', 'hash-1', '[]', 1, 1)`
      ).run()
      expect(
        db.prepare(
          'SELECT input_hash FROM bibliographic_semantic_profiles WHERE item_id = ?'
        ).get('item-1')?.input_hash
      ).toBe('hash-1')
      // A catalog row delete cascades: profiles are reconstructible, never
      // entangled with citations.
      db.prepare('DELETE FROM bibliographic_items WHERE id = ?').run('item-1')
      expect(
        db.prepare('SELECT COUNT(*) AS n FROM bibliographic_semantic_profiles').get()?.n
      ).toBe(0)
    } finally {
      db.close()
    }
  })
})

describe('bibliography profile tasks migration (0046)', () => {
  const MIGRATION_0046 = '0046_bibliography_profile_tasks'
  const mirrorPath = resolve(here, 'migrations/0046_bibliography_profile_tasks.sql')

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

  /** Build a database that has every migration before 0046 recorded. */
  const before0046 = (db: DatabaseSync) => {
    const fullFixture = buildSchemaFixture()
    const marker = `-- ${MIGRATION_0046}`
    const cut = fullFixture.indexOf(marker)
    const prefix = cut < 0 ? fullFixture : fullFixture.slice(0, cut)
    db.exec(prefix)
    const names = [...prefix.matchAll(/^-- (\d{4}_[A-Za-z0-9_]+)\s*$/gm)].map(
      (match) => match[1] as string
    )
    for (const name of names) {
      db.prepare('INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?, 1)').run(name)
    }
  }

  it('registers 0046 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0046)
    expect(migrationSql).toContain("'bibliography_profile'")
    expect(migrationSql).toContain('bibliographic_item_embeddings')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0046}\n${mirror}`)
  })

  const tableRows = (db: DatabaseSync, table: string): unknown[][] =>
    (db.prepare(`SELECT * FROM ${table} ORDER BY 1`).all() as Array<Record<string, unknown>>).map(
      Object.values
    )

  it('upgrades pre-0046 queue rows byte-identically and admits the profile kind', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      before0046(db)
      db.prepare(
        `INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES ('b1', 'req-1', 'bibliography', 'running', 'run', '[]', 1, 1, 1)`
      ).run()
      for (const [id, kind] of [
        ['t-sync', 'bibliography_sync'],
        ['t-ocr', 'ocr'],
      ] as Array<[string, string]>) {
        db.prepare(
          `INSERT INTO processing_tasks
             (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
           VALUES (?, ?, 'subj', 'bibliography', 'library', 'lib-1', 'pending', 1, 1)`
        ).run(id, kind)
        db.prepare(
          `INSERT INTO processing_batch_tasks
             (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
           VALUES ('b1', ?, ?, 'subj', 'bibliography', 'library', 'lib-1', 'active')`
        ).run(id, kind)
      }
      db.prepare(
        `INSERT INTO processing_attempts (task_id, attempt_number, lease_epoch, started_at, outcome)
         VALUES ('t-ocr', 1, 3, 10, 'open')`
      ).run()
      db.prepare(
        `INSERT INTO processing_checkpoints (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
         VALUES ('t-sync', 'page:0', 'fp', 'ch', '{}', 'sum', 11)`
      ).run()

      const beforeTasks = tableRows(db, 'processing_tasks')
      const beforeLinks = tableRows(db, 'processing_batch_tasks')
      const beforeAttempts = tableRows(db, 'processing_attempts')
      const beforeCheckpoints = tableRows(db, 'processing_checkpoints')

      await runMigrations(shim(db))

      expect(tableRows(db, 'processing_tasks')).toEqual(beforeTasks)
      expect(tableRows(db, 'processing_batch_tasks')).toEqual(beforeLinks)
      expect(tableRows(db, 'processing_attempts')).toEqual(beforeAttempts)
      expect(tableRows(db, 'processing_checkpoints')).toEqual(beforeCheckpoints)
      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0046}'`).get()?.n
      ).toBe(1)
      // The widened kind admits a profile task through both tables.
      db.prepare(
        `INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
         VALUES ('t-profile', 'bibliography_profile', 'item-1', 'bibliography', 'item', 'item-1', 'pending', 1, 1)`
      ).run()
      expect(
        db.prepare("SELECT kind FROM processing_tasks WHERE id='t-profile'").get()?.kind
      ).toBe('bibliography_profile')
      expect(() =>
        db.prepare(
          `INSERT INTO processing_tasks
             (id, kind, asset_id_snapshot, state, created_at, updated_at)
           VALUES ('t-bad', 'not-a-kind', 'x', 'pending', 1, 1)`
        ).run()
      ).toThrow()
      db.prepare(
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES ('c1', 'api', 'm1', 4, 'chunk', 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_index_generations
           (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
         VALUES ('gen-46', 'c1', 'staging', 1, 0, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_item_embeddings
           (item_id, generation_id, embedding_contract, embedding_model, dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
         VALUES ('item-1', 'gen-46', 'c1', 'm1', 4, zeroblob(4), 'h1', 1, 1, 1)`
      ).run()
      expect(
        db.prepare(
          'SELECT input_hash FROM bibliographic_item_embeddings WHERE item_id=? AND embedding_contract=?'
        ).get('item-1', 'c1')?.input_hash
      ).toBe('h1')
    } finally {
      db.close()
    }
  })
})

describe('bibliographic index generations migration (0047)', () => {
  const MIGRATION_0047 = '0047_bibliographic_index_generations'
  const mirrorPath = resolve(here, 'migrations/0047_bibliographic_index_generations.sql')

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

  it('registers 0047 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0047)
    expect(migrationSql).toContain('bibliographic_embedding_contracts')
    expect(migrationSql).toContain('bibliographic_index_generations')
    expect(migrationSql).toContain('idx_bibliographic_generations_single_active')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0047}\n${mirror}`)
  })

  it('freshly applies and replays 0047 with one active generation per contract', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0047}'`).get()?.n
      ).toBe(1)
      db.prepare(
        `INSERT INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES ('c1', 'api', 'baai/bge-m3', 1024, 'rag-chunk-800-100-char-v1', 1)`
      ).run()
      for (const [id, status] of [
        ['g-staging', 'staging'],
        ['g-active', 'active'],
      ] as Array<[string, string]>) {
        db.prepare(
          `INSERT INTO bibliographic_index_generations
             (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
           VALUES (?, 'c1', ?, 2, 2, 1)`
        ).run(id, status)
      }
      expect(() =>
        db.prepare(
          `INSERT INTO bibliographic_index_generations
             (id, contract_hash, status, created_at)
           VALUES ('g-second-active', 'c1', 'active', 1)`
        ).run()
      ).toThrow()
    } finally {
      db.close()
    }
  })
})

describe('bibliographic embedding generations migration (0048)', () => {
  const MIGRATION_0048 = '0048_bibliographic_embedding_generations'
  const mirrorPath = resolve(here, 'migrations/0048_bibliographic_embedding_generations.sql')

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

  /** Build a database that has every migration before 0048 recorded. */
  const before0048 = (db: DatabaseSync) => {
    const fullFixture = buildSchemaFixture()
    const marker = `-- ${MIGRATION_0048}`
    const cut = fullFixture.indexOf(marker)
    const prefix = cut < 0 ? fullFixture : fullFixture.slice(0, cut)
    db.exec(prefix)
    const names = [...prefix.matchAll(/^-- (\d{4}_[A-Za-z0-9_]+)\s*$/gm)].map(
      (match) => match[1] as string
    )
    for (const name of names) {
      db.prepare('INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?, 1)').run(name)
    }
  }

  const tableRows = (db: DatabaseSync, table: string): unknown[][] =>
    (db.prepare(`SELECT * FROM ${table} ORDER BY 1`).all() as Array<Record<string, unknown>>).map(
      Object.values
    )

  it('registers 0048 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0048)
    expect(migrationSql).toContain('PRIMARY KEY (item_id, generation_id)')
    expect(migrationSql).toContain('gen-legacy-')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0048}\n${mirror}`)
  })

  it('keeps legacy vectors under honest retired ancestry and enforces object/generation uniqueness', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      before0048(db)
      db.prepare(
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_item_embeddings
           (item_id, embedding_contract, embedding_model, dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
         VALUES ('item-1', 'c-old', 'm-old', 4, zeroblob(4), 'h1', 1, 1, 1)`
      ).run()

      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0048}'`).get()?.n
      ).toBe(1)
      const legacy = db.prepare(
        'SELECT generation_id, embedding, input_hash, profile_revision FROM bibliographic_item_embeddings WHERE item_id = ?'
      ).get('item-1') as { generation_id: string; input_hash: string; profile_revision: number }
      expect(legacy.generation_id).toMatch(/^gen-legacy-/)
      expect(legacy.input_hash).toBe('h1')
      expect(legacy.profile_revision).toBe(1)
      const gen = db.prepare(
        'SELECT status, contract_hash FROM bibliographic_index_generations WHERE id = ?'
      ).get(legacy.generation_id) as { status: string; contract_hash: string }
      expect(gen.status).toBe('retired')
      expect(gen.contract_hash).toBe('c-old')
      // The same work may carry a fresh-generation vector beside the legacy
      // one; two vectors for the same generation may not.
      db.prepare(
        `INSERT INTO bibliographic_index_generations
           (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
         VALUES ('gen-new', 'c-old', 'staging', 1, 0, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_item_embeddings
           (item_id, generation_id, embedding_contract, embedding_model, dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
         VALUES ('item-1', 'gen-new', 'c-old', 'm-old', 4, zeroblob(4), 'h2', 2, 1, 1)`
      ).run()
      expect(() =>
        db.prepare(
          `INSERT INTO bibliographic_item_embeddings
             (item_id, generation_id, embedding_contract, embedding_model, dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
           VALUES ('item-1', 'gen-new', 'c-old', 'm-old', 4, zeroblob(4), 'h3', 3, 1, 1)`
        ).run()
      ).toThrow()
    } finally {
      db.close()
    }
  })
})

describe('bibliographic extraction tasks migration (0050)', () => {
  const MIGRATION_0050 = '0050_bibliographic_extraction_tasks'
  const mirrorPath = resolve(here, 'migrations/0050_bibliographic_extraction_tasks.sql')

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

  /** Build a database that has every migration before 0050 recorded. */
  const before0050 = (db: DatabaseSync) => {
    const fullFixture = buildSchemaFixture()
    const marker = `-- ${MIGRATION_0050}`
    const cut = fullFixture.indexOf(marker)
    const prefix = cut < 0 ? fullFixture : fullFixture.slice(0, cut)
    db.exec(prefix)
    const names = [...prefix.matchAll(/^-- (\d{4}_[A-Za-z0-9_]+)\s*$/gm)].map(
      (match) => match[1] as string
    )
    for (const name of names) {
      db.prepare('INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?, 1)').run(name)
    }
  }

  const tableRows = (db: DatabaseSync, table: string): unknown[][] =>
    (db.prepare(`SELECT * FROM ${table} ORDER BY 1`).all() as Array<Record<string, unknown>>).map(
      Object.values
    )

  it('registers 0050 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0050)
    expect(migrationSql).toContain("'bibliography_extract'")
    expect(migrationSql).toContain('bibliographic_extractions')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0050}\n${mirror}`)
  })

  it('upgrades pre-0050 queue rows byte-identically and cascades extraction rows', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      before0050(db)
      db.prepare(
        `INSERT INTO processing_batches
           (id, request_id, origin, state, desired_state, operations, planning_done, created_at, updated_at)
         VALUES ('b1', 'req-1', 'bibliography', 'running', 'run', '[]', 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
         VALUES ('t-prof', 'bibliography_profile', 'item-1', 'bibliography', 'item', 'item-1', 'pending', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO processing_batch_tasks
           (batch_id, task_id, kind, asset_id_snapshot, domain, subject_kind, subject_id, request_state)
         VALUES ('b1', 't-prof', 'bibliography_profile', 'item-1', 'bibliography', 'item', 'item-1', 'active')`
      ).run()

      const beforeTasks = tableRows(db, 'processing_tasks')
      const beforeLinks = tableRows(db, 'processing_batch_tasks')

      await runMigrations(shim(db))

      expect(tableRows(db, 'processing_tasks')).toEqual(beforeTasks)
      expect(tableRows(db, 'processing_batch_tasks')).toEqual(beforeLinks)
      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0050}'`).get()?.n
      ).toBe(1)
      // The widened kind admits an extraction task.
      db.prepare(
        `INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
         VALUES ('t-ext', 'bibliography_extract', 'att-1', 'bibliography', 'attachment', 'att-1', 'pending', 1, 1)`
      ).run()
      expect(
        db.prepare("SELECT kind FROM processing_tasks WHERE id='t-ext'").get()?.kind
      ).toBe('bibliography_extract')
      // Extraction rows cascade with the catalog: deleting the work removes
      // the attachment row and its extraction in one statement.
      db.prepare(
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_attachments (id, item_id, attachment_key, native_json_snapshot, created_at, updated_at, verified_at)
         VALUES ('att-1', 'item-1', 'ABCDEF12', '{}', 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_extractions
           (attachment_id, item_id, page_count, method, text_content, text_hash, text_chars, quality, source_bytes, created_at, updated_at)
         VALUES ('att-1', 'item-1', 3, 'native', 'texto nativo', 'h1', 12, 'sparse', 99, 1, 1)`
      ).run()
      expect(
        db.prepare(
          'SELECT quality FROM bibliographic_extractions WHERE attachment_id = ?'
        ).get('att-1')?.quality
      ).toBe('sparse')
      db.prepare('DELETE FROM bibliographic_items WHERE id = ?').run('item-1')
      expect(
        db.prepare('SELECT COUNT(*) AS n FROM bibliographic_extractions').get()?.n
      ).toBe(0)
    } finally {
      db.close()
    }
  })
})

describe('bibliographic page texts migration (0051)', () => {
  const MIGRATION_0051 = '0051_bibliographic_page_texts'
  const mirrorPath = resolve(here, 'migrations/0051_bibliographic_page_texts.sql')

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

  it('registers 0051 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0051)
    expect(migrationSql).toContain('bibliographic_page_texts')
    expect(migrationSql).toContain('idx_bibliographic_page_texts_attachment')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0051}\n${mirror}`)
  })

  it('freshly applies and replays 0051 with per-page rows and cascade', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0051}'`).get()?.n
      ).toBe(1)
      db.prepare(
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_attachments (id, item_id, attachment_key, native_json_snapshot, created_at, updated_at, verified_at)
         VALUES ('att-1', 'item-1', 'ABCDEF12', '{}', 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_page_texts
           (attachment_id, page_number, method, text_content, text_hash, text_chars, quality, created_at, updated_at)
         VALUES ('att-1', 1, 'native', 'texto pagina uno', 'h1', 16, 'sparse', 1, 1),
                ('att-1', 2, 'native', 'texto pagina dos', 'h2', 16, 'sparse', 1, 1)`
      ).run()
      expect(
        db.prepare(
          'SELECT COUNT(*) AS n FROM bibliographic_page_texts WHERE attachment_id = ?'
        ).get('att-1')?.n
      ).toBe(2)
      expect(() =>
        db.prepare(
          `INSERT INTO bibliographic_page_texts
             (attachment_id, page_number, method, text_content, text_hash, text_chars, quality, created_at, updated_at)
           VALUES ('att-1', 0, 'native', 'x', 'h0', 1, 'sparse', 1, 1)`
        ).run()
      ).toThrow()
      db.prepare('DELETE FROM bibliographic_items WHERE id = ?').run('item-1')
      expect(
        db.prepare('SELECT COUNT(*) AS n FROM bibliographic_page_texts').get()?.n
      ).toBe(0)
    } finally {
      db.close()
    }
  })
})

describe('bibliographic chunks migration (0052)', () => {
  const MIGRATION_0052 = '0052_bibliographic_chunks'
  const mirrorPath = resolve(here, 'migrations/0052_bibliographic_chunks.sql')

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

  it('registers 0052 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0052)
    expect(migrationSql).toContain('bibliographic_chunks')
    expect(migrationSql).toContain('bibliographic_chunk_spans')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0052}\n${mirror}`)
  })

  it('freshly applies and replays 0052 with spans and cascade', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0052}'`).get()?.n
      ).toBe(1)
      db.prepare(
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_attachments (id, item_id, attachment_key, native_json_snapshot, created_at, updated_at, verified_at)
         VALUES ('att-1', 'item-1', 'ABCDEF12', '{}', 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_chunks
           (id, item_id, attachment_id, ordinal, text_content, text_hash, chunking_contract, created_at, updated_at)
         VALUES ('chunk-1', 'item-1', 'att-1', 0, 'texto', 'h1', 'test-contract', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_chunk_spans (chunk_id, page_number, start_char, end_char)
         VALUES ('chunk-1', 1, 0, 5), ('chunk-1', 2, 0, 5)`
      ).run()
      expect(
        db.prepare('SELECT COUNT(*) AS n FROM bibliographic_chunk_spans WHERE chunk_id = ?').get(
          'chunk-1'
        )?.n
      ).toBe(2)
      expect(() =>
        db.prepare(
          `INSERT INTO bibliographic_chunks
             (id, item_id, attachment_id, ordinal, text_content, text_hash, chunking_contract, created_at, updated_at)
           VALUES ('chunk-2', 'item-1', 'att-1', 0, 'otro', 'h2', 'test-contract', 1, 1)`
        ).run()
      ).toThrow()
      db.prepare('DELETE FROM bibliographic_items WHERE id = ?').run('item-1')
      expect(db.prepare('SELECT COUNT(*) AS n FROM bibliographic_chunks').get()?.n).toBe(0)
      expect(db.prepare('SELECT COUNT(*) AS n FROM bibliographic_chunk_spans').get()?.n).toBe(0)
    } finally {
      db.close()
    }
  })
})

describe('bibliographic chunks migration (0052)', () => {
  const MIGRATION_0052 = '0052_bibliographic_chunks'
  const mirrorPath = resolve(here, 'migrations/0052_bibliographic_chunks.sql')

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

  it('registers 0052 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0052)
    expect(migrationSql).toContain('bibliographic_chunks')
    expect(migrationSql).toContain('bibliographic_chunk_spans')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0052}\n${mirror}`)
  })

  it('freshly applies and replays 0052 with spans and cascade', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0052}'`).get()?.n
      ).toBe(1)
      db.prepare(
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO zotero_attachments (id, item_id, attachment_key, native_json_snapshot, created_at, updated_at, verified_at)
         VALUES ('att-1', 'item-1', 'ABCDEF12', '{}', 1, 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_chunks
           (id, item_id, attachment_id, ordinal, text_content, text_hash, chunking_contract, created_at, updated_at)
         VALUES ('chunk-1', 'item-1', 'att-1', 0, 'texto', 'h1', 'test-contract', 1, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_chunk_spans (chunk_id, page_number, start_char, end_char)
         VALUES ('chunk-1', 1, 0, 5), ('chunk-1', 2, 0, 5)`
      ).run()
      expect(
        db.prepare('SELECT COUNT(*) AS n FROM bibliographic_chunk_spans WHERE chunk_id = ?').get(
          'chunk-1'
        )?.n
      ).toBe(2)
      expect(() =>
        db.prepare(
          `INSERT INTO bibliographic_chunks
             (id, item_id, attachment_id, ordinal, text_content, text_hash, chunking_contract, created_at, updated_at)
           VALUES ('chunk-2', 'item-1', 'att-1', 0, 'otro', 'h2', 'test-contract', 1, 1)`
        ).run()
      ).toThrow()
      db.prepare('DELETE FROM bibliographic_items WHERE id = ?').run('item-1')
      expect(db.prepare('SELECT COUNT(*) AS n FROM bibliographic_chunks').get()?.n).toBe(0)
      expect(db.prepare('SELECT COUNT(*) AS n FROM bibliographic_chunk_spans').get()?.n).toBe(0)
    } finally {
      db.close()
    }
  })
})

describe('bibliographic chunk embeddings migration (0053)', () => {
  const MIGRATION_0053 = '0053_bibliographic_chunk_embeddings'
  const mirrorPath = resolve(here, 'migrations/0053_bibliographic_chunk_embeddings.sql')

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

  it('registers 0053 and keeps its checked-in SQL mirror byte-identical', async () => {
    const client = createMockDbClient()
    await runMigrations(client)

    const migrationSql = client._executedSql.join('\n')
    expect(migrationSql).toContain(MIGRATION_0053)
    expect(migrationSql).toContain('bibliographic_chunk_embeddings')
    expect(migrationSql).toContain('PRIMARY KEY (chunk_id, generation_id)')
    expect(migrationSql).toContain('BEGIN IMMEDIATE')

    const mirror = readFileSync(mirrorPath, 'utf8').trim()
    expect(buildSchemaFixture()).toContain(`-- ${MIGRATION_0053}\n${mirror}`)
  })

  it('freshly applies and replays 0053 with per-chunk generation uniqueness', async () => {
    const db = new DatabaseSync(':memory:')
    try {
      db.exec('PRAGMA foreign_keys=ON')
      await runMigrations(shim(db))
      await runMigrations(shim(db))

      expect(
        db.prepare(`SELECT COUNT(*) AS n FROM _migrations WHERE name='${MIGRATION_0053}'`).get()?.n
      ).toBe(1)
      for (const stmt of [
        `INSERT INTO zotero_connections (id, source_origin, capabilities_json, state, created_at, updated_at)
         VALUES ('conn-1', 'local', '{}', 'available', 1, 1)`,
        `INSERT INTO zotero_libraries (id, connection_id, library_type, library_id, name, created_at, updated_at)
         VALUES ('lib-1', 'conn-1', 'user', '0', 'Personal', 1, 1)`,
        `INSERT INTO bibliographic_items (id, library_id, item_key, title, native_json_snapshot, csl_json_snapshot, item_version, verified_at, created_at, updated_at)
         VALUES ('item-1', 'lib-1', 'AAAA1111', 'Obra', '{}', '{}', 1, 1, 1, 1)`,
        `INSERT INTO zotero_attachments (id, item_id, attachment_key, native_json_snapshot, created_at, updated_at, verified_at)
         VALUES ('att-1', 'item-1', 'ABCDEF12', '{}', 1, 1, 1)`,
        `INSERT INTO bibliographic_chunks
           (id, item_id, attachment_id, ordinal, text_content, text_hash, chunking_contract, created_at, updated_at)
         VALUES ('chunk-1', 'item-1', 'att-1', 0, 'texto', 'h1', 'c', 1, 1)`,
        `INSERT INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES ('ch-1', 'api', 'm', 4, 'chunking', 1)`,
        `INSERT INTO bibliographic_index_generations
           (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
         VALUES ('gen-1', 'ch-1', 'active', 1, 1, 1)`,
        `INSERT INTO bibliographic_chunk_embeddings
           (chunk_id, generation_id, embedding_contract, embedding_model, dimensions, embedding, input_hash, created_at, updated_at)
         VALUES ('chunk-1', 'gen-1', 'ch-1', 'm', 4, zeroblob(4), 'h1', 1, 1)`,
      ]) {
        db.prepare(stmt).run()
      }
      expect(
        db.prepare(
          'SELECT input_hash FROM bibliographic_chunk_embeddings WHERE chunk_id = ? AND generation_id = ?'
        ).get('chunk-1', 'gen-1')?.input_hash
      ).toBe('h1')
      // Same chunk under another generation coexists; same pair does not.
      db.prepare(
        `INSERT INTO bibliographic_index_generations
           (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
         VALUES ('gen-2', 'ch-1', 'staging', 1, 0, 1)`
      ).run()
      db.prepare(
        `INSERT INTO bibliographic_chunk_embeddings
           (chunk_id, generation_id, embedding_contract, embedding_model, dimensions, embedding, input_hash, created_at, updated_at)
         VALUES ('chunk-1', 'gen-2', 'ch-1', 'm', 4, zeroblob(4), 'h1', 1, 1)`
      ).run()
      expect(() =>
        db.prepare(
          `INSERT INTO bibliographic_chunk_embeddings
             (chunk_id, generation_id, embedding_contract, embedding_model, dimensions, embedding, input_hash, created_at, updated_at)
           VALUES ('chunk-1', 'gen-2', 'ch-1', 'm', 4, zeroblob(4), 'h9', 1, 1)`
        ).run()
      ).toThrow()
      // Deleting the chunk cascades its vectors.
      db.prepare('DELETE FROM bibliographic_chunks WHERE id = ?').run('chunk-1')
      expect(
        db.prepare('SELECT COUNT(*) AS n FROM bibliographic_chunk_embeddings').get()?.n
      ).toBe(0)
    } finally {
      db.close()
    }
  })
})
