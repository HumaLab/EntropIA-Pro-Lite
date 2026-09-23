import { cleanup, render } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import EntropicConstellation from './EntropicConstellation.svelte'

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
  })
})
