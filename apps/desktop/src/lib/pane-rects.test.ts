import { describe, it, expect, beforeEach } from 'vitest'
import { registerPaneRect, unregisterPaneRect, currentPaneRects } from './pane-rects'

describe('pane-rects registry', () => {
  beforeEach(() => {
    for (const { paneId } of currentPaneRects()) unregisterPaneRect(paneId)
  })

  it("reports every registered pane's rect", () => {
    registerPaneRect('a', () => ({ left: 0, top: 0, right: 10, bottom: 10 }))
    registerPaneRect('b', () => ({ left: 10, top: 0, right: 20, bottom: 10 }))
    expect(
      currentPaneRects()
        .map((p) => p.paneId)
        .sort()
    ).toEqual(['a', 'b'])
  })

  it('re-measures on every call rather than caching a stale rect', () => {
    let width = 100
    registerPaneRect('a', () => ({ left: 0, top: 0, right: width, bottom: 10 }))
    expect(currentPaneRects()[0]!.rect.right).toBe(100)
    width = 200
    expect(currentPaneRects()[0]!.rect.right).toBe(200)
  })

  it('unregister removes the pane', () => {
    registerPaneRect('a', () => ({ left: 0, top: 0, right: 10, bottom: 10 }))
    unregisterPaneRect('a')
    expect(currentPaneRects()).toEqual([])
  })
})
