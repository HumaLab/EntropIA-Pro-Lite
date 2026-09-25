import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { watchStacking } from './resize-stacking'
import { MIN_PANE_PX } from './split-ratio'

type Callback = (entries: { contentRect: { width: number } }[]) => void

class FakeResizeObserver {
  static instances: FakeResizeObserver[] = []
  callback: Callback
  constructor(callback: Callback) {
    this.callback = callback
    FakeResizeObserver.instances.push(this)
  }
  observe() {}
  disconnect() {}
}

describe('watchStacking', () => {
  beforeEach(() => {
    FakeResizeObserver.instances = []
    vi.stubGlobal('ResizeObserver', FakeResizeObserver)
  })
  afterEach(() => vi.unstubAllGlobals())

  it('calls back only when crossing the two-pane threshold, not on every resize', () => {
    const onChange = vi.fn()
    const el = document.createElement('div')
    watchStacking(el, onChange)
    const observer = FakeResizeObserver.instances.at(-1)!

    observer.callback([{ contentRect: { width: 2 * MIN_PANE_PX + 40 } }])
    expect(onChange).not.toHaveBeenCalled()

    observer.callback([{ contentRect: { width: 2 * MIN_PANE_PX - 1 } }])
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith(true)

    observer.callback([{ contentRect: { width: 2 * MIN_PANE_PX - 2 } }])
    expect(onChange).toHaveBeenCalledTimes(1) // still stacked — no duplicate call

    observer.callback([{ contentRect: { width: 2 * MIN_PANE_PX + 1 } }])
    expect(onChange).toHaveBeenCalledTimes(2)
    expect(onChange).toHaveBeenLastCalledWith(false)
  })

  it('disconnects the observer on cleanup', () => {
    const el = document.createElement('div')
    const disconnect = vi.spyOn(FakeResizeObserver.prototype, 'disconnect')
    const stop = watchStacking(el, vi.fn())
    stop()
    expect(disconnect).toHaveBeenCalledTimes(1)
  })

  it('is a safe no-op when ResizeObserver is unavailable, rather than throwing', () => {
    vi.unstubAllGlobals()
    // @ts-expect-error -- deliberately simulating an environment without it
    delete globalThis.ResizeObserver
    const el = document.createElement('div')
    expect(() => watchStacking(el, vi.fn())()).not.toThrow()
  })
})
