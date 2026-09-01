import { describe, expect, it } from 'vitest'
import {
  computeVirtualGridWindow,
  resolveColumnCount,
  resolveFocusTarget,
} from './virtual-grid'

describe('computeVirtualGridWindow', () => {
  const base = {
    scrollTop: 250,
    gridOffset: 50,
    rowHeight: 100,
    columns: 2,
    totalItems: 10,
    overscanRows: 1,
    viewportHeight: 200,
  }

  it('returns the overscanned window and the spacers that hold its place', () => {
    expect(computeVirtualGridWindow(base)).toEqual({
      startIndex: 2,
      endIndex: 10,
      beforeHeight: 100,
      afterHeight: 0,
    })
  })

  it('starts at the top with no leading spacer before any scrolling', () => {
    // 200px of viewport over 100px rows shows 2 rows; overscan adds a third.
    // 3 rows x 2 columns = 6 items, and the 2 rows below become the spacer.
    expect(computeVirtualGridWindow({ ...base, scrollTop: 0 })).toEqual({
      startIndex: 0,
      endIndex: 6,
      beforeHeight: 0,
      afterHeight: 200,
    })
  })

  it('keeps the total spacer height equal to the rows it replaces', () => {
    const tall = { ...base, totalItems: 1000, scrollTop: 10_000 }
    const result = computeVirtualGridWindow(tall)
    const renderedRows = (result.endIndex - result.startIndex) / tall.columns
    const totalRows = tall.totalItems / tall.columns

    expect(result.beforeHeight + result.afterHeight + renderedRows * tall.rowHeight).toBe(
      totalRows * tall.rowHeight
    )
  })

  it('never runs past the end of the collection', () => {
    const result = computeVirtualGridWindow({ ...base, scrollTop: 99_999 })

    expect(result.endIndex).toBe(10)
    expect(result.afterHeight).toBe(0)
    expect(result.startIndex).toBeLessThan(result.endIndex)
  })

  it('handles a partly filled last row', () => {
    // 9 items in 2 columns is 4 full rows plus one holding a single card.
    const result = computeVirtualGridWindow({ ...base, totalItems: 9, scrollTop: 99_999 })

    expect(result).toEqual({
      startIndex: 8,
      endIndex: 9,
      beforeHeight: 400,
      afterHeight: 0,
    })
  })

  it('renders nothing for an empty collection', () => {
    expect(computeVirtualGridWindow({ ...base, totalItems: 0 })).toEqual({
      startIndex: 0,
      endIndex: 0,
      beforeHeight: 0,
      afterHeight: 0,
    })
  })

  it('degrades to the whole collection rather than nothing when height is unknown', () => {
    // happy-dom and a first paint both report zero height. Rendering an empty
    // window there would leave the grid permanently blank.
    expect(computeVirtualGridWindow({ ...base, rowHeight: 0, viewportHeight: 0 })).toEqual({
      startIndex: 0,
      endIndex: 10,
      beforeHeight: 0,
      afterHeight: 0,
    })
  })
})

describe('resolveColumnCount', () => {
  it('fits as many minimum-width columns as the container allows', () => {
    expect(resolveColumnCount({ containerWidth: 900, minColumnWidth: 260, gap: 12 })).toBe(3)
    expect(resolveColumnCount({ containerWidth: 560, minColumnWidth: 260, gap: 12 })).toBe(2)
  })

  it('never drops below a single column', () => {
    expect(resolveColumnCount({ containerWidth: 100, minColumnWidth: 260, gap: 12 })).toBe(1)
    expect(resolveColumnCount({ containerWidth: 0, minColumnWidth: 260, gap: 12 })).toBe(1)
  })
})

describe('resolveFocusTarget', () => {
  const rendered = ['doc-6', 'doc-7', 'doc-8']

  it('leaves focus alone while the focused card is still rendered', () => {
    expect(
      resolveFocusTarget({
        focusedKey: 'doc-7',
        previousKeys: rendered,
        nextKeys: ['doc-6', 'doc-7', 'doc-8', 'doc-9'],
      })
    ).toEqual({ kind: 'keep', key: 'doc-7' })
  })

  it('hands focus to the next surviving card when the focused one is evicted', () => {
    expect(
      resolveFocusTarget({
        focusedKey: 'doc-7',
        previousKeys: rendered,
        nextKeys: ['doc-8', 'doc-9', 'doc-10'],
      })
    ).toEqual({ kind: 'handoff', key: 'doc-8' })
  })

  it('falls back to the previous card when nothing after it survives', () => {
    expect(
      resolveFocusTarget({
        focusedKey: 'doc-7',
        previousKeys: rendered,
        nextKeys: ['doc-4', 'doc-5', 'doc-6'],
      })
    ).toEqual({ kind: 'handoff', key: 'doc-6' })
  })

  it('falls back to the container when no rendered card survives', () => {
    expect(
      resolveFocusTarget({ focusedKey: 'doc-7', previousKeys: rendered, nextKeys: [] })
    ).toEqual({ kind: 'container', key: null })
  })

  it('does nothing when nothing was focused', () => {
    expect(
      resolveFocusTarget({ focusedKey: null, previousKeys: rendered, nextKeys: ['doc-9'] })
    ).toEqual({ kind: 'keep', key: null })
  })

  it('falls back to the container when the focused card was never rendered here', () => {
    expect(
      resolveFocusTarget({ focusedKey: 'ghost', previousKeys: rendered, nextKeys: rendered })
    ).toEqual({ kind: 'container', key: null })
  })
})
