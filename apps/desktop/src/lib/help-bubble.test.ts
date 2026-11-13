import { beforeEach, describe, expect, it } from 'vitest'
import {
  clampBubblePosition,
  defaultBubblePosition,
  exceedsDragThreshold,
  HELP_BUBBLE_DRAG_THRESHOLD_PX,
  HELP_BUBBLE_STORAGE_KEY,
  loadBubblePosition,
  saveBubblePosition,
  type BubblePosition,
  type BubbleViewport,
} from './help-bubble'

const VIEWPORT: BubbleViewport = { width: 1024, height: 768 }

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

describe('help-bubble position helpers (S13, D9)', () => {
  describe('defaultBubblePosition', () => {
    it('sits at the bottom-right with a margin that clears the status bar', () => {
      // 56px bubble: 24px from the right edge, 30px status bar + 16px above it.
      expect(defaultBubblePosition(VIEWPORT)).toEqual({ x: 1024 - 56 - 24, y: 768 - 56 - 46 })
    })

    it('never leaves the window, even on a tiny viewport', () => {
      const position = defaultBubblePosition({ width: 200, height: 100 })
      expect(position.x).toBeGreaterThanOrEqual(0)
      expect(position.y).toBeGreaterThanOrEqual(0)
      expect(position.x + 56).toBeLessThanOrEqual(200)
      expect(position.y + 56).toBeLessThanOrEqual(100)
    })
  })

  describe('clampBubblePosition', () => {
    it('keeps a position inside the window untouched', () => {
      const position: BubblePosition = { x: 100, y: 100 }
      expect(clampBubblePosition(position, VIEWPORT)).toEqual(position)
    })

    it('pulls negative coordinates back inside', () => {
      expect(clampBubblePosition({ x: -50, y: -20 }, VIEWPORT).x).toBeGreaterThanOrEqual(0)
      expect(clampBubblePosition({ x: -50, y: -20 }, VIEWPORT).y).toBeGreaterThanOrEqual(0)
    })

    it('pulls coordinates past the far edges back inside', () => {
      const clamped = clampBubblePosition({ x: 5000, y: 5000 }, VIEWPORT)
      expect(clamped.x + 56).toBeLessThanOrEqual(VIEWPORT.width)
      expect(clamped.y + 56).toBeLessThanOrEqual(VIEWPORT.height)
    })

    it('survives a degenerate viewport without NaN', () => {
      const clamped = clampBubblePosition({ x: 10, y: 10 }, { width: 0, height: 0 })
      expect(Number.isFinite(clamped.x)).toBe(true)
      expect(Number.isFinite(clamped.y)).toBe(true)
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

  describe('loadBubblePosition / saveBubblePosition', () => {
    let storage: Storage

    beforeEach(() => {
      storage = storageWith()
    })

    it('round-trips a saved position', () => {
      saveBubblePosition(storage, { x: 123, y: 456 })
      expect(loadBubblePosition(storage)).toEqual({ x: 123, y: 456 })
    })

    it('writes under a stable per-device key', () => {
      expect(HELP_BUBBLE_STORAGE_KEY).toBe('entropia.help-bubble.position')
      saveBubblePosition(storage, { x: 1, y: 2 })
      expect(storage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBe('{"x":1,"y":2}')
    })

    it('returns null when nothing was saved', () => {
      expect(loadBubblePosition(storage)).toBeNull()
    })

    it.each([
      ['not json at all', 'not json at all'],
      ['a bare number', '42'],
      ['an array', '[1,2]'],
      ['a missing y', '{"x":1}'],
      ['a missing x', '{"y":2}'],
      ['a NaN coordinate', '{"x":NaN,"y":2}'],
      ['an infinite coordinate', '{"x":1e999,"y":2}'],
      ['a string coordinate', '{"x":"1","y":2}'],
    ])('falls back to default on corrupt storage (%s)', (_label, raw) => {
      storage.setItem(HELP_BUBBLE_STORAGE_KEY, raw)
      expect(loadBubblePosition(storage)).toBeNull()
    })

    it('falls back to default when storage itself is unavailable', () => {
      expect(loadBubblePosition(null)).toBeNull()
      expect(loadBubblePosition(undefined)).toBeNull()
      expect(() => saveBubblePosition(null, { x: 1, y: 2 })).not.toThrow()
    })

    it('falls back to default when storage throws', () => {
      const throwing: Storage = {
        ...storage,
        getItem: () => {
          throw new Error('denied')
        },
        setItem: () => {
          throw new Error('denied')
        },
      }
      expect(loadBubblePosition(throwing)).toBeNull()
      expect(() => saveBubblePosition(throwing, { x: 1, y: 2 })).not.toThrow()
    })
  })
})
