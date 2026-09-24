import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import WorkPane from './WorkPane.svelte'
import { workspace } from '$lib/workspace'
import { locale } from '$lib/i18n'
import type { View } from '$lib/navigation'

// A shared, mutable store double: `getStore()` must return the SAME object
// on every call so a test can pre-configure a resolved value before
// rendering, the way `$lib/db`'s real store singleton behaves.
const { storeRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      items: { findPreviousCardSummary: vi.fn(), findNextCardSummary: vi.fn() },
      assets: { findByItem: vi.fn(), deleteWithCascade: vi.fn() },
      // Only what the root breadcrumb crumb's landing view (`collections`,
      // plural — the one real, eagerly-mounted view these tests ever visit)
      // needs on mount. An empty list means `countItems` is never called.
      collections: { findAll: vi.fn() },
    },
  },
}))

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

// Every routed name other than 'db-browser' (which the lazy-race test below
// depends on, through the real loader) is left pending forever here: these
// tests only assert on the per-pane location strip, never on a routed body,
// so the real — often heavy, store-hungry — view components (CollectionView,
// ItemView, ...) are never pulled in. `@tauri-apps/api/core` and
// `@tauri-apps/plugin-fs` are already safely mocked globally (test-setup.ts),
// so the asset-delete flow below runs its real production code path.
vi.mock('$lib/route-loader', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/route-loader')>()
  return {
    ...actual,
    loadRouteView: (name: Parameters<typeof actual.loadRouteView>[0]) =>
      name === 'db-browser' ? actual.loadRouteView(name) : new Promise(() => {}),
  }
})

function resetWorkspace() {
  while (workspace.tabs.length > 1) {
    workspace.closeTab(workspace.tabs.at(-1)!.id)
  }
  workspace.activeNavigation.resetToPath([{ name: 'home' }])
}

function itemView(overrides: Partial<Extract<View, { name: 'item' }>> = {}): View {
  return {
    name: 'item',
    collectionId: 'col-1',
    collectionName: 'Archivo',
    itemId: 'item-1',
    itemTitle: 'Acta 1',
    ...overrides,
  }
}

describe('WorkPane', () => {
  beforeEach(() => {
    locale.set('es')
    resetWorkspace()
    storeRef.current.items.findPreviousCardSummary.mockReset()
    storeRef.current.items.findNextCardSummary.mockReset()
    storeRef.current.assets.findByItem.mockReset()
    storeRef.current.assets.deleteWithCascade.mockReset()
    storeRef.current.collections.findAll.mockReset().mockResolvedValue([])
  })

  it('renders HomeView synchronously for the home route (no lazy-load flash)', () => {
    render(WorkPane, { paneId: workspace.activeTabId })
    expect(screen.getByText('Espacio de trabajo')).toBeInTheDocument()
  })

  it('a slow lazy view in one pane does not block or corrupt an independent second pane', async () => {
    const secondPaneId = workspace.openTab({ name: 'db-browser' })!
    workspace.navigationFor(workspace.tabs[0]!.id).navigate({ name: 'db-browser' })

    render(WorkPane, { paneId: workspace.tabs[0]!.id })
    render(WorkPane, { paneId: secondPaneId })

    // Both panes requested the same lazy view name ('db-browser'); the first
    // pane then navigates away before either import settles. The second pane
    // must still render once the shared route-loader promise resolves —
    // its own effect's `cancelled` flag must never be tripped by the other
    // pane's navigation (Review Focus #5).
    workspace.navigationFor(workspace.tabs[0]!.id).navigate({ name: 'collections' })

    await waitFor(() => {
      expect(screen.getAllByRole('heading', { level: 1 }).length).toBeGreaterThan(0)
    })
  })

  // Spec, Location strip: "Each strip reads and drives only its own pane's
  // navigation." Every test below mounts two real `WorkPane`s on two
  // different real workspace tabs and proves an action taken in one never
  // leaks into the other.
  describe('pane isolation', () => {
    it("Back in pane B pops only tab B's history; pane A is unchanged", async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)
      navA.navigate({ name: 'collection', id: 'col-a', collectionName: 'Archivo A' })
      navB.navigate({ name: 'collection', id: 'col-b', collectionName: 'Archivo B' })

      render(WorkPane, { paneId: tabA })
      const { container: containerB } = render(WorkPane, { paneId: tabB })

      await fireEvent.click(within(containerB).getByRole('button', { name: /Volver/ }))

      expect(navB.current).toEqual({ name: 'home' })
      expect(navA.current).toEqual({
        name: 'collection',
        id: 'col-a',
        collectionName: 'Archivo A',
      })
    })

    it('clicking a breadcrumb crumb in pane B navigates tab B only', async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)
      navA.navigate({ name: 'collection', id: 'col-a', collectionName: 'Archivo A' })
      navB.navigate({ name: 'collection', id: 'col-b', collectionName: 'Archivo B' })

      render(WorkPane, { paneId: tabA })
      const { container: containerB } = render(WorkPane, { paneId: tabB })

      await fireEvent.click(within(containerB).getByRole('button', { name: 'Colecciones' }))

      expect(navB.current).toEqual({ name: 'collections' })
      expect(navA.current).toEqual({
        name: 'collection',
        id: 'col-a',
        collectionName: 'Archivo A',
      })
    })

    it('a sibling next/previous arrow in pane B navigates tab B only', async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)
      navB.navigate(itemView())
      storeRef.current.items.findPreviousCardSummary.mockResolvedValue({
        id: 'item-0',
        title: 'Acta 0',
      })
      storeRef.current.items.findNextCardSummary.mockResolvedValue({
        id: 'item-2',
        title: 'Acta 2',
      })

      render(WorkPane, { paneId: tabA })
      const { container: containerB } = render(WorkPane, { paneId: tabB })

      const nextButton = await within(containerB).findByRole('button', {
        name: 'Documento siguiente',
      })
      await waitFor(() => expect(nextButton).toBeEnabled())
      await fireEvent.click(nextButton)

      expect(navB.current).toEqual({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Archivo',
        itemId: 'item-2',
        itemTitle: 'Acta 2',
      })
      expect(navA.current).toEqual({ name: 'home' })
    })

    describe('asset delete', () => {
      afterEach(() => {
        vi.restoreAllMocks()
      })

      it('confirming asset delete in a pane prunes it from every tab via workspace.forgetAsset', async () => {
        const tabA = workspace.activeTabId
        const tabB = workspace.openTab()!
        const navA = workspace.navigationFor(tabA)
        const navB = workspace.navigationFor(tabB)
        // The same page open in two tabs at once (spec, "Hazards of a view
        // mounted twice") is the strongest proof that pruning is cross-tab,
        // not just a same-pane replace.
        const shared = itemView({ assetId: 'asset-1', assetLabel: 'acta-1.png' })
        navA.navigate(shared)
        navB.navigate(shared)

        const asset = {
          id: 'asset-1',
          itemId: 'item-1',
          path: 'docs/acta-1.png',
          type: 'image',
          size: 10,
          sortIndex: 0,
          createdAt: 1,
          parentAssetId: null,
        }
        storeRef.current.assets.findByItem.mockResolvedValueOnce([asset]).mockResolvedValueOnce([])
        storeRef.current.assets.deleteWithCascade.mockResolvedValue(asset)

        const forgetAssetSpy = vi.spyOn(workspace, 'forgetAsset')

        render(WorkPane, { paneId: tabA })
        const { container: containerB } = render(WorkPane, { paneId: tabB })

        await fireEvent.click(
          within(containerB).getByRole('button', { name: 'Eliminar página activa' })
        )
        await fireEvent.click(within(containerB).getByRole('button', { name: 'Eliminar página' }))

        await waitFor(() => {
          expect(forgetAssetSpy).toHaveBeenCalledWith('asset-1')
        })
        // The spy call alone proves the request; this proves the prune
        // actually reached the OTHER tab's own navigation.
        expect(navA.current).toEqual({ name: 'home' })
      })

      it('a cancelled confirmation does not call workspace.forgetAsset', async () => {
        const tabB = workspace.openTab()!
        const navB = workspace.navigationFor(tabB)
        navB.navigate(itemView({ assetId: 'asset-1', assetLabel: 'acta-1.png' }))

        const forgetAssetSpy = vi.spyOn(workspace, 'forgetAsset')

        const { container: containerB } = render(WorkPane, { paneId: tabB })

        await fireEvent.click(
          within(containerB).getByRole('button', { name: 'Eliminar página activa' })
        )
        await fireEvent.click(within(containerB).getByRole('button', { name: 'Cancelar' }))

        expect(forgetAssetSpy).not.toHaveBeenCalled()
        expect(
          within(containerB).queryByRole('button', { name: 'Eliminar página' })
        ).not.toBeInTheDocument()
      })
    })
  })
})
