import type { ViewerAnnotation } from '@entropia/ui'

export type NormalizedRegion = {
  x: number
  y: number
  width: number
  height: number
}

export type PixelRegion = NormalizedRegion

export type RotationDirection = 'left' | 'right'

export function clampNormalized(value: number) {
  return Math.max(0, Math.min(1, value))
}

export function normalizedToPixels(
  region: NormalizedRegion,
  naturalW: number,
  naturalH: number
): PixelRegion {
  return {
    x: Math.round(region.x * naturalW),
    y: Math.round(region.y * naturalH),
    width: Math.round(region.width * naturalW),
    height: Math.round(region.height * naturalH),
  }
}

export function rotateAnnotation(
  annotation: ViewerAnnotation,
  rotation: RotationDirection
): ViewerAnnotation {
  if (rotation === 'right') {
    return {
      ...annotation,
      x: 1 - annotation.y - annotation.height,
      y: annotation.x,
      width: annotation.height,
      height: annotation.width,
    }
  }

  return {
    ...annotation,
    x: annotation.y,
    y: 1 - annotation.x - annotation.width,
    width: annotation.height,
    height: annotation.width,
  }
}

export function rotateAnnotations(
  annotations: ViewerAnnotation[],
  rotation: RotationDirection
): ViewerAnnotation[] {
  return annotations.map((annotation) => rotateAnnotation(annotation, rotation))
}

export function cropAnnotations(
  annotations: ViewerAnnotation[],
  region: NormalizedRegion
): ViewerAnnotation[] {
  const { x: cx, y: cy, width: cw, height: ch } = region

  return annotations
    .filter((annotation) => {
      const overlapsX = annotation.x < cx + cw && annotation.x + annotation.width > cx
      const overlapsY = annotation.y < cy + ch && annotation.y + annotation.height > cy
      return overlapsX && overlapsY
    })
    .map((annotation) => {
      const clampedX = Math.max(annotation.x, cx)
      const clampedY = Math.max(annotation.y, cy)
      const clampedRight = Math.min(annotation.x + annotation.width, cx + cw)
      const clampedBottom = Math.min(annotation.y + annotation.height, cy + ch)
      const newWidth = clampedRight - clampedX
      const newHeight = clampedBottom - clampedY

      return {
        ...annotation,
        x: (clampedX - cx) / cw,
        y: (clampedY - cy) / ch,
        width: newWidth / cw,
        height: newHeight / ch,
      }
    })
}

export function normalizeAnnotationForAsset({
  annotation,
  assetId,
  page = 1,
  now,
  createId,
}: {
  annotation: ViewerAnnotation
  assetId: string
  page?: number
  now: number
  createId: () => string
}): ViewerAnnotation {
  const isRotation = annotation.kind === 'rotation'
  return {
    ...annotation,
    id: annotation.id || createId(),
    assetId,
    page,
    color: annotation.color,
    x: isRotation ? Math.round(annotation.x) : clampNormalized(annotation.x),
    y: isRotation ? Math.max(-30, Math.min(30, annotation.y)) : clampNormalized(annotation.y),
    width: clampNormalized(annotation.width),
    height: clampNormalized(annotation.height),
    createdAt: annotation.createdAt || now,
    updatedAt: now,
  }
}

export function normalizeAnnotationsForAsset({
  annotations,
  assetId,
  page = 1,
  now,
  createId,
}: {
  annotations: ViewerAnnotation[]
  assetId: string
  page?: number
  now: number
  createId: () => string
}): ViewerAnnotation[] {
  return annotations.map((annotation) =>
    normalizeAnnotationForAsset({ annotation, assetId, page, now, createId })
  )
}
