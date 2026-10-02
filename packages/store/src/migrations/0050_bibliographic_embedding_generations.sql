-- 0050_bibliographic_embedding_generations: generation identity on work
-- embeddings (E3c-WU2).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0050_bibliographic_embedding_generations']); this file mirrors
-- it for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier processing/bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0050 behind.
--
-- Plan sections 6 and E3c demand uniqueness per object/generation: the
-- primary key moves from (item_id, embedding_contract) to
-- (item_id, generation_id), and every row names the generation it was
-- computed in. Rows already stored predate the generation lifecycle, so the
-- migration gives them honest ancestry instead of fabricating it: for each
-- distinct stored contract it registers the contract row (provider/model/
-- dimensions taken from the stored rows; chunking recorded as unknown) and
-- one retired legacy generation — retired, never queryable — then points
-- the restored rows at it. Nothing is dropped; no vector is relabeled as
-- freshly gated.
--
-- E3c-WU3 retrieval reads only vectors stamped with the active generation
-- of the query contract.

PRAGMA defer_foreign_keys=ON;

CREATE TABLE _backup_0050_item_embeddings AS SELECT * FROM bibliographic_item_embeddings;

DROP TABLE bibliographic_item_embeddings;

CREATE TABLE bibliographic_item_embeddings (
    item_id TEXT NOT NULL REFERENCES bibliographic_items(id) ON DELETE CASCADE,
    generation_id TEXT NOT NULL REFERENCES bibliographic_index_generations(id),
    embedding_contract TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    input_hash TEXT NOT NULL,
    profile_revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (item_id, generation_id)
);

CREATE INDEX idx_bibliographic_item_embeddings_hash
    ON bibliographic_item_embeddings(input_hash);

CREATE INDEX idx_bibliographic_item_embeddings_generation
    ON bibliographic_item_embeddings(generation_id, item_id);

INSERT INTO bibliographic_embedding_contracts
  (contract_hash, provider, model, dimensions, chunking_contract, created_at)
  SELECT DISTINCT embedding_contract, 'unknown', embedding_model, dimensions, '', strftime('%s', 'now')
    FROM _backup_0050_item_embeddings
    WHERE embedding_contract NOT IN (SELECT contract_hash FROM bibliographic_embedding_contracts);

INSERT INTO bibliographic_index_generations
  (id, contract_hash, status, expected_inputs, completed_inputs, created_at, retired_at)
  SELECT 'gen-legacy-' || substr(embedding_contract, 1, 12), embedding_contract, 'retired',
         COUNT(*), COUNT(*), strftime('%s', 'now') * 1000, strftime('%s', 'now') * 1000
    FROM _backup_0050_item_embeddings
   GROUP BY embedding_contract;

INSERT INTO bibliographic_item_embeddings
  (item_id, generation_id, embedding_contract, embedding_model, dimensions, embedding,
   input_hash, profile_revision, created_at, updated_at)
  SELECT item_id, 'gen-legacy-' || substr(embedding_contract, 1, 12), embedding_contract,
         embedding_model, dimensions, embedding, input_hash, profile_revision, created_at, updated_at
    FROM _backup_0050_item_embeddings;

DROP TABLE _backup_0050_item_embeddings;
