import { fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App.svelte'

const { initDbMock, initLocaleMock, setupKeyboardShortcutsMock, cleanupKeyboardMock, navigationStore } =
  vi.hoisted(() => ({
    initDbMock: vi.fn<() => Promise<void>>(),
    initLocaleMock: vi.fn<() => Promise<void>>(),
    setupKeyboardShortcutsMock: vi.fn(),
    cleanupKeyboardMock: vi.fn(),
    navigationStore: {
      subscribe(run: (value: unknown) => void) {
        run({
          history: [{ name: 'collections' }],
          current: { name: 'collections' },
          canGoBack: false,
          breadcrumb: ['Collections'],
        })
        return () => {}
      },
    },
  }))

vi.mock('@tauri-apps/api/core', () => ({
  // Pro's AppShell probes the local deps/runtime subsystem on mount once the app
  // is ready. Resolve those bridge calls so the startup test stays isolated.
  invoke: vi.fn((command: string) => {
    if (command === 'deps_get_cached_statuses') return Promise.resolve([])
    if (command === 'runtime_get_status') {
      return Promise.resolve({
        state: 'healthy',
        packVersion: null,
        repairNeeded: false,
        repairAvailable: false,
        summary: 'Runtime listo',
        blockedCapabilities: [],
        details: [],
        guidance: [],
        bootstrapEligible: false,
        bootstrapRequired: false,
        activeOperation: null,
      })
    }
    return Promise.resolve(undefined)
  }),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(vi.fn())),
}))

vi.mock('$lib/db', () => ({
  initDb: initDbMock,
}))

vi.mock('$lib/i18n', async () => {
  const actual = await vi.importActual<typeof import('$lib/i18n')>('$lib/i18n')
  return {
    ...actual,
    initLocale: initLocaleMock,
  }
})

vi.mock('$lib/keyboard', () => ({
  setupKeyboardShortcuts: setupKeyboardShortcutsMock,
}))

vi.mock('$lib/navigation', () => ({
  navigation: {
    subscribe: navigationStore.subscribe,
  },
}))

describe('App startup', () => {
  beforeEach(() => {
    initDbMock.mockReset().mockResolvedValue(undefined)
    initLocaleMock.mockReset().mockResolvedValue(undefined)
    cleanupKeyboardMock.mockReset()
    setupKeyboardShortcutsMock.mockReset().mockReturnValue(cleanupKeyboardMock)
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    delete document.documentElement.dataset.platform
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('shows a recoverable startup error and retries initialization without duplicate keyboard setup', async () => {
    let resolveRetry: (() => void) | undefined
    initDbMock
      .mockRejectedValueOnce(new Error('database unavailable'))
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            resolveRetry = resolve
          }),
      )

    render(App)

    expect(screen.getByRole('status')).toHaveTextContent('Inicializando...')
    expect(await screen.findByRole('alert')).toHaveTextContent('database unavailable')

    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))

    expect(screen.getByRole('status')).toHaveTextContent('Inicializando...')
    expect(initLocaleMock).toHaveBeenCalledTimes(2)
    expect(initDbMock).toHaveBeenCalledTimes(2)
    expect(setupKeyboardShortcutsMock).toHaveBeenCalledTimes(1)

    resolveRetry?.()
  })

  it('marks the document root with the detected desktop platform', async () => {
    vi.spyOn(window.navigator, 'platform', 'get').mockReturnValue('Linux x86_64')

    render(App)

    expect(document.documentElement.dataset.platform).toBe('linux')
  })
})
