import type { SqliteRemoteDatabase } from 'drizzle-orm/sqlite-proxy'

/**
 * Low-level database client port.
 * Used by both the migration runner and the Drizzle sqlite-proxy adapter.
 */
export interface DbClient {
  /** Execute a write query (INSERT, UPDATE, DELETE, DDL). */
  execute(sql: string, params?: unknown[]): Promise<{ rowsAffected: number }>
  /** Execute multiple SQL statements atomically within a transaction. */
  executeBatch(sql: string): Promise<void>
  /** Execute parameterized DML statements atomically within one transaction. */
  executeTransaction?(statements: Array<{ sql: string; params?: unknown[] }>): Promise<void>
  /** Execute a read query (SELECT) and return typed rows. */
  select<T = Record<string, unknown>>(sql: string, params?: unknown[]): Promise<T[]>
  /** Execute a row-returning query and preserve database column order. */
  selectRows(sql: string, params?: unknown[]): Promise<unknown[][]>
}

export interface LayoutBoundingBox {
  x: number
  y: number
  width: number
  height: number
}

export interface LayoutRegion {
  page?: number
  imageWidth?: number
  imageHeight?: number
  groupId?: number
  category: string
  bbox: LayoutBoundingBox
  confidence: number
}

export interface LayoutBlock {
  page?: number
  imageWidth?: number
  imageHeight?: number
  label: string
  content: string
  bbox: LayoutBoundingBox
  order: number
  groupId: number
}

export interface AssetLayout {
  id: string
  assetId: string
  regions: LayoutRegion[]
  blocks: LayoutBlock[]
  model: string
  imageWidth: number
  imageHeight: number
  createdAt: number
}

/**
 * Drizzle ORM client type for sqlite-proxy mode.
 * Used by repository classes for typed query building.
 */
export type DrizzleClient = SqliteRemoteDatabase
