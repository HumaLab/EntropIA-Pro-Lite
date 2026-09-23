-- 0054_bibliographic_ingest_operations: durable pending tray (E5a-WU1).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0054_bibliographic_ingest_operations']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0054 behind.
--
-- One row per explicit user decision: link an existing work or create a
-- parent (and eventually upload an attachment) in one library. `request_id`
-- is the idempotency key — double-clicks record exactly one operation.
-- Receipts carry the verified Zotero identity (item key/version) that E5c
-- gates processing demand on. Recovery parks `running` rows back to
-- `queued`: every transport op is idempotent by request or by key, so a
-- retry after a crash never duplicates a record.

CREATE TABLE bibliographic_ingest_operations (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('link_match', 'create_parent', 'upload_attachment')),
    library_id TEXT NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    state TEXT NOT NULL CHECK(state IN ('queued', 'running', 'blocked', 'succeeded', 'failed', 'cancelled')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    receipt_json TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_bibliographic_ingest_operations_state
    ON bibliographic_ingest_operations(state, library_id);
