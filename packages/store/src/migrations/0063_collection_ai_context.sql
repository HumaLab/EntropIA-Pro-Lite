-- 0063_collection_ai_context: a project context the AI reads for a collection.
--
-- ai_context: free text the person writes for a collection (topic, period,
--             conventions, spellings of names). Prepended to the RAG chat, the
--             investigation, OCR correction and the writing agent (a
--             manuscript uses its primary collection's). NULL means no
--             context, and every prompt stays as it was before.
--
-- Additive only: one nullable column on a synced table, no backfill, no
-- table rebuild. Sync carries every column of the row, so the first push from
-- a device with this migration raises the account schema_tag. Devices that
-- have not updated drop the unknown column on apply and journal the drift.
--
-- ALTER TABLE ... ADD COLUMN has no IF NOT EXISTS form in SQLite. Like 0062,
-- this runs through the single-batch path with its _migrations row, so a
-- second runMigrations() pass skips it. Do not apply this file twice by hand.

ALTER TABLE collections ADD COLUMN ai_context TEXT;
