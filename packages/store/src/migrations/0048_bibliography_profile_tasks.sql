-- 0048_bibliography_profile_tasks: per-work profile tasks and embeddings (E3b-WU2).
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0048_bibliography_profile_tasks']); this file mirrors it
-- for review and for the Rust processing tests (include_str!). Keep both
-- identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- 0032/0040/0041/0042/0043/0044/0045/0046/0047): the whole body goes inside
-- one BEGIN IMMEDIATE ... COMMIT together with the _migrations row, so a
-- crash between DDL and bookkeeping can never leave a half-applied 0048
-- behind.
--
-- Two changes:
-- 1. The kind CHECK on processing_tasks and processing_batch_tasks widens to
--    admit 'bibliography_profile'. SQLite cannot ALTER a CHECK, so both
--    tables are rebuilt preserving every column, both task indexes, every
--    FK, and every row byte-identically (dependents are backed up and
--    recreated verbatim). processing_batches is untouched: its origin CHECK
--    already admits 'bibliography' and 0046's priority column survives
--    because this migration never drops that table.
-- 2. bibliographic_item_embeddings stores one vector per (work, contract)
--    under the effective embedding contract, with the profile input hash it
--    was computed from. Generations (plan §6) arrive in E3c without breaking
--    this key: a generation column can be added later without rewriting
--    identity.

PRAGMA defer_foreign_keys=ON;

CREATE TABLE _backup_0048_processing_tasks AS SELECT * FROM processing_tasks;
CREATE TABLE _backup_0048_processing_batch_tasks AS SELECT * FROM processing_batch_tasks;
CREATE TABLE _backup_0048_processing_attempts AS SELECT * FROM processing_attempts;
CREATE TABLE _backup_0048_processing_checkpoints AS SELECT * FROM processing_checkpoints;

DROP TABLE processing_checkpoints;
DROP TABLE processing_attempts;
DROP TABLE processing_batch_tasks;
DROP TABLE processing_tasks;

CREATE TABLE processing_tasks (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding', 'bibliography_sync', 'bibliography_profile')),
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
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding', 'bibliography_sync', 'bibliography_profile')),
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
  SELECT id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, stage, progress_done, progress_total, outcome, attempt_count, retry_cycle, retry_count, next_retry_at, owner_session, lease_epoch, heartbeat_at, lease_expires_at, last_error_code, last_error_message, result_receipt_json, created_at, updated_at, source_invalidation_count, domain, subject_kind, subject_id FROM _backup_0048_processing_tasks;
INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id, domain, subject_kind, subject_id)
  SELECT batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id, domain, subject_kind, subject_id FROM _backup_0048_processing_batch_tasks;
INSERT INTO processing_attempts (task_id, attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message, provider_request_id)
  SELECT task_id, attempt_number, lease_epoch, started_at, finished_at, outcome, retryable, error_code, error_message, provider_request_id FROM _backup_0048_processing_attempts;
INSERT INTO processing_checkpoints (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
  SELECT task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at FROM _backup_0048_processing_checkpoints;

DROP TABLE _backup_0048_processing_checkpoints;
DROP TABLE _backup_0048_processing_attempts;
DROP TABLE _backup_0048_processing_batch_tasks;
DROP TABLE _backup_0048_processing_tasks;

CREATE TABLE bibliographic_item_embeddings (
    item_id TEXT NOT NULL REFERENCES bibliographic_items(id) ON DELETE CASCADE,
    embedding_contract TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    input_hash TEXT NOT NULL,
    profile_revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (item_id, embedding_contract)
);

CREATE INDEX idx_bibliographic_item_embeddings_hash
    ON bibliographic_item_embeddings(input_hash);

-- Rebuilding processing_tasks and processing_batch_tasks above dropped the
-- claim index, the dependency index and the settle trigger that
-- 0038_processing_settle_on_terminal created on them. Restore them verbatim
-- so the queue keeps settling dependents on every terminal transition.
CREATE INDEX IF NOT EXISTS idx_processing_tasks_state_id
  ON processing_tasks(state, id);

CREATE INDEX IF NOT EXISTS idx_processing_batch_tasks_dependency
  ON processing_batch_tasks(dependency_task_id)
  WHERE dependency_task_id IS NOT NULL;

CREATE TRIGGER IF NOT EXISTS processing_tasks_settle_dependents
AFTER UPDATE OF state ON processing_tasks
WHEN NEW.state IN ('succeeded', 'failed', 'cancelled') AND OLD.state IS NOT NEW.state
BEGIN
  UPDATE processing_attempts SET outcome = 'failed', finished_at = strftime('%s', 'now') * 1000
   WHERE NEW.state <> 'succeeded' AND outcome = 'open'
     AND task_id IN (
       SELECT l.task_id FROM processing_batch_tasks l
         JOIN processing_tasks t ON t.id = l.task_id
        WHERE l.dependency_task_id = NEW.id AND t.state = 'blocked');
  UPDATE processing_tasks
     SET state = 'failed', outcome = 'dependency_failed', last_error_code = 'dependency_failed',
         last_error_message = 'dependency ' || NEW.id || ' ended as ' || NEW.state,
         updated_at = strftime('%s', 'now') * 1000
   WHERE NEW.state <> 'succeeded' AND state = 'blocked'
     AND id IN (SELECT task_id FROM processing_batch_tasks WHERE dependency_task_id = NEW.id);
  UPDATE processing_tasks
     SET state = 'pending', stage = '', updated_at = strftime('%s', 'now') * 1000
   WHERE NEW.state = 'succeeded' AND state = 'blocked'
     AND id IN (SELECT task_id FROM processing_batch_tasks WHERE dependency_task_id = NEW.id);
END;
