import { eq, asc } from 'drizzle-orm'
import type { DrizzleClient, DbClient } from '../types'
import { assets } from '../schema'

// The relationship fields were added after existing callers and persisted
// fixtures. Keep them optional at this boundary so legacy asset records remain
// readable while database reads always populate them as nullable values.
export type Asset = Omit<typeof assets.$inferSelect, 'parentAssetId' | 'pageNumber'> & {
  parentAssetId?: string | null
  pageNumber?: number | null
}
export type NewAsset = typeof assets.$inferInsert

type AssetRow = {
  id: string
  item_id: string
  path: string
  type: string
  sort_index: number
  size: number | null
  parent_asset_id: string | null
  page_number: number | null
  created_at: number
}

function orderAssetsForDisplay(rows: Asset[]): Asset[] {
  const preservesPageOrder = rows.length > 1 && rows.some((asset) => asset.sortIndex !== 0)
  return [...rows].sort((a, b) => {
    if (preservesPageOrder) {
      const bySortIndex = a.sortIndex - b.sortIndex
      if (bySortIndex !== 0) return bySortIndex
    }

    const byPath = a.path.localeCompare(b.path, undefined, { sensitivity: 'base' })
    if (byPath !== 0) return byPath
    return a.id.localeCompare(b.id)
  })
}

export class AssetRepo {
  constructor(
    private db: DrizzleClient,
    private rawClient?: DbClient
  ) {}

  async create(data: Omit<NewAsset, 'id' | 'createdAt'>): Promise<Asset> {
    const createdAsset: Asset = {
      id: crypto.randomUUID(),
      itemId: data.itemId,
      path: data.path,
      type: data.type,
      sortIndex: data.sortIndex ?? 0,
      size: data.size ?? null,
      parentAssetId: data.parentAssetId ?? null,
      pageNumber: data.pageNumber ?? null,
      createdAt: Date.now(),
    }

    if (this.rawClient) {
      // Validate that the parent item exists before inserting (FK constraint)
      const itemExists = await this.rawClient.select('SELECT id FROM items WHERE id = ?', [
        createdAsset.itemId,
      ])
      if (itemExists.length === 0) {
        throw new Error(`Cannot create asset: item "${createdAsset.itemId}" does not exist`)
      }

      await this.rawClient.execute(
        'INSERT INTO assets (id, item_id, path, type, sort_index, size, parent_asset_id, page_number, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)',
        [
          createdAsset.id,
          createdAsset.itemId,
          createdAsset.path,
          createdAsset.type,
          createdAsset.sortIndex,
          createdAsset.size,
          createdAsset.parentAssetId,
          createdAsset.pageNumber,
          createdAsset.createdAt,
        ]
      )
    } else {
      await this.db.insert(assets).values(createdAsset)
    }

    return createdAsset
  }

  async findByItem(itemId: string): Promise<Asset[]> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<AssetRow>(
        `SELECT id, item_id, path, type, sort_index, size, parent_asset_id, page_number, created_at
         FROM assets
         WHERE item_id = ?
         ORDER BY path COLLATE NOCASE ASC, id ASC`,
        [itemId]
      )

      return orderAssetsForDisplay(rows.map((row) => ({
        id: row.id,
        itemId: row.item_id,
        path: row.path,
        type: row.type,
        sortIndex: row.sort_index,
        size: row.size,
        parentAssetId: row.parent_asset_id,
        pageNumber: row.page_number,
        createdAt: row.created_at,
      })))
    }

    const rows = await this.db
      .select()
      .from(assets)
      .where(eq(assets.itemId, itemId))
      .orderBy(asc(assets.path), asc(assets.id))
    return orderAssetsForDisplay(rows)
  }

  /**
   * Whether the asset already has a stored embedding vector.
   *
   * `vec_assets` is outside the Drizzle schema, so this goes through the raw
   * client; without one there is nothing to read and the answer is "no".
   */
  async hasEmbedding(assetId: string): Promise<boolean> {
    if (!this.rawClient) return false

    const rows = await this.rawClient.select<{ embedded: number }>(
      'SELECT COUNT(*) AS embedded FROM vec_assets WHERE asset_id = ?',
      [assetId]
    )

    return (rows[0]?.embedded ?? 0) > 0
  }

  async findById(id: string): Promise<Asset | null> {
    const rows = await this.db.select().from(assets).where(eq(assets.id, id))

    return rows[0] ?? null
  }

  async findByParentAssetId(parentAssetId: string): Promise<Asset[]> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<AssetRow>(
        `SELECT id, item_id, path, type, sort_index, size, parent_asset_id, page_number, created_at
         FROM assets
         WHERE parent_asset_id = ?
         ORDER BY page_number ASC, id ASC`,
        [parentAssetId]
      )

      return rows.map((row) => ({
        id: row.id,
        itemId: row.item_id,
        path: row.path,
        type: row.type,
        sortIndex: row.sort_index,
        size: row.size,
        parentAssetId: row.parent_asset_id,
        pageNumber: row.page_number,
        createdAt: row.created_at,
      }))
    }

    return this.db
      .select()
      .from(assets)
      .where(eq(assets.parentAssetId, parentAssetId))
      .orderBy(asc(assets.pageNumber), asc(assets.id))
  }

  async delete(id: string): Promise<void> {
    await this.db.delete(assets).where(eq(assets.id, id))
  }

  /**
   * Update the path of an asset (e.g. after JPEG → PNG conversion).
   */
  async updatePath(id: string, newPath: string): Promise<void> {
    if (this.rawClient) {
      await this.rawClient.execute('UPDATE assets SET path = ? WHERE id = ?', [
        newPath,
        id,
      ])
    } else {
      await this.db.update(assets).set({ path: newPath }).where(eq(assets.id, id))
    }
  }

  /**
   * Delete an asset and all its dependent records
   * in a single atomic transaction. Returns the deleted asset record
   * so the caller can remove the associated file from the filesystem.
   *
   * @throws Error if the asset is not found
   * @throws Error if the transaction fails before committing
   */
  async deleteWithCascade(id: string): Promise<Asset> {
    if (!this.rawClient) {
      throw new Error('deleteWithCascade requires a rawClient for transactional execution')
    }

    // Step 1: Fetch the asset to get its path and verify it exists
    const asset = await this.findById(id)
    if (!asset) {
      throw new Error(`Asset not found: ${id}`)
    }

    const escapedId = id.replace(/'/g, "''")

    // Step 2: Execute all deletes in a single transaction.
    // Keep BEGIN/COMMIT inside the batch because executeBatch delegates to the
    // backend SQL runner and must not rely on implicit transaction behavior.
    try {
      await this.rawClient.executeBatch(`
        BEGIN;
        DELETE FROM extractions WHERE asset_id = '${escapedId}';
        DELETE FROM layouts WHERE asset_id = '${escapedId}';
        DELETE FROM transcriptions WHERE asset_id = '${escapedId}';
        DELETE FROM llm_results WHERE target_id = '${escapedId}' AND (target_type = 'asset' OR target_type = 'unknown');
        DELETE FROM annotations WHERE asset_id = '${escapedId}';
        DELETE FROM extractions WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}');
        DELETE FROM layouts WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}');
        DELETE FROM transcriptions WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}');
        DELETE FROM llm_results WHERE target_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}') AND (target_type = 'asset' OR target_type = 'unknown');
        DELETE FROM annotations WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}');
        DELETE FROM entities WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}');
        DELETE FROM triples WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}');
        DELETE FROM vec_assets WHERE asset_id IN (SELECT id FROM assets WHERE parent_asset_id = '${escapedId}');
        DELETE FROM entities WHERE asset_id = '${escapedId}';
        DELETE FROM triples WHERE asset_id = '${escapedId}';
        DELETE FROM vec_assets WHERE asset_id = '${escapedId}';
        DELETE FROM assets WHERE parent_asset_id = '${escapedId}';
        DELETE FROM assets WHERE id = '${escapedId}';
        COMMIT;
      `)
    } catch (e) {
      // Transaction failed — ensure the explicit BEGIN does not leave the
      // connection in an open transaction if the backend stops before COMMIT.
      try {
        await this.rawClient.executeBatch('ROLLBACK')
      } catch {
        /* rollback is best-effort; preserve the original failure */
      }

      throw new Error(
        `Failed to delete asset cascade for ${id}: ${e instanceof Error ? e.message : String(e)}`
      )
    }

    return asset
  }
}
