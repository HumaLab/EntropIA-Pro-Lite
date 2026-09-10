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
