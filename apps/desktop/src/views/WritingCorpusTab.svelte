<script lang="ts">
  import { onDestroy } from 'svelte'
  import { ActionIcon, Button, Checkbox, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { FtsSearchController } from '$lib/item-view-search'
  import { corpusPageLabel, writingCorpus } from '$lib/writing-corpus'
  import { hashSourceText, selectionRange } from '$lib/source-selection'
  import { wordRanges } from '$lib/text-highlight'
  import { getAssetUrl } from '$lib/file-import'
  import { mapRenderedText, type RenderedTextMap } from '$lib/rendered-text-map'
  import { highlightRanges, renderedSelection, type RenderedChoice } from '$lib/rendered-selection'
  import OcrRichText from '../components/OcrRichText.svelte'

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
  void store.loadPreferences()

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

  const HIGHLIGHT = 'corpus-match'

  /** The plain text of a transcription: one bare text node (see below). */
  let textElement = $state<HTMLParagraphElement | undefined>(undefined)
  /** The scroll box around rendered OCR. */
  let renderedScroll = $state<HTMLDivElement | undefined>(undefined)
  /**
   * The rendered OCR and the map from its visible text back to the raw one.
   * Set when OcrRichText finishes a render — it renders asynchronously, so
   * there is nothing to select or mark before that.
   */
  let rendered = $state.raw<{ box: HTMLDivElement; map: RenderedTextMap } | null>(null)
  /** The fragment selected: raw offsets to anchor, visible words to quote. */
  let chosen = $state<RenderedChoice | null>(null)
  /** Something is selected in the rendered text but cannot be anchored. */
  let unmapped = $state(false)
  let inserted = $state(false)

  const openPage = $derived(snapshot.pages.find((page) => page.assetId === snapshot.openPageId))

  // A new page is a new render; the previous one's map says nothing about it.
  $effect(() => {
    void snapshot.openPageId
    void snapshot.pageText
    rendered = null
  })

  /**
   * Watches the document's selection rather than listening on the text.
   *
   * Reading a selection is observing, not interacting — putting mouse and key
   * handlers on the text claims otherwise, which is what the accessibility
   * guard objects to. It also catches the selections those handlers missed:
   * double click to take a word, Shift+arrow, and select-all inside the pane.
   *
   * OCR is selected on its rendered form and mapped back to the raw extraction
   * (rendered-text-map.ts); a transcription is plain text and read directly.
   */
  $effect(() => {
    const read = () => {
      const selection = window.getSelection()
      if (snapshot.pageTextKind === 'extraction') {
        const range = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null
        const result = rendered ? renderedSelection(rendered.box, range, rendered.map) : null
        unmapped = result === 'unmapped'
        chosen = result === 'unmapped' ? null : result
      } else {
        const plain = selectionRange(textElement ?? null, selection)
        unmapped = false
        chosen = plain ? { start: plain.start, end: plain.end, quote: plain.text } : null
      }
      inserted = false
    }
    document.addEventListener('selectionchange', read)
    return () => document.removeEventListener('selectionchange', read)
  })

  /**
   * Marks the searched words — and the variants an approximate find was found
   * as — in the page text, and scrolls the first one into view.
   *
   * Through the CSS Custom Highlight API rather than `<mark>`: the paragraph
   * must stay one bare text node, because its DOM offsets are the citation's
   * character offsets (see the comment on it below). A highlight paints ranges
   * without adding a single node. Where the API is missing, nothing is marked
   * and everything else works as before.
   */
  $effect(() => {
    const words = snapshot.highlight
    if (typeof CSS === 'undefined' || !('highlights' in CSS)) return

    let ranges: Range[]
    let scroller: HTMLElement
    if (snapshot.pageTextKind === 'extraction') {
      // Rendered OCR: the words, wherever formatting put their text nodes.
      if (!rendered || !renderedScroll) return
      ranges = highlightRanges(rendered.box, words)
      scroller = renderedScroll
    } else {
      const node = textElement?.firstChild
      if (!textElement || !node || node.nodeType !== Node.TEXT_NODE) return
      const length = (node as Text).length
      ranges = wordRanges(snapshot.pageText, words)
        .filter(([, end]) => end <= length)
        .map(([start, end]) => {
          const range = new Range()
          range.setStart(node, start)
          range.setEnd(node, end)
          return range
        })
      scroller = textElement
    }
    CSS.highlights.set(HIGHLIGHT, new Highlight(...ranges))

    const first = ranges[0]
    if (first) {
      const box = scroller.getBoundingClientRect()
      const at = first.getBoundingClientRect()
      scroller.scrollTop += at.top - box.top - box.height / 3
    }
    return () => {
      CSS.highlights.delete(HIGHLIGHT)
    }
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

    // The hash is of the RAW words at those offsets — what citation-target.ts
    // checks when the citation is followed. The quote is what the reader saw:
    // for rendered OCR it has no `#` or tags, for plain text the two coincide.
    const sourceTextHash = await hashSourceText(snapshot.pageText.slice(chosen.start, chosen.end))
    const id = oninsertcitation({
      collectionId: snapshot.openItem.collectionId,
      itemId: snapshot.openItem.itemId,
      assetId: page.assetId,
      pageNumber: page.pageNumber,
      startChar: chosen.start,
      endChar: chosen.end,
      quotedText: chosen.quote,
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

  <!-- One switch for every search in the app (search-preferences.ts). -->
  <Checkbox
    class="corpus__fuzzy"
    checked={snapshot.fuzzy}
    onchange={(checked) => void store.setFuzzy(checked)}
  >
    {t('writing.corpusFuzzy')}
  </Checkbox>

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
    {:else if snapshot.pages.length === 1}
      <!-- One file, nothing to choose: its name, and the text already open. -->
      <p class="corpus__label">{t('writing.corpusFile')}</p>
      <p class="corpus__file">{corpusPageLabel(snapshot.pages[0]!, t)}</p>
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
        {#if snapshot.pageTextKind === 'extraction'}
          <!-- OCR is shown the way the item's extracted-text tab shows it. A
               selection here is mapped back to the raw extraction before it
               becomes a citation, and refused if it cannot be (see the notice
               below) — never anchored to other words. -->
          <div class="corpus__text corpus__text--rendered" bind:this={renderedScroll}>
            <OcrRichText
              text={snapshot.pageText}
              assetUrl={openPage ? getAssetUrl(openPage.path) : ''}
              sourceType={openPage?.type === 'pdf' ? 'pdf' : 'image'}
              referenceWidth={0}
              referenceHeight={0}
              onrendered={(box) => {
                rendered = { box, map: mapRenderedText(box.textContent ?? '', snapshot.pageText) }
              }}
            />
          </div>
        {:else}
          <!-- One bare text node, deliberately: the DOM offsets inside it are the
               character offsets into the transcription, and any markup in here
               would break that identity. `selectionRange` refuses rather than
               mis-anchoring if it ever does. -->
          <p class="corpus__text" bind:this={textElement}>{snapshot.pageText}</p>
        {/if}

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
            {:else if unmapped}
              {t('writing.corpusSelectionUnmapped')}
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

  .corpus :global(.corpus__fuzzy) {
    padding: 0 var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
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

  .corpus__file {
    margin: 0;
    padding: 0 var(--space-2);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
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

  /* Rendered OCR lays itself out; pre-wrap would turn the newlines between its
     blocks into blank lines. */
  .corpus__text--rendered {
    white-space: normal;
  }

  /* The same mark as a search hit elsewhere (.fts-match in ItemSearchPanel).
     A highlight is styled on the element that holds the text, and rendered OCR
     holds it in headings, paragraphs and cells — so descendants too. */
  .corpus__text::highlight(corpus-match),
  .corpus__text--rendered :global(*)::highlight(corpus-match) {
    background-color: color-mix(in srgb, var(--color-warning) 30%, transparent);
    color: var(--color-text-primary);
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
