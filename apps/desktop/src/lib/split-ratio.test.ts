import { describe, it, expect } from 'vitest'
import { clampSplitRatio, shouldStack, MIN_PANE_PX } from './split-ratio'

describe('clampSplitRatio', () => {
  it('leaves a mid-range ratio untouched in a wide container', () => {
    expect(clampSplitRatio(0.5, 1000)).toBe(0.5)
  })

  it('clamps to the fraction that keeps both panes at least 320px', () => {
    // 320 / 1000 = 0.32
    expect(clampSplitRatio(0.01, 1000)).toBeCloseTo(0.32)
    expect(clampSplitRatio(0.99, 1000)).toBeCloseTo(0.68)
  })

  it('clamps to exactly 0.5 when the container is precisely two panes wide', () => {
    expect(clampSplitRatio(0.9, 2 * MIN_PANE_PX)).toBe(0.5)
  })

  it('falls back to the coarse [0.25, 0.75] bound for a zero or unknown container size', () => {
    expect(clampSplitRatio(0.02, 0)).toBe(0.25)
    expect(clampSplitRatio(0.99, NaN)).toBe(0.75)
  })

  /**
   * Each pane can shrink to at most 25% of the split — the ratio is clamped
   * to [0.25, 0.75] — in addition to the existing 320px-per-pane floor. The
   * stricter of the two wins: in a wide container the 320px floor allows a
   * fraction well under 0.25 (e.g. 320/2000 = 0.16), but the 25% rule still
   * refuses it.
   */
  describe('the 25% ratio floor is at least as strict as the 320px pane floor', () => {
    it('refuses a ratio below 0.25 even when 320px alone would allow it, in a wide container', () => {
      // 320 / 2000 = 0.16 — the pixel floor alone would allow 0.16, but the
      // 25% ratio floor is stricter and wins.
      expect(clampSplitRatio(0.05, 2000)).toBe(0.25)
      expect(clampSplitRatio(0.95, 2000)).toBe(0.75)
    })

    it('holds exactly at the 0.25/0.75 boundary', () => {
      expect(clampSplitRatio(0.25, 2000)).toBe(0.25)
      expect(clampSplitRatio(0.75, 2000)).toBe(0.75)
    })

    it('still enforces the 320px pane floor when it is the stricter bound, in a narrow container', () => {
      // 320 / 1000 = 0.32, stricter than the 0.25 ratio floor.
      expect(clampSplitRatio(0.05, 1000)).toBeCloseTo(0.32)
      expect(clampSplitRatio(0.95, 1000)).toBeCloseTo(0.68)
    })

    it('leaves a ratio already inside both bounds untouched', () => {
      expect(clampSplitRatio(0.4, 2000)).toBe(0.4)
    })
  })
})

describe('shouldStack', () => {
  it('does not stack when two 320px panes plus the divider still fit', () => {
    expect(shouldStack(2 * MIN_PANE_PX + 40)).toBe(false)
  })

  it('stacks once the container is narrower than two panes can fit', () => {
    expect(shouldStack(2 * MIN_PANE_PX - 1)).toBe(true)
  })
})
