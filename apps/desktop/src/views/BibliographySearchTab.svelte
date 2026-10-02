<script lang="ts">
  import { t } from '$lib/i18n'
  import {
    bibliographySearchWorks,
    type BibliographySearchHit,
    type BibliographySearchResponse,
  } from '$lib/bibliography-search'
  import { ActionIcon, Button, Card } from '@entropia/ui'

  let query = $state('')
  let searching = $state(false)
  let answer = $state<BibliographySearchResponse | null>(null)
  let error = $state<string | null>(null)
  let selectionEmpty = $state(false)

  interface Props {
    /** Reads the manuscript selection for anchored search (E6a). */
    getSelection?: () => string
  }

  let { getSelection }: Props = $props()

  function methodLabel(hit: BibliographySearchHit): string {
    switch (hit.method) {
      case 'hybrid':
        return t('bibliography.searchMethodHybrid')
      case 'vector':
        return t('bibliography.searchMethodVector')
      default:
        return t('bibliography.searchMethodLexical')
    }
  }

  function shortHash(hash: string | null): string {
    if (!hash) return '—'
    return hash.length > 12 ? `${hash.slice(0, 12)}…` : hash
  }

  async function handleSearch(event?: Event): Promise<void> {
    event?.preventDefault()
    await runSearch(query.trim())
  }

  async function runSearch(text: string): Promise<void> {
    if (!text || searching) return
    searching = true
    error = null
    try {
      answer = await bibliographySearchWorks(text)
    } catch (failure) {
      answer = null
      error = failure instanceof Error ? failure.message : String(failure)
    } finally {
      searching = false
    }
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
</script>

<Card>
  <h3>{t('bibliography.searchTitle')}</h3>
  <p>{t('bibliography.searchHint')}</p>
  <form class="bib-search__form" onsubmit={handleSearch}>
    <label class="bib-search__label" for="bib-search-query">
      {t('bibliography.searchLabel')}
    </label>
    <div class="bib-search__field">
      <span class="search-field__icon" aria-hidden="true">
        <ActionIcon name="search" size={16} />
      </span>
      <input
        id="bib-search-query"
        class="bib-search__input"
        type="search"
        placeholder={t('bibliography.searchPlaceholder')}
        disabled={searching}
        bind:value={query}
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
  <p class="bib-search__consent">{t('bibliography.searchConsent')}</p>
  {#if selectionEmpty}
    <p class="bib-search__error" role="alert">{t('bibliography.searchSelectionEmpty')}</p>
  {/if}

  {#if error}
    <p class="bib-search__error" role="alert">{error}</p>
  {/if}

  {#if answer}
    {#if !answer.vectorAvailable}
      <p class="bib-search__notice">{t('bibliography.searchLexicalOnly')}</p>
    {/if}
    {#if answer.hits.length === 0}
      <p>{t('bibliography.searchEmpty')}</p>
    {:else}
      <ul class="bib-search__results">
        {#each answer.hits as hit (hit.itemId)}
          <li class="bib-search__hit">
            <div class="bib-search__hit-head">
              <span class="bib-search__title">{hit.title}</span>
              <span class="bib-search__method">{methodLabel(hit)}</span>
            </div>
            <p class="bib-search__meta">
              {hit.itemKey} · {t('bibliography.searchSimilarity', {
                score: hit.vectorScore ?? hit.fusedScore,
              })} · {shortHash(hit.contractHash)}
            </p>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</Card>

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
    position: relative;
    flex: 1;
  }

  .bib-search__input {
    width: 100%;
    box-sizing: border-box;
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-family: var(--font-ui);
    padding: var(--space-2) var(--space-3);
    padding-inline-start: var(--search-field-inset);
    border: 1px solid var(--border-subtle);
    outline: none;
  }

  .bib-search__error {
    color: var(--color-text-danger, inherit);
  }

  .bib-search__notice {
    margin-bottom: var(--space-2);
  }

  .bib-search__results {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .bib-search__hit-head {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .bib-search__meta {
    margin: 0;
    color: var(--color-text-secondary);
  }
</style>
