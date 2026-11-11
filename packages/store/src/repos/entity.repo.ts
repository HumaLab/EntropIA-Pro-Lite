import { eq, and, asc, ne, or, isNull, inArray, sql } from 'drizzle-orm'
import type { DrizzleClient } from '../types'
import { entities } from '../schema'
import { normalizeEntityAliasKey } from './entity-alias.repo'

export type Entity = typeof entities.$inferSelect
export type NewEntity = {
  itemId: string
  assetId?: string | null
  entityType: string
  value: string
  startOffset?: number
  endOffset?: number
  confidence?: number
  source?: string | null
  modelName?: string | null
  createdAt: number
}

export type EntityType =
  'person' | 'place' | 'date' | 'institution' | 'organization' | 'misc' | 'custom'

/** A distinct entity surface form with its mention count across the archive. */
export interface DistinctEntityValue {
  value: string
  mentions: number
}

function assertCoordinates(latitude: number, longitude: number) {
  if (!Number.isFinite(latitude) || latitude < -90 || latitude > 90) {
    throw new RangeError('Latitude must be a finite number between -90 and 90')
  }
  if (!Number.isFinite(longitude) || longitude < -180 || longitude > 180) {
    throw new RangeError('Longitude must be a finite number between -180 and 180')
  }
}

export class EntityRepo {
  constructor(private db: DrizzleClient) {}

  async findByItemId(itemId: string): Promise<Entity[]> {
    return this.db
      .select()
      .from(entities)
      .where(
        and(
          eq(entities.itemId, itemId),
          or(isNull(entities.source), ne(entities.source, 'manual_deleted'))
        )
      )
      .orderBy(asc(entities.createdAt), asc(entities.startOffset), asc(entities.value))
  }

  /** Find entities scoped to a specific asset, plus item-level entities (assetId = null). */
  async findByAssetId(itemId: string, assetId: string): Promise<Entity[]> {
    return this.db
      .select()
      .from(entities)
      .where(
        and(
          eq(entities.itemId, itemId),
          or(eq(entities.assetId, assetId), isNull(entities.assetId)),
          or(isNull(entities.source), ne(entities.source, 'manual_deleted'))
        )
      )
      .orderBy(asc(entities.createdAt), asc(entities.startOffset), asc(entities.value))
  }

  async findByItemIdAndType(itemId: string, type: EntityType): Promise<Entity[]> {
    return this.db
      .select()
      .from(entities)
      .where(
        and(
          eq(entities.itemId, itemId),
          eq(entities.entityType, type),
          or(isNull(entities.source), ne(entities.source, 'manual_deleted'))
        )
      )
  }

  async create(data: NewEntity): Promise<Entity> {
    const rows = await this.db
      .insert(entities)
      .values({
        id: crypto.randomUUID(),
        itemId: data.itemId,
        assetId: data.assetId ?? null,
        entityType: data.entityType,
        value: data.value,
        startOffset: data.startOffset ?? 0,
        endOffset: data.endOffset ?? 0,
        confidence: data.confidence ?? 1.0,
        source: data.source ?? null,
        modelName: data.modelName ?? null,
        createdAt: data.createdAt,
      })
      .returning()

    return rows[0]!
  }

  async update(
    id: string,
    data: Partial<
      Pick<
        NewEntity,
        'entityType' | 'value' | 'startOffset' | 'endOffset' | 'confidence' | 'source' | 'modelName'
      >
    >
  ): Promise<Entity> {
    const invalidatesGeocode = data.entityType !== undefined || data.value !== undefined
    const rows = await this.db
      .update(entities)
      .set({
        ...(data.entityType !== undefined ? { entityType: data.entityType } : {}),
        ...(data.value !== undefined ? { value: data.value } : {}),
        ...(data.startOffset !== undefined ? { startOffset: data.startOffset } : {}),
        ...(data.endOffset !== undefined ? { endOffset: data.endOffset } : {}),
        ...(data.confidence !== undefined ? { confidence: data.confidence } : {}),
        ...(data.source !== undefined ? { source: data.source } : {}),
        ...(data.modelName !== undefined ? { modelName: data.modelName } : {}),
        ...(invalidatesGeocode ? { latitude: null, longitude: null, geoStatus: 'pending' } : {}),
      })
      .where(eq(entities.id, id))
      .returning()

    return rows[0]!
  }

  async setManualLocation(id: string, latitude: number, longitude: number): Promise<Entity> {
    assertCoordinates(latitude, longitude)
    const rows = await this.db
      .update(entities)
      .set({ manualLatitude: latitude, manualLongitude: longitude })
      .where(eq(entities.id, id))
      .returning()

    return rows[0]!
  }

  async resetManualLocation(id: string): Promise<Entity> {
    const rows = await this.db
      .update(entities)
      .set({ manualLatitude: null, manualLongitude: null })
      .where(eq(entities.id, id))
      .returning()

    return rows[0]!
  }

  async delete(id: string): Promise<void> {
    await this.db
      .update(entities)
      .set({
        source: 'manual_deleted',
        confidence: 1.0,
        modelName: null,
      })
      .where(eq(entities.id, id))
  }

  async deleteByItemId(itemId: string): Promise<void> {
    await this.db.delete(entities).where(eq(entities.itemId, itemId))
  }

  /**
   * Distinct entity values of the given types with mention counts across
   * the whole archive (T5 suggestions, S5). Rows are grouped in JS by the
   * alias key (`normalizeEntityAliasKey`), so values that differ only in
   * case or whitespace count as one; the display form is the one with most
   * mentions (ties: longest, then alphabetical). Manually deleted mentions
   * (`source = 'manual_deleted'`) are excluded, like every other read.
   */
  async listDistinctValues(input: { types: string[] }): Promise<DistinctEntityValue[]> {
    if (input.types.length === 0) return []
    const rows = await this.db
      .select({ value: entities.value, mentions: sql<number>`count(*)` })
      .from(entities)
      .where(
        and(
          inArray(entities.entityType, input.types),
          or(isNull(entities.source), ne(entities.source, 'manual_deleted'))
        )
      )
      .groupBy(entities.value)

    const perRaw = new Map<string, Map<string, number>>()
    for (const row of rows) {
      const key = normalizeEntityAliasKey(row.value)
      if (!key) continue
      let counts = perRaw.get(key)
      if (!counts) {
        counts = new Map()
        perRaw.set(key, counts)
      }
      counts.set(row.value, (counts.get(row.value) ?? 0) + Number(row.mentions))
    }

    const distinct: DistinctEntityValue[] = []
    for (const counts of perRaw.values()) {
      let display = ''
      let displayMentions = -1
      let total = 0
      for (const [raw, count] of counts) {
        total += count
        if (
          count > displayMentions ||
          (count === displayMentions &&
            (raw.length > display.length || (raw.length === display.length && raw < display)))
        ) {
          display = raw
          displayMentions = count
        }
      }
      distinct.push({ value: display, mentions: total })
    }
    distinct.sort((a, b) => b.mentions - a.mentions || (a.value < b.value ? -1 : 1))
    return distinct
  }
}
