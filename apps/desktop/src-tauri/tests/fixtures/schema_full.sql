-- AUTO-GENERATED schema fixture for the Rust sync tests. DO NOT EDIT BY HAND.
-- Regenerate with: pnpm --filter @entropia/store export-schema
-- Source of truth: packages/store/src/runner.ts (MIGRATIONS + LAYOUTS_DDL).

-- 0001_initial
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

-- 0002_metadata_search
-- Add search_text generated column for LIKE queries
ALTER TABLE items ADD COLUMN search_text TEXT GENERATED ALWAYS AS (
  COALESCE(title, '') || ' ' || COALESCE(json(metadata), '')
) STORED;

-- Performance indexes
CREATE INDEX IF NOT EXISTS idx_items_search ON items(search_text);
CREATE INDEX IF NOT EXISTS idx_items_collection ON items(collection_id);
CREATE INDEX IF NOT EXISTS idx_assets_item ON assets(item_id);
CREATE INDEX IF NOT EXISTS idx_notes_item ON notes(item_id);

-- 0003_extractions
CREATE TABLE IF NOT EXISTS extractions (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  text_content TEXT NOT NULL,
  method TEXT NOT NULL,
  confidence REAL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_extractions_asset_id ON extractions(asset_id);

-- 0004_fts5
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
FROM items i;

-- 0005_nlp_tables
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
CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type);

-- 0006_triples
CREATE TABLE IF NOT EXISTS triples (
  id TEXT PRIMARY KEY NOT NULL,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  subject TEXT NOT NULL,
  predicate TEXT NOT NULL,
  object TEXT NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS triples_item_id_idx ON triples(item_id);

-- 0007_annotations
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
CREATE INDEX IF NOT EXISTS annotations_asset_page_idx ON annotations(asset_id, page);

-- 0008_transcriptions
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
CREATE INDEX IF NOT EXISTS idx_transcriptions_asset_id ON transcriptions(asset_id);

-- 0009_entities_provenance
CREATE TEMP TABLE IF NOT EXISTS __entropia_migration_0009_noop (id INTEGER);
DROP TABLE IF EXISTS __entropia_migration_0009_noop;

-- 0010_entities_type_expansion
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
CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type);

-- 0011_entities_geocoding
ALTER TABLE entities ADD COLUMN latitude REAL;
ALTER TABLE entities ADD COLUMN longitude REAL;
ALTER TABLE entities ADD COLUMN geo_status TEXT NOT NULL DEFAULT 'pending';
CREATE INDEX IF NOT EXISTS idx_entities_geo_status ON entities(geo_status);

-- 0012_llm_results
CREATE TABLE IF NOT EXISTS llm_results (
  id TEXT PRIMARY KEY,
  target_id TEXT NOT NULL,
  job_type TEXT NOT NULL,
  result TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_llm_results_target ON llm_results(target_id);

-- 0013_assets_sort_index
-- Add sort_index to assets for stable page ordering (e.g. scanned PDF pages)
ALTER TABLE assets ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_assets_item_sort ON assets(item_id, sort_index);

-- 0014_asset_scoping
-- Add asset_id to notes, entities, and triples for per-page scoping.
-- Nullable: legacy rows without asset_id are considered "item-level" (shown on all pages).
ALTER TABLE notes ADD COLUMN asset_id TEXT;
ALTER TABLE entities ADD COLUMN asset_id TEXT;
ALTER TABLE triples ADD COLUMN asset_id TEXT;
CREATE INDEX IF NOT EXISTS idx_notes_asset_id ON notes(asset_id);
CREATE INDEX IF NOT EXISTS idx_entities_asset_id ON entities(asset_id);
CREATE INDEX IF NOT EXISTS idx_triples_asset_id ON triples(asset_id);

-- 0015_topics
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

-- 0016_asset_unique_ocr_transcription
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

-- 0017_vec_assets
CREATE TABLE IF NOT EXISTS vec_assets(
  asset_id TEXT PRIMARY KEY,
  item_id TEXT NOT NULL,
  embedding BLOB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vec_assets_item_id ON vec_assets(item_id);

-- 0018_fts_rowid_canonical
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
FROM items i;

-- 0019_llm_results_target_type
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
ON llm_results(target_type, target_id, job_type);

-- 0020_layouts
CREATE TABLE IF NOT EXISTS layouts (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  regions TEXT NOT NULL,
  blocks TEXT NOT NULL DEFAULT '[]',
  model TEXT NOT NULL,
  image_width INTEGER NOT NULL,
  image_height INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id);
CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id);

-- 0021_drop_unused_processing_table
DROP TABLE IF EXISTS jobs;

-- 0022_rag_conversations
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

-- 0023_sync_ids
-- Deterministic ids for one-per-asset tables (DESIGN §4.6). Rewrites existing
-- rows so two devices that OCR/transcribe the same asset converge on a single
-- server row. Nothing references these ids by FK (verified), so the rewrite is
-- safe. Additive — no rebuild of a synced table, so no re-seed is required.
UPDATE extractions SET id = 'ext-' || asset_id;
UPDATE transcriptions SET id = 'trx-' || asset_id;
UPDATE layouts SET id = 'lay-' || asset_id;

-- 0024_pdf_page_assets
-- Page children are additive: legacy assets remain standalone and keep their
-- existing aggregate OCR/layout rows. New GLM PDF pages point at their source.
ALTER TABLE assets ADD COLUMN parent_asset_id TEXT REFERENCES assets(id) ON DELETE CASCADE;
ALTER TABLE assets ADD COLUMN page_number INTEGER;
CREATE INDEX IF NOT EXISTS idx_assets_parent_asset_id ON assets(parent_asset_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_assets_parent_page
ON assets(parent_asset_id, page_number)
WHERE parent_asset_id IS NOT NULL;

-- 0025_document_view_edits
-- Rebuild annotations so PDF page edits can be persisted without modifying the
-- source document. Existing rectangle/underline rows retain their ids and data.
CREATE TABLE annotations_v2 (
  id TEXT PRIMARY KEY NOT NULL,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  page INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL CHECK(kind IN ('rectangle', 'underline', 'crop', 'erase', 'rotation')),
  color TEXT NOT NULL,
  x REAL NOT NULL,
  y REAL NOT NULL,
  width REAL NOT NULL,
  height REAL NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
INSERT INTO annotations_v2
SELECT id, asset_id, page, kind, color, x, y, width, height, created_at, updated_at
FROM annotations;
DROP TABLE annotations;
ALTER TABLE annotations_v2 RENAME TO annotations;
CREATE INDEX annotations_asset_id_idx ON annotations(asset_id);
CREATE INDEX annotations_asset_page_idx ON annotations(asset_id, page);

-- 0026_entity_manual_coordinates
ALTER TABLE entities ADD COLUMN manual_lat REAL;
ALTER TABLE entities ADD COLUMN manual_lon REAL;

-- 0027_collection_activity
-- Keep collections.updated_at as the canonical last-activity timestamp. Database
-- triggers cover repository writes, Rust worker writes, and synchronized changes.
CREATE TRIGGER collection_activity_items_insert
AFTER INSERT ON items
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = NEW.collection_id;
END;

CREATE TRIGGER collection_activity_items_update
AFTER UPDATE ON items
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = OLD.collection_id;
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = NEW.collection_id;
END;

CREATE TRIGGER collection_activity_items_delete
BEFORE DELETE ON items
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = OLD.collection_id;
END;

CREATE TRIGGER collection_activity_assets_insert
AFTER INSERT ON assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_assets_update
AFTER UPDATE ON assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_assets_delete
BEFORE DELETE ON assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_notes_insert
AFTER INSERT ON notes
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_notes_update
AFTER UPDATE ON notes
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_notes_delete
BEFORE DELETE ON notes
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_extractions_insert
AFTER INSERT ON extractions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_extractions_update
AFTER UPDATE ON extractions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_extractions_delete
BEFORE DELETE ON extractions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_layouts_insert
AFTER INSERT ON layouts
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_layouts_update
AFTER UPDATE ON layouts
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_layouts_delete
BEFORE DELETE ON layouts
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_transcriptions_insert
AFTER INSERT ON transcriptions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_transcriptions_update
AFTER UPDATE ON transcriptions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_transcriptions_delete
BEFORE DELETE ON transcriptions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_annotations_insert
AFTER INSERT ON annotations
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_annotations_update
AFTER UPDATE ON annotations
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_annotations_delete
BEFORE DELETE ON annotations
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_entities_insert
AFTER INSERT ON entities
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_entities_update
AFTER UPDATE ON entities
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_entities_delete
BEFORE DELETE ON entities
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_triples_insert
AFTER INSERT ON triples
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_triples_update
AFTER UPDATE ON triples
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_triples_delete
BEFORE DELETE ON triples
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_vec_assets_insert
AFTER INSERT ON vec_assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_vec_assets_update
AFTER UPDATE ON vec_assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_vec_assets_delete
BEFORE DELETE ON vec_assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_llm_results_insert
AFTER INSERT ON llm_results
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (NEW.target_type IN ('collection', 'unknown') AND id = NEW.target_id)
  OR (NEW.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = NEW.target_id
  ))
  OR (NEW.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = NEW.target_id
  ));
END;

CREATE TRIGGER collection_activity_llm_results_update
AFTER UPDATE ON llm_results
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (OLD.target_type IN ('collection', 'unknown') AND id = OLD.target_id)
  OR (OLD.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = OLD.target_id
  ))
  OR (OLD.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = OLD.target_id
  ));
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (NEW.target_type IN ('collection', 'unknown') AND id = NEW.target_id)
  OR (NEW.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = NEW.target_id
  ))
  OR (NEW.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = NEW.target_id
  ));
END;

CREATE TRIGGER collection_activity_llm_results_delete
BEFORE DELETE ON llm_results
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (OLD.target_type IN ('collection', 'unknown') AND id = OLD.target_id)
  OR (OLD.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = OLD.target_id
  ))
  OR (OLD.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = OLD.target_id
  ));
END;

-- 0028_vec_assets_embedding_contract
ALTER TABLE vec_assets ADD COLUMN embedding_model TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE vec_assets ADD COLUMN embedding_contract TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE vec_assets ADD COLUMN dimensions INTEGER NOT NULL DEFAULT 0;

-- 0029_rag_chunks
CREATE TABLE rag_chunks (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  source_kind TEXT NOT NULL CHECK(source_kind IN ('extraction', 'transcription')),
  source_id TEXT NOT NULL,
  chunk_ordinal INTEGER NOT NULL CHECK(chunk_ordinal >= 0),
  text_content TEXT NOT NULL,
  start_char INTEGER NOT NULL CHECK(start_char >= 0),
  end_char INTEGER NOT NULL CHECK(end_char > start_char),
  source_text_hash TEXT NOT NULL,
  chunking_contract TEXT NOT NULL,
  embedding BLOB NOT NULL,
  embedding_model TEXT NOT NULL,
  embedding_contract TEXT NOT NULL,
  dimensions INTEGER NOT NULL CHECK(dimensions > 0),
  UNIQUE(asset_id, source_kind, source_id, chunk_ordinal)
);
CREATE INDEX idx_rag_chunks_asset_id ON rag_chunks(asset_id);
CREATE INDEX idx_rag_chunks_item_id ON rag_chunks(item_id);
CREATE INDEX idx_rag_chunks_embedding_contract
ON rag_chunks(embedding_model, embedding_contract, dimensions);
CREATE VIRTUAL TABLE rag_chunks_fts USING fts5(
  chunk_id UNINDEXED,
  text_content,
  tokenize = 'unicode61 remove_diacritics 1'
);
CREATE TRIGGER rag_chunks_fts_insert
AFTER INSERT ON rag_chunks
BEGIN
  INSERT INTO rag_chunks_fts(chunk_id, text_content)
  VALUES (NEW.id, NEW.text_content);
END;
CREATE TRIGGER rag_chunks_fts_delete
AFTER DELETE ON rag_chunks
BEGIN
  DELETE FROM rag_chunks_fts WHERE chunk_id = OLD.id;
END;

-- 0030_items_collection_title_index
CREATE INDEX IF NOT EXISTS idx_items_collection_title
  ON items (collection_id, title COLLATE NOCASE, id);

-- 0031_items_source_directory
-- A collection is imported one directory at a time, and the directory is the
-- unit the archivist works in: a legajo, a box, a folder of scans. Ordering the
-- whole collection by title shuffles those together, so three folders that each
-- number their pages 0001, 0002, 0003 interleave into a list where nothing sits
-- next to what it belongs with.
--
-- Both facts needed to undo that already live in every item's metadata: the
-- importer writes originalPath and importedAt for every file. Deriving them
-- into generated columns makes them indexable with no backfill, no change to
-- the import path, and nothing new to ask the user for.
--
-- These stay local on purpose. PRAGMA table_xinfo reports generated columns as
-- hidden, which is exactly how the sync layer decides what to replicate, so
-- each device recomputes them from the metadata that does replicate.
--
-- char(92) is the path separator, spelled that way so the expression survives
-- both the SQL parser and the TypeScript template literal carrying it.
--
-- The dirname idiom: rtrim(path, <every character of path except separators>)
-- eats the filename from the right and stops at the last separator.
ALTER TABLE items ADD COLUMN source_dir TEXT GENERATED ALWAYS AS (
  NULLIF(
    rtrim(
      rtrim(
        replace(json_extract(metadata, '$.__entropia_file_metadata.originalPath'), '/', char(92)),
        replace(
          replace(json_extract(metadata, '$.__entropia_file_metadata.originalPath'), '/', char(92)),
          char(92),
          ''
        )
      ),
      char(92)
    ),
    ''
  )
);

ALTER TABLE items ADD COLUMN imported_at TEXT GENERATED ALWAYS AS (
  json_extract(metadata, '$.__entropia_file_metadata.importedAt')
);

-- Paging inside one directory group, in document-name order.
CREATE INDEX IF NOT EXISTS idx_items_collection_source_dir
  ON items (collection_id, source_dir, title COLLATE NOCASE, id);

-- Ordering the groups themselves by their earliest import.
CREATE INDEX IF NOT EXISTS idx_items_collection_source_dir_import
  ON items (collection_id, source_dir, imported_at);

-- 0032_batch_processing
CREATE TABLE processing_batches (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  origin TEXT NOT NULL CHECK(origin IN ('user', 'manual', 'repair')),
  state TEXT NOT NULL CHECK(state IN ('preparing', 'ready', 'running', 'pausing', 'paused', 'cancelling', 'cancelled', 'interrupted', 'completed', 'completed_with_errors')),
  desired_state TEXT NOT NULL CHECK(desired_state IN ('run', 'pause', 'cancel')),
  operations TEXT NOT NULL,
  config_snapshot_json TEXT NOT NULL DEFAULT '{}',
  planning_cursor INTEGER NOT NULL DEFAULT 0,
  planning_done INTEGER NOT NULL DEFAULT 0 CHECK(planning_done IN (0, 1)),
  revision INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  started_at INTEGER,
  finished_at INTEGER,
  last_error TEXT
);
CREATE TABLE processing_batch_collections (
  batch_id TEXT NOT NULL REFERENCES processing_batches(id) ON DELETE CASCADE,
  collection_id_snapshot TEXT NOT NULL,
  name_snapshot TEXT NOT NULL,
  PRIMARY KEY (batch_id, collection_id_snapshot)
);
CREATE TABLE processing_batch_members (
  batch_id TEXT NOT NULL REFERENCES processing_batches(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  asset_id_snapshot TEXT NOT NULL,
  item_id_snapshot TEXT NOT NULL,
  collection_id_snapshot TEXT NOT NULL,
  title_snapshot TEXT NOT NULL DEFAULT '',
  classification TEXT NOT NULL DEFAULT 'unclassified',
  reason TEXT,
  PRIMARY KEY (batch_id, ordinal),
  UNIQUE (batch_id, asset_id_snapshot)
);
CREATE INDEX idx_processing_members_batch_asset
  ON processing_batch_members(batch_id, asset_id_snapshot);
CREATE TABLE processing_tasks (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding')),
  asset_id_snapshot TEXT NOT NULL,
  input_revision INTEGER NOT NULL DEFAULT 0,
  input_fingerprint TEXT NOT NULL DEFAULT '',
  contract_hash TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL CHECK(state IN ('pending', 'blocked', 'running', 'retry_wait', 'interrupted', 'succeeded', 'failed', 'skipped', 'cancelled')),
  stage TEXT NOT NULL DEFAULT '',
  progress_done INTEGER NOT NULL DEFAULT 0 CHECK(progress_done >= 0),
  progress_total INTEGER NOT NULL DEFAULT 0 CHECK(progress_total >= 0),
  outcome TEXT NOT NULL DEFAULT '',
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count >= 0),
  retry_cycle INTEGER NOT NULL DEFAULT 0 CHECK(retry_cycle >= 0),
  retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count >= 0),
  next_retry_at INTEGER,
  owner_session TEXT,
  lease_epoch INTEGER NOT NULL DEFAULT 0,
  heartbeat_at INTEGER,
  lease_expires_at INTEGER,
  last_error_code TEXT,
  last_error_message TEXT,
  result_receipt_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_processing_tasks_claimable
  ON processing_tasks(state, next_retry_at, id);
CREATE UNIQUE INDEX idx_processing_tasks_active_unique
  ON processing_tasks(kind, asset_id_snapshot)
  WHERE state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled');
CREATE TABLE processing_batch_tasks (
  batch_id TEXT NOT NULL REFERENCES processing_batches(id) ON DELETE CASCADE,
  task_id TEXT NOT NULL REFERENCES processing_tasks(id),
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding')),
  asset_id_snapshot TEXT NOT NULL,
  request_state TEXT NOT NULL DEFAULT 'active' CHECK(request_state IN ('active', 'paused', 'cancelled')),
  dependency_task_id TEXT REFERENCES processing_tasks(id),
  PRIMARY KEY (batch_id, task_id)
);
CREATE INDEX idx_processing_batch_tasks_task
  ON processing_batch_tasks(task_id, request_state);
CREATE INDEX idx_processing_batch_tasks_batch
  ON processing_batch_tasks(batch_id, task_id);
CREATE TABLE processing_requests (
  request_id TEXT PRIMARY KEY,
  action TEXT NOT NULL,
  batch_id TEXT REFERENCES processing_batches(id) ON DELETE CASCADE,
  payload_hash TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open', 'applied', 'rejected')),
  selection_cursor INTEGER NOT NULL DEFAULT 0,
  response_json TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE processing_attempts (
  task_id TEXT NOT NULL REFERENCES processing_tasks(id) ON DELETE CASCADE,
  attempt_number INTEGER NOT NULL CHECK(attempt_number >= 1),
  lease_epoch INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  outcome TEXT NOT NULL DEFAULT 'open' CHECK(outcome IN ('open', 'succeeded', 'failed', 'interrupted', 'cancelled')),
  retryable INTEGER NOT NULL DEFAULT 0 CHECK(retryable IN (0, 1)),
  error_code TEXT,
  error_message TEXT,
  provider_request_id TEXT,
  PRIMARY KEY (task_id, attempt_number)
);
CREATE INDEX idx_processing_attempts_task
  ON processing_attempts(task_id, attempt_number);
CREATE TABLE processing_checkpoints (
  task_id TEXT NOT NULL REFERENCES processing_tasks(id) ON DELETE CASCADE,
  unit_key TEXT NOT NULL,
  input_fingerprint TEXT NOT NULL,
  contract_hash TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  payload_checksum TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (task_id, unit_key)
);
CREATE TABLE processing_asset_revisions (
  asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  source_revision INTEGER NOT NULL DEFAULT 0,
  embedding_completed_revision INTEGER,
  invalidated_at INTEGER,
  invalidation_reason TEXT,
  auto_suppressed_revision INTEGER
);
-- Single-row liveness for the supervisor thread (scheduler_heartbeat =
-- "session-id|epoch-millis"). Recovery and claiming consult it so a
-- second process never steals units from a live scheduler, while a dead
-- scheduler's fresh-looking leases still converge on restart.
CREATE TABLE processing_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TRIGGER trg_processing_extractions_ai
AFTER INSERT ON extractions
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'extraction_insert')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_extractions_au
AFTER UPDATE OF text_content, method ON extractions
WHEN OLD.text_content IS NOT NEW.text_content OR OLD.method IS NOT NEW.method
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'extraction_update')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_extractions_ad
AFTER DELETE ON extractions
WHEN EXISTS (SELECT 1 FROM assets WHERE id = OLD.asset_id)
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (OLD.asset_id, 1, (strftime('%s', 'now') * 1000), 'extraction_delete')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_transcriptions_ai
AFTER INSERT ON transcriptions
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'transcription_insert')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_transcriptions_au
AFTER UPDATE OF text_content ON transcriptions
WHEN OLD.text_content IS NOT NEW.text_content
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'transcription_update')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_transcriptions_ad
AFTER DELETE ON transcriptions
WHEN EXISTS (SELECT 1 FROM assets WHERE id = OLD.asset_id)
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (OLD.asset_id, 1, (strftime('%s', 'now') * 1000), 'transcription_delete')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;

-- 0033_processing_source_invalidation
ALTER TABLE processing_tasks ADD COLUMN source_invalidation_count INTEGER NOT NULL DEFAULT 0;

-- 0034_fts_contentless_delete
DROP TABLE IF EXISTS fts_items;

CREATE VIRTUAL TABLE fts_items USING fts5(
  item_id UNINDEXED,
  title,
  metadata,
  extracted_text,
  tokenize='unicode61 remove_diacritics 1',
  content='',
  contentless_delete=1
);

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
FROM items i;

-- 0035_writing_workspace
-- Writing workspace (plan-editor.md §9). The canonical manuscript is the
-- ProseMirror JSON in writing_documents.current_content_json; the citation
-- tables are projections of the current revision (§8.4), never a second
-- editable source of truth.
--
-- Corpus references (collection_id, item_id, asset_id on the citation
-- projection) are plain columns WITHOUT a foreign key, matching the
-- snapshot-not-hard-link convention of entities.asset_id, triples.asset_id and
-- the processing_* family: §10.3 requires a citation to survive its source
-- being deleted, carrying its metadata snapshot and an integrity status.
--
-- The one real corpus foreign key is the document-collection association, and
-- it cascades on purpose: the corpus delete path in collection.repo.ts is
-- hand-rolled and knows nothing about these tables, so a RESTRICT here would
-- block deleting a collection.

CREATE TABLE IF NOT EXISTS writing_documents (
  id                   TEXT    PRIMARY KEY,
  title                TEXT    NOT NULL,
  document_type        TEXT    NOT NULL,
  status               TEXT    NOT NULL DEFAULT 'active'
                               CHECK (status IN ('active', 'archived', 'trashed')),
  schema_version       INTEGER NOT NULL,
  current_content_json TEXT    NOT NULL,
  revision             INTEGER NOT NULL DEFAULT 0,
  plain_text_cache     TEXT,
  citation_style_id    TEXT,
  citation_locale      TEXT,
  bibliography_enabled INTEGER NOT NULL DEFAULT 1,
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  last_opened_at       INTEGER
);

CREATE INDEX IF NOT EXISTS idx_writing_documents_status_updated
  ON writing_documents (status, updated_at DESC);

CREATE TABLE IF NOT EXISTS writing_document_collections (
  document_id   TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  collection_id TEXT    NOT NULL REFERENCES collections(id)       ON DELETE CASCADE,
  is_primary    INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  PRIMARY KEY (document_id, collection_id)
);

CREATE INDEX IF NOT EXISTS idx_writing_document_collections_collection
  ON writing_document_collections (collection_id);

-- A version must carry everything needed to reproduce its citations and
-- bibliography, not just the text (§8.4).
CREATE TABLE IF NOT EXISTS writing_document_versions (
  id                     TEXT    PRIMARY KEY,
  document_id            TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  version_number         INTEGER NOT NULL,
  content_json           TEXT    NOT NULL,
  schema_version         INTEGER NOT NULL,
  document_settings_json TEXT    NOT NULL DEFAULT '{}',
  reason                 TEXT    NOT NULL
                                 CHECK (reason IN ('auto', 'close', 'checkpoint', 'restore', 'migration')),
  content_hash           TEXT    NOT NULL,
  created_at             INTEGER NOT NULL,
  UNIQUE (document_id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_writing_document_versions_document
  ON writing_document_versions (document_id, version_number DESC);

CREATE TABLE IF NOT EXISTS writing_document_citations (
  id                     TEXT    PRIMARY KEY,
  document_id            TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  citation_node_id       TEXT    NOT NULL,
  collection_id          TEXT,
  item_id                TEXT,
  asset_id               TEXT,
  page_number            INTEGER,
  start_char             INTEGER,
  end_char               INTEGER,
  source_region_json     TEXT,
  quoted_text            TEXT,
  source_text_hash       TEXT,
  locator_json           TEXT,
  metadata_snapshot_json TEXT    NOT NULL DEFAULT '{}',
  integrity_status       TEXT    NOT NULL DEFAULT 'valid'
                                 CHECK (integrity_status IN ('valid', 'source_modified', 'source_missing', 'unverifiable')),
  created_at             INTEGER NOT NULL,
  updated_at             INTEGER NOT NULL,
  UNIQUE (document_id, citation_node_id)
);

CREATE INDEX IF NOT EXISTS idx_writing_document_citations_asset
  ON writing_document_citations (asset_id);

-- One visible citation can be a cluster of several works, so identity is
-- (cluster, position), never one row per parenthesis (§9.5).
CREATE TABLE IF NOT EXISTS writing_zotero_citations (
  id                      TEXT    PRIMARY KEY,
  document_id             TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  citation_node_id        TEXT    NOT NULL,
  citation_cluster_id     TEXT    NOT NULL,
  item_position           INTEGER NOT NULL,
  source_origin           TEXT    NOT NULL DEFAULT 'local'
                                  CHECK (source_origin IN ('local', 'web')),
  source_instance_id      TEXT,
  library_type            TEXT    NOT NULL,
  library_id              TEXT    NOT NULL,
  item_key                TEXT    NOT NULL,
  item_version            INTEGER,
  locator_type            TEXT,
  locator                 TEXT,
  prefix                  TEXT,
  suffix                  TEXT,
  suppress_author         INTEGER NOT NULL DEFAULT 0,
  author_only             INTEGER NOT NULL DEFAULT 0,
  item_csl_json_snapshot  TEXT    NOT NULL DEFAULT '{}',
  integrity_status        TEXT    NOT NULL DEFAULT 'valid'
                                  CHECK (integrity_status IN ('valid', 'zotero_unavailable', 'item_missing', 'item_modified')),
  created_at              INTEGER NOT NULL,
  updated_at              INTEGER NOT NULL,
  UNIQUE (document_id, citation_cluster_id, item_position)
);

CREATE INDEX IF NOT EXISTS idx_writing_zotero_citations_item
  ON writing_zotero_citations (library_type, library_id, item_key);

-- Append-only record of operations (§8.4): it is not a projection and is not
-- cleared when the text it describes is edited away.
CREATE TABLE IF NOT EXISTS writing_provenance_events (
  id                    TEXT    PRIMARY KEY,
  document_id           TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  version_id            TEXT    REFERENCES writing_document_versions(id) ON DELETE SET NULL,
  range_anchor_json     TEXT,
  origin_type           TEXT    NOT NULL
                                CHECK (origin_type IN ('manual', 'corpus', 'note', 'zotero', 'agent', 'import')),
  operation_type        TEXT    NOT NULL
                                CHECK (operation_type IN ('insert', 'replace', 'rewrite', 'restore', 'other')),
  source_reference_json TEXT,
  model_provider        TEXT,
  model_name            TEXT,
  prompt_template_id    TEXT,
  created_at            INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_writing_provenance_events_document
  ON writing_provenance_events (document_id, created_at DESC);

-- Pending suggestions live outside the canonical content (§8.2). The target is
-- pinned by an anchor plus a hash of the selected content, never by a hash of
-- the whole manuscript (§9.7).
CREATE TABLE IF NOT EXISTS writing_agent_suggestions (
  id                    TEXT    PRIMARY KEY,
  document_id           TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  selection_anchor_json TEXT,
  source_revision       INTEGER NOT NULL,
  selected_content_hash TEXT    NOT NULL,
  action_type           TEXT    NOT NULL,
  original_text         TEXT,
  suggested_text        TEXT,
  rationale             TEXT,
  evidence_json         TEXT    NOT NULL DEFAULT '{}',
  status                TEXT    NOT NULL DEFAULT 'pending'
                                CHECK (status IN ('pending', 'accepted', 'inserted_below', 'discarded')),
  provider              TEXT,
  model                 TEXT,
  created_at            INTEGER NOT NULL,
  resolved_at           INTEGER
);

CREATE INDEX IF NOT EXISTS idx_writing_agent_suggestions_document
  ON writing_agent_suggestions (document_id, status);

-- 0036_writing_journal
-- Durable recovery journal for in-progress manuscript edits (plan-editor.md
-- §16.1). Measured in spike S6: a ~1 KB delta costs a p95 of 1.17 ms under the
-- synchronous=FULL this repository already sets, while writing the whole
-- manuscript costs 97.5 ms. So this table stores DELTAS, never documents.
--
-- delta_json is opaque to the backend: the editor produces ProseMirror steps,
-- this table makes the sequence durable, and recovery hands it back in order to
-- be replayed. Nothing here parses it.
--
-- Persisting a journal entry is NOT a canonical save (§16.2): the UI shows
-- "Guardado" only after the content commit advances writing_documents.revision.
-- Entries stay until a canonical revision that already contains them exists,
-- which is why pruning is keyed on base_revision rather than on age.

CREATE TABLE IF NOT EXISTS writing_journal (
  document_id    TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  -- Monotonic per document. Gaps are expected: a crash between the commit and
  -- the writer's acknowledgement leaves an entry nobody was told about, which
  -- recovery must accept rather than treat as corruption.
  seq            INTEGER NOT NULL,
  -- The canonical revision this delta applies on top of. Entries below the
  -- document's current revision are already folded into the content.
  base_revision  INTEGER NOT NULL,
  schema_version INTEGER NOT NULL,
  delta_json     TEXT    NOT NULL,
  -- Guards against a truncated or corrupt entry being replayed. §16.3 requires
  -- that such an entry is never applied partially and is kept for diagnosis.
  checksum       TEXT    NOT NULL,
  created_at     INTEGER NOT NULL,
  PRIMARY KEY (document_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_writing_journal_replay
  ON writing_journal (document_id, base_revision, seq);
