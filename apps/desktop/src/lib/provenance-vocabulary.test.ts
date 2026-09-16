import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The provenance vocabulary, held equal to the migration that enforces it.
 *
 * `writing_provenance_events` constrains `origin_type` and `operation_type`
 * with a `CHECK`, and SQLite only complains when the transaction runs — by
 * which time the save has already failed in front of the writer. That is
 * exactly how `insert_citation` reached production: a value nobody had checked
 * against the column that would refuse it.
 *
 * The union in `writing.ts` is what stops a bad literal compiling. This is what
 * stops the union drifting from the column.
 */

const MIGRATION = readFileSync(
  resolve(import.meta.dirname, '../../../../packages/store/src/migrations/0035_writing_workspace.sql'),
  'utf-8'
)
const SOURCE = readFileSync(resolve(import.meta.dirname, 'writing.ts'), 'utf-8')

/** The values a named CHECK allows, read out of the migration. */
function allowedBySql(column: string): string[] {
  const at = MIGRATION.indexOf(`CHECK (${column} IN (`)
  expect(at, `no CHECK for ${column}`).toBeGreaterThan(-1)
  const list = MIGRATION.slice(at + `CHECK (${column} IN (`.length)
  return [...list.slice(0, list.indexOf(')')).matchAll(/'([a-z_]+)'/g)].map(([, value]) => value!)
}

/** The members of a union declared in the store. */
function declaredInTypeScript(name: string): string[] {
  const at = SOURCE.indexOf(`export type ${name} =`)
  expect(at, `no type ${name}`).toBeGreaterThan(-1)
  const line = SOURCE.slice(at, SOURCE.indexOf('\n', at))
  return [...line.matchAll(/'([a-z_]+)'/g)].map(([, value]) => value!)
}

describe('the provenance vocabulary', () => {
  it('offers exactly the origins the column accepts', () => {
    expect(declaredInTypeScript('ProvenanceOrigin').sort()).toEqual(allowedBySql('origin_type').sort())
  })

  it('offers exactly the operations the column accepts', () => {
    expect(declaredInTypeScript('ProvenanceOperation').sort()).toEqual(
      allowedBySql('operation_type').sort()
    )
  })

  /** The value that caused the failure must not be accepted by either side. */
  it('does not accept the value that failed', () => {
    expect(allowedBySql('operation_type')).not.toContain('insert_citation')
    expect(declaredInTypeScript('ProvenanceOperation')).not.toContain('insert_citation')
  })
})
