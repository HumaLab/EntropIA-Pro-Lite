-- 0045_bibliographic_semantic_profiles: per-work semantic profiles (E3b-WU1).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0045_bibliographic_semantic_profiles']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- 0032/0038/0039/0040/0041/0042/0043/0044): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0045 behind.
--
-- Plan §6 bibliographic_semantic_profiles: one row per verified work —
-- profile revision, template version, canonical text, field provenance, and
-- the input hash. Keyed by the internal bibliographic_items.id; a catalog
-- row delete cascades (profiles are reconstructible, citations never touch
-- them). No vector columns live here: embeddings get their own tables with
-- contract/generation identity (E3c), so changing models never rewrites
-- profile history.

CREATE TABLE bibliographic_semantic_profiles (
    item_id TEXT PRIMARY KEY NOT NULL REFERENCES bibliographic_items(id) ON DELETE CASCADE,
    profile_revision INTEGER NOT NULL,
    template_version TEXT NOT NULL,
    canonical_text TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    field_provenance_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_bibliographic_semantic_profiles_hash
    ON bibliographic_semantic_profiles(input_hash);
