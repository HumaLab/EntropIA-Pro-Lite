import type { Entity, NewEntity } from '@entropia/store'

export type EditableEntityType = 'person' | 'organization' | 'place' | 'misc' | 'date'

export type ManualEntityUpdatePayload = Pick<
  NewEntity,
  'entityType' | 'value' | 'confidence' | 'source'
>

export function normalizeManualEntityValue(value: string) {
  return value
    .trim()
    .replace(/^["'“”‘’«»\-–—\s]+|["'“”‘’«»\-–—\s]+$/g, '')
    .trim()
}

export function toEditableEntityType(entityType: Entity['entityType']): EditableEntityType {
  if (
    entityType === 'person' ||
    entityType === 'organization' ||
    entityType === 'place' ||
    entityType === 'misc' ||
    entityType === 'date'
  ) {
    return entityType
  }
  return 'organization'
}

export function buildManualEntityCreatePayload({
  itemId,
  assetId,
  entityType,
  value,
  now = Date.now,
}: {
  itemId: string
  assetId: string | null
  entityType: EditableEntityType
  value: string
  now?: () => number
}): NewEntity {
  return {
    itemId,
    assetId,
    entityType,
    value,
    startOffset: 0,
    endOffset: 0,
    confidence: 1.0,
    source: 'manual',
    modelName: null,
    createdAt: now(),
  }
}

export function buildManualEntityUpdatePayload(entity: Pick<Entity, 'entityType'>, value: string) {
  return {
    entityType: toEditableEntityType(entity.entityType),
    value,
    confidence: 1.0,
    source: 'manual',
  } satisfies ManualEntityUpdatePayload
}
