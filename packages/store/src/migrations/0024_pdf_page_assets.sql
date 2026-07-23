-- Page children are additive: legacy assets remain standalone and keep their
-- existing aggregate OCR/layout rows. New GLM PDF pages point at their source.
ALTER TABLE assets ADD COLUMN parent_asset_id TEXT REFERENCES assets(id) ON DELETE CASCADE;
ALTER TABLE assets ADD COLUMN page_number INTEGER;
CREATE INDEX IF NOT EXISTS idx_assets_parent_asset_id ON assets(parent_asset_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_assets_parent_page
ON assets(parent_asset_id, page_number)
WHERE parent_asset_id IS NOT NULL;
