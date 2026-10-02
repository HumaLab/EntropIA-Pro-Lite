-- E1b-2 durable per-library bibliography reconciliation state.
--
-- The current row is keyed by the internal library FK. A generated run_id
-- scopes the normalized seen-set, while connection_revision is the stale
-- identity fence; zotero_libraries.last_modified_version remains catalog data.

CREATE TABLE IF NOT EXISTS zotero_reconciliation_runs (
  library_id              TEXT PRIMARY KEY NOT NULL
                            REFERENCES zotero_libraries(id) ON DELETE CASCADE
                            CHECK(length(trim(library_id)) > 0),
  run_id                  TEXT NOT NULL CHECK(length(trim(run_id)) > 0),
  connection_revision     INTEGER NOT NULL CHECK(connection_revision >= 0),
  state                   TEXT NOT NULL
                            CHECK(state IN ('running', 'retry_wait', 'interrupted', 'blocked', 'failed', 'completed')),
  phase                   TEXT NOT NULL
                            CHECK(phase IN ('versions', 'catalog', 'finalize')),
  cursor_start            INTEGER NOT NULL DEFAULT 0 CHECK(cursor_start >= 0),
  cursor_limit            INTEGER NOT NULL CHECK(cursor_limit > 0),
  remote_total            INTEGER CHECK(remote_total IS NULL OR remote_total >= 0),
  target_version          INTEGER CHECK(target_version IS NULL OR target_version >= 0),
  checkpoint_version      INTEGER CHECK(checkpoint_version IS NULL OR checkpoint_version >= 0),
  retry_count             INTEGER NOT NULL DEFAULT 0 CHECK(retry_count >= 0),
  attempt_count           INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count >= 0),
  next_retry_at           INTEGER CHECK(next_retry_at IS NULL OR next_retry_at >= 0),
  last_attempt_at         INTEGER CHECK(last_attempt_at IS NULL OR last_attempt_at >= 0),
  latest_error_phase      TEXT CHECK(latest_error_phase IS NULL OR latest_error_phase IN ('versions', 'catalog', 'finalize')),
  latest_error_code       TEXT CHECK(latest_error_code IS NULL OR (length(trim(latest_error_code)) > 0 AND length(latest_error_code) <= 128)),
  latest_error_message    TEXT CHECK(latest_error_message IS NULL OR (length(trim(latest_error_message)) > 0 AND length(latest_error_message) <= 1024)),
  latest_error_retryable  INTEGER CHECK(latest_error_retryable IS NULL OR latest_error_retryable IN (0, 1)),
  latest_error_at         INTEGER CHECK(latest_error_at IS NULL OR latest_error_at >= 0),
  revision                INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  checkpointed_at         INTEGER CHECK(checkpointed_at IS NULL OR checkpointed_at >= 0),
  completed_at            INTEGER CHECK(completed_at IS NULL OR completed_at >= 0),
  created_at              INTEGER NOT NULL CHECK(created_at >= 0),
  updated_at              INTEGER NOT NULL CHECK(updated_at >= 0),
  UNIQUE(library_id, run_id),
  UNIQUE(run_id),
  CHECK(
    (latest_error_phase IS NULL AND latest_error_code IS NULL
      AND latest_error_message IS NULL AND latest_error_retryable IS NULL
      AND latest_error_at IS NULL)
    OR
    (latest_error_phase IS NOT NULL AND latest_error_code IS NOT NULL
      AND latest_error_message IS NOT NULL AND latest_error_retryable IS NOT NULL
      AND latest_error_at IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS idx_zotero_reconciliation_runs_state
  ON zotero_reconciliation_runs(state, next_retry_at, library_id);

CREATE TABLE IF NOT EXISTS zotero_reconciliation_seen (
  library_id      TEXT NOT NULL CHECK(length(trim(library_id)) > 0),
  run_id          TEXT NOT NULL,
  entity_kind     TEXT NOT NULL
                  CHECK(entity_kind IN ('item', 'collection', 'tag', 'attachment')),
  entity_key      TEXT NOT NULL CHECK(length(trim(entity_key)) > 0),
  parent_key      TEXT NOT NULL DEFAULT '',
  remote_version  INTEGER CHECK(remote_version IS NULL OR remote_version >= 0),
  observed_at     INTEGER NOT NULL CHECK(observed_at >= 0),
  PRIMARY KEY (library_id, run_id, entity_kind, entity_key, parent_key),
  FOREIGN KEY (library_id, run_id)
    REFERENCES zotero_reconciliation_runs(library_id, run_id) ON DELETE CASCADE,
  CHECK(
    (entity_kind = 'attachment' AND length(trim(parent_key)) > 0)
    OR (entity_kind <> 'attachment' AND parent_key = '')
  )
);

CREATE INDEX IF NOT EXISTS idx_zotero_reconciliation_seen_kind
  ON zotero_reconciliation_seen(library_id, run_id, entity_kind, entity_key);
