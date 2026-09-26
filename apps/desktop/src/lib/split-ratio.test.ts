import { describe, it, expect } from 'vitest'
import { clampSplitRatio, fitsSideBySide, MIN_PANE_PX, SPLIT_DIVIDER_PX } from './split-ratio'

describe('clampSplitRatio', () => {
  it('leaves a mid-range ratio untouched in a wide container', () => {
    expect(clampSplitRatio(0.5, 1400)).toBe(0.5)
  })

  it('clamps to the fraction that keeps both panes at least 640px', () => {
    // 640 / 1500 ≈ 0.427, stricter than the 40% ratio floor.
    expect(clampSplitRatio(0.01, 1500)).toBeCloseTo(640 / 1500)
    expect(clampSplitRatio(0.99, 1500)).toBeCloseTo(1 - 640 / 1500)
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
   * [0.4, 0.6] (user rule) — in addition to the 640px-per-pane floor. The
   * stricter of the two wins: in a container of 1600px or more the 40% rule
   * is stricter; below that the 640px floor is.
   */
  describe('the 40% ratio floor and the 640px pane floor, stricter wins', () => {
    it('refuses a ratio below 0.4 even when 640px alone would allow it, in a wide container', () => {
      // 640 / 2000 = 0.32 — the pixel floor alone would allow 0.32, but the
      // 40% ratio floor is stricter and wins.
      expect(clampSplitRatio(0.05, 2000)).toBe(0.4)
      expect(clampSplitRatio(0.95, 2000)).toBe(0.6)
    })

    it('holds exactly at the 0.4/0.6 boundary', () => {
      expect(clampSplitRatio(0.4, 2000)).toBe(0.4)
      expect(clampSplitRatio(0.6, 2000)).toBe(0.6)
    })

    it('still enforces the 640px pane floor when it is the stricter bound, in a narrow container', () => {
      // 640 / 1500 ≈ 0.427, stricter than the 0.4 ratio floor.
      expect(clampSplitRatio(0.05, 1500)).toBeCloseTo(640 / 1500)
      expect(clampSplitRatio(0.95, 1500)).toBeCloseTo(1 - 640 / 1500)
    })

    it('leaves a ratio already inside both bounds untouched', () => {
      expect(clampSplitRatio(0.45, 2000)).toBe(0.45)
    })
  })
})

describe('fitsSideBySide', () => {
  // User rule 2026-09-25: when the split area cannot give BOTH panes at
  // least 640px side by side, there is no split view at all — no vertical
  // stacking fallback. `fitsSideBySide` is the single source of truth for
  // that decision, shared by the TopBar toggle (disable it) and AppShell
  // (collapse to the active pane alone).
  it('keeps a 640px floor per pane', () => {
    expect(MIN_PANE_PX).toBe(640)
    expect(fitsSideBySide(1285)).toBe(false)
    expect(fitsSideBySide(1300)).toBe(true)
  })

  it('accounts for the real divider width, not just the two panes', () => {
    expect(SPLIT_DIVIDER_PX).toBe(6)
    expect(fitsSideBySide(2 * MIN_PANE_PX)).toBe(false)
    expect(fitsSideBySide(2 * MIN_PANE_PX + SPLIT_DIVIDER_PX)).toBe(true)
  })

  it('fits when two 640px panes plus the divider still fit', () => {
    expect(fitsSideBySide(2 * MIN_PANE_PX + 40)).toBe(true)
  })

  it('does not fit once the container is narrower than two panes plus the divider', () => {
    expect(fitsSideBySide(2 * MIN_PANE_PX + SPLIT_DIVIDER_PX - 1)).toBe(false)
  })

  it('accepts a custom divider width, e.g. a differently themed build', () => {
    expect(fitsSideBySide(2 * MIN_PANE_PX + 10, 10)).toBe(true)
    expect(fitsSideBySide(2 * MIN_PANE_PX + 9, 10)).toBe(false)
  })
})
