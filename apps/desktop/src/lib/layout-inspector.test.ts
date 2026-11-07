import { describe, expect, it } from 'vitest'
import {
  formatLayoutBbox,
  getLayoutOverlaySourceMeta,
  serializeLayoutBlock,
} from './layout-inspector'

describe('layout inspector helpers', () => {
  it('formats bbox values compactly for badges and clipboard', () => {
    expect(formatLayoutBbox({ x: 10.123, y: 20, width: 30.5, height: 40.987 })).toBe(
      'x:10.12 y:20 w:30.5 h:40.99'
    )
  })

  it('returns human labels for overlay sources', () => {
    expect(getLayoutOverlaySourceMeta('region')).toMatchObject({
      shortLabel: 'Región',
      label: 'Región matcheada',
    })

    expect(getLayoutOverlaySourceMeta('block')).toMatchObject({
      shortLabel: 'Bloque',
      label: 'BBox del bloque',
    })
  })

  it('serializes the selected block with the rich inspector fields', () => {
    const json = serializeLayoutBlock({
      id: 'layout-block-3',
      sourceIndex: 3,
      regionId: 'layout-block-3::overlay',
      label: 'figure',
      order: 4,
      content: 'Bloque figura completo',
      preview: 'Bloque figura',
      bbox: { x: 1, y: 2, width: 3, height: 4 },
      overlayBbox: { x: 5, y: 6, width: 7, height: 8 },
      overlaySource: 'block',
      page: 2,
      groupId: 9,
      imageWidth: 1000,
      imageHeight: 1400,
    })

    expect(JSON.parse(json)).toMatchObject({
      id: 'layout-block-3',
      overlaySource: 'block',
      groupId: 9,
      content: 'Bloque figura completo',
      overlayBbox: { x: 5, y: 6, width: 7, height: 8 },
    })
  })
})
