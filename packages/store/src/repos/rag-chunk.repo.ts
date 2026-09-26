import { eq } from 'drizzle-orm'
import type { DrizzleClient } from '../types'
import { ragChunks } from '../schema'

/**
 * A chunk's identity fields — enough to resolve which asset (and item) a
 * cited passage came from. Never exposes the embedding vector or text: this
 * accessor is for source resolution, not for reading chunk content.
 */
export type RagChunkRef = {
  id: string
  assetId: string
  itemId: string
}

export class RagChunkRepo {
  constructor(private db: DrizzleClient) {}

  /**
   * Resolve a chunk's owning asset and item by its id.
   *
   * Returns `null` when the chunk no longer exists — a re-index can drop old
   * chunk rows while a previously generated report still cites them, and
   * that is not an error for the caller to crash on.
   */
  async findById(id: string): Promise<RagChunkRef | null> {
    const rows = await this.db
      .select({ id: ragChunks.id, assetId: ragChunks.assetId, itemId: ragChunks.itemId })
      .from(ragChunks)
      .where(eq(ragChunks.id, id))
    return rows[0] ?? null
  }
}
