<script lang="ts">
  import { onDestroy } from 'svelte'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { workspace } from '$lib/workspace'
  import { getStore } from '$lib/db'
  import { locale, t } from '$lib/i18n'
  import { isCriticalMissing, onCriticalMissingChange } from '$lib/deps'
  import { LOCAL_ML, NAVEGADOR } from '$lib/capabilities'
  import { PRODUCT_NAME } from '$lib/product'
  // Black on transparent, the 'e' only: hlab-mark.png is a white disc behind
  // the 'e', so as a mask it paints a full circle.
  import appMark from '../assets/entropia-mark.png'
  import { ActionIcon, IconButton, SearchClearButton, StatusBadge } from '@entropia/ui'
  import type { Collection, Item } from '@entropia/store'
  import { bibliographySearchWorks, type BibliographySearchHit } from '$lib/bibliography-search'
  import type { LibraryEntry } from '$lib/writing-zotero'
  import WritingZoteroDetails from '../views/WritingZoteroDetails.svelte'
  import TabStrip from './TabStrip.svelte'

  // Measured by AppShell (the split container's real width) and forwarded
  // here: whether the split area can currently fit two 640px panes side by
  // side (user rule, 2026-09-25). Defaults to `true` so every other caller
  // (most tests included) keeps the toggle enabled without wiring this up.
  let { splitAvailable = true }: { splitAvailable?: boolean } = $props()

  let hasDepsWarning = $state(isCriticalMissing())
  const unsubDeps = onCriticalMissingChange((v) => {
    hasDepsWarning = v
  })

  interface SearchResult {
    item: Item
    collection: Collection
  }

  // How many bibliography works the dropdown offers. The corpus keeps its own
  // 20: the two groups are separate lists with unrelated score scales.
  const BIBLIOGRAPHY_LIMIT = 5

  let searchQuery = $state('')
  let searchResults = $state<SearchResult[]>([])
  // Works of the synced Zotero libraries, as their own group. Never merged
  // with `searchResults`: the corpus and the bibliography rank on scales that
  // cannot be compared, so each keeps its order and the group sits below.
  let bibliographyResults = $state<BibliographySearchHit[]>([])
  let bibliographySearching = $state(false)
  // The work whose ficha replaces the list in the dropdown, if any.
  let detailsEntry = $state<LibraryEntry | null>(null)
  let searchError = $state('')
  let showResults = $state(false)
  let searching = $state(false)
  let searchRequestId = 0
  let debounceTimer: ReturnType<typeof setTimeout> | null = null
  let searchInputEl: HTMLInputElement | undefined = $state()
  let searchContainerEl: HTMLDivElement | undefined = $state()
  let activeResultIndex = $state(-1)
  const searchListboxId = 'topbar-global-search-listbox'
  const currentLocale = locale
  const translate = (key: string, params?: Record<string, string | number>) =>
    t(key as never, params)
  // The split toggle (Task 3.3) is chrome, so it reads the workspace's own
  // subscription rather than a pane-scoped navigation — split state applies
  // to the whole tab pairing, not to any one pane.
  const wsSnapshot = $derived($workspace)
  const splitPressed = $derived(wsSnapshot.split !== null)
  // Disabled only while split is OFF and the window is too narrow to turn it
  // on — never while it is already ON, or the toggle would be the only way
  // to turn split off and just made itself unreachable.
  const splitToggleDisabled = $derived(!splitPressed && !splitAvailable)
  const splitTitle = $derived(
    splitToggleDisabled
      ? $currentLocale
        ? translate('topbar.splitDisabledTitle')
        : 'La ventana es muy angosta para la vista dividida'
      : $currentLocale
        ? translate('topbar.splitTitle')
        : 'Vista dividida'
  )
  const splitAria = $derived(
    $currentLocale ? translate('topbar.splitAria') : 'Alternar vista dividida'
  )
  const optionCount = $derived(searchResults.length + bibliographyResults.length)
  const hasResultOptions = $derived(!detailsEntry && !searching && !searchError && optionCount > 0)
  const activeOptionId = $derived(
    showResults && hasResultOptions && activeResultIndex >= 0
      ? `${searchListboxId}-option-${activeResultIndex}`
      : undefined
  )
  const dbBrowserTitle = $derived(
    $currentLocale ? translate('topbar.dbBrowserTitle') : 'Base de datos'
  )
  const dbBrowserAria = $derived(
    $currentLocale ? translate('topbar.dbBrowserAria') : 'Abrir navegador de base de datos'
  )
  const homeTitle = $derived($currentLocale ? translate('topbar.homeTitle') : 'Inicio')
  const homeAria = $derived($currentLocale ? translate('topbar.homeAria') : 'Abrir Inicio')
  const collectionsTitle = $derived(
    $currentLocale ? translate('topbar.collectionsTitle') : 'Colecciones'
  )
  const collectionsAria = $derived(
    $currentLocale ? translate('topbar.collectionsAria') : 'Abrir Colecciones'
  )
  const ragChatTitle = $derived(
    $currentLocale ? translate('topbar.ragChatTitle') : 'Chat de investigación'
  )
  const ragChatAria = $derived(
    $currentLocale ? translate('topbar.ragChatAria') : 'Abrir chat de investigación'
  )
  const researchTitle = $derived(
    $currentLocale ? translate('topbar.researchTitle') : 'Agente de investigación'
  )
  const researchAria = $derived(
    $currentLocale ? translate('topbar.researchAria') : 'Abrir agente de investigación'
  )
  const navegadorTitle = $derived(
    $currentLocale ? translate('topbar.navegadorTitle') : 'Navegador (experimental)'
  )
  const navegadorAria = $derived(
    $currentLocale ? translate('topbar.navegadorAria') : 'Abrir el Navegador experimental'
  )
  const writingTitle = $derived($currentLocale ? translate('topbar.writingTitle') : 'Escritura')
  const writingAria = $derived($currentLocale ? translate('topbar.writingAria') : 'Abrir Escritura')
  const settingsTitle = $derived(
    hasDepsWarning
      ? $currentLocale
        ? t('topbar.depsWarningTitle')
        : 'Dependencias de IA pendientes - click para configurar'
      : $currentLocale
        ? t('topbar.settingsTitle')
        : 'Configuración'
  )
  const settingsAria = $derived(
    hasDepsWarning
      ? $currentLocale
        ? t('topbar.depsWarningAria')
        : 'Dependencias de IA pendientes'
      : $currentLocale
        ? t('topbar.settingsAria')
        : 'Abrir configuración'
  )
  // Split toggle focus restore (final visual check, split view): 8e57aa7f
  // stopped the pane that stays on screen from remounting, so an open
  // editor keeps its text across a split toggle — but a mouse click on the
  // toggle still steals focus from it the way clicking any focusable button
  // does, silently losing the caret even though the DOM node never went
  // away. Captured on `pointerdown` — before the browser's own default
  // mousedown action moves focus onto the button — so it holds whatever had
  // focus right before THIS click. A keyboard activation (Enter/Space while
  // already tab-focused on the toggle) never fires `pointerdown`, so nothing
  // is captured then and focus is correctly left on the toggle.
  let preSplitToggleFocus: HTMLElement | null = null

  function captureFocusBeforeSplitToggle(event: PointerEvent) {
    const active = document.activeElement
    preSplitToggleFocus =
      active instanceof HTMLElement && active !== event.currentTarget ? active : null
  }

  function handleSplitToggleClick() {
    const restoreTarget = preSplitToggleFocus
    preSplitToggleFocus = null
    workspace.toggleSplit()
    if (restoreTarget?.isConnected) restoreTarget.focus()
  }

  function minimizeWindow() {
    void getCurrentWindow().minimize()
  }

  function toggleMaximizeWindow() {
    void getCurrentWindow().toggleMaximize()
  }

  function closeWindow() {
    void getCurrentWindow().close()
  }

  onDestroy(() => {
    unsubDeps()
  })

  async function searchBibliography(query: string, requestId: number) {
    const isCurrentRequest = () => requestId === searchRequestId
    bibliographyResults = []
    bibliographySearching = true
    try {
      const answer = await bibliographySearchWorks(query, { topK: BIBLIOGRAPHY_LIMIT })
      if (!isCurrentRequest()) return
      // A hit without a native library identity cannot open its ficha.
      bibliographyResults = answer.hits.filter((hit) => hit.libraryType && hit.libraryNativeId)
    } catch (e) {
      // Never breaks the corpus results: this leg only ever adds a group.
      if (!isCurrentRequest()) return
      console.warn('[Search] bibliography error:', e)
      bibliographyResults = []
    } finally {
      if (isCurrentRequest()) bibliographySearching = false
    }
  }

  /** The held entry the ficha reads, built from the hit's own catalog data. */
  function entryFromHit(hit: BibliographySearchHit): LibraryEntry {
    return {
      key: hit.itemKey,
      itemVersion: 0,
      libraryType: hit.libraryType,
      libraryId: hit.libraryNativeId,
      title: hit.title,
      authors: hit.authors,
      year: hit.year ? String(hit.year) : '',
      csl_json: hit.cslJson,
    }
  }

  function authorsAndYear(hit: BibliographySearchHit): string {
    return [hit.authors, hit.year ? String(hit.year) : ''].filter(Boolean).join(' · ')
  }

  async function performSearch(query: string, requestId: number) {
    const isCurrentRequest = () => requestId === searchRequestId

    if (!isCurrentRequest()) return

    if (!query.trim()) {
      searchResults = []
      bibliographyResults = []
      searchError = ''
      showResults = false
      return
    }

    searching = true
    searchError = ''
    // Started before the corpus is awaited: the two run side by side, and a
    // slow (embedding) bibliography leg never holds the corpus results back.
    void searchBibliography(query, requestId)
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
    detailsEntry = null

    if (!searchQuery.trim()) {
      searchResults = []
      bibliographyResults = []
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
    bibliographyResults = []
    bibliographySearching = false
    detailsEntry = null
    searchError = ''
    activeResultIndex = -1
    showResults = false
    if (debounceTimer) clearTimeout(debounceTimer)
  }

  // One flat sequence for the keyboard and the option ids: corpus rows first,
  // then the bibliography group.
  function handleOptionActivate(index: number) {
    const corpusResult = searchResults[index]
    if (corpusResult) {
      handleResultClick(corpusResult)
      return
    }
    const hit = bibliographyResults[index - searchResults.length]
    if (hit) openBibliographyHit(hit)
  }

  function openBibliographyHit(hit: BibliographySearchHit) {
    // The ficha opens where the search is: no navigation, no tab change.
    detailsEntry = entryFromHit(hit)
    activeResultIndex = -1
  }

  function handleResultClick(result: SearchResult) {
    // Routed through `navigateActive` (evaluated fresh, at click time)
    // rather than a captured navigation reference, so a result always opens
    // on whichever tab is active right now.
    workspace.navigateActive({
      name: 'collection',
      id: result.collection.id,
      collectionName: result.collection.name,
    })
    workspace.navigateActive({
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
      if (!showResults || detailsEntry || optionCount === 0) return
      e.preventDefault()
      const lastIndex = optionCount - 1
      if (e.key === 'ArrowDown') {
        activeResultIndex = activeResultIndex >= lastIndex ? 0 : activeResultIndex + 1
      } else {
        activeResultIndex = activeResultIndex <= 0 ? lastIndex : activeResultIndex - 1
      }
      return
    }

    // keyCode 229 cubre WKWebView, donde isComposing puede no reportarse durante IME.
    if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
      if (!showResults || detailsEntry) return
      if (activeResultIndex < 0 || activeResultIndex >= optionCount) return
      e.preventDefault()
      handleOptionActivate(activeResultIndex)
    }
  }

  function handleFocusOut(event: FocusEvent) {
    const nextFocused = event.relatedTarget
    if (nextFocused instanceof Node && searchContainerEl?.contains(nextFocused)) return
    showResults = false
    activeResultIndex = -1
    detailsEntry = null
  }

  function handleFocus() {
    if (optionCount > 0 || searchError) {
      showResults = true
    }
  }
</script>

<!-- Window drag: Tauri's drag script only reads data-tauri-drag-region on the
     exact mousedown target, never on an ancestor. Every empty container below
     carries it; controls never do, so they keep their clicks. -->
<header class="topbar" data-tauri-drag-region>
  <div class="topbar__leading" data-tauri-drag-region>
    <div class="topbar__back-slot" data-tauri-drag-region>
      <span class="topbar__app-title" data-tauri-drag-region>
        <span
          class="topbar__app-mark"
          aria-hidden="true"
          style:mask-image={`url(${appMark})`}
          data-tauri-drag-region
        ></span>
        {PRODUCT_NAME}
      </span>
    </div>
  </div>

  <div class="topbar__center" data-tauri-drag-region>
    <TabStrip />
  </div>

  <div class="global-search" bind:this={searchContainerEl} onfocusout={handleFocusOut}>
    <div class="global-search__input-wrap">
      <span class="search-field__icon" aria-hidden="true">
        <ActionIcon name="search" size={16} />
      </span>
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
        role={detailsEntry ? 'group' : hasResultOptions ? 'listbox' : 'status'}
        aria-label={$currentLocale && translate('topbar.searchAria')}
      >
        {#if detailsEntry}
          <!-- The existing ficha, untouched. tabindex lets a click on its plain
               text keep focus inside the container, so the dropdown stays open. -->
          <div class="global-search__details" tabindex="-1">
            {#key detailsEntry.key}
              <WritingZoteroDetails entry={detailsEntry} onclose={() => (detailsEntry = null)} />
            {/key}
          </div>
        {:else if searching || (bibliographySearching && optionCount === 0)}
          <div class="global-search__status">
            {$currentLocale && translate('topbar.searchSearching')}
          </div>
        {:else if searchError}
          <div class="global-search__status" class:error={Boolean(searchError)}>
            {searchError}
          </div>
        {:else if optionCount === 0}
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
          {#if bibliographyResults.length > 0}
            <div
              class="global-search__group"
              role="group"
              aria-labelledby="{searchListboxId}-bibliography"
            >
              <p class="global-search__group-label" id="{searchListboxId}-bibliography">
                {$currentLocale && translate('topbar.searchBibliographyGroup')}
              </p>
              {#each bibliographyResults as hit, hitIndex (hit.itemId)}
                {@const index = searchResults.length + hitIndex}
                <button
                  class="global-search__result"
                  class:global-search__result--active={index === activeResultIndex}
                  type="button"
                  role="option"
                  id={`${searchListboxId}-option-${index}`}
                  aria-selected={index === activeResultIndex}
                  onclick={() => openBibliographyHit(hit)}
                >
                  <span class="global-search__result-title">{hit.title}</span>
                  {#if authorsAndYear(hit)}
                    <span class="global-search__result-collection">{authorsAndYear(hit)}</span>
                  {/if}
                  <span class="global-search__result-collection">
                    {hit.libraryName}
                    {#if hit.method !== 'lexical'}
                      <span class="global-search__semantic">
                        {translate('writing.zoteroSemanticTag')}
                      </span>
                    {/if}
                  </span>
                </button>
              {/each}
            </div>
          {/if}
        {/if}
      </div>
    {/if}
  </div>

  <div class="topbar__actions" data-tauri-drag-region>
    {#if LOCAL_ML && hasDepsWarning}
      <StatusBadge
        variant="warning"
        size="sm"
        class="topbar__deps-badge"
        title="Dependencias de IA pendientes">IA</StatusBadge
      >
    {/if}

    <!-- Colecciones is a screen like any other: pushed, so Back returns to
         wherever it was opened from instead of always landing on Inicio. -->
    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={homeAria}
      onclick={() => workspace.navigateActive({ name: 'home' })}
      title={homeTitle}
    >
      <ActionIcon name="home" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={collectionsAria}
      onclick={() => workspace.navigateActive({ name: 'collections' })}
      title={collectionsTitle}
    >
      <ActionIcon name="folder" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={ragChatAria}
      onclick={() => workspace.navigateActive({ name: 'rag-chat' })}
      title={ragChatTitle}
    >
      <ActionIcon name="message-circle" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={researchAria}
      onclick={() => workspace.navigateActive({ name: 'research' })}
      title={researchTitle}
    >
      <ActionIcon name="research" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={writingAria}
      onclick={() => workspace.navigateActive({ name: 'writing' })}
      title={writingTitle}
    >
      <ActionIcon name="edit" size={16} />
    </IconButton>

    {#if NAVEGADOR}
      <IconButton
        class="topbar__icon-btn"
        size="md"
        variant="secondary"
        label={navegadorAria}
        onclick={() => workspace.navigateActive({ name: 'navegador' })}
        title={navegadorTitle}
      >
        <ActionIcon name="browser" size={16} />
      </IconButton>
    {/if}

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={splitAria}
      active={splitPressed}
      disabled={splitToggleDisabled}
      onpointerdown={captureFocusBeforeSplitToggle}
      onclick={handleSplitToggleClick}
      title={splitTitle}
    >
      <ActionIcon name="split" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={dbBrowserAria}
      onclick={() => workspace.navigateActive({ name: 'db-browser' })}
      title={dbBrowserTitle}
    >
      <ActionIcon name="database" size={16} />
    </IconButton>

    <IconButton
      class="topbar__icon-btn topbar__icon-btn--settings"
      size="md"
      variant="secondary"
      label={settingsAria}
      onclick={() => workspace.navigateActive({ name: 'settings' })}
      title={settingsTitle}
    >
      <ActionIcon name="settings" size={16} />
      {#if LOCAL_ML && hasDepsWarning}
        <span class="topbar__badge" aria-label="Dependencias pendientes"></span>
      {/if}
    </IconButton>

    <span class="topbar__window-controls" aria-label="Controles de ventana" data-tauri-drag-region>
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
    /* The tab strip (center), not the title (leading), is the flexible
       track: tabs start right after the title and grow toward the search
       box, instead of the title eating all the free space and pushing the
       strip flush against search. */
    grid-template-columns: auto minmax(0, 1fr) minmax(220px, 320px) auto;
    grid-template-areas: 'leading center search actions';
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
    min-width: 0;
    /* A press on the bar moves the window; it must never select text or start
       an HTML drag of content (the search field opts back in below). */
    user-select: none;
    -webkit-user-drag: none;
  }

  .topbar__leading {
    grid-area: leading;
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .topbar__back-slot {
    display: flex;
    align-items: center;
    min-width: 0;
    flex-shrink: 0;
  }

  .topbar__app-title {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.02em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  /* The mark is a black-on-transparent PNG. Used as a mask it takes the
     title's colour, so it reads on every theme. */
  .topbar__app-mark {
    flex-shrink: 0;
    /* The cropped 'e' is 161 x 210. */
    width: 11px;
    height: 14px;
    background: currentColor;
    mask-position: left center;
    mask-repeat: no-repeat;
    mask-size: contain;
  }

  .topbar__center {
    grid-area: center;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    min-width: 0;
    flex: 1;
    overflow: hidden;
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

  /* Size comes from IconButton's own `size="sm"` (--icon-button-size:
     28px), not from here: this used to also set `width`/`height:
     var(--control-height-sm)` (30px), but that never won against
     IconButton's own scoped rule at equal specificity — dead code that
     rendered nothing different from the size prop alone. */
  :global(.topbar__window-btn) {
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

  /* Size comes from IconButton's own `size="md"` (--icon-button-size:
     32px), not from here: this used to also set `width`/`height:
     var(--control-height-sm)` (30px), but that never won against
     IconButton's own scoped rule at equal specificity — dead code that
     rendered nothing different from the size prop alone. */
  :global(.topbar__icon-btn) {
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
    user-select: text;
    min-height: var(--control-height-md);
    padding: 0 calc(var(--space-4) + 18px) 0 var(--search-field-inset);
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
    font-family: var(--font-ui);
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

  /* The bibliography group: a label over rows that are the corpus rows. */
  .global-search__group {
    border-top: 1px solid var(--border-subtle);
  }

  .global-search__group:first-child {
    border-top: none;
  }

  .global-search__group-label {
    margin: 0;
    padding: var(--space-2) var(--space-3) 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  .global-search__group-label + .global-search__result {
    border-top: none;
  }

  .global-search__semantic {
    margin-left: var(--space-1);
    padding: 0 var(--space-1);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    color: var(--color-text-secondary);
  }

  .global-search__details {
    padding: var(--space-3);
    user-select: text;
  }

  .global-search__details:focus {
    outline: none;
  }

  @media (max-width: 900px) {
    .topbar {
      grid-template-columns: auto minmax(0, 1fr) auto;
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
