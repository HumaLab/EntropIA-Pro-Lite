import { render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import TopBar from './TopBar.svelte'
import { locale } from '$lib/i18n'
import type { View } from '$lib/navigation'

type NavigationSnapshot = {
  history: View[]
  current: View
  canGoBack: boolean
  breadcrumb: string[]
}

const { navigationStore, setNavigationState, replaceMock, storeRef } = vi.hoisted(() => {
  let current: NavigationSnapshot = {
    history: [{ name: 'collections' as const }],
    current: { name: 'collections' as const },
    canGoBack: false,
    breadcrumb: ['Collections'],
  }
  const subscribers = new Set<(value: NavigationSnapshot) => void>()

  return {
    navigationStore: {
      subscribe(run: (value: NavigationSnapshot) => void) {
        subscribers.add(run)
        run(current)
        return () => subscribers.delete(run)
      },
    },
    setNavigationState(value: NavigationSnapshot) {
      current = value
      subscribers.forEach((run) => run(current))
    },
    replaceMock: vi.fn(),
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
    replace: replaceMock,
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

const collectionId = 'col-1'

function summary(id: string, title: string) {
  return {
    id,
    title,
    collectionId,
    metadata: null,
    createdAt: 0,
    updatedAt: 0,
    assetCount: 0,
    primaryAssetId: null,
    primaryAssetPath: null,
    primaryAssetType: null,
  }
}

function openItem(itemId: string, itemTitle: string) {
  setNavigationState({
    history: [
      { name: 'collections' },
      { name: 'collection', id: collectionId, collectionName: 'Archivo' },
    ],
    current: { name: 'item', collectionId, collectionName: 'Archivo', itemId, itemTitle },
    canGoBack: true,
    breadcrumb: ['Collections', 'Archivo', itemTitle],
  })
}

const previousButton = () => screen.getByRole('button', { name: 'Documento anterior' })
const nextButton = () => screen.getByRole('button', { name: 'Documento siguiente' })

describe('TopBar sibling navigation', () => {
  beforeEach(() => {
    locale.set('es')
    localStorage.clear()
    vi.clearAllMocks()
    storeRef.current.items.searchGlobal.mockResolvedValue([])
    storeRef.current.items.findByCollection.mockResolvedValue([])
    storeRef.current.collections.findById.mockResolvedValue(null)
    storeRef.current.assets.findByItem.mockResolvedValue([])
  })

  it('uses two single-row keyset queries instead of loading the whole collection', async () => {
    storeRef.current.items.findPreviousCardSummary.mockResolvedValue(summary('doc-09', 'Luna'))
    storeRef.current.items.findNextCardSummary.mockResolvedValue(summary('doc-11', 'Nimbus'))
    render(TopBar)

    openItem('doc-10', 'Mosaic')

    await waitFor(() =>
      expect(storeRef.current.items.findPreviousCardSummary).toHaveBeenCalledWith(collectionId, {
        title: 'Mosaic',
        id: 'doc-10',
      })
    )
    expect(storeRef.current.items.findNextCardSummary).toHaveBeenCalledWith(collectionId, {
      title: 'Mosaic',
      id: 'doc-10',
    })
    // The whole point of this unit: the collection is never walked.
    expect(storeRef.current.items.findByCollection).not.toHaveBeenCalled()

    await waitFor(() => expect(previousButton()).toBeEnabled())
    expect(nextButton()).toBeEnabled()
  })

  it('navigates to the sibling the keyset query returned', async () => {
    storeRef.current.items.findPreviousCardSummary.mockResolvedValue(summary('doc-09', 'Luna'))
    storeRef.current.items.findNextCardSummary.mockResolvedValue(summary('doc-11', 'Nimbus'))
    render(TopBar)
    openItem('doc-10', 'Mosaic')
    await waitFor(() => expect(nextButton()).toBeEnabled())

    nextButton().click()

    expect(replaceMock).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'item', itemId: 'doc-11', itemTitle: 'Nimbus' })
    )
  })

  it('disables the edge control that has no sibling without reloading the collection', async () => {
    storeRef.current.items.findPreviousCardSummary.mockResolvedValue(null)
    storeRef.current.items.findNextCardSummary.mockResolvedValue(summary('doc-02', 'Beta'))
    render(TopBar)

    openItem('doc-01', 'Alpha')

    await waitFor(() => expect(nextButton()).toBeEnabled())
    expect(previousButton()).toBeDisabled()
    expect(storeRef.current.items.findByCollection).not.toHaveBeenCalled()
  })

  it('discards a stale sibling response after the document changes', async () => {
    let releaseFirst: ((value: unknown) => void) | undefined
    storeRef.current.items.findPreviousCardSummary
      .mockImplementationOnce(() => new Promise((resolve) => (releaseFirst = resolve)))
      .mockResolvedValue(summary('doc-20', 'Tango'))
    storeRef.current.items.findNextCardSummary.mockResolvedValue(null)
    render(TopBar)

    openItem('doc-10', 'Mosaic')
    openItem('doc-21', 'Ubaldo')
    releaseFirst?.(summary('doc-09', 'Luna'))

    await waitFor(() => expect(previousButton()).toBeEnabled())
    previousButton().click()

    // Luna belonged to the document the user already left.
    expect(replaceMock).toHaveBeenCalledWith(
      expect.objectContaining({ itemId: 'doc-20', itemTitle: 'Tango' })
    )
  })

  it('falls back to the whole-collection loader only when the store has no keyset queries', async () => {
    const legacyItems = storeRef.current.items as unknown as Record<string, unknown>
    const previous = legacyItems.findPreviousCardSummary
    const next = legacyItems.findNextCardSummary
    delete legacyItems.findPreviousCardSummary
    delete legacyItems.findNextCardSummary
    storeRef.current.items.findByCollection.mockResolvedValue([
      { id: 'doc-09', title: 'Luna', collectionId },
      { id: 'doc-10', title: 'Mosaic', collectionId },
      { id: 'doc-11', title: 'Nimbus', collectionId },
    ])

    try {
      render(TopBar)
      openItem('doc-10', 'Mosaic')

      await waitFor(() => expect(storeRef.current.items.findByCollection).toHaveBeenCalled())
      await waitFor(() => expect(previousButton()).toBeEnabled())
      expect(nextButton()).toBeEnabled()
    } finally {
      legacyItems.findPreviousCardSummary = previous
      legacyItems.findNextCardSummary = next
    }
  })
})
