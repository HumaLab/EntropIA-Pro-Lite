import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FIRST_PAINT_TIMEOUT_MS, waitForFirstPaint } from './first-paint'

// The main window starts hidden (tauri.conf.json `visible: false`). WebView2
// runs requestAnimationFrame for it anyway; WKWebView and WebKitGTK never do,
// which left macOS and Linux waiting for the 20 s splash watchdog.
describe('waitForFirstPaint', () => {
  let rafCallbacks: FrameRequestCallback[]

  beforeEach(() => {
    vi.useFakeTimers()
    rafCallbacks = []
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
      rafCallbacks.push(cb)
      return rafCallbacks.length
    })
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  function flushFrame() {
    const pending = rafCallbacks
    rafCallbacks = []
    for (const cb of pending) cb(performance.now())
  }

  it('resolves after two frames when the engine paints the view (WebView2)', async () => {
    let done = false
    void waitForFirstPaint('windows').then(() => (done = true))

    flushFrame()
    await Promise.resolve()
    expect(done).toBe(false)

    flushFrame()
    await vi.advanceTimersByTimeAsync(0)
    expect(done).toBe(true)
  })

  it('does not wait on frames that a hidden window never gets (WKWebView, WebKitGTK)', async () => {
    let done = false
    void waitForFirstPaint('macos').then(() => (done = true))

    // No frame is ever delivered.
    await vi.advanceTimersByTimeAsync(FIRST_PAINT_TIMEOUT_MS - 1)
    expect(done).toBe(false)
    await vi.advanceTimersByTimeAsync(1)
    expect(done).toBe(true)
  })

  it('on Windows it never reveals before the frames: WebView2 always paints, a timeout could only flash white', async () => {
    let done = false
    void waitForFirstPaint('windows').then(() => (done = true))

    // A slow first launch: no frame for far longer than the bound.
    await vi.advanceTimersByTimeAsync(FIRST_PAINT_TIMEOUT_MS * 20)
    expect(done).toBe(false)

    flushFrame()
    flushFrame()
    await vi.advanceTimersByTimeAsync(0)
    expect(done).toBe(true)
  })

  it('on macOS and Linux the bound still applies', async () => {
    for (const platform of ['macos', 'linux'] as const) {
      let done = false
      void waitForFirstPaint(platform).then(() => (done = true))
      await vi.advanceTimersByTimeAsync(FIRST_PAINT_TIMEOUT_MS)
      expect(done).toBe(true)
    }
  })

  it('keeps the timeout short enough that the window shows well before the splash watchdog', () => {
    expect(FIRST_PAINT_TIMEOUT_MS).toBeGreaterThanOrEqual(100)
    expect(FIRST_PAINT_TIMEOUT_MS).toBeLessThanOrEqual(500)
  })

  it('on macOS and Linux a painting engine still wins before the bound', async () => {
    let done = false
    void waitForFirstPaint('macos').then(() => (done = true))

    // A painting engine delivers both frames well inside the timeout, so the
    // timeout never decides the handover there.
    await vi.advanceTimersByTimeAsync(32)
    flushFrame()
    flushFrame()
    await vi.advanceTimersByTimeAsync(0)
    expect(done).toBe(true)
  })
})
