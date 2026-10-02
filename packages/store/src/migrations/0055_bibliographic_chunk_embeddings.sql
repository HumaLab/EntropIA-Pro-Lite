-- 0055_bibliographic_chunk_embeddings: chunk vectors per generation (E4c-WU2).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0055_bibliographic_chunk_embeddings']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0055 behind.
--
-- One vector per (chunk, generation) under the effective embedding
-- contract, stamping the chunk text hash it was computed from — the same
-- identity shape as work embeddings (plan section 6: FK to chunk,
-- contract/generation, vector, dimension, input hash, date; uniqueness per
-- object/generation). Re-chunking deletes a work's chunks and the vectors
-- cascade; re-embedding a live chunk upserts its generation row.
-- Retrieval (E4d) reads only the active generation of the query contract.

CREATE TABLE bibliographic_chunk_embeddings (
    chunk_id TEXT NOT NULL REFERENCES bibliographic_chunks(id) ON DELETE CASCADE,
    generation_id TEXT NOT NULL REFERENCES bibliographic_index_generations(id),
    embedding_contract TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    input_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (chunk_id, generation_id)
);

CREATE INDEX idx_bibliographic_chunk_embeddings_generation
    ON bibliographic_chunk_embeddings(generation_id, chunk_id);

CREATE INDEX idx_bibliographic_chunk_embeddings_hash
    ON bibliographic_chunk_embeddings(input_hash);
