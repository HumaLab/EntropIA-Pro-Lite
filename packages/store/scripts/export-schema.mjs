// Exports the full application schema (the JS migration registry plus the
// programmatic layouts DDL) into the Rust test fixture consumed by the sync
// module tests. Run with: pnpm --filter @entropia/store export-schema
//
// Node v24+ strips types when importing the .ts source directly, so this stays
// dependency-free (no tsx/ts-node).

import { writeFile, mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { buildSchemaFixture, COLLECTION_ACTIVITY_DDL } from '../src/runner.ts'

const here = dirname(fileURLToPath(import.meta.url))
const fixturePath = resolve(here, '../../../apps/desktop/src-tauri/tests/fixtures/schema_full.sql')
const collectionActivityMigrationPath = resolve(
  here,
  '../src/migrations/0027_collection_activity.sql'
)

const sql = buildSchemaFixture()
await mkdir(dirname(fixturePath), { recursive: true })
await writeFile(fixturePath, sql, 'utf8')
// Byte-identical to the registry entry (migration-files.test.ts), so no header.
await writeFile(collectionActivityMigrationPath, `${COLLECTION_ACTIVITY_DDL}\n`, 'utf8')

console.log(`Wrote ${sql.length} bytes to ${fixturePath}`)
console.log(`Wrote collection activity migration to ${collectionActivityMigrationPath}`)
