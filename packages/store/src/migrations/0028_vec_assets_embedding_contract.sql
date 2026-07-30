ALTER TABLE vec_assets ADD COLUMN embedding_model TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE vec_assets ADD COLUMN embedding_contract TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE vec_assets ADD COLUMN dimensions INTEGER NOT NULL DEFAULT 0;
