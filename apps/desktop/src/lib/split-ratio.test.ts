import { describe, it, expect } from 'vitest'
import { clampSplitRatio, shouldStack, MIN_PANE_PX } from './split-ratio'

describe('clampSplitRatio', () => {
  it('leaves a mid-range ratio untouched in a wide container', () => {
    expect(clampSplitRatio(0.5, 1000)).toBe(0.5)
  })

  it('clamps to the fraction that keeps both panes at least 320px', () => {
    // 320 / 700 ≈ 0.457, stricter than the 40% ratio floor.
    expect(clampSplitRatio(0.01, 700)).toBeCloseTo(320 / 700)
    expect(clampSplitRatio(0.99, 700)).toBeCloseTo(1 - 320 / 700)
  })

  it('clamps to exactly 0.5 when the container is precisely two panes wide', () => {
    expect(clampSplitRatio(0.9, 2 * MIN_PANE_PX)).toBe(0.5)
  })

  it('falls back to the coarse [0.4, 0.6] bound for a zero or unknown container size', () => {
    expect(clampSplitRatio(0.02, 0)).toBe(0.4)
    expect(clampSplitRatio(0.99, NaN)).toBe(0.6)
  })

  /**
   * Each pane keeps at least 40% of the split — the ratio is clamped to
   * [0.4, 0.6] (user rule) — in addition to the 320px-per-pane floor. The
   * stricter of the two wins: in a container of 800px or more the 40% rule
   * is stricter; below that the 320px floor is.
   */
  describe('the 40% ratio floor and the 320px pane floor, stricter wins', () => {
    it('refuses a ratio below 0.4 even when 320px alone would allow it, in a wide container', () => {
      // 320 / 2000 = 0.16 — the pixel floor alone would allow 0.16, but the
      // 40% ratio floor is stricter and wins.
      expect(clampSplitRatio(0.05, 2000)).toBe(0.4)
      expect(clampSplitRatio(0.95, 2000)).toBe(0.6)
    })

    it('holds exactly at the 0.4/0.6 boundary', () => {
      expect(clampSplitRatio(0.4, 2000)).toBe(0.4)
      expect(clampSplitRatio(0.6, 2000)).toBe(0.6)
    })

    it('still enforces the 320px pane floor when it is the stricter bound, in a narrow container', () => {
      // 320 / 700 ≈ 0.457, stricter than the 0.4 ratio floor.
      expect(clampSplitRatio(0.05, 700)).toBeCloseTo(320 / 700)
      expect(clampSplitRatio(0.95, 700)).toBeCloseTo(1 - 320 / 700)
    })

    it('leaves a ratio already inside both bounds untouched', () => {
      expect(clampSplitRatio(0.45, 2000)).toBe(0.45)
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
