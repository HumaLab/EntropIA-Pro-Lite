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
