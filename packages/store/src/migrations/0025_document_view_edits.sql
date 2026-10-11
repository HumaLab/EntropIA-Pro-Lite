-- Rebuild annotations so PDF page edits can be persisted without modifying the
-- source document. Existing rectangle/underline rows retain their ids and data.
CREATE TABLE annotations_v2 (
  id TEXT PRIMARY KEY NOT NULL,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  page INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL CHECK(kind IN ('rectangle', 'underline', 'crop', 'erase', 'rotation')),
  color TEXT NOT NULL,
  x REAL NOT NULL,
  y REAL NOT NULL,
  width REAL NOT NULL,
  height REAL NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
INSERT INTO annotations_v2
SELECT id, asset_id, page, kind, color, x, y, width, height, created_at, updated_at
FROM annotations;
DROP TABLE annotations;
ALTER TABLE annotations_v2 RENAME TO annotations;
CREATE INDEX annotations_asset_id_idx ON annotations(asset_id);
CREATE INDEX annotations_asset_page_idx ON annotations(asset_id, page);
