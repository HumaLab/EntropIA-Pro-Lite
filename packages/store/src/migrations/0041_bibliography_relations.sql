-- E1b-1b relational Zotero catalog slice.
--
-- Native collection/tag identity is qualified by the owning library. Attachment
-- identity is qualified by its mandatory parent item. Parent collection keys are
-- opaque native values: no parent FK is required and sync order is irrelevant.
-- Tombstones are side tables so 0040 snapshots and relations stay intact.

-- Composite parent keys for the library-scoped membership foreign keys below.
CREATE UNIQUE INDEX IF NOT EXISTS idx_bibliographic_items_id_library
  ON bibliographic_items(id, library_id);

CREATE TABLE IF NOT EXISTS zotero_collections (
  id                    TEXT PRIMARY KEY,
  library_id            TEXT NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
  collection_key        TEXT NOT NULL,
  name                  TEXT NOT NULL,
  parent_collection_key TEXT,
  native_json_snapshot  TEXT NOT NULL CHECK(json_valid(native_json_snapshot)),
  native_version        INTEGER,
  revision              INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at            INTEGER NOT NULL,
  updated_at            INTEGER NOT NULL,
  verified_at           INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_zotero_collections_library_key
  ON zotero_collections(library_id, collection_key);
CREATE UNIQUE INDEX IF NOT EXISTS idx_zotero_collections_id_library
  ON zotero_collections(id, library_id);
CREATE INDEX IF NOT EXISTS idx_zotero_collections_library
  ON zotero_collections(library_id);

CREATE TABLE IF NOT EXISTS zotero_tags (
  id                    TEXT PRIMARY KEY,
  library_id            TEXT NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
  tag_text              TEXT NOT NULL,
  tag_type              TEXT,
  native_json_snapshot  TEXT NOT NULL CHECK(json_valid(native_json_snapshot)),
  native_version        INTEGER,
  revision              INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  verified_at   INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_zotero_tags_library_text
  ON zotero_tags(library_id, tag_text);
CREATE UNIQUE INDEX IF NOT EXISTS idx_zotero_tags_id_library
  ON zotero_tags(id, library_id);
CREATE INDEX IF NOT EXISTS idx_zotero_tags_library
  ON zotero_tags(library_id);

CREATE TABLE IF NOT EXISTS zotero_attachments (
  id             TEXT PRIMARY KEY,
  item_id        TEXT NOT NULL REFERENCES bibliographic_items(id) ON DELETE CASCADE,
  attachment_key TEXT NOT NULL,
  content_type   TEXT,
  link_mode      TEXT,
  filename       TEXT,
  native_path           TEXT,
  url                   TEXT,
  md5                   TEXT,
  mtime                 INTEGER,
  native_json_snapshot  TEXT NOT NULL CHECK(json_valid(native_json_snapshot)),
  native_version        INTEGER,
  revision              INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  verified_at    INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_zotero_attachments_item_key
  ON zotero_attachments(item_id, attachment_key);
CREATE INDEX IF NOT EXISTS idx_zotero_attachments_item
  ON zotero_attachments(item_id);

CREATE TABLE IF NOT EXISTS zotero_item_collections (
  library_id    TEXT NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
  item_id       TEXT NOT NULL,
  collection_id TEXT NOT NULL,
  PRIMARY KEY (library_id, item_id, collection_id),
  FOREIGN KEY (item_id, library_id)
    REFERENCES bibliographic_items(id, library_id) ON DELETE CASCADE,
  FOREIGN KEY (collection_id, library_id)
    REFERENCES zotero_collections(id, library_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_zotero_item_collections_item
  ON zotero_item_collections(library_id, item_id);
CREATE INDEX IF NOT EXISTS idx_zotero_item_collections_collection
  ON zotero_item_collections(library_id, collection_id);

CREATE TABLE IF NOT EXISTS zotero_item_tags (
  library_id TEXT NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
  item_id    TEXT NOT NULL,
  tag_id     TEXT NOT NULL,
  PRIMARY KEY (library_id, item_id, tag_id),
  FOREIGN KEY (item_id, library_id)
    REFERENCES bibliographic_items(id, library_id) ON DELETE CASCADE,
  FOREIGN KEY (tag_id, library_id)
    REFERENCES zotero_tags(id, library_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_zotero_item_tags_item
  ON zotero_item_tags(library_id, item_id);
CREATE INDEX IF NOT EXISTS idx_zotero_item_tags_tag
  ON zotero_item_tags(library_id, tag_id);

-- One replay-safe tombstone per live entity. The entity FK is deliberately the
-- only deletion path: tombstoning never deletes snapshots or membership rows.
CREATE TABLE IF NOT EXISTS zotero_item_tombstones (
  item_id        TEXT PRIMARY KEY REFERENCES bibliographic_items(id) ON DELETE CASCADE,
  observed_at    INTEGER NOT NULL,
  remote_version INTEGER,
  reason         TEXT NOT NULL CHECK(length(trim(reason)) > 0)
);

CREATE TABLE IF NOT EXISTS zotero_collection_tombstones (
  collection_id  TEXT PRIMARY KEY REFERENCES zotero_collections(id) ON DELETE CASCADE,
  observed_at    INTEGER NOT NULL,
  remote_version INTEGER,
  reason         TEXT NOT NULL CHECK(length(trim(reason)) > 0)
);

CREATE TABLE IF NOT EXISTS zotero_tag_tombstones (
  tag_id         TEXT PRIMARY KEY REFERENCES zotero_tags(id) ON DELETE CASCADE,
  observed_at    INTEGER NOT NULL,
  remote_version INTEGER,
  reason         TEXT NOT NULL CHECK(length(trim(reason)) > 0)
);

CREATE TABLE IF NOT EXISTS zotero_attachment_tombstones (
  attachment_id  TEXT PRIMARY KEY REFERENCES zotero_attachments(id) ON DELETE CASCADE,
  observed_at    INTEGER NOT NULL,
  remote_version INTEGER,
  reason         TEXT NOT NULL CHECK(length(trim(reason)) > 0)
);
