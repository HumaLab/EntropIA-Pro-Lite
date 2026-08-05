import { eq, and, like, or, asc, sql } from 'drizzle-orm'
import type { DrizzleClient, DbClient } from '../types'
import { items, assets } from '../schema'
import { FtsRepo, type FtsResult } from './fts.repo'

export type Item = typeof items.$inferSelect
export type NewItem = typeof items.$inferInsert

export type CollectionItemCardSummary = Item & {
  assetCount: number
  primaryAssetId: string | null
  primaryAssetPath: string | null
  primaryAssetType: string | null
}

type CollectionItemCardSummaryRow = {
  id: string
  title: string
  collection_id: string
  metadata: string | null
  created_at: number
  updated_at: number
  asset_count: number | null
  primary_asset_id: string | null
  primary_asset_path: string | null
  primary_asset_type: string | null
}

/**
 * Collection-wide statistics for the header stats line:
 * - items: total documents in the collection
 * - assets: total assets (images, files, pages, ...) in the collection
 * - ocr / embeddings / ner / triples: distinct assets that have that
 *   analysis stage applied. NER and triples can be stored at item level
 *   (asset_id NULL); in that case every asset of the item counts.
 * Each counter is independent — one asset may be counted in several stages.
 */
export type CollectionStats = {
  items: number
  assets: number
  ocr: number
  embeddings: number
  ner: number
  triples: number
}

type CollectionStatsRow = {
  items_count: number | null
  assets_count: number | null
  ocr_count: number | null
  embed_count: number | null
  ner_count: number | null
  triples_count: number | null
}

export class ItemRepo {
  private ftsRepo: FtsRepo | null
  private rawClient?: DbClient

  constructor(
    private db: DrizzleClient,
    rawClient?: DbClient
  ) {
    this.rawClient = rawClient
    this.ftsRepo = rawClient ? new FtsRepo(rawClient) : null
  }

  async create(data: Omit<NewItem, 'id' | 'createdAt' | 'updatedAt'>): Promise<Item> {
    const now = Date.now()
    const createdItem: Item = {
      id: crypto.randomUUID(),
      title: data.title,
      collectionId: data.collectionId,
      metadata: data.metadata ?? null,
      createdAt: now,
      updatedAt: now,
    }

    if (this.rawClient) {
      // Validate that the parent collection exists before inserting (FK constraint)
      const collectionExists = await this.rawClient.select(
        'SELECT id FROM collections WHERE id = ?',
        [createdItem.collectionId]
      )
      if (collectionExists.length === 0) {
        throw new Error(
          `Cannot create item: collection "${createdItem.collectionId}" does not exist`
        )
      }

      await this.rawClient.execute(
        'INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)',
        [
          createdItem.id,
          createdItem.title,
          createdItem.collectionId,
          createdItem.metadata,
          createdItem.createdAt,
          createdItem.updatedAt,
        ]
      )
    } else {
      await this.db.insert(items).values(createdItem)
    }

    return createdItem
  }

  async findByCollection(collectionId: string): Promise<Item[]> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<CollectionItemCardSummaryRow>(
        `SELECT id, title, collection_id, metadata, created_at, updated_at
         FROM items
         WHERE collection_id = ?
         ORDER BY title COLLATE NOCASE ASC, id ASC`,
        [collectionId]
      )

      return rows.map((row) => ({
        id: row.id,
        title: row.title,
        collectionId: row.collection_id,
        metadata: row.metadata,
        createdAt: row.created_at,
        updatedAt: row.updated_at,
      }))
    }

    return this.db
      .select()
      .from(items)
      .where(eq(items.collectionId, collectionId))
      .orderBy(asc(items.title), asc(items.id))
  }

  async findCardSummariesByCollection(
    collectionId: string,
    query = ''
  ): Promise<CollectionItemCardSummary[]> {
    if (!this.rawClient) {
      const baseItems = query.trim()
        ? await this.searchByText(collectionId, query)
        : await this.findByCollection(collectionId)

      return baseItems.map((item) => ({
        ...item,
        assetCount: 0,
        primaryAssetId: null,
        primaryAssetPath: null,
        primaryAssetType: null,
      }))
    }

    const trimmedQuery = query.trim()
    const matchedIds = trimmedQuery
      ? (await this.searchByText(collectionId, trimmedQuery)).map((item) => item.id)
      : []
    if (trimmedQuery && matchedIds.length === 0) return []

    const params: unknown[] = trimmedQuery ? matchedIds : [collectionId]
    const filterSql = trimmedQuery
      ? `i.id IN (${matchedIds.map(() => '?').join(', ')})`
      : 'i.collection_id = ?'

    const rows = await this.rawClient.select<CollectionItemCardSummaryRow>(
      `
        SELECT
          i.id,
          i.title,
          i.collection_id,
          i.metadata,
          i.created_at,
          i.updated_at,
          (SELECT COUNT(*)
             FROM assets leaf
             WHERE leaf.item_id = i.id
               AND NOT EXISTS (
                 SELECT 1 FROM assets child WHERE child.parent_asset_id = leaf.id
               )
          ) AS asset_count,
          pa.id AS primary_asset_id,
          pa.path AS primary_asset_path,
          pa.type AS primary_asset_type
        FROM items i
        LEFT JOIN assets pa ON pa.id = (
          SELECT p.id
          FROM assets p
          WHERE p.item_id = i.id AND p.parent_asset_id IS NULL
          ORDER BY
            CASE p.type
              WHEN 'image' THEN 0
              WHEN 'pdf' THEN 1
              ELSE 2
            END,
            p.sort_index ASC,
            p.created_at ASC
          LIMIT 1
        )
        WHERE ${filterSql}
        ORDER BY i.title COLLATE NOCASE ASC, i.id ASC
      `,
      params
    )

    return rows.map((row) => ({
      id: row.id,
      title: row.title,
      collectionId: row.collection_id,
      metadata: row.metadata,
      createdAt: row.created_at,
      updatedAt: row.updated_at,
      assetCount: Number(row.asset_count ?? 0),
      primaryAssetId: row.primary_asset_id,
      primaryAssetPath: row.primary_asset_path,
      primaryAssetType: row.primary_asset_type,
    }))
  }

  /**
   * Collection-wide statistics for the header stats line:
   * - items: total documents in the collection
   * - assets: viewable assets — leaf rows only, i.e. excluding parent
   *   containers that own page children (matches the item-card asset counts)
   * - ocr: distinct viewable assets with at least one extraction
   * - embeddings: distinct viewable assets with a row in vec_assets
   * - ner: distinct viewable assets with a direct entity, or whose item has
   *   item-level entities (asset_id NULL)
   * - triples: distinct viewable assets with a direct triple, or whose item
   *   has item-level triples (asset_id NULL)
   */
  async getCollectionStats(collectionId: string): Promise<CollectionStats> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<CollectionStatsRow>(
        `
          WITH viewable_assets AS (
            SELECT a.id, a.item_id
              FROM assets a
             WHERE NOT EXISTS (
               SELECT 1 FROM assets child WHERE child.parent_asset_id = a.id
             )
          )
          SELECT
            (SELECT COUNT(*)
               FROM items i
              WHERE i.collection_id = ?
            ) AS items_count,
            (SELECT COUNT(*)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
            ) AS assets_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND EXISTS (
                  SELECT 1 FROM extractions e WHERE e.asset_id = va.id
                )
            ) AS ocr_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND EXISTS (
                  SELECT 1 FROM vec_assets v WHERE v.asset_id = va.id
                )
            ) AS embed_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND (
                  EXISTS (SELECT 1 FROM entities en WHERE en.asset_id = va.id)
                  OR EXISTS (
                    SELECT 1 FROM entities en
                    WHERE en.item_id = i.id AND en.asset_id IS NULL
                  )
                )
            ) AS ner_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND (
                  EXISTS (SELECT 1 FROM triples tr WHERE tr.asset_id = va.id)
                  OR EXISTS (
                    SELECT 1 FROM triples tr
                    WHERE tr.item_id = i.id AND tr.asset_id IS NULL
                  )
                )
            ) AS triples_count
        `,
        [
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
        ]
      )

      const row = rows[0] ?? ({} as CollectionStatsRow)
      return {
        items: Number(row.items_count ?? 0),
        assets: Number(row.assets_count ?? 0),
        ocr: Number(row.ocr_count ?? 0),
        embeddings: Number(row.embed_count ?? 0),
        ner: Number(row.ner_count ?? 0),
        triples: Number(row.triples_count ?? 0),
      }
    }

    // Drizzle fallback (no raw client): one aggregate per statistic.
    // Leaf filter mirrors the raw query: assets that own page children are
    // parent containers and never count.
    const leafFilter = sql`NOT EXISTS (
      SELECT 1 FROM assets child WHERE child.parent_asset_id = ${assets.id}
    )`
    const [itemsRows, assetsRows, ocrRows, embedRows, nerRows, triplesRows] = await Promise.all([
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(items)
        .where(eq(items.collectionId, collectionId)),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(and(eq(items.collectionId, collectionId), leafFilter)),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`EXISTS (SELECT 1 FROM extractions e WHERE e.asset_id = ${assets.id})`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`EXISTS (SELECT 1 FROM vec_assets v WHERE v.asset_id = ${assets.id})`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`(
              EXISTS (SELECT 1 FROM entities en WHERE en.asset_id = ${assets.id})
              OR EXISTS (
                SELECT 1 FROM entities en
                WHERE en.item_id = ${items.id} AND en.asset_id IS NULL
              )
            )`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`(
              EXISTS (SELECT 1 FROM triples tr WHERE tr.asset_id = ${assets.id})
              OR EXISTS (
                SELECT 1 FROM triples tr
                WHERE tr.item_id = ${items.id} AND tr.asset_id IS NULL
              )
            )`
          )
        ),
    ])

    return {
      items: Number(itemsRows[0]?.count ?? 0),
      assets: Number(assetsRows[0]?.count ?? 0),
      ocr: Number(ocrRows[0]?.count ?? 0),
      embeddings: Number(embedRows[0]?.count ?? 0),
      ner: Number(nerRows[0]?.count ?? 0),
      triples: Number(triplesRows[0]?.count ?? 0),
    }
  }

  async findById(id: string): Promise<Item | null> {
    const rows = await this.db.select().from(items).where(eq(items.id, id))

    return rows[0] ?? null
  }

  async update(id: string, data: Partial<Pick<NewItem, 'title' | 'metadata'>>): Promise<Item> {
    const rows = await this.db
      .update(items)
      .set({ ...data, updatedAt: Date.now() })
      .where(eq(items.id, id))
      .returning()

    return rows[0]!
  }

  async delete(id: string): Promise<void> {
    await this.db.delete(items).where(eq(items.id, id))
  }

  /**
   * Delete an item and ALL its associated data in a single atomic transaction.
   * This is used when the last asset of an item is removed — the item becomes
   * an orphan and should be fully cleaned up.
   *
   * Cleanup order (dependencies first):
   * 1. Extractions (FK → assets)
   * 2. Assets (FK → items)
   * 3. Entities (FK → items)
   * 4. Triples (FK → items)
   * 5. Asset embeddings (item_id in vec_assets)
   * 6. FTS rebuild from canonical rowid sources
   * 7. Notes (FK → items)
   * 8. Item itself
   *
   * @throws Error if rawClient is not available
   * @throws Error if the transaction fails
   */
  async deleteWithCascade(id: string): Promise<void> {
    if (!this.rawClient) {
      throw new Error('deleteWithCascade requires a rawClient for transactional execution')
    }

    const esc = id.replace(/'/g, "''")

    // Get the parent collection ID before deleting the item (needed for auto-cleanup)
    const parentRows = await this.rawClient.select(
      `SELECT collection_id FROM items WHERE id = '${esc}'`,
      []
    )
    const collectionId = parentRows[0]?.collection_id as string | undefined
    const escCollectionId = collectionId !== undefined ? collectionId.replace(/'/g, "''") : ''

    // Phase 1: Atomic transaction for core tables (always exist)
    try {
      await this.rawClient.executeBatch(`
        BEGIN;
        DELETE FROM extractions WHERE asset_id IN (SELECT id FROM assets WHERE item_id = '${esc}');
        DELETE FROM layouts WHERE asset_id IN (SELECT id FROM assets WHERE item_id = '${esc}');
        DELETE FROM llm_results WHERE (target_type = 'asset' OR target_type = 'unknown') AND target_id IN (SELECT id FROM assets WHERE item_id = '${esc}');
        DELETE FROM llm_results WHERE target_id = '${esc}' AND (target_type = 'item' OR target_type = 'unknown');
        DELETE FROM assets WHERE item_id = '${esc}';
        DELETE FROM entities WHERE item_id = '${esc}';
        DELETE FROM triples WHERE item_id = '${esc}';
        DELETE FROM notes WHERE item_id = '${esc}';
        DELETE FROM items WHERE id = '${esc}';
        DELETE FROM collections WHERE id = '${escCollectionId}' AND id NOT IN (SELECT DISTINCT collection_id FROM items);
        COMMIT;
      `)
    } catch (e) {
      try {
        await this.rawClient.executeBatch('ROLLBACK')
      } catch {
        /* rollback is best-effort; preserve the original failure */
      }

      throw new Error(
        `Failed to delete item cascade for ${id}: ${e instanceof Error ? e.message : String(e)}`
      )
    }

    // Phase 2: Best-effort cleanup for optional tables / derived indexes
    try {
      await this.ftsRepo?.rebuildIndex()
    } catch {
      /* table may not exist — non-fatal */
    }

    try {
      await this.rawClient.execute(`DELETE FROM vec_assets WHERE item_id = '${esc}'`)
    } catch {
      /* table may not exist — non-fatal */
    }
  }

  /**
   * Search items by text.
   * - If a rawClient was provided (FTS5 available), tries FTS5 first.
   *   If FTS5 returns results, fetches those items from Drizzle and returns them.
   * - Falls back to SQL LIKE on title and metadata if FTS5 is unavailable or returns nothing.
   */
  async searchByText(collectionId: string, query: string): Promise<Item[]> {
    // Try FTS5 first if rawClient is available
    if (this.ftsRepo && query.trim()) {
      const ftsResults = await this.ftsRepo.search(query, 50)
      if (ftsResults.length > 0) {
        // Fetch the actual items from Drizzle using the IDs returned by FTS5
        const ids = ftsResults.map((r) => r.itemId)
        const rows = await this.db
          .select()
          .from(items)
          .where(
            and(
              eq(items.collectionId, collectionId),
              // Filter to items whose IDs are in the FTS5 result set
              // We use an OR chain over all matched IDs
              ids.length === 1 ? eq(items.id, ids[0]!) : or(...ids.map((id) => eq(items.id, id)))!
            )
          )
          .orderBy(asc(items.title), asc(items.id))

        return rows
      }
    }

    // Fallback: SQL LIKE on title and metadata
    const pattern = `%${query}%`
    return this.db
      .select()
      .from(items)
      .where(
        and(
          eq(items.collectionId, collectionId),
          or(like(items.title, pattern), like(items.metadata, pattern))
        )
      )
      .orderBy(asc(items.title), asc(items.id))
  }

  /**
   * FTS5-based search. Returns FtsResult[] with itemId and rank.
   * Requires a rawClient (DbClient) to be provided at construction time.
   * Returns empty array if no rawClient or empty query.
   */
  async searchByFts5(query: string, _collectionId?: string): Promise<FtsResult[]> {
    if (!this.ftsRepo || !query.trim()) return []
    return this.ftsRepo.search(query, 50)
  }

  /**
   * Search items across ALL collections.
   * Tries FTS5 first, falls back to SQL LIKE on title and metadata.
   */
  async searchGlobal(query: string, limit = 20): Promise<Item[]> {
    if (!query.trim()) return []

    // Try FTS5 first
    if (this.ftsRepo) {
      const ftsResults = await this.ftsRepo.search(query, limit)
      if (ftsResults.length > 0) {
        const ids = ftsResults.map((r) => r.itemId)
        return this.db
          .select()
          .from(items)
          .where(
            ids.length === 1 ? eq(items.id, ids[0]!) : or(...ids.map((id) => eq(items.id, id)))!
          )
          .orderBy(asc(items.title), asc(items.id))
      }
    }

    // Fallback: SQL LIKE on title and metadata
    const pattern = `%${query}%`
    return this.db
      .select()
      .from(items)
      .where(or(like(items.title, pattern), like(items.metadata, pattern)))
      .orderBy(asc(items.title), asc(items.id))
      .limit(limit)
  }
}
