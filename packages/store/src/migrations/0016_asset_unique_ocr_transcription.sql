-- Enforce one extraction/transcription row per asset to enable true UPSERT.
-- Keep the most recent row (largest rowid) if any legacy duplicates exist.
DELETE FROM extractions
WHERE rowid NOT IN (
  SELECT MAX(rowid) FROM extractions GROUP BY asset_id
);

DELETE FROM transcriptions
WHERE rowid NOT IN (
  SELECT MAX(rowid) FROM transcriptions GROUP BY asset_id
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_extractions_asset_id_unique
ON extractions(asset_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_transcriptions_asset_id_unique
ON transcriptions(asset_id);
