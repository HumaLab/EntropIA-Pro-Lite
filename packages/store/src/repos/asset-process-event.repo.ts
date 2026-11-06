import { asc, eq } from 'drizzle-orm'
import type { AssetProcessEvent, DrizzleClient } from '../types'
import { assetProcessEvents } from '../schema'

type AssetProcessEventRow = typeof assetProcessEvents.$inferSelect

function parseParams(raw: string | null): Record<string, unknown> | null {
  if (!raw) return null
  try {
    const parsed: unknown = JSON.parse(raw)
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null
  } catch {
    return null
  }
}

function toEvent(row: AssetProcessEventRow): AssetProcessEvent {
  return {
    id: row.id,
    assetId: row.assetId,
    process: row.process,
    tool: row.tool,
    toolVersion: row.toolVersion,
    model: row.model,
    modelVersion: row.modelVersion,
    appVersion: row.appVersion,
    params: parseParams(row.paramsJson),
    inputSha256: row.inputSha256,
    outputSha256: row.outputSha256,
    createdAt: row.createdAt,
  }
}

/**
 * Read side of the asset process history (migration 0061). The backend is the
 * only writer: it stamps the app version and hashes the files itself.
 */
export class AssetProcessEventRepo {
  constructor(private db: DrizzleClient) {}

  /** Every process the asset went through, oldest first. */
  async findByAsset(assetId: string): Promise<AssetProcessEvent[]> {
    const rows = await this.db
      .select()
      .from(assetProcessEvents)
      .where(eq(assetProcessEvents.assetId, assetId))
      .orderBy(asc(assetProcessEvents.createdAt))
    return rows.map(toEvent)
  }
}
