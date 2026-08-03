CREATE TABLE rag_chunks (
  id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  source_kind TEXT NOT NULL CHECK(source_kind IN ('extraction', 'transcription')),
  source_id TEXT NOT NULL,
  chunk_ordinal INTEGER NOT NULL CHECK(chunk_ordinal >= 0),
  text_content TEXT NOT NULL,
  start_char INTEGER NOT NULL CHECK(start_char >= 0),
  end_char INTEGER NOT NULL CHECK(end_char > start_char),
  source_text_hash TEXT NOT NULL,
  chunking_contract TEXT NOT NULL,
  embedding BLOB NOT NULL,
  embedding_model TEXT NOT NULL,
  embedding_contract TEXT NOT NULL,
  dimensions INTEGER NOT NULL CHECK(dimensions > 0),
  UNIQUE(asset_id, source_kind, source_id, chunk_ordinal)
);
CREATE INDEX idx_rag_chunks_asset_id ON rag_chunks(asset_id);
CREATE INDEX idx_rag_chunks_item_id ON rag_chunks(item_id);
CREATE INDEX idx_rag_chunks_embedding_contract
ON rag_chunks(embedding_model, embedding_contract, dimensions);
CREATE VIRTUAL TABLE rag_chunks_fts USING fts5(
  chunk_id UNINDEXED,
  text_content,
  tokenize = 'unicode61 remove_diacritics 1'
);
CREATE TRIGGER rag_chunks_fts_insert
AFTER INSERT ON rag_chunks
BEGIN
  INSERT INTO rag_chunks_fts(chunk_id, text_content)
  VALUES (NEW.id, NEW.text_content);
END;
CREATE TRIGGER rag_chunks_fts_delete
AFTER DELETE ON rag_chunks
BEGIN
  DELETE FROM rag_chunks_fts WHERE chunk_id = OLD.id;
END;
