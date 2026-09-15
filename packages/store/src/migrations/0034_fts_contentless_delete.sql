-- Recreate the FTS5 index with `contentless_delete=1`.
--
-- `fts_items` is contentless: the text lives in `extractions` and
-- `transcriptions`, and the index only holds tokens. Without this option
-- SQLite has no copy to diff against, so replacing a row ADDS the new tokens
-- and keeps the old ones. Re-running OCR, correcting an extraction or deleting
-- an asset left the previous text permanently searchable.
--
-- The option can only be set at creation, so the index is dropped, recreated
-- and rebuilt from the same source-of-truth query 0018 established.
-- SQLite 3.43+ (the bundled build is 3.45).
DROP TABLE IF EXISTS fts_items;

CREATE VIRTUAL TABLE fts_items USING fts5(
  item_id UNINDEXED,
  title,
  metadata,
  extracted_text,
  tokenize='unicode61 remove_diacritics 1',
  content='',
  contentless_delete=1
);

INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)
SELECT
  i.rowid,
  i.id,
  i.title,
  COALESCE(i.metadata, ''),
  COALESCE((
    SELECT GROUP_CONCAT(text_part, ' ')
    FROM (
      SELECT text_part
      FROM (
        SELECT COALESCE(e.text_content, '') AS text_part,
               0 AS source_order,
               COALESCE(a.sort_index, 0) AS sort_index,
               e.created_at AS created_at
        FROM extractions e
        JOIN assets a ON a.id = e.asset_id
        WHERE a.item_id = i.id

        UNION ALL

        SELECT COALESCE(t.text_content, '') AS text_part,
               1 AS source_order,
               COALESCE(a.sort_index, 0) AS sort_index,
               t.created_at AS created_at
        FROM transcriptions t
        JOIN assets a ON a.id = t.asset_id
        WHERE a.item_id = i.id
      ) ordered_text
      ORDER BY source_order ASC, sort_index ASC, created_at ASC
    )
  ), '')
FROM items i;
