import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { watchSplitFit } from './resize-split-fit'
import { MIN_PANE_PX, SPLIT_DIVIDER_PX } from './split-ratio'

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

describe('watchSplitFit', () => {
  beforeEach(() => {
    FakeResizeObserver.instances = []
    vi.stubGlobal('ResizeObserver', FakeResizeObserver)
  })
  afterEach(() => vi.unstubAllGlobals())

  const THRESHOLD = 2 * MIN_PANE_PX + SPLIT_DIVIDER_PX

  it('calls back only when crossing the side-by-side threshold, not on every resize', () => {
    const onChange = vi.fn()
    const el = document.createElement('div')
    watchSplitFit(el, onChange)
    const observer = FakeResizeObserver.instances.at(-1)!

    observer.callback([{ contentRect: { width: THRESHOLD + 40 } }])
    expect(onChange).not.toHaveBeenCalled()

    observer.callback([{ contentRect: { width: THRESHOLD - 1 } }])
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith(false)

    observer.callback([{ contentRect: { width: THRESHOLD - 2 } }])
    expect(onChange).toHaveBeenCalledTimes(1) // still doesn't fit — no duplicate call

    observer.callback([{ contentRect: { width: THRESHOLD + 1 } }])
    expect(onChange).toHaveBeenCalledTimes(2)
    expect(onChange).toHaveBeenLastCalledWith(true)
  })

  it('disconnects the observer on cleanup', () => {
    const el = document.createElement('div')
    const disconnect = vi.spyOn(FakeResizeObserver.prototype, 'disconnect')
    const stop = watchSplitFit(el, vi.fn())
    stop()
    expect(disconnect).toHaveBeenCalledTimes(1)
  })

  it('is a safe no-op when ResizeObserver is unavailable, rather than throwing', () => {
    vi.unstubAllGlobals()
    // @ts-expect-error -- deliberately simulating an environment without it
    delete globalThis.ResizeObserver
    const el = document.createElement('div')
    expect(() => watchSplitFit(el, vi.fn())()).not.toThrow()
  })
})
