import { beforeEach, describe, expect, it } from 'vitest'
import {
  clampBubbleOffsets,
  clampBubblePosition,
  clearStoredBubblePosition,
  defaultBubbleOffsets,
  exceedsDragThreshold,
  HELP_BUBBLE_DRAG_THRESHOLD_PX,
  HELP_BUBBLE_MAXIMIZED_KEY,
  HELP_BUBBLE_STORAGE_KEY,
  loadBubbleMaximized,
  offsetsToPosition,
  positionToOffsets,
  saveBubbleMaximized,
  type BubbleOffsets,
  type BubbleViewport,
} from './help-bubble'

const SMALL_VIEWPORT: BubbleViewport = { width: 1024, height: 700 }
const LARGE_VIEWPORT: BubbleViewport = { width: 1920, height: 1080 }

function storageWith(initial: Record<string, string> = {}): Storage {
  const data = new Map(Object.entries(initial))
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value)
    },
    removeItem: (key: string) => {
      data.delete(key)
    },
    clear: () => data.clear(),
    key: (index: number) => [...data.keys()][index] ?? null,
    get length() {
      return data.size
    },
  }
}

describe('help-bubble position helpers (S13, T16)', () => {
  describe('defaultBubbleOffsets', () => {
    it('sits at the bottom-right with a margin that clears the status bar', () => {
      // 56px bubble: 24px from the right edge, 30px status bar + 16px above it.
      expect(defaultBubbleOffsets()).toEqual({ right: 24, bottom: 46 })
    })

    it('lands at the bottom-right corner of any window', () => {
      // The task's resize case: 1024x700 -> 1920x1080 follows the corner.
      expect(offsetsToPosition(defaultBubbleOffsets(), SMALL_VIEWPORT)).toEqual({
        x: 1024 - 56 - 24,
        y: 700 - 56 - 46,
      })
      expect(offsetsToPosition(defaultBubbleOffsets(), LARGE_VIEWPORT)).toEqual({
        x: 1920 - 56 - 24,
        y: 1080 - 56 - 46,
      })
    })
  })

  describe('offsetsToPosition / positionToOffsets', () => {
    it('round-trips a dragged bubble', () => {
      const offsets: BubbleOffsets = { right: 124, bottom: 96 }
      const position = offsetsToPosition(offsets, LARGE_VIEWPORT)
      expect(position).toEqual({ x: 1920 - 56 - 124, y: 1080 - 56 - 96 })
      expect(positionToOffsets(position, LARGE_VIEWPORT)).toEqual(offsets)
    })

    it('a dragged bubble keeps its right/bottom offsets after a resize', () => {
      // Dragged 100px left and 50px up from the default at 1024x768.
      const offsets: BubbleOffsets = { right: 24 + 100, bottom: 46 + 50 }
      const before = offsetsToPosition(offsets, { width: 1024, height: 768 })
      expect(before).toEqual({ x: 1024 - 56 - 124, y: 768 - 56 - 96 })
      const after = offsetsToPosition(offsets, LARGE_VIEWPORT)
      // Same distance to the corner, new corner.
      expect(after.x).toBe(LARGE_VIEWPORT.width - 56 - offsets.right)
      expect(after.y).toBe(LARGE_VIEWPORT.height - 56 - offsets.bottom)
    })

    it('clamps the position into the window', () => {
      const clamped = offsetsToPosition({ right: -1000, bottom: -1000 }, SMALL_VIEWPORT)
      expect(clamped.x + 56).toBeLessThanOrEqual(SMALL_VIEWPORT.width)
      expect(clamped.y + 56).toBeLessThanOrEqual(SMALL_VIEWPORT.height)
    })
  })

  describe('clampBubblePosition', () => {
    it('keeps a position inside the window untouched', () => {
      expect(clampBubblePosition({ x: 100, y: 100 }, SMALL_VIEWPORT)).toEqual({
        x: 100,
        y: 100,
      })
    })

    it('pulls negative coordinates back inside', () => {
      expect(clampBubblePosition({ x: -50, y: -20 }, SMALL_VIEWPORT).x).toBeGreaterThanOrEqual(0)
      expect(clampBubblePosition({ x: -50, y: -20 }, SMALL_VIEWPORT).y).toBeGreaterThanOrEqual(0)
    })

    it('pulls coordinates past the far edges back inside', () => {
      const clamped = clampBubblePosition({ x: 5000, y: 5000 }, SMALL_VIEWPORT)
      expect(clamped.x + 56).toBeLessThanOrEqual(SMALL_VIEWPORT.width)
      expect(clamped.y + 56).toBeLessThanOrEqual(SMALL_VIEWPORT.height)
    })

    it('survives a degenerate viewport without NaN', () => {
      const clamped = clampBubblePosition({ x: 10, y: 10 }, { width: 0, height: 0 })
      expect(Number.isFinite(clamped.x)).toBe(true)
      expect(Number.isFinite(clamped.y)).toBe(true)
    })
  })

  describe('clampBubbleOffsets', () => {
    it('keeps fitting offsets untouched', () => {
      expect(clampBubbleOffsets({ right: 24, bottom: 46 }, SMALL_VIEWPORT)).toEqual({
        right: 24,
        bottom: 46,
      })
    })

    it('pulls the bubble back inside when the window shrinks under it', () => {
      const clamped = clampBubbleOffsets({ right: 24, bottom: 46 }, { width: 100, height: 100 })
      const position = offsetsToPosition(clamped, { width: 100, height: 100 })
      expect(position.x).toBeGreaterThanOrEqual(0)
      expect(position.y).toBeGreaterThanOrEqual(0)
      expect(position.x + 56).toBeLessThanOrEqual(100)
      expect(position.y + 56).toBeLessThanOrEqual(100)
    })

    it('is a fixed point: clamping twice changes nothing', () => {
      const once = clampBubbleOffsets({ right: 24, bottom: 46 }, { width: 100, height: 100 })
      expect(clampBubbleOffsets(once, { width: 100, height: 100 })).toEqual(once)
    })
  })

  describe('drag threshold', () => {
    it('is about four pixels', () => {
      expect(HELP_BUBBLE_DRAG_THRESHOLD_PX).toBe(4)
    })

    it('treats a movement under four pixels as a click', () => {
      expect(exceedsDragThreshold({ x: 0, y: 0 }, { x: 3, y: 0 })).toBe(false)
      expect(exceedsDragThreshold({ x: 0, y: 0 }, { x: 4, y: 0 })).toBe(false)
    })

    it('treats a movement over four pixels as a drag', () => {
      expect(exceedsDragThreshold({ x: 0, y: 0 }, { x: 5, y: 0 })).toBe(true)
      expect(exceedsDragThreshold({ x: 10, y: 10 }, { x: 13, y: 14 })).toBe(true)
    })
  })

  describe('clearStoredBubblePosition', () => {
    let storage: Storage

    beforeEach(() => {
      storage = storageWith()
    })

    it('uses a stable per-device key', () => {
      expect(HELP_BUBBLE_STORAGE_KEY).toBe('entropia.help-bubble.position')
    })

    it('removes a stored T14 {right, bottom} value', () => {
      storage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"right":124,"bottom":96}')
      clearStoredBubblePosition(storage)
      expect(storage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBeNull()
    })

    it('removes a stored legacy {x, y} value: both shapes shared the one key', () => {
      storage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"x":120,"y":200}')
      clearStoredBubblePosition(storage)
      expect(storage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBeNull()
    })

    it('is a no-op when nothing was stored', () => {
      expect(() => clearStoredBubblePosition(storage)).not.toThrow()
      expect(storage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBeNull()
    })

    it('leaves the maximized key alone', () => {
      storage.setItem(HELP_BUBBLE_MAXIMIZED_KEY, 'true')
      storage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"right":1,"bottom":2}')
      clearStoredBubblePosition(storage)
      expect(storage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBeNull()
      expect(storage.getItem(HELP_BUBBLE_MAXIMIZED_KEY)).toBe('true')
    })

    it('never throws when storage is unavailable or denies access', () => {
      expect(() => clearStoredBubblePosition(null)).not.toThrow()
      expect(() => clearStoredBubblePosition(undefined)).not.toThrow()
      const throwing: Storage = {
        ...storage,
        removeItem: () => {
          throw new Error('denied')
        },
      }
      expect(() => clearStoredBubblePosition(throwing)).not.toThrow()
    })
  })

  describe('loadBubbleMaximized / saveBubbleMaximized', () => {
    let storage: Storage

    beforeEach(() => {
      storage = storageWith()
    })

    it('writes under its own per-device key', () => {
      expect(HELP_BUBBLE_MAXIMIZED_KEY).toBe('entropia.help-bubble.maximized')
    })

    it('defaults to the normal size', () => {
      expect(loadBubbleMaximized(storage)).toBe(false)
    })

    it('round-trips the maximized state', () => {
      saveBubbleMaximized(storage, true)
      expect(loadBubbleMaximized(storage)).toBe(true)
      saveBubbleMaximized(storage, false)
      expect(loadBubbleMaximized(storage)).toBe(false)
    })

    it('treats corrupt storage as the normal size', () => {
      storage.setItem(HELP_BUBBLE_MAXIMIZED_KEY, 'maybe')
      expect(loadBubbleMaximized(storage)).toBe(false)
    })

    it('never throws when storage is unavailable', () => {
      expect(loadBubbleMaximized(null)).toBe(false)
      expect(() => saveBubbleMaximized(null, true)).not.toThrow()
      const throwing: Storage = {
        ...storage,
        getItem: () => {
          throw new Error('denied')
        },
        setItem: () => {
          throw new Error('denied')
        },
      }
      expect(loadBubbleMaximized(throwing)).toBe(false)
      expect(() => saveBubbleMaximized(throwing, true)).not.toThrow()
    })
  })
})
