import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { setupKeyboardShortcuts, registerEscapeInterceptor } from './keyboard'
import { zoomIn, zoomOut, resetZoom } from './zoom'

// Escape-to-back acts on the active pane's own navigation (fix round 1:
// the retired `./navigation` singleton is disconnected from what the app
// actually renders once a view is pane-scoped), so we mock `./workspace`
// and spy on `workspace.activeNavigation.back()`.
vi.mock('./workspace', () => {
  const activeNavigation = {
    back: vi.fn(),
    current: { name: 'collections' as const },
    canGoBack: false,
    breadcrumb: ['Collections'],
    navigate: vi.fn(),
  }
  return {
    workspace: { activeNavigation },
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

  it('calls workspace.activeNavigation.back() on Escape key', async () => {
    const { workspace } = await import('./workspace')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(workspace.activeNavigation.back).toHaveBeenCalledOnce()
  })

  it('does not call back on other keys', async () => {
    const { workspace } = await import('./workspace')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }))
    expect(workspace.activeNavigation.back).not.toHaveBeenCalled()
  })

  it('removes listener on cleanup', async () => {
    const { workspace } = await import('./workspace')
    cleanup()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(workspace.activeNavigation.back).not.toHaveBeenCalled()
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
    const { workspace } = await import('./workspace')
    const interceptor = vi.fn().mockReturnValue(true)
    const unregister = registerEscapeInterceptor(interceptor)

    pressEscape()

    expect(interceptor).toHaveBeenCalledOnce()
    expect(workspace.activeNavigation.back).not.toHaveBeenCalled()
    unregister()
  })

  it('falls through to back-navigation when no interceptor consumes Escape', async () => {
    const { workspace } = await import('./workspace')
    const interceptor = vi.fn().mockReturnValue(false)
    const unregister = registerEscapeInterceptor(interceptor)

    pressEscape()

    expect(interceptor).toHaveBeenCalledOnce()
    expect(workspace.activeNavigation.back).toHaveBeenCalledOnce()
    unregister()
  })

  it('runs interceptors most-recently-registered first and stops at the first consumer', async () => {
    const { workspace } = await import('./workspace')
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
    expect(workspace.activeNavigation.back).not.toHaveBeenCalled()
    unregisterFirst()
    unregisterSecond()
  })

  it('restores back-navigation after an interceptor unregisters', async () => {
    const { workspace } = await import('./workspace')
    const unregister = registerEscapeInterceptor(() => true)

    unregister()
    pressEscape()

    expect(workspace.activeNavigation.back).toHaveBeenCalledOnce()
  })

  it('does not run interceptors when the Escape is ignored (e.g. typed in an input)', async () => {
    const { workspace } = await import('./workspace')
    const interceptor = vi.fn().mockReturnValue(true)
    const unregister = registerEscapeInterceptor(interceptor)

    const input = document.createElement('input')
    document.body.appendChild(input)
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))

    expect(interceptor).not.toHaveBeenCalled()
    expect(workspace.activeNavigation.back).not.toHaveBeenCalled()
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
      new KeyboardEvent('keydown', { key: '=', ctrlKey: true, bubbles: true, cancelable: true })
    )
    expect(zoomIn).toHaveBeenCalledOnce()
    input.remove()
  })

  it('leaves back-navigation alone', async () => {
    const { workspace } = await import('./workspace')
    press('=', { ctrlKey: true })
    expect(workspace.activeNavigation.back).not.toHaveBeenCalled()
  })

  it('stops after cleanup', () => {
    cleanup()
    press('=', { ctrlKey: true })
    expect(zoomIn).not.toHaveBeenCalled()
  })
})
