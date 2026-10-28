-- Zotero annotations of the Biblioteca (docs/zotero-annotations-plan.md, section 3).
--
-- One row per Zotero annotation item of a library, whatever its parent
-- attachment. The row holds the last confirmed server state (base_*), the
-- pending local intent (local_*), the request in flight (sent_*) and the
-- conflict being resolved (conflict_*). JSON columns hold the Zotero `data`
-- object verbatim.
--
-- There is no foreign key to zotero_attachments: the catalog sync removing an
-- attachment must neither cascade to these rows nor be blocked by them.
--
-- Versions here are Zotero Web API versions, never the local API watermark in
-- zotero_libraries.last_modified_version.
--
-- Local only: nothing here is in the device sync set. Each device pulls from
-- Zotero.

CREATE TABLE IF NOT EXISTS zotero_annotation_items (
  library_row_id  TEXT    NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
  web_user_id     TEXT    NOT NULL,
  annotation_key  TEXT    NOT NULL CHECK (length(annotation_key) = 8),
  parent_key      TEXT    NOT NULL,
  base_version    INTEGER,
  base_json       TEXT,
  remote_trashed  INTEGER NOT NULL DEFAULT 0 CHECK (remote_trashed IN (0, 1)),
  local_json      TEXT,
  local_deleted   INTEGER NOT NULL DEFAULT 0 CHECK (local_deleted IN (0, 1)),
  edit_gen        INTEGER NOT NULL DEFAULT 0,
  sent_gen        INTEGER,
  sent_json       TEXT,
  sent_uncertain  INTEGER NOT NULL DEFAULT 0 CHECK (sent_uncertain IN (0, 1)),
  sync_state      TEXT    NOT NULL CHECK (sync_state IN ('synced', 'pending', 'waiting_parent', 'conflict', 'error')),
  conflict_fields TEXT    NOT NULL DEFAULT '[]',
  conflict_json   TEXT,
  last_error      TEXT,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  PRIMARY KEY (library_row_id, annotation_key),
  CHECK (local_deleted = 0 OR local_json IS NULL)
);

CREATE INDEX IF NOT EXISTS idx_zotero_annotation_items_parent
  ON zotero_annotation_items (library_row_id, parent_key);
CREATE INDEX IF NOT EXISTS idx_zotero_annotation_items_state
  ON zotero_annotation_items (library_row_id, sync_state);

-- The Web API pull cursor and the shared pause of one library for one Web
-- account.
CREATE TABLE IF NOT EXISTS zotero_annotation_sync_state (
  library_row_id      TEXT    NOT NULL REFERENCES zotero_libraries(id) ON DELETE CASCADE,
  web_user_id         TEXT    NOT NULL,
  web_library_version INTEGER NOT NULL DEFAULT 0,
  paused_until        INTEGER,
  pause_reason        TEXT,
  updated_at          INTEGER NOT NULL,
  PRIMARY KEY (library_row_id, web_user_id)
);
