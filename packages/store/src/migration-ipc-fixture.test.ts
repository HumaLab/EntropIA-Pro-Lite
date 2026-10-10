import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { runMigrations } from './runner'
import {
  appliedMigrationNames,
  recordFreshInstallMigrationIpc,
  recordMigrationIpcFixtureJson,
} from './migration-ipc-recorder'

const here = dirname(fileURLToPath(import.meta.url))
const fixturePath = resolve(here, '../../../apps/desktop/src-tauri/tests/fixtures/migration_ipc.json')

// Normalize CRLF so the check is stable across platforms / git autocrlf.
const norm = (s: string) => s.replace(/\r\n/g, '\n')

describe('migration IPC fixture export', () => {
  it('the checked-in Rust fixture is up to date with the migration runner', async () => {
    const expected = await recordMigrationIpcFixtureJson(runMigrations)
    let actual: string
    try {
      actual = readFileSync(fixturePath, 'utf8')
    } catch {
      throw new Error(
        `Missing migration IPC fixture at ${fixturePath}. Run: pnpm --filter @entropia/store export-migration-ipc`
      )
    }

    expect(
      norm(actual),
      'Stale fixture — run: pnpm --filter @entropia/store export-migration-ipc'
    ).toBe(norm(expected))
  })

  it('records a fresh install that applies every migration through 0058_processing_ner_tasks', async () => {
    const calls = await recordFreshInstallMigrationIpc(runMigrations)
    const names = appliedMigrationNames(calls)

    expect(names.at(-1)).toBe('0058_processing_ner_tasks')
    expect(new Set(names).size).toBe(names.length)
  })

  it('captures the 0032 repair drops separately from the fresh-install calls', async () => {
    const json = await recordMigrationIpcFixtureJson(runMigrations)
    const fixture = JSON.parse(json) as { repair_drops: string[] }

    expect(fixture.repair_drops.length).toBeGreaterThan(0)
    expect(fixture.repair_drops.every((sql) => sql.endsWith(';'))).toBe(true)
    expect(fixture.repair_drops.some((sql) => sql.startsWith('DROP TRIGGER IF EXISTS'))).toBe(true)
    expect(fixture.repair_drops.some((sql) => sql.startsWith('DROP TABLE IF EXISTS'))).toBe(true)
  })
})
