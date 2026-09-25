import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { invoke } from '@tauri-apps/api/core'
import { remove } from '@tauri-apps/plugin-fs'
import WorkPane from './WorkPane.svelte'
import CollectionRouteProbe from './__fixtures__/CollectionRouteProbe.svelte'
import ItemRouteProbe from './__fixtures__/ItemRouteProbe.svelte'
import { routeProbeLog } from './__fixtures__/route-probe-log'
import { workspace } from '$lib/workspace'
import { locale } from '$lib/i18n'
import type { View } from '$lib/navigation'
import { citationsForAsset } from '$lib/writing'
import { resolveStoredAssetPath } from '$lib/file-import'
import {
  DOCUMENT_ASSET_DELETED_EVENT,
  DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
} from '$lib/document-explorer'

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
//
// `routeOverride.current`, when set by a test, answers first: it lets one
// test resolve chosen names to small probe components (see the stale-mount
// test below) without changing what every other test gets.
const { routeOverride } = vi.hoisted(() => ({
  routeOverride: {
    current: null as null | ((name: string) => Promise<{ default: unknown }> | undefined),
  },
}))

vi.mock('$lib/route-loader', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/route-loader')>()
  return {
    ...actual,
    loadRouteView: (name: Parameters<typeof actual.loadRouteView>[0]) =>
      routeOverride.current?.(name) ??
      (name === 'db-browser' ? actual.loadRouteView(name) : new Promise(() => {})),
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
    routeOverride.current = null
    routeProbeLog.length = 0
  })

  it('renders HomeView synchronously for the home route (no lazy-load flash)', () => {
    render(WorkPane, { paneId: workspace.activeTabId })
    expect(screen.getByText('Espacio de trabajo')).toBeInTheDocument()
  })

  // Svelte runs template effects before user `$effect`s, so on a navigation
  // the body used to switch to the NEW view's branch while `routeLoad` still
  // held the PREVIOUS view's module — mounting, for one flush, the old view
  // with the new view's props (a CollectionView handed `{ itemId, ... }`),
  // running its onMount and destroying it at once. That throwaway
  // CollectionView registered a Tauri drop listener it could never release,
  // so every collection -> item navigation added one more import per drop
  // (drop-dup diagnosis, defect 1).
  it("never mounts the previous routed view with the next view's props on navigation", async () => {
    routeOverride.current = (name) => {
      if (name === 'collection') return Promise.resolve({ default: CollectionRouteProbe })
      if (name === 'item') return Promise.resolve({ default: ItemRouteProbe })
      return undefined
    }
    const nav = workspace.activeNavigation
    nav.navigate({ name: 'collection', id: 'col-1', collectionName: 'Archivo' })
    render(WorkPane, { paneId: workspace.activeTabId })
    await waitFor(() => expect(screen.getByTestId('collection-route-probe')).toBeInTheDocument())

    nav.navigate(itemView({ itemId: 'it-1', collectionId: 'col-1' }))
    await waitFor(() => expect(screen.getByTestId('item-route-probe')).toBeInTheDocument())

    nav.navigate({ name: 'collection', id: 'col-1', collectionName: 'Archivo' })
    await waitFor(() => expect(screen.getByTestId('collection-route-probe')).toBeInTheDocument())

    // Each probe is only ever initialised with the props of its own view.
    expect(routeProbeLog).toEqual([
      { probe: 'collection', props: { collectionId: 'col-1' } },
      { probe: 'item', props: { itemId: 'it-1', collectionId: 'col-1' } },
      { probe: 'collection', props: { collectionId: 'col-1' } },
    ])
  })

  // Home and Collections render eagerly and never touch `routeLoad`, so a
  // module loaded before them used to stay `ready` under its old name. The
  // next arrival at that same name mounted the cached module for one flush,
  // then the effect's reset destroyed it and mounted a fresh one: a
  // throwaway onMount per round trip through an eager view.
  it('mounts a routed view once when returning to it through an eager view', async () => {
    routeOverride.current = (name) =>
      name === 'collection' ? Promise.resolve({ default: CollectionRouteProbe }) : undefined
    const nav = workspace.activeNavigation
    nav.navigate({ name: 'collection', id: 'col-1', collectionName: 'Archivo' })
    render(WorkPane, { paneId: workspace.activeTabId })
    await waitFor(() => expect(screen.getByTestId('collection-route-probe')).toBeInTheDocument())

    nav.navigate({ name: 'collections' })
    await waitFor(() => expect(screen.queryByTestId('collection-route-probe')).toBeNull())

    nav.navigate({ name: 'collection', id: 'col-2', collectionName: 'Otro' })
    await waitFor(() => expect(screen.getByTestId('collection-route-probe')).toBeInTheDocument())

    expect(routeProbeLog).toEqual([
      { probe: 'collection', props: { collectionId: 'col-1' } },
      { probe: 'collection', props: { collectionId: 'col-2' } },
    ])
  })

  it('mounts Writing once when returning to it through Home', async () => {
    routeOverride.current = (name) =>
      name === 'writing' ? Promise.resolve({ default: ItemRouteProbe }) : undefined
    const nav = workspace.activeNavigation
    nav.navigate({ name: 'writing', documentId: null })
    render(WorkPane, { paneId: workspace.activeTabId })
    await waitFor(() => expect(screen.getByTestId('item-route-probe')).toBeInTheDocument())

    nav.navigate({ name: 'home' })
    await waitFor(() => expect(screen.queryByTestId('item-route-probe')).toBeNull())

    nav.navigate({ name: 'writing', documentId: null })
    await waitFor(() => expect(screen.getByTestId('item-route-probe')).toBeInTheDocument())

    expect(routeProbeLog).toHaveLength(2)
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
        // The confirmation floats out of the pane into <body> (see `portal`);
        // only B opened one, so it is the only dialog on screen.
        await fireEvent.click(
          within(screen.getByRole('dialog')).getByRole('button', { name: 'Eliminar página' })
        )

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
        await fireEvent.click(
          within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancelar' })
        )

        expect(forgetAssetSpy).not.toHaveBeenCalled()
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      })
    })
  })

  // Ported from the retired `TopBar.keyset.test.ts` (Task 2.4 fix round 1):
  // the keyset sibling-lookup behaviour moved into WorkPane.svelte's own
  // `loadSiblingItems`, but its dedicated coverage did not move with it.
  describe('sibling navigation', () => {
    it('queries siblings with the keyset shape and enables both controls when both exist', async () => {
      storeRef.current.items.findPreviousCardSummary.mockResolvedValue({
        id: 'item-0',
        title: 'Acta 0',
      })
      storeRef.current.items.findNextCardSummary.mockResolvedValue({
        id: 'item-2',
        title: 'Acta 2',
      })

      const nav = workspace.activeNavigation
      nav.navigate(itemView({ itemId: 'item-1', itemTitle: 'Acta 1' }))

      render(WorkPane, { paneId: workspace.activeTabId })

      await waitFor(() => {
        expect(storeRef.current.items.findPreviousCardSummary).toHaveBeenCalledWith('col-1', {
          title: 'Acta 1',
          id: 'item-1',
        })
      })
      expect(storeRef.current.items.findNextCardSummary).toHaveBeenCalledWith('col-1', {
        title: 'Acta 1',
        id: 'item-1',
      })

      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Documento anterior' })).toBeEnabled()
      )
      expect(screen.getByRole('button', { name: 'Documento siguiente' })).toBeEnabled()
    })

    it('disables the edge control that has no sibling', async () => {
      storeRef.current.items.findPreviousCardSummary.mockResolvedValue(null)
      storeRef.current.items.findNextCardSummary.mockResolvedValue({
        id: 'item-2',
        title: 'Acta 2',
      })

      const nav = workspace.activeNavigation
      nav.navigate(itemView({ itemId: 'item-1', itemTitle: 'Acta 1' }))

      render(WorkPane, { paneId: workspace.activeTabId })

      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Documento siguiente' })).toBeEnabled()
      )
      expect(screen.getByRole('button', { name: 'Documento anterior' })).toBeDisabled()
    })

    /**
     * Regression guard for the `siblingRequestId` counter in
     * `WorkPane.svelte`'s `loadSiblingItems` (~lines 93-109): a slow first
     * lookup must never overwrite the sibling state of whatever document the
     * user has since navigated to.
     */
    it('discards a stale sibling response after the document changes', async () => {
      let releaseFirst: ((value: unknown) => void) | undefined
      storeRef.current.items.findPreviousCardSummary
        .mockImplementationOnce(() => new Promise((resolve) => (releaseFirst = resolve)))
        .mockResolvedValue({ id: 'item-20', title: 'Tango' })
      storeRef.current.items.findNextCardSummary.mockResolvedValue(null)

      const nav = workspace.activeNavigation
      nav.navigate(itemView({ itemId: 'item-10', itemTitle: 'Mosaic' }))

      render(WorkPane, { paneId: workspace.activeTabId })

      nav.navigate(itemView({ itemId: 'item-21', itemTitle: 'Ubaldo' }))
      releaseFirst?.({ id: 'item-09', title: 'Luna' })

      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Documento anterior' })).toBeEnabled()
      )
      await fireEvent.click(screen.getByRole('button', { name: 'Documento anterior' }))

      // Luna belonged to the document the user already left.
      expect(nav.current).toEqual(
        expect.objectContaining({ itemId: 'item-20', itemTitle: 'Tango' })
      )
    })
  })

  // Ported from the retired `TopBar.test.ts` (Task 2.4 fix round 1): the
  // asset-delete confirmation flow moved into WorkPane.svelte's own
  // `handleDeleteAssetConfirm`, but its dedicated coverage did not move with
  // it. `invoke`/`remove` are the global mocks from `test-setup.ts`; the real
  // `$lib/file-import` helpers run for real on top of them, the same way
  // production code does.
  describe('asset delete (single pane)', () => {
    afterEach(() => {
      vi.restoreAllMocks()
    })

    it('deletes the active asset and selects the next remaining asset', async () => {
      const currentAsset = {
        id: 'asset-1',
        itemId: 'item-1',
        path: 'docs/11111111-1111-4111-8111-111111111111_acta-1.png',
        type: 'image',
        size: 10,
        sortIndex: 0,
        createdAt: 1,
        parentAssetId: null,
      }
      const nextAsset = {
        ...currentAsset,
        id: 'asset-2',
        path: 'docs/22222222-2222-4222-8222-222222222222_acta-2.png',
        sortIndex: 1,
      }
      storeRef.current.assets.findByItem
        .mockResolvedValueOnce([currentAsset, nextAsset])
        .mockResolvedValueOnce([nextAsset])
      storeRef.current.assets.deleteWithCascade.mockResolvedValue(currentAsset)

      const nav = workspace.activeNavigation
      nav.navigate(
        itemView({
          itemId: 'item-1',
          itemTitle: 'Acta 1',
          assetId: 'asset-1',
          assetLabel: 'acta-1.png',
        })
      )

      const deletedEvents: Event[] = []
      const changedEvents: Event[] = []
      const onDeleted = (event: Event) => deletedEvents.push(event)
      const onChanged = (event: Event) => changedEvents.push(event)
      window.addEventListener(DOCUMENT_ASSET_DELETED_EVENT, onDeleted)
      window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, onChanged)

      try {
        render(WorkPane, { paneId: workspace.activeTabId })
        await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página activa' }))
        expect(screen.getByText(/¿Seguro que querés eliminar acta-1\.png\?/)).toBeInTheDocument()

        await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página' }))

        await waitFor(() => {
          expect(storeRef.current.assets.deleteWithCascade).toHaveBeenCalledWith('asset-1')
        })
        expect(vi.mocked(invoke)).toHaveBeenCalledWith('delete_asset_files', {
          assetPath: currentAsset.path,
        })
        expect(vi.mocked(invoke)).toHaveBeenCalledWith('delete_image_thumbnail', {
          assetId: 'asset-1',
        })
        await waitFor(() => {
          expect(nav.current).toEqual({
            name: 'item',
            collectionId: 'col-1',
            collectionName: 'Archivo',
            itemId: 'item-1',
            itemTitle: 'Acta 1',
            assetId: 'asset-2',
            assetLabel: 'acta-2.png',
          })
        })
        expect(deletedEvents).toHaveLength(1)
        expect(changedEvents).toHaveLength(1)
      } finally {
        window.removeEventListener(DOCUMENT_ASSET_DELETED_EVENT, onDeleted)
        window.removeEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, onChanged)
      }
    })

    it('replaces with the collection after deleting the last asset (Back never lands on it)', async () => {
      const currentAsset = {
        id: 'asset-1',
        itemId: 'item-1',
        path: 'docs/acta-1.pdf',
        type: 'pdf',
        size: 10,
        sortIndex: 0,
        createdAt: 1,
        parentAssetId: null,
      }
      storeRef.current.assets.findByItem
        .mockResolvedValueOnce([currentAsset])
        .mockResolvedValueOnce([])
      storeRef.current.assets.deleteWithCascade.mockResolvedValue(currentAsset)

      const nav = workspace.activeNavigation
      nav.navigate(
        itemView({
          itemId: 'item-1',
          itemTitle: 'Acta 1',
          assetId: 'asset-1',
          assetLabel: 'acta-1.pdf',
        })
      )

      render(WorkPane, { paneId: workspace.activeTabId })
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página activa' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página' }))

      await waitFor(() => {
        expect(nav.current).toEqual({
          name: 'collection',
          id: 'col-1',
          collectionName: 'Archivo',
        })
      })
      expect(vi.mocked(remove)).toHaveBeenCalledWith(resolveStoredAssetPath('docs/acta-1.pdf'))
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('delete_pdf_thumbnail', { assetId: 'asset-1' })
      expect(vi.mocked(remove)).toHaveBeenCalledWith(
        resolveStoredAssetPath('docs/acta-1.pdf').replace(/\.pdf$/i, '.pages'),
        { recursive: true }
      )
    })

    /**
     * Regression: `back()` could land on a deleted page's own screen once it
     * no longer existed. `forgetAsset` runs after `replace` on purpose: the
     * two must not fight over the current entry.
     */
    it('replaces the view before pruning history, once the cascade succeeds', async () => {
      const currentAsset = {
        id: 'asset-1',
        itemId: 'item-1',
        path: 'docs/11111111-1111-4111-8111-111111111111_acta-1.png',
        type: 'image',
        size: 10,
        sortIndex: 0,
        createdAt: 1,
        parentAssetId: null,
      }
      const nextAsset = {
        ...currentAsset,
        id: 'asset-2',
        path: 'docs/22222222-2222-4222-8222-222222222222_acta-2.png',
        sortIndex: 1,
      }
      storeRef.current.assets.findByItem
        .mockResolvedValueOnce([currentAsset, nextAsset])
        .mockResolvedValueOnce([nextAsset])
      storeRef.current.assets.deleteWithCascade.mockResolvedValue(currentAsset)

      const nav = workspace.activeNavigation
      const replaceSpy = vi.spyOn(nav, 'replace')
      const forgetAssetSpy = vi.spyOn(workspace, 'forgetAsset')
      nav.navigate(
        itemView({
          itemId: 'item-1',
          itemTitle: 'Acta 1',
          assetId: 'asset-1',
          assetLabel: 'acta-1.png',
        })
      )

      render(WorkPane, { paneId: workspace.activeTabId })
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página activa' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página' }))

      await waitFor(() => {
        expect(forgetAssetSpy).toHaveBeenCalledWith('asset-1')
      })
      // Never fighting `replace`: prune runs after it, not before.
      const replaceOrder = replaceSpy.mock.invocationCallOrder.at(0)
      const forgetOrder = forgetAssetSpy.mock.invocationCallOrder.at(0)
      expect(replaceOrder).toBeDefined()
      expect(forgetOrder).toBeDefined()
      expect(replaceOrder as number).toBeLessThan(forgetOrder as number)
    })

    it('does not prune history when the cascade delete fails', async () => {
      const currentAsset = {
        id: 'asset-1',
        itemId: 'item-1',
        path: 'docs/acta-1.pdf',
        type: 'pdf',
        size: 10,
        sortIndex: 0,
        createdAt: 1,
        parentAssetId: null,
      }
      storeRef.current.assets.findByItem.mockResolvedValueOnce([currentAsset])
      storeRef.current.assets.deleteWithCascade.mockRejectedValue(new Error('DB locked'))

      const nav = workspace.activeNavigation
      const replaceSpy = vi.spyOn(nav, 'replace')
      const forgetAssetSpy = vi.spyOn(workspace, 'forgetAsset')
      nav.navigate(
        itemView({
          itemId: 'item-1',
          itemTitle: 'Acta 1',
          assetId: 'asset-1',
          assetLabel: 'acta-1.pdf',
        })
      )

      render(WorkPane, { paneId: workspace.activeTabId })
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página activa' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar página' }))

      await waitFor(() => {
        expect(storeRef.current.assets.deleteWithCascade).toHaveBeenCalledWith('asset-1')
      })
      expect(forgetAssetSpy).not.toHaveBeenCalled()
      expect(replaceSpy).not.toHaveBeenCalled()
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

  // Regression guard (Stage 2 visual fix #3): `.breadcrumb` had no flex-grow,
  // so it only ever took up its own text width and the sibling prev/next
  // arrows + delete-asset button sat right after it instead of pinned to the
  // strip's right edge — they visibly moved with the file title's length.
  // `.breadcrumb` must grow to fill the space between Back and the controls
  // (truncating its own crumbs via `min-width: 0` + ellipsis instead), and
  // the controls group must not shrink.
  it('lets the breadcrumb grow/truncate so prev/next + delete stay pinned right', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'WorkPane.svelte'), 'utf-8')

    const breadcrumbStart = source.indexOf('  .breadcrumb {')
    const breadcrumbRule = source.slice(breadcrumbStart, source.indexOf('}', breadcrumbStart))
    expect(breadcrumbRule).toMatch(/flex:\s*1/)

    const crumbStart = source.indexOf('  .crumb {')
    const crumbRule = source.slice(crumbStart, source.indexOf('}', crumbStart))
    expect(crumbRule).toMatch(/min-width:\s*0;/)

    const crumbNavStart = source.indexOf('  .crumb-nav {')
    const crumbNavRule = source.slice(crumbNavStart, source.indexOf('}', crumbNavStart))
    expect(crumbNavRule).toMatch(/flex-shrink:\s*0;/)

    const deleteStart = source.indexOf('  .location-strip__delete {')
    expect(deleteStart).toBeGreaterThan(-1)
    const deleteRule = source.slice(deleteStart, source.indexOf('}', deleteStart))
    expect(deleteRule).toMatch(/flex-shrink:\s*0;/)
  })

  // A split pane and the single-pane view are the same width the window is,
  // but a split pane rarely is — so views that reflow their own grids off a
  // viewport `@media` query break at pane widths a query keyed to the window
  // never sees. `.work-pane` names a `pane` container so those views can key
  // off its own rendered width instead (columns-adapt-to-pane-width fix).
  it('names a CSS container so views can size columns from this pane, not the viewport', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'WorkPane.svelte'), 'utf-8')

    const rootStart = source.indexOf('  .work-pane {')
    const rootRule = source.slice(rootStart, source.indexOf('}', rootStart))
    expect(rootRule).toMatch(/container-type:\s*inline-size;/)
    expect(rootRule).toMatch(/container-name:\s*pane;/)

    // The 320px side-by-side floor now lives solely on AppShell's
    // `.content__pane` (what SplitDivider/clampSplitRatio actually clamp). A
    // second, independent floor here would refuse to shrink below 320 of
    // CONTENT width and overflow that already-padded parent by the padding
    // amount at the clamp (split inner-spacing fix).
    expect(rootRule).not.toMatch(/min-width:\s*320px;/)
  })

  // Final visual check (split view, narrowed pane): `.work-pane__body` set
  // only `overflow-y: auto`, leaving `overflow-x` at its CSS-spec default —
  // which computes to `auto` too once paired with a non-`visible`
  // `overflow-y` value. Combined with a view whose flex/grid items could not
  // shrink below their own intrinsic content width (HomeView's header row
  // and quick-access grid), this gave the pane body its own independent
  // horizontal scroll axis and let content clip against the pane's edge
  // instead of reflowing. `overflow-x: hidden` removes that scroll axis
  // outright: horizontal overflow must be fixed by making views reflow (see
  // HomeView.layout.test.ts), never papered over with a second scrollbar.
  it('never lets the pane body scroll or overflow horizontally on its own', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'WorkPane.svelte'), 'utf-8')

    const bodyStart = source.indexOf('  .work-pane__body {')
    const bodyRule = source.slice(bodyStart, source.indexOf('}', bodyStart))
    expect(bodyRule).toMatch(/overflow-x:\s*hidden;/)
    expect(bodyRule).toMatch(/overflow-y:\s*auto;/)
  })

  // Visual round 3, item 1: the pane's horizontal inset used to live only on
  // AppShell's `.content__pane` (an ancestor OUTSIDE this scrolling box), so
  // when `.work-pane__body` grew a vertical scrollbar, cards rendered flush
  // against it (zero gap) while the scrollbar's own track ate into what
  // looked like the pane's right margin — the left margin, never touched by
  // a scrollbar, stayed intact. This box must carry its own inline padding
  // (so the scrollbar — when shown — renders outside it, level with the
  // content, not through it) while cancelling the ancestor's padding via a
  // matching negative margin, so a pane with no scrollbar renders at
  // exactly the same width as before (no visible band, single-pane look
  // unchanged).
  it('carries its own inline padding (cancelling the ancestor pane inset) so a scrollbar never eats the right margin', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'WorkPane.svelte'), 'utf-8')

    const bodyStart = source.indexOf('  .work-pane__body {')
    const bodyRule = source.slice(bodyStart, source.indexOf('}', bodyStart))
    expect(bodyRule).toMatch(/margin-inline:\s*calc\(-1 \* var\(--space-5\)\);/)
    expect(bodyRule).toMatch(/padding-inline:\s*var\(--space-5\);/)
  })

  // Deferred edge from Task 1.5 (progress.md ruling, carried to 3.3):
  // workspace.navigateActive()'s Writing single-tab redirect only guards
  // entry through the workspace. A pane's own NavigationStore can still
  // reach `writing` directly — via Back/forward history — bypassing that
  // guard. When that happens while another tab already shows Writing, this
  // pane must not mount a second WritingView.
  describe('Writing open in another tab (pane-level guard)', () => {
    it('keeps the incumbent (chronological owner) shown, not "first in tab-list order", when a later arrival also reaches Writing through its own history', async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)

      // Tab B reaches `writing` FIRST — the incumbent (workspace.ts's
      // writingOwnerId). Tab A reaches it SECOND, directly on its own
      // NavigationStore — simulating Back into old history, bypassing
      // workspace.navigateActive() entirely (the same hazard the workspace-
      // level `writingOwnerId` tests cover). Tab A was created first, so
      // "first in tab-list order" — the bug this replaces — would wrongly
      // treat A as the owner and unmount B's live WritingView.
      navB.navigate({ name: 'writing', documentId: 'doc-1', documentTitle: 'Doc 1' })
      navA.navigate({ name: 'writing', documentId: 'doc-1', documentTitle: 'Doc 1' })

      render(WorkPane, { paneId: tabB })
      render(WorkPane, { paneId: tabA })

      // Only the non-owner (tab A) shows the notice — `getByText` also
      // proves it is not duplicated onto tab B's pane, since it would throw
      // on more than one match.
      expect(screen.getByText('Escritura está abierta en otra pestaña.')).toBeInTheDocument()

      await fireEvent.click(screen.getByRole('button', { name: 'Ir a esa pestaña' }))

      expect(workspace.activeTabId).toBe(tabB)
    })
  })
})
