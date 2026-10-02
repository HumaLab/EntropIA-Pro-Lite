-- 0052_bibliographic_chunks: structural work chunks and spans (E4c-WU1).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0052_bibliographic_chunks']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0052 behind.
--
-- One chunk row per (work, ordinal) with its text, hash, and the chunking
-- contract that produced it; one span row per page range the chunk covers,
-- with exact offsets into the page's text — including chunks that span
-- pages. Re-chunking replaces a work's set atomically (delete + insert in
-- the publisher transaction), so a top-insert that shifts ordinals never
-- leaves half-old sets behind. Chunk vectors live in their own table
-- (E4c-WU2) keyed by chunk id and generation. Managed derivatives: catalog
-- deletes cascade.

CREATE TABLE bibliographic_chunks (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES bibliographic_items(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES zotero_attachments(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    text_content TEXT NOT NULL,
    text_hash TEXT NOT NULL,
    chunking_contract TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(item_id, ordinal)
);

CREATE TABLE bibliographic_chunk_spans (
    chunk_id TEXT NOT NULL REFERENCES bibliographic_chunks(id) ON DELETE CASCADE,
    page_number INTEGER NOT NULL,
    start_char INTEGER NOT NULL,
    end_char INTEGER NOT NULL,
    PRIMARY KEY (chunk_id, page_number, start_char)
);

CREATE INDEX idx_bibliographic_chunks_item
    ON bibliographic_chunks(item_id, ordinal);
