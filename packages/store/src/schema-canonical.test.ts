import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { beforeAll, describe, it, expect } from 'vitest'
import { buildCanonicalSchemaFixture, type CanonicalSchemaDump } from './schema-canonical'

const here = dirname(fileURLToPath(import.meta.url))
const fixturePath = resolve(
  here,
  '../../../apps/desktop/src-tauri/tests/fixtures/schema_canonical.json'
)

describe('canonical schema fixture export', () => {
  let expected: string
  let dump: CanonicalSchemaDump

  beforeAll(async () => {
    expected = await buildCanonicalSchemaFixture()
    dump = JSON.parse(expected) as CanonicalSchemaDump
  })

  it('the checked-in Rust fixture is up to date with the TypeScript runner', () => {
    let actual: string
    try {
      actual = readFileSync(fixturePath, 'utf8')
    } catch {
      throw new Error(
        `Missing canonical schema fixture at ${fixturePath}. Run: pnpm --filter @entropia/store export-schema-canonical`
      )
    }

    // Normalize CRLF so the check is stable across platforms / git autocrlf.
    const norm = (s: string) => s.replace(/\r\n/g, '\n')
    expect(
      norm(actual),
      'Stale fixture — run: pnpm --filter @entropia/store export-schema-canonical'
    ).toBe(norm(expected))
  })

  it('records every migration name in order, ending with 0058_processing_ner_tasks', () => {
    expect(dump.migrations).toHaveLength(58)
    expect(dump.migrations.at(-1)).toBe('0058_processing_ner_tasks')
  })

  it('captures base tables, the FTS virtual table and the collection activity triggers', () => {
    const names = dump.objects.map((object) => object.name)
    expect(names).toContain('items')
    expect(names).toContain('fts_items')
    expect(names).toContain('collection_activity_items_insert')
  })

  it('excludes SQLite-internal names and the sync capture schema', () => {
    const names = dump.objects.map((object) => object.name)
    expect(names.filter((name) => name.startsWith('sqlite_'))).toEqual([])
    // Sync capture is installed by `ensure_capture`, after migrations; if it
    // ever leaks into this fixture the oracle is wrong and the Rust
    // cross-check would hide it.
    expect(names.filter((name) => name.startsWith('trg_sync_'))).toEqual([])
  })
})
