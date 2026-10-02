-- Web sources and their captures (Navegador, docs/navegador/plan.md section 5).
--
-- A web source is one page the person saved from the in-app browser. It
-- belongs to no collection and no library, so there is no corpus foreign key.
-- A capture is one saved version of it: a page snapshot, a selection or a PDF.
-- Captures never change once written; a new capture is a new row, so earlier
-- evidence is never overwritten.
--
-- Time columns: accessed_at and first_accessed_at are provenance, kept as the
-- RFC 3339 UTC text the person saw when they saved (the form CSL and Zotero
-- take as "accessed"). created_at and updated_at follow the rest of the
-- archive: epoch milliseconds.
--
-- Files live under <data>/web-captures/<web_source_id>/<capture_id>.<ext>;
-- rel_path and text_rel_path hold the relative key, never an absolute path.
-- Text up to 512 KB stays in text; larger text goes to text_rel_path.
--
-- Local only for now: nothing here is in the sync set.

CREATE TABLE IF NOT EXISTS web_sources (
  id                TEXT    PRIMARY KEY,
  original_url      TEXT    NOT NULL,
  final_url         TEXT    NOT NULL,
  canonical_url     TEXT,
  title             TEXT,
  site_name         TEXT,
  first_accessed_at TEXT    NOT NULL,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);

-- Not unique on purpose: two devices may each save the same page before they
-- meet, and sync must be able to hold both.
CREATE INDEX IF NOT EXISTS idx_web_sources_final_url ON web_sources (final_url);
CREATE INDEX IF NOT EXISTS idx_web_sources_updated ON web_sources (updated_at DESC);

CREATE TABLE IF NOT EXISTS web_captures (
  id                TEXT    PRIMARY KEY,
  web_source_id     TEXT    NOT NULL REFERENCES web_sources(id) ON DELETE CASCADE,
  accessed_at       TEXT    NOT NULL,
  final_url         TEXT    NOT NULL,
  kind              TEXT    NOT NULL CHECK (kind IN ('page', 'selection', 'pdf')),
  mime_type         TEXT    NOT NULL,
  text              TEXT    CHECK (text IS NULL OR length(CAST(text AS BLOB)) <= 524288),
  text_rel_path     TEXT,
  quote_prefix      TEXT,
  quote_suffix      TEXT,
  rel_path          TEXT,
  sha256            TEXT    NOT NULL,
  hash_of           TEXT    NOT NULL CHECK (hash_of IN ('html', 'quote', 'pdf')),
  size_bytes        INTEGER NOT NULL,
  extractor_version TEXT,
  title             TEXT,
  created_at        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_web_captures_source ON web_captures (web_source_id, accessed_at DESC);
