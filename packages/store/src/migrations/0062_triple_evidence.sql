-- 0062_triple_evidence: the source sentence of each extracted triple.
--
-- evidence_text:  the sentence the model built the triple from, as it returned
--                 it. NULL for triples extracted before this migration, added
--                 by hand, or produced by a custom prompt that does not ask
--                 for it.
-- evidence_start / evidence_end: UTF-16 offsets of that sentence in the text
--                 the extraction read (the asset's text at that moment), end
--                 exclusive. NULL when the sentence was not found verbatim, so
--                 a reader never highlights a wrong span. Both are set or both
--                 are NULL.
--
-- Additive only: three nullable columns on a synced table, no backfill, no
-- table rebuild. Sync carries every column of the row, so the first push from
-- a device with this migration raises the account schema_tag. Devices that
-- have not updated drop the unknown columns on apply and journal the drift.
--
-- ALTER TABLE ... ADD COLUMN has no IF NOT EXISTS form in SQLite. Like 0046,
-- this runs through the single-batch path with its _migrations row, so a
-- second runMigrations() pass skips it. Do not apply this file twice by hand.

ALTER TABLE triples ADD COLUMN evidence_text TEXT;
ALTER TABLE triples ADD COLUMN evidence_start INTEGER CHECK(evidence_start IS NULL OR evidence_start >= 0);
ALTER TABLE triples ADD COLUMN evidence_end INTEGER CHECK(evidence_end IS NULL OR (evidence_start IS NOT NULL AND evidence_end > evidence_start));
