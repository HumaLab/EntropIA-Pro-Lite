-- Durable recovery journal for in-progress manuscript edits (plan-editor.md
-- §16.1). Measured in spike S6: a ~1 KB delta costs a p95 of 1.17 ms under the
-- synchronous=FULL this repository already sets, while writing the whole
-- manuscript costs 97.5 ms. So this table stores DELTAS, never documents.
--
-- delta_json is opaque to the backend: the editor produces ProseMirror steps,
-- this table makes the sequence durable, and recovery hands it back in order to
-- be replayed. Nothing here parses it.
--
-- Persisting a journal entry is NOT a canonical save (§16.2): the UI shows
-- "Guardado" only after the content commit advances writing_documents.revision.
-- Entries stay until a canonical revision that already contains them exists,
-- which is why pruning is keyed on base_revision rather than on age.

CREATE TABLE IF NOT EXISTS writing_journal (
  document_id    TEXT    NOT NULL REFERENCES writing_documents(id) ON DELETE CASCADE,
  -- Monotonic per document. Gaps are expected: a crash between the commit and
  -- the writer's acknowledgement leaves an entry nobody was told about, which
  -- recovery must accept rather than treat as corruption.
  seq            INTEGER NOT NULL,
  -- The canonical revision this delta applies on top of. Entries below the
  -- document's current revision are already folded into the content.
  base_revision  INTEGER NOT NULL,
  schema_version INTEGER NOT NULL,
  delta_json     TEXT    NOT NULL,
  -- Guards against a truncated or corrupt entry being replayed. §16.3 requires
  -- that such an entry is never applied partially and is kept for diagnosis.
  checksum       TEXT    NOT NULL,
  created_at     INTEGER NOT NULL,
  PRIMARY KEY (document_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_writing_journal_replay
  ON writing_journal (document_id, base_revision, seq);
