import { describe, expect, it, vi } from 'vitest'
import { handleWritingImageDrop, type WritingImageDropTarget } from './writing-image-drop'
import type { PickedWritingImage } from './writing-image-picker'

/**
 * WritingView.svelte's Tauri `onDragDropEvent` handler is thin wiring over
 * this function — proven by source-text assertions in
 * WritingView.dragdrop.test.ts, the same convention every other WritingView
 * feature (dictation, the toolbar picker) already uses, because rendering
 * the whole view needs the whole store and Tauri behind it. The actual
 * decision-making — scope the drop to the open manuscript's own surface,
 * convert Tauri's physical-pixel position, fall back to the caret when exact
 * placement fails — lives here instead, where it can be driven directly.
 */

const ATTRS: PickedWritingImage = {
  src: 'writing-images/abc.png',
  alt: null,
  title: null,
  width: null,
  height: 10,
  align: 'center',
}

function fakeTarget(overrides: Partial<WritingImageDropTarget> = {}) {
  return {
    containsPoint: vi.fn(() => true),
    posAtCoords: vi.fn(() => 6),
    insertImage: vi.fn(() => true),
    ...overrides,
  } satisfies WritingImageDropTarget
}

describe('scoping the drop to the open manuscript', () => {
  it('does nothing when no editable manuscript is open', async () => {
    const readImage = vi.fn()

    const inserted = await handleWritingImageDrop(
      null,
      ['C:/photos/sunset.png'],
      { x: 100, y: 100 },
      1,
      readImage
    )

    expect(inserted).toBe(false)
    expect(readImage).not.toHaveBeenCalled()
  })

  it('does nothing when the drop lands outside the manuscript surface', async () => {
    const target = fakeTarget({ containsPoint: vi.fn(() => false) })
    const readImage = vi.fn()

    const inserted = await handleWritingImageDrop(
      target,
      ['C:/photos/sunset.png'],
      { x: 100, y: 100 },
      1,
      readImage
    )

    expect(inserted).toBe(false)
    expect(readImage).not.toHaveBeenCalled()
    expect(target.insertImage).not.toHaveBeenCalled()
  })

  it('does nothing when the drop carries no path', async () => {
    const target = fakeTarget()
    const readImage = vi.fn()

    const inserted = await handleWritingImageDrop(target, [], { x: 100, y: 100 }, 1, readImage)

    expect(inserted).toBe(false)
    expect(readImage).not.toHaveBeenCalled()
  })

  it('does nothing, and inserts nothing, when the path is refused (unreadable or unsupported format)', async () => {
    const target = fakeTarget()
    const readImage = vi.fn(async () => null)

    const inserted = await handleWritingImageDrop(
      target,
      ['C:/photos/notes.txt'],
      { x: 100, y: 100 },
      1,
      readImage
    )

    expect(inserted).toBe(false)
    expect(target.insertImage).not.toHaveBeenCalled()
  })

  it('only ever reads the first dropped path', async () => {
    const target = fakeTarget()
    const readImage = vi.fn(async () => ATTRS)

    await handleWritingImageDrop(
      target,
      ['C:/photos/first.png', 'C:/photos/second.png'],
      { x: 100, y: 100 },
      1,
      readImage
    )

    expect(readImage).toHaveBeenCalledExactlyOnceWith('C:/photos/first.png')
  })
})

describe('placing the image where it was dropped', () => {
  it('converts Tauri physical-pixel coordinates to logical ones before hit-testing and placing', async () => {
    const target = fakeTarget()
    const readImage = vi.fn(async () => ATTRS)

    await handleWritingImageDrop(target, ['C:/photos/sunset.png'], { x: 200, y: 300 }, 2, readImage)

    expect(target.containsPoint).toHaveBeenCalledWith(100, 150)
    expect(target.posAtCoords).toHaveBeenCalledWith(100, 150)
  })

  it('inserts at the resolved position when one is found', async () => {
    const target = fakeTarget({ posAtCoords: vi.fn(() => 6) })
    const readImage = vi.fn(async () => ATTRS)

    const inserted = await handleWritingImageDrop(
      target,
      ['C:/photos/sunset.png'],
      { x: 100, y: 100 },
      1,
      readImage
    )

    expect(inserted).toBe(true)
    expect(target.insertImage).toHaveBeenCalledExactlyOnceWith(ATTRS, 6)
  })

  it('falls back to the caret (no explicit position) when the position cannot be resolved', async () => {
    const target = fakeTarget({ posAtCoords: vi.fn(() => null) })
    const readImage = vi.fn(async () => ATTRS)

    const inserted = await handleWritingImageDrop(
      target,
      ['C:/photos/sunset.png'],
      { x: 100, y: 100 },
      1,
      readImage
    )

    expect(inserted).toBe(true)
    expect(target.insertImage).toHaveBeenCalledExactlyOnceWith(ATTRS, undefined)
  })
})
