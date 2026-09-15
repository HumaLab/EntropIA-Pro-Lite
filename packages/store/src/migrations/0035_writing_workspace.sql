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
