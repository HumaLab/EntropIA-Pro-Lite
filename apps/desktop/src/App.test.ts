import { fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App.svelte'
import LazyRouteStub from './test/LazyRouteStub.svelte'

const {
  initDbMock,
  initLocaleMock,
  setupKeyboardShortcutsMock,
  cleanupKeyboardMock,
  navigationStore,
  loadRouteViewMock,
} = vi.hoisted(() => {
  let navigationSubscriber: ((value: unknown) => void) | undefined
  const emit = (current: Record<string, unknown>) => {
    navigationSubscriber?.({
      history: [current],
      current,
      canGoBack: false,
      breadcrumb: ['Collections'],
    })
  }

  return {
    initDbMock: vi.fn<() => Promise<void>>(),
    initLocaleMock: vi.fn<() => Promise<void>>(),
    setupKeyboardShortcutsMock: vi.fn(),
    cleanupKeyboardMock: vi.fn(),
    loadRouteViewMock: vi.fn(),
    navigationStore: {
      subscribe(run: (value: unknown) => void) {
        navigationSubscriber = run
        emit({ name: 'collections' })
        return () => {
          if (navigationSubscriber === run) navigationSubscriber = undefined
        }
      },
      emit,
      reset() {
        navigationSubscriber = undefined
      },
    },
  }
})

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
  registerEscapeInterceptor: vi.fn(() => vi.fn()),
}))

vi.mock('$lib/navigation', () => ({
  navigation: {
    subscribe: navigationStore.subscribe,
  },
}))

vi.mock('$lib/route-loader', () => ({
  loadRouteView: loadRouteViewMock,
}))

beforeEach(() => {
  initDbMock.mockReset().mockResolvedValue(undefined)
  initLocaleMock.mockReset().mockResolvedValue(undefined)
  cleanupKeyboardMock.mockReset()
  setupKeyboardShortcutsMock.mockReset().mockReturnValue(cleanupKeyboardMock)
  loadRouteViewMock.mockReset()
  navigationStore.reset()
  vi.spyOn(console, 'error').mockImplementation(() => undefined)
  delete document.documentElement.dataset.platform
})

afterEach(() => {
  vi.restoreAllMocks()
})

async function waitForStartupToFinish() {
  await vi.waitFor(() => {
    expect(screen.queryByText('Inicializando...')).not.toBeInTheDocument()
  })
}

describe('App startup', () => {
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

describe('App lazy routes', () => {
  it('shows a pending state and completes item navigation with the required props', async () => {
    let resolveRoute:
      | ((module: { default: typeof LazyRouteStub }) => void)
      | undefined
    loadRouteViewMock.mockImplementation(
      () =>
        new Promise<{ default: typeof LazyRouteStub }>((resolve) => {
          resolveRoute = resolve
        }),
    )

    render(App)
    await waitForStartupToFinish()
    navigationStore.emit({
      name: 'item',
      itemId: 'item-1',
      collectionId: 'collection-1',
      collectionName: 'Collection',
      itemTitle: 'Item',
    })

    expect(await screen.findByRole('status')).toHaveTextContent('Inicializando...')
    resolveRoute?.({ default: LazyRouteStub })

    expect(await screen.findByTestId('lazy-route')).toHaveTextContent(
      'item-1:collection-1',
    )
    expect(loadRouteViewMock).toHaveBeenCalledWith('item')
  })

  it('shows a visible import error and retries the same route', async () => {
    loadRouteViewMock
      .mockRejectedValueOnce(new Error('chunk unavailable'))
      .mockResolvedValueOnce({ default: LazyRouteStub })

    render(App)
    await waitForStartupToFinish()
    navigationStore.emit({ name: 'settings' })

    expect(await screen.findByRole('alert')).toHaveTextContent('chunk unavailable')
    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))

    expect(await screen.findByTestId('lazy-route')).toBeInTheDocument()
    expect(loadRouteViewMock).toHaveBeenNthCalledWith(1, 'settings')
    expect(loadRouteViewMock).toHaveBeenNthCalledWith(2, 'settings')
  })
})
