import { eq, desc } from 'drizzle-orm'
import type { DrizzleClient } from '../types'
import { extractions, assets, items } from '../schema'

export type Extraction = typeof extractions.$inferSelect
export type NewExtraction = typeof extractions.$inferInsert

export class ExtractionRepo {
  constructor(private db: DrizzleClient) {}

  /**
   * Upsert extraction for an asset — deletes any existing extractions
   * for the assetId, then inserts a new one. Guarantees single row per asset.
   */
  async upsert(data: {
    assetId: string
    textContent: string
    method: string
    confidence?: number | null
  }): Promise<Extraction> {
    // Delete existing extractions for this asset
    await this.db.delete(extractions).where(eq(extractions.assetId, data.assetId))

    // Insert new extraction
    const rows = await this.db
      .insert(extractions)
      .values({
        id: crypto.randomUUID(),
        assetId: data.assetId,
        textContent: data.textContent,
        method: data.method,
        confidence: data.confidence ?? null,
        createdAt: Date.now(),
      })
      .returning()

    return rows[0]!
  }

  async findByAsset(assetId: string): Promise<Extraction | null> {
    const rows = await this.db
      .select()
      .from(extractions)
      .where(eq(extractions.assetId, assetId))
      .orderBy(desc(extractions.createdAt))
      .limit(1)

    return rows[0] ?? null
  }

  async findAllByAsset(assetId: string): Promise<Extraction[]> {
    return this.db
      .select()
      .from(extractions)
      .where(eq(extractions.assetId, assetId))
      .orderBy(desc(extractions.createdAt))
  }

  /**
   * All extraction texts for a collection in a single query
   * (avoids per-item/per-asset round-trips when building a corpus).
   */
  async findTextByCollection(
    collectionId: string
  ): Promise<Array<{ assetId: string; textContent: string; createdAt: number }>> {
    return this.db
      .select({
        assetId: extractions.assetId,
        textContent: extractions.textContent,
        createdAt: extractions.createdAt,
      })
      .from(extractions)
      .innerJoin(assets, eq(assets.id, extractions.assetId))
      .innerJoin(items, eq(items.id, assets.itemId))
      .where(eq(items.collectionId, collectionId))
  }

  async delete(id: string): Promise<void> {
    await this.db.delete(extractions).where(eq(extractions.id, id))
  }

  async deleteByAsset(assetId: string): Promise<void> {
    await this.db.delete(extractions).where(eq(extractions.assetId, assetId))
  }

  /**
   * Update only the text_content of the latest extraction for an asset.
   * Preserves id, created_at, method, and confidence.
   */
  async updateText(assetId: string, textContent: string): Promise<void> {
    const latest = await this.findByAsset(assetId)
    if (!latest) return
    await this.db.update(extractions).set({ textContent }).where(eq(extractions.id, latest.id))
  }
}
