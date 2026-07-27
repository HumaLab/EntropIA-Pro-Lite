import { describe, expect, it } from 'vitest'
import {
  appendImageEditUndoEntry,
  cloneViewerAnnotations,
  createImageEditUndoEntry,
  createImageUpdatedPayload,
  discardLatestImageEditUndoEntry,
  getLatestImageEditUndoEntry,
  updateAssetPathInList,
} from './item-view-image-edit'
import type { ViewerAnnotation } from '@entropia/ui'

function annotation(overrides: Partial<ViewerAnnotation> = {}): ViewerAnnotation {
  return {
    id: 'ann-1',
    assetId: 'asset-1',
    page: 1,
    kind: 'rectangle',
    color: '#ff0000',
    x: 0.1,
    y: 0.2,
    width: 0.3,
    height: 0.4,
    createdAt: 100,
    updatedAt: 200,
    ...overrides,
  }
}

describe('item view image edit helpers', () => {
  it('creates undo entries with cloned annotations', () => {
    const annotations = [annotation()]
    const entry = createImageEditUndoEntry({
      path: '/old.png',
      width: 800,
      height: 600,
      annotations,
    })

    annotations[0]!.x = 0.9

    expect(entry).toEqual({
      path: '/old.png',
      page: 1,
      width: 800,
      height: 600,
      annotations: [expect.objectContaining({ x: 0.1 })],
    })
  })

  it('clones viewer annotations without preserving object identity', () => {
    const annotations = [annotation()]
    const cloned = cloneViewerAnnotations(annotations)

    expect(cloned).toEqual(annotations)
    expect(cloned[0]).not.toBe(annotations[0])
  })

  it('appends, reads, and discards undo entries immutably', () => {
    const first = createImageEditUndoEntry({
      path: '/first.png',
      width: 800,
      height: 600,
      annotations: [annotation({ id: 'first' })],
    })
    const second = createImageEditUndoEntry({
      path: '/second.png',
      width: 400,
      height: 300,
      annotations: [annotation({ id: 'second' })],
    })
    const stack = appendImageEditUndoEntry([first], second)

    expect(stack).toEqual([first, second])
    expect(getLatestImageEditUndoEntry(stack)).toBe(second)
    expect(discardLatestImageEditUndoEntry(stack)).toEqual([first])
    expect(stack).toEqual([first, second])
  })

  it('returns null when reading an empty undo stack', () => {
    expect(getLatestImageEditUndoEntry([])).toBeNull()
  })

  it('updates only the matching asset path immutably', () => {
    const first = { id: 'asset-1', path: '/old.png', title: 'First' }
    const second = { id: 'asset-2', path: '/other.png', title: 'Second' }
    const updated = updateAssetPathInList([first, second], 'asset-1', '/new.png')

    expect(updated).toEqual([
      expect.objectContaining({ id: 'asset-1', path: '/new.png' }),
      second,
    ])
    expect(updated[0]).not.toBe(first)
    expect(updated[1]).toBe(second)
  })

  it('builds the asset image updated event payload', () => {
    expect(
      createImageUpdatedPayload({ itemId: 'item-1', assetId: 'asset-1', path: '/new.png' })
    ).toEqual({ itemId: 'item-1', assetId: 'asset-1', path: '/new.png' })
  })
})
