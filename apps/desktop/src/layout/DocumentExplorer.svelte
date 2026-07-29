<script lang="ts">
  import { onMount } from 'svelte'
  import { getStore } from '$lib/db'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import { navigation, type View } from '$lib/navigation'
  import { getAssetPathLabel } from '$lib/item-metadata'
  import { ActionIcon, type ActionIconName } from '@entropia/ui'
  import {
    DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
    DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT,
    type DocumentExplorerAssetDetail,
    type DocumentExplorerCollectionChangedDetail,
  } from '$lib/document-explorer'
  import type { Asset, Collection, Item, CollectionItemCardSummary } from '@entropia/store'

  type ItemAssetSummary = Pick<
    CollectionItemCardSummary,
    'assetCount' | 'primaryAssetId' | 'primaryAssetPath' | 'primaryAssetType'
  >

  let { filterText = '' }: { filterText?: string } = $props()

  const TREE_STORAGE_KEY = 'entropia-document-explorer-tree'
  const WIDTH_STORAGE_KEY = 'entropia-document-explorer-width'
  const DEFAULT_WIDTH = 244
  const MIN_WIDTH = 220
  const MAX_WIDTH = Math.round(DEFAULT_WIDTH * 1.1)
  const MAX_RESTORED_OPEN_COLLECTIONS = 16
  const MAX_RESTORED_OPEN_ITEMS = 16
  const TREE_VISUAL_LEVEL = {
    collection: 0,
    item: 1,
    asset: 2,
  } as const

  const pendingItemLoads = new Map<string, Promise<void>>()
  const pendingAssetLoads = new Map<string, Promise<void>>()

  let collectionsRequest: Promise<void> | null = null
  let loading = $state(false)
  let loadError = $state<string | null>(null)
  let collections = $state<Collection[]>([])
  const filteredCollections = $derived(
    filterText
      ? collections.filter((c) => c.name.toLowerCase().includes(filterText.toLowerCase()))
      : collections,
  )
  let itemsByCollection = $state<Record<string, Item[]>>({})
  let assetsByItem = $state<Record<string, Asset[]>>({})
  let assetSummariesByItem = $state<Record<string, ItemAssetSummary>>({})
  let itemCounts = $state<Record<string, number>>({})
  let loadingCollections = $state<string[]>([])
  let loadingItems = $state<string[]>([])
  let openCollections = $state<string[]>([])
  let openItems = $state<string[]>([])
  let activeAssetId = $state<string | null>(null)
  let explorerWidth = $state(DEFAULT_WIDTH)
  const currentLocale = locale
  const translateExplorer = (key: string) => t(key as I18nKey)

  function clampExplorerWidth(value: number) {
    return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(value)))
  }

  function readPersistedWidth() {
    try {
      const stored = Number(localStorage.getItem(WIDTH_STORAGE_KEY))
      if (Number.isFinite(stored)) return clampExplorerWidth(stored)
    } catch {}

    return DEFAULT_WIDTH
  }

  function persistExplorerWidth(value: number) {
    try {
      localStorage.setItem(WIDTH_STORAGE_KEY, String(clampExplorerWidth(value)))
    } catch {}
  }

  function startResize(event: PointerEvent) {
    event.preventDefault()
    const startX = event.clientX
    const startWidth = explorerWidth

    const handleMove = (moveEvent: PointerEvent) => {
      explorerWidth = clampExplorerWidth(startWidth + moveEvent.clientX - startX)
    }

    const handleUp = () => {
      persistExplorerWidth(explorerWidth)
      window.removeEventListener('pointermove', handleMove)
      window.removeEventListener('pointerup', handleUp)
    }

    window.addEventListener('pointermove', handleMove)
    window.addEventListener('pointerup', handleUp, { once: true })
  }

  function uniqueIds(ids: string[]) {
    return [...new Set(ids)]
  }

  function readPersistedTreeState() {
    try {
      const stored = localStorage.getItem(TREE_STORAGE_KEY)
      if (!stored) {
        return { collections: [], items: [] }
      }

      const parsed = JSON.parse(stored) as {
        collections?: unknown
        items?: unknown
      }

      return {
        collections: Array.isArray(parsed.collections)
          ? parsed.collections.filter((entry): entry is string => typeof entry === 'string')
          : [],
        items: Array.isArray(parsed.items)
          ? parsed.items.filter((entry): entry is string => typeof entry === 'string')
          : [],
      }
    } catch {
      return { collections: [], items: [] }
    }
  }

  function persistTreeState(collectionIds: string[], itemIds: string[]) {
    try {
      localStorage.setItem(
        TREE_STORAGE_KEY,
        JSON.stringify({
          collections: uniqueIds(collectionIds),
          items: uniqueIds(itemIds),
        })
      )
    } catch {}
  }

  function limitRestoredIds(ids: string[], limit: number, requiredId: string | null) {
    const limitedIds = uniqueIds(ids).slice(-limit)
    return requiredId ? uniqueIds([...limitedIds, requiredId]) : limitedIds
  }

  function limitRestoredTreeState(
    persistedTree: { collections: string[]; items: string[] },
    currentView: View
  ) {
    return {
      collections: limitRestoredIds(
        persistedTree.collections,
        MAX_RESTORED_OPEN_COLLECTIONS,
        getActiveCollectionId(currentView)
      ),
      items: limitRestoredIds(
        persistedTree.items,
        MAX_RESTORED_OPEN_ITEMS,
        getActiveItemId(currentView)
      ),
    }
  }

  function commitTreeState(nextCollections: string[], nextItems: string[]) {
    openCollections = uniqueIds(nextCollections)
    openItems = uniqueIds(nextItems)
    persistTreeState(openCollections, openItems)
  }

  function getToggleLabel(kind: 'collection' | 'item', expanded: boolean, label: string) {
    const localeValue = $currentLocale

    if (localeValue === 'en') {
      return `${expanded ? 'Collapse' : 'Expand'} ${kind === 'collection' ? 'collection' : 'document'} ${label}`
    }

    return `${expanded ? 'Colapsar' : 'Expandir'} ${kind === 'collection' ? 'colección' : 'documento'} ${label}`
  }

  function getActiveCollectionId(view: View): string | null {
    if (view.name === 'collection') return view.id
    if (view.name === 'item') return view.collectionId
    return null
  }

  function getActiveItemId(view: View): string | null {
    return view.name === 'item' ? view.itemId : null
  }

  function getAssetLabel(asset: Asset, index: number): string {
    const fileName = getAssetPathLabel(asset.path).trim()
    if (fileName) return fileName
    return `${asset.type.toUpperCase()} ${index + 1}`
  }

  function getAssetIcon(assetType: Asset['type']): ActionIconName {
    if (assetType === 'audio') return 'file-audio'
    if (assetType === 'pdf') return 'file-text'
    if (assetType === 'image') return 'file-image'
    return 'file'
  }

  function leafAssetsOf(assets: Asset[]): Asset[] {
    // Viewable assets are leaves: standalone assets plus page children. A
    // parent that owns page children is just a container and is not shown or
    // counted as an independent asset.
    const parentIds = new Set(
      assets.filter((a) => a.parentAssetId).map((a) => a.parentAssetId as string),
    )
    return assets.filter((asset) => !parentIds.has(asset.id))
  }

  function getItemAssetSummary(itemId: string) {
    const assets = assetsByItem[itemId]
    if (assets) {
      const leaves = leafAssetsOf(assets)
      const [primaryAsset] = leaves
      return {
        assetCount: leaves.length,
        primaryAssetId: primaryAsset?.id ?? null,
        primaryAssetPath: primaryAsset?.path ?? null,
        primaryAssetType: primaryAsset?.type ?? null,
      }
    }

    return assetSummariesByItem[itemId] ?? null
  }

  function getSingleAssetForItem(item: Item): Asset | null {
    const summary = getItemAssetSummary(item.id)
    if (summary?.assetCount !== 1) return null
    if (!summary.primaryAssetId || !summary.primaryAssetPath || !summary.primaryAssetType) return null

    return {
      id: summary.primaryAssetId,
      itemId: item.id,
      path: summary.primaryAssetPath,
      type: summary.primaryAssetType,
      size: 0,
      sortIndex: 0,
      createdAt: item.createdAt,
    }
  }

  function canExpandItem(item: Item) {
    const summary = getItemAssetSummary(item.id)
    return summary ? summary.assetCount > 1 : true
  }

  function cacheItemAssetSummaries(summaries: CollectionItemCardSummary[]) {
    assetSummariesByItem = {
      ...assetSummariesByItem,
      ...Object.fromEntries(
        summaries.map((summary) => [
          summary.id,
          {
            assetCount: summary.assetCount,
            primaryAssetId: summary.primaryAssetId,
            primaryAssetPath: summary.primaryAssetPath,
            primaryAssetType: summary.primaryAssetType,
          },
        ])
      ),
    }
  }

  function isCollectionExpanded(collectionId: string) {
    return openCollections.includes(collectionId)
  }

  function isItemExpanded(itemId: string) {
    return openItems.includes(itemId)
  }

  function isCollectionLoading(collectionId: string) {
    return loadingCollections.includes(collectionId)
  }

  function isItemLoading(itemId: string) {
    return loadingItems.includes(itemId)
  }

  async function ensureCollectionsLoaded() {
    if (collections.length > 0) return
    if (collectionsRequest) {
      await collectionsRequest
      return
    }

    collectionsRequest = (async () => {
      loading = true
      loadError = null

      try {
        const store = getStore()
        const loadedCollections = await store.collections.findAll()
        const countEntries = await Promise.all(
          loadedCollections.map(async (collection) => {
            const count = await store.collections.countItems(collection.id)
            return [collection.id, count] as const
          })
        )

        collections = loadedCollections
        itemCounts = Object.fromEntries(countEntries)
      } catch (error) {
        loadError = error instanceof Error ? error.message : translateExplorer('explorer.loadError')
      } finally {
        loading = false
        collectionsRequest = null
      }
    })()

    await collectionsRequest
  }

  async function ensureCollectionItemsLoaded(collectionId: string) {
    if (itemsByCollection[collectionId]) return
    const pending = pendingItemLoads.get(collectionId)
    if (pending) {
      await pending
      return
    }

    const request = (async () => {
      loadingCollections = uniqueIds([...loadingCollections, collectionId])
      loadError = null

      try {
        const summaries = await getStore().items.findCardSummariesByCollection(collectionId)
        cacheItemAssetSummaries(summaries)
        itemsByCollection = {
          ...itemsByCollection,
          [collectionId]: summaries,
        }
      } catch (error) {
        loadError = error instanceof Error ? error.message : translateExplorer('explorer.loadError')
      } finally {
        loadingCollections = loadingCollections.filter((entry) => entry !== collectionId)
        pendingItemLoads.delete(collectionId)
      }
    })()

    pendingItemLoads.set(collectionId, request)
    await request
  }

  async function refreshCollection(collectionId: string, itemId?: string) {
    const pending = pendingItemLoads.get(collectionId)
    if (pending) await pending

    loadingCollections = uniqueIds([...loadingCollections, collectionId])
    loadError = null

    try {
      const store = getStore()
      const [summaries, count] = await Promise.all([
        store.items.findCardSummariesByCollection(collectionId),
        store.collections.countItems(collectionId),
      ])
      const items = summaries
      const itemIds = new Set(items.map((item) => item.id))
      const previousItemIds = itemsByCollection[collectionId]?.map((item) => item.id) ?? []
      const nextAssetsByItem = { ...assetsByItem }
      const nextAssetSummariesByItem = { ...assetSummariesByItem }

      for (const previousItemId of previousItemIds) {
        if (previousItemId === itemId || !itemIds.has(previousItemId)) {
          delete nextAssetsByItem[previousItemId]
          delete nextAssetSummariesByItem[previousItemId]
        }
      }

      for (const summary of summaries) {
        nextAssetSummariesByItem[summary.id] = {
          assetCount: summary.assetCount,
          primaryAssetId: summary.primaryAssetId,
          primaryAssetPath: summary.primaryAssetPath,
          primaryAssetType: summary.primaryAssetType,
        }
      }

      itemsByCollection = {
        ...itemsByCollection,
        [collectionId]: items,
      }
      itemCounts = {
        ...itemCounts,
        [collectionId]: count,
      }
      assetsByItem = nextAssetsByItem
      assetSummariesByItem = nextAssetSummariesByItem
    } catch (error) {
      loadError = error instanceof Error ? error.message : translateExplorer('explorer.loadError')
    } finally {
      loadingCollections = loadingCollections.filter((entry) => entry !== collectionId)
    }
  }

  async function ensureItemAssetsLoaded(itemId: string) {
    if (assetsByItem[itemId]) return
    const pending = pendingAssetLoads.get(itemId)
    if (pending) {
      await pending
      return
    }

    const request = (async () => {
      loadingItems = uniqueIds([...loadingItems, itemId])
      loadError = null

      try {
        const assets = await getStore().assets.findByItem(itemId)
        assetsByItem = {
          ...assetsByItem,
          [itemId]: assets,
        }
      } catch (error) {
        loadError = error instanceof Error ? error.message : translateExplorer('explorer.loadError')
      } finally {
        loadingItems = loadingItems.filter((entry) => entry !== itemId)
        pendingAssetLoads.delete(itemId)
      }
    })()

    pendingAssetLoads.set(itemId, request)
    await request
  }

  async function toggleCollectionExpanded(collection: Collection) {
    const expanded = isCollectionExpanded(collection.id)
    if (expanded) {
      const collectionItemIds = (itemsByCollection[collection.id] ?? []).map((item) => item.id)
      commitTreeState(
        openCollections.filter((entry) => entry !== collection.id),
        openItems.filter((itemId) => !collectionItemIds.includes(itemId))
      )
      return
    }

    commitTreeState([...openCollections, collection.id], openItems)
    await ensureCollectionItemsLoaded(collection.id)
  }

  async function toggleItemExpanded(item: Item) {
    const expanded = isItemExpanded(item.id)
    if (expanded) {
      commitTreeState(
        openCollections,
        openItems.filter((entry) => entry !== item.id)
      )
      return
    }

    commitTreeState([...openCollections, item.collectionId], [...openItems, item.id])
    await ensureItemAssetsLoaded(item.id)
  }

  function handleCollectionClick(collection: Collection) {
    const current = $navigation.current
    if (current.name === 'collection' && current.id === collection.id) return

    if (current.name === 'item' && current.collectionId === collection.id) {
      navigation.replace({
        name: 'collection',
        id: collection.id,
        collectionName: collection.name,
      })
      return
    }

    if (
      (current.name === 'collection' && current.id !== collection.id) ||
      (current.name === 'item' && current.collectionId !== collection.id)
    ) {
      navigation.resetToPath([
        { name: 'collections' },
        {
          name: 'collection',
          id: collection.id,
          collectionName: collection.name,
        },
      ])
      return
    }

    navigation.replace({
      name: 'collection',
      id: collection.id,
      collectionName: collection.name,
    })
  }

  function handleItemClick(item: Item, asset?: { id: string; label: string }) {
    const current = $navigation.current
    const collection = collections.find((entry) => entry.id === item.collectionId)
    const collectionName = collection?.name ?? ''
    const nextView = {
      name: 'item' as const,
      collectionId: item.collectionId,
      collectionName,
      itemId: item.id,
      itemTitle: item.title,
      ...(asset ? { assetId: asset.id, assetLabel: asset.label } : {}),
    }

    if (current.name === 'item' && current.itemId === item.id) {
      if (current.assetId === nextView.assetId && current.assetLabel === nextView.assetLabel) return
      navigation.replace(nextView)
      return
    }
    if (current.name === 'item' && current.collectionId === item.collectionId) {
      navigation.replace(nextView)
      return
    }

    if (
      (current.name === 'collection' && current.id !== item.collectionId) ||
      (current.name === 'item' && current.collectionId !== item.collectionId)
    ) {
      navigation.resetToPath([
        { name: 'collections' },
        {
          name: 'collection',
          id: item.collectionId,
          collectionName,
        },
        nextView,
      ])
      return
    }

    navigation.navigate(nextView)
  }

  function handleAssetClick(asset: Asset, index = 0) {
    const item = Object.values(itemsByCollection)
      .flat()
      .find((entry) => entry.id === asset.itemId)
    const assetLabel = getAssetLabel(asset, index)

    if (item) {
      handleItemClick(item, { id: asset.id, label: assetLabel })
    }

    activeAssetId = asset.id
  }

  function handleSingleAssetItemClick(item: Item, asset: Asset) {
    handleItemClick(item, { id: asset.id, label: getAssetLabel(asset, 0) })
    activeAssetId = asset.id
  }

  const activeCollectionId = $derived(getActiveCollectionId($navigation.current))
  const activeItemId = $derived(getActiveItemId($navigation.current))

  $effect(() => {
    const currentView = $navigation.current
    const nextActiveCollectionId = getActiveCollectionId(currentView)
    const nextActiveItemId = getActiveItemId(currentView)

    void ensureCollectionsLoaded()

    if (!nextActiveItemId) {
      activeAssetId = null
    } else if (currentView.name === 'item' && currentView.assetId) {
      activeAssetId = currentView.assetId
    }

    if (nextActiveCollectionId) {
      void ensureCollectionItemsLoaded(nextActiveCollectionId)
    }

    if (nextActiveItemId) {
      void ensureItemAssetsLoaded(nextActiveItemId)
    }
  })

  $effect(() => {
    collections
    for (const collectionId of openCollections) {
      void ensureCollectionItemsLoaded(collectionId)
    }
  })

  $effect(() => {
    itemsByCollection
    for (const itemId of openItems) {
      void ensureItemAssetsLoaded(itemId)
    }
  })

  onMount(() => {
    explorerWidth = readPersistedWidth()
    const persistedTree = limitRestoredTreeState(readPersistedTreeState(), $navigation.current)
    openCollections = persistedTree.collections
    openItems = persistedTree.items

    const handleAssetSelected = (event: Event) => {
      const detail = (event as CustomEvent<DocumentExplorerAssetDetail>).detail
      if (detail.itemId === activeItemId) {
        activeAssetId = detail.assetId
      }
    }

    const handleCollectionChanged = (event: Event) => {
      const detail = (event as CustomEvent<DocumentExplorerCollectionChangedDetail>).detail
      if (!detail?.collectionId) return
      void refreshCollection(detail.collectionId, detail.itemId)
    }

    window.addEventListener(DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT, handleAssetSelected)
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handleCollectionChanged)

    return () => {
      window.removeEventListener(DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT, handleAssetSelected)
      window.removeEventListener(
        DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
        handleCollectionChanged
      )
    }
  })
</script>

<aside
  class="explorer"
  style:width={`${explorerWidth}px`}
  aria-label={$currentLocale && translateExplorer('explorer.aria')}
>
  <div id="document-explorer-panel" class="explorer__panel">
      <div id="document-explorer-content" class="explorer__scroll">
        {#if loadError}
          <p class="explorer__message explorer__message--error">{loadError}</p>
        {:else if loading}
          <p class="explorer__message">{$currentLocale && translateExplorer('explorer.loading')}</p>
        {:else if filteredCollections.length === 0}
          <p class="explorer__message">
            {filterText
              ? `Sin resultados para "${filterText}"`
              : ($currentLocale && translateExplorer('explorer.emptyCollections'))}
          </p>
        {:else}
          <section
            class="explorer__section"
            aria-label={$currentLocale && translateExplorer('explorer.collections')}
          >
            <div class="explorer__section-label">
              {$currentLocale && translateExplorer('explorer.collections')}
            </div>

            <div
              class="explorer__tree"
              role="tree"
              aria-label={$currentLocale && translateExplorer('explorer.aria')}
            >
              {#each filteredCollections as collection (collection.id)}
                {@const collectionExpanded = isCollectionExpanded(collection.id)}
                {@const collectionItems = itemsByCollection[collection.id] ?? []}
                <div
                  class="explorer__treeitem"
                  class:is-active={collection.id === activeCollectionId}
                  role="treeitem"
                  aria-level="1"
                  aria-expanded={collectionExpanded}
                  aria-selected={collection.id === activeCollectionId}
                  aria-current={collection.id === activeCollectionId ? 'true' : undefined}
                  aria-label={collection.name}
                >
                  <div
                    class="explorer__row explorer__row--collection"
                    style:--tree-level={TREE_VISUAL_LEVEL.collection}
                  >
                    <button
                      type="button"
                      class="explorer__chevron"
                      aria-label={getToggleLabel('collection', collectionExpanded, collection.name)}
                      aria-expanded={collectionExpanded}
                      onclick={() => toggleCollectionExpanded(collection)}
                    >
                      <span
                        class:explorer__chevron-icon--open={collectionExpanded}
                        class="explorer__chevron-icon"
                      >
                        <ActionIcon name="chevron-right" size={12} />
                      </span>
                    </button>

                    <button
                      type="button"
                      class="explorer__node explorer__node--collection"
                      class:is-active={collection.id === activeCollectionId}
                      onclick={() => handleCollectionClick(collection)}
                    >
                      <span
                        class="explorer__node-icon explorer__node-icon--collection"
                        aria-hidden="true"
                      >
                        <ActionIcon name="folder" size={14} />
                      </span>
                      <span class="explorer__node-main">{collection.name}</span>
                      <span class="explorer__node-meta">{itemCounts[collection.id] ?? 0}</span>
                    </button>
                  </div>

                  {#if collectionExpanded}
                    <div class="explorer__group" role="group">
                      {#if isCollectionLoading(collection.id)}
                        <p
                          class="explorer__message explorer__message--nested"
                          style:--tree-level={TREE_VISUAL_LEVEL.item}
                        >
                          {$currentLocale && translateExplorer('explorer.loading')}
                        </p>
                      {:else if collectionItems.length === 0}
                        <p
                          class="explorer__message explorer__message--nested"
                          style:--tree-level={TREE_VISUAL_LEVEL.item}
                        >
                          {$currentLocale && translateExplorer('explorer.emptyDocuments')}
                        </p>
                      {:else}
                        {#each collectionItems as item (item.id)}
                          {@const itemExpanded = isItemExpanded(item.id)}
                          {@const itemAssets = leafAssetsOf(assetsByItem[item.id] ?? [])}
                          {@const singleAsset = getSingleAssetForItem(item)}
                          {@const itemExpandable = canExpandItem(item)}
                          {#if singleAsset}
                            <div
                              class="explorer__treeitem"
                              class:is-active={item.id === activeItemId || singleAsset.id === activeAssetId}
                              role="treeitem"
                              aria-level="2"
                              aria-selected={item.id === activeItemId || singleAsset.id === activeAssetId}
                              aria-current={item.id === activeItemId || singleAsset.id === activeAssetId
                                ? 'true'
                                : undefined}
                              aria-label={item.title}
                            >
                              <div
                                class="explorer__row explorer__row--item"
                                style:--tree-level={TREE_VISUAL_LEVEL.item}
                              >
                                <span class="explorer__chevron-spacer" aria-hidden="true"></span>
                                <button
                                  type="button"
                                  class="explorer__node explorer__node--item explorer__node--flush"
                                  class:is-active={item.id === activeItemId || singleAsset.id === activeAssetId}
                                  aria-label={item.title}
                                  onclick={() => handleSingleAssetItemClick(item, singleAsset)}
                                >
                                  <span
                                    class="explorer__node-icon explorer__node-icon--item"
                                    aria-hidden="true"
                                  >
                                    <ActionIcon name={getAssetIcon(singleAsset.type)} size={14} />
                                  </span>
                                  <span class="explorer__node-main">{item.title}</span>
                                  <span class="explorer__asset-type">{singleAsset.type}</span>
                                </button>
                              </div>
                            </div>
                          {:else if itemExpandable}
                          <div
                            class="explorer__treeitem"
                            class:is-active={item.id === activeItemId}
                            role="treeitem"
                            aria-level="2"
                            aria-expanded={itemExpanded}
                             aria-selected={item.id === activeItemId}
                             aria-current={item.id === activeItemId ? 'true' : undefined}
                             aria-label={item.title}
                           >
                            <div
                              class="explorer__row explorer__row--item"
                              style:--tree-level={TREE_VISUAL_LEVEL.item}
                            >
                               <button
                                 type="button"
                                 class="explorer__chevron"
                                aria-label={getToggleLabel('item', itemExpanded, item.title)}
                                aria-expanded={itemExpanded}
                                onclick={() => toggleItemExpanded(item)}
                              >
                                <span
                                  class:explorer__chevron-icon--open={itemExpanded}
                                  class="explorer__chevron-icon"
                                >
                                  <ActionIcon name="chevron-right" size={12} />
                                </span>
                              </button>

                              <button
                                type="button"
                                class="explorer__node explorer__node--item"
                                class:is-active={item.id === activeItemId}
                                onclick={() => handleItemClick(item)}
                              >
                                <span
                                  class="explorer__node-icon explorer__node-icon--item"
                                  aria-hidden="true"
                                >
                                  <ActionIcon name="file-text" size={14} />
                                </span>
                                <span class="explorer__node-main">{item.title}</span>
                              </button>
                            </div>

                            {#if itemExpanded}
                              <div class="explorer__group" role="group">
                                {#if isItemLoading(item.id)}
                                  <p
                                    class="explorer__message explorer__message--nested"
                                    style:--tree-level={TREE_VISUAL_LEVEL.asset}
                                  >
                                    {$currentLocale && translateExplorer('explorer.loading')}
                                  </p>
                                {:else if itemAssets.length === 0}
                                  <p
                                    class="explorer__message explorer__message--nested"
                                    style:--tree-level={TREE_VISUAL_LEVEL.asset}
                                  >
                                    {$currentLocale && translateExplorer('explorer.emptyAssets')}
                                  </p>
                                {:else}
                                  {#each itemAssets as asset, index (asset.id)}
                                    {@const assetIcon = getAssetIcon(asset.type)}
                                    <div
                                      class="explorer__treeitem"
                                      role="treeitem"
                                      aria-level="3"
                                       aria-selected={asset.id === activeAssetId}
                                       aria-current={asset.id === activeAssetId ? 'true' : undefined}
                                       aria-label={getAssetLabel(asset, index)}
                                     >
                                      <div
                                        class="explorer__row explorer__row--asset"
                                        style:--tree-level={TREE_VISUAL_LEVEL.asset}
                                      >
                                        <span class="explorer__chevron-spacer" aria-hidden="true"></span>
                                        <button
                                          type="button"
                                          class="explorer__node explorer__node--asset"
                                          class:is-active={asset.id === activeAssetId}
                                          onclick={() => handleAssetClick(asset, index)}
                                        >
                                          <span
                                            class="explorer__node-icon explorer__node-icon--asset"
                                            aria-hidden="true"
                                          >
                                            <ActionIcon name={assetIcon} size={14} />
                                          </span>
                                          <span class="explorer__node-main"
                                            >{getAssetLabel(asset, index)}</span
                                          >
                                          <span class="explorer__asset-type">{asset.type}</span>
                                        </button>
                                      </div>
                                    </div>
                                  {/each}
                                {/if}
                              </div>
                            {/if}
                          </div>
                          {:else}
                            <div
                              class="explorer__treeitem"
                              class:is-active={item.id === activeItemId}
                              role="treeitem"
                              aria-level="2"
                              aria-selected={item.id === activeItemId}
                              aria-current={item.id === activeItemId ? 'true' : undefined}
                              aria-label={item.title}
                            >
                              <div
                                class="explorer__row explorer__row--item"
                                style:--tree-level={TREE_VISUAL_LEVEL.item}
                              >
                                <span class="explorer__chevron-spacer" aria-hidden="true"></span>
                                <button
                                  type="button"
                                  class="explorer__node explorer__node--item explorer__node--flush"
                                  class:is-active={item.id === activeItemId}
                                  onclick={() => handleItemClick(item)}
                                >
                                  <span
                                    class="explorer__node-icon explorer__node-icon--item"
                                    aria-hidden="true"
                                  >
                                    <ActionIcon name="file-text" size={14} />
                                  </span>
                                  <span class="explorer__node-main">{item.title}</span>
                                </button>
                              </div>
                            </div>
                          {/if}
                        {/each}
                      {/if}
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          </section>
        {/if}
      </div>

      <div
        class="explorer__resize-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label={$currentLocale && translateExplorer('explorer.resize')}
        title={$currentLocale && translateExplorer('explorer.resize')}
        onpointerdown={startResize}
      ></div>
  </div>
</aside>

<style>
  .explorer {
    position: relative;
    display: flex;
    flex: 0 0 auto;
    min-width: 180px;
    max-width: 300px;
    border-right: 1px solid var(--color-border-subtle);
    background: var(--color-surface);
    overflow: hidden;
    font-size: var(--font-size-xs);
  }

  .explorer__panel {
    position: relative;
    display: flex;
    flex: 1;
    min-width: 0;
    flex-direction: column;
  }

  .explorer__resize-handle {
    position: absolute;
    top: 0;
    right: -3px;
    bottom: 0;
    z-index: 4;
    width: 7px;
    cursor: col-resize;
  }

  .explorer__resize-handle::after {
    content: '';
    position: absolute;
    top: 48px;
    right: 3px;
    bottom: 8px;
    width: 1px;
    border-radius: var(--radius-xs);
    background: transparent;
    transition: background-color var(--transition-base);
  }

  .explorer__resize-handle:hover::after {
    background: color-mix(in srgb, var(--color-accent) 44%, transparent);
  }

  .explorer__resize-handle:focus-visible {
    outline: none;
  }

  .explorer__resize-handle:focus-visible::after {
    background: color-mix(in srgb, var(--color-accent) 58%, transparent);
  }

  .explorer__scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 36px 5px 10px 6px;
    scrollbar-color: color-mix(in srgb, var(--color-text-muted) 58%, transparent) transparent;
    scrollbar-width: thin;
  }

  .explorer__scroll::-webkit-scrollbar {
    width: 8px;
  }

  .explorer__scroll::-webkit-scrollbar-track {
    background: transparent;
  }

  .explorer__scroll::-webkit-scrollbar-thumb {
    border: 2px solid transparent;
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--color-text-muted) 52%, transparent);
    background-clip: padding-box;
  }

  .explorer__section-label {
    margin-bottom: 5px;
    padding-left: 5px;
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.075em;
  }

  .explorer__tree {
    --tree-indent-step: 24px;
    --tree-guide-offset: 8px;
    --tree-control-width: 17px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .explorer__treeitem {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .explorer__row {
    position: relative;
    display: grid;
    grid-template-columns: var(--tree-control-width) minmax(0, 1fr);
    align-items: center;
    gap: 3px;
    min-width: 0;
    min-height: 23px;
    padding-left: calc(var(--tree-level, 0) * var(--tree-indent-step));
  }

  .explorer__row::before,
  .explorer__row::after {
    content: '';
    position: absolute;
    pointer-events: none;
  }

  .explorer__row::before {
    left: calc((var(--tree-level, 0) * var(--tree-indent-step)) + var(--tree-guide-offset));
    top: -3px;
    bottom: -3px;
    width: 1px;
    background: color-mix(in srgb, var(--color-border-subtle) 58%, transparent);
  }

  .explorer__row::after {
    left: calc((var(--tree-level, 0) * var(--tree-indent-step)) + var(--tree-guide-offset));
    top: 50%;
    width: 8px;
    height: 1px;
    background: color-mix(in srgb, var(--color-border-subtle) 72%, transparent);
    transform: translateY(-0.5px);
  }

  .explorer__treeitem[aria-level='1'] > .explorer__row::before {
    top: 6px;
    bottom: 6px;
    opacity: 0.48;
  }

  .explorer__treeitem[aria-level='1'] > .explorer__row::after {
    width: 8px;
    opacity: 0.42;
  }

  .explorer__group {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .explorer__chevron-spacer {
    width: var(--tree-control-width);
    height: 20px;
    pointer-events: none;
  }

  .explorer__chevron {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--tree-control-width);
    height: 20px;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
    transition:
      color var(--transition-base),
      background-color var(--transition-base),
      border-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .explorer__chevron:hover {
    color: var(--color-text-primary);
    border-color: color-mix(in srgb, var(--color-border-subtle) 38%, transparent);
    background: var(--color-surface-raised);
  }

  .explorer__chevron:focus-visible,
  .explorer__node:focus-visible {
    outline: none;
    border-color: color-mix(in srgb, var(--color-accent) 44%, transparent);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--color-accent) 22%, transparent),
      0 0 0 3px color-mix(in srgb, var(--color-accent) 12%, transparent);
  }

  .explorer__chevron-icon {
    width: 12px;
    height: 12px;
    display: inline-flex;
    transition: transform var(--transition-base);
  }

  .explorer__chevron-icon--open {
    transform: rotate(90deg);
  }

  .explorer__node {
    position: relative;
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    min-width: 0;
    padding: 3px 6px 3px 7px;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    text-align: left;
    cursor: pointer;
    transition:
      color var(--transition-base),
      border-color var(--transition-base),
      background-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .explorer__node::before {
    content: '';
    position: absolute;
    left: 0;
    top: 4px;
    bottom: 4px;
    width: 1px;
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--color-accent) 70%, white 12%);
    opacity: 0;
    transform: scaleY(0.7);
    transition:
      opacity var(--transition-base),
      transform var(--transition-base);
  }

  .explorer__node:hover {
    color: var(--color-text-primary);
    border-color: transparent;
    background: color-mix(in srgb, var(--color-surface-raised) 48%, transparent);
  }

  .explorer__node.is-active {
    color: var(--color-text-primary);
    border-color: transparent;
    background: color-mix(in srgb, var(--color-text-primary) 10%, var(--color-surface-raised));
    box-shadow:
      inset 0 1px 0 color-mix(in srgb, var(--color-text-primary) 8%, transparent),
      0 0 0 1px color-mix(in srgb, var(--color-hairline) 72%, transparent);
  }

  .explorer__node.is-active .explorer__node-main {
    color: color-mix(in srgb, var(--color-text-primary) 92%, white 8%);
  }

  .explorer__node.is-active::before {
    opacity: 1;
    transform: scaleY(1);
  }

  .explorer__treeitem.is-active > .explorer__row::before {
    background: color-mix(in srgb, var(--color-accent) 32%, transparent);
  }

  .explorer__treeitem.is-active > .explorer__row::after {
    background: color-mix(in srgb, var(--color-accent) 38%, transparent);
  }

  .explorer__treeitem.is-active > .explorer__row > .explorer__chevron {
    color: color-mix(in srgb, var(--color-accent) 42%, var(--color-text-primary));
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
  }

  .explorer__node-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 15px;
    height: 15px;
    flex: 0 0 15px;
    color: color-mix(in srgb, var(--color-text-secondary) 88%, white 12%);
  }

  .explorer__node-icon :global(svg) {
    width: 14px;
    height: 14px;
    overflow: visible;
  }

  .explorer__node-icon--collection,
  .explorer__node-icon--item,
  .explorer__node-icon--asset {
    border-radius: 4px;
    background: transparent;
    box-shadow: none;
  }

  .explorer__node.is-active .explorer__node-icon {
    color: color-mix(in srgb, var(--color-accent) 34%, white 26%);
  }

  .explorer__node-main {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-xs);
    line-height: var(--line-height-tight);
  }

  .explorer__node-meta,
  .explorer__asset-type {
    flex: 0 0 auto;
    font-size: var(--font-size-2xs);
    color: color-mix(in srgb, var(--color-text-muted) 82%, transparent);
    text-transform: uppercase;
    letter-spacing: 0.075em;
  }

  .explorer__node.is-active .explorer__node-meta,
  .explorer__node.is-active .explorer__asset-type {
    color: color-mix(in srgb, var(--color-accent) 42%, var(--color-text-secondary));
  }

  .explorer__asset-type {
    min-width: 0;
  }

  .explorer__message {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .explorer__message--nested {
    padding: 4px 0 5px calc((var(--tree-level, 0) * var(--tree-indent-step)) + 20px);
  }

  .explorer__message--error {
    color: var(--color-danger);
  }

  @media (max-width: 900px) {
    .explorer {
      width: min(70vw, 280px);
      max-width: min(70vw, 280px);
    }
  }
</style>
