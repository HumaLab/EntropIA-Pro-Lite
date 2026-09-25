import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { invoke } from '@tauri-apps/api/core'
import { remove } from '@tauri-apps/plugin-fs'
import WorkPane from './WorkPane.svelte'
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

  // Deferred edge from Task 1.5 (progress.md ruling, carried to 3.3):
  // workspace.navigateActive()'s Writing single-tab redirect only guards
  // entry through the workspace. A pane's own NavigationStore can still
  // reach `writing` directly — via Back/forward history — bypassing that
  // guard. When that happens while another tab already shows Writing, this
  // pane must not mount a second WritingView.
  describe('Writing open in another tab (pane-level guard)', () => {
    it('shows a notice instead of mounting a second WritingView, and its button activates the tab that owns Writing', async () => {
      const tabA = workspace.activeTabId
      const tabB = workspace.openTab()!
      const navA = workspace.navigationFor(tabA)
      const navB = workspace.navigationFor(tabB)

      // Both panes reach `writing` on their OWN NavigationStore directly —
      // simulating history/back, not workspace.navigateActive() — so tab A
      // (first in tab order) is the legitimate owner and tab B is the one
      // that must fall back to the notice.
      navA.navigate({ name: 'writing', documentId: 'doc-1', documentTitle: 'Doc 1' })
      navB.navigate({ name: 'writing', documentId: 'doc-1', documentTitle: 'Doc 1' })

      render(WorkPane, { paneId: tabA })
      render(WorkPane, { paneId: tabB })

      expect(screen.getByText('Escritura está abierta en otra pestaña.')).toBeInTheDocument()

      await fireEvent.click(screen.getByRole('button', { name: 'Ir a esa pestaña' }))

      expect(workspace.activeTabId).toBe(tabA)
    })
  })
})
