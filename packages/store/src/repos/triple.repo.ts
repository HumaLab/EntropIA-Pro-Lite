import { eq, and, or, isNull } from 'drizzle-orm'
import type { DrizzleClient } from '../types'
import { triples } from '../schema'

export type Triple = typeof triples.$inferSelect

export type NewTriple = {
  subject: string
  predicate: string
  object: string
  assetId?: string | null
}

/** The only fields a manual edit may touch. */
export type TripleUpdate = Partial<Pick<NewTriple, 'subject' | 'predicate' | 'object'>>

export class TripleRepo {
  constructor(private db: DrizzleClient) {}

  async findByItemId(itemId: string): Promise<Triple[]> {
    return this.db.select().from(triples).where(eq(triples.itemId, itemId))
  }

  /** Find triples scoped to a specific asset, plus item-level triples (assetId = null). */
  async findByAssetId(itemId: string, assetId: string): Promise<Triple[]> {
    return this.db
      .select()
      .from(triples)
      .where(
        and(
          eq(triples.itemId, itemId),
          or(eq(triples.assetId, assetId), isNull(triples.assetId))
        )
      )
  }

  /**
   * Inserts ONE triple, for manual entry.
   *
   * Builds exactly the same row `replaceByItemId` builds for extracted triples
   * — same id shape, same `createdAt` clock — so a hand-written triple is
   * indistinguishable from a generated one everywhere downstream.
   */
  async create(data: NewTriple & { itemId: string }): Promise<Triple> {
    const rows = await this.db
      .insert(triples)
      .values({
        id: crypto.randomUUID(),
        itemId: data.itemId,
        assetId: data.assetId ?? null,
        subject: data.subject,
        predicate: data.predicate,
        object: data.object,
        createdAt: Date.now(),
      })
      .returning()

    return rows[0]!
  }

  /**
   * Edits one triple in place, by id.
   *
   * Only the S|P|O text is writable: `itemId`, `assetId` and `createdAt`
   * identify the row, so editing a triple can never move it to another
   * document or page, and never touches sibling rows.
   */
  async update(id: string, data: TripleUpdate): Promise<Triple> {
    const rows = await this.db
      .update(triples)
      .set({
        ...(data.subject !== undefined ? { subject: data.subject } : {}),
        ...(data.predicate !== undefined ? { predicate: data.predicate } : {}),
        ...(data.object !== undefined ? { object: data.object } : {}),
      })
      .where(eq(triples.id, id))
      .returning()

    return rows[0]!
  }

  /**
   * Removes ONE triple, by id.
   *
   * Hard delete on purpose. Entities are tombstoned instead (`source` set to
   * `manual_deleted`) because a later NER run appends to the same table and
   * would resurrect them. Triples have no `source` column and every extraction
   * goes through `replaceByItemId`, which wipes the item's rows first — a
   * tombstone would have nothing to protect and nowhere to live.
   */
  async delete(id: string): Promise<void> {
    await this.db.delete(triples).where(eq(triples.id, id))
  }

  async replaceByItemId(itemId: string, rows: NewTriple[]): Promise<void> {
    await this.db.delete(triples).where(eq(triples.itemId, itemId))
    if (rows.length === 0) return

    await this.db.insert(triples).values(
      rows.map((row) => ({
        id: crypto.randomUUID(),
        itemId,
        assetId: row.assetId ?? null,
        subject: row.subject,
        predicate: row.predicate,
        object: row.object,
        createdAt: Date.now(),
      }))
    )
  }
}
