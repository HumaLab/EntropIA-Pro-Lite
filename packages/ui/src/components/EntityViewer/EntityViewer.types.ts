export type EntityType =
  | 'person'
  | 'place'
  | 'date'
  | 'institution'
  | 'organization'
  | 'misc'
  | 'custom'

export interface Entity {
  id: string
  itemId: string
  entityType: EntityType
  value: string
  startOffset: number | null
  endOffset: number | null
  confidence: number | null
  createdAt: number
}

export interface EntityViewerProps {
  entities: Entity[]
  editingEntityId?: string | null
  editingValue?: string
  onhighlight?: (detail: { startOffset: number; endOffset: number }) => void
  onentityclick?: (entity: Entity) => void
  oneditvaluechange?: (value: string) => void
  onsaveentity?: (entityId: string, value: string) => void | Promise<void>
  oncancelentityedit?: () => void
  ondeleteentity?: (entityId: string) => void | Promise<void>
  labels?: Partial<EntityViewerLabels>
}

export interface EntityViewerLabels {
  emptyText: string
  editValueAria: string
  entityAriaLabel: (value: string) => string
  editEntityAria: (value: string) => string
  editEntityTitle: string
  saveEntityAria: string
  saveEntityTitle: string
  cancelEntityEditAria: string
  cancelEntityEditTitle: string
  deleteEntityAria: (value: string) => string
  confirmDeleteEntityAria: (value: string) => string
  deleteEntityTitle: string
  confirmDeleteEntityTitle: string
  deletePrompt: string
}

export const ENTITY_TYPE_TAGS: Record<EntityType, string> = {
  person: 'PER',
  place: 'LOC',
  date: 'DATE',
  institution: 'ORG',
  organization: 'ORG',
  misc: 'MISC',
  custom: 'CUSTOM',
}

/** CSS class suffix per entity type for color-coding */
export const ENTITY_TYPE_COLORS: Record<EntityType, string> = {
  person: 'person',
  place: 'place',
  date: 'date',
  institution: 'institution',
  organization: 'organization',
  misc: 'misc',
  custom: 'custom',
}
