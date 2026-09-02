import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import TopBar from './TopBar.svelte'
import { locale } from '$lib/i18n'
import { resetZoom, ZOOM_MAX, ZOOM_MIN, zoomIn, zoomOut } from '$lib/zoom'
import type { View } from '$lib/navigation'

type NavigationSnapshot = {
  history: View[]
  current: View
  canGoBack: boolean
  breadcrumb: string[]
}

const { navigationStore, setZoomMock, storeRef } = vi.hoisted(() => {
  const current: NavigationSnapshot = {
    history: [{ name: 'collections' as const }],
    current: { name: 'collections' as const },
    canGoBack: false,
    breadcrumb: ['Collections'],
  }

  return {
    navigationStore: {
      subscribe(run: (value: NavigationSnapshot) => void) {
        run(current)
        return () => {}
      },
    },
    setZoomMock: vi.fn(),
    storeRef: {
      current: {
        items: {
          searchGlobal: vi.fn().mockResolvedValue([]),
          findByCollection: vi.fn().mockResolvedValue([]),
          findPreviousCardSummary: vi.fn().mockResolvedValue(null),
          findNextCardSummary: vi.fn().mockResolvedValue(null),
        },
        collections: { findById: vi.fn().mockResolvedValue(null) },
        assets: { findByItem: vi.fn().mockResolvedValue([]), deleteWithCascade: vi.fn() },
      },
    },
  }
})

vi.mock('$lib/navigation', () => ({
  navigation: {
    subscribe: navigationStore.subscribe,
    navigate: vi.fn(),
    replace: vi.fn(),
    resetToPath: vi.fn(),
    openRootSection: vi.fn(),
    back: vi.fn(),
  },
}))

vi.mock('$lib/db', () => ({ getStore: () => storeRef.current }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@tauri-apps/plugin-fs', () => ({ remove: vi.fn() }))
vi.mock('$lib/file-import', () => ({
  deleteAssetFile: vi.fn(),
  deleteImageThumbnail: vi.fn(),
  deletePdfThumbnail: vi.fn(),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ minimize: vi.fn(), toggleMaximize: vi.fn(), close: vi.fn() }),
}))
// The zoom module itself is real here — the point of these tests is that the
// menu stays in step with it, including changes it never triggered.
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ setZoom: setZoomMock }),
}))

const trigger = () => screen.getByRole('button', { name: 'Zoom' })
const decrease = () => screen.getByRole('button', { name: 'Reducir zoom' })
const increase = () => screen.getByRole('button', { name: 'Aumentar zoom' })
const reset = () => screen.getByRole('button', { name: 'Restablecer zoom' })
const level = () => screen.getByTestId('topbar-zoom-level')

describe('TopBar zoom control', () => {
  beforeEach(async () => {
    locale.set('es')
    localStorage.clear()
    vi.clearAllMocks()
    setZoomMock.mockResolvedValue(undefined)
    await resetZoom()
  })

  it('keeps the menu closed until the trigger is used', () => {
    render(TopBar)
    expect(screen.queryByRole('group', { name: 'Zoom' })).not.toBeInTheDocument()
  })

  it('opens a menu showing the current level', async () => {
    render(TopBar)

    await fireEvent.click(trigger())

    expect(screen.getByRole('group', { name: 'Zoom' })).toBeInTheDocument()
    expect(level()).toHaveTextContent('100%')
  })

  it('steps up and down by 5%', async () => {
    render(TopBar)
    await fireEvent.click(trigger())

    await fireEvent.click(increase())
    await waitFor(() => expect(level()).toHaveTextContent('105%'))

    await fireEvent.click(decrease())
    await fireEvent.click(decrease())
    await waitFor(() => expect(level()).toHaveTextContent('95%'))
  })

  it('restores 100% from the reset action', async () => {
    render(TopBar)
    await fireEvent.click(trigger())

    await fireEvent.click(increase())
    await waitFor(() => expect(level()).toHaveTextContent('105%'))

    await fireEvent.click(reset())
    await waitFor(() => expect(level()).toHaveTextContent('100%'))
  })

  it('follows a zoom change it did not trigger, such as the keyboard shortcut', async () => {
    render(TopBar)
    await fireEvent.click(trigger())

    await zoomIn()
    await zoomIn()

    await waitFor(() => expect(level()).toHaveTextContent('110%'))
  })

  it('disables the step that would leave the allowed range', async () => {
    render(TopBar)
    await fireEvent.click(trigger())

    while ((await zoomIn()) < ZOOM_MAX);
    await waitFor(() => expect(increase()).toBeDisabled())
    expect(decrease()).toBeEnabled()

    while ((await zoomOut()) > ZOOM_MIN);
    await waitFor(() => expect(decrease()).toBeDisabled())
    expect(increase()).toBeEnabled()
  })

  it('closes the menu when focus leaves it', async () => {
    render(TopBar)
    await fireEvent.click(trigger())
    const container = screen.getByTestId('topbar-zoom')
    expect(screen.getByRole('group', { name: 'Zoom' })).toBeInTheDocument()

    await fireEvent.focusOut(container, { relatedTarget: document.body })

    await waitFor(() =>
      expect(screen.queryByRole('group', { name: 'Zoom' })).not.toBeInTheDocument(),
    )
  })
})
