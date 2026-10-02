-- 0044_processing_priority: per-batch interactive priority (E2c-WU3).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0044_processing_priority']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- 0032/0038/0039/0040/0041/0042/0043): the whole body goes inside one BEGIN
-- IMMEDIATE ... COMMIT together with the _migrations row, so a crash between
-- DDL and bookkeeping can never leave a half-applied 0044 behind.
--
-- E2c-WU3 is additive only: one priority column on processing_batches plus
-- an index. Existing rows default to 0 (background); no backfill, no CHECK
-- widening, no table rebuild. Claim ordering, aging, and the set-priority
-- API read this column but live in the Rust scheduler, not in this file.
--
-- Replay contract (why a second runMigrations() pass is an error-free no-op):
-- the registry row is recorded in the same atomic batch, so a second pass
-- skips this migration entirely. The statements themselves are
-- replay-tolerant where SQLite allows it:
-- - CREATE INDEX IF NOT EXISTS is a native no-op on replay;
-- - ALTER TABLE ... ADD COLUMN has no IF NOT EXISTS form in SQLite (same
--   limitation as 0011/0013/0014/0024/0026/0028/0033/0041, which rely on the
--   runner's duplicate-column tolerance). Inside the single-batch path there
--   is no per-statement rescue, so the registry skip above is what makes
--   runner-level replay error-free; do not apply this file twice by hand.

ALTER TABLE processing_batches ADD COLUMN priority INTEGER NOT NULL DEFAULT 0 CHECK(priority IN (0, 1, 2));
CREATE INDEX IF NOT EXISTS idx_processing_batches_priority
  ON processing_batches(priority, created_at, id);
