-- 0050_bibliographic_extraction_tasks: native extraction tasks and rows (E4a-WU2).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0050_bibliographic_extraction_tasks']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- earlier processing/bibliography migrations): the whole body goes inside one
-- BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a crash
-- between DDL and bookkeeping can never leave a half-applied 0050 behind.
--
-- Two changes:
-- 1. The kind CHECK on processing_tasks and processing_batch_tasks widens to
--    admit 'bibliography_extract'. SQLite cannot ALTER a CHECK, so both
--    tables are rebuilt preserving every column, both task indexes, every
--    FK, and every row byte-identically (dependents are backed up and
--    recreated verbatim). processing_batches is untouched.
-- 2. bibliographic_extractions stores one native-text row per attachment:
--    whole-document text plus page count, quality verdict, and the source
--    file identity (mtime/size) the text was read from, so a replaced file
--    never passes as current. Per-page rows arrive with selective OCR (E4b)
--    under their own migration; this table's one-row-per-attachment shape
--    stays the whole-document native record.
--
-- Plan section 6 "Limpieza local": these rows are managed derivatives — a
-- catalog row delete cascades, citations never touch them.

PRAGMA defer_foreign_keys=ON;

CREATE TABLE _backup_0050_processing_tasks AS SELECT * FROM processing_tasks;
CREATE TABLE _backup_0050_processing_batch_tasks AS SELECT * FROM processing_batch_tasks;
CREATE TABLE _backup_0050_processing_attempts AS SELECT * FROM processing_attempts;
CREATE TABLE _backup_0050_processing_checkpoints AS SELECT * FROM processing_checkpoints;

DROP TABLE processing_checkpoints;
DROP TABLE processing_attempts;
DROP TABLE processing_batch_tasks;
DROP TABLE processing_tasks;

CREATE TABLE processing_tasks (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding', 'bibliography_sync', 'bibliography_profile', 'bibliography_extract')),
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
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding', 'bibliography_sync', 'bibliography_profile', 'bibliography_extract')),
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

INSERT INTO processing_tasks (id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, stage, progress_done, progress_total, outcome, attempt_count, retry_cycle, retry_count, next_retry_at, owner_session, lease_epoch, heartbeat_at, lease_expires_at, last_error_code, last_error_message, result_receipt_json, created_at, updated_at, source_invalidation_count, domain, subject_kind, subject_id)
  SELECT id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, stage, progress_done, progress_total, outcome, attempt_count, retry_cycle, retry_count, next_retry_at, owner_session, lease_epoch, heartbeat_at, lease_expires_at, last_error_code, last_error_message, result_receipt_json, created_at, updated_at, source_invalidation_count, domain, subject_kind, subject_id FROM _backup_0050_processing_tasks;
INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id, domain, subject_kind, subject_id)
  SELECT batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id, domain, subject_kind, subject_id FROM _backup_0050_processing_batch_tasks;
INSERT INTO processing_attempts (task_id, attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message, provider_request_id)
  SELECT task_id, attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message, provider_request_id FROM _backup_0050_processing_attempts;
INSERT INTO processing_checkpoints (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
  SELECT task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at FROM _backup_0050_processing_checkpoints;

DROP TABLE _backup_0050_processing_checkpoints;
DROP TABLE _backup_0050_processing_attempts;
DROP TABLE _backup_0050_processing_batch_tasks;
DROP TABLE _backup_0050_processing_tasks;

CREATE TABLE bibliographic_extractions (
    attachment_id TEXT PRIMARY KEY NOT NULL REFERENCES zotero_attachments(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    page_count INTEGER NOT NULL,
    method TEXT NOT NULL CHECK(method IN ('native')),
    text_content TEXT NOT NULL,
    text_hash TEXT NOT NULL,
    text_chars INTEGER NOT NULL,
    quality TEXT NOT NULL CHECK(quality IN ('rich', 'sparse', 'empty')),
    source_mtime INTEGER,
    source_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_bibliographic_extractions_item
    ON bibliographic_extractions(item_id);
