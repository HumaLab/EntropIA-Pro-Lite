import { describe, expect, it } from 'vitest'
import {
  MIN_LAYOUT_EDIT_SIDE,
  dragLayoutBox,
  layoutHandleCenter,
  roundLayoutBox,
} from './layoutBoxEdit'

const box = { x: 100, y: 100, width: 200, height: 50 }

describe('dragLayoutBox', () => {
  it('stretches only the dragged edge', () => {
    expect(dragLayoutBox(box, 's', 30, 80, 1000, 1000)).toEqual({
      x: 100,
      y: 100,
      width: 200,
      height: 130,
    })
    expect(dragLayoutBox(box, 'nw', -20, -10, 1000, 1000)).toEqual({
      x: 80,
      y: 90,
      width: 220,
      height: 60,
    })
  })

  it('moves the whole box inside the page', () => {
    expect(dragLayoutBox(box, 'move', 10, 20, 1000, 1000)).toEqual({
      x: 110,
      y: 120,
      width: 200,
      height: 50,
    })
    expect(dragLayoutBox(box, 'move', 5000, 5000, 1000, 1000)).toEqual({
      x: 800,
      y: 950,
      width: 200,
      height: 50,
    })
  })

  it('never inverts or collapses the box and clamps to the page', () => {
    const collapsed = dragLayoutBox(box, 's', 0, -500, 1000, 1000)
    expect(collapsed.height).toBe(MIN_LAYOUT_EDIT_SIDE)
    expect(collapsed.y).toBe(100)
    expect(dragLayoutBox(box, 'e', 5000, 0, 1000, 1000).width).toBe(900)
    expect(dragLayoutBox(box, 'n', 0, -5000, 1000, 1000).y).toBe(0)
  })
})

describe('layout handle helpers', () => {
  it('rounds to whole pixels keeping the far edges', () => {
    expect(roundLayoutBox({ x: 10.4, y: 9.6, width: 20.4, height: 0.2 })).toEqual({
      x: 10,
      y: 10,
      width: 21,
      height: 1,
    })
  })

  it('places handles on corners and edge midpoints', () => {
    expect(layoutHandleCenter(box, 'se')).toEqual({ x: 300, y: 150 })
    expect(layoutHandleCenter(box, 'n')).toEqual({ x: 200, y: 100 })
    expect(layoutHandleCenter(box, 'w')).toEqual({ x: 100, y: 125 })
  })
})
