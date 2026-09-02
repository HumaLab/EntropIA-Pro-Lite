<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { invoke } from '@tauri-apps/api/core'
  import { remove } from '@tauri-apps/plugin-fs'
  import { navigation, type View } from '$lib/navigation'
  import { getStore } from '$lib/db'
  import {
    deleteAssetFile,
    deleteImageThumbnail,
    deletePdfThumbnail,
  } from '$lib/file-import'
  import { getAssetPathLabel } from '$lib/item-metadata'
  import {
    DOCUMENT_ASSET_DELETED_EVENT,
    DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
    type DocumentAssetDeletedDetail,
    type DocumentExplorerCollectionChangedDetail,
  } from '$lib/document-explorer'
  import { locale, setLocale, t, type Locale } from '$lib/i18n'
  import { resetZoom, zoomFactor, zoomIn, zoomOut, ZOOM_MAX, ZOOM_MIN } from '$lib/zoom'
  import { isCriticalMissing, onCriticalMissingChange } from '$lib/deps'
  import { LOCAL_ML } from '$lib/capabilities'
  import { PRODUCT_NAME } from '$lib/product'
  import { ActionIcon, Button, ConfirmDialog, IconButton, SearchClearButton, StatusBadge } from '@entropia/ui'
  import type { Asset, Collection, Item } from '@entropia/store'

  let hasDepsWarning = $state(isCriticalMissing())
  const unsubDeps = onCriticalMissingChange((v) => { hasDepsWarning = v })

  type AppTheme = 'dark' | 'dim' | 'light'

  const THEME_STORAGE_KEY = 'entropia-theme'

  interface SearchResult {
    item: Item
    collection: Collection
  }

  type ItemNavigationView = Extract<View, { name: 'item' }>

  let searchQuery = $state('')
  let searchResults = $state<SearchResult[]>([])
  let searchError = $state('')
  let showResults = $state(false)
  let searching = $state(false)
  let previousItem = $state<Item | null>(null)
  let nextItem = $state<Item | null>(null)
  let theme = $state<AppTheme>('dark')
  let siblingRequestId = 0
  let searchRequestId = 0
  let debounceTimer: ReturnType<typeof setTimeout> | null = null
  let searchInputEl: HTMLInputElement | undefined = $state()
  let searchContainerEl: HTMLDivElement | undefined = $state()
  let languageContainerEl: HTMLDivElement | undefined = $state()
  let zoomContainerEl: HTMLDivElement | undefined = $state()
  let activeResultIndex = $state(-1)
  let languageMenuOpen = $state(false)
  let zoomMenuOpen = $state(false)
  let showDeleteAssetConfirm = $state(false)
  let deletingAsset = $state(false)
  let deleteAssetError = $state<string | null>(null)
  let pendingDeleteAssetView = $state<ItemNavigationView | null>(null)
  const searchListboxId = 'topbar-global-search-listbox'
  const currentLocale = locale
  const translate = (key: string, params?: Record<string, string | number>) =>
    t(key as never, params)
  const THEME_CYCLE: AppTheme[] = ['dark', 'dim', 'light']
  const themeLabels: Record<AppTheme, string> = {
    dark: 'Oscuro',
    dim: 'Cálido',
    light: 'Claro',
  }
  const themeToggleLabel = $derived(themeLabels[theme])
  const hasResultOptions = $derived(!searching && !searchError && searchResults.length > 0)
  const activeOptionId = $derived(
    showResults && hasResultOptions && activeResultIndex >= 0
      ? `${searchListboxId}-option-${activeResultIndex}`
      : undefined,
  )
  const previousDocumentLabel = $derived($currentLocale ? t('topbar.previousDocument') : 'Documento anterior')
  const nextDocumentLabel = $derived($currentLocale ? t('topbar.nextDocument') : 'Documento siguiente')
  const dbBrowserTitle = $derived($currentLocale ? translate('topbar.dbBrowserTitle') : 'Base de datos')
  const dbBrowserAria = $derived($currentLocale ? translate('topbar.dbBrowserAria') : 'Abrir navegador de base de datos')
  const ragChatTitle = $derived($currentLocale ? translate('topbar.ragChatTitle') : 'Chat de investigación')
  const ragChatAria = $derived($currentLocale ? translate('topbar.ragChatAria') : 'Abrir chat de investigación')
  const settingsTitle = $derived(
    hasDepsWarning
      ? ($currentLocale ? t('topbar.depsWarningTitle') : 'Dependencias de IA pendientes - click para configurar')
      : ($currentLocale ? t('topbar.settingsTitle') : 'Configuración'),
  )
  const settingsAria = $derived(
    hasDepsWarning
      ? ($currentLocale ? t('topbar.depsWarningAria') : 'Dependencias de IA pendientes')
      : ($currentLocale ? t('topbar.settingsAria') : 'Abrir configuración'),
  )
  const languageTitle = $derived($currentLocale ? t('topbar.languageTitle') : 'Idioma')
  const currentZoom = zoomFactor
  const zoomPercent = $derived(Math.round($currentZoom * 100))
  const zoomTitle = $derived($currentLocale ? t('topbar.zoomTitle') : 'Zoom')
  const zoomInLabel = $derived($currentLocale ? t('topbar.zoomIn') : 'Aumentar zoom')
  const zoomOutLabel = $derived($currentLocale ? t('topbar.zoomOut') : 'Reducir zoom')
  const zoomResetLabel = $derived($currentLocale ? t('topbar.zoomReset') : 'Restablecer zoom')
  const zoomHint = $derived($currentLocale ? t('topbar.zoomHint') : 'Ctrl + / Ctrl − / Ctrl 0')
  const zoomLevelAria = $derived(
    $currentLocale
      ? translate('topbar.zoomLevelAria', { value: zoomPercent })
      : `Zoom actual: ${zoomPercent}%`,
  )
  const deleteAssetAria = $derived(
    $currentLocale ? t('topbar.deleteAssetAria') : 'Eliminar asset activo'
  )
  function minimizeWindow() {
    void getCurrentWindow().minimize()
  }

  function toggleMaximizeWindow() {
    void getCurrentWindow().toggleMaximize()
  }

  function closeWindow() {
    void getCurrentWindow().close()
  }

  function toggleLanguageMenu() {
    languageMenuOpen = !languageMenuOpen
  }

  async function chooseLanguage(nextLocale: Locale) {
    languageMenuOpen = false
    await setLocale(nextLocale)
  }

  function handleLanguageFocusOut(event: FocusEvent) {
    const nextFocused = event.relatedTarget
    if (nextFocused instanceof Node && languageContainerEl?.contains(nextFocused)) return
    languageMenuOpen = false
  }

  function toggleZoomMenu() {
    zoomMenuOpen = !zoomMenuOpen
  }

  function handleZoomFocusOut(event: FocusEvent) {
    const nextFocused = event.relatedTarget
    if (nextFocused instanceof Node && zoomContainerEl?.contains(nextFocused)) return
    zoomMenuOpen = false
  }

  function readPersistedTheme(): AppTheme {
    try {
      const stored = localStorage.getItem(THEME_STORAGE_KEY)
      if (stored === 'dim' || stored === 'light') return stored
      return 'dark'
    } catch {
      return 'dark'
    }
  }

  function applyTheme(nextTheme: AppTheme) {
    theme = nextTheme

    if (typeof document !== 'undefined') {
      if (nextTheme === 'dark') {
        delete document.documentElement.dataset.theme
      } else {
        document.documentElement.dataset.theme = nextTheme
      }
    }

    try {
      localStorage.setItem(THEME_STORAGE_KEY, nextTheme)
    } catch {}
  }

  function toggleTheme() {
    const idx = THEME_CYCLE.indexOf(theme)
    const nextTheme = THEME_CYCLE[(idx + 1) % THEME_CYCLE.length] ?? 'dark'
    applyTheme(nextTheme)
  }

  onMount(() => {
    applyTheme(readPersistedTheme())
  })

  onDestroy(() => {
    unsubDeps()
  })

  function buildItemView(item: Item) {
    const currentView = $navigation.current
    if (currentView.name !== 'item') return null

    return {
      name: 'item' as const,
      collectionId: currentView.collectionId,
      collectionName: currentView.collectionName,
      itemId: item.id,
      itemTitle: item.title,
    }
  }

  /**
   * Resolve the previous and next documents.
   *
   * Two indexed single-row queries, not a whole-collection load. Opening one
   * document used to read every row in its collection purely to compute two
   * neighbours, which cost the same as opening the collection itself and grew
   * with it. The `(title, id)` cursor is exactly the ordering the collection
   * grid uses, so the neighbours here are the neighbours there.
   */
  async function loadSiblingItems() {
    const currentView = $navigation.current
    const requestId = ++siblingRequestId

    previousItem = null
    nextItem = null

    if (currentView.name !== 'item') return

    const store = getStore()
    const cursor = { title: currentView.itemTitle, id: currentView.itemId }

    try {
      if (
        typeof store.items.findPreviousCardSummary === 'function' &&
        typeof store.items.findNextCardSummary === 'function'
      ) {
        const [previous, next] = await Promise.all([
          store.items.findPreviousCardSummary(currentView.collectionId, cursor),
          store.items.findNextCardSummary(currentView.collectionId, cursor),
        ])
        if (requestId !== siblingRequestId) return

        previousItem = previous
        nextItem = next
        return
      }

      // A store from before the keyset queries. Kept so an older build still
      // navigates rather than silently losing the controls.
      const items = await store.items.findByCollection(currentView.collectionId)
      if (requestId !== siblingRequestId) return

      const currentIndex = items.findIndex((item) => item.id === currentView.itemId)
      if (currentIndex === -1) return

      previousItem = items[currentIndex - 1] ?? null
      nextItem = items[currentIndex + 1] ?? null
    } catch (error) {
      if (requestId !== siblingRequestId) return
      console.error('[TopBar] Failed to load sibling documents', error)
    }
  }

  function navigateToSibling(item: Item | null) {
    const nextView = item ? buildItemView(item) : null
    if (!nextView) return
    navigation.replace(nextView)
  }

  function getBreadcrumbPath(index: number): [View, ...View[]] | null {
    const currentView = $navigation.current
    const collectionsView: View = { name: 'collections' }

    if (index === 0) {
      return currentView.name === 'collections' ? null : [collectionsView]
    }

    if (currentView.name !== 'item') return null

    const collectionView: View = {
      name: 'collection',
      id: currentView.collectionId,
      collectionName: currentView.collectionName,
    }

    if (index === 1) return [collectionsView, collectionView]
    return null
  }

  function navigateToBreadcrumb(index: number) {
    const path = getBreadcrumbPath(index)
    if (path) navigation.resetToPath(path)
  }

  function leafAssetsOf(assets: Asset[]) {
    const parentIds = new Set(
      assets.filter((asset) => asset.parentAssetId).map((asset) => asset.parentAssetId as string)
    )
    return assets.filter((asset) => !parentIds.has(asset.id))
  }

  function openDeleteAssetConfirm() {
    if (
      $navigation.current.name !== 'item' ||
      !$navigation.current.assetId ||
      !$navigation.current.assetLabel
    ) {
      return
    }
    pendingDeleteAssetView = { ...$navigation.current }
    deleteAssetError = null
    showDeleteAssetConfirm = true
  }

  function closeDeleteAssetConfirm() {
    if (deletingAsset) return
    showDeleteAssetConfirm = false
    deleteAssetError = null
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
          await remove(asset.path.replace(/\.pdf$/i, '.pages'), { recursive: true })
        }
      }
    } catch (error) {
      console.warn('[TopBar] Asset file cleanup warning:', error)
    }
  }

  async function handleDeleteAssetConfirm() {
    const currentView = pendingDeleteAssetView
    if (!currentView?.assetId) return

    deletingAsset = true
    deleteAssetError = null
    const store = getStore()
    const assetId = currentView.assetId
    let deletedIndex = 0

    try {
      const assetsBeforeDelete = leafAssetsOf(await store.assets.findByItem(currentView.itemId))
      deletedIndex = Math.max(0, assetsBeforeDelete.findIndex((asset) => asset.id === assetId))
    } catch (error) {
      console.warn('[TopBar] Failed to load assets before deletion:', error)
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
      remainingAssets = leafAssetsOf(await store.assets.findByItem(currentView.itemId))
    } catch (error) {
      console.warn('[TopBar] Failed to load assets after deletion:', error)
    }

    await cleanupDeletedAssetFile(deletedAsset)

    window.dispatchEvent(
      new CustomEvent<DocumentAssetDeletedDetail>(DOCUMENT_ASSET_DELETED_EVENT, {
        detail: { itemId: currentView.itemId, assetId },
      })
    )
    window.dispatchEvent(
      new CustomEvent<DocumentExplorerCollectionChangedDetail>(
        DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
        { detail: { collectionId: currentView.collectionId, itemId: currentView.itemId } }
      )
    )

    const nextAsset = remainingAssets[Math.min(deletedIndex, remainingAssets.length - 1)] ?? null
    if (nextAsset) {
      navigation.replace({
        ...currentView,
        assetId: nextAsset.id,
        assetLabel: getAssetPathLabel(nextAsset.path),
      })
    } else {
      navigation.resetToPath([
        { name: 'collections' },
        {
          name: 'collection',
          id: currentView.collectionId,
          collectionName: currentView.collectionName,
        },
      ])
    }

    deletingAsset = false
    showDeleteAssetConfirm = false
    pendingDeleteAssetView = null
  }

  $effect(() => {
    $navigation.current
    void loadSiblingItems()
  })

  async function performSearch(query: string, requestId: number) {
    const isCurrentRequest = () => requestId === searchRequestId

    if (!isCurrentRequest()) return

    if (!query.trim()) {
      searchResults = []
      searchError = ''
      showResults = false
      return
    }

    searching = true
    searchError = ''
    try {
      const store = getStore()
      const matchedItems = await store.items.searchGlobal(query, 20)
      if (!isCurrentRequest()) return

      const results: SearchResult[] = []

      // Cache collections to avoid repeated lookups
      const collectionCache = new Map<string, Collection>()
      for (const item of matchedItems) {
        let collection = collectionCache.get(item.collectionId)
        if (!collection) {
          const found = await store.collections.findById(item.collectionId)
          if (!isCurrentRequest()) return
          if (!found) continue
          collection = found
          collectionCache.set(item.collectionId, collection)
        }
        results.push({ item, collection })
      }

      searchResults = results
      searchError = ''
      activeResultIndex = -1
      showResults = true
    } catch (e) {
      if (!isCurrentRequest()) return
      console.error('[Search] error:', e)
      searchResults = []
      searchError = translate('topbar.searchError')
      activeResultIndex = -1
      showResults = true
    } finally {
      if (isCurrentRequest()) searching = false
    }
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer)
  }

  function handleSearchValueChange(query: string, _e: Event) {
    searchQuery = query
    handleInput()
    const requestId = ++searchRequestId

    if (!searchQuery.trim()) {
      searchResults = []
      searchError = ''
      activeResultIndex = -1
      showResults = false
      return
    }

    debounceTimer = setTimeout(() => {
      performSearch(searchQuery, requestId)
    }, 300)
  }

  function handleClear() {
    searchRequestId += 1
    searchQuery = ''
    searchResults = []
    searchError = ''
    activeResultIndex = -1
    showResults = false
    if (debounceTimer) clearTimeout(debounceTimer)
  }

  function handleResultClick(result: SearchResult) {
    navigation.navigate({
      name: 'collection',
      id: result.collection.id,
      collectionName: result.collection.name,
    })
    navigation.navigate({
      name: 'item',
      collectionId: result.collection.id,
      collectionName: result.collection.name,
      itemId: result.item.id,
      itemTitle: result.item.title,
    })
    handleClear()
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      handleClear()
      searchInputEl?.blur()
      return
    }

    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (!showResults || searchResults.length === 0) return
      e.preventDefault()
      const lastIndex = searchResults.length - 1
      if (e.key === 'ArrowDown') {
        activeResultIndex = activeResultIndex >= lastIndex ? 0 : activeResultIndex + 1
      } else {
        activeResultIndex = activeResultIndex <= 0 ? lastIndex : activeResultIndex - 1
      }
      return
    }

    // keyCode 229 cubre WKWebView, donde isComposing puede no reportarse durante IME.
    if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
      if (!showResults) return
      const activeResult = searchResults[activeResultIndex]
      if (!activeResult) return
      e.preventDefault()
      handleResultClick(activeResult)
    }
  }

  function handleFocusOut(event: FocusEvent) {
    const nextFocused = event.relatedTarget
    if (nextFocused instanceof Node && searchContainerEl?.contains(nextFocused)) return
    showResults = false
    activeResultIndex = -1
  }

  function handleFocus() {
    if (searchResults.length > 0 || searchError) {
      showResults = true
    }
  }
</script>

<header class="topbar">
  <div class="topbar__leading">
    <div class="topbar__back-slot">
      {#if $navigation.canGoBack}
        <Button variant="ghost" size="sm" onclick={() => navigation.back()}
          >{$currentLocale && t('topbar.back')}</Button
        >
      {:else}
        <span class="topbar__app-title" data-tauri-drag-region>{PRODUCT_NAME}</span>
      {/if}
    </div>
    <nav class="breadcrumb" aria-label={$currentLocale && t('topbar.breadcrumb')} data-tauri-drag-region>
      {#each $navigation.breadcrumb as crumb, i (i)}
        {#if i > 0}<span class="sep">/</span>{/if}
        {#if getBreadcrumbPath(i)}
          <button class="crumb crumb--link" type="button" onclick={() => navigateToBreadcrumb(i)}>
            {crumb}
          </button>
        {:else if i === $navigation.breadcrumb.length - 1}
          <span class="crumb crumb--current last" aria-current="page" data-tauri-drag-region>
            <span class="crumb__label" data-tauri-drag-region>{crumb}</span>
          </span>
        {:else}
          <span class="crumb" data-tauri-drag-region>{crumb}</span>
        {/if}
      {/each}
    </nav>
    {#if $navigation.current.name === 'item' && $navigation.current.assetId && $navigation.current.assetLabel}
      <IconButton
        class="breadcrumb__delete"
        size="sm"
        variant="ghost"
        label={deleteAssetAria}
        title={deleteAssetAria}
        disabled={deletingAsset}
        onclick={openDeleteAssetConfirm}
      >
        <ActionIcon name="delete" size={16} />
      </IconButton>
    {/if}
  </div>

  <div class="topbar__center" class:topbar__center--inactive={$navigation.current.name !== 'item'}>
    {#if $navigation.current.name === 'item'}
      <span class="crumb-nav" aria-label={$currentLocale && t('topbar.breadcrumb')}>
        <IconButton
          class="crumb-nav__button"
          size="sm"
          variant="ghost"
          label={previousDocumentLabel}
          title={previousDocumentLabel}
          disabled={!previousItem}
          onclick={() => navigateToSibling(previousItem)}
        >
          <ActionIcon name="chevron-left" size={16} />
        </IconButton>
        <span class="crumb-nav__separator" aria-hidden="true">|</span>
        <IconButton
          class="crumb-nav__button"
          size="sm"
          variant="ghost"
          label={nextDocumentLabel}
          title={nextDocumentLabel}
          disabled={!nextItem}
          onclick={() => navigateToSibling(nextItem)}
        >
          <ActionIcon name="chevron-right" size={16} />
        </IconButton>
      </span>
    {/if}
  </div>

  <div class="global-search" bind:this={searchContainerEl} onfocusout={handleFocusOut}>
    <div class="global-search__input-wrap">
      <input
        class="global-search__input"
        type="text"
        role="combobox"
        aria-expanded={showResults}
        aria-controls={searchListboxId}
        aria-autocomplete="list"
        aria-activedescendant={activeOptionId}
        bind:value={searchQuery}
        bind:this={searchInputEl}
        placeholder={$currentLocale && translate('topbar.searchPlaceholder')}
        aria-label={$currentLocale && translate('topbar.searchAria')}
        oninput={(event: Event) =>
          handleSearchValueChange((event.currentTarget as HTMLInputElement).value, event)}
        onkeydown={handleKeydown}
        onfocus={handleFocus}
      />

      {#if searchQuery}
        <SearchClearButton
          class="search-clear-button--overlay"
          label={$currentLocale ? translate('topbar.searchClear') : 'Limpiar búsqueda'}
          title={$currentLocale ? translate('topbar.searchClear') : 'Limpiar búsqueda'}
          onclick={handleClear}
        />
      {/if}
    </div>

    {#if showResults}
      <div
        class="global-search__dropdown"
        id={searchListboxId}
        role={hasResultOptions ? 'listbox' : 'status'}
        aria-label={$currentLocale && translate('topbar.searchAria')}
      >
        {#if searching}
          <div class="global-search__status">
            {$currentLocale && translate('topbar.searchSearching')}
          </div>
        {:else if searchError}
          <div class="global-search__status" class:error={Boolean(searchError)}>
            {searchError}
          </div>
        {:else if searchResults.length === 0}
          <div class="global-search__status">
            {$currentLocale && translate('topbar.searchNoResults', { query: searchQuery })}
          </div>
        {:else}
          {#each searchResults as result, index (result.item.id)}
            <button
              class="global-search__result"
              class:global-search__result--active={index === activeResultIndex}
              type="button"
              role="option"
              id={`${searchListboxId}-option-${index}`}
              aria-selected={index === activeResultIndex}
              onclick={() => handleResultClick(result)}
            >
              <span class="global-search__result-title">{result.item.title}</span>
              <span class="global-search__result-collection">{result.collection.name}</span>
            </button>
          {/each}
        {/if}
      </div>
    {/if}
  </div>

  <div class="topbar__actions">
    {#if LOCAL_ML && hasDepsWarning}
      <StatusBadge
        variant="warning"
        size="sm"
        class="topbar__deps-badge"
        title="Dependencias de IA pendientes"
      >IA</StatusBadge>
    {/if}

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={dbBrowserAria}
      onclick={() => navigation.openRootSection({ name: 'db-browser' })}
      title={dbBrowserTitle}
    >
      <ActionIcon name="database" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={ragChatAria}
      onclick={() => navigation.openRootSection({ name: 'rag-chat' })}
      title={ragChatTitle}
    >
      <ActionIcon name="message-circle" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={themeToggleLabel}
      onclick={toggleTheme}
      title={themeToggleLabel}
    >
      <ActionIcon name="theme" size={16} />
    </IconButton>

    <div
      class="topbar__zoom"
      data-testid="topbar-zoom"
      bind:this={zoomContainerEl}
      onfocusout={handleZoomFocusOut}
    >
      <IconButton
        class="topbar__icon-btn"
        size="md"
        variant="secondary"
        label={zoomTitle}
        title={zoomTitle}
        active={zoomMenuOpen}
        onclick={toggleZoomMenu}
      >
        <ActionIcon name="zoom-in" size={16} />
      </IconButton>

      {#if zoomMenuOpen}
        <div class="topbar__zoom-menu" role="group" aria-label={zoomTitle}>
          <div class="topbar__zoom-stepper">
            <button
              type="button"
              class="topbar__zoom-step"
              aria-label={zoomOutLabel}
              title={zoomOutLabel}
              disabled={$currentZoom <= ZOOM_MIN}
              onclick={() => void zoomOut()}
            >
              <ActionIcon name="zoom-out" size={14} />
            </button>
            <span
              class="topbar__zoom-level"
              data-testid="topbar-zoom-level"
              aria-label={zoomLevelAria}
              aria-live="polite"
            >{zoomPercent}%</span>
            <button
              type="button"
              class="topbar__zoom-step"
              aria-label={zoomInLabel}
              title={zoomInLabel}
              disabled={$currentZoom >= ZOOM_MAX}
              onclick={() => void zoomIn()}
            >
              <ActionIcon name="zoom-in" size={14} />
            </button>
          </div>

          <button
            type="button"
            class="topbar__zoom-reset"
            onclick={() => void resetZoom()}
          >{zoomResetLabel}</button>

          <p class="topbar__zoom-hint">{zoomHint}</p>
        </div>
      {/if}
    </div>

    <IconButton
      class="topbar__icon-btn topbar__icon-btn--settings"
      size="md"
      variant="secondary"
      label={settingsAria}
      onclick={() => navigation.openRootSection({ name: 'settings' })}
      title={settingsTitle}
    >
      <ActionIcon name="settings" size={16} />
      {#if LOCAL_ML && hasDepsWarning}
        <span class="topbar__badge" aria-label="Dependencias pendientes"></span>
      {/if}
    </IconButton>

    <div class="topbar__language" bind:this={languageContainerEl} onfocusout={handleLanguageFocusOut}>
      <IconButton
        class="topbar__icon-btn"
        size="md"
        variant="secondary"
        label={languageTitle}
        title={languageTitle}
        active={languageMenuOpen}
        onclick={toggleLanguageMenu}
      >
        <ActionIcon name="languages" size={16} />
      </IconButton>

      {#if languageMenuOpen}
        <div class="topbar__language-menu" role="menu" aria-label={languageTitle}>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={$currentLocale === 'es'}
            class="topbar__language-option"
            class:active={$currentLocale === 'es'}
            onclick={() => chooseLanguage('es')}
          >ES</button>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={$currentLocale === 'en'}
            class="topbar__language-option"
            class:active={$currentLocale === 'en'}
            onclick={() => chooseLanguage('en')}
          >EN</button>
        </div>
      {/if}
    </div>

    <span class="topbar__window-controls" aria-label="Controles de ventana">
      <IconButton
        class="topbar__window-btn"
        size="sm"
        variant="ghost"
        label="Minimizar ventana"
        title="Minimizar ventana"
        onclick={minimizeWindow}
      >
        <span class="topbar__window-glyph topbar__window-glyph--minimize" aria-hidden="true"></span>
      </IconButton>

      <IconButton
        class="topbar__window-btn"
        size="sm"
        variant="ghost"
        label="Maximizar o restaurar ventana"
        title="Maximizar o restaurar ventana"
        onclick={toggleMaximizeWindow}
      >
        <span class="topbar__window-glyph topbar__window-glyph--maximize" aria-hidden="true"></span>
      </IconButton>

      <IconButton
        class="topbar__window-btn topbar__window-btn--close"
        size="sm"
        variant="ghost"
        label="Cerrar ventana"
        title="Cerrar ventana"
        onclick={closeWindow}
      >
        <ActionIcon name="close" size={14} />
      </IconButton>
    </span>
  </div>
</header>

{#if showDeleteAssetConfirm && pendingDeleteAssetView}
  <ConfirmDialog
    title={t('collection.deleteAssetTitle')}
    titleId="topbar-delete-asset-title"
    message={t('collection.deleteAssetMessage', {
      name: pendingDeleteAssetView.assetLabel ?? '',
    })}
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
  .topbar {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(220px, 320px) auto;
    grid-template-areas: 'leading center search actions';
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
    min-width: 0;
  }

  .topbar__leading {
    grid-area: leading;
    display: grid;
    grid-template-columns: minmax(140px, auto) minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }

  .topbar__back-slot {
    display: flex;
    align-items: center;
    min-width: 0;
    flex-shrink: 0;
  }

  .topbar__app-title {
    min-width: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.02em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .topbar__center {
    grid-area: center;
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 56px;
  }

  .topbar__center--inactive {
    visibility: hidden;
    pointer-events: none;
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
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .crumb__label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .crumb.last {
    color: var(--color-text-primary);
    font-weight: var(--font-weight-medium);
  }
  .sep {
    color: var(--color-text-muted);
  }

  :global(.breadcrumb__delete) {
    width: 28px;
    height: 28px;
    color: var(--color-danger);
  }

  :global(.breadcrumb__delete:hover:not(:disabled)) {
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .crumb-nav {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--color-text-muted);
  }

  :global(.crumb-nav__button) {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    color: inherit;
    font-size: var(--font-size-2xs);
    line-height: 1;
  }

  :global(.crumb-nav__button:disabled) {
    opacity: 0.48;
  }

  .crumb-nav__separator {
    font-size: var(--font-size-2xs);
    opacity: 0.55;
  }

  .topbar__actions {
    grid-area: actions;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .topbar__window-controls {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: var(--space-1);
    padding-left: var(--space-2);
    border-left: 1px solid var(--border-subtle);
  }

  :global(.topbar__window-btn) {
    width: var(--control-height-sm);
    height: var(--control-height-sm);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
  }

  :global(.topbar__window-btn--close:hover:not(:disabled)) {
    border-color: color-mix(in srgb, var(--color-danger) 24%, transparent);
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .topbar__window-glyph {
    display: block;
    width: 12px;
    height: 12px;
    position: relative;
  }

  .topbar__window-glyph--minimize::before {
    content: '';
    position: absolute;
    left: 1px;
    right: 1px;
    bottom: 3px;
    height: 1.5px;
    border-radius: var(--radius-xs);
    background: currentColor;
  }

  .topbar__window-glyph--maximize {
    border: 1.5px solid currentColor;
    border-radius: 2px;
  }

  :global(.topbar__icon-btn) {
    width: var(--control-height-sm);
    height: var(--control-height-sm);
    border-radius: var(--radius-control);
  }

  :global(.topbar__icon-btn--settings) {
    position: relative;
  }

  .topbar__language {
    position: relative;
    display: inline-flex;
  }

  .topbar__zoom {
    position: relative;
    display: inline-flex;
  }

  .topbar__zoom-menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    right: 0;
    z-index: 210;
    display: grid;
    gap: var(--space-1);
    min-width: 156px;
    padding: var(--space-1);
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    box-shadow: var(--shadow-lg);
  }

  .topbar__zoom-stepper {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-1);
  }

  .topbar__zoom-step {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .topbar__zoom-step:hover:not(:disabled) {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  .topbar__zoom-step:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .topbar__zoom-level {
    text-align: center;
    color: var(--color-text-primary);
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    font-variant-numeric: tabular-nums;
  }

  .topbar__zoom-reset {
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
  }

  .topbar__zoom-reset:hover {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  .topbar__zoom-hint {
    margin: 0;
    padding: 0 var(--space-2) var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    text-align: center;
  }

  .topbar__language-menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    right: 0;
    z-index: 210;
    display: flex;
    min-width: 84px;
    padding: var(--space-1);
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    box-shadow: var(--shadow-lg);
  }

  .topbar__language-option {
    flex: 1;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    cursor: pointer;
  }

  .topbar__language-option:hover,
  .topbar__language-option.active {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  :global(.topbar__deps-badge) {
    min-height: 24px;
    font-size: var(--font-size-2xs);
  }

  .topbar__badge {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--color-warning);
    border: 1.5px solid var(--surface-toolbar);
    pointer-events: none;
    animation: none;
  }

  .global-search {
    grid-area: search;
    justify-self: end;
    position: relative;
    width: min(100%, 320px);
    min-width: 0;
  }

  .global-search__input-wrap {
    position: relative;
  }

  .global-search__input {
    width: 100%;
    min-height: var(--control-height-md);
    padding: 0 calc(var(--space-4) + 18px) 0 var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .global-search__input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: var(--surface-panel);
  }

  .global-search__result:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .global-search__dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    margin-top: var(--space-1);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    box-shadow: var(--shadow-lg);
    max-height: 320px;
    overflow-y: auto;
    z-index: 200;
  }

  .global-search__status {
    padding: var(--space-3);
    text-align: center;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .error {
    color: var(--color-danger);
  }

  .global-search__result {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    padding: var(--space-3);
    border: none;
    background: none;
    cursor: pointer;
    text-align: left;
    font-family: var(--font-sans);
    transition:
      background-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .global-search__result:hover {
     background-color: var(--surface-toolbar);
  }

  .global-search__result--active {
    background-color: var(--surface-toolbar);
  }

  .global-search__result + .global-search__result {
    border-top: 1px solid var(--border-subtle);
  }

  .global-search__result-title {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .global-search__result-collection {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  @media (max-width: 900px) {
    .topbar {
      grid-template-columns: minmax(0, 1fr) auto auto;
      grid-template-areas:
        'leading center actions'
        'search search search';
    }

    .topbar__leading {
      grid-area: leading;
    }

    .topbar__center {
      grid-area: center;
    }

    .topbar__actions {
      grid-area: actions;
    }

    .global-search {
      grid-area: search;
      width: 100%;
    }
  }
</style>
