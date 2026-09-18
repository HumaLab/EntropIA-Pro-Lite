import { describe, expect, it } from 'vitest'

import { fitToolbar, rowWidth, type ToolbarFitGroup } from './toolbar-fit'

/**
 * The toolbar's layout arithmetic, kept apart from the DOM so it can be held
 * exactly. A flex row costs the widths of its units plus one boundary between
 * each pair: a bare gap when the later unit is joined to the one before it, a
 * separator with a gap on each side otherwise.
 */
const GAP = 4
const SEPARATOR = 9 // 1px line plus 4px margin each side
const OVERFLOW = 28
const SEPARATED = GAP + SEPARATOR + GAP // 17

const GROUPS: ToolbarFitGroup[] = [
  { id: 'history', width: 60, priority: 'pinned' },
  { id: 'marks', width: 92, priority: 'pinned' },
  { id: 'marksExtra', width: 60, priority: 1, joined: true },
  { id: 'headings', width: 92, priority: 4 },
  { id: 'lists', width: 92, priority: 2 },
  { id: 'insert', width: 92, priority: 3 },
  { id: 'utility', width: 73, priority: 'pinned', joined: true },
]

function fit(available: number, groups = GROUPS) {
  return fitToolbar({
    groups,
    available,
    gap: GAP,
    separatorWidth: SEPARATOR,
    overflowWidth: OVERFLOW,
    overflowBefore: 'utility',
  })
}

/** Everything on one row, with no trigger. */
const NATURAL =
  60 + SEPARATED + 92 + GAP + 60 + SEPARATED + 92 + SEPARATED + 92 + SEPARATED + 92 + GAP + 73

describe('rowWidth', () => {
  it('adds a bare gap before a joined unit and a separator before any other', () => {
    expect(
      rowWidth(
        [{ width: 10 }, { width: 20, joined: true }, { width: 30, joined: false }],
        GAP,
        SEPARATOR
      )
    ).toBe(10 + GAP + 20 + SEPARATED + 30)
  })

  it('costs nothing for an empty row, and no boundary before the first unit', () => {
    expect(rowWidth([], GAP, SEPARATOR)).toBe(0)
    expect(rowWidth([{ width: 12, joined: true }], GAP, SEPARATOR)).toBe(12)
  })
})

describe('fitToolbar', () => {
  it('shows every group and no trigger when everything fits', () => {
    expect(fit(NATURAL)).toEqual({
      visible: GROUPS.map((group) => group.id),
      hidden: [],
      overflow: false,
    })
  })

  it('collapses the lowest priority first, and counts the trigger it now needs', () => {
    // One pixel short: marksExtra (priority 1) goes, and the trigger comes in
    // as a separated unit before `utility`, which joins it.
    const result = fit(NATURAL - 1)

    expect(result.hidden).toEqual(['marksExtra'])
    expect(result.overflow).toBe(true)
    expect(result.visible).toEqual(['history', 'marks', 'headings', 'lists', 'insert', 'utility'])
  })

  it('keeps collapsing in priority order, never by position', () => {
    const withoutExtra = NATURAL - (GAP + 60) + SEPARATED + OVERFLOW
    // Just enough for that: only the first group goes.
    expect(fit(withoutExtra).hidden).toEqual(['marksExtra'])
    // One pixel less: lists (priority 2) next, although insert sits after it.
    expect(fit(withoutExtra - 1).hidden).toEqual(['marksExtra', 'lists'])
  })

  it('never collapses a pinned group, even when nothing else is left to hide', () => {
    const result = fit(10)

    expect(result.hidden).toEqual(['marksExtra', 'lists', 'insert', 'headings'])
    expect(result.visible).toEqual(['history', 'marks', 'utility'])
    expect(result.overflow).toBe(true)
  })

  it('keeps the visible groups in their own order', () => {
    const result = fit(60 + SEPARATED + 92 + SEPARATED + 92 + SEPARATED + OVERFLOW + GAP + 73)

    // headings (priority 4) outlasts lists and insert and stays in its slot.
    expect(result.visible).toEqual(['history', 'marks', 'headings', 'utility'])
  })

  it('puts the trigger at the end when the group it precedes is not there', () => {
    const groups = GROUPS.filter((group) => group.id !== 'utility')
    const natural = NATURAL - GAP - 73

    expect(fit(natural, groups).hidden).toEqual([])
    // Trailing trigger: marks + separator + trigger instead of the extra group.
    expect(fit(natural - 1, groups).hidden).toEqual(['marksExtra'])
  })

  it('breaks priority ties from the right, so the row shrinks from its end', () => {
    const groups: ToolbarFitGroup[] = [
      { id: 'a', width: 50, priority: 'pinned' },
      { id: 'b', width: 50, priority: 1 },
      { id: 'c', width: 50, priority: 1 },
    ]
    const result = fitToolbar({
      groups,
      available: 50 + SEPARATED + 50 + SEPARATED + OVERFLOW,
      gap: GAP,
      separatorWidth: SEPARATOR,
      overflowWidth: OVERFLOW,
    })

    expect(result.hidden).toEqual(['c'])
  })

  it('does not collapse anything when there is no width to measure yet', () => {
    // A toolbar that has not been laid out (hidden tab, first frame) reports
    // zero. Collapsing everything on that would flash an empty toolbar.
    expect(fit(0).hidden).toEqual([])
  })
})
