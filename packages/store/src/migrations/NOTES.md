# Migration notes

Rationale kept from the inline registry comments that used to live in
`src/runner.ts`. Since Phase 4.1b the `.sql` files under `src/migrations/` are
the single source of truth and the registry is generated from them
(`scripts/generate-migrations.mjs`), so anything that only explains _why_ a
migration looks the way it does lives here instead of inside the applied SQL.

The now-obsolete "NNNN mirrors packages/store/src/migrations/NNNN\_\*.sql — keep
both identical" sentences were dropped with the inline copies. Migrations whose
registry comment only carried that sentence (0037, 0038, 0039, 0042) have no
entry here.

## 0032_batch_processing

Contains triggers, so it also belongs to the trigger-safe single-batch branch in
`runMigrations()`.

## 0033_processing_source_invalidation

Durable per-cycle source-invalidation counter (plan-lote.md §3.3 — the third
mid-flight input change blocks the task as `source_unstable`). Additive over the
historical 0032 shape; never edit an applied migration.

## 0034_fts_contentless_delete

The FTS index is contentless, so without `contentless_delete=1` SQLite cannot
find a replaced row's old tokens — re-running OCR or correcting an extraction
left the previous text searchable forever. The option is creation-only, so the
index is rebuilt from the same source-of-truth query 0018 established. SQLite
3.43+ (bundled: 3.45).

## 0040_bibliography_catalog

This is the foundation only: later migrations own collections, tags,
attachments, tombstones and reconciliation state.

## 0041_bibliography_relations

Relational catalog entities use composite membership foreign keys so an item
cannot be associated with a collection or tag from another library.

## 0043_processing_task_subject_identity

Additive task-subject identity columns (E2a-1): corpus/asset dual-write columns
plus the parallel composite partial unique. The old snapshot unique stays until
the E2a-2 cutover. Applied in the trigger-safe single-batch branch in
`runMigrations()`; the backfill must run before the unique index is created (see
the file header for the order and replay contract).

## 0044_processing_task_subject_cutover

Single-flight cutover (E2a-2): drops the snapshot-scoped partial unique so the
composite built in 0043 becomes the sole authority. Applied in the trigger-safe
single-batch branch in `runMigrations()`; `DROP INDEX IF EXISTS` makes it
replay-safe by itself.

## 0045_bibliography_sync_tasks

Bibliographic task admission (E2b-1): widens the kind CHECK on
processing_tasks/processing_batch_tasks to 'bibliography_sync' and the origin
CHECK on processing_batches to 'bibliography'. SQLite cannot ALTER a CHECK, so
the three tables are rebuilt preserving every column, both task indexes, every
FK, and every row byte-identically (dependents are backed up and recreated
verbatim). Applied in the trigger-safe single-batch branch in `runMigrations()`
like 0032/0043/0044.

## 0046_processing_priority

Per-batch interactive priority (E2c-WU3): one additive column plus an index.
Runs through the trigger-safe single-batch path in `runMigrations()` like
0032/0043/0044/0045.

## 0047_bibliographic_semantic_profiles

Per-work semantic profiles (E3b-WU1): one canonical-text row per verified work.
Runs through the trigger-safe single-batch path in `runMigrations()` like
0032/0043/0044/0045/0046.

## 0048_bibliography_profile_tasks

Per-work profile tasks (E3b-WU2): widens the kind CHECK to bibliography_profile
via the 0045-style table rebuild and adds the per-(work, contract) embedding
table. Trigger-safe single-batch path.

## 0049_bibliographic_index_generations

Index generations (E3c-WU1): immutable contract rows plus generation lifecycle
with a per-contract active pointer. Trigger-safe single-batch.

## 0050_bibliographic_embedding_generations

Generation identity on work embeddings (E3c-WU2): primary key moves to (item,
generation) with honest retired legacy ancestry for pre-existing rows.
Trigger-safe single-batch.

## 0051_bibliographic_profile_fts

Lexical search over work profiles (E3c-WU3): FTS5 with transactional triggers.
Trigger-safe single-batch.

## 0052_bibliographic_extraction_tasks

Native extraction tasks and rows (E4a-WU2): kind CHECK widening plus one
whole-document native-text row per attachment. Trigger-safe single-batch.

## 0053_bibliographic_page_texts

Per-page native texts (E4b-WU2): one row per attachment page with its own hash
and quality. Trigger-safe single-batch.

## 0054_bibliographic_chunks

Structural work chunks and spans (E4c-WU1). Trigger-safe single-batch.

## 0055_bibliographic_chunk_embeddings

Chunk vectors per generation (E4c-WU2). Trigger-safe single-batch.

## 0056_bibliographic_ingest_operations

Durable pending tray (E5a-WU1). Trigger-safe single-batch.

## 0057_web_captures

Written as 0055 on the browser branch and renumbered when it merged after the
Zotero range (0040-0056): the sync schema tag is the highest applied name, so a
new migration must sort above every one already on main.

## 0058_processing_ner_tasks

Kind CHECK widening for batch NER/triples. Trigger-safe single-batch.
