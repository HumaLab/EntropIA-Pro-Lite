# Debugging de base de datos de EntropIA Pro y Lite

**English:** [DATABASE_DEBUGGING.en.md](./DATABASE_DEBUGGING.en.md)

Guía operativa para diagnosticar problemas en la base SQLite de EntropIA (Pro y Lite comparten el mismo esquema) sin dar vueltas. El inventario completo de tablas, columnas, índices y triggers está en [SQLite.md](./SQLite.md).

## Base activa

Pro, Lite y desarrollo comparten una única base, `entropia.sqlite`, dentro del directorio compartido `com.entropia.shared` del directorio de datos del sistema:

| Sistema | Ruta |
|---|---|
| Windows | `%APPDATA%\com.entropia.shared\entropia.sqlite` |
| macOS | `~/Library/Application Support/com.entropia.shared/entropia.sqlite` |
| Linux | `~/.local/share/com.entropia.shared/entropia.sqlite` |
| Perfil de desarrollo (`ENTROPIA_DEV_PROFILE=<nombre>`, solo builds debug) | `<datos>/com.entropia.shared/dev-profiles/<nombre>/entropia.sqlite` |
| Legacy reconocido | `<datos>/<identificador>/entropia.sqlite`, para los ocho identificadores de `LEGACY_APP_IDENTIFIERS` (`lib.rs`); el backend los combina en la base compartida al iniciar |

El directorio compartido no es el `app_data_dir()` de ninguna variante: los
identificadores no pueden cambiar porque son lo que permite instalar Lite y Pro
en paralelo, así que el canónico es un directorio fijo hermano de los de cada
variante. En la versión MSIX de Microsoft Store, Windows puede redirigirlo dentro
de `%LOCALAPPDATA%\Packages\CONICET.EntropIALite_…\LocalCache\Roaming\`.

Una copia de seguridad (`entropia-backup-<UTC>`, en la carpeta elegida en **Configuración → Copias**) contiene su propio `entropia.sqlite`: sirve para comparar sin tocar la base activa.

## Abrir la base

```powershell
sqlite3 "$env:APPDATA\com.entropia.shared\entropia.sqlite"
```

Leé con la app abierta si querés (la base está en WAL), pero **cerrá EntropIA antes de escribir**. Las migraciones las aplica el backend al iniciar; no corras DDL a mano sobre la base activa.

## Filosofía de debugging

No mires tablas aisladas como si fueran cajitas mágicas. Pensá el flujo:

```text
collection -> item -> asset -> procesamiento -> enriquecimiento -> búsqueda
```

Traducido a tablas:

```text
collections -> items -> assets -> extractions / transcriptions / layouts
collections / items / assets -> llm_results
items -> notes / entities / triples / item_topics
item_topics -> topics
assets -> vec_assets
items -> fts_items
processing_batches -> processing_batch_tasks -> processing_tasks -> processing_attempts
writing_documents -> writing_document_versions / writing_document_citations / writing_zotero_citations
zotero_libraries -> bibliographic_items -> zotero_attachments -> bibliographic_page_texts / bibliographic_chunks
```

---

## Flujo de diagnóstico rápido

### 1. Verificar existencia de colección, ítem y asset

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

### 2. Verificar persistencia de resultados

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

### 3. Verificar otros resultados persistidos

```sql
SELECT target_id, target_type, job_type, created_at
FROM llm_results
ORDER BY created_at DESC;
```

### 4. Verificar enriquecimiento semántico

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

### 5. Verificar indexación/búsqueda

```sql
-- fts_items es contentless: se une con items por rowid.
SELECT i.id AS item_id, i.title
FROM fts_items
JOIN items i ON i.rowid = fts_items.rowid
LIMIT 20;

SELECT asset_id, item_id, length(embedding) AS bytes
FROM vec_assets
LIMIT 20;
```

### 6. Medir cobertura real de embeddings asset-level

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

### 7. Backfill operativo de `vec_assets`

Hay un comando Tauri real para esto: `backfill_asset_embeddings`.

- recorre assets con texto útil en `extractions` y/o `transcriptions`
- por default **saltea** assets que ya tienen fila en `vec_assets`
- con `force: true` recomputa embeddings existentes
- `limit` sirve para corridas chicas de auditoría/debug

Ejemplo desde el frontend/lib Tauri:

```ts
import { backfillAssetEmbeddings } from './apps/desktop/src/lib/nlp'

const report = await backfillAssetEmbeddings({ force: false, limit: 100 })
console.log(report)
```

---

## Problema -> dónde mirar -> query

### A. “No aparece una colección”

Mirar:

- `collections`

```sql
SELECT id, name, description, created_at, updated_at
FROM collections
ORDER BY created_at DESC;
```

### B. “No aparece un ítem”

Mirar:

- `items`
- `collections`

```sql
SELECT i.id, i.title, i.collection_id, c.name
FROM items i
LEFT JOIN collections c ON c.id = i.collection_id
ORDER BY i.created_at DESC;
```

### C. “El ítem existe, pero no tiene assets”

Mirar:

- `assets`

```sql
SELECT id, item_id, path, type, size, sort_index, created_at
FROM assets
WHERE item_id = 'ITEM_ID_AQUI'
ORDER BY sort_index, created_at;
```

### D. “El asset está, pero OCR/transcripción no corrió”

Mirar:

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
WHERE a.id = 'ASSET_ID_AQUI'
LIMIT 1;
```

Si lo pediste y no hay resultado, mirá su tarea de procesamiento: pulsar OCRH o EMBED y los lotes crean tareas en `processing_tasks` (la transcripción STT no pasa por ahí).

```sql
SELECT id, kind, state, stage, attempt_count, last_error_code, last_error_message,
       datetime(updated_at / 1000, 'unixepoch', 'localtime') AS updated_local
FROM processing_tasks
WHERE subject_id = 'ASSET_ID_AQUI' OR asset_id_snapshot = 'ASSET_ID_AQUI'
ORDER BY updated_at DESC;
```

### E. “OCR High no dejó layout”

Mirar:

- `layouts`
- `extractions`

```sql
SELECT asset_id, model, image_width, image_height, created_at
FROM layouts
WHERE asset_id = 'ASSET_ID_AQUI';
```

### F. “No aparecen entidades o triples”

Mirar:

- `entities`
- `triples`

```sql
SELECT id, entity_type, value, confidence, source, model_name
FROM entities
WHERE item_id = 'ITEM_ID_AQUI'
ORDER BY confidence DESC;

SELECT subject, predicate, object, created_at
FROM triples
WHERE item_id = 'ITEM_ID_AQUI'
ORDER BY created_at DESC;
```

### G. “No aparecen topics”

Mirar:

- `item_topics`
- `topics`

```sql
SELECT t.name
FROM item_topics it
JOIN topics t ON t.id = it.topic_id
WHERE it.item_id = 'ITEM_ID_AQUI';
```

### H. “La búsqueda FTS no devuelve nada”

Mirar:

- `fts_items`

```sql
SELECT i.id AS item_id, i.title, bm25(fts_items) AS rank
FROM fts_items
JOIN items i ON i.rowid = fts_items.rowid
WHERE fts_items MATCH 'termino'
ORDER BY rank;
```

`fts_items` es contentless: sus columnas se leen como NULL. No leas `title` ni `extracted_text` desde ahí.

### I. “La similitud/embeddings no funciona”

Mirar:

- `vec_assets`
- `extractions`
- `transcriptions`

```sql
SELECT asset_id, item_id, length(embedding) AS bytes
FROM vec_assets
WHERE asset_id = 'ASSET_ID_AQUI';
```

APIs/runtime activos para este flujo: `embed_asset`, `backfill_asset_embeddings`, `similar_assets` y sus wrappers TS `embedAsset`, `backfillAssetEmbeddings`, `similarAssets`.

Si el asset tiene texto pero no embedding, mirá primero si quedó un reintento pendiente y por qué:

```sql
SELECT asset_id, rag_incomplete, failure_count, last_error,
       datetime(next_retry_at_ms / 1000, 'unixepoch', 'localtime') AS next_retry_local
FROM rag_asset_embedding_state
WHERE asset_id = 'ASSET_ID_AQUI';
```

Un 429 del proveedor o una clave faltante dejan la tarea en espera, no fallida; al guardar la clave o reiniciar se retoma. Si no hay nada pendiente, corré el backfill y después auditá de nuevo.

### J. “El resultado LLM quedó mezclado entre asset/item/collection o desapareció”

Mirar:

- `llm_results`

```sql
SELECT id, target_id, target_type, job_type, created_at, result
FROM llm_results
WHERE target_id = 'TARGET_ID_AQUI'
ORDER BY created_at DESC;

SELECT id, target_id, target_type, job_type, created_at
FROM llm_results
WHERE created_at < 1000000000000
ORDER BY created_at ASC;
```

---

### K. “Un lote no avanza o terminó con errores”

Mirar:

- `processing_batches`
- `processing_batch_tasks` / `processing_tasks`
- `processing_attempts`

```sql
SELECT id, origin, state, desired_state, priority, last_error
FROM processing_batches
ORDER BY created_at DESC
LIMIT 10;

SELECT t.kind, t.state, COUNT(*) AS tasks
FROM processing_batch_tasks bt
JOIN processing_tasks t ON t.id = bt.task_id
WHERE bt.batch_id = 'BATCH_ID_AQUI'
GROUP BY t.kind, t.state;

SELECT t.id, t.kind, t.last_error_code, t.last_error_message
FROM processing_batch_tasks bt
JOIN processing_tasks t ON t.id = bt.task_id
WHERE bt.batch_id = 'BATCH_ID_AQUI' AND t.state = 'failed';
```

`blocked` espera a otra tarea (por ejemplo, embeddings que esperan al OCR); `retry_wait` espera un reintento; `interrupted` quedó así al cerrar la app. `dependency_failed` indica que falló la tarea de la que dependía. Un OCR que terminó sin texto cuenta como `succeeded`.

### L. “La Biblioteca no encuentra pasajes o no busca por significado”

Mirar:

- `zotero_reconciliation_runs`
- `bibliographic_extractions` / `bibliographic_page_texts`
- `bibliographic_index_generations`
- tareas `bibliography_*` en `processing_tasks`

```sql
SELECT library_id, state, phase, remote_total, latest_error_code, latest_error_message
FROM zotero_reconciliation_runs;

SELECT kind, state, last_error_code, COUNT(*) AS tasks
FROM processing_tasks
WHERE kind LIKE 'bibliography_%'
GROUP BY kind, state, last_error_code;

SELECT status, expected_inputs, completed_inputs, activated_at
FROM bibliographic_index_generations
ORDER BY created_at DESC;
```

Sin una generación `active` no hay búsqueda por significado. Tareas bloqueadas por configuración suelen indicar que falta la clave de OpenRouter (embeddings) o la de GLM-OCR (páginas escaneadas).

### M. “Un manuscrito perdió cambios”

Mirar:

- `writing_documents` (contenido y `revision` actuales)
- `writing_document_versions` (historial: `auto`, `checkpoint` antes de recibir, `restore`)
- `writing_journal` (deltas todavía no consolidados)

```sql
SELECT id, title, revision, datetime(updated_at / 1000, 'unixepoch', 'localtime') AS updated_local
FROM writing_documents
WHERE title LIKE '%TITULO%';

SELECT version_number, reason, datetime(created_at / 1000, 'unixepoch', 'localtime') AS created_local
FROM writing_document_versions
WHERE document_id = 'DOCUMENT_ID_AQUI'
ORDER BY version_number DESC;
```

Para recuperar, usá **Historial de versiones** en la app; no edites estas tablas a mano.

## Arquitectura vigente de embeddings

La verdad runtime/producto verificada hoy es esta:

- embeddings y similitud del corpus son **asset-only**
- tablas activas: `vec_assets` (un vector por asset) y `rag_chunks` (vectores por fragmento para el Chat y la Investigación), con `rag_asset_embedding_state` para los reintentos
- APIs activas: `embed_asset`, `backfill_asset_embeddings`, `similar_assets`
- la Biblioteca tiene su propio índice versionado: `bibliographic_item_embeddings` y `bibliographic_chunk_embeddings`, atados a una generación de `bibliographic_index_generations`

Si ves `vec_items`, `embed_item`, `similar_items` o `embeddings_fallback` en notas viejas o snapshots de una DB local anterior, tratálos como **legacy/archive**, no como arquitectura soportada.

## Consultas de auditoría relacional

### Assets huérfanos respecto de items

```sql
SELECT a.*
FROM assets a
LEFT JOIN items i ON i.id = a.item_id
WHERE i.id IS NULL;
```

### Items huérfanos respecto de collections

```sql
SELECT i.*
FROM items i
LEFT JOIN collections c ON c.id = i.collection_id
WHERE c.id IS NULL;
```

### Notes apuntando a assets inexistentes

```sql
SELECT n.*
FROM notes n
LEFT JOIN assets a ON a.id = n.asset_id
WHERE n.asset_id IS NOT NULL
  AND a.id IS NULL;
```

### Entities con asset_id roto

```sql
SELECT e.*
FROM entities e
LEFT JOIN assets a ON a.id = e.asset_id
WHERE e.asset_id IS NOT NULL
  AND a.id IS NULL;
```

### Triples con asset_id roto

```sql
SELECT t.*
FROM triples t
LEFT JOIN assets a ON a.id = t.asset_id
WHERE t.asset_id IS NOT NULL
  AND a.id IS NULL;
```

### Vec assets con referencias dudosas

```sql
SELECT va.*
FROM vec_assets va
LEFT JOIN assets a ON a.id = va.asset_id
LEFT JOIN items i ON i.id = va.item_id
WHERE a.id IS NULL OR i.id IS NULL;
```

### LLM results con target roto o legacy no migrado

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

## Consultas para entender cobertura del pipeline

### Qué assets tienen qué tipo de salida

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

### Qué ítems están enriquecidos semánticamente

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

## Comandos útiles de sqlite3

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

## Recomendación brutalmente práctica

Cuando algo falla, no empieces por inferencias. Empezá por evidencia:

1. `assets`
2. `extractions` / `transcriptions` / `layouts`
3. `entities` / `triples` / `topics`
4. `fts_items` / `vec_assets` / `rag_chunks`
5. `processing_tasks` / `processing_attempts`, si lo que falta lo produce un lote o una tarea en segundo plano

Es así de simple. Primero verificás persistencia. Después discutís lógica.

> Compatibilidad: si abrís una base vieja y todavía aparece `jobs`, tratala como una tabla legacy. El runtime actual no la usa y la cleanup vigente la elimina con la migración `0021_drop_unused_processing_table`.
