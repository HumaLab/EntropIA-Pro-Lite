-- A read-only window onto the terms fts_items holds, with how many documents
-- hold each one. Approximate search draws its variants from here, so it never
-- suggests a word no document contains (see src/fuzzy.ts).
--
-- fts5vocab stores nothing of its own: it reads the index directly, so it
-- needs no rebuild when fts_items changes, and it survives 0034 dropping and
-- recreating fts_items because it resolves the table by name on every read.
CREATE VIRTUAL TABLE IF NOT EXISTS fts_items_vocab USING fts5vocab(fts_items, 'row');
