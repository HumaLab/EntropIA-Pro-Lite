import { afterEach, describe, expect, it, vi } from 'vitest'

const handlers: { resized: Array<() => void>; focus: Array<() => void> } = {
  resized: [],
  focus: [],
}
const unlistenResized = vi.fn()
const unlistenFocus = vi.fn()
let minimized = false
let insideTauri = true

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn(() => {
    if (!insideTauri) throw new Error('window.__TAURI_INTERNALS__ is undefined')
    return {
      isMinimized: vi.fn(async () => minimized),
      onResized: vi.fn(async (handler: () => void) => {
        handlers.resized.push(handler)
        return unlistenResized
      }),
      onFocusChanged: vi.fn(async (handler: () => void) => {
        handlers.focus.push(handler)
        return unlistenFocus
      }),
    }
  }),
}))

import { watchWindowMinimized } from './window-minimized'

/** Lets the pending isMinimized()/listener promises settle. */
async function flush() {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}

describe('watchWindowMinimized', () => {
  afterEach(() => {
    handlers.resized.length = 0
    handlers.focus.length = 0
    unlistenResized.mockClear()
    unlistenFocus.mockClear()
    minimized = false
    insideTauri = true
  })

  it('reports minimize and restore from the window resize events, once each', async () => {
    const onChange = vi.fn()
    watchWindowMinimized(onChange)
    await flush()

    minimized = true
    handlers.resized[0]!()
    handlers.focus[0]!()
    await flush()
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenLastCalledWith(true)

    minimized = false
    handlers.resized[0]!()
    await flush()
    expect(onChange).toHaveBeenCalledTimes(2)
    expect(onChange).toHaveBeenLastCalledWith(false)
  })

  it('stops listening and reporting once stopped', async () => {
    const onChange = vi.fn()
    const stop = watchWindowMinimized(onChange)
    await flush()

    stop()
    minimized = true
    handlers.resized[0]!()
    await flush()

    expect(unlistenResized).toHaveBeenCalledTimes(1)
    expect(unlistenFocus).toHaveBeenCalledTimes(1)
    expect(onChange).not.toHaveBeenCalled()
  })

  it('releases the listeners even when stopped before they finished registering', async () => {
    const stop = watchWindowMinimized(vi.fn())
    stop()
    await flush()

    expect(unlistenResized).toHaveBeenCalledTimes(1)
    expect(unlistenFocus).toHaveBeenCalledTimes(1)
  })

  it('is a no-op outside Tauri', () => {
    insideTauri = false
    const onChange = vi.fn()

    const stop = watchWindowMinimized(onChange)

    expect(() => stop()).not.toThrow()
    expect(onChange).not.toHaveBeenCalled()
  })
})
