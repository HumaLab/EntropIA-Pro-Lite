-- Add asset_id to notes, entities, and triples for per-page scoping.
-- Nullable: legacy rows without asset_id are considered "item-level" (shown on all pages).
ALTER TABLE notes ADD COLUMN asset_id TEXT;
ALTER TABLE entities ADD COLUMN asset_id TEXT;
ALTER TABLE triples ADD COLUMN asset_id TEXT;
CREATE INDEX IF NOT EXISTS idx_notes_asset_id ON notes(asset_id);
CREATE INDEX IF NOT EXISTS idx_entities_asset_id ON entities(asset_id);
CREATE INDEX IF NOT EXISTS idx_triples_asset_id ON triples(asset_id);
