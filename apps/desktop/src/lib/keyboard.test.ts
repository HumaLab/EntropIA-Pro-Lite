import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { setupKeyboardShortcuts, registerEscapeInterceptor } from './keyboard'
import { zoomIn, zoomOut, resetZoom } from './zoom'

// We mock the navigation module so we can spy on .back()
vi.mock('./navigation', () => {
  const store = {
    back: vi.fn(),
    current: { name: 'collections' as const },
    canGoBack: false,
    breadcrumb: ['Collections'],
    navigate: vi.fn(),
  }
  return {
    navigation: store,
    NavigationStore: vi.fn(),
  }
})

// Zoom talks to the Tauri webview; the shortcut tests only care that the right
// action fires.
vi.mock('./zoom', () => ({
  zoomIn: vi.fn().mockResolvedValue(1.05),
  zoomOut: vi.fn().mockResolvedValue(0.95),
  resetZoom: vi.fn().mockResolvedValue(1),
}))

describe('setupKeyboardShortcuts', () => {
  let cleanup: () => void

  beforeEach(() => {
    vi.clearAllMocks()
    cleanup = setupKeyboardShortcuts()
  })

  afterEach(() => {
    cleanup()
  })

  it('calls navigation.back() on Escape key', async () => {
    const { navigation } = await import('./navigation')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(navigation.back).toHaveBeenCalledOnce()
  })

  it('does not call back on other keys', async () => {
    const { navigation } = await import('./navigation')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }))
    expect(navigation.back).not.toHaveBeenCalled()
  })

  it('removes listener on cleanup', async () => {
    const { navigation } = await import('./navigation')
    cleanup()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(navigation.back).not.toHaveBeenCalled()
  })
})

describe('registerEscapeInterceptor', () => {
  let cleanup: () => void

  beforeEach(() => {
    vi.clearAllMocks()
    cleanup = setupKeyboardShortcuts()
  })

  afterEach(() => {
    cleanup()
  })

  function pressEscape() {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
  }

  it('skips back-navigation when an interceptor consumes Escape', async () => {
    const { navigation } = await import('./navigation')
    const interceptor = vi.fn().mockReturnValue(true)
    const unregister = registerEscapeInterceptor(interceptor)

    pressEscape()

    expect(interceptor).toHaveBeenCalledOnce()
    expect(navigation.back).not.toHaveBeenCalled()
    unregister()
  })

  it('falls through to back-navigation when no interceptor consumes Escape', async () => {
    const { navigation } = await import('./navigation')
    const interceptor = vi.fn().mockReturnValue(false)
    const unregister = registerEscapeInterceptor(interceptor)

    pressEscape()

    expect(interceptor).toHaveBeenCalledOnce()
    expect(navigation.back).toHaveBeenCalledOnce()
    unregister()
  })

  it('runs interceptors most-recently-registered first and stops at the first consumer', async () => {
    const { navigation } = await import('./navigation')
    const calls: string[] = []
    const unregisterFirst = registerEscapeInterceptor(() => {
      calls.push('first')
      return true
    })
    const unregisterSecond = registerEscapeInterceptor(() => {
      calls.push('second')
      return true
    })

    pressEscape()

    expect(calls).toEqual(['second'])
    expect(navigation.back).not.toHaveBeenCalled()
    unregisterFirst()
    unregisterSecond()
  })

  it('restores back-navigation after an interceptor unregisters', async () => {
    const { navigation } = await import('./navigation')
    const unregister = registerEscapeInterceptor(() => true)

    unregister()
    pressEscape()

    expect(navigation.back).toHaveBeenCalledOnce()
  })

  it('does not run interceptors when the Escape is ignored (e.g. typed in an input)', async () => {
    const { navigation } = await import('./navigation')
    const interceptor = vi.fn().mockReturnValue(true)
    const unregister = registerEscapeInterceptor(interceptor)

    const input = document.createElement('input')
    document.body.appendChild(input)
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))

    expect(interceptor).not.toHaveBeenCalled()
    expect(navigation.back).not.toHaveBeenCalled()
    input.remove()
    unregister()
  })
})

describe('zoom shortcuts', () => {
  let cleanup: () => void

  beforeEach(() => {
    vi.clearAllMocks()
    cleanup = setupKeyboardShortcuts()
  })

  afterEach(() => {
    cleanup()
  })

  function press(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
    const event = new KeyboardEvent('keydown', { key, cancelable: true, ...init })
    window.dispatchEvent(event)
    return event
  }

  it('zooms in on Ctrl and the plus family of keys', () => {
    for (const key of ['=', '+', 'Add']) {
      press(key, { ctrlKey: true })
    }
    expect(zoomIn).toHaveBeenCalledTimes(3)
  })

  it('zooms out on Ctrl and the minus family of keys', () => {
    for (const key of ['-', '_', 'Subtract']) {
      press(key, { ctrlKey: true })
    }
    expect(zoomOut).toHaveBeenCalledTimes(3)
  })

  it('resets on Ctrl+0', () => {
    press('0', { ctrlKey: true })
    expect(resetZoom).toHaveBeenCalledOnce()
  })

  it('accepts Cmd on macOS', () => {
    press('=', { metaKey: true })
    expect(zoomIn).toHaveBeenCalledOnce()
  })

  it('prevents the default so the webview does not zoom a second time', () => {
    const event = press('-', { ctrlKey: true })
    expect(event.defaultPrevented).toBe(true)
  })

  it('ignores the keys without a modifier', () => {
    press('=')
    press('-')
    press('0')
    expect(zoomIn).not.toHaveBeenCalled()
    expect(zoomOut).not.toHaveBeenCalled()
    expect(resetZoom).not.toHaveBeenCalled()
  })

  it('ignores Alt combinations so it does not steal other chords', () => {
    press('=', { ctrlKey: true, altKey: true })
    expect(zoomIn).not.toHaveBeenCalled()
  })

  it('works while typing, the way browser zoom does', () => {
    const input = document.createElement('input')
    document.body.appendChild(input)
    input.dispatchEvent(
      new KeyboardEvent('keydown', { key: '=', ctrlKey: true, bubbles: true, cancelable: true }),
    )
    expect(zoomIn).toHaveBeenCalledOnce()
    input.remove()
  })

  it('leaves back-navigation alone', async () => {
    const { navigation } = await import('./navigation')
    press('=', { ctrlKey: true })
    expect(navigation.back).not.toHaveBeenCalled()
  })

  it('stops after cleanup', () => {
    cleanup()
    press('=', { ctrlKey: true })
    expect(zoomIn).not.toHaveBeenCalled()
  })
})
