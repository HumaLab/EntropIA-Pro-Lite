-- 0064_derived_source_path: the file a layout or an extraction was computed from.
--
-- source_path:   relative path (assets/...) of the file the row was computed
--                from. An image asset changes file on every edit (crop,
--                rotate, erase, undo), so this names the version the layout
--                boxes or the text belong to.
-- source_sha256: sha256 of the raw bytes of that file as read from disk,
--                before any EXIF orientation is applied. The hash decides
--                whether a row matches the file on screen. The path explains.
--
-- NULL on both means the origin is unknown: every row from before this
-- migration, and any row a writer does not stamp. There is no backfill,
-- because SQL cannot read a file.
--
-- Additive only: two nullable columns on each of two synced tables, no table
-- rebuild. Sync carries every column of the row, so the first push from a
-- device with this migration raises the account schema_tag. Devices that have
-- not updated drop the unknown columns on apply and journal the drift.
--
-- ALTER TABLE ... ADD COLUMN has no IF NOT EXISTS form in SQLite. Like 0062
-- and 0063, this runs through the single-batch path with its _migrations row,
-- so a second runMigrations() pass skips it. Do not apply this file twice by
-- hand.

ALTER TABLE layouts ADD COLUMN source_path TEXT;
ALTER TABLE layouts ADD COLUMN source_sha256 TEXT;
ALTER TABLE extractions ADD COLUMN source_path TEXT;
ALTER TABLE extractions ADD COLUMN source_sha256 TEXT;
