import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, it, expect } from 'vitest'
import { LAYOUTS_DDL, MIGRATIONS } from './runner'

const migrationsDir = resolve(dirname(fileURLToPath(import.meta.url)), 'migrations')

/** Applied programmatically by `applyLayoutsMigration`; its file holds the final DDL. */
const PROGRAMMATIC_MIGRATION = '0020_layouts'

function readMigrationFile(name: string): string {
  return readFileSync(resolve(migrationsDir, `${name}.sql`), 'utf8')
}

describe('migration .sql files', () => {
  it('has exactly one file per registry entry', () => {
    const files = readdirSync(migrationsDir)
      .filter((file) => file.endsWith('.sql'))
      .map((file) => file.slice(0, -'.sql'.length))
      .sort()
    expect(files).toEqual(Object.keys(MIGRATIONS).sort())
  })

  it.each(Object.keys(MIGRATIONS).filter((name) => name !== PROGRAMMATIC_MIGRATION))(
    '%s.sql is byte-identical to the SQL the registry applies',
    (name) => {
      expect(readMigrationFile(name)).toBe(`${MIGRATIONS[name]}\n`)
    }
  )

  it('0020_layouts.sql holds the final layouts DDL a fresh install gets', () => {
    expect(readMigrationFile(PROGRAMMATIC_MIGRATION)).toBe(`${LAYOUTS_DDL}\n`)
  })
})
