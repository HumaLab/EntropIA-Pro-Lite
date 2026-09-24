import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createDbBrowserAutoReload } from './db-browser-auto-reload'

describe('createDbBrowserAutoReload', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('does nothing until notified', () => {
    const onReload = vi.fn()
    createDbBrowserAutoReload({ onReload, delayMs: 500 })

    vi.advanceTimersByTime(5000)

    expect(onReload).not.toHaveBeenCalled()
  })

  it('reloads once after the trailing debounce window', () => {
    const onReload = vi.fn()
    const { notify } = createDbBrowserAutoReload({ onReload, delayMs: 500 })

    notify()
    vi.advanceTimersByTime(499)
    expect(onReload).not.toHaveBeenCalled()

    vi.advanceTimersByTime(1)
    expect(onReload).toHaveBeenCalledTimes(1)
  })

  it('coalesces a burst of notifications into exactly one reload', () => {
    const onReload = vi.fn()
    const { notify } = createDbBrowserAutoReload({ onReload, delayMs: 500 })

    // A running OCR batch: many change signals close together must not each
    // trigger their own reload.
    for (let i = 0; i < 20; i++) {
      notify()
      vi.advanceTimersByTime(100)
    }
    expect(onReload).not.toHaveBeenCalled()

    vi.advanceTimersByTime(500)
    expect(onReload).toHaveBeenCalledTimes(1)
  })

  it('never overlaps reloads: a notify during an in-flight reload queues exactly one follow-up', async () => {
    let resolveReload!: () => void
    const onReload = vi.fn().mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveReload = resolve
        })
    )
    const { notify } = createDbBrowserAutoReload({ onReload, delayMs: 500 })

    notify()
    await vi.advanceTimersByTimeAsync(500)
    expect(onReload).toHaveBeenCalledTimes(1)

    // Changes arrive while the first reload is still running.
    notify()
    await vi.advanceTimersByTimeAsync(500)
    notify()
    await vi.advanceTimersByTimeAsync(500)
    expect(onReload).toHaveBeenCalledTimes(1)

    resolveReload()
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    expect(onReload).toHaveBeenCalledTimes(2)
  })

  it('resets the debounce window on every notify (trailing edge only)', () => {
    const onReload = vi.fn()
    const { notify } = createDbBrowserAutoReload({ onReload, delayMs: 500 })

    notify()
    vi.advanceTimersByTime(400)
    notify()
    vi.advanceTimersByTime(400)
    expect(onReload).not.toHaveBeenCalled()

    vi.advanceTimersByTime(100)
    expect(onReload).toHaveBeenCalledTimes(1)
  })

  it('dispose stops a pending reload and ignores further notify calls', () => {
    const onReload = vi.fn()
    const { notify, dispose } = createDbBrowserAutoReload({ onReload, delayMs: 500 })

    notify()
    dispose()
    vi.advanceTimersByTime(5000)
    expect(onReload).not.toHaveBeenCalled()

    notify()
    vi.advanceTimersByTime(5000)
    expect(onReload).not.toHaveBeenCalled()
  })

  it('dispose during an in-flight reload drops the queued follow-up', async () => {
    let resolveReload!: () => void
    const onReload = vi.fn().mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveReload = resolve
        })
    )
    const { notify, dispose } = createDbBrowserAutoReload({ onReload, delayMs: 500 })

    notify()
    await vi.advanceTimersByTimeAsync(500)
    expect(onReload).toHaveBeenCalledTimes(1)

    notify()
    await vi.advanceTimersByTimeAsync(500)
    dispose()

    resolveReload()
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    expect(onReload).toHaveBeenCalledTimes(1)
  })

  it('swallows an onReload rejection instead of leaving the coalescer stuck', async () => {
    const onReload = vi.fn().mockRejectedValueOnce(new Error('boom')).mockResolvedValue(undefined)
    const { notify } = createDbBrowserAutoReload({ onReload, delayMs: 500 })

    notify()
    await vi.advanceTimersByTimeAsync(500)
    expect(onReload).toHaveBeenCalledTimes(1)

    notify()
    await vi.advanceTimersByTimeAsync(500)
    expect(onReload).toHaveBeenCalledTimes(2)
  })
})
