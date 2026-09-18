import { describe, expect, it } from 'vitest'

import { placeMenu } from '../menu-placement'

const VIEWPORT = { width: 800, height: 600 }
const MENU = { width: 200, height: 150 }

function anchorAt(left: number, top = 40, width = 28, height = 28) {
  return { left, top, width, height }
}

describe('placeMenu', () => {
  it('opens below the trigger, aligned to its start edge', () => {
    expect(placeMenu({ anchor: anchorAt(100), menu: MENU, viewport: VIEWPORT })).toEqual({
      top: 40 + 28 + 4,
      left: 100,
      maxHeight: 600 - 72 - 8,
    })
  })

  it('aligns to the end edge when asked', () => {
    const placed = placeMenu({
      anchor: anchorAt(400),
      menu: MENU,
      viewport: VIEWPORT,
      align: 'end',
    })

    expect(placed.left).toBe(400 + 28 - 200)
  })

  it('flips to the end edge rather than run off the right of the window', () => {
    const placed = placeMenu({ anchor: anchorAt(700), menu: MENU, viewport: VIEWPORT })

    expect(placed.left).toBe(700 + 28 - 200)
  })

  it('flips to the start edge rather than run off the left of the window', () => {
    const placed = placeMenu({ anchor: anchorAt(20), menu: MENU, viewport: VIEWPORT, align: 'end' })

    expect(placed.left).toBe(20)
  })

  it('shifts inside the margin when neither edge fits', () => {
    const narrow = { width: 240, height: 600 }

    // A 200px menu in a 240px window may start anywhere in [8, 32].
    expect(placeMenu({ anchor: anchorAt(2), menu: MENU, viewport: narrow }).left).toBe(8)
    expect(placeMenu({ anchor: anchorAt(220), menu: MENU, viewport: narrow }).left).toBe(32)
  })

  it('pins to the margin when the menu is wider than the window', () => {
    const tiny = { width: 150, height: 600 }

    expect(placeMenu({ anchor: anchorAt(60), menu: MENU, viewport: tiny }).left).toBe(8)
  })

  it('opens above when there is no room below and there is above', () => {
    const placed = placeMenu({ anchor: anchorAt(100, 500), menu: MENU, viewport: VIEWPORT })

    expect(placed.top).toBe(500 - 4 - 150)
    expect(placed.maxHeight).toBe(500 - 4 - 8)
  })

  it('stays below, capped to the room there, when neither side has enough', () => {
    const placed = placeMenu({
      anchor: anchorAt(100, 100),
      menu: { width: 200, height: 580 },
      viewport: VIEWPORT,
    })

    expect(placed.top).toBe(132)
    expect(placed.maxHeight).toBe(600 - 132 - 8)
  })
})
