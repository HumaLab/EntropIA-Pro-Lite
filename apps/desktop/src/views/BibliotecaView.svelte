<script lang="ts">
  import { onMount } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import { getNavigation } from '$lib/pane-context'
  import { workspace } from '$lib/workspace'
  import {
    bibliographyLibraryStatus,
    bibliographySearchWorks,
    type BibliographyLibraryStatusRow,
  } from '$lib/bibliography-search'
  import { bibliographyListWorks, type BibliographyWorkRow } from '$lib/bibliography-library'
  import { workLine } from '$lib/rag-scope'
  import { ActionIcon, Button, SearchBar, ToolbarMenu, type ToolbarMenuItem } from '@entropia/ui'

  /**
   * Biblioteca (P2): the works of the synced Zotero libraries, as a paged
   * list of rows in the Colecciones manner — one row per work, a search over
   * title/author/meaning at the top, and a library picker when more than one
   * library is synced. Opening a row pushes the work view. Read-only: this
   * view only ever lists, never imports or edits.
   */

  const navigation = getNavigation()
  const currentLocale = locale

  const PAGE_SIZE = 50

  type LibraryRef = { libraryType: 'user' | 'group'; libraryId: string }

  /** One rendered row, whichever leg produced it (browse or search). */
  type DisplayRow = {
    itemId: string
    itemKey: string
    libraryRowId: string
    title: string
    authors: string
    year: number | null
    libraryName: string
  }

  let libraries = $state<BibliographyLibraryStatusRow[]>([])
  let selected = $state<LibraryRef | null>(null)
  let noLibraries = $state(false)
  let works = $state<BibliographyWorkRow[]>([])
  let total = $state(0)
  let loading = $state(true)
  let loadingMore = $state(false)
  let loadError = $state(false)
  let query = $state('')
  let searchHits = $state<DisplayRow[] | null>(null)
  let searchFailed = $state(false)
  let libraryMenuOpen = $state(false)

  const rows = $derived(searchHits ?? works)
  const countsLabel = $derived.by(() => {
    $currentLocale
    const count = searchHits === null ? total : rows.length
    return count === 1
      ? t('biblioteca.worksCount.one', { count })
      : t('biblioteca.worksCount.other', { count })
  })
  const heading = $derived.by(() => {
    $currentLocale
    const picked = selected
    if (!picked) return t('biblioteca.libraryAll')
    return (
      libraries.find(
        (library) =>
          library.libraryType === picked.libraryType && library.libraryId === picked.libraryId
      )?.name ?? t('biblioteca.libraryAll')
    )
  })
  const showLoadMore = $derived(searchHits === null && !loading && works.length < total)
  const libraryItems = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio',
      id: 'all',
      label: $currentLocale && t('biblioteca.libraryAll'),
      checked: selected === null,
      onselect: () => chooseLibrary(null),
    },
    ...libraries.map((library) => {
      const ref: LibraryRef = {
        libraryType: library.libraryType as 'user' | 'group',
        libraryId: library.libraryId,
      }
      return {
        kind: 'radio' as const,
        id: `${library.libraryType}/${library.libraryId}`,
        label: library.name,
        checked:
          selected?.libraryType === ref.libraryType && selected?.libraryId === ref.libraryId,
        onselect: () => chooseLibrary(ref),
      }
    }),
  ])

  async function loadPage(reset: boolean): Promise<void> {
    if (reset) {
      loading = true
      loadError = false
    } else {
      loadingMore = true
    }
    try {
      const page = await bibliographyListWorks({
        library: selected,
        offset: reset ? 0 : works.length,
        limit: PAGE_SIZE,
      })
      works = reset ? page.works : [...works, ...page.works]
      total = page.total
    } catch {
      if (reset) {
        works = []
        total = 0
        loadError = true
      }
    } finally {
      loading = false
      loadingMore = false
    }
  }

  async function bootstrap(): Promise<void> {
    try {
      const status = await bibliographyLibraryStatus()
      libraries = status.libraries
      if (libraries.length === 0) {
        noLibraries = true
        loading = false
        return
      }
    } catch {
      loadError = true
      loading = false
      return
    }
    await loadPage(true)
  }

  onMount(() => {
    void bootstrap()
  })

  function chooseLibrary(ref: LibraryRef | null): void {
    libraryMenuOpen = false
    if (
      selected?.libraryType === ref?.libraryType &&
      selected?.libraryId === ref?.libraryId
    ) {
      return
    }
    selected = ref
    // A scope change starts the list over: a search of the old scope would
    // answer a question nobody asked any more.
    query = ''
    searchHits = null
    searchFailed = false
    void loadPage(true)
  }

  async function runSearch(text: string): Promise<void> {
    const trimmed = text.trim()
    if (!trimmed) {
      searchHits = null
      searchFailed = false
      return
    }
    searchFailed = false
    try {
      const answer = await bibliographySearchWorks(trimmed, {
        topK: 50,
        ...(selected ? { zoteroLibrary: selected } : {}),
      })
      searchHits = answer.hits.map((hit) => ({
        itemId: hit.itemId,
        itemKey: hit.itemKey,
        libraryRowId: hit.libraryId,
        title: hit.title,
        authors: hit.authors,
        year: hit.year,
        libraryName: hit.libraryName,
      }))
    } catch {
      searchHits = null
      searchFailed = true
    }
  }

  function clearSearch(): void {
    query = ''
    searchHits = null
    searchFailed = false
  }

  function openWork(row: DisplayRow): void {
    navigation.navigate({
      name: 'bibliography-work',
      libraryRowId: row.libraryRowId,
      itemId: row.itemId,
      itemKey: row.itemKey,
      title: row.title,
    })
  }
</script>

<div class="biblioteca page-shell">
  <section class="page-header">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('biblioteca.eyebrow')}</span>
      <h1>{heading}</h1>
      <p>{$currentLocale && t('biblioteca.subtitle')}</p>
    </div>
    {#if !noLibraries}
      <div class="page-toolbar biblioteca__toolbar">
        <!-- Uncontrolled on purpose: echoing `value` back into SearchBar
             resets its debounce timer and the search would never fire. The
             scope key remounts it when the library changes, so the box and
             the listing can never disagree. -->
        {#key selected ? `${selected.libraryType}/${selected.libraryId}` : 'all'}
          <SearchBar
            placeholder={$currentLocale && t('biblioteca.searchPlaceholder')}
            ariaLabel={$currentLocale && t('biblioteca.searchLabel')}
            clearAriaLabel={$currentLocale && t('biblioteca.searchClear')}
            onvaluechange={(next) => {
              query = next
            }}
            onsearch={(next) => void runSearch(next)}
            onclear={clearSearch}
          />
        {/key}
        {#if libraries.length > 1}
          <ToolbarMenu
            label={$currentLocale && t('biblioteca.libraryMenu')}
            items={libraryItems}
            bind:open={libraryMenuOpen}
          >
            {#snippet trigger(props, { open })}
              <button
                type="button"
                class="biblioteca__library-trigger"
                class:biblioteca__library-trigger--open={open}
                aria-label={$currentLocale && t('biblioteca.libraryMenu')}
                {...props}
              >
                <span class="biblioteca__library-trigger-label">{heading}</span>
                <ActionIcon name="chevron-down" size={12} />
              </button>
            {/snippet}
          </ToolbarMenu>
        {/if}
      </div>
    {/if}
  </section>

  {#if loadError}
    <p class="surface-message surface-message--error">{$currentLocale && t('biblioteca.error')}</p>
    <div class="biblioteca__retry">
      <Button variant="secondary" size="sm" onclick={() => void bootstrap()}>
        {$currentLocale && t('biblioteca.retry')}
      </Button>
    </div>
  {:else if searchFailed}
    <p class="surface-message surface-message--error">{$currentLocale && t('biblioteca.error')}</p>
  {:else if noLibraries}
    <div class="surface-message surface-message--center empty">
      <p>{$currentLocale && t('biblioteca.empty')}</p>
      <p class="biblioteca__hint">{$currentLocale && t('biblioteca.emptyHint')}</p>
      <Button variant="secondary" size="sm" onclick={() => workspace.navigateActive({ name: 'writing' })}>
        {$currentLocale && t('biblioteca.emptyAction')}
      </Button>
    </div>
  {:else if loading}
    <p class="surface-message surface-message--center">{$currentLocale && t('biblioteca.loading')}</p>
  {:else if rows.length === 0}
    <div class="surface-message surface-message--center empty">
      <p>
        {$currentLocale &&
          (searchHits !== null
            ? t('biblioteca.emptySearch', { query })
            : t('biblioteca.empty'))}
      </p>
    </div>
  {:else}
    <ul class="biblioteca__rows" aria-label={$currentLocale && t('biblioteca.gridAria')}>
      {#each rows as row (row.itemId)}
        <li class="biblioteca__row-item">
          <button type="button" class="biblioteca__row" onclick={() => openWork(row)}>
            <span class="biblioteca__row-title">{row.title}</span>
            <span class="biblioteca__row-meta">{workLine(row)}</span>
            <span class="biblioteca__row-chip">{row.libraryName}</span>
          </button>
        </li>
      {/each}
    </ul>
    <p class="biblioteca__counts">{$currentLocale && countsLabel}</p>
    {#if showLoadMore}
      <div class="page-continuation">
        <Button variant="secondary" size="sm" loading={loadingMore} onclick={() => void loadPage(false)}>
          {$currentLocale && t('biblioteca.loadMore')}
        </Button>
      </div>
    {/if}
    {#if loadingMore}
      <p class="page-continuation">{$currentLocale && t('biblioteca.loadingMore')}</p>
    {/if}
  {/if}
</div>

<style>
  .biblioteca {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .biblioteca__toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .biblioteca__library-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 260px;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: var(--color-surface-glass);
    color: var(--color-text-primary);
    cursor: pointer;
  }

  .biblioteca__library-trigger:hover,
  .biblioteca__library-trigger--open {
    border-color: color-mix(in srgb, var(--color-accent) 40%, var(--color-hairline));
  }

  .biblioteca__library-trigger:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .biblioteca__library-trigger-label {
    overflow: hidden;
    font-size: var(--font-size-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Rows carry the item-card surface (Colecciones' card language) as a
     horizontal list: one title, one reading line, one library chip. */
  .biblioteca__rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .biblioteca__row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas:
      'title chip'
      'meta chip';
    align-items: center;
    gap: var(--space-1) var(--space-3);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth);
  }

  .biblioteca__row:hover {
    border-color: color-mix(in srgb, var(--color-accent) 26%, var(--color-border-strong));
    box-shadow: var(--shadow-surface);
  }

  .biblioteca__row:focus-visible {
    border-color: color-mix(in srgb, var(--color-accent) 26%, var(--color-border-strong));
  }

  .biblioteca__row:focus-visible {
    outline: none;
    box-shadow: var(--shadow-surface), var(--focus-ring);
  }

  .biblioteca__row-title {
    grid-area: title;
    overflow: hidden;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-medium);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .biblioteca__row-meta {
    grid-area: meta;
    overflow: hidden;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .biblioteca__row-chip {
    grid-area: chip;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    white-space: nowrap;
  }

  .biblioteca__counts {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    text-align: center;
  }

  .biblioteca__hint {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .biblioteca__retry {
    display: flex;
    justify-content: center;
    padding-bottom: var(--space-4);
  }
</style>
