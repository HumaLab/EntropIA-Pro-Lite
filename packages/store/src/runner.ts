import type { DbClient } from './types'
// @ts-expect-error Node's type stripping needs the explicit .ts extension; TS bundler resolution flags it.
import { MIGRATION_FILES } from './migrations.generated.ts'

type TriggerRow = 'NEW' | 'OLD'

const activityTimestamp = `MAX(
  updated_at + 1,
  CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
)`

function touchCollections(where: string): string {
  return `UPDATE collections
SET updated_at = ${activityTimestamp}
WHERE ${where};`
}

function activityTriggers(table: string, collectionScope: (row: TriggerRow) => string): string {
  return `CREATE TRIGGER collection_activity_${table}_insert
AFTER INSERT ON ${table}
BEGIN
  ${touchCollections(collectionScope('NEW'))}
END;

CREATE TRIGGER collection_activity_${table}_update
AFTER UPDATE ON ${table}
BEGIN
  ${touchCollections(collectionScope('OLD'))}
  ${touchCollections(collectionScope('NEW'))}
END;

CREATE TRIGGER collection_activity_${table}_delete
BEFORE DELETE ON ${table}
BEGIN
  ${touchCollections(collectionScope('OLD'))}
END;`
}

const itemCollectionScope = (row: TriggerRow) => `id = ${row}.collection_id`
const itemOwnedScope = (row: TriggerRow) =>
  `id IN (SELECT collection_id FROM items WHERE id = ${row}.item_id)`
const assetOwnedScope = (row: TriggerRow) =>
  `id IN (
  SELECT i.collection_id
  FROM items i
  JOIN assets a ON a.item_id = i.id
  WHERE a.id = ${row}.asset_id
)`
const llmResultScope = (row: TriggerRow) =>
  `(${row}.target_type IN ('collection', 'unknown') AND id = ${row}.target_id)
  OR (${row}.target_type IN ('item', 'unknown') AND id IN (
    SELECT collection_id FROM items WHERE id = ${row}.target_id
  ))
  OR (${row}.target_type IN ('asset', 'unknown') AND id IN (
    SELECT i.collection_id
    FROM items i
    JOIN assets a ON a.item_id = i.id
    WHERE a.id = ${row}.target_id
  ))`

export const COLLECTION_ACTIVITY_DDL = `
-- Keep collections.updated_at as the canonical last-activity timestamp. Database
-- triggers cover repository writes, Rust worker writes, and synchronized changes.
${activityTriggers('items', itemCollectionScope)}

${activityTriggers('assets', itemOwnedScope)}

${activityTriggers('notes', itemOwnedScope)}

${activityTriggers('extractions', assetOwnedScope)}

${activityTriggers('layouts', assetOwnedScope)}

${activityTriggers('transcriptions', assetOwnedScope)}

${activityTriggers('annotations', assetOwnedScope)}

${activityTriggers('entities', itemOwnedScope)}

${activityTriggers('triples', itemOwnedScope)}

${activityTriggers('vec_assets', itemOwnedScope)}

${activityTriggers('llm_results', llmResultScope)}
`.trim()

// ---------------------------------------------------------------------------
// Migration registry — generated from src/migrations/*.sql, the single source.
// Regenerate with: pnpm --filter @entropia/store generate-migrations
// The Tauri webview cannot read the filesystem, so MIGRATION_FILES inlines the
// exact bytes of every file; the SQL below is that content minus its final
// newline, matching the `.trim()`ed values the registry used to hold inline.
// ---------------------------------------------------------------------------
export const MIGRATIONS: Record<string, string> = {
  ...Object.fromEntries(
    Object.entries(MIGRATION_FILES).map(([name, sql]) => [name, sql.replace(/\n$/, '')])
  ),
  // 0020 is applied programmatically by applyLayoutsMigration() below; this
  // no-op marker keeps its registry row. 0020_layouts.sql documents the DDL a
  // fresh install ends up with (LAYOUTS_DDL).
  '0020_layouts': `
-- Handled programmatically in runMigrations() because existing desktop databases
-- may already contain a legacy layouts table without the blocks column.
CREATE TEMP TABLE IF NOT EXISTS __entropia_migration_0020_noop (id INTEGER);
DROP TABLE IF EXISTS __entropia_migration_0020_noop
  `.trim(),
}

// Objects the atomic 0032 batch creates. A database holding all of them but
// no 0032 registry row was burned by the earlier non-atomic registration:
// repair the row instead of re-running DDL that would collide.
const PROCESSING_0032_SENTINELS = [
  'processing_batches',
  'processing_batch_members',
  'processing_tasks',
  'processing_batch_tasks',
  'processing_requests',
  'processing_attempts',
  'processing_checkpoints',
  'processing_asset_revisions',
  'processing_meta',
  'trg_processing_extractions_ai',
] as const

// Every object 0032 creates, children first so the DROPs in a partial-state
// repair never trip a foreign key. Superset of the sentinels above: the
// sentinels answer "was this applied?", this list answers "what has to go?".
const PROCESSING_0032_TABLES_CHILD_FIRST = [
  'processing_asset_revisions',
  'processing_checkpoints',
  'processing_attempts',
  'processing_requests',
  'processing_batch_tasks',
  'processing_tasks',
  'processing_batch_members',
  'processing_batch_collections',
  'processing_batches',
  'processing_meta',
] as const

const PROCESSING_0032_TRIGGERS = [
  'trg_processing_extractions_ai',
  'trg_processing_extractions_au',
  'trg_processing_extractions_ad',
  'trg_processing_transcriptions_ai',
  'trg_processing_transcriptions_au',
  'trg_processing_transcriptions_ad',
] as const

/**
 * Total rows held by whichever 0032 tables survive in a partial state.
 *
 * A partial 0032 is only safe to replay when the queue never ran: the repair
 * drops those tables, and a drop is unrecoverable. Counting first turns
 * "probably empty" into a fact before anything is destroyed.
 */
async function countSurvivingProcessingRows(client: DbClient): Promise<number> {
  const placeholders = PROCESSING_0032_TABLES_CHILD_FIRST.map(() => '?').join(',')
  const present = await client.select<{ name: string }>(
    `SELECT name FROM sqlite_master WHERE type = 'table' AND name IN (${placeholders})`,
    [...PROCESSING_0032_TABLES_CHILD_FIRST]
  )
  if (present.length === 0) return 0
  // The names come from the constant above, never from user input: the query
  // above only decides WHICH of those fixed names are still there to count.
  const union = present.map((row) => `SELECT COUNT(*) AS n FROM ${row.name}`).join(' UNION ALL ')
  const totals = await client.select<{ n: number | bigint | null }>(
    `SELECT SUM(n) AS n FROM (${union})`
  )
  return Number(totals[0]?.n ?? 0)
}

/**
 * Canonical final shape of the `layouts` table on a fresh install, derived from
 * `0020_layouts.sql` (its exact content minus the file's final newline). Exported
 * so the schema-fixture export script can reproduce it without replaying the
 * legacy-migration branches inside {@link applyLayoutsMigration}.
 */
export const LAYOUTS_DDL: string = MIGRATION_FILES['0020_layouts']!.replace(/\n$/, '')

async function applyLayoutsMigration(client: DbClient): Promise<void> {
  const now = Date.now()
  await client.executeBatch(`
    CREATE TABLE IF NOT EXISTS layouts (
      id TEXT PRIMARY KEY,
      asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
      regions TEXT NOT NULL,
      blocks TEXT NOT NULL DEFAULT '[]',
      model TEXT NOT NULL,
      image_width INTEGER NOT NULL,
      image_height INTEGER NOT NULL,
      created_at INTEGER NOT NULL
    )
  `)

  try {
    await client.executeBatch("ALTER TABLE layouts ADD COLUMN blocks TEXT NOT NULL DEFAULT '[]'")
  } catch (error) {
    const message = String(error).toLowerCase()
    if (!message.includes('duplicate column name')) {
      throw error
    }
  }

  await client.execute(`
    DELETE FROM layouts
    WHERE rowid NOT IN (
      SELECT MAX(rowid) FROM layouts GROUP BY asset_id
    )
  `)
  await client.executeBatch(
    'CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id)'
  )
  await client.executeBatch('CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id)')
  await client.execute(`
    UPDATE layouts
    SET created_at = ${now}
    WHERE created_at IS NULL OR created_at = 0
  `)
}

/**
 * Builds the full application schema as a single SQL string, in migration order,
 * for use as a fixture by the Rust sync tests. Reproduces what a FRESH install
 * ends up with after `applyMigrations`:
 *
 * - every registry migration's SQL, applied in sorted (lexicographic) order;
 * - the `0020_layouts` registry entry (a no-op marker) is replaced by the
 *   canonical programmatic {@link LAYOUTS_DDL}, mirroring how
 *   {@link applyLayoutsMigration} runs imperatively at runtime.
 *
 * Deterministic so a checked-in artifact can be diffed against it (see
 * `scripts/export-schema.mjs` and the matching vitest freshness test).
 */
export function buildSchemaFixture(): string {
  const header = [
    '-- AUTO-GENERATED schema fixture for the Rust sync tests. DO NOT EDIT BY HAND.',
    '-- Regenerate with: pnpm --filter @entropia/store export-schema',
    '-- Source of truth: packages/store/src/runner.ts (MIGRATIONS + LAYOUTS_DDL).',
    '',
  ].join('\n')

  const sections = Object.keys(MIGRATIONS)
    .sort()
    .map((name) => {
      const sql = (name === '0020_layouts' ? LAYOUTS_DDL : MIGRATIONS[name]!).trim()
      // Several registry entries omit the trailing semicolon on their last
      // statement (harmless at runtime — the runner splits per migration on
      // ';'). The fixture is fed to one execute_batch, so each section must be
      // self-terminated or the next migration's CREATE runs into it.
      const terminated = sql.endsWith(';') ? sql : `${sql};`
      return `-- ${name}\n${terminated}`
    })

  return `${header}\n${sections.join('\n\n')}\n`
}

/**
 * Split a multi-statement SQL string into individual statements.
 * Strips comments and empty lines, splits on semicolons.
 */
function splitStatements(sql: string): string[] {
  return sql
    .split(';')
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
}

/**
 * Reference TypeScript migration applier: runs all pending migrations in
 * filename order and records each one in `_migrations`.
 *
 * This is NOT the desktop runtime path anymore. Since Phase 4.2c/4.3 the Rust
 * backend applies every migration during setup, before the webview loads, so
 * the renderer only verifies with {@link runMigrations}. Tests and tooling that
 * need a schema built on `node:sqlite` use this applier: the canonical schema
 * fixture (`schema-canonical.ts`) and the migration tests in `runner.test.ts`.
 *
 * - Ensures the `_migrations` tracking table exists
 * - Fetches already-applied migration names
 * - Applies each pending migration inside a BEGIN/COMMIT transaction
 * - Inserts a record into `_migrations` on success
 * - On error: rolls back the transaction and rethrows with the migration name
 */
export async function applyMigrations(client: DbClient): Promise<void> {
  // Ensure tracking table exists (idempotent).
  // DDL must go through the batch path: the Tauri db_execute IPC is hardened
  // to DML-only (INSERT/UPDATE/DELETE), so schema statements are rejected there.
  // db_execute_batch validates the batch and runs CREATE/ALTER/DROP.
  await client.executeBatch(`
    CREATE TABLE IF NOT EXISTS _migrations (
      id    INTEGER PRIMARY KEY AUTOINCREMENT,
      name  TEXT    NOT NULL UNIQUE,
      applied_at INTEGER NOT NULL
    )
  `)

  // Fetch already-applied migrations
  const applied = await client.select<{ name: string }>('SELECT name FROM _migrations ORDER BY id')
  const appliedSet = new Set(applied.map((row) => row.name))

  // Sort migration keys by filename order (lexicographic)
  const pending = Object.keys(MIGRATIONS)
    .sort()
    .filter((name) => !appliedSet.has(name))

  for (const name of pending) {
    // Statements that clear a half-applied 0032 before it is replayed. They
    // ride inside the migration's own BEGIN/COMMIT so the repair is atomic.
    let repairDrops = ''
    try {
      if (name === '0032_batch_processing') {
        const placeholders = PROCESSING_0032_SENTINELS.map(() => '?').join(',')
        const present = await client.select<{ cnt: number | bigint }>(
          `SELECT COUNT(*) AS cnt FROM sqlite_master WHERE name IN (${placeholders})`,
          [...PROCESSING_0032_SENTINELS]
        )
        const found = Number(present[0]?.cnt ?? 0)
        if (found === PROCESSING_0032_SENTINELS.length) {
          // Burned by the earlier non-atomic registration: the whole 0032
          // object set exists without the registry row. Repair the row —
          // re-running the DDL would collide on the first CREATE TABLE.
          await client.execute('INSERT INTO _migrations (name, applied_at) VALUES (?, ?)', [
            name,
            Math.floor(Date.now() / 1000),
          ])
          continue
        }
        if (found > 0) {
          // A PARTIAL set. The first build to ship 0032 split the migration
          // on ';', which shreds the CREATE TRIGGER … BEGIN … END; bodies, and
          // ran each fragment on its own autocommit: everything before the
          // first trigger stuck, the rest never ran, no registry row.
          //
          // Those objects are pure new schema, so replaying is safe as long as
          // nothing was ever queued into them. Prove that, then drop and
          // replay inside the migration's own transaction — refusing outright
          // leaves the app unable to start with nowhere to go.
          const occupied = await countSurvivingProcessingRows(client)
          if (occupied > 0) {
            throw new Error(
              `incomplete 0032 state: ${found}/${PROCESSING_0032_SENTINELS.length} objects exist without a registry row and still hold ${occupied} queue row(s); restore from a backup before retrying`
            )
          }
          repairDrops = [
            ...PROCESSING_0032_TRIGGERS.map((trigger) => `DROP TRIGGER IF EXISTS ${trigger};`),
            ...PROCESSING_0032_TABLES_CHILD_FIRST.map((table) => `DROP TABLE IF EXISTS ${table};`),
          ].join('\n')
        }
      }
      if (name === '0020_layouts') {
        await applyLayoutsMigration(client)
      } else if (
        name === '0025_document_view_edits' ||
        name === '0027_collection_activity' ||
        name === '0029_rag_chunks' ||
        name === '0032_batch_processing' ||
        name === '0034_fts_contentless_delete' ||
        name === '0035_writing_workspace' ||
        name === '0036_writing_journal' ||
        name === '0038_processing_settle_on_terminal' ||
        name === '0040_bibliography_catalog' ||
        name === '0041_bibliography_relations' ||
        name === '0042_bibliography_reconciliation' ||
        name === '0043_processing_task_subject_identity' ||
        name === '0044_processing_task_subject_cutover' ||
        name === '0045_bibliography_sync_tasks' ||
        name === '0046_processing_priority' ||
        name === '0047_bibliographic_semantic_profiles' ||
        name === '0048_bibliography_profile_tasks' ||
        name === '0049_bibliographic_index_generations' ||
        name === '0050_bibliographic_embedding_generations' ||
        name === '0051_bibliographic_profile_fts' ||
        name === '0052_bibliographic_extraction_tasks' ||
        name === '0053_bibliographic_page_texts' ||
        name === '0054_bibliographic_chunks' ||
        name === '0055_bibliographic_chunk_embeddings' ||
        name === '0056_bibliographic_ingest_operations' ||
        name === '0057_web_captures' ||
        name === '0058_processing_ner_tasks' ||
        name === '0059_zotero_annotations'
      ) {
        const appliedAt = Math.floor(Date.now() / 1000)
        const escapedName = name.replaceAll("'", "''")
        const repairPrefix = repairDrops ? `${repairDrops}\n` : ''
        await client.executeBatch(
          `BEGIN IMMEDIATE;\n${repairPrefix}${MIGRATIONS[name]!}\nINSERT INTO _migrations (name, applied_at) VALUES ('${escapedName}', ${appliedAt});\nCOMMIT;`
        )
        continue
      } else {
        const sql = MIGRATIONS[name]!
        const statements = splitStatements(sql)

        // Schema statements (CREATE/ALTER/DROP) go through executeBatch because
        // the db_execute IPC is restricted to DML-only. db_execute_batch is the
        // sanctioned path for DDL and also accepts the DML inside these batches.
        for (const stmt of statements) {
          try {
            await client.executeBatch(stmt)
          } catch (stmtErr) {
            const msg = stmtErr instanceof Error ? stmtErr.message : String(stmtErr)
            if (/duplicate column name/i.test(msg)) {
              continue
            }
            throw stmtErr
          }
        }
      }

      // Tracking insert stays on the DML path (db_execute).
      await client.execute('INSERT INTO _migrations (name, applied_at) VALUES (?, ?)', [
        name,
        Math.floor(Date.now() / 1000),
      ])
    } catch (error) {
      try {
        await client.executeBatch('ROLLBACK;')
      } catch {
        // A failed BEGIN or a disconnected client has no transaction to roll back.
      }
      throw new Error(
        `Migration "${name}" failed: ${error instanceof Error ? error.message : String(error)}`
      )
    }
  }
}

/**
 * Verify-only migration gate for the renderer (Phase 4.4a).
 *
 * The Rust backend has already applied every migration when the webview loads
 * (Phase 4.2c/4.3), so this only reads `_migrations` and throws when any
 * registry migration is missing: that is the signal the backend did not
 * migrate. It performs no write of any kind.
 *
 * A missing `_migrations` table is treated as an empty registry — the backend
 * never migrated. Names present in `_migrations` but absent from
 * {@link MIGRATIONS} are ignored on purpose: a newer app version may have
 * written migrations this build does not know about, and a downgrade must
 * still start (the old applying runner ignored them too).
 */
export async function runMigrations(client: DbClient): Promise<void> {
  let appliedNames: string[]
  try {
    const rows = await client.select<{ name: string }>('SELECT name FROM _migrations ORDER BY id')
    appliedNames = rows.map((row) => row.name)
  } catch (error) {
    if (!/no such table.*_migrations/i.test(String(error))) {
      throw error
    }
    appliedNames = []
  }

  const applied = new Set(appliedNames)
  const missing = Object.keys(MIGRATIONS)
    .sort()
    .filter((name) => !applied.has(name))

  if (missing.length > 0) {
    throw new Error(`Archive is missing migrations applied by the backend: ${missing.join(', ')}`)
  }
}
