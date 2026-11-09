<!--
  Phrases and associations of a collection's texts: its frequent n-grams, the
  words that keep company with a chosen word (log-likelihood G²) and a word
  tree of what follows or precedes it. All computed here, from the corpus the
  analysis panel already loaded.
-->
<script lang="ts">
  import { Button, NumberField, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { DEFAULT_STOPWORDS } from '$lib/stopwords'
  import {
    buildCollocates,
    buildNgrams,
    buildWordTree,
    type CorpusText,
    type WordTreeNode,
  } from '$lib/text-analysis'

  let { texts, customStopwords }: { texts: CorpusText[]; customStopwords: string[] } = $props()

  const PHRASE_LIMIT = 30
  const COLLOCATE_LIMIT = 20

  let length = $state<number | null>(3)
  let word = $state('')
  let side = $state<'right' | 'left'>('right')

  const stopwords = $derived(
    customStopwords.length > 0 ? new Set([...DEFAULT_STOPWORDS, ...customStopwords]) : undefined
  )
  const options = $derived(stopwords ? { stopwords } : {})
  const n = $derived(Math.min(5, Math.max(2, Math.round(length ?? 3))))
  const phrases = $derived(buildNgrams(texts, n, options).slice(0, PHRASE_LIMIT))
  const target = $derived(word.trim())
  const collocates = $derived(
    target ? buildCollocates(texts, target, options).slice(0, COLLOCATE_LIMIT) : []
  )
  const tree = $derived(target ? buildWordTree(texts, target, { side, depth: 3 }) : null)
</script>

{#snippet branch(node: WordTreeNode, root: boolean)}
  <li
    role="treeitem"
    aria-selected="false"
    aria-expanded={node.children.length > 0 ? true : undefined}
  >
    <span class="phrases__node" class:phrases__node--root={root}>{node.word}</span>
    <span class="phrases__count">{node.count}</span>
    {#if node.children.length > 0}
      <ul role="group" class="phrases__tree-level">
        {#each node.children as child (child.word)}
          {@render branch(child, false)}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<div class="phrases">
  <section class="phrases__section">
    <h3 class="phrases__title">{t('phraseAnalysis.phrasesTitle')}</h3>
    <div class="phrases__field">
      <span class="phrases__label number-field-label" id="phrase-length-label"
        >{t('phraseAnalysis.length')}</span
      >
      <NumberField
        bind:value={length}
        min={2}
        max={5}
        aria-labelledby="phrase-length-label"
        decrementLabel={t('numberField.decrease')}
        incrementLabel={t('numberField.increase')}
      />
    </div>
    {#if phrases.length > 0}
      <ul class="phrases__list" aria-label={t('phraseAnalysis.phrasesTitle')}>
        {#each phrases as phrase (phrase.gram)}
          <li class="phrases__row">
            <span>{phrase.gram}</span>
            <span class="phrases__count">{phrase.count}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="phrases__empty">{t('phraseAnalysis.noPhrases')}</p>
    {/if}
  </section>

  <section class="phrases__section">
    <h3 class="phrases__title">{t('phraseAnalysis.wordTitle')}</h3>
    <span class="phrases__label" aria-hidden="true">{t('phraseAnalysis.word')}</span>
    <div class="phrases__search">
      <SearchBar
        id="phrase-word"
        value={word}
        ariaLabel={t('phraseAnalysis.word')}
        placeholder={t('phraseAnalysis.wordPlaceholder')}
        autocomplete="off"
        emitSearch={false}
        onvaluechange={(next) => (word = next)}
      />
    </div>

    {#if tree && tree.count === 0}
      <p class="phrases__empty">{t('phraseAnalysis.absent', { word: target })}</p>
    {:else if tree}
      <h4 class="phrases__subtitle">{t('phraseAnalysis.collocatesTitle')}</h4>
      {#if collocates.length > 0}
        <ul class="phrases__list" aria-label={t('phraseAnalysis.collocatesTitle')}>
          {#each collocates as collocate (collocate.word)}
            <li class="phrases__row">
              <span>{collocate.word}</span>
              <span class="phrases__count">
                {t('phraseAnalysis.collocateStats', {
                  count: collocate.count,
                  g2: collocate.g2.toFixed(1),
                })}
              </span>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="phrases__empty">{t('phraseAnalysis.noCollocates')}</p>
      {/if}

      <div class="phrases__tree-head">
        <h4 class="phrases__subtitle">{t('phraseAnalysis.treeTitle')}</h4>
        <div class="phrases__sides">
          <Button
            variant={side === 'right' ? 'secondary' : 'ghost'}
            size="sm"
            aria-pressed={side === 'right'}
            onclick={() => (side = 'right')}>{t('phraseAnalysis.after')}</Button
          >
          <Button
            variant={side === 'left' ? 'secondary' : 'ghost'}
            size="sm"
            aria-pressed={side === 'left'}
            onclick={() => (side = 'left')}>{t('phraseAnalysis.before')}</Button
          >
        </div>
      </div>
      <ul class="phrases__tree" role="tree" aria-label={t('phraseAnalysis.treeTitle')}>
        {@render branch(tree, true)}
      </ul>
    {/if}
  </section>
</div>

<style>
  .phrases {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: var(--space-4);
  }

  .phrases__section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .phrases__title {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .phrases__subtitle {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .phrases__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .phrases__label {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .phrases__search {
    max-width: 18rem;
  }

  .phrases__list {
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 22rem;
    overflow-y: auto;
  }

  .phrases__row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-1) 0;
    font-size: var(--font-size-sm);
    border-bottom: 1px solid var(--color-hairline);
  }

  .phrases__count {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
    color: var(--color-text-muted);
  }

  .phrases__empty {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }

  .phrases__tree-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .phrases__sides {
    display: flex;
    gap: var(--space-1);
  }

  .phrases__tree,
  .phrases__tree-level {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .phrases__tree-level {
    padding-left: var(--space-4);
    border-left: 1px solid var(--color-hairline);
  }

  .phrases__tree li {
    font-size: var(--font-size-sm);
    line-height: var(--line-height-base);
  }

  .phrases__node {
    margin-right: var(--space-2);
  }

  .phrases__node--root {
    font-weight: var(--font-weight-bold);
    color: var(--color-accent);
  }
</style>
