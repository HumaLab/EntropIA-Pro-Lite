import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import WorkPane from './WorkPane.svelte'
import { workspace } from '$lib/workspace'
import { locale } from '$lib/i18n'
import type { View } from '$lib/navigation'
import { citationsForAsset } from '$lib/writing'

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

vi.mock('$lib/writing', () => ({
  citationsForAsset: vi.fn().mockResolvedValue([]),
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
    // Re-armed every test: the 'asset delete' describe below calls
    // `vi.restoreAllMocks()` in its own `afterEach`, which also strips this
    // module-level `vi.fn()`'s resolved value back to `undefined`.
    vi.mocked(citationsForAsset).mockReset().mockResolvedValue([])
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
  // leaks into the other. Tab A is made the *active* tab and then acted on
  // through pane B — so a regression reading `workspace.activeNavigation`
  // instead of its own `navigationFor(paneId)` (which would still pass a
  // same-active-tab test, since acting "in B" would coincidentally hit A's
  // store anyway) is actually exercised: see the mutation-check evidence in
  // the task report for a recorded failing run of exactly that regression.
  describe('pane isolation', () => {
    it("Back in pane B pops only tab B's history; pane A is unchanged", async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)
      navA.navigate({ name: 'collection', id: 'col-a', collectionName: 'Archivo A' })
      navB.navigate({ name: 'collection', id: 'col-b', collectionName: 'Archivo B' })
      workspace.activateTab(tabA)

      const { container: containerA } = render(WorkPane, { paneId: tabA })
      const { container: containerB } = render(WorkPane, { paneId: tabB })

      await fireEvent.click(within(containerB).getByRole('button', { name: /Volver/ }))

      expect(navB.current).toEqual({ name: 'home' })
      expect(navA.current).toEqual({
        name: 'collection',
        id: 'col-a',
        collectionName: 'Archivo A',
      })
      // Pane A's own rendered strip still shows tab A's location.
      expect(within(containerA).getByText('Archivo A')).toBeInTheDocument()
    })

    it('clicking a breadcrumb crumb in pane B navigates tab B only', async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)
      navA.navigate({ name: 'collection', id: 'col-a', collectionName: 'Archivo A' })
      navB.navigate({ name: 'collection', id: 'col-b', collectionName: 'Archivo B' })
      workspace.activateTab(tabA)

      const { container: containerA } = render(WorkPane, { paneId: tabA })
      const { container: containerB } = render(WorkPane, { paneId: tabB })

      await fireEvent.click(within(containerB).getByRole('button', { name: 'Colecciones' }))

      expect(navB.current).toEqual({ name: 'collections' })
      expect(navA.current).toEqual({
        name: 'collection',
        id: 'col-a',
        collectionName: 'Archivo A',
      })
      // Pane A's own rendered strip still shows tab A's location.
      expect(within(containerA).getByText('Archivo A')).toBeInTheDocument()
    })

    it('a sibling next/previous arrow in pane B navigates tab B only', async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)
      navB.navigate(itemView())
      workspace.activateTab(tabA)
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

      it("confirming asset delete in pane B calls workspace.forgetAsset with B's own assetId, not A's", async () => {
        const tabA = workspace.activeTabId
        const tabB = workspace.openTab()!
        const navA = workspace.navigationFor(tabA)
        const navB = workspace.navigationFor(tabB)
        // Deliberately different assets per tab: a regression reading the
        // active tab's navigation instead of its own would call
        // `forgetAsset` with A's id (since A is made active below), not B's.
        const viewA = itemView({ itemId: 'item-a', assetId: 'asset-a', assetLabel: 'acta-a.png' })
        const viewB = itemView({ itemId: 'item-b', assetId: 'asset-b', assetLabel: 'acta-b.png' })
        navA.navigate(viewA)
        navB.navigate(viewB)
        workspace.activateTab(tabA)

        const assetB = {
          id: 'asset-b',
          itemId: 'item-b',
          path: 'docs/acta-b.png',
          type: 'image',
          size: 10,
          sortIndex: 0,
          createdAt: 1,
          parentAssetId: null,
        }
        storeRef.current.assets.findByItem.mockResolvedValueOnce([assetB]).mockResolvedValueOnce([])
        storeRef.current.assets.deleteWithCascade.mockResolvedValue(assetB)

        const forgetAssetSpy = vi.spyOn(workspace, 'forgetAsset')

        render(WorkPane, { paneId: tabA })
        const { container: containerB } = render(WorkPane, { paneId: tabB })

        await fireEvent.click(
          within(containerB).getByRole('button', { name: 'Eliminar página activa' })
        )
        await fireEvent.click(within(containerB).getByRole('button', { name: 'Eliminar página' }))

        await waitFor(() => {
          expect(forgetAssetSpy).toHaveBeenCalledWith('asset-b')
        })
        expect(forgetAssetSpy).not.toHaveBeenCalledWith('asset-a')
        // Different assetId per tab, so B's own deletion never prunes A.
        expect(navA.current).toEqual(viewA)
      })

      it('a cancelled confirmation does not call workspace.forgetAsset', async () => {
        const tabA = workspace.activeTabId
        const tabB = workspace.openTab()!
        const navA = workspace.navigationFor(tabA)
        navA.navigate(itemView({ itemId: 'item-a', assetId: 'asset-a', assetLabel: 'acta-a.png' }))
        const navB = workspace.navigationFor(tabB)
        navB.navigate(itemView({ itemId: 'item-b', assetId: 'asset-b', assetLabel: 'acta-b.png' }))
        workspace.activateTab(tabA)

        const forgetAssetSpy = vi.spyOn(workspace, 'forgetAsset')

        render(WorkPane, { paneId: tabA })
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

  it('confirming asset delete prunes it across every tab, not only this pane', async () => {
    const deleteWithCascade = vi.fn().mockResolvedValue({ id: 'a1', type: 'image', path: 'a1.png' })
    const findByItem = vi.fn().mockResolvedValue([])
    const dbModule = (await import('$lib/db')) as unknown as { getStore: () => unknown }
    dbModule.getStore = () => ({
      items: { findPreviousCardSummary: vi.fn(), findNextCardSummary: vi.fn() },
      assets: { findByItem, deleteWithCascade },
    })

    workspace.activeNavigation.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'i1',
      itemTitle: 'Acta',
      assetId: 'a1',
      assetLabel: 'acta.png',
    })
    const forgetAssetSpy = vi.spyOn(workspace, 'forgetAsset')

    render(WorkPane, { paneId: workspace.activeTabId })

    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página activa' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página' }))

    await waitFor(() => expect(forgetAssetSpy).toHaveBeenCalledWith('a1'))
    forgetAssetSpy.mockRestore()
  })
})
