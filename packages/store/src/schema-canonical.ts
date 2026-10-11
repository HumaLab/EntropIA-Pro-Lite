// Canonical schema dump — the equivalence oracle for the Phase 4 Rust migration
// runner (odd/plans/plan-migraciones-rust.md §3).
//
// A fresh database migrated by the TypeScript runner is dumped here; the Rust
// side (`apps/desktop/src-tauri/src/db/canonical_dump.rs`) computes the same
// structure from a `rusqlite::Connection`, so 4.2b can assert that an empty
// database migrated in Rust equals this one. The JSON shape is part of the
// contract: keep both sides (and their field names) in lockstep.

import { DatabaseSync, type SQLInputValue, type SQLOutputValue } from 'node:sqlite'
// @ts-expect-error Node's type stripping needs the explicit .ts extension; TS bundler resolution flags it.
import { applyMigrations } from './runner.ts'
import type { DbClient } from './types'

export interface CanonicalSchemaObject {
  type: string
  name: string
  tbl_name: string
  sql: string | null
}

export interface CanonicalSchemaColumn {
  cid: number
  name: string
  type: string
  notnull: number
  dflt_value: string | null
  pk: number
  hidden: number
}

export interface CanonicalSchemaForeignKey {
  id: number
  seq: number
  table: string
  from: string
  to: string | null
  on_update: string
  on_delete: string
  match: string
}

export interface CanonicalSchemaTable {
  name: string
  columns: CanonicalSchemaColumn[]
  foreign_keys: CanonicalSchemaForeignKey[]
  // Set when the PRAGMA itself fails (exotic virtual tables); the corresponding
  // list stays empty so the failure is recorded instead of silently skipped.
  columns_error: string | null
  foreign_keys_error: string | null
}

export interface CanonicalSchemaIndexColumn {
  seqno: number
  cid: number
  name: string | null
  desc: number
  coll: string | null
  key: number
}

export interface CanonicalSchemaIndex {
  name: string
  columns: CanonicalSchemaIndexColumn[]
  columns_error: string | null
}

export interface CanonicalSchemaDump {
  objects: CanonicalSchemaObject[]
  tables: CanonicalSchemaTable[]
  indexes: CanonicalSchemaIndex[]
  migrations: string[]
}

/**
 * `DbClient` over a synchronous `node:sqlite` connection. Mirrors the inline
 * adapter used by the runner tests so the export script and the freshness test
 * exercise the reference migration applier without a Tauri IPC hop.
 */
export function createNodeSqliteClient(db: DatabaseSync): DbClient {
  return {
    async execute(sql, params = []) {
      return { rowsAffected: Number(db.prepare(sql).run(...(params as SQLInputValue[])).changes) }
    },
    async executeBatch(sql) {
      db.exec(sql)
    },
    async select<T>(sql: string, params: unknown[] = []) {
      return db.prepare(sql).all(...(params as SQLInputValue[])) as T[]
    },
    async selectRows(sql, params = []) {
      return db
        .prepare(sql)
        .all(...(params as SQLInputValue[]))
        .map(Object.values)
    },
  }
}

function quoteIdentifier(name: string): string {
  return `"${name.replaceAll('"', '""')}"`
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** One row of a PRAGMA result, keyed by the PRAGMA's column names. */
type PragmaRow = Record<string, SQLOutputValue>

/** PRAGMA rows for one object; a failure is data, not an exception. */
function readPragma(db: DatabaseSync, sql: string): { rows: PragmaRow[]; error: string | null } {
  try {
    return { rows: db.prepare(sql).all(), error: null }
  } catch (error) {
    return { rows: [], error: errorMessage(error) }
  }
}

/**
 * Full dump of the observable schema: every `sqlite_master` object (minus
 * SQLite-internal `sqlite_%` names) plus the column, foreign-key and index
 * details needed to distinguish two databases. `sqlite_master.sql` is kept
 * verbatim; `_migrations` contributes names only, so two runs on different days
 * produce the same JSON.
 */
export function canonicalSchemaDump(db: DatabaseSync): CanonicalSchemaDump {
  const objects = (
    db
      .prepare(
        `SELECT type, name, tbl_name, sql FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%'
         ORDER BY type, name`
      )
      .all() as Array<{ type: string; name: string; tbl_name: string; sql: string | null }>
  ).map((row) => ({
    type: String(row.type),
    name: String(row.name),
    tbl_name: String(row.tbl_name),
    sql: row.sql === null ? null : String(row.sql),
  }))

  const tables: CanonicalSchemaTable[] = []
  const indexes: CanonicalSchemaIndex[] = []
  for (const object of objects) {
    if (object.type === 'table') {
      const quoted = quoteIdentifier(object.name)
      const columnRows = readPragma(db, `PRAGMA table_xinfo(${quoted})`)
      const foreignKeyRows = readPragma(db, `PRAGMA foreign_key_list(${quoted})`)
      tables.push({
        name: object.name,
        columns: columnRows.rows.map((row) => ({
          cid: Number(row.cid),
          name: String(row.name),
          type: String(row.type),
          notnull: Number(row.notnull),
          dflt_value: row.dflt_value === null ? null : String(row.dflt_value),
          pk: Number(row.pk),
          hidden: Number(row.hidden),
        })),
        foreign_keys: foreignKeyRows.rows
          .map((row) => ({
            id: Number(row.id),
            seq: Number(row.seq),
            table: String(row.table),
            from: String(row.from),
            to: row.to === null ? null : String(row.to),
            on_update: String(row.on_update),
            on_delete: String(row.on_delete),
            match: String(row.match),
          }))
          .sort((a, b) => a.id - b.id || a.seq - b.seq),
        columns_error: columnRows.error,
        foreign_keys_error: foreignKeyRows.error,
      })
    } else if (object.type === 'index') {
      const indexRows = readPragma(db, `PRAGMA index_xinfo(${quoteIdentifier(object.name)})`)
      indexes.push({
        name: object.name,
        columns: indexRows.rows.map((row) => ({
          seqno: Number(row.seqno),
          cid: Number(row.cid),
          name: row.name === null ? null : String(row.name),
          desc: Number(row.desc),
          coll: row.coll === null ? null : String(row.coll),
          key: Number(row.key),
        })),
        columns_error: indexRows.error,
      })
    }
  }

  const migrations = (
    db.prepare('SELECT name FROM _migrations ORDER BY name').all() as Array<{ name: string }>
  ).map((row) => String(row.name))

  return { objects, tables, indexes, migrations }
}

/**
 * Canonical fixture for the Rust side: migrate an empty in-memory database
 * with the reference TypeScript applier, then dump it. Pretty-printed with a
 * trailing newline so the checked-in file diffs cleanly.
 */
export async function buildCanonicalSchemaFixture(): Promise<string> {
  const db = new DatabaseSync(':memory:')
  try {
    await applyMigrations(createNodeSqliteClient(db))
    return `${JSON.stringify(canonicalSchemaDump(db), null, 2)}\n`
  } finally {
    db.close()
  }
}
