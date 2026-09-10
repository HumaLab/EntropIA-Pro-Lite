import type { LayoutBoundingBox } from '@entropia/store'
import type { LayoutBlockView } from './layouts'

export interface LayoutOverlaySourceMeta {
  shortLabel: string
  label: string
  description: string
}

const OVERLAY_SOURCE_META: Record<LayoutBlockView['overlaySource'], LayoutOverlaySourceMeta> = {
  region: {
    shortLabel: 'Región',
    label: 'Región matcheada',
    description: 'El overlay usa la región detectada por layout porque hubo match con el bloque.',
  },
  block: {
    shortLabel: 'Bloque',
    label: 'BBox del bloque',
    description:
      'El overlay usa siempre el bbox detectado para este bloque, sin fusionarlo con una región más grande.',
  },
}

function roundLayoutNumber(value: number) {
  if (!Number.isFinite(value)) {
    return 0
  }

  return Number(value.toFixed(2))
}

export function getLayoutOverlaySourceMeta(source: LayoutBlockView['overlaySource']) {
  return OVERLAY_SOURCE_META[source]
}

export function formatLayoutBbox(bbox: Partial<LayoutBoundingBox>) {
  return `x:${roundLayoutNumber(bbox.x ?? 0)} y:${roundLayoutNumber(bbox.y ?? 0)} w:${roundLayoutNumber(bbox.width ?? 0)} h:${roundLayoutNumber(bbox.height ?? 0)}`
}

export function serializeLayoutBlock(block: LayoutBlockView) {
  return JSON.stringify(
    {
      id: block.id,
      regionId: block.regionId,
      label: block.label,
      order: block.order,
      page: block.page,
      groupId: block.groupId,
      overlaySource: block.overlaySource,
      bbox: block.bbox,
      overlayBbox: block.overlayBbox,
      preview: block.preview,
      content: block.content,
      imageWidth: block.imageWidth,
      imageHeight: block.imageHeight,
    },
    null,
    2
  )
}
