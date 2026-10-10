// Records the exact `DbClient` calls the real `runMigrations` makes against a
// fake, from-scratch database, so the Rust side can replay the migration IPC
// sequence (S-02c) without a live SQLite file. Shared by
// `scripts/export-migration-ipc.mjs` (writes the checked-in fixture) and
// `src/migration-ipc-fixture.test.ts` (drift guard), so there is exactly one
// implementation.
//
// Two scenarios are recorded:
// - FRESH install: `_migrations` is empty and no 0032 sentinel exists, so the
//   runner walks every registered migration in order up to
//   `0058_processing_ner_tasks`;
// - REPAIR: the 0032 sentinel probe reports a partial set while no table holds
//   rows, which makes the runner prepend its repair `DROP TRIGGER IF EXISTS` /
//   `DROP TABLE IF EXISTS` statements to the 0032 batch. Only those repair
//   statements are exported (under `repair_drops`), not the whole second run.

import type { DbClient } from './types'

/** The `DbClient` methods the desktop adapter maps onto the `db_*` IPC commands. */
export type MigrationIpcCommand =
  | 'db_execute_batch'
  | 'db_execute'
  | 'db_execute_transaction'
  | 'db_select'
  | 'db_select_rows'

export interface MigrationIpcCall {
  command: MigrationIpcCommand
  sql: string
  params: unknown[]
}

export interface MigrationIpcFixture {
  calls: MigrationIpcCall[]
  repair_drops: string[]
}

/** `runMigrations` passed in by the caller to keep this module Node-strippable. */
export type RunMigrationsFn = (client: DbClient) => Promise<void>

export interface RecordingOptions {
  /** Rows the fake returns for `SELECT name FROM _migrations ORDER BY id`. */
  applied?: string[]
  /** How many 0032 sentinel objects the fake pretends exist. */
  sentinelCount?: number
}

// Fixed clock: `runMigrations` stamps `applied_at` and the layouts backfill
// from `Date.now()`, which would make the checked-in fixture change every run.
const FIXED_NOW_MS = 1_700_000_000_000

// `PROCESSING_0032_TRIGGERS` + `PROCESSING_0032_TABLES_CHILD_FIRST`, as the
// runner joins them with '\n' into the repair prefix of the 0032 batch.
const REPAIR_DROP_LINE = /^DROP (?:TRIGGER|TABLE) IF EXISTS [A-Za-z_][A-Za-z0-9_]*;$/

const MIGRATION_INSERT_PARAMS =
  "INSERT INTO _migrations (name, applied_at) VALUES ('0032_batch_processing'"

/**
 * A `DbClient` that records every call and answers the runner's reads with
 * deterministic, plausible fresh-install values.
 */
class RecordingDbClient implements DbClient {
  readonly calls: MigrationIpcCall[] = []

  private readonly applied: string[]
  private readonly sentinelCount: number

  constructor(options: RecordingOptions = {}) {
    this.applied = options.applied ?? []
    this.sentinelCount = options.sentinelCount ?? 0
  }

  private record(command: MigrationIpcCommand, sql: string, params: unknown[]): void {
    this.calls.push({ command, sql, params })
  }

  async execute(sql: string, params: unknown[] = []): Promise<{ rowsAffected: number }> {
    this.record('db_execute', sql, params)
    return { rowsAffected: 1 }
  }

  async executeBatch(sql: string): Promise<void> {
    this.record('db_execute_batch', sql, [])
  }

  async executeTransaction(statements: Array<{ sql: string; params?: unknown[] }>): Promise<void> {
    for (const statement of statements) {
      this.record('db_execute_transaction', statement.sql, statement.params ?? [])
    }
  }

  async select<T = Record<string, unknown>>(sql: string, params: unknown[] = []): Promise<T[]> {
    this.record('db_select', sql, params)
    if (sql.includes('FROM _migrations')) {
      return this.applied.map((name) => ({ name })) as T[]
    }
    // `runMigrations`' 0032 sentinel probe: a fresh install has none of them.
    if (sql.includes('COUNT(*) AS cnt')) {
      return [{ cnt: this.sentinelCount }] as T[]
    }
    // `countSurvivingProcessingRows`' "which of the fixed tables still exist?"
    // probe (only reached on the repair path): the fake reports no survivors,
    // so the repair may proceed.
    if (sql.includes("type = 'table'")) {
      return [] as T[]
    }
    return [] as T[]
  }

  async selectRows(sql: string, params: unknown[] = []): Promise<unknown[][]> {
    this.record('db_select_rows', sql, params)
    return []
  }
}

async function withFixedClock<T>(fn: () => Promise<T>): Promise<T> {
  const realNow = Date.now
  Date.now = () => FIXED_NOW_MS
  try {
    return await fn()
  } finally {
    Date.now = realNow
  }
}

/** Records the full IPC sequence of a from-scratch `runMigrations`. */
export async function recordFreshInstallMigrationIpc(
  runMigrations: RunMigrationsFn
): Promise<MigrationIpcCall[]> {
  const client = new RecordingDbClient({ sentinelCount: 0 })
  await withFixedClock(() => runMigrations(client))
  return client.calls
}

/** Records a run whose 0032 sentinel probe reports a partial, empty state. */
export async function recordRepairMigrationIpc(
  runMigrations: RunMigrationsFn
): Promise<MigrationIpcCall[]> {
  const client = new RecordingDbClient({ sentinelCount: 1 })
  await withFixedClock(() => runMigrations(client))
  return client.calls
}

/**
 * Pulls the repair `DROP TRIGGER/TABLE IF EXISTS` lines out of the recorded
 * 0032 batch. The runner builds them as one statement per line and joins with
 * '\n', so this reverses that construction; it is not a SQL splitter (the Rust
 * side splits the exported statements, never this module).
 */
export function extractRepairDrops(calls: MigrationIpcCall[]): string[] {
  const repairBatch = calls.find(
    (call) =>
      call.command === 'db_execute_batch' &&
      call.sql.includes(MIGRATION_INSERT_PARAMS) &&
      call.sql.includes('DROP TRIGGER IF EXISTS')
  )
  if (!repairBatch) {
    throw new Error('0032 repair batch was not recorded; the runner repair path changed')
  }
  const drops: string[] = []
  // Line 0 is `BEGIN IMMEDIATE;`; the repair prefix is the run of drop lines
  // right after it, before the migration body.
  for (const line of repairBatch.sql.split('\n').slice(1)) {
    const trimmed = line.trim()
    if (!REPAIR_DROP_LINE.test(trimmed)) break
    drops.push(trimmed)
  }
  if (drops.length === 0) {
    throw new Error('0032 repair batch no longer starts with its repair drops')
  }
  return drops
}

export async function recordMigrationIpcFixture(
  runMigrations: RunMigrationsFn
): Promise<MigrationIpcFixture> {
  const calls = await recordFreshInstallMigrationIpc(runMigrations)
  const repairCalls = await recordRepairMigrationIpc(runMigrations)
  return { calls, repair_drops: extractRepairDrops(repairCalls) }
}

/** Stable, pretty, newline-terminated JSON — safe to diff and commit. */
export function renderMigrationIpcFixture(fixture: MigrationIpcFixture): string {
  return `${JSON.stringify(fixture, null, 2)}\n`
}

export async function recordMigrationIpcFixtureJson(runMigrations: RunMigrationsFn): Promise<string> {
  return renderMigrationIpcFixture(await recordMigrationIpcFixture(runMigrations))
}

/**
 * The migration names a recorded run applied, in run order. Batch migrations
 * carry their `INSERT INTO _migrations` inside the batch SQL; the rest use the
 * parameterized `db_execute` path.
 */
export function appliedMigrationNames(calls: MigrationIpcCall[]): string[] {
  const names: string[] = []
  for (const call of calls) {
    if (call.command === 'db_execute' && call.sql.includes('INSERT INTO _migrations')) {
      const name = call.params[0]
      if (typeof name === 'string') names.push(name)
    } else if (call.command === 'db_execute_batch') {
      const match = call.sql.match(/INSERT INTO _migrations \(name, applied_at\) VALUES \('([^']+)'/)
      if (match?.[1]) names.push(match[1])
    }
  }
  return names
}
