<script module lang="ts">
  export type ResearchTab = 'corpus' | 'zotero' | 'notes' | 'agent'
</script>

<script lang="ts">
  import { TabButton, TabList } from '@entropia/ui'
  import { t, type I18nKey } from '$lib/i18n'
  import WritingCorpusTab from './WritingCorpusTab.svelte'
  import WritingNotesTab from './WritingNotesTab.svelte'
  import WritingZoteroTab from './WritingZoteroTab.svelte'
  import WritingAgentTab from './WritingAgentTab.svelte'
  import type { SuggestionRow } from '$lib/writing-agent'

  /**
   * The right-hand panel of the three-panel shell (plan-editor.md §6.3).
   *
   * The shell is this unit's work; the bodies belong to the units that own each
   * source of evidence — Corpus to Unit 4, Notas to 5, Zotero to 6, Agente to
   * 7. Each tab therefore names the work it is waiting on: an empty tab that
   * says nothing reads as a defect rather than as something not built yet.
   */

  interface Props {
    tab?: ResearchTab
    /** Raised when a tab wants a citation put into the manuscript. */
    oninsertcitation?: (attrs: Record<string, unknown>) => string | null
    /** Raised to put a note's words in as independent text (§13). */
    oncopynote?: (text: string) => boolean
    /** Raised to put a live link to a note in (§13). */
    onlinknote?: (attrs: Record<string, unknown>) => string | null
    /** The passage selected in the manuscript, for writing it down (§13.1). */
    selection?: () => string
    /** Raised to put a bibliographic citation in (§11.5). */
    oncitezotero?: (attrs: Record<string, unknown>) => string | null
    /** The manuscript the agent is assisting with, when one is open (§14). */
    documentId?: string | null
    /** Whether a chat model is configured, so the agent can be asked (§14.1). */
    hasChat?: boolean
    /** The revision an agent proposal is made against, recorded with it (§14). */
    sourceRevision?: number
    /** Whether a proposal's target is still in the manuscript, word for word. */
    passagePresent?: (passage: string) => boolean
    /** Raised to put an accepted proposal into the manuscript (§14.2). */
    onapplysuggestion?: (suggestion: SuggestionRow, text: string, below: boolean) => void
  }

  let {
    tab = $bindable<ResearchTab>('corpus'),
    oninsertcitation,
    oncopynote,
    onlinknote,
    selection,
    oncitezotero,
    documentId = null,
    hasChat = false,
    sourceRevision = 0,
    passagePresent,
    onapplysuggestion,
  }: Props = $props()

  const TABS: { id: ResearchTab; label: I18nKey; pending: I18nKey }[] = [
    { id: 'corpus', label: 'writing.tab.corpus', pending: 'writing.tabPending.corpus' },
    { id: 'zotero', label: 'writing.tab.zotero', pending: 'writing.tabPending.zotero' },
    { id: 'notes', label: 'writing.tab.notes', pending: 'writing.tabPending.notes' },
    { id: 'agent', label: 'writing.tab.agent', pending: 'writing.tabPending.agent' },
  ]

  const active = $derived(TABS.find((entry) => entry.id === tab) ?? TABS[0])
</script>

<section class="research" aria-label={t('writing.research')}>
  <TabList class="research__tabs" aria-label={t('writing.research')}>
    {#each TABS as entry (entry.id)}
      <TabButton
        id="writing-research-tab-{entry.id}"
        active={tab === entry.id}
        aria-controls="writing-research-panel-{entry.id}"
        onclick={() => (tab = entry.id)}
      >
        {t(entry.label)}
      </TabButton>
    {/each}
  </TabList>

  <!-- Only the chosen body is rendered. Four panels with three hidden would
       still be four panels to a screen reader walking the tree. -->
  {#if active}
    <div
      class="research__body"
      id="writing-research-panel-{active.id}"
      role="tabpanel"
      aria-labelledby="writing-research-tab-{active.id}"
      tabindex="0"
    >
      {#if active.id === 'corpus'}
        <WritingCorpusTab {oninsertcitation} />
      {:else if active.id === 'zotero'}
        <WritingZoteroTab oncite={oncitezotero} />
      {:else if active.id === 'notes'}
        <WritingNotesTab oncopy={oncopynote} onlink={onlinknote} {selection} />
      {:else if active.id === 'agent'}
        <WritingAgentTab
          {documentId}
          {selection}
          {hasChat}
          {sourceRevision}
          {passagePresent}
          onapply={onapplysuggestion}
        />
      {:else}
        <p class="research__pending">{t(active.pending)}</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .research {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }

  /* `TabList` is `inline-flex` with no wrapping and no width of its own, which
     suits the two or three tabs its other callers have. Four of them in a fixed
     280px column overflow the panel, so here the row is told to fill the column
     and share it out. Scoped to this section rather than changed in the shared
     primitive, which the item and collection panels also use.

     `flex-wrap` is the backstop: at a narrower column the tabs drop to a second
     row instead of spilling out again. */
  .research :global(.research__tabs) {
    display: flex;
    flex-wrap: wrap;
    width: 100%;
    box-sizing: border-box;
  }

  .research :global(.research__tabs > button) {
    flex: 1 1 auto;
    min-width: 0;
    padding: 0 var(--space-2);
  }

  .research__body {
    min-height: 0;
    padding: var(--space-2);
    overflow-y: auto;
  }

  .research__body:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    border-radius: var(--radius-surface);
  }

  .research__pending {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-base);
  }
</style>
