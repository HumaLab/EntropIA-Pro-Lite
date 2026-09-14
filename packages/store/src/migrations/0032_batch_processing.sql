-- 0032_batch_processing: durable background batch queue for OCR + embeddings.
--
-- Source of truth at runtime is the inlined copy in packages/store/src/runner.ts
-- (MIGRATIONS['0032_batch_processing']); this file mirrors it for review and for
-- the Rust processing tests (include_str!). Keep both identical.
--
-- Runs through the trigger-safe single-batch path in runMigrations() (same as
-- 0025/0027/0029): the whole body goes inside one BEGIN IMMEDIATE ... COMMIT,
-- so the naive `splitStatements` must never see the trigger bodies.
--
-- Design (plan-lote.md §5): the queue lives in entropia.sqlite next to the
-- results it coordinates, so receipts and terminal states commit atomically
-- with canonical output. Historical asset/collection ids are snapshots (plain
-- TEXT, no CASCADE that would destroy attempts when content is deleted).

CREATE TABLE processing_batches (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  origin TEXT NOT NULL CHECK(origin IN ('user', 'manual', 'repair')),
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
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding')),
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
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_processing_tasks_claimable
  ON processing_tasks(state, next_retry_at, id);
-- One active writer per operation+asset: retries reuse the same row (new
-- retry_cycle), so terminal rows are excluded and a fresh admission after a
-- terminal state must transition that row, never insert a second one.
CREATE UNIQUE INDEX idx_processing_tasks_active_unique
  ON processing_tasks(kind, asset_id_snapshot)
  WHERE state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled');

CREATE TABLE processing_batch_tasks (
  batch_id TEXT NOT NULL REFERENCES processing_batches(id) ON DELETE CASCADE,
  task_id TEXT NOT NULL REFERENCES processing_tasks(id),
  kind TEXT NOT NULL CHECK(kind IN ('ocr', 'embedding')),
  asset_id_snapshot TEXT NOT NULL,
  request_state TEXT NOT NULL DEFAULT 'active' CHECK(request_state IN ('active', 'paused', 'cancelled')),
  dependency_task_id TEXT REFERENCES processing_tasks(id),
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

-- Monotonic source revisions per asset. Bumped in the SAME transaction that
-- mutates extraction/transcription rows, so an embedding result computed from
-- older text can never clear a newer invalidation (Unidad 2+ compares these).
-- Rows cascade when the asset itself is deleted; per-row triggers below skip
-- the bump when the asset no longer exists so FK cascades cannot resurrect it.
CREATE TABLE processing_asset_revisions (
  asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  source_revision INTEGER NOT NULL DEFAULT 0,
  embedding_completed_revision INTEGER,
  invalidated_at INTEGER,
  invalidation_reason TEXT,
  auto_suppressed_revision INTEGER
);
-- Single-row liveness for the supervisor thread (`scheduler_heartbeat` =
-- "<session-id>|<epoch-millis>"). Recovery and claiming consult it so a
-- second process never steals units from a live scheduler, while a dead
-- scheduler's fresh-looking leases still converge on restart.
CREATE TABLE processing_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TRIGGER trg_processing_extractions_ai
AFTER INSERT ON extractions
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'extraction_insert')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_extractions_au
AFTER UPDATE OF text_content, method ON extractions
WHEN OLD.text_content IS NOT NEW.text_content OR OLD.method IS NOT NEW.method
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'extraction_update')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_extractions_ad
AFTER DELETE ON extractions
WHEN EXISTS (SELECT 1 FROM assets WHERE id = OLD.asset_id)
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (OLD.asset_id, 1, (strftime('%s', 'now') * 1000), 'extraction_delete')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_transcriptions_ai
AFTER INSERT ON transcriptions
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'transcription_insert')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_transcriptions_au
AFTER UPDATE OF text_content ON transcriptions
WHEN OLD.text_content IS NOT NEW.text_content
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (NEW.asset_id, 1, (strftime('%s', 'now') * 1000), 'transcription_update')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
CREATE TRIGGER trg_processing_transcriptions_ad
AFTER DELETE ON transcriptions
WHEN EXISTS (SELECT 1 FROM assets WHERE id = OLD.asset_id)
BEGIN
  INSERT INTO processing_asset_revisions(asset_id, source_revision, invalidated_at, invalidation_reason)
  VALUES (OLD.asset_id, 1, (strftime('%s', 'now') * 1000), 'transcription_delete')
  ON CONFLICT(asset_id) DO UPDATE SET
    source_revision = processing_asset_revisions.source_revision + 1,
    invalidated_at = excluded.invalidated_at,
    invalidation_reason = excluded.invalidation_reason;
END;
