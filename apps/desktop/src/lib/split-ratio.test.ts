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

  it('falls back to the coarse [0.15, 0.85] bound for a zero or unknown container size', () => {
    expect(clampSplitRatio(0.02, 0)).toBe(0.15)
    expect(clampSplitRatio(0.99, NaN)).toBe(0.85)
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
