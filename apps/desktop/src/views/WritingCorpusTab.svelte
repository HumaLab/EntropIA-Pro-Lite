<script lang="ts">
  import { onDestroy } from 'svelte'
  import { ActionIcon, Button, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { FtsSearchController } from '$lib/item-view-search'
  import { writingCorpus } from '$lib/writing-corpus'

  /**
   * The Corpus tab of the research panel (plan-editor.md §6.3, §10.1).
   *
   * Two levels, because that is the shape of the anchor a citation stores: a
   * search names an item, and the item offers the pages one of which the
   * citation will point at. `fts_items` is indexed per item and contentless,
   * and an asset *is* a page (migration 0024) — so flattening the two would
   * mean inventing a level the data does not have.
   *
   * The search itself is `store.fts`, driven by the same `FtsSearchController`
   * the item view uses. Unit 4 forbids a second retrieval engine, and reusing
   * the controller is what keeps the debounce, the Enter and the Escape
   * behaving the same here as everywhere else.
   */

  const store = writingCorpus
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  const controller = new FtsSearchController({
    getQuery: () => snapshot.query,
    setQuery: () => {},
    reset: () => void store.search(''),
    search: (query) => store.search(query),
  })

  onDestroy(() => {
    unsubscribe()
    controller.cancel()
  })

  function pageLabel(pageNumber: number | null): string {
    return pageNumber === null
      ? t('writing.corpusPageUnnumbered')
      : t('writing.corpusPage', { page: String(pageNumber) })
  }
</script>

<div class="corpus">
  <SearchBar
    value={snapshot.query}
    debounceMs={0}
    emitSearch={false}
    ariaLabel={t('writing.corpusSearch')}
    placeholder={t('writing.corpusSearch')}
    onvaluechange={(query) => controller.handleInput(query)}
    onkeydown={(event) => controller.handleKeydown(event)}
  />

  {#if snapshot.error}
    <p class="corpus__error" role="alert">{snapshot.error}</p>
  {/if}

  {#if snapshot.openItem}
    <div class="corpus__open">
      <Button variant="ghost" size="sm" onclick={() => store.closeItem()}>
        <ActionIcon name="chevron-left" size={14} />
        {t('writing.corpusBack')}
      </Button>
      <h3 class="corpus__open-title">{snapshot.openItem.title}</h3>
    </div>

    {#if snapshot.pages.length === 0}
      <p class="corpus__notice">{t('writing.corpusNoPages')}</p>
    {:else}
      <p class="corpus__label">{t('writing.corpusPages')}</p>
      <ul class="corpus__list">
        {#each snapshot.pages as page (page.assetId)}
          <li>
            <button
              type="button"
              class="corpus__row"
              class:corpus__row--open={snapshot.openPageId === page.assetId}
              onclick={() => store.openPage(page.assetId)}
            >
              {pageLabel(page.pageNumber)}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if snapshot.openPageId}
      {#if snapshot.pageText}
        <!-- The extracted text, which is what a citation quotes and anchors
             into. Selecting inside it is how a fragment gets chosen. -->
        <p class="corpus__text">{snapshot.pageText}</p>
      {:else}
        <p class="corpus__notice">{t('writing.corpusNoText')}</p>
      {/if}
    {/if}
  {:else if snapshot.searching}
    <p class="corpus__notice" role="status">{t('writing.corpusSearching')}</p>
  {:else if snapshot.results.length > 0}
    <ul class="corpus__list">
      {#each snapshot.results as hit (hit.itemId)}
        <li>
          <button type="button" class="corpus__row" onclick={() => store.openItem(hit.itemId)}>
            {hit.title}
          </button>
        </li>
      {/each}
    </ul>
  {:else if snapshot.query.trim()}
    <p class="corpus__notice">{t('writing.corpusEmpty')}</p>
  {:else}
    <p class="corpus__notice">{t('writing.corpusStart')}</p>
  {/if}
</div>

<style>
  .corpus {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }

  .corpus__open {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
  }

  .corpus__open-title {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .corpus__label {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .corpus__list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .corpus__row {
    display: block;
    width: 100%;
    min-height: 28px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font: inherit;
    font-size: var(--font-size-xs);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition: background var(--transition-base), color var(--transition-base);
  }

  .corpus__row:hover {
    background: var(--color-accent-faint);
    color: var(--color-text-primary);
  }

  .corpus__row--open {
    background: var(--color-accent-soft);
    color: var(--color-text-primary);
  }

  .corpus__text {
    max-height: 40vh;
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
    white-space: pre-wrap;
    overflow-y: auto;
    user-select: text;
  }

  .corpus__notice,
  .corpus__error {
    margin: 0;
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .corpus__notice {
    color: var(--color-text-muted);
  }

  .corpus__error {
    color: var(--color-danger);
  }
</style>
