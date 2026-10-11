// Exports the canonical schema dump (the Phase 4 migration equivalence oracle)
// into the Rust test fixture consumed by db::canonical_dump. Run with:
// pnpm --filter @entropia/store export-schema-canonical
//
// Node v24+ strips types when importing the .ts source directly, so this stays
// dependency-free (no tsx/ts-node).

import { writeFile, mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { buildCanonicalSchemaFixture } from '../src/schema-canonical.ts'

const here = dirname(fileURLToPath(import.meta.url))
const fixturePath = resolve(
  here,
  '../../../apps/desktop/src-tauri/tests/fixtures/schema_canonical.json'
)

const json = await buildCanonicalSchemaFixture()
await mkdir(dirname(fixturePath), { recursive: true })
await writeFile(fixturePath, json, 'utf8')

console.log(`Wrote ${json.length} bytes to ${fixturePath}`)
