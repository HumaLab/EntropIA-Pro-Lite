<script lang="ts">
  import { t } from '$lib/i18n'
  import { onMount } from 'svelte'
  import {
    bibliographySearchPassages,
    bibliographyTab,
    type BibliographyPassage,
  } from '$lib/bibliography-search'
  import {
    locationText,
    locatorOf,
    passageHeading,
    passagesNoticeKey,
    withNoticeDetail,
  } from '$lib/rag-scope'
  import { searchPreferences } from '$lib/search-preferences'
  import PassageReaderDialog from '../components/PassageReaderDialog.svelte'
  import SearchFuzzyToggle from '../components/SearchFuzzyToggle.svelte'
  import SearchMatchLine from '../components/SearchMatchLine.svelte'
  import { ActionIcon, Button, Card, IconButton, SearchBar } from '@entropia/ui'

  // The tab's state lives in a store so it survives leaving the tab.
  const kept = bibliographyTab.state
  let query = $state(kept.query)
  let searching = $state(false)
  let selectionEmpty = $state(false)
  let passageAnswer = $state(kept.passageAnswer)
  let passagesFailed = $state(kept.passagesFailed)
  let openedPassage = $state<BibliographyPassage | null>(null)
  let cited = $state(false)
  /** Whether close variants of the words are searched too (the shared preference). */
  let fuzzy = $state(true)
  void searchPreferences.fuzzyEnabled().then((enabled) => (fuzzy = enabled))

  async function setFuzzy(enabled: boolean): Promise<void> {
    fuzzy = enabled
    try {
      await searchPreferences.setFuzzyEnabled(enabled)
    } catch {
      // The switch still applies to this session; only remembering it failed.
    }
    if (query.trim()) await runSearch(query.trim())
  }

  interface Props {
    /** Reads the manuscript selection for anchored search (E6a). */
    getSelection?: () => string
    /** Inserts a Zotero citation at the caret. Absent when no manuscript is open. */
    oncite?: (attrs: Record<string, unknown>) => string | null
  }

  let { getSelection, oncite }: Props = $props()

  let root: HTMLElement | undefined = $state()

  // The scroll box is the research panel around the tab. Its offset is kept
  // while the tab is open and put back when the tab returns.
  onMount(() => {
    const panel = root?.closest<HTMLElement>('[role="tabpanel"]')
    if (!panel) return
    const top = bibliographyTab.state.scrollTop
    if (top > 0) requestAnimationFrame(() => (panel.scrollTop = top))
    const keep = () => bibliographyTab.patch({ scrollTop: panel.scrollTop })
    panel.addEventListener('scroll', keep, { passive: true })
    return () => panel.removeEventListener('scroll', keep)
  })

  function clearSearch(): void {
    query = ''
    passageAnswer = null
    passagesFailed = false
    cited = false
    bibliographyTab.reset()
  }

  /** Passages grouped under their work, in the order each work first appears. */
  let passageGroups = $derived.by(() => {
    const groups = new Map<
      string,
      { first: BibliographyPassage; passages: BibliographyPassage[] }
    >()
    for (const passage of passageAnswer?.passages ?? []) {
      const group = groups.get(passage.itemId)
      if (group) group.passages.push(passage)
      else groups.set(passage.itemId, { first: passage, passages: [passage] })
    }
    return [...groups.values()]
  })
  let passagesNotice = $derived(
    passagesFailed ? passagesNoticeKey('failed') : passagesNoticeKey(passageAnswer?.notice)
  )
  function passagesNoticeText(): string {
    if (!passagesNotice) return ''
    const text = t(passagesNotice)
    if (passagesFailed) return text
    return withNoticeDetail(text, passageAnswer?.notice, passageAnswer?.noticeDetail)
  }

  async function handleSearch(event?: Event): Promise<void> {
    event?.preventDefault()
    await runSearch(query.trim())
  }

  async function runSearch(text: string): Promise<void> {
    if (!text || searching) return
    searching = true
    cited = false
    passagesFailed = false
    try {
      passageAnswer = await bibliographySearchPassages(text, { fuzzy })
    } catch {
      passageAnswer = null
      passagesFailed = true
    }
    bibliographyTab.patch({ query: text, passageAnswer, passagesFailed })
    searching = false
  }

  // E6a: anchored search. The manuscript selection becomes the query
  // verbatim — no hidden expansion — and only on explicit click: the
  // click is the scoped consent, the visible query box is the scope.
  async function handleSearchSelection(): Promise<void> {
    const selected = getSelection?.() ?? ''
    if (!selected.trim()) {
      selectionEmpty = true
      return
    }
    selectionEmpty = false
    query = selected.trim()
    await runSearch(query)
  }

  function citePassage(passage: BibliographyPassage) {
    if (!oncite || !passage.cslJson) return
    cited =
      oncite({
        sourceOrigin: 'local',
        sourceInstanceId: null,
        itemKey: passage.itemKey,
        // The catalog does not keep Zotero's item version: unknown, not zero.
        itemVersion: null,
        libraryType: passage.libraryType,
        libraryId: passage.libraryNativeId,
        metadataSnapshot: passage.cslJson,
        ...(locatorOf(passage.location) ?? {}),
      }) !== null
  }
</script>

<div bind:this={root}>
  <Card>
    <h3>{t('bibliography.searchTitle')}</h3>
    <p>{t('bibliography.searchHint')}</p>
    <form class="bib-search__form" onsubmit={handleSearch}>
      <label class="bib-search__label" for="bib-search-query">
        {t('bibliography.searchLabel')}
      </label>
      <div class="bib-search__field">
        <SearchBar
          id="bib-search-query"
          value={query}
          placeholder={t('bibliography.searchPlaceholder')}
          ariaLabel={t('bibliography.searchLabel')}
          clearAriaLabel={t('topbar.searchClear')}
          disabled={searching}
          emitSearch={false}
          onvaluechange={(next) => {
            query = next
            bibliographyTab.patch({ query: next })
          }}
          onclear={clearSearch}
        />
      </div>
      <Button variant="secondary" size="sm" type="submit" loading={searching}>
        {t('bibliography.searchAction')}
      </Button>
      {#if getSelection}
        <Button
          variant="ghost"
          size="sm"
          type="button"
          disabled={searching}
          onclick={handleSearchSelection}
        >
          {t('bibliography.searchFromSelection')}
        </Button>
      {/if}
    </form>
    <!-- One switch for every search in the app (search-preferences.ts). -->
    <SearchFuzzyToggle checked={fuzzy} onchange={(checked) => void setFuzzy(checked)} />
    <p class="bib-search__consent">{t('bibliography.searchConsent')}</p>
    {#if selectionEmpty}
      <p class="bib-search__error" role="alert">{t('bibliography.searchSelectionEmpty')}</p>
    {/if}

    {#if passageAnswer || passagesFailed}
      <h4 class="bib-search__section">{t('bibliography.passagesTitle')}</h4>
      {#if passagesNotice}
        <p class="bib-search__notice" role="status">{passagesNoticeText()}</p>
      {:else if passageGroups.length === 0}
        <p>{t('bibliography.passagesEmpty')}</p>
      {:else}
        <div class="bib-search__groups">
          {#each passageGroups as group (group.first.itemId)}
            <div class="bib-search__group" role="group" aria-label={group.first.title}>
              <div class="bib-search__group-head">
                <span class="bib-search__group-title">{group.first.title}</span>
                <span class="bib-search__meta">
                  {passageHeading(
                    {
                      authors: group.first.authors,
                      year: group.first.year,
                      libraryName: group.first.libraryName,
                    },
                    null
                  )}
                </span>
              </div>
              <ul class="bib-search__passages">
                {#each group.passages as passage (passage.chunkId)}
                  <li class="bib-search__passage">
                    <span class="bib-search__passage-body">
                      <span class="bib-search__snippet">{passage.snippet}</span>
                      {#if passage.location}
                        <span class="bib-search__meta">{locationText(passage.location)}</span>
                      {/if}
                      <SearchMatchLine kind={passage.matchKind} terms={passage.matchTerms} />
                    </span>
                    <span class="bib-search__row-actions">
                      <IconButton
                        size="sm"
                        label={t('bibliography.passageOpen')}
                        onclick={() => (openedPassage = passage)}
                      >
                        <ActionIcon name="eye" size={14} />
                      </IconButton>
                      <IconButton
                        size="sm"
                        label={t('bibliography.passageCite')}
                        disabled={!oncite || !passage.cslJson}
                        onclick={() => citePassage(passage)}
                      >
                        <ActionIcon name="text-quote" size={14} />
                      </IconButton>
                    </span>
                  </li>
                {/each}
              </ul>
            </div>
          {/each}
        </div>
        <p class="bib-search__notice" role="status">
          {#if cited}
            {t('writing.zoteroCited')}
          {:else if !oncite}
            {t('writing.zoteroNoDocument')}
          {/if}
        </p>
      {/if}
    {/if}
  </Card>
</div>

{#if openedPassage}
  {#key openedPassage.chunkId}
    <PassageReaderDialog
      chunkId={openedPassage.chunkId}
      title={openedPassage.title}
      heading={passageHeading(openedPassage, openedPassage.location)}
      fallbackSnippet={openedPassage.snippet}
      onclose={() => (openedPassage = null)}
    />
  {/key}
{/if}

<style>
  .bib-search__form {
    display: flex;
    gap: var(--space-2);
    align-items: end;
    margin-bottom: var(--space-3);
  }

  .bib-search__label {
    display: block;
    margin-bottom: var(--space-1);
  }

  .bib-search__field {
    flex: 1;
  }

  .bib-search__error {
    color: var(--color-text-danger, inherit);
  }

  .bib-search__notice {
    margin-bottom: var(--space-2);
  }

  .bib-search__meta {
    margin: 0;
    color: var(--color-text-secondary);
  }

  .bib-search__section {
    margin: var(--space-3) 0 var(--space-2);
  }

  .bib-search__groups {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .bib-search__group-head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 var(--space-2);
  }

  .bib-search__group-title {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    overflow-wrap: anywhere;
  }

  .bib-search__group-head .bib-search__meta {
    font-size: var(--font-size-2xs);
  }

  .bib-search__passages {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* The same row as the Zotero tab's: text on the left, eye + quote on the right. */
  .bib-search__passage {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-control);
  }

  .bib-search__passage:hover {
    background: var(--color-accent-faint);
  }

  .bib-search__passage-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .bib-search__snippet {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
  }

  .bib-search__passage-body .bib-search__meta {
    font-size: var(--font-size-2xs);
  }

  .bib-search__row-actions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
</style>
