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

/** Where to look for notes related to a passage. */
export interface RelatedNotesOptions {
  /** Collections to stay inside. An empty array is no collection, not all. */
  collectionIds?: string[]
  limit?: number
  /**
   * Fewest terms a note must share to count as related; 1 when absent. One
   * shared word out of a long passage is usually coincidence, not a relation.
   */
  minMatchedTerms?: number
}

/** A note related to a passage, and how many of the passage's words it holds. */
export interface RelatedNote extends NoteSearchHit {
  matchedTerms: number
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
    // An empty scope is a scope, not the absence of one: narrowing to no
    // collection must find nothing rather than quietly searching everywhere.
    if (options.collectionIds && options.collectionIds.length === 0) return []

    const query = options.query?.trim()
    const limit = options.limit ?? SEARCH_LIMIT
    if (!query) return this.#inScope(options, limit)

    // The limit applies to what matched, so every note in scope is read.
    const inScope = await this.#inScope(options)
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

  /**
   * Notes that share words with a passage, the most shared first.
   *
   * `search` wants every word, which is right for a query someone types and
   * wrong for a passage: a note never repeats a whole sentence of the
   * manuscript. Here each term counts on its own, with the same tolerance for
   * case, accents and typos, and a note is ranked by how many it holds. Ties
   * go to the note edited most recently.
   *
   * The terms are the caller's: which words of a passage carry meaning is a
   * language question, not a storage one.
   */
  async related(terms: string[], options: RelatedNotesOptions = {}): Promise<RelatedNote[]> {
    if (options.collectionIds && options.collectionIds.length === 0) return []
    const unique = [...new Set(terms.map((term) => term.trim()).filter(Boolean))]
    if (unique.length === 0) return []

    const found: RelatedNote[] = []
    for (const note of await this.#inScope(options)) {
      const text = `${note.content} ${note.itemTitle}`
      const matchedTerms = unique.filter((term) => matchText(term, text) !== null).length
      if (matchedTerms >= Math.max(1, options.minMatchedTerms ?? 1)) {
        found.push({ ...note, matchedTerms })
      }
    }
    // Stable, so notes holding as many words keep their recency order.
    found.sort((a, b) => b.matchedTerms - a.matchedTerms)
    return found.slice(0, options.limit ?? SEARCH_LIMIT)
  }

  /** Every note in scope, most recently edited first, with what it is on. */
  async #inScope(
    options: { collectionIds?: string[]; itemId?: string },
    limit?: number
  ): Promise<NoteSearchHit[]> {
    if (!this.client) throw new Error('NoteRepo needs the raw client to search notes')
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

    const sql = `SELECT n.id, n.item_id AS itemId, i.title AS itemTitle,
                        i.collection_id AS collectionId, n.asset_id AS assetId,
                        n.content, n.created_at AS createdAt, n.updated_at AS updatedAt
                   FROM notes n
                   JOIN items i ON i.id = n.item_id
                  ${where.length > 0 ? `WHERE ${where.join(' AND ')}` : ''}
                  ORDER BY n.updated_at DESC
                  ${limit === undefined ? '' : 'LIMIT ?'}`
    return this.client.select<NoteSearchHit>(sql, limit === undefined ? params : [...params, limit])
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
