-- 0051_bibliographic_profile_fts: lexical search over work profiles (E3c-WU3).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0051_bibliographic_profile_fts']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0051 behind.
--
-- Plan section 6 "Indices FTS bibliograficos": FTS5 over the canonical text
-- of every stored profile, maintained transactionally by triggers on the
-- profile table — a profile write and its index row commit together, and a
-- profile delete removes its index row in the same statement. The index is
-- reconstructible: deleting and re-inserting every profile row rebuilds it.
-- Retrieval ranks with bm25 and always joins back to the profile row, so a
-- lexical hit can never describe a work whose profile no longer exists.

CREATE VIRTUAL TABLE bibliographic_profile_fts USING fts5(
    item_id UNINDEXED,
    canonical_text,
    tokenize = 'unicode61 remove_diacritics 1'
);

CREATE TRIGGER bibliographic_profile_fts_insert AFTER INSERT ON bibliographic_semantic_profiles
BEGIN
    INSERT INTO bibliographic_profile_fts(item_id, canonical_text)
    VALUES (NEW.item_id, NEW.canonical_text);
END;

CREATE TRIGGER bibliographic_profile_fts_delete AFTER DELETE ON bibliographic_semantic_profiles
BEGIN
    DELETE FROM bibliographic_profile_fts WHERE item_id = OLD.item_id;
END;

CREATE TRIGGER bibliographic_profile_fts_update AFTER UPDATE OF canonical_text ON bibliographic_semantic_profiles
BEGIN
    DELETE FROM bibliographic_profile_fts WHERE item_id = OLD.item_id;
    INSERT INTO bibliographic_profile_fts(item_id, canonical_text)
    VALUES (NEW.item_id, NEW.canonical_text);
END;
