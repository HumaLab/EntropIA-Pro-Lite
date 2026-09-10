import { eq, desc, and, or, isNull } from 'drizzle-orm'
import type { DrizzleClient } from '../types'
import { notes } from '../schema'

export type Note = typeof notes.$inferSelect
export type NewNote = typeof notes.$inferInsert

export class NoteRepo {
  constructor(private db: DrizzleClient) {}

  async create(
    data: Omit<NewNote, 'id' | 'createdAt' | 'updatedAt'> & { assetId?: string | null }
  ): Promise<Note> {
    const now = Date.now()
    const rows = await this.db
      .insert(notes)
      .values({
        id: crypto.randomUUID(),
        itemId: data.itemId,
        assetId: data.assetId ?? null,
        content: data.content,
        createdAt: now,
        updatedAt: now,
      })
      .returning()

    return rows[0]!
  }

  async findByItem(itemId: string): Promise<Note[]> {
    return this.db
      .select()
      .from(notes)
      .where(eq(notes.itemId, itemId))
      .orderBy(desc(notes.createdAt))
  }

  /** Find notes scoped to a specific asset, plus item-level notes (assetId = null). */
  async findByAsset(itemId: string, assetId: string): Promise<Note[]> {
    return this.db
      .select()
      .from(notes)
      .where(and(eq(notes.itemId, itemId), or(eq(notes.assetId, assetId), isNull(notes.assetId))))
      .orderBy(desc(notes.createdAt))
  }

  async update(id: string, content: string): Promise<Note> {
    const rows = await this.db
      .update(notes)
      .set({ content, updatedAt: Date.now() })
      .where(eq(notes.id, id))
      .returning()

    return rows[0]!
  }

  async delete(id: string): Promise<void> {
    await this.db.delete(notes).where(eq(notes.id, id))
  }
}
