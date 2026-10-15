-- Add sort_index to assets for stable page ordering (e.g. scanned PDF pages)
ALTER TABLE assets ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_assets_item_sort ON assets(item_id, sort_index);
