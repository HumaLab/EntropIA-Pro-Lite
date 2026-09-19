<script lang="ts">
  import { onDestroy } from 'svelte'
  import { ActionIcon, Button, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { FtsSearchController } from '$lib/item-view-search'
  import { corpusPageLabel, writingCorpus } from '$lib/writing-corpus'
  import { hashSourceText, selectionRange, type SourceRange } from '$lib/source-selection'

  interface Props {
    /** Puts a citation in the manuscript and returns the identity it minted. */
    oninsertcitation?: (attrs: Record<string, unknown>) => string | null
  }

  let { oninsertcitation }: Props = $props()

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

  /** The fragment currently highlighted in the page text, if any. */
  let textElement = $state<HTMLParagraphElement | undefined>(undefined)
  let chosen = $state<SourceRange | null>(null)
  let inserted = $state(false)

  /**
   * Watches the document's selection rather than listening on the paragraph.
   *
   * Reading a selection is observing, not interacting — putting mouse and key
   * handlers on a `<p>` claims otherwise, which is what the accessibility guard
   * objects to. It also catches the selections those handlers missed: double
   * click to take a word, Shift+arrow, and select-all inside the pane.
   */
  $effect(() => {
    const read = () => {
      chosen = selectionRange(textElement ?? null, window.getSelection())
      inserted = false
    }
    document.addEventListener('selectionchange', read)
    return () => document.removeEventListener('selectionchange', read)
  })

  /**
   * Builds the citation and hands it to the editor (§10.1).
   *
   * Everything the anchor needs goes on the node, because the projection is
   * derived from the document: what the node does not carry never reaches the
   * database. The hash is awaited before inserting so the node is complete the
   * first time it is saved, rather than gaining a field on some later edit.
   */
  async function insertCitation() {
    const page = snapshot.pages.find((entry) => entry.assetId === snapshot.openPageId)
    if (!chosen || !snapshot.openItem || !page || !oninsertcitation) return

    const sourceTextHash = await hashSourceText(chosen.text)
    const id = oninsertcitation({
      collectionId: snapshot.openItem.collectionId,
      itemId: snapshot.openItem.itemId,
      assetId: page.assetId,
      pageNumber: page.pageNumber,
      startChar: chosen.start,
      endChar: chosen.end,
      quotedText: chosen.text,
      sourceTextHash,
      metadataSnapshot: { title: snapshot.openItem.title, pageNumber: page.pageNumber },
    })

    inserted = id !== null
    if (inserted) chosen = null
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
              {corpusPageLabel(page, t)}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if snapshot.openPageId}
      {#if snapshot.pageText}
        <!-- One bare text node, deliberately: the DOM offsets inside it are the
             character offsets into the extraction, and any markup in here would
             break that identity. `selectionRange` refuses rather than
             mis-anchoring if it ever does. -->
        <p class="corpus__text" bind:this={textElement}>{snapshot.pageText}</p>

        <div class="corpus__insert">
          <Button
            variant="secondary"
            size="sm"
            disabled={!chosen || !oninsertcitation}
            onclick={insertCitation}
          >
            <ActionIcon name="text-quote" size={14} />
            {t('writing.corpusInsert')}
          </Button>
          <p class="corpus__notice" role="status">
            {#if inserted}
              {t('writing.corpusInserted')}
            {:else if !oninsertcitation}
              {t('writing.corpusNoDocument')}
            {:else if !chosen}
              {t('writing.corpusSelectFirst')}
            {/if}
          </p>
        </div>
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
            <span class="corpus__row-title">{hit.title}</span>
            {#if hit.foundAs}
              <!-- Says why a document without the searched word is here: the
                   variant it holds instead, usually an OCR misreading. -->
              <span class="corpus__row-found">
                {t('writing.corpusFoundAs', { words: hit.foundAs.join(', ') })}
              </span>
            {/if}
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
    transition:
      background var(--transition-base),
      color var(--transition-base);
  }

  .corpus__row-title,
  .corpus__row-found {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .corpus__row-found {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-style: italic;
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

  .corpus__insert {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
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
