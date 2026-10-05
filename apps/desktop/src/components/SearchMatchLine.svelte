<script lang="ts">
  import { t } from '$lib/i18n'

  /**
   * The small line under a search result that says why it is there. One
   * component for every writing search (Corpus, Zotero, Obras), so the three
   * read the same: "Exacto: …", "Aproximado: …" or "Por significado".
   */
  interface Props {
    kind: 'exact' | 'approximate' | 'meaning'
    /** The words behind an exact or approximate match. */
    terms?: string[]
  }

  let { kind, terms = [] }: Props = $props()

  let words = $derived(terms.join(', '))
</script>

{#if kind === 'approximate'}
  <span class="match-line">{t('writing.corpusFoundAs', { words })}</span>
{:else if kind === 'exact'}
  {#if words}
    <span class="match-line">{t('writing.searchFoundExact', { words })}</span>
  {/if}
{:else if kind === 'meaning'}
  <span class="match-line">{t('writing.searchFoundMeaning')}</span>
{/if}

<style>
  .match-line {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-style: italic;
  }
</style>
