-- Process history of each asset: one row per process an asset went through
-- (import, page split, image edit, OCR, text correction, transcription,
-- embedding, entity extraction, summary...), with the tool, the model and the
-- versions that produced it, so every change to a document can be traced and
-- audited by the person.
--
-- Append-only: rows are never updated by the app, with one exception. Manual
-- text editing saves every couple of seconds while the person types, so the
-- edits of one session coalesce into a single 'text_edit' row whose
-- params_json counts the saves (see `asset_history::record_text_edit`).
--
-- process:       stable code ('import', 'image_crop', 'ocr', ...).
-- tool:          what did the work ('GLM-OCR API', 'PaddleOCR', 'pdfium', ...).
-- tool_version:  its version when known (crate, runtime pack, API version).
-- model:         the model id when there is one ('glm-ocr', 'baai/bge-m3', ...).
-- model_version: the model's own version or contract when known.
-- app_version:   the EntropIA build that ran the process.
-- params_json:   process parameters (region, degrees, page, mode...), a JSON
--                object or NULL.
-- input_sha256 / output_sha256: hashes of the file or text before and after,
--                when the process changes one.
--
-- Local only for now: nothing here is in the device sync set, so each device
-- lists the processes it ran. Deleting an asset deletes its history with it.

CREATE TABLE IF NOT EXISTS asset_process_events (
  id            TEXT    PRIMARY KEY,
  asset_id      TEXT    NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  process       TEXT    NOT NULL,
  tool          TEXT    NOT NULL,
  tool_version  TEXT,
  model         TEXT,
  model_version TEXT,
  app_version   TEXT    NOT NULL,
  params_json   TEXT    CHECK (params_json IS NULL OR json_valid(params_json)),
  input_sha256  TEXT,
  output_sha256 TEXT,
  created_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_asset_process_events_asset
  ON asset_process_events (asset_id, created_at);
