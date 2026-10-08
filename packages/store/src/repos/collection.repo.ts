import { eq, desc, sql } from 'drizzle-orm'
import type { DrizzleClient, DbClient } from '../types'
import { collections, items } from '../schema'
import { FtsRepo } from './fts.repo'

export type Collection = typeof collections.$inferSelect
export type NewCollection = typeof collections.$inferInsert

export class CollectionRepo {
  private ftsRepo: FtsRepo | null

  constructor(
    private db: DrizzleClient,
    private rawClient?: DbClient
  ) {
    this.ftsRepo = rawClient ? new FtsRepo(rawClient) : null
  }

  /** `id` is normally minted here; the sample collection passes its fixed one. */
  async create(
    data: Omit<NewCollection, 'id' | 'createdAt' | 'updatedAt'> & { id?: string }
  ): Promise<Collection> {
    const now = Date.now()
    const createdCollection: Collection = {
      id: data.id ?? crypto.randomUUID(),
      name: data.name,
      description: data.description ?? null,
      createdAt: now,
      updatedAt: now,
    }

    if (this.rawClient) {
      await this.rawClient.execute(
        'INSERT INTO collections (id, name, description, created_at, updated_at) VALUES (?, ?, ?, ?, ?)',
        [
          createdCollection.id,
          createdCollection.name,
          createdCollection.description,
          createdCollection.createdAt,
          createdCollection.updatedAt,
        ]
      )
    } else {
      await this.db.insert(collections).values(createdCollection)
    }

    return createdCollection
  }

  async findAll(): Promise<Collection[]> {
    return this.db.select().from(collections).orderBy(desc(collections.updatedAt))
  }

  /**
   * Returns only collections that have at least 1 item.
   * Collections with 0 items are filtered out at the query level.
   */
  async findAllNonEmpty(): Promise<Collection[]> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<Collection>(
        `SELECT c.* FROM collections c INNER JOIN items i ON i.collection_id = c.id GROUP BY c.id ORDER BY c.updated_at DESC`
      )
      return rows
    }
    // Fallback: use Drizzle with a subquery
    return this.db
      .select()
      .from(collections)
      .where(sql`id IN (SELECT DISTINCT collection_id FROM items)`)
      .orderBy(desc(collections.updatedAt))
  }

  async findById(id: string): Promise<Collection | null> {
    const rows = await this.db.select().from(collections).where(eq(collections.id, id))

    return rows[0] ?? null
  }

  async update(
    id: string,
    data: Partial<Pick<NewCollection, 'name' | 'description'>>
  ): Promise<Collection> {
    const rows = await this.db
      .update(collections)
      .set({ ...data, updatedAt: Date.now() })
      .where(eq(collections.id, id))
      .returning()

    return rows[0]!
  }

  /**
   * Delete a collection and ALL its associated data.
   *
   * Two-phase approach:
   * Phase 1 (atomic): Core tables that ALWAYS exist — wrapped in BEGIN/COMMIT.
   *   If any statement fails, the whole transaction rolls back.
   * Phase 2 (best-effort): Derived/optional tables that may not exist
   *   (`fts_items`, `vec_assets`). These are cleaned up
   *   after Phase 1 succeeds.
   *   Failures here are silently ignored — they're index/cache data.
   *
   * @throws Error if rawClient is not available
   * @throws Error if the core transaction fails
   */
  async delete(id: string): Promise<void> {
    if (!this.rawClient) {
      throw new Error('delete requires a rawClient for transactional execution')
    }

    const esc = id.replace(/'/g, "''")

    // Step 0: Get item IDs before deleting (needed for optional table cleanup)
    const itemRows = await this.rawClient.select(
      `SELECT id FROM items WHERE collection_id = '${esc}'`,
      []
    )
    const itemIds = itemRows.map((r: Record<string, unknown>) => r.id as string)
    const itemIdsList = itemIds.map((i) => `'${(i as string).replace(/'/g, "''")}'`).join(',')

    // Phase 1: Atomic transaction for core tables (always exist)
    try {
      await this.rawClient.executeBatch(`
        BEGIN;
        DELETE FROM extractions WHERE asset_id IN (SELECT id FROM assets WHERE item_id IN (SELECT id FROM items WHERE collection_id = '${esc}'));
        DELETE FROM layouts WHERE asset_id IN (SELECT id FROM assets WHERE item_id IN (SELECT id FROM items WHERE collection_id = '${esc}'));
        DELETE FROM llm_results WHERE (target_type = 'asset' OR target_type = 'unknown') AND target_id IN (SELECT id FROM assets WHERE item_id IN (SELECT id FROM items WHERE collection_id = '${esc}'));
        DELETE FROM llm_results WHERE (target_type = 'item' OR target_type = 'unknown') AND target_id IN (SELECT id FROM items WHERE collection_id = '${esc}');
        DELETE FROM llm_results WHERE target_id = '${esc}' AND (target_type = 'collection' OR target_type = 'unknown');
        DELETE FROM assets WHERE item_id IN (SELECT id FROM items WHERE collection_id = '${esc}');
        DELETE FROM entities WHERE item_id IN (SELECT id FROM items WHERE collection_id = '${esc}');
        DELETE FROM triples WHERE item_id IN (SELECT id FROM items WHERE collection_id = '${esc}');
        DELETE FROM notes WHERE item_id IN (SELECT id FROM items WHERE collection_id = '${esc}');
        DELETE FROM fts_items WHERE rowid IN (SELECT rowid FROM items WHERE collection_id = '${esc}');
        DELETE FROM items WHERE collection_id = '${esc}';
        DELETE FROM collections WHERE id = '${esc}';
        COMMIT;
      `)
    } catch (e) {
      try {
        await this.rawClient.executeBatch('ROLLBACK')
      } catch {
        /* rollback is best-effort; preserve the original failure */
      }

      throw new Error(
        `Failed to delete collection ${id}: ${e instanceof Error ? e.message : String(e)}`
      )
    }

    // Phase 2: Best-effort cleanup for optional tables (items already deleted, use cached IDs)
    if (itemIdsList.length > 0) {
      this.ftsRepo?.forgetVocabulary()

      // Asset embedding vectors
      try {
        await this.rawClient.execute(`DELETE FROM vec_assets WHERE item_id IN (${itemIdsList})`)
      } catch {
        /* table may not exist — non-fatal */
      }
    }
  }

  /**
   * Delete a collection only if it has 0 items. Used for auto-cleanup when
   * the last item of a collection is removed.
   */
  async deleteIfEmpty(id: string): Promise<boolean> {
    if (!this.rawClient) {
      throw new Error('deleteIfEmpty requires a rawClient for transactional execution')
    }

    const esc = id.replace(/'/g, "''")

    try {
      await this.rawClient.executeBatch(`
        DELETE FROM collections WHERE id = '${esc}' AND id NOT IN (SELECT DISTINCT collection_id FROM items);
      `)
      // Check if the collection was actually deleted
      const rows = await this.rawClient.select(`SELECT id FROM collections WHERE id = '${esc}'`, [])
      return rows.length === 0
    } catch (e) {
      throw new Error(
        `Failed to delete empty collection ${id}: ${e instanceof Error ? e.message : String(e)}`
      )
    }
  }

  async countItems(id: string): Promise<number> {
    const rows = await this.db
      .select({ count: sql<number>`count(*)` })
      .from(items)
      .where(eq(items.collectionId, id))

    return rows[0]?.count ?? 0
  }
}
