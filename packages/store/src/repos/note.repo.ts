import { eq, desc, and, or, isNull } from 'drizzle-orm'
import type { DbClient, DrizzleClient } from '../types'
import { notes } from '../schema'
import { matchText } from '../fuzzy'

export type Note = typeof notes.$inferSelect
export type NewNote = typeof notes.$inferInsert

/** A note plus what it is attached to, which is what makes it readable. */
export interface NoteSearchHit {
  id: string
  itemId: string
  itemTitle: string
  collectionId: string
  assetId: string | null
  content: string
  createdAt: number
  updatedAt: number
  /** Found only by forgiving a typo; ranked after every exact match. */
  approximate?: boolean
}

export interface NoteSearchOptions {
  /** Words to look for, in any order. Absent lists everything in scope. */
  query?: string
  /** Collections to stay inside. An empty array is no collection, not all. */
  collectionIds?: string[]
  itemId?: string
  limit?: number
}

const SEARCH_LIMIT = 50

export class NoteRepo {
  constructor(
    private db: DrizzleClient,
    private client?: DbClient
  ) {}

  /**
   * Notes across the corpus, narrowed by scope (G11, §13).
   *
   * Raw SQL rather than the query builder because the answer has to carry the
   * item and collection a note belongs to — a note read without knowing what it
   * is attached to is not usable evidence — and because the scope is a variable
   * set of collections.
   *
   * `notes.item_id` is untouched and stays `NOT NULL`. §13.1 forbids relaxing it
   * or minting fictitious items to work around it, so every note found here
   * belongs to a real item.
   *
   * The words are matched here rather than with SQL `LIKE`, which folds the
   * case of ASCII letters only — `Zárate` never matched `zarate` — and cannot
   * forgive a typo. Notes are few and written by hand, so reading every one in
   * scope costs nothing an index would save.
   */
  async search(options: NoteSearchOptions = {}): Promise<NoteSearchHit[]> {
    if (!this.client) throw new Error('NoteRepo.search needs the raw client')
    // An empty scope is a scope, not the absence of one: narrowing to no
    // collection must find nothing rather than quietly searching everywhere.
    if (options.collectionIds && options.collectionIds.length === 0) return []

    const where: string[] = []
    const params: unknown[] = []

    if (options.itemId) {
      where.push('n.item_id = ?')
      params.push(options.itemId)
    }
    if (options.collectionIds) {
      where.push(`i.collection_id IN (${options.collectionIds.map(() => '?').join(', ')})`)
      params.push(...options.collectionIds)
    }

    const query = options.query?.trim()
    const limit = options.limit ?? SEARCH_LIMIT
    const sql = `SELECT n.id, n.item_id AS itemId, i.title AS itemTitle,
                        i.collection_id AS collectionId, n.asset_id AS assetId,
                        n.content, n.created_at AS createdAt, n.updated_at AS updatedAt
                   FROM notes n
                   JOIN items i ON i.id = n.item_id
                  ${where.length > 0 ? `WHERE ${where.join(' AND ')}` : ''}
                  ORDER BY n.updated_at DESC
                  ${query ? '' : 'LIMIT ?'}`

    if (!query) return this.client.select<NoteSearchHit>(sql, [...params, limit])

    // The limit applies to what matched, so every note in scope is read.
    const inScope = await this.client.select<NoteSearchHit>(sql, params)
    const exact: NoteSearchHit[] = []
    const approximate: NoteSearchHit[] = []
    for (const note of inScope) {
      // The item's title counts: a note on "Acta del gremio" is about the gremio.
      const match = matchText(query, `${note.content} ${note.itemTitle}`)
      if (match === 'exact') exact.push(note)
      else if (match === 'approximate') approximate.push({ ...note, approximate: true })
    }
    return [...exact, ...approximate].slice(0, limit)
  }

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

  /**
   * One note by its id, or null when it is gone.
   *
   * A linked note has to be readable on its own: §13 asks for a divergence to
   * be reported, and that needs the note as it stands now next to the snapshot
   * the manuscript holds.
   */
  async findById(id: string): Promise<Note | null> {
    const rows = await this.db.select().from(notes).where(eq(notes.id, id)).limit(1)
    return rows[0] ?? null
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
