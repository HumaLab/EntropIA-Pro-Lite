-- 0066_transcription_versions: the transcripts a re-transcription replaced.
--
-- `transcriptions` keeps one row per asset (UNIQUE asset_id) and every reader
-- relies on that, so the replaced transcript moves here instead: the Rust
-- writer copies the current row in the same transaction that overwrites it.
-- text_content keeps the speaker names the user typed (renames live in the
-- text), segments keeps the engine's timing and speaker labels.
--
-- transcribed_at: created_at of the replaced row (when that transcript was
--                 made). replaced_at: when it stopped being the current one.
--
-- Additive only: one new table, no change to any existing table. Local to the
-- device: sync does NOT carry this table (it is not in SYNCED_TABLES), so a
-- version exists only on the device where the replacement happened. Like every
-- migration, the first push from an updated device raises the account
-- schema_tag.
--
-- Runs through the single-batch path with its _migrations row (same as 0062
-- to 0065), so a second runMigrations() pass skips it. Do not apply this file
-- twice by hand.

CREATE TABLE IF NOT EXISTS transcription_versions (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  text_content TEXT NOT NULL,
  language TEXT,
  duration_ms INTEGER,
  model TEXT,
  segments TEXT,
  confidence REAL,
  transcribed_at INTEGER NOT NULL,
  replaced_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_transcription_versions_asset
  ON transcription_versions(asset_id, replaced_at);
