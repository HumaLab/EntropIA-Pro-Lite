-- E1b-1a bibliography catalog foundation.
--
-- Connections are namespaces for a Zotero source. A nullable source_instance_id
-- is intentional: an uncorroborated instance must not be invented from a
-- Last-Modified-Version value or silently merged with another connection.
-- Collections, tags, attachments, tombstones and reconciliation state arrive in
-- later migrations.

CREATE TABLE IF NOT EXISTS zotero_connections (
  id                  TEXT PRIMARY KEY,
  source_origin       TEXT NOT NULL CHECK(source_origin IN ('local', 'web')),
  source_instance_id  TEXT,
  endpoint            TEXT,
  capabilities_json   TEXT NOT NULL DEFAULT '{}',
  credential_ref      TEXT,
  state               TEXT NOT NULL DEFAULT 'unknown'
                      CHECK(state IN ('unknown', 'available', 'unavailable', 'disabled', 'error')),
  revision            INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  CHECK(json_valid(capabilities_json))
);

CREATE INDEX IF NOT EXISTS idx_zotero_connections_source
  ON zotero_connections(source_origin, source_instance_id);

CREATE TABLE IF NOT EXISTS zotero_libraries (
  id                       TEXT PRIMARY KEY,
  connection_id            TEXT NOT NULL REFERENCES zotero_connections(id) ON DELETE CASCADE,
  library_type             TEXT NOT NULL CHECK(library_type IN ('user', 'group')),
  library_id               TEXT NOT NULL,
  name                     TEXT NOT NULL,
  last_modified_version    INTEGER,
  revision                 INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at               INTEGER NOT NULL,
  updated_at               INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_zotero_libraries_identity
  ON zotero_libraries(connection_id, library_type, library_id);
CREATE INDEX IF NOT EXISTS idx_zotero_libraries_connection
  ON zotero_libraries(connection_id);

CREATE TABLE IF NOT EXISTS bibliographic_items (
  id                    TEXT PRIMARY KEY,
  library_id            TEXT NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
  item_key              TEXT NOT NULL,
  item_version          INTEGER,
  native_json_snapshot  TEXT NOT NULL CHECK(json_valid(native_json_snapshot)),
  csl_json_snapshot     TEXT NOT NULL CHECK(json_valid(csl_json_snapshot)),
  item_type             TEXT,
  title                 TEXT,
  creators_json         TEXT CHECK(creators_json IS NULL OR json_valid(creators_json)),
  publication_title     TEXT,
  publisher             TEXT,
  date                  TEXT,
  doi                   TEXT,
  isbn                  TEXT,
  abstract              TEXT,
  language              TEXT,
  url                   TEXT,
  revision              INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at            INTEGER NOT NULL,
  updated_at            INTEGER NOT NULL,
  verified_at           INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_bibliographic_items_library_key
  ON bibliographic_items(library_id, item_key);
CREATE INDEX IF NOT EXISTS idx_bibliographic_items_key
  ON bibliographic_items(item_key);
CREATE INDEX IF NOT EXISTS idx_bibliographic_items_title
  ON bibliographic_items(title COLLATE NOCASE);
