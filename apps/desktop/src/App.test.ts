import { fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App.svelte'
import LazyRouteStub from './test/LazyRouteStub.svelte'
import { workspace } from '$lib/workspace'
import type { View } from '$lib/navigation'

const {
  initDbMock,
  initLocaleMock,
  setupKeyboardShortcutsMock,
  cleanupKeyboardMock,
  loadRouteViewMock,
  storeCheckMock,
  openExternalMock,
  storeRef,
} = vi.hoisted(() => {
  return {
    initDbMock: vi.fn<() => Promise<void>>(),
    initLocaleMock: vi.fn<() => Promise<void>>(),
    setupKeyboardShortcutsMock: vi.fn(),
    cleanupKeyboardMock: vi.fn(),
    loadRouteViewMock: vi.fn(),
    storeCheckMock: vi.fn<() => Promise<string>>(),
    openExternalMock: vi.fn<(args: unknown) => Promise<void>>(),
    // AppShell's TopBar and the sidebar explorer both read the store as soon as
    // they mount; without it their effects reject and Vitest flags the run.
    storeRef: {
      current: {
        collections: {
          findAll: vi.fn().mockResolvedValue([]),
          countItems: vi.fn().mockResolvedValue(0),
          findById: vi.fn().mockResolvedValue(null),
        },
        assets: { findByItem: vi.fn().mockResolvedValue([]) },
        items: {
          searchGlobal: vi.fn().mockResolvedValue([]),
          findByCollection: vi.fn().mockResolvedValue([]),
          findPreviousCardSummary: vi.fn().mockResolvedValue(null),
          findNextCardSummary: vi.fn().mockResolvedValue(null),
        },
      },
    },
  }
})

/**
 * App.svelte now derives its view from `workspace.activeNavigation` (the
 * real `NavigationStore` Stage 1's single tab owns), not the retired
 * `$lib/navigation` singleton — see workspace.ts and pane-context.ts. Every
 * scenario below drives that real store directly instead of a mock double.
 * `$lib/navigation`'s own singleton is left unmocked: AppShell.svelte still
 * reads it for chrome-only state (unmigrated until Tasks 1.5/1.6), and no
 * assertion here depends on it, so it needs no test double.
 */
function navigateActiveTo(view: View): void {
  workspace.activeNavigation.resetToPath([view])
}

vi.mock('@tauri-apps/api/core', () => ({
  // Pro's AppShell probes the local deps/runtime subsystem on mount once the app
  // is ready. Resolve those bridge calls so the startup test stays isolated.
  invoke: vi.fn((command: string, args?: unknown) => {
    if (command === 'deps_get_cached_statuses') return Promise.resolve([])
    if (command === 'check_microsoft_store_update') return storeCheckMock()
    if (command === 'open_external_url') return openExternalMock(args)
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
  getStore: () => storeRef.current,
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

vi.mock('$lib/route-loader', () => ({
  loadRouteView: loadRouteViewMock,
}))

beforeEach(() => {
  initDbMock.mockReset().mockResolvedValue(undefined)
  initLocaleMock.mockReset().mockResolvedValue(undefined)
  cleanupKeyboardMock.mockReset()
  setupKeyboardShortcutsMock.mockReset().mockReturnValue(cleanupKeyboardMock)
  loadRouteViewMock.mockReset()
  storeCheckMock.mockReset().mockResolvedValue('skipped')
  openExternalMock.mockReset().mockResolvedValue(undefined)
  // afterEach restores every mock, which strips these implementations. Re-arm
  // them or the explorer's loader keeps retrying an undefined result.
  storeRef.current.collections.findAll.mockReset().mockResolvedValue([])
  storeRef.current.collections.countItems.mockReset().mockResolvedValue(0)
  storeRef.current.collections.findById.mockReset().mockResolvedValue(null)
  storeRef.current.assets.findByItem.mockReset().mockResolvedValue([])
  storeRef.current.items.searchGlobal.mockReset().mockResolvedValue([])
  storeRef.current.items.findByCollection.mockReset().mockResolvedValue([])
  storeRef.current.items.findPreviousCardSummary.mockReset().mockResolvedValue(null)
  storeRef.current.items.findNextCardSummary.mockReset().mockResolvedValue(null)
  // `workspace` is a module singleton that outlives each test; reset its one
  // Stage-1 tab back to a known screen so a previous test's navigation can't
  // leak into the next one. Stage 1 never opens a second tab, so there is
  // nothing else on the workspace to close here.
  workspace.activeNavigation.resetToPath([{ name: 'home' }])
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
  // The loading text disappears as soon as `ready` flips, but App.svelte's
  // dismissSplash() still has `tick()` + two rAF frames + an async
  // `invoke('splash_finish')` to run after that before it decides whether to
  // call checkStoreUpdate(). Settle that here too: otherwise, under enough
  // scheduler contention (e.g. the full suite running many files at once),
  // that decision can still be pending when this test returns, and it fires
  // during whichever test happens to be running when it finally resolves —
  // that landed as a spurious storeCheckMock call in an unrelated later test.
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  await new Promise((resolve) => setTimeout(resolve, 0))
}

describe('App startup', () => {
  it('shows a recoverable startup error and retries initialization without duplicate keyboard setup', async () => {
    let resolveRetry: (() => void) | undefined
    initDbMock.mockRejectedValueOnce(new Error('database unavailable')).mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          resolveRetry = resolve
        })
    )

    render(App)

    expect(screen.getByRole('status')).toHaveTextContent('Inicializando...')
    expect(await screen.findByRole('alert')).toHaveTextContent('database unavailable')

    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))

    expect(screen.getByRole('status')).toHaveTextContent('Inicializando...')
    expect(initLocaleMock).toHaveBeenCalledTimes(2)
    expect(initDbMock).toHaveBeenCalledTimes(2)
    expect(setupKeyboardShortcutsMock).toHaveBeenCalledTimes(1)

    // Let the retry settle inside this test: an App left mid-initialization
    // finishes its startup during the next one, and its effects then run
    // against that test's fixtures.
    resolveRetry?.()
    await waitForStartupToFinish()
  })

  it('marks the document root with the detected desktop platform', async () => {
    vi.spyOn(window.navigator, 'platform', 'get').mockReturnValue('Linux x86_64')

    render(App)

    expect(document.documentElement.dataset.platform).toBe('linux')
    await waitForStartupToFinish()
  })
})

describe('App lazy routes', () => {
  it('shows a pending state and completes item navigation with the required props', async () => {
    let resolveRoute: ((module: { default: typeof LazyRouteStub }) => void) | undefined
    loadRouteViewMock.mockImplementation(
      () =>
        new Promise<{ default: typeof LazyRouteStub }>((resolve) => {
          resolveRoute = resolve
        })
    )

    render(App)
    await waitForStartupToFinish()
    navigateActiveTo({
      name: 'item',
      itemId: 'item-1',
      collectionId: 'collection-1',
      collectionName: 'Collection',
      itemTitle: 'Item',
    })

    const pending = await screen.findByRole('status')
    expect(pending).toHaveTextContent('Inicializando...')
    // The same mark as the startup screen: a route loading is the app loading,
    // not a card of the page's content.
    expect(pending.querySelector('img.startup-mark')).not.toBeNull()
    resolveRoute?.({ default: LazyRouteStub })

    expect(await screen.findByTestId('lazy-route')).toHaveTextContent('item-1:collection-1')
    expect(loadRouteViewMock).toHaveBeenCalledWith('item')
  })

  it('preserves route drafts when an edit updates only the asset breadcrumb', async () => {
    loadRouteViewMock.mockResolvedValue({ default: LazyRouteStub })
    render(App)
    await waitForStartupToFinish()
    const route: View = {
      name: 'item',
      itemId: 'item-1',
      collectionId: 'collection-1',
      collectionName: 'Collection',
      itemTitle: 'Item',
      assetId: 'asset-1',
      assetLabel: 'page.png',
    }
    navigateActiveTo(route)
    const draft = await screen.findByRole('textbox', { name: 'Route draft' })
    await fireEvent.input(draft, { target: { value: 'Unsaved viewer state' } })

    navigateActiveTo({ ...route, assetLabel: 'page_v2.png' })

    await vi.waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Route draft' })).toHaveValue(
        'Unsaved viewer state'
      )
    })
  })

  it('ignores a route import that fails after navigating elsewhere', async () => {
    let rejectSettings!: (error: Error) => void
    const settingsImport = new Promise<{ default: typeof LazyRouteStub }>((_resolve, reject) => {
      rejectSettings = reject
    })
    loadRouteViewMock.mockImplementation((name: string) =>
      name === 'settings' ? settingsImport : Promise.resolve({ default: LazyRouteStub })
    )
    render(App)
    await waitForStartupToFinish()
    navigateActiveTo({ name: 'settings' })
    expect(await screen.findByRole('status')).toHaveTextContent('Inicializando...')

    navigateActiveTo({
      name: 'item',
      itemId: 'item-2',
      collectionId: 'collection-1',
      collectionName: 'Collection',
      itemTitle: 'Item',
    })
    const draft = await screen.findByRole('textbox', { name: 'Route draft' })
    await fireEvent.input(draft, { target: { value: 'Current document draft' } })
    rejectSettings(new Error('Obsolete settings import failed'))
    await settingsImport.catch(() => undefined)

    await vi.waitFor(() => {
      expect(screen.getByTestId('lazy-route')).toHaveTextContent('item-2:collection-1')
      expect(screen.getByRole('textbox', { name: 'Route draft' })).toHaveValue(
        'Current document draft'
      )
    })
  })

  it('shows a visible import error and retries the same route', async () => {
    loadRouteViewMock
      .mockRejectedValueOnce(new Error('chunk unavailable'))
      .mockResolvedValueOnce({ default: LazyRouteStub })

    render(App)
    await waitForStartupToFinish()
    navigateActiveTo({ name: 'settings' })

    expect(await screen.findByRole('alert')).toHaveTextContent('chunk unavailable')
    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))

    expect(await screen.findByTestId('lazy-route')).toBeInTheDocument()
    expect(loadRouteViewMock).toHaveBeenNthCalledWith(1, 'settings')
    expect(loadRouteViewMock).toHaveBeenNthCalledWith(2, 'settings')
  })

  it('mounts the home view eagerly, like collections, without a lazy route load', async () => {
    render(App)
    await waitForStartupToFinish()

    navigateActiveTo({ name: 'home' })

    expect(await screen.findByRole('heading', { name: 'Espacio de trabajo' })).toBeInTheDocument()
    expect(loadRouteViewMock).not.toHaveBeenCalledWith('home')
  })
})

describe('App Microsoft Store update notice', () => {
  const STORE_URI = 'ms-windows-store://pdp/?ProductId=9N328K9L95JD'

  it('stays hidden unless Store reports an update', async () => {
    storeCheckMock.mockResolvedValue('up_to_date')
    render(App)
    await waitForStartupToFinish()

    await vi.waitFor(() => expect(storeCheckMock).toHaveBeenCalledTimes(1))
    expect(screen.queryByText('Actualización disponible')).not.toBeInTheDocument()
  })

  it('does not ask Store when initialization failed', async () => {
    initDbMock.mockRejectedValue(new Error('database unavailable'))
    render(App)

    expect(await screen.findByRole('alert')).toHaveTextContent('database unavailable')
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(storeCheckMock).not.toHaveBeenCalled()
  })

  it('keeps the app usable while the check is pending or rejected', async () => {
    let rejectCheck!: (error: Error) => void
    storeCheckMock.mockReturnValue(
      new Promise<string>((_resolve, reject) => {
        rejectCheck = reject
      })
    )
    loadRouteViewMock.mockResolvedValue({ default: LazyRouteStub })
    render(App)
    await waitForStartupToFinish()
    await vi.waitFor(() => expect(storeCheckMock).toHaveBeenCalledTimes(1))

    navigateActiveTo({ name: 'settings' })
    expect(await screen.findByTestId('lazy-route')).toBeInTheDocument()

    rejectCheck(new Error('ipc failed'))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(screen.getByTestId('lazy-route')).toBeInTheDocument()
  })

  it('opens the exact Store listing and keeps the notice', async () => {
    storeCheckMock.mockResolvedValue('available')
    render(App)
    await waitForStartupToFinish()

    await fireEvent.click(await screen.findByRole('button', { name: 'Ver actualización' }))

    expect(openExternalMock).toHaveBeenCalledWith({ url: STORE_URI })
    expect(screen.getByText('Actualización disponible')).toBeInTheDocument()
  })

  it('says so next to the action when the listing cannot be opened', async () => {
    storeCheckMock.mockResolvedValue('available')
    openExternalMock.mockRejectedValue(new Error('spawn failed'))
    render(App)
    await waitForStartupToFinish()

    await fireEvent.click(await screen.findByRole('button', { name: 'Ver actualización' }))

    expect(await screen.findByText('No se pudo abrir Microsoft Store.')).toBeInTheDocument()
    expect(screen.getByText('Actualización disponible')).toBeInTheDocument()
  })

  it('stays dismissed across routes, returns in a new session, and asks Store once', async () => {
    storeCheckMock.mockResolvedValue('available')
    loadRouteViewMock.mockResolvedValue({ default: LazyRouteStub })
    const { unmount } = render(App)
    await waitForStartupToFinish()

    const close = await screen.findByRole('button', { name: 'Cerrar aviso de actualización' })
    close.focus()
    await fireEvent.click(close)

    expect(screen.queryByText('Actualización disponible')).not.toBeInTheDocument()
    expect(document.activeElement).toBe(document.querySelector('main.content'))

    navigateActiveTo({ name: 'settings' })
    expect(await screen.findByTestId('lazy-route')).toBeInTheDocument()
    navigateActiveTo({ name: 'collections' })
    await vi.waitFor(() => expect(screen.queryByTestId('lazy-route')).not.toBeInTheDocument())
    expect(screen.queryByText('Actualización disponible')).not.toBeInTheDocument()
    expect(storeCheckMock).toHaveBeenCalledTimes(1)

    unmount()
    render(App)
    await waitForStartupToFinish()
    expect(await screen.findByText('Actualización disponible')).toBeInTheDocument()
  })
})
