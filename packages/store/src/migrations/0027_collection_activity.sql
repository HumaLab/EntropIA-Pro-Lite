-- Generated from COLLECTION_ACTIVITY_DDL in ../runner.ts.
-- Keep collections.updated_at as the canonical last-activity timestamp. Database
-- triggers cover repository writes, Rust worker writes, and synchronized changes.
CREATE TRIGGER collection_activity_items_insert
AFTER INSERT ON items
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = NEW.collection_id;
END;

CREATE TRIGGER collection_activity_items_update
AFTER UPDATE ON items
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = OLD.collection_id;
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = NEW.collection_id;
END;

CREATE TRIGGER collection_activity_items_delete
BEFORE DELETE ON items
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id = OLD.collection_id;
END;

CREATE TRIGGER collection_activity_assets_insert
AFTER INSERT ON assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_assets_update
AFTER UPDATE ON assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_assets_delete
BEFORE DELETE ON assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_notes_insert
AFTER INSERT ON notes
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_notes_update
AFTER UPDATE ON notes
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_notes_delete
BEFORE DELETE ON notes
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_extractions_insert
AFTER INSERT ON extractions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_extractions_update
AFTER UPDATE ON extractions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_extractions_delete
BEFORE DELETE ON extractions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_layouts_insert
AFTER INSERT ON layouts
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_layouts_update
AFTER UPDATE ON layouts
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_layouts_delete
BEFORE DELETE ON layouts
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_transcriptions_insert
AFTER INSERT ON transcriptions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_transcriptions_update
AFTER UPDATE ON transcriptions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_transcriptions_delete
BEFORE DELETE ON transcriptions
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_annotations_insert
AFTER INSERT ON annotations
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_annotations_update
AFTER UPDATE ON annotations
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = NEW.asset_id
);
END;

CREATE TRIGGER collection_activity_annotations_delete
BEFORE DELETE ON annotations
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = OLD.asset_id
);
END;

CREATE TRIGGER collection_activity_entities_insert
AFTER INSERT ON entities
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_entities_update
AFTER UPDATE ON entities
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_entities_delete
BEFORE DELETE ON entities
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_triples_insert
AFTER INSERT ON triples
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_triples_update
AFTER UPDATE ON triples
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_triples_delete
BEFORE DELETE ON triples
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_vec_assets_insert
AFTER INSERT ON vec_assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_vec_assets_update
AFTER UPDATE ON vec_assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = NEW.item_id);
END;

CREATE TRIGGER collection_activity_vec_assets_delete
BEFORE DELETE ON vec_assets
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE id IN (SELECT collection_id FROM items WHERE id = OLD.item_id);
END;

CREATE TRIGGER collection_activity_llm_results_insert
AFTER INSERT ON llm_results
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (NEW.target_type IN ('collection', 'unknown') AND id = NEW.target_id)
  OR (NEW.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = NEW.target_id
  ))
  OR (NEW.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = NEW.target_id
  ));
END;

CREATE TRIGGER collection_activity_llm_results_update
AFTER UPDATE ON llm_results
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (OLD.target_type IN ('collection', 'unknown') AND id = OLD.target_id)
  OR (OLD.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = OLD.target_id
  ))
  OR (OLD.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = OLD.target_id
  ));
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (NEW.target_type IN ('collection', 'unknown') AND id = NEW.target_id)
  OR (NEW.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = NEW.target_id
  ))
  OR (NEW.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = NEW.target_id
  ));
END;

CREATE TRIGGER collection_activity_llm_results_delete
BEFORE DELETE ON llm_results
BEGIN
  UPDATE collections
SET updated_at = MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)
WHERE (OLD.target_type IN ('collection', 'unknown') AND id = OLD.target_id)
  OR (OLD.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = OLD.target_id
  ))
  OR (OLD.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = OLD.target_id
  ));
END;
