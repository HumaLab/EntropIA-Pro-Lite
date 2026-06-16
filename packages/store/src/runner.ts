import type { DbClient } from './types'

// ---------------------------------------------------------------------------
// Migration registry — SQL inlined as strings for Tauri bundling.
// Cannot use dynamic fs reads at runtime in a Tauri webview.
// ---------------------------------------------------------------------------
const MIGRATIONS: Record<string, string> = {
  '0001_initial': `
-- Migration tracking table
CREATE TABLE IF NOT EXISTS _migrations (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  name  TEXT    NOT NULL UNIQUE,
  applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS collections (
  id          TEXT    PRIMARY KEY,
  name        TEXT    NOT NULL,
  description TEXT,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS items (
  id            TEXT    PRIMARY KEY,
  title         TEXT    NOT NULL,
  collection_id TEXT    NOT NULL REFERENCES collections(id),
  metadata      TEXT,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS assets (
  id         TEXT    PRIMARY KEY,
  item_id    TEXT    NOT NULL REFERENCES items(id),
  path       TEXT    NOT NULL,
  type       TEXT    NOT NULL,
  size       INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS notes (
  id         TEXT    PRIMARY KEY,
  item_id    TEXT    NOT NULL REFERENCES items(id),
  content    TEXT    NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
  `.trim(),

  '0002_metadata_search': `
-- Add search_text generated column for LIKE queries
ALTER TABLE items ADD COLUMN search_text TEXT GENERATED ALWAYS AS (
  COALESCE(title, '') || ' ' || COALESCE(json(metadata), '')
) STORED;

-- Performance indexes
CREATE INDEX IF NOT EXISTS idx_items_search ON items(search_text);
CREATE INDEX IF NOT EXISTS idx_items_collection ON items(collection_id);
CREATE INDEX IF NOT EXISTS idx_assets_item ON assets(item_id);
CREATE INDEX IF NOT EXISTS idx_notes_item ON notes(item_id);
  `.trim(),

  '0003_extractions': `
CREATE TABLE IF NOT EXISTS extractions (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  text_content TEXT NOT NULL,
  method TEXT NOT NULL,
  confidence REAL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_extractions_asset_id ON extractions(asset_id);
  `.trim(),

  '0004_fts5': `
CREATE VIRTUAL TABLE IF NOT EXISTS fts_items
USING fts5(
  item_id UNINDEXED,
  title,
  metadata,
  extracted_text,
  tokenize='unicode61 remove_diacritics 1',
  content=''
);

INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)
SELECT i.rowid, i.id, i.title, COALESCE(i.metadata,''),
       COALESCE((SELECT GROUP_CONCAT(e.text_content,' ') FROM extractions e
                 JOIN assets a ON e.asset_id=a.id WHERE a.item_id=i.id), '')
FROM items i
  `.trim(),

  '0005_nlp_tables': `
CREATE TABLE IF NOT EXISTS entities (
  id TEXT PRIMARY KEY NOT NULL,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  entity_type TEXT NOT NULL CHECK(entity_type IN ('person','place','date','institution','organization','misc','custom')),
  value TEXT NOT NULL,
  start_offset INTEGER NOT NULL DEFAULT 0,
  end_offset INTEGER NOT NULL DEFAULT 0,
  confidence REAL NOT NULL DEFAULT 1.0,
  created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_entities_item_id ON entities(item_id);
CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type)
  `.trim(),

  '0006_triples': `
CREATE TABLE IF NOT EXISTS triples (
  id TEXT PRIMARY KEY NOT NULL,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  subject TEXT NOT NULL,
  predicate TEXT NOT NULL,
  object TEXT NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS triples_item_id_idx ON triples(item_id)
  `.trim(),

  '0007_annotations': `
CREATE TABLE IF NOT EXISTS annotations (
  id TEXT PRIMARY KEY NOT NULL,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  page INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL CHECK(kind IN ('rectangle', 'underline')),
  color TEXT NOT NULL,
  x REAL NOT NULL,
  y REAL NOT NULL,
  width REAL NOT NULL,
  height REAL NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS annotations_asset_id_idx ON annotations(asset_id);
CREATE INDEX IF NOT EXISTS annotations_asset_page_idx ON annotations(asset_id, page)
  `.trim(),

  '0008_transcriptions': `
CREATE TABLE IF NOT EXISTS transcriptions (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  text_content TEXT NOT NULL,
  language TEXT,
  duration_ms INTEGER,
  model TEXT NOT NULL,
  segments TEXT,
  confidence REAL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_transcriptions_asset_id ON transcriptions(asset_id)
  `.trim(),

  '0009_entities_provenance': `
CREATE TEMP TABLE IF NOT EXISTS __entropia_migration_0009_noop (id INTEGER);
DROP TABLE IF EXISTS __entropia_migration_0009_noop
  `.trim(),

  '0010_entities_type_expansion': `

DROP TABLE IF EXISTS entities_v2;

CREATE TABLE entities_v2 (
  id TEXT PRIMARY KEY NOT NULL,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  entity_type TEXT NOT NULL CHECK(entity_type IN ('person','place','date','institution','organization','misc','custom')),
  value TEXT NOT NULL,
  start_offset INTEGER NOT NULL DEFAULT 0,
  end_offset INTEGER NOT NULL DEFAULT 0,
  confidence REAL NOT NULL DEFAULT 1.0,
  source TEXT,
  model_name TEXT,
  created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

INSERT INTO entities_v2 (
  id, item_id, entity_type, value, start_offset, end_offset,
  confidence, source, model_name, created_at
)
SELECT
  id, item_id, entity_type, value, start_offset, end_offset,
  confidence, NULL, NULL, created_at
FROM entities;

DROP TABLE entities;
ALTER TABLE entities_v2 RENAME TO entities;

CREATE INDEX IF NOT EXISTS idx_entities_item_id ON entities(item_id);
CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type)
  `.trim(),

  '0011_entities_geocoding': `
ALTER TABLE entities ADD COLUMN latitude REAL;
ALTER TABLE entities ADD COLUMN longitude REAL;
ALTER TABLE entities ADD COLUMN geo_status TEXT NOT NULL DEFAULT 'pending';
CREATE INDEX IF NOT EXISTS idx_entities_geo_status ON entities(geo_status)
  `.trim(),

  '0012_llm_results': `
CREATE TABLE IF NOT EXISTS llm_results (
  id TEXT PRIMARY KEY,
  target_id TEXT NOT NULL,
  job_type TEXT NOT NULL,
  result TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_llm_results_target ON llm_results(target_id)
  `.trim(),

  '0013_assets_sort_index': `
-- Add sort_index to assets for stable page ordering (e.g. scanned PDF pages)
ALTER TABLE assets ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_assets_item_sort ON assets(item_id, sort_index);
  `.trim(),

  '0014_asset_scoping': `
-- Add asset_id to notes, entities, and triples for per-page scoping.
-- Nullable: legacy rows without asset_id are considered "item-level" (shown on all pages).
ALTER TABLE notes ADD COLUMN asset_id TEXT;
ALTER TABLE entities ADD COLUMN asset_id TEXT;
ALTER TABLE triples ADD COLUMN asset_id TEXT;
CREATE INDEX IF NOT EXISTS idx_notes_asset_id ON notes(asset_id);
CREATE INDEX IF NOT EXISTS idx_entities_asset_id ON entities(asset_id);
CREATE INDEX IF NOT EXISTS idx_triples_asset_id ON triples(asset_id);
  `.trim(),

  '0015_topics': `
-- Create topics table and item_topics junction table for reusable topic tagging
CREATE TABLE IF NOT EXISTS topics (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS item_topics (
  id TEXT PRIMARY KEY,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_item_topics_item_topic ON item_topics(item_id, topic_id);
CREATE INDEX IF NOT EXISTS idx_item_topics_topic_id ON item_topics(topic_id);
  `.trim(),

  '0016_asset_unique_ocr_transcription': `
-- Enforce one extraction/transcription row per asset to enable true UPSERT.
-- Keep the most recent row (largest rowid) if any legacy duplicates exist.
DELETE FROM extractions
WHERE rowid NOT IN (
  SELECT MAX(rowid) FROM extractions GROUP BY asset_id
);

DELETE FROM transcriptions
WHERE rowid NOT IN (
  SELECT MAX(rowid) FROM transcriptions GROUP BY asset_id
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_extractions_asset_id_unique
ON extractions(asset_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_transcriptions_asset_id_unique
ON transcriptions(asset_id);
  `.trim(),

  '0017_vec_assets': `
CREATE TABLE IF NOT EXISTS vec_assets(
  asset_id TEXT PRIMARY KEY,
  item_id TEXT NOT NULL,
  embedding BLOB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vec_assets_item_id ON vec_assets(item_id);
  `.trim(),

  '0018_fts_rowid_canonical': `
-- Rebuild FTS rows so fts_items.rowid always matches items.rowid.
INSERT INTO fts_items(fts_items) VALUES('delete-all');

INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)
SELECT
  i.rowid,
  i.id,
  i.title,
  COALESCE(i.metadata, ''),
  COALESCE((
    SELECT GROUP_CONCAT(text_part, ' ')
    FROM (
      SELECT text_part
      FROM (
        SELECT COALESCE(e.text_content, '') AS text_part,
               0 AS source_order,
               COALESCE(a.sort_index, 0) AS sort_index,
               e.created_at AS created_at
        FROM extractions e
        JOIN assets a ON a.id = e.asset_id
        WHERE a.item_id = i.id

        UNION ALL

        SELECT COALESCE(t.text_content, '') AS text_part,
               1 AS source_order,
               COALESCE(a.sort_index, 0) AS sort_index,
               t.created_at AS created_at
        FROM transcriptions t
        JOIN assets a ON a.id = t.asset_id
        WHERE a.item_id = i.id
      ) ordered_text
      ORDER BY source_order ASC, sort_index ASC, created_at ASC
    )
  ), '')
FROM items i
  `.trim(),

  '0019_llm_results_target_type': `
CREATE TABLE llm_results_v2 (
  id TEXT PRIMARY KEY,
  target_id TEXT NOT NULL,
  target_type TEXT NOT NULL CHECK(target_type IN ('asset', 'item', 'collection', 'unknown')),
  job_type TEXT NOT NULL,
  result TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

INSERT INTO llm_results_v2 (id, target_id, target_type, job_type, result, created_at)
SELECT
  lr.id,
  lr.target_id,
  CASE
    WHEN EXISTS (SELECT 1 FROM assets a WHERE a.id = lr.target_id) THEN 'asset'
    WHEN EXISTS (SELECT 1 FROM items i WHERE i.id = lr.target_id) THEN 'item'
    WHEN EXISTS (SELECT 1 FROM collections c WHERE c.id = lr.target_id) THEN 'collection'
    ELSE 'unknown'
  END,
  lr.job_type,
  lr.result,
  CASE
    WHEN lr.created_at > 0 AND lr.created_at < 1000000000000 THEN lr.created_at * 1000
    ELSE lr.created_at
  END
FROM llm_results lr;

DROP TABLE llm_results;
ALTER TABLE llm_results_v2 RENAME TO llm_results;

CREATE INDEX IF NOT EXISTS idx_llm_results_target ON llm_results(target_id);
CREATE INDEX IF NOT EXISTS idx_llm_results_target_typed
ON llm_results(target_type, target_id, job_type)
  `.trim(),

  '0020_layouts': `
-- Handled programmatically in runMigrations() because existing desktop databases
-- may already contain a legacy layouts table without the blocks column.
CREATE TEMP TABLE IF NOT EXISTS __entropia_migration_0020_noop (id INTEGER);
DROP TABLE IF EXISTS __entropia_migration_0020_noop
  `.trim(),

  '0021_drop_unused_processing_table': `
DROP TABLE IF EXISTS jobs
  `.trim(),

  '0022_rag_conversations': `
CREATE TABLE IF NOT EXISTS rag_conversations (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS rag_messages (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES rag_conversations(id) ON DELETE CASCADE,
  sort_index INTEGER NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('user','assistant')),
  content TEXT NOT NULL,
  sources TEXT,
  model TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rag_messages_conversation ON rag_messages(conversation_id, sort_index);
  `.trim(),

  '0023_sync_ids': `
-- Deterministic ids for one-per-asset tables (DESIGN §4.6). Rewrites existing
-- rows so two devices that OCR/transcribe the same asset converge on a single
-- server row. Nothing references these ids by FK (verified), so the rewrite is
-- safe. Additive — no rebuild of a synced table, so no re-seed is required.
UPDATE extractions SET id = 'ext-' || asset_id;
UPDATE transcriptions SET id = 'trx-' || asset_id;
UPDATE layouts SET id = 'lay-' || asset_id;
  `.trim(),
}

async function applyLayoutsMigration(client: DbClient): Promise<void> {
  const now = Date.now()
  await client.executeBatch(`
    CREATE TABLE IF NOT EXISTS layouts (
      id TEXT PRIMARY KEY,
      asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
      regions TEXT NOT NULL,
      blocks TEXT NOT NULL DEFAULT '[]',
      model TEXT NOT NULL,
      image_width INTEGER NOT NULL,
      image_height INTEGER NOT NULL,
      created_at INTEGER NOT NULL
    )
  `)

  try {
    await client.executeBatch("ALTER TABLE layouts ADD COLUMN blocks TEXT NOT NULL DEFAULT '[]'")
  } catch (error) {
    const message = String(error).toLowerCase()
    if (!message.includes('duplicate column name')) {
      throw error
    }
  }

  await client.execute(`
    DELETE FROM layouts
    WHERE rowid NOT IN (
      SELECT MAX(rowid) FROM layouts GROUP BY asset_id
    )
  `)
  await client.executeBatch(
    'CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id)'
  )
  await client.executeBatch('CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id)')
  await client.execute(`
    UPDATE layouts
    SET created_at = ${now}
    WHERE created_at IS NULL OR created_at = 0
  `)
}

/**
 * Split a multi-statement SQL string into individual statements.
 * Strips comments and empty lines, splits on semicolons.
 */
function splitStatements(sql: string): string[] {
  return sql
    .split(';')
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
}

/**
 * Runs all pending migrations in filename order.
 *
 * - Ensures the `_migrations` tracking table exists
 * - Fetches already-applied migration names
 * - Applies each pending migration inside a BEGIN/COMMIT transaction
 * - Inserts a record into `_migrations` on success
 * - On error: rolls back the transaction and rethrows with the migration name
 */
export async function runMigrations(client: DbClient): Promise<void> {
  console.log('[runner] runMigrations start')
  // Ensure tracking table exists (idempotent).
  // DDL must go through the batch path: the Tauri db_execute IPC is hardened
  // to DML-only (INSERT/UPDATE/DELETE), so schema statements are rejected there.
  // db_execute_batch validates the batch and runs CREATE/ALTER/DROP.
  await client.executeBatch(`
    CREATE TABLE IF NOT EXISTS _migrations (
      id    INTEGER PRIMARY KEY AUTOINCREMENT,
      name  TEXT    NOT NULL UNIQUE,
      applied_at INTEGER NOT NULL
    )
  `)

  // Fetch already-applied migrations
  const applied = await client.select<{ name: string }>('SELECT name FROM _migrations ORDER BY id')
  const appliedSet = new Set(applied.map((row) => row.name))

  // Sort migration keys by filename order (lexicographic)
  const pending = Object.keys(MIGRATIONS)
    .sort()
    .filter((name) => !appliedSet.has(name))

  for (const name of pending) {
    try {
      if (name === '0020_layouts') {
        await applyLayoutsMigration(client)
      } else {
        const sql = MIGRATIONS[name]!
        const statements = splitStatements(sql)

        // Schema statements (CREATE/ALTER/DROP) go through executeBatch because
        // the db_execute IPC is restricted to DML-only. db_execute_batch is the
        // sanctioned path for DDL and also accepts the DML inside these batches.
        for (const stmt of statements) {
          try {
            await client.executeBatch(stmt)
          } catch (stmtErr) {
            const msg = stmtErr instanceof Error ? stmtErr.message : String(stmtErr)
            if (/duplicate column name/i.test(msg)) {
              console.warn(`[runner] Skipping statement in "${name}" (column already exists)`)
              continue
            }
            throw stmtErr
          }
        }
      }

      // Tracking insert stays on the DML path (db_execute).
      await client.execute('INSERT INTO _migrations (name, applied_at) VALUES (?, ?)', [
        name,
        Math.floor(Date.now() / 1000),
      ])
    } catch (error) {
      throw new Error(
        `Migration "${name}" failed: ${error instanceof Error ? error.message : String(error)}`
      )
    }
  }
}
