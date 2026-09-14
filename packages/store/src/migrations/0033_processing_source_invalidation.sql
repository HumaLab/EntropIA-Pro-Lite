-- 0033_processing_source_invalidation: durable per-cycle source-invalidation
-- counter (plan-lote.md §3.3 — the third mid-flight input change blocks the
-- task as source_unstable instead of recomputing against a moving source).
--
-- Runtime source of truth is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0033_processing_source_invalidation']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical. Plain single statement — no triggers, no special batch path.
--

ALTER TABLE processing_tasks ADD COLUMN source_invalidation_count INTEGER NOT NULL DEFAULT 0;
