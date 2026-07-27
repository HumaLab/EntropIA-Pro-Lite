import { describe, expect, it } from 'vitest'
import {
  clampNormalized,
  cropAnnotations,
  normalizeAnnotationsForAsset,
  normalizedToPixels,
  rotateAnnotations,
} from './item-view-geometry'
import type { ViewerAnnotation } from '@entropia/ui'

function annotation(overrides: Partial<ViewerAnnotation> = {}): ViewerAnnotation {
  return {
    id: 'ann-1',
    assetId: 'asset-old',
    kind: 'rectangle',
    page: 1,
    color: '#ff0000',
    x: 0.2,
    y: 0.3,
    width: 0.4,
    height: 0.1,
    createdAt: 100,
    updatedAt: 100,
    ...overrides,
  }
}

describe('item view geometry helpers', () => {
  it('clamps normalized values to the 0-1 range', () => {
    expect(clampNormalized(-0.5)).toBe(0)
    expect(clampNormalized(0.4)).toBe(0.4)
    expect(clampNormalized(2)).toBe(1)
  })

  it('converts normalized regions to rounded pixel regions', () => {
    expect(normalizedToPixels({ x: 0.1, y: 0.2, width: 0.333, height: 0.5 }, 1200, 900)).toEqual({
      x: 120,
      y: 180,
      width: 400,
      height: 450,
    })
  })

  it('rotates annotations right and left preserving their normalized bounds', () => {
    expect(rotateAnnotations([annotation()], 'right')).toEqual([
      expect.objectContaining({ x: 0.6, y: 0.2, width: 0.1, height: 0.4 }),
    ])
    expect(rotateAnnotations([annotation()], 'left')).toEqual([
      expect.objectContaining({ x: 0.3, y: 0.4, width: 0.1, height: 0.4 }),
    ])
  })

  it('crops annotations to the selected region and drops non-overlapping annotations', () => {
    const cropped = cropAnnotations(
      [
        annotation({ id: 'inside', x: 0.25, y: 0.25, width: 0.25, height: 0.25 }),
        annotation({ id: 'edge', x: 0.1, y: 0.1, width: 0.3, height: 0.3 }),
        annotation({ id: 'outside', x: 0.8, y: 0.8, width: 0.1, height: 0.1 }),
      ],
      { x: 0.2, y: 0.2, width: 0.4, height: 0.4 }
    )

    expect(cropped.map((entry) => entry.id)).toEqual(['inside', 'edge'])
    expect(cropped[0]?.x).toBeCloseTo(0.125)
    expect(cropped[0]?.y).toBeCloseTo(0.125)
    expect(cropped[0]?.width).toBeCloseTo(0.625)
    expect(cropped[0]?.height).toBeCloseTo(0.625)
    expect(cropped[1]?.x).toBeCloseTo(0)
    expect(cropped[1]?.y).toBeCloseTo(0)
    expect(cropped[1]?.width).toBeCloseTo(0.5)
    expect(cropped[1]?.height).toBeCloseTo(0.5)
  })

  it('normalizes annotations for an asset with stable timestamps and generated ids', () => {
    expect(
      normalizeAnnotationsForAsset({
        annotations: [annotation({ id: '', x: -1, y: 2, width: 0.5, height: 2, createdAt: 0 })],
        assetId: 'asset-1',
        now: 500,
        createId: () => 'generated-id',
      })
    ).toEqual([
      expect.objectContaining({
        id: 'generated-id',
        assetId: 'asset-1',
        page: 1,
        x: 0,
        y: 1,
        width: 0.5,
        height: 1,
        createdAt: 500,
        updatedAt: 500,
      }),
    ])
  })

  it('keeps PDF rotation metadata outside normalized coordinate clamping and scopes by page', () => {
    expect(
      normalizeAnnotationsForAsset({
        annotations: [annotation({ kind: 'rotation', x: 3, y: -12, width: 0, height: 0 })],
        assetId: 'asset-pdf',
        page: 4,
        now: 500,
        createId: () => 'rotation-id',
      })
    ).toEqual([
      expect.objectContaining({
        assetId: 'asset-pdf',
        page: 4,
        kind: 'rotation',
        x: 3,
        y: -12,
      }),
    ])
  })
})
