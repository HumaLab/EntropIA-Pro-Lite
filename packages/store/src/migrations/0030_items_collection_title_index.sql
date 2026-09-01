-- Covers the collection card query end to end: the collection_id filter, the
-- ORDER BY title COLLATE NOCASE, id that every card list uses, and the keyset
-- cursor built on that same pair. Without the sort columns in the index SQLite
-- filters through idx_items_collection and then sorts the whole result set in a
-- temp b-tree on every collection open.
CREATE INDEX IF NOT EXISTS idx_items_collection_title
  ON items (collection_id, title COLLATE NOCASE, id);
