import { desc, eq } from 'drizzle-orm'
import type { AssetLayout, DrizzleClient, LayoutBlock, LayoutRegion } from '../types'
import { layouts } from '../schema'

type LayoutRow = typeof layouts.$inferSelect

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function toNumber(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function parseJsonArray(raw: string, field: string): unknown[] {
  let parsed: unknown

  try {
    parsed = JSON.parse(raw)
  } catch (error) {
    throw new Error(
      `Failed to parse layouts.${field}: ${error instanceof Error ? error.message : String(error)}`
    )
  }

  if (!Array.isArray(parsed)) {
    throw new Error(`Invalid layouts.${field}: expected JSON array`)
  }

  return parsed
}

function parseBbox(value: unknown, field: string) {
  if (!isRecord(value)) {
    throw new Error(`Invalid layouts.${field}.bbox: expected object`)
  }

  return {
    x: toNumber(value.x),
    y: toNumber(value.y),
    width: toNumber(value.width),
    height: toNumber(value.height),
  }
}

function parseRegion(value: unknown): LayoutRegion {
  if (!isRecord(value)) {
    throw new Error('Invalid layouts.regions entry: expected object')
  }

  return {
    page: typeof value.page === 'number' ? value.page : undefined,
    imageWidth: typeof value.image_width === 'number' ? value.image_width : undefined,
    imageHeight: typeof value.image_height === 'number' ? value.image_height : undefined,
    groupId: typeof value.group_id === 'number' ? value.group_id : undefined,
    category: typeof value.category === 'string' ? value.category : 'unknown',
    bbox: parseBbox(value.bbox, 'regions'),
    confidence: toNumber(value.confidence),
  }
}

function parseBlock(value: unknown): LayoutBlock {
  if (!isRecord(value)) {
    throw new Error('Invalid layouts.blocks entry: expected object')
  }

  return {
    page: typeof value.page === 'number' ? value.page : undefined,
    imageWidth: typeof value.image_width === 'number' ? value.image_width : undefined,
    imageHeight: typeof value.image_height === 'number' ? value.image_height : undefined,
    label: typeof value.label === 'string' ? value.label : 'unknown',
    content: typeof value.content === 'string' ? value.content : '',
    bbox: parseBbox(value.bbox, 'blocks'),
    order: toNumber(value.order),
    groupId: toNumber(value.group_id),
  }
}

function toAssetLayout(row: LayoutRow): AssetLayout {
  return {
    id: row.id,
    assetId: row.assetId,
    model: row.model,
    imageWidth: row.imageWidth,
    imageHeight: row.imageHeight,
    createdAt: row.createdAt,
    regions: parseJsonArray(row.regions, 'regions').map(parseRegion),
    blocks: parseJsonArray(row.blocks, 'blocks').map(parseBlock),
  }
}

export class LayoutRepo {
  constructor(private db: DrizzleClient) {}

  async findByAsset(assetId: string): Promise<AssetLayout | null> {
    const rows = await this.db
      .select()
      .from(layouts)
      .where(eq(layouts.assetId, assetId))
      .orderBy(desc(layouts.createdAt))
      .limit(1)

    return rows[0] ? toAssetLayout(rows[0]) : null
  }

  async deleteByAssetId(assetId: string): Promise<void> {
    await this.db.delete(layouts).where(eq(layouts.assetId, assetId))
  }
}
