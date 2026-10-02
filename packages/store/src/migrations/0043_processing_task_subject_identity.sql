-- 0043_processing_task_subject_identity: additive task-subject identity columns (E2a-1).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0043_processing_task_subject_identity']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- 0032/0040/0041/0042): the whole body goes inside one BEGIN IMMEDIATE ...
-- COMMIT together with the _migrations row, so a crash between DDL and
-- bookkeeping can never leave a half-applied 0043 behind.
--
-- E2a-1 is additive only: every lookup keeps resolving on
-- (kind, asset_id_snapshot) and the old partial unique
-- idx_processing_tasks_active_unique stays until the E2a-2 cutover. The new
-- composite partial unique idx_processing_tasks_subject_active_unique is
-- built now so both indexes can be observed side by side before the cutover.
-- Documentary subject identity is corpus/asset keyed by the snapshot:
-- subject_id == asset_id_snapshot via dual-write (never dropped), and the
-- backfill below only writes subject_id — it must never rewrite
-- input_fingerprint/contract_hash.
--
-- Statement order matters: the backfill runs BEFORE the unique index is
-- created. Freshly added columns default every existing row to subject_id =
-- '', and several live tasks would share that value; creating the unique
-- index first would fail on those duplicates. After the backfill each live
-- row carries its own snapshot, and the pre-existing
-- idx_processing_tasks_active_unique already guarantees (kind, snapshot) is
-- unique over live rows, so index creation cannot fail on upgraded data.
--
-- Replay contract (why a second runMigrations() pass is an error-free no-op):
-- the registry row is recorded in the same atomic batch, so a second pass
-- skips this migration entirely. The statements themselves are
-- replay-tolerant where SQLite allows it:
-- - CREATE UNIQUE INDEX IF NOT EXISTS is a native no-op on replay;
-- - both backfill UPDATEs only touch rows with subject_id = '', so a replay
--   updates zero rows;
-- - ALTER TABLE ... ADD COLUMN has no IF NOT EXISTS form in SQLite (same
--   limitation as 0011/0013/0014/0024/0026/0028/0033, which rely on the
--   runner's duplicate-column tolerance). Inside the single-batch path there
--   is no per-statement rescue, so the registry skip above is what makes
--   runner-level replay error-free; do not apply this file twice by hand.

ALTER TABLE processing_tasks ADD COLUMN domain TEXT NOT NULL DEFAULT 'corpus' CHECK(domain IN ('corpus', 'bibliography'));
ALTER TABLE processing_tasks ADD COLUMN subject_kind TEXT NOT NULL DEFAULT 'asset' CHECK(subject_kind IN ('asset', 'library', 'item', 'attachment', 'page_range'));
ALTER TABLE processing_tasks ADD COLUMN subject_id TEXT NOT NULL DEFAULT '';
ALTER TABLE processing_batch_tasks ADD COLUMN domain TEXT NOT NULL DEFAULT 'corpus' CHECK(domain IN ('corpus', 'bibliography'));
ALTER TABLE processing_batch_tasks ADD COLUMN subject_kind TEXT NOT NULL DEFAULT 'asset' CHECK(subject_kind IN ('asset', 'library', 'item', 'attachment', 'page_range'));
ALTER TABLE processing_batch_tasks ADD COLUMN subject_id TEXT NOT NULL DEFAULT '';
UPDATE processing_tasks SET subject_id = asset_id_snapshot WHERE subject_id = '';
UPDATE processing_batch_tasks SET subject_id = asset_id_snapshot WHERE subject_id = '';
CREATE UNIQUE INDEX IF NOT EXISTS idx_processing_tasks_subject_active_unique
  ON processing_tasks(domain, subject_kind, subject_id, kind)
  WHERE state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled');
