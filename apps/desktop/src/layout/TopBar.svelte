<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { navigation } from '$lib/navigation'
  import { getStore } from '$lib/db'
  import { locale, t } from '$lib/i18n'
  import { isCriticalMissing, onCriticalMissingChange } from '$lib/deps'
  import { ActionIcon, Button, IconButton, StatusBadge } from '@entropia/ui'
  import type { Collection, Item } from '@entropia/store'

  let hasDepsWarning = $state(isCriticalMissing())
  const unsubDeps = onCriticalMissingChange((v) => { hasDepsWarning = v })

  type AppTheme = 'dark' | 'dim' | 'light'

  const THEME_STORAGE_KEY = 'entropia-theme'

  interface SearchResult {
    item: Item
    collection: Collection
  }

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
  let activeResultIndex = $state(-1)
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
  function minimizeWindow() {
    void getCurrentWindow().minimize()
  }

  function toggleMaximizeWindow() {
    void getCurrentWindow().toggleMaximize()
  }

  function closeWindow() {
    void getCurrentWindow().close()
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

  async function loadSiblingItems() {
    const currentView = $navigation.current
    const requestId = ++siblingRequestId

    previousItem = null
    nextItem = null

    if (currentView.name !== 'item') return

    try {
      const items = await getStore().items.findByCollection(currentView.collectionId)
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
        <span class="topbar__app-title" data-tauri-drag-region>EntropIA Pro</span>
      {/if}
    </div>
    <nav class="breadcrumb" aria-label={$currentLocale && t('topbar.breadcrumb')} data-tauri-drag-region>
      {#each $navigation.breadcrumb as crumb, i (i)}
        {#if i > 0}<span class="sep">/</span>{/if}
        {#if i === $navigation.breadcrumb.length - 1}
          <span class="crumb crumb--current" class:last={i === $navigation.breadcrumb.length - 1} data-tauri-drag-region>
            <span class="crumb__label" data-tauri-drag-region>{crumb}</span>
          </span>
        {:else}
          <span class="crumb" data-tauri-drag-region>{crumb}</span>
        {/if}
      {/each}
    </nav>
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
        type="search"
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
        <IconButton
          class="global-search__clear"
          size="sm"
          variant="ghost"
          label={$currentLocale ? translate('topbar.searchClear') : 'Limpiar búsqueda'}
          title={$currentLocale ? translate('topbar.searchClear') : 'Limpiar búsqueda'}
          onclick={handleClear}
        >
          <ActionIcon name="close" size={14} />
        </IconButton>
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
    {#if hasDepsWarning}
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
      active={theme === 'dim'}
      onclick={toggleTheme}
      title={themeToggleLabel}
    >
      <ActionIcon name="moon" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn topbar__icon-btn--settings"
      size="md"
      variant="secondary"
      label={settingsAria}
      onclick={() => navigation.openRootSection({ name: 'settings' })}
      title={settingsTitle}
    >
      <ActionIcon name="settings" size={16} />
      {#if hasDepsWarning}
        <span class="topbar__badge" aria-label="Dependencias pendientes"></span>
      {/if}
    </IconButton>

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
    grid-template-columns: minmax(140px, auto) minmax(0, 1fr);
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

  :global(.global-search__clear) {
    position: absolute;
    top: 50%;
    right: var(--space-2);
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    transform: translateY(-50%);
  }

  :global(.global-search__clear:hover:not(:disabled)) {
    transform: translateY(-50%);
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
