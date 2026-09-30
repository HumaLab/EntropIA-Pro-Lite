-- Index the source path the import duplicate check looks up for every file
-- (ItemRepo.findImportedFromSource). Without it each file scanned and parsed
-- the metadata JSON of every item in the collection: 91 ms per file at 40k
-- documents, so importing 1000 more files spent 1.5 min just checking.
--
-- The expression must match the query exactly for SQLite to use the index.
CREATE INDEX IF NOT EXISTS idx_items_import_source
  ON items(collection_id, lower(json_extract(metadata, '$.__entropia_file_metadata.originalPath')));
