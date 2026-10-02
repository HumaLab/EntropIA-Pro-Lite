-- 0053_bibliographic_page_texts: per-page native texts (E4b-WU2).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0053_bibliographic_page_texts']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0053 behind.
--
-- One row per (attachment, 1-based page): the native text layer of exactly
-- that page with its own hash and quality verdict, so E4b-WU3's selective
-- OCR can skip rich pages and supplement sparse ones without re-reading
-- the file. The method CHECK already admits 'ocr' rows — nothing writes
-- them yet; the selective pass arrives in E4b-WU3 and documents its own
-- writes. The 'unreadable' quality is for pages no engine could read
-- (E4b-WU4 owns those states); the native fill stores it when lopdf
-- cannot decode a page it counted.
--
-- Managed derivatives like the whole-document row: a catalog row delete
-- cascades.

CREATE TABLE bibliographic_page_texts (
    attachment_id TEXT NOT NULL REFERENCES zotero_attachments(id) ON DELETE CASCADE,
    page_number INTEGER NOT NULL CHECK(page_number >= 1),
    method TEXT NOT NULL CHECK(method IN ('native', 'ocr')),
    text_content TEXT NOT NULL,
    text_hash TEXT NOT NULL,
    text_chars INTEGER NOT NULL,
    quality TEXT NOT NULL CHECK(quality IN ('rich', 'sparse', 'empty', 'unreadable')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (attachment_id, page_number)
);

CREATE INDEX idx_bibliographic_page_texts_attachment
    ON bibliographic_page_texts(attachment_id, page_number);
