-- AUTO-GENERATED schema fixture for the Rust sync tests. DO NOT EDIT BY HAND.
-- Pro schema dumped from a fully-migrated database (through 0023).

CREATE TABLE layouts (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                regions TEXT NOT NULL,
                blocks TEXT NOT NULL,
                model TEXT NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
CREATE INDEX idx_layouts_asset_id ON layouts(asset_id);
CREATE UNIQUE INDEX idx_layouts_asset_id_unique ON layouts(asset_id);
CREATE TABLE app_settings (
                        key TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    );
CREATE TABLE vec_assets(
                    asset_id TEXT PRIMARY KEY,
                    item_id TEXT NOT NULL,
                    embedding BLOB NOT NULL
                );
CREATE INDEX idx_vec_assets_item_id ON vec_assets(item_id);
CREATE TABLE _migrations (
      id    INTEGER PRIMARY KEY AUTOINCREMENT,
      name  TEXT    NOT NULL UNIQUE,
      applied_at INTEGER NOT NULL
    );
CREATE TABLE collections (
  id          TEXT    PRIMARY KEY,
  name        TEXT    NOT NULL,
  description TEXT,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE TABLE items (
  id            TEXT    PRIMARY KEY,
  title         TEXT    NOT NULL,
  collection_id TEXT    NOT NULL REFERENCES collections(id),
  metadata      TEXT,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
, search_text TEXT GENERATED ALWAYS AS (
  COALESCE(title, '') || ' ' || COALESCE(json(metadata), '')
) STORED);
CREATE TABLE assets (
  id         TEXT    PRIMARY KEY,
  item_id    TEXT    NOT NULL REFERENCES items(id),
  path       TEXT    NOT NULL,
  type       TEXT    NOT NULL,
  size       INTEGER,
  created_at INTEGER NOT NULL
, sort_index INTEGER NOT NULL DEFAULT 0);
CREATE TABLE notes (
  id         TEXT    PRIMARY KEY,
  item_id    TEXT    NOT NULL REFERENCES items(id),
  content    TEXT    NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
, asset_id TEXT);
CREATE INDEX idx_items_search ON items(search_text);
CREATE INDEX idx_items_collection ON items(collection_id);
CREATE INDEX idx_assets_item ON assets(item_id);
CREATE INDEX idx_notes_item ON notes(item_id);
CREATE TABLE extractions (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  text_content TEXT NOT NULL,
  method TEXT NOT NULL,
  confidence REAL,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_extractions_asset_id ON extractions(asset_id);
CREATE VIRTUAL TABLE fts_items
USING fts5(
  item_id UNINDEXED,
  title,
  metadata,
  extracted_text,
  tokenize='unicode61 remove_diacritics 1',
  content=''
);
CREATE TABLE triples (
  id TEXT PRIMARY KEY NOT NULL,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  subject TEXT NOT NULL,
  predicate TEXT NOT NULL,
  object TEXT NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
, asset_id TEXT);
CREATE INDEX triples_item_id_idx ON triples(item_id);
CREATE TABLE annotations (
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
CREATE INDEX annotations_asset_id_idx ON annotations(asset_id);
CREATE INDEX annotations_asset_page_idx ON annotations(asset_id, page);
CREATE TABLE transcriptions (
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
CREATE INDEX idx_transcriptions_asset_id ON transcriptions(asset_id);
CREATE TABLE "entities" (
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
, latitude REAL, longitude REAL, geo_status TEXT NOT NULL DEFAULT 'pending', asset_id TEXT);
CREATE INDEX idx_entities_item_id ON entities(item_id);
CREATE INDEX idx_entities_type ON entities(entity_type);
CREATE INDEX idx_entities_geo_status ON entities(geo_status);
CREATE INDEX idx_assets_item_sort ON assets(item_id, sort_index);
CREATE INDEX idx_notes_asset_id ON notes(asset_id);
CREATE INDEX idx_entities_asset_id ON entities(asset_id);
CREATE INDEX idx_triples_asset_id ON triples(asset_id);
CREATE TABLE topics (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL
);
CREATE TABLE item_topics (
  id TEXT PRIMARY KEY,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_item_topics_item_topic ON item_topics(item_id, topic_id);
CREATE INDEX idx_item_topics_topic_id ON item_topics(topic_id);
CREATE UNIQUE INDEX idx_extractions_asset_id_unique
ON extractions(asset_id);
CREATE UNIQUE INDEX idx_transcriptions_asset_id_unique
ON transcriptions(asset_id);
CREATE TABLE "llm_results" (
  id TEXT PRIMARY KEY,
  target_id TEXT NOT NULL,
  target_type TEXT NOT NULL CHECK(target_type IN ('asset', 'item', 'collection', 'unknown')),
  job_type TEXT NOT NULL,
  result TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_llm_results_target ON llm_results(target_id);
CREATE INDEX idx_llm_results_target_typed
ON llm_results(target_type, target_id, job_type);
CREATE TABLE rag_conversations (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE rag_messages (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES rag_conversations(id) ON DELETE CASCADE,
  sort_index INTEGER NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('user','assistant')),
  content TEXT NOT NULL,
  sources TEXT,
  model TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_rag_messages_conversation ON rag_messages(conversation_id, sort_index);
