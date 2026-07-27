import { and, desc, eq } from 'drizzle-orm'
import { annotations } from '../schema'
import type { DbClient, DrizzleClient } from '../types'

export type Annotation = typeof annotations.$inferSelect
export type NewAnnotation = typeof annotations.$inferInsert
export type AnnotationKind = Annotation['kind']
export type AnnotationInput = Omit<
  NewAnnotation,
  'id' | 'assetId' | 'page' | 'createdAt' | 'updatedAt'
>

export class AnnotationRepo {
  constructor(
    private db: DrizzleClient,
    private rawClient?: DbClient
  ) {}

  private createId(assetId: string, page: number, kind: string) {
    return kind === 'crop' || kind === 'rotation'
      ? `document-edit:${assetId}:${page}:${kind}`
      : crypto.randomUUID()
  }

  async create(data: Omit<NewAnnotation, 'id' | 'createdAt' | 'updatedAt'>): Promise<Annotation> {
    const now = Date.now()
    const rows = await this.db
      .insert(annotations)
      .values({
        id: this.createId(data.assetId, data.page ?? 1, data.kind),
        ...data,
        createdAt: now,
        updatedAt: now,
      })
      .returning()

    return rows[0]!
  }

  async findByAsset(assetId: string, page?: number): Promise<Annotation[]> {
    const scope =
      page === undefined
        ? eq(annotations.assetId, assetId)
        : and(eq(annotations.assetId, assetId), eq(annotations.page, page))

    return this.db
      .select()
      .from(annotations)
      .where(scope)
      .orderBy(desc(annotations.updatedAt), desc(annotations.createdAt))
  }

  async replaceForAssetPage(
    assetId: string,
    page: number,
    nextAnnotations: AnnotationInput[]
  ): Promise<Annotation[]> {
    const now = Date.now()
    const numeric = (value: unknown, field: string) => {
      const parsed = Number(value)
      if (!Number.isFinite(parsed)) {
        throw new Error(`Invalid annotation ${field}`)
      }
      return parsed
    }
    const normalizedPage = numeric(page, 'page')
    if (!Number.isInteger(normalizedPage) || normalizedPage < 1) {
      throw new Error('Invalid annotation page')
    }
    const rows = nextAnnotations.map((annotation) => ({
      id: this.createId(assetId, normalizedPage, annotation.kind),
      assetId,
      page: normalizedPage,
      ...annotation,
      x: numeric(annotation.x, 'x'),
      y: numeric(annotation.y, 'y'),
      width: numeric(annotation.width, 'width'),
      height: numeric(annotation.height, 'height'),
      createdAt: now,
      updatedAt: now,
    }))

    if (this.rawClient?.executeTransaction) {
      await this.rawClient.executeTransaction([
        {
          sql: 'DELETE FROM annotations WHERE asset_id = ? AND page = ?',
          params: [assetId, normalizedPage],
        },
        ...rows.map((annotation) => ({
          sql: 'INSERT INTO annotations (id, asset_id, page, kind, color, x, y, width, height, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
          params: [
            annotation.id,
            annotation.assetId,
            annotation.page,
            annotation.kind,
            annotation.color,
            annotation.x,
            annotation.y,
            annotation.width,
            annotation.height,
            annotation.createdAt,
            annotation.updatedAt,
          ],
        })),
      ])

      return this.findByAsset(assetId, normalizedPage)
    }

    await this.db
      .delete(annotations)
      .where(and(eq(annotations.assetId, assetId), eq(annotations.page, page)))

    if (nextAnnotations.length === 0) {
      return []
    }

    return this.db.insert(annotations).values(rows).returning()
  }

  async delete(id: string): Promise<void> {
    await this.db.delete(annotations).where(eq(annotations.id, id))
  }
}
