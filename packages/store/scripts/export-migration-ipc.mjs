// Exports the recorded migration IPC sequence (fresh install + the 0032
// repair drops) into the Rust fixture consumed by the S-02c tests. Run with:
// pnpm --filter @entropia/store export-migration-ipc
//
// The recording itself lives in src/migration-ipc-recorder.ts and is shared
// with the vitest drift guard, so the fixture and the test can never disagree
// about what `runMigrations` sends. Node v24+ strips types when importing the
// .ts sources directly, so this stays dependency-free.

import { writeFile, mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { runMigrations } from '../src/runner.ts'
import { recordMigrationIpcFixtureJson } from '../src/migration-ipc-recorder.ts'

const here = dirname(fileURLToPath(import.meta.url))
const fixturePath = resolve(here, '../../../apps/desktop/src-tauri/tests/fixtures/migration_ipc.json')

const json = await recordMigrationIpcFixtureJson(runMigrations)
await mkdir(dirname(fixturePath), { recursive: true })
await writeFile(fixturePath, json, 'utf8')

const fixture = JSON.parse(json)
const batches = fixture.calls.filter((call) => call.command === 'db_execute_batch').length

console.log(`Wrote ${json.length} bytes to ${fixturePath}`)
console.log(
  `Recorded ${fixture.calls.length} IPC calls (${batches} db_execute_batch) and ${fixture.repair_drops.length} repair drops`
)
