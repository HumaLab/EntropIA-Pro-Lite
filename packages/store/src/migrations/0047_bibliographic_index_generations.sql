-- 0047_bibliographic_index_generations: immutable embedding contracts and
-- index generations with a single-global-active pointer (E3c-WU1).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0047_bibliographic_index_generations']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier processing/bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0047 behind.
--
-- Plan sections 6 and E3c: one immutable row names a vector space
-- (provider, model, dimensions, chunking — the resolution inputs of
-- resolve_effective_embedding_contract); each generation builds vectors for
-- exactly that space. expected_inputs is the manifest of eligible works a
-- generation must complete before it may become active; completed_inputs is
-- its progress. Exactly one generation may be active at a time (partial
-- unique index): the queryable space is singular, so equal dimensions never
-- get compared across spaces. Retiring the active generation with no
-- replacement is legitimate — retrieval then serves the labeled lexical
-- fallback until a new generation completes.

CREATE TABLE bibliographic_embedding_contracts (
    contract_hash TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    chunking_contract TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE bibliographic_index_generations (
    id TEXT PRIMARY KEY,
    contract_hash TEXT NOT NULL REFERENCES bibliographic_embedding_contracts(contract_hash),
    status TEXT NOT NULL CHECK(status IN ('staging', 'active', 'retired')),
    expected_inputs INTEGER NOT NULL DEFAULT 0,
    completed_inputs INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    activated_at INTEGER,
    retired_at INTEGER
);

CREATE UNIQUE INDEX idx_bibliographic_generations_single_active
    ON bibliographic_index_generations(contract_hash)
    WHERE status = 'active';

CREATE INDEX idx_bibliographic_generations_contract
    ON bibliographic_index_generations(contract_hash, status);
