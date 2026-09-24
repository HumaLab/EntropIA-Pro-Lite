<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { remove } from '@tauri-apps/plugin-fs'
  import { workspace } from '$lib/workspace'
  import { setPaneNavigation } from '$lib/pane-context'
  import { loadRouteView, type LazyViewName } from '$lib/route-loader'
  import { getStore } from '$lib/db'
  import { citationsForAsset, type AssetDependency } from '$lib/writing'
  import type { View } from '$lib/navigation'
  import {
    deleteAssetFile,
    deleteImageThumbnail,
    deletePdfThumbnail,
    resolveStoredAssetPath,
  } from '$lib/file-import'
  import { getAssetPathLabel } from '$lib/item-metadata'
  import {
    DOCUMENT_ASSET_DELETED_EVENT,
    DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
    type DocumentAssetDeletedDetail,
    type DocumentExplorerCollectionChangedDetail,
  } from '$lib/document-explorer'
  import { collapseBreadcrumb } from '$lib/breadcrumb-overflow'
  import { locale, t } from '$lib/i18n'
  import { ActionIcon, Button, ConfirmDialog, IconButton } from '@entropia/ui'
  import type { Asset, Item } from '@entropia/store'
  import startupMark from '../assets/hlab-mark.png'
  import CollectionsView from '../views/CollectionsView.svelte'
  import HomeView from '../views/HomeView.svelte'

  let { paneId }: { paneId: string } = $props()

  const nav = workspace.navigationFor(paneId)
  // One call per mounted WorkPane instance, at init — AppShell (Task 2.3)
  // keys each instance by its tab id, so a tab switch remounts WorkPane and
  // re-binds the context to the newly active tab.
  setPaneNavigation(nav, paneId)

  const currentLocale = locale
  type ItemNavigationView = Extract<View, { name: 'item' }>

  const currentView = $derived($nav.current as View)
  const currentViewName = $derived(($nav.current as { name: string }).name)
  const currentItemId = $derived(currentView.name === 'item' ? currentView.itemId : null)
  const currentCollectionId = $derived(
    currentView.name === 'item'
      ? currentView.collectionId
      : currentView.name === 'collection'
        ? currentView.id
        : null
  )

  // ── Routed view (ported from App.svelte) ──
  let routeLoadRevision = $state(0)
  let routeLoad = $state.raw<
    | { status: 'loading' }
    | { status: 'ready'; module: Awaited<ReturnType<typeof loadRouteView>> }
    | { status: 'error'; error: unknown }
  >({ status: 'loading' })

  $effect(() => {
    routeLoadRevision
    const name = currentViewName
    if (name === 'collections' || name === 'home') return
    let cancelled = false
    routeLoad = { status: 'loading' }
    loadRouteView(name as LazyViewName).then(
      (module) => {
        // This pane's own flag — a sibling pane racing the same cached
        // import resolves independently and is never gated by it.
        if (!cancelled) routeLoad = { status: 'ready', module }
      },
      (error: unknown) => {
        if (!cancelled) routeLoad = { status: 'error', error }
      }
    )
    return () => {
      cancelled = true
    }
  })

  function retryRouteLoad() {
    routeLoadRevision += 1
  }

  // ── Sibling prev/next (Deviation 1) ──
  let previousItem = $state<Item | null>(null)
  let nextItem = $state<Item | null>(null)
  let siblingRequestId = 0

  async function loadSiblingItems() {
    const view = nav.current
    const requestId = ++siblingRequestId
    previousItem = null
    nextItem = null
    if (view.name !== 'item') return

    const store = getStore()
    const cursor = { title: view.itemTitle, id: view.itemId }
    try {
      const [previous, next] = await Promise.all([
        store.items.findPreviousCardSummary(view.collectionId, cursor),
        store.items.findNextCardSummary(view.collectionId, cursor),
      ])
      if (requestId !== siblingRequestId) return
      previousItem = previous
      nextItem = next
    } catch (error) {
      if (requestId !== siblingRequestId) return
      console.error('[WorkPane] Failed to load sibling documents', error)
    }
  }

  function buildItemView(item: Item) {
    const view = nav.current
    if (view.name !== 'item') return null
    return {
      name: 'item' as const,
      collectionId: view.collectionId,
      collectionName: view.collectionName,
      itemId: item.id,
      itemTitle: item.title,
    }
  }

  function navigateToSibling(item: Item | null) {
    const next = item ? buildItemView(item) : null
    if (next) nav.navigate(next)
  }

  $effect(() => {
    $nav.current
    void loadSiblingItems()
  })

  // ── Breadcrumb ──
  function getBreadcrumbPath(index: number): [View, ...View[]] | null {
    const view = nav.current
    const collectionsView: View = { name: 'collections' }
    if (index === 0) {
      return view.name === 'collections' || view.name === 'home' ? null : [collectionsView]
    }
    if (view.name === 'item') {
      if (index === 1) {
        return [
          collectionsView,
          { name: 'collection', id: view.collectionId, collectionName: view.collectionName },
        ]
      }
      return null
    }
    if (view.name === 'research')
      return index === 1 ? [collectionsView, { name: 'research' }] : null
    if (view.name === 'writing') {
      return index === 1 ? [collectionsView, { name: 'writing', documentId: null }] : null
    }
    if (view.name === 'investigation') {
      return index === 1 ? [collectionsView, { name: 'research' }] : null
    }
    return null
  }

  function navigateToBreadcrumb(index: number) {
    const path = getBreadcrumbPath(index)
    if (path) nav.navigate(path[path.length - 1]!)
  }

  const breadcrumbEntries = $derived(collapseBreadcrumb($nav.breadcrumb, 4))

  // ── Asset delete ──
  let showDeleteAssetConfirm = $state(false)
  let deletingAsset = $state(false)
  let deleteAssetError = $state<string | null>(null)
  let deleteAssetCitations = $state<AssetDependency[]>([])
  let pendingDeleteAssetView = $state<ItemNavigationView | null>(null)
  const deleteAssetAria = $derived(
    $currentLocale ? t('topbar.deleteAssetAria') : 'Eliminar página activa'
  )

  function leafAssetsOf(assets: Asset[]) {
    const parentIds = new Set(
      assets.filter((asset) => asset.parentAssetId).map((asset) => asset.parentAssetId as string)
    )
    return assets.filter((asset) => !parentIds.has(asset.id))
  }

  function openDeleteAssetConfirm() {
    if (nav.current.name !== 'item' || !nav.current.assetId || !nav.current.assetLabel) return
    pendingDeleteAssetView = { ...nav.current }
    deleteAssetError = null
    deleteAssetCitations = []
    showDeleteAssetConfirm = true
    const assetId = nav.current.assetId
    void citationsForAsset(assetId).then((found) => {
      if (pendingDeleteAssetView?.assetId === assetId) deleteAssetCitations = found
    })
  }

  function closeDeleteAssetConfirm() {
    if (deletingAsset) return
    showDeleteAssetConfirm = false
    deleteAssetError = null
    deleteAssetCitations = []
    pendingDeleteAssetView = null
  }

  async function cleanupDeletedAssetFile(asset: Asset) {
    try {
      if (asset.type === 'image') {
        await invoke('delete_asset_files', { assetPath: asset.path })
        await deleteImageThumbnail(asset.id)
        return
      }
      await deleteAssetFile(asset.path)
      if (asset.type === 'pdf') {
        await deletePdfThumbnail(asset.id)
        if (!asset.parentAssetId) {
          await remove(resolveStoredAssetPath(asset.path).replace(/\.pdf$/i, '.pages'), {
            recursive: true,
          })
        }
      }
    } catch (error) {
      console.warn('[WorkPane] Asset file cleanup warning:', error)
    }
  }

  async function handleDeleteAssetConfirm() {
    const view = pendingDeleteAssetView
    if (!view?.assetId) return

    deletingAsset = true
    deleteAssetError = null
    const store = getStore()
    const assetId = view.assetId
    let deletedIndex = 0

    try {
      const before = leafAssetsOf(await store.assets.findByItem(view.itemId))
      deletedIndex = Math.max(
        0,
        before.findIndex((asset: Asset) => asset.id === assetId)
      )
    } catch (error) {
      console.warn('[WorkPane] Failed to load assets before deletion:', error)
    }

    let deletedAsset: Asset
    try {
      deletedAsset = await store.assets.deleteWithCascade(assetId)
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      deleteAssetError = t('collection.error.deleteAsset', { message })
      deletingAsset = false
      return
    }

    let remainingAssets: Asset[] = []
    try {
      remainingAssets = leafAssetsOf(await store.assets.findByItem(view.itemId))
    } catch (error) {
      console.warn('[WorkPane] Failed to load assets after deletion:', error)
    }

    await cleanupDeletedAssetFile(deletedAsset)

    window.dispatchEvent(
      new CustomEvent<DocumentAssetDeletedDetail>(DOCUMENT_ASSET_DELETED_EVENT, {
        detail: { itemId: view.itemId, assetId },
      })
    )
    window.dispatchEvent(
      new CustomEvent<DocumentExplorerCollectionChangedDetail>(
        DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
        { detail: { collectionId: view.collectionId, itemId: view.itemId } }
      )
    )

    const nextAsset = remainingAssets[Math.min(deletedIndex, remainingAssets.length - 1)] ?? null
    if (nextAsset) {
      const nextView = { ...view }
      delete nextView.citationRange
      nav.replace({
        ...nextView,
        assetId: nextAsset.id,
        assetLabel: getAssetPathLabel(nextAsset.path),
      })
    } else {
      nav.replace({
        name: 'collection',
        id: view.collectionId,
        collectionName: view.collectionName,
      })
    }

    // Cross-tab pruning finally happens for real here (Deviation 4 resolved):
    // any other tab still pointing at this asset is pruned too.
    workspace.forgetAsset(assetId)

    deletingAsset = false
    showDeleteAssetConfirm = false
    pendingDeleteAssetView = null
  }
</script>

<div class="work-pane">
  <div class="location-strip" data-pane-id={paneId}>
    {#if $nav.canGoBack}
      <Button variant="ghost" size="sm" onclick={() => nav.back()}
        >{$currentLocale && t('topbar.back')}</Button
      >
    {/if}
    <nav class="breadcrumb" aria-label={$currentLocale && t('topbar.breadcrumb')}>
      {#each breadcrumbEntries as crumb, i (i)}
        {#if i > 0}<span class="sep">/</span>{/if}
        {#if crumb.collapsed}
          <span class="crumb crumb--ellipsis">{crumb.label}</span>
        {:else if getBreadcrumbPath(crumb.index)}
          <button
            type="button"
            class="crumb crumb--link"
            onclick={() => navigateToBreadcrumb(crumb.index)}
          >
            {crumb.label}
          </button>
        {:else}
          <span class="crumb crumb--current" aria-current="page">{crumb.label}</span>
        {/if}
      {/each}
    </nav>
    {#if currentView.name === 'item'}
      <span class="crumb-nav">
        <IconButton
          size="sm"
          variant="ghost"
          label={t('topbar.previousDocument')}
          disabled={!previousItem}
          onclick={() => navigateToSibling(previousItem)}
        >
          <ActionIcon name="chevron-left" size={16} />
        </IconButton>
        <IconButton
          size="sm"
          variant="ghost"
          label={t('topbar.nextDocument')}
          disabled={!nextItem}
          onclick={() => navigateToSibling(nextItem)}
        >
          <ActionIcon name="chevron-right" size={16} />
        </IconButton>
      </span>
    {/if}
    {#if currentView.name === 'item' && currentView.assetId && currentView.assetLabel}
      <IconButton
        size="sm"
        variant="ghost"
        label={deleteAssetAria}
        disabled={deletingAsset}
        onclick={openDeleteAssetConfirm}
      >
        <ActionIcon name="delete" size={16} />
      </IconButton>
    {/if}
  </div>

  <div class="work-pane__body">
    {#if currentViewName === 'collections'}
      <CollectionsView />
    {:else if currentViewName === 'home'}
      <HomeView />
    {:else if routeLoad.status === 'loading'}
      <div class="route-state">
        <section class="startup-card startup-card--compact" role="status" aria-live="polite">
          <img class="startup-mark" src={startupMark} alt="" />
          <p>{t('app.initializing')}</p>
        </section>
      </div>
    {:else if routeLoad.status === 'ready'}
      {@const RouteView = routeLoad.module.default}
      {#if currentViewName === 'collection'}
        <RouteView collectionId={currentCollectionId!} />
      {:else if currentViewName === 'item'}
        <RouteView itemId={currentItemId!} collectionId={currentCollectionId!} />
      {:else if currentViewName === 'investigation'}
        <RouteView
          jobId={(currentView as Extract<View, { name: 'investigation' }>).jobId}
          title={(currentView as Extract<View, { name: 'investigation' }>).title}
        />
      {:else}
        <RouteView />
      {/if}
    {:else}
      {@const routeError = routeLoad.error}
      <div class="route-state">
        <section class="startup-card startup-card--error" role="alert" aria-live="assertive">
          <div class="startup-copy">
            <h2>{t('app.initError')}</h2>
            <p>{routeError instanceof Error ? routeError.message : t('app.initError')}</p>
          </div>
          <button type="button" class="startup-action" onclick={retryRouteLoad}
            >{t('app.retryInit')}</button
          >
        </section>
      </div>
    {/if}
  </div>
</div>

{#if showDeleteAssetConfirm && pendingDeleteAssetView}
  <ConfirmDialog
    title={t('collection.deleteAssetTitle')}
    titleId="workpane-{paneId}-delete-asset-title"
    message={t('collection.deleteAssetMessage', { name: pendingDeleteAssetView.assetLabel ?? '' }) +
      (deleteAssetCitations.length > 0
        ? ' ' +
          t('collection.deleteAssetCited', {
            count: deleteAssetCitations.length,
            documents: deleteAssetCitations.map((d) => d.document_title).join(', '),
          })
        : '')}
    error={deleteAssetError}
    cancelLabel={t('collections.cancel')}
    confirmIcon="delete"
    confirmAriaLabel={t('collection.deleteAssetAria')}
    confirmTitle={deletingAsset
      ? t('collection.deletingAssetTitle')
      : t('collection.deleteAssetAria')}
    variant="destructive"
    confirming={deletingAsset}
    cancelDisabled={deletingAsset}
    oncancel={closeDeleteAssetConfirm}
    onconfirm={handleDeleteAssetConfirm}
  />
{/if}

<style>
  /* Ports TopBar.svelte's breadcrumb/crumb/crumb-nav rule set (original
     lines 968-1055) renamed to this component's own class names below, plus
     App.svelte's route-state/startup-card rules (original lines 224-267)
     verbatim under the same class names — both carried over unchanged, only
     the selectors' file moved. New rules specific to this component: */
  .work-pane {
    display: flex;
    flex-direction: column;
    min-width: 320px;
    min-height: 0;
    flex: 1;
  }

  .location-strip {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border-bottom: 1px solid var(--border-subtle);
  }

  .work-pane__body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .breadcrumb {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }

  .crumb {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .crumb--link {
    appearance: none;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .crumb--link:hover {
    color: var(--color-text-primary);
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  .crumb--link:focus-visible {
    outline: none;
    border-radius: var(--radius-sm);
    box-shadow: var(--focus-ring);
  }

  .crumb--current {
    color: var(--color-text-primary);
    font-weight: var(--font-weight-medium);
  }

  .crumb--ellipsis {
    color: var(--color-text-muted);
  }

  .sep {
    color: var(--color-text-muted);
  }

  .crumb-nav {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--color-text-muted);
  }

  .route-state {
    display: grid;
    place-items: center;
    min-height: 100%;
    padding-block: var(--space-5);
  }

  .startup-card {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    gap: var(--space-4);
    width: min(100%, 440px);
    padding: var(--space-5);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
    box-shadow: var(--shadow-surface);
  }

  .startup-card--compact {
    width: auto;
    max-width: 100%;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    grid-template-columns: auto auto;
  }

  .startup-card--compact .startup-mark {
    width: 20px;
    height: 20px;
  }

  .startup-card--compact p {
    color: var(--color-text-secondary);
  }

  .startup-card--error {
    border-color: color-mix(in srgb, var(--color-danger) 32%, var(--color-hairline));
  }

  .startup-mark {
    display: block;
    width: 44px;
    height: 44px;
    object-fit: contain;
  }

  .startup-copy {
    display: grid;
    gap: var(--space-1);
  }

  .startup-action {
    grid-column: 2;
    justify-self: start;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-accent) 22%, var(--color-hairline));
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--color-accent) 14%, var(--color-surface-glass));
    color: var(--color-text-primary);
    cursor: pointer;
    transition:
      background-color var(--transition-smooth),
      border-color var(--transition-smooth);
  }

  .startup-action:hover {
    border-color: color-mix(in srgb, var(--color-accent) 40%, var(--color-hairline));
    background: color-mix(in srgb, var(--color-accent) 20%, var(--color-surface-glass));
  }

  .startup-action:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
</style>
