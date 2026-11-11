import { cleanup, render } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import EntropicConstellation from './EntropicConstellation.svelte'
import { stepMotionPoint } from './constellation-motion'

const minimizedWatchers: Array<(minimized: boolean) => void> = []
const stopWatchingMinimized = vi.fn()

vi.mock('$lib/window-minimized', () => ({
  watchWindowMinimized: vi.fn((onChange: (minimized: boolean) => void) => {
    minimizedWatchers.push(onChange)
    return stopWatchingMinimized
  }),
}))

vi.mock('./constellation-motion', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./constellation-motion')>()
  return { ...actual, stepMotionPoint: vi.fn(actual.stepMotionPoint) }
})

/** Runs the most recently scheduled frame callback at `timestamp` ms. */
function runScheduledFrame(timestamp: number) {
  const calls = vi.mocked(window.requestAnimationFrame).mock.calls
  const callback = calls[calls.length - 1]?.[0]
  if (!callback) throw new Error('no frame scheduled')
  callback(timestamp)
}

/** Runs `count` frames spaced like a 60 Hz display, starting at `from` ms. */
function runFramesAt60Hz(count: number, from = 0) {
  for (let index = 0; index < count; index++) runScheduledFrame(from + index * (1000 / 60))
}

/**
 * Behavior tests for the animated-on-Inicio-only constellation (home-view.md
 * T6). happy-dom has no real canvas, so `getContext` returns a stubbed 2D
 * context and the frame scheduler is stubbed globally — the same approach the
 * frozen `EntropicConstellation.test.ts` uses for `getContext`/`matchMedia`.
 * These tests never inspect `EntropicConstellation.svelte`'s source text, so
 * they cannot collide with that file's frozen assertions.
 */

function mockContext() {
  return {
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 0,
    createLinearGradient: vi.fn(() => ({ addColorStop: vi.fn() })),
    createRadialGradient: vi.fn(() => ({ addColorStop: vi.fn() })),
    fillRect: vi.fn(),
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    arc: vi.fn(),
    fill: vi.fn(),
    stroke: vi.fn(),
    setTransform: vi.fn(),
  }
}

function stubMatchMedia(reducedMotion: boolean) {
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: reducedMotion }))
  )
}

describe('EntropicConstellation animated mode (Inicio only)', () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(
      mockContext() as unknown as CanvasRenderingContext2D
    )
    vi.stubGlobal(
      'requestAnimationFrame',
      vi.fn(() => 1)
    )
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
    stubMatchMedia(false)
  })

  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
    vi.mocked(stepMotionPoint).mockClear()
    stopWatchingMinimized.mockClear()
    minimizedWatchers.length = 0
    delete document.documentElement.dataset.theme
  })

  it('never schedules a frame when animated is false (default)', () => {
    render(EntropicConstellation, { animated: false })
    expect(window.requestAnimationFrame).not.toHaveBeenCalled()
  })

  it('never schedules a frame when animated is omitted', () => {
    render(EntropicConstellation)
    expect(window.requestAnimationFrame).not.toHaveBeenCalled()
  })

  it('starts a rAF loop when animated is true', () => {
    render(EntropicConstellation, { animated: true })
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(1)
  })

  it('cancels the loop when animated toggles from true to false', async () => {
    const { rerender } = render(EntropicConstellation, { animated: true })
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(1)

    await rerender({ animated: false })

    expect(window.cancelAnimationFrame).toHaveBeenCalledTimes(1)
  })

  it('restarts the loop when animated toggles back from false to true', async () => {
    const { rerender } = render(EntropicConstellation, { animated: true })
    await rerender({ animated: false })
    await rerender({ animated: true })

    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(2)
  })

  it('draws one frame and schedules no loop under prefers-reduced-motion', () => {
    stubMatchMedia(true)
    render(EntropicConstellation, { animated: true })

    expect(window.requestAnimationFrame).not.toHaveBeenCalled()
  })

  it('pauses the loop while the document is hidden and resumes when visible', () => {
    render(EntropicConstellation, { animated: true })
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(1)

    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true })
    document.dispatchEvent(new Event('visibilitychange'))
    expect(window.cancelAnimationFrame).toHaveBeenCalledTimes(1)

    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false })
    document.dispatchEvent(new Event('visibilitychange'))
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(2)
  })

  it('does not schedule an extra frame on a hidden visibilitychange while already stopped', async () => {
    const { rerender } = render(EntropicConstellation, { animated: true })
    await rerender({ animated: false })
    vi.mocked(window.requestAnimationFrame).mockClear()

    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false })
    document.dispatchEvent(new Event('visibilitychange'))

    expect(window.requestAnimationFrame).not.toHaveBeenCalled()
  })

  it('cleans up the loop and listeners on destroy', () => {
    const removeEventListenerSpy = vi.spyOn(window, 'removeEventListener')
    const { unmount } = render(EntropicConstellation, { animated: true })

    unmount()

    expect(window.cancelAnimationFrame).toHaveBeenCalled()
    expect(removeEventListenerSpy).toHaveBeenCalledWith('pointermove', expect.any(Function))
    expect(stopWatchingMinimized).toHaveBeenCalledTimes(1)
  })

  // Measured on the Store build 1.6.0: minimizing the window does not mark the
  // WebView2 document hidden, so the loop kept running at ~1.2 cores.
  it('pauses the loop while the window is minimized and resumes on restore', () => {
    render(EntropicConstellation, { animated: true })
    expect(minimizedWatchers).toHaveLength(1)
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(1)

    minimizedWatchers[0]!(true)
    expect(window.cancelAnimationFrame).toHaveBeenCalledTimes(1)

    minimizedWatchers[0]!(false)
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(2)
  })

  it('stays paused on a visible visibilitychange while the window is minimized', () => {
    render(EntropicConstellation, { animated: true })
    minimizedWatchers[0]!(true)
    vi.mocked(window.requestAnimationFrame).mockClear()

    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false })
    document.dispatchEvent(new Event('visibilitychange'))

    expect(window.requestAnimationFrame).not.toHaveBeenCalled()
  })

  it('stops watching the window when the animated mode turns off', async () => {
    const { rerender } = render(EntropicConstellation, { animated: true })
    await rerender({ animated: false })

    expect(stopWatchingMinimized).toHaveBeenCalledTimes(1)
  })

  it('reads the theme colors once, not on every frame', () => {
    const computedStyle = vi.spyOn(window, 'getComputedStyle')
    render(EntropicConstellation, { animated: true })
    runFramesAt60Hz(2)
    const afterWarmUp = computedStyle.mock.calls.length

    runFramesAt60Hz(60, 1000)

    expect(computedStyle.mock.calls.length).toBe(afterWarmUp)
  })

  it('reads the theme colors again after the theme changes', async () => {
    const computedStyle = vi.spyOn(window, 'getComputedStyle')
    render(EntropicConstellation, { animated: true })
    runFramesAt60Hz(2)
    const beforeThemeChange = computedStyle.mock.calls.length

    document.documentElement.dataset.theme = 'light'
    let clock = 2000
    await vi.waitFor(() => {
      runFramesAt60Hz(2, clock)
      clock += 1000
      expect(computedStyle.mock.calls.length).toBeGreaterThan(beforeThemeChange)
    })
  })

  it('draws at most 30 frames per second on a 60 Hz display', () => {
    const context = mockContext()
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(
      context as unknown as CanvasRenderingContext2D
    )
    render(EntropicConstellation, { animated: true })
    context.fillRect.mockClear()

    runFramesAt60Hz(60)

    // Every drawn frame paints the two background layers.
    const drawnFrames = context.fillRect.mock.calls.length / 2
    expect(drawnFrames).toBeGreaterThan(0)
    expect(drawnFrames).toBeLessThanOrEqual(31)
  })

  it('keeps the drift speed of the original 60 Hz loop while drawing fewer frames', () => {
    render(EntropicConstellation, { animated: true })
    vi.mocked(stepMotionPoint).mockClear()

    runFramesAt60Hz(61)

    // One physics step per 60 Hz frame for each of the 60 points: one second.
    const steps = vi.mocked(stepMotionPoint).mock.calls.length
    expect(steps).toBeGreaterThanOrEqual(59 * 60)
    expect(steps).toBeLessThanOrEqual(61 * 60)
  })
})
