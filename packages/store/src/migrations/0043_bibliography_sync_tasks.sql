-- 0043_bibliography_sync_tasks: bibliographic task admission and system-batch origin (E2b-1).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0043_bibliography_sync_tasks']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- 0032/0038/0039/0040/0041/0042): the whole body goes inside one BEGIN IMMEDIATE ...
-- COMMIT together with the _migrations row, so a crash between DDL and
-- bookkeeping can never leave a half-applied 0043 behind.
--
-- SQLite cannot ALTER a CHECK, so widening kind/origin requires rebuilding the
-- three tables: processing_tasks and processing_batch_tasks gain
-- kind='bibliography_sync', processing_batches gains origin='bibliography'.
-- All columns, both task indexes (partial composite subject unique +
-- claimable), every FK, and every row are preserved byte-identically; old
-- kinds ('ocr', 'embedding') and origins ('user', 'manual', 'repair') are
-- unchanged, and the snapshot-scoped unique dropped in the 0042 cutover stays
-- dropped (only claimable + composite subject unique are rebuilt).
--
-- DROP order is child-first per the actual FK direction (same order as
-- PROCESSING_0032_TABLES_CHILD_FIRST in runner.ts): processing_batch_tasks is
-- the child of processing_tasks (task_id and dependency_task_id reference
-- tasks.id) and of processing_batches (batch_id references batches.id);
-- processing_attempts and processing_checkpoints are children of tasks;
-- processing_batch_collections, processing_batch_members and
-- processing_requests are children of batches. Dependents are backed up with
-- CREATE TABLE _backup_... AS SELECT * (plain data copies, no FKs), dropped
-- child-first so no parent DROP ever meets an inbound FK, then recreated
-- parent-first with data restored parent-first so immediate FK checks pass.
-- Only the three target tables change shape; the five dependents are
-- recreated from their 0032 DDL verbatim. A parent DROP with CASCADE children
-- still present would delete their rows, which is why every dependent is
-- backed up and dropped first instead of relying on deferral.
--
-- Replay contract (why a second runMigrations() pass is an error-free no-op):
-- the registry row is recorded in the same atomic batch, so a second pass
-- skips this migration entirely. The statements themselves are replay-tolerant
-- by hand too: backups are created and dropped inside the same transaction,
-- so a manual second run rebuilds the already-widened tables to the same
-- shape with the same rows; do not apply this file twice by hand outside a
-- transaction (harmless, but the runner owns replay).

PRAGMA defer_foreign_keys=ON;

CREATE TABLE _backup_0043_processing_batches AS SELECT * FROM processing_batches;
CREATE TABLE _backup_0043_processing_batch_collections AS SELECT * FROM processing_batch_collections;
CREATE TABLE _backup_0043_processing_batch_members AS SELECT * FROM processing_batch_members;
CREATE TABLE _backup_0043_processing_tasks AS SELECT * FROM processing_tasks;
CREATE TABLE _backup_0043_processing_batch_tasks AS SELECT * FROM processing_batch_tasks;
CREATE TABLE _backup_0043_processing_requests AS SELECT * FROM processing_requests;
CREATE TABLE _backup_0043_processing_attempts AS SELECT * FROM processing_attempts;
CREATE TABLE _backup_0043_processing_checkpoints AS SELECT * FROM processing_checkpoints;

DROP TABLE processing_checkpoints;
DROP TABLE processing_attempts;
DROP TABLE processing_requests;
DROP TABLE processing_batch_tasks;
DROP TABLE processing_tasks;
DROP TABLE processing_batch_members;
DROP TABLE processing_batch_collections;
DROP TABLE processing_batches;

CREATE TABLE processing_batches (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  origin TEXT NOT NULL CHECK(origin IN ('user', 'manual', 'repair', 'bibliography')),
  state TEXT NOT NULL CHECK(state IN ('preparing', 'ready', 'running', 'pausing', 'paused', 'cancelling', 'cancelled', 'interrupted', 'completed', 'completed_with_errors')),
  desired_state TEXT NOT NULL CHECK(desired_state IN ('run', 'pause', 'cancel')),
  operations TEXT NOT NULL,
  config_snapshot_json TEXT NOT NULL DEFAULT '{}',
  planning_cursor INTEGER NOT NULL DEFAULT 0,
  planning_done INTEGER NOT NULL DEFAULT 0 CHECK(planning_done IN (0, 1)),
  revision INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  started_at INTEGER,
  finished_at INTEGER,
  last_error TEXT
);
CREATE TABLE processing_batch_collections (
  batch_id TEXT NOT NULL REFERENCES processing_batches(id) ON DELETE CASCADE,
  collection_id_snapshot TEXT NOT NULL,
  name_snapshot TEXT NOT NULL,
  PRIMARY KEY (batch_id, collection_id_snapshot)
);
CREATE TABLE processing_batch_members (
  batch_id TEXT NOT NULL REFERENCES processing_batches(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  asset_id_snapshot TEXT NOT NULL,
  item_id_snapshot TEXT NOT NULL,
  collection_id_snapshot TEXT NOT NULL,
  title_snapshot TEXT NOT NULL DEFAULT '',
  classification TEXT NOT NULL DEFAULT 'unclassified',
  reason TEXT,
  PRIMARY KEY (batch_id, ordinal),
  UNIQUE (batch_id, asset_id_snapshot)
);
CREATE INDEX idx_processing_members_batch_asset
  ON processing_batch_members(batch_id, asset_id_snapshot);
CREATE TABLE processing_tasks (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding', 'bibliography_sync')),
  asset_id_snapshot TEXT NOT NULL,
  input_revision INTEGER NOT NULL DEFAULT 0,
  input_fingerprint TEXT NOT NULL DEFAULT '',
  contract_hash TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL CHECK(state IN ('pending', 'blocked', 'running', 'retry_wait', 'interrupted', 'succeeded', 'failed', 'skipped', 'cancelled')),
  stage TEXT NOT NULL DEFAULT '',
  progress_done INTEGER NOT NULL DEFAULT 0 CHECK(progress_done >= 0),
  progress_total INTEGER NOT NULL DEFAULT 0 CHECK(progress_total >= 0),
  outcome TEXT NOT NULL DEFAULT '',
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count >= 0),
  retry_cycle INTEGER NOT NULL DEFAULT 0 CHECK(retry_cycle >= 0),
  retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count >= 0),
  next_retry_at INTEGER,
  owner_session TEXT,
  lease_epoch INTEGER NOT NULL DEFAULT 0,
  heartbeat_at INTEGER,
  lease_expires_at INTEGER,
  last_error_code TEXT,
  last_error_message TEXT,
  result_receipt_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  source_invalidation_count INTEGER NOT NULL DEFAULT 0,
  domain TEXT NOT NULL DEFAULT 'corpus' CHECK(domain IN ('corpus', 'bibliography')),
  subject_kind TEXT NOT NULL DEFAULT 'asset' CHECK(subject_kind IN ('asset', 'library', 'item', 'attachment', 'page_range')),
  subject_id TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_processing_tasks_claimable
  ON processing_tasks(state, next_retry_at, id);
CREATE UNIQUE INDEX idx_processing_tasks_subject_active_unique
  ON processing_tasks(domain, subject_kind, subject_id, kind)
  WHERE state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled');
CREATE TABLE processing_batch_tasks (
  batch_id TEXT NOT NULL REFERENCES processing_batches(id) ON DELETE CASCADE,
  task_id TEXT NOT NULL REFERENCES processing_tasks(id),
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding', 'bibliography_sync')),
  asset_id_snapshot TEXT NOT NULL,
  request_state TEXT NOT NULL DEFAULT 'active' CHECK(request_state IN ('active', 'paused', 'cancelled')),
  dependency_task_id TEXT REFERENCES processing_tasks(id),
  domain TEXT NOT NULL DEFAULT 'corpus' CHECK(domain IN ('corpus', 'bibliography')),
  subject_kind TEXT NOT NULL DEFAULT 'asset' CHECK(subject_kind IN ('asset', 'library', 'item', 'attachment', 'page_range')),
  subject_id TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (batch_id, task_id)
);
CREATE INDEX idx_processing_batch_tasks_task
  ON processing_batch_tasks(task_id, request_state);
CREATE INDEX idx_processing_batch_tasks_batch
  ON processing_batch_tasks(batch_id, task_id);
CREATE TABLE processing_requests (
  request_id TEXT PRIMARY KEY,
  action TEXT NOT NULL,
  batch_id TEXT REFERENCES processing_batches(id) ON DELETE CASCADE,
  payload_hash TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open', 'applied', 'rejected')),
  selection_cursor INTEGER NOT NULL DEFAULT 0,
  response_json TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE processing_attempts (
  task_id TEXT NOT NULL REFERENCES processing_tasks(id) ON DELETE CASCADE,
  attempt_number INTEGER NOT NULL CHECK(attempt_number >= 1),
  lease_epoch INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  outcome TEXT NOT NULL DEFAULT 'open' CHECK(outcome IN ('open', 'succeeded', 'failed', 'interrupted', 'cancelled')),
  retryable INTEGER NOT NULL DEFAULT 0 CHECK(retryable IN (0, 1)),
  error_code TEXT,
  error_message TEXT,
  provider_request_id TEXT,
  PRIMARY KEY (task_id, attempt_number)
);
CREATE INDEX idx_processing_attempts_task
  ON processing_attempts(task_id, attempt_number);
CREATE TABLE processing_checkpoints (
  task_id TEXT NOT NULL REFERENCES processing_tasks(id) ON DELETE CASCADE,
  unit_key TEXT NOT NULL,
  input_fingerprint TEXT NOT NULL,
  contract_hash TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  payload_checksum TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (task_id, unit_key)
);

INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, config_snapshot_json, planning_cursor, planning_done, revision, created_at, updated_at, started_at, finished_at, last_error)
  SELECT id, request_id, origin, state, desired_state, operations, config_snapshot_json, planning_cursor, planning_done, revision, created_at, updated_at, started_at, finished_at, last_error FROM _backup_0043_processing_batches;
INSERT INTO processing_batch_collections (batch_id, collection_id_snapshot, name_snapshot)
  SELECT batch_id, collection_id_snapshot, name_snapshot FROM _backup_0043_processing_batch_collections;
INSERT INTO processing_batch_members (batch_id, ordinal, asset_id_snapshot, item_id_snapshot, collection_id_snapshot, title_snapshot, classification, reason)
  SELECT batch_id, ordinal, asset_id_snapshot, item_id_snapshot, collection_id_snapshot, title_snapshot, classification, reason FROM _backup_0043_processing_batch_members;
INSERT INTO processing_tasks (id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, stage, progress_done, progress_total, outcome, attempt_count, retry_cycle, retry_count, next_retry_at, owner_session, lease_epoch, heartbeat_at, lease_expires_at, last_error_code, last_error_message, result_receipt_json, created_at, updated_at, source_invalidation_count, domain, subject_kind, subject_id)
  SELECT id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, stage, progress_done, progress_total, outcome, attempt_count, retry_cycle, retry_count, next_retry_at, owner_session, lease_epoch, heartbeat_at, lease_expires_at, last_error_code, last_error_message, result_receipt_json, created_at, updated_at, source_invalidation_count, domain, subject_kind, subject_id FROM _backup_0043_processing_tasks;
INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id, domain, subject_kind, subject_id)
  SELECT batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id, domain, subject_kind, subject_id FROM _backup_0043_processing_batch_tasks;
INSERT INTO processing_requests (request_id, action, batch_id, payload_hash, state, selection_cursor, response_json, created_at)
  SELECT request_id, action, batch_id, payload_hash, state, selection_cursor, response_json, created_at FROM _backup_0043_processing_requests;
INSERT INTO processing_attempts (task_id, attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message, provider_request_id)
  SELECT task_id, attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message, provider_request_id FROM _backup_0043_processing_attempts;
INSERT INTO processing_checkpoints (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
  SELECT task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at FROM _backup_0043_processing_checkpoints;

DROP TABLE _backup_0043_processing_checkpoints;
DROP TABLE _backup_0043_processing_attempts;
DROP TABLE _backup_0043_processing_requests;
DROP TABLE _backup_0043_processing_batch_tasks;
DROP TABLE _backup_0043_processing_tasks;
DROP TABLE _backup_0043_processing_batch_members;
DROP TABLE _backup_0043_processing_batch_collections;
DROP TABLE _backup_0043_processing_batches;
