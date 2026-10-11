CREATE INDEX IF NOT EXISTS idx_items_collection_title
  ON items (collection_id, title COLLATE NOCASE, id);
