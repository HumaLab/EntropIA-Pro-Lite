# EntropIA Pro Database Debugging

**Español:** [DATABASE_DEBUGGING.md](./DATABASE_DEBUGGING.md)

Operational guide for diagnosing problems in the EntropIA Pro SQLite database without wasting time.

## Active database

The path depends on the variant (Tauri resolves `app.path().app_data_dir()` from the effective `identifier` and opens `entropia.sqlite` inside that directory):

| Variant | Configuration | Path |
|---|---|---|
| Pro | `tauri.conf.json`: `com.entropia.pro.desktop` | `%APPDATA%\com.entropia.pro.desktop\entropia.sqlite` |
| Lite | `tauri.lite.conf.json`: `com.entropia.lite` | `%APPDATA%\com.entropia.lite\entropia.sqlite` |
| Explicit dev | `tauri.dev.conf.json`: `com.entropia.pro.desktop.dev` | `%APPDATA%\com.entropia.pro.desktop.dev\entropia.sqlite` |
| Recognized legacy | `com.entropia.app` constant in `lib.rs` | `%APPDATA%\com.entropia.app\entropia.sqlite` |

## Open the database

```powershell
sqlite3 "$env:APPDATA\com.entropia.pro.desktop\entropia.sqlite"
sqlite3 "$env:APPDATA\com.entropia.lite\entropia.sqlite"
sqlite3 "$env:APPDATA\com.entropia.pro.desktop.dev\entropia.sqlite"
```

## Debugging philosophy

Do not look at isolated tables as if they were magic boxes. Think through the flow:

```text
collection -> item -> asset -> processing -> enrichment -> search
```

Translated to tables:

```text
collections -> items -> assets -> extractions / transcriptions / layouts
collections / items / assets -> llm_results
items -> notes / entities / triples / item_topics
item_topics -> topics
assets -> vec_assets
items -> fts_items
```

---

## Quick diagnosis flow

### 1. Verify collection, item, and asset existence

```sql
SELECT * FROM collections ORDER BY created_at DESC;

SELECT i.id, i.title, c.name AS collection_name
FROM items i
JOIN collections c ON c.id = i.collection_id
ORDER BY i.created_at DESC;

SELECT *
FROM assets
ORDER BY created_at DESC;
```

### 2. Verify result persistence

```sql
SELECT asset_id, method, confidence, created_at
FROM extractions
ORDER BY created_at DESC;

SELECT asset_id, language, duration_ms, model, confidence, created_at
FROM transcriptions
ORDER BY created_at DESC;

SELECT asset_id, model, created_at
FROM layouts
ORDER BY created_at DESC;
```

### 3. Verify other persisted results

```sql
SELECT target_id, target_type, job_type, created_at
FROM llm_results
ORDER BY created_at DESC;
```

### 4. Verify semantic enrichment

```sql
SELECT item_id, COUNT(*) AS entities
FROM entities
GROUP BY item_id
ORDER BY entities DESC;

SELECT item_id, COUNT(*) AS triples
FROM triples
GROUP BY item_id
ORDER BY triples DESC;

SELECT item_id, COUNT(*) AS notes
FROM notes
GROUP BY item_id
ORDER BY notes DESC;
```

### 5. Verify indexing/search

```sql
SELECT item_id, title
FROM fts_items
LIMIT 20;

SELECT asset_id, item_id, length(embedding) AS bytes
FROM vec_assets
LIMIT 20;
```

### 6. Measure real asset-level embedding coverage

```sql
WITH asset_embedding_audit AS (
  SELECT
    a.id AS asset_id,
    a.item_id,
    a.type,
    EXISTS(
      SELECT 1 FROM extractions e
      WHERE e.asset_id = a.id
        AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0
    )
    OR EXISTS(
      SELECT 1 FROM transcriptions t
      WHERE t.asset_id = a.id
        AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0
    ) AS has_text,
    EXISTS(
      SELECT 1 FROM vec_assets v
      WHERE v.asset_id = a.id
    ) AS has_embedding
  FROM assets a
)
SELECT
  COUNT(*) AS total_assets,
  SUM(CASE WHEN has_text THEN 1 ELSE 0 END) AS assets_with_text,
  SUM(CASE WHEN has_embedding THEN 1 ELSE 0 END) AS assets_with_embedding,
  SUM(CASE WHEN has_text AND NOT has_embedding THEN 1 ELSE 0 END) AS assets_missing_embedding
FROM asset_embedding_audit;

WITH asset_embedding_audit AS (
  SELECT
    a.id AS asset_id,
    a.item_id,
    a.type,
    EXISTS(
      SELECT 1 FROM extractions e
      WHERE e.asset_id = a.id
        AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0
    )
    OR EXISTS(
      SELECT 1 FROM transcriptions t
      WHERE t.asset_id = a.id
        AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0
    ) AS has_text,
    EXISTS(
      SELECT 1 FROM vec_assets v
      WHERE v.asset_id = a.id
    ) AS has_embedding
  FROM assets a
)
SELECT
  type,
  SUM(CASE WHEN has_text THEN 1 ELSE 0 END) AS assets_with_text,
  SUM(CASE WHEN has_embedding THEN 1 ELSE 0 END) AS assets_with_embedding,
  SUM(CASE WHEN has_text AND NOT has_embedding THEN 1 ELSE 0 END) AS assets_missing_embedding
FROM asset_embedding_audit
GROUP BY type
ORDER BY assets_missing_embedding DESC, type ASC;
```

### 7. Operational `vec_assets` backfill

There is a real Tauri command for this: `backfill_asset_embeddings`.

- scans assets with useful text in `extractions` and/or `transcriptions`
- by default **skips** assets that already have a row in `vec_assets`
- with `force: true`, recomputes existing embeddings
- `limit` is useful for small audit/debug runs

Example from the frontend/Tauri lib:

```ts
import { backfillAssetEmbeddings } from './apps/desktop/src/lib/nlp'

const report = await backfillAssetEmbeddings({ force: false, limit: 100 })
console.log(report)
```

---

## Problem -> where to look -> query

### A. “A collection does not show up”

Look at:

- `collections`

```sql
SELECT id, name, description, created_at, updated_at
FROM collections
ORDER BY created_at DESC;
```

### B. “An item does not show up”

Look at:

- `items`
- `collections`

```sql
SELECT i.id, i.title, i.collection_id, c.name
FROM items i
LEFT JOIN collections c ON c.id = i.collection_id
ORDER BY i.created_at DESC;
```

### C. “The item exists, but it has no assets”

Look at:

- `assets`

```sql
SELECT id, item_id, path, type, size, sort_index, created_at
FROM assets
WHERE item_id = 'ITEM_ID_HERE'
ORDER BY sort_index, created_at;
```

### D. “The asset exists, but OCR/transcription did not run”

Look at:

- `extractions`
- `transcriptions`
- `layouts`

```sql
SELECT
  a.id,
  a.path,
  CASE WHEN e.asset_id IS NOT NULL THEN 'yes' ELSE 'no' END AS extraction,
  CASE WHEN t.asset_id IS NOT NULL THEN 'yes' ELSE 'no' END AS transcription,
  CASE WHEN l.asset_id IS NOT NULL THEN 'yes' ELSE 'no' END AS layout
FROM assets a
LEFT JOIN extractions e ON e.asset_id = a.id
LEFT JOIN transcriptions t ON t.asset_id = a.id
LEFT JOIN layouts l ON l.asset_id = a.id
WHERE a.id = 'ASSET_ID_HERE'
LIMIT 1;
```

### E. “OCR High did not leave layout”

Look at:

- `layouts`
- `extractions`

```sql
SELECT asset_id, model, image_width, image_height, created_at
FROM layouts
WHERE asset_id = 'ASSET_ID_HERE';
```

### F. “Entities or triples do not show up”

Look at:

- `entities`
- `triples`

```sql
SELECT id, entity_type, value, confidence, source, model_name
FROM entities
WHERE item_id = 'ITEM_ID_HERE'
ORDER BY confidence DESC;

SELECT subject, predicate, object, created_at
FROM triples
WHERE item_id = 'ITEM_ID_HERE'
ORDER BY created_at DESC;
```

### G. “Topics do not show up”

Look at:

- `item_topics`
- `topics`

```sql
SELECT t.name
FROM item_topics it
JOIN topics t ON t.id = it.topic_id
WHERE it.item_id = 'ITEM_ID_HERE';
```

### H. “FTS search returns nothing”

Look at:

- `fts_items`

```sql
SELECT item_id, title, extracted_text
FROM fts_items
WHERE fts_items MATCH 'term';
```

### I. “Similarity/embeddings are not working”

Look at:

- `vec_assets`
- `extractions`
- `transcriptions`

```sql
SELECT asset_id, item_id, length(embedding) AS bytes
FROM vec_assets
WHERE asset_id = 'ASSET_ID_HERE';
```

Active APIs/runtime for this flow: `embed_asset`, `backfill_asset_embeddings`, `similar_assets`, and their TS wrappers `embedAsset`, `backfillAssetEmbeddings`, `similarAssets`.

If the asset has text but no embedding, the problem is NO LONGER theoretical: run the backfill, then audit again.

### J. “The LLM result was mixed between asset/item/collection or disappeared”

Look at:

- `llm_results`

```sql
SELECT id, target_id, target_type, job_type, created_at, result
FROM llm_results
WHERE target_id = 'TARGET_ID_HERE'
ORDER BY created_at DESC;

SELECT id, target_id, target_type, job_type, created_at
FROM llm_results
WHERE created_at < 1000000000000
ORDER BY created_at ASC;
```

---

## Current embedding architecture

The verified runtime/product truth today is this:

- embeddings and similarity are **asset-only**
- active table: `vec_assets`
- active APIs: `embed_asset`, `backfill_asset_embeddings`, `similar_assets`

If you see `vec_items`, `embed_item`, `similar_items`, or `embeddings_fallback` in old notes or snapshots of a previous local DB, treat them as **legacy/archive**, not as supported architecture.

## Relational audit queries

### Assets orphaned from items

```sql
SELECT a.*
FROM assets a
LEFT JOIN items i ON i.id = a.item_id
WHERE i.id IS NULL;
```

### Items orphaned from collections

```sql
SELECT i.*
FROM items i
LEFT JOIN collections c ON c.id = i.collection_id
WHERE c.id IS NULL;
```

### Notes pointing to missing assets

```sql
SELECT n.*
FROM notes n
LEFT JOIN assets a ON a.id = n.asset_id
WHERE n.asset_id IS NOT NULL
  AND a.id IS NULL;
```

### Entities with broken asset_id

```sql
SELECT e.*
FROM entities e
LEFT JOIN assets a ON a.id = e.asset_id
WHERE e.asset_id IS NOT NULL
  AND a.id IS NULL;
```

### Triples with broken asset_id

```sql
SELECT t.*
FROM triples t
LEFT JOIN assets a ON a.id = t.asset_id
WHERE t.asset_id IS NOT NULL
  AND a.id IS NULL;
```

### Vec assets with suspicious references

```sql
SELECT va.*
FROM vec_assets va
LEFT JOIN assets a ON a.id = va.asset_id
LEFT JOIN items i ON i.id = va.item_id
WHERE a.id IS NULL OR i.id IS NULL;
```

### LLM results with broken target or unmigrated legacy target

```sql
SELECT lr.*
FROM llm_results lr
LEFT JOIN assets a ON lr.target_type = 'asset' AND a.id = lr.target_id
LEFT JOIN items i ON lr.target_type = 'item' AND i.id = lr.target_id
LEFT JOIN collections c ON lr.target_type = 'collection' AND c.id = lr.target_id
WHERE (lr.target_type = 'asset' AND a.id IS NULL)
   OR (lr.target_type = 'item' AND i.id IS NULL)
   OR (lr.target_type = 'collection' AND c.id IS NULL)
   OR lr.target_type = 'unknown';
```

---

## Queries for understanding pipeline coverage

### Which assets have which output type

```sql
SELECT
  a.id AS asset_id,
  a.type,
  CASE WHEN e.asset_id IS NOT NULL THEN 'yes' ELSE 'no' END AS extraction,
  CASE WHEN t.asset_id IS NOT NULL THEN 'yes' ELSE 'no' END AS transcription,
  CASE WHEN l.asset_id IS NOT NULL THEN 'yes' ELSE 'no' END AS layout
FROM assets a
LEFT JOIN extractions e ON e.asset_id = a.id
LEFT JOIN transcriptions t ON t.asset_id = a.id
LEFT JOIN layouts l ON l.asset_id = a.id
ORDER BY a.created_at DESC;
```

### Which items are semantically enriched

```sql
SELECT
  i.id,
  i.title,
  (SELECT COUNT(*) FROM entities e WHERE e.item_id = i.id) AS entity_count,
  (SELECT COUNT(*) FROM triples t WHERE t.item_id = i.id) AS triple_count,
  (SELECT COUNT(*) FROM item_topics it WHERE it.item_id = i.id) AS topic_count,
  (SELECT COUNT(*) FROM notes n WHERE n.item_id = i.id) AS note_count
FROM items i
ORDER BY i.updated_at DESC;
```

---

## Useful sqlite3 commands

```sql
.tables
.schema
.schema items
.schema assets
.indexes items
.indexes assets
PRAGMA foreign_keys;
PRAGMA integrity_check;
PRAGMA quick_check;
```

## Brutally practical recommendation

When something fails, do not start with guesses. Start with evidence:

1. `assets`
2. `extractions` / `transcriptions` / `layouts`
3. `entities` / `triples` / `topics`
4. `fts_items` / `vec_assets`

It is that simple. First you verify persistence. Then you discuss logic.

> Compatibility: if you open an old database and `jobs` still appears, treat it as a legacy table. The current runtime does not use it, and the active cleanup removes it with migration `0021_drop_unused_processing_table`.
