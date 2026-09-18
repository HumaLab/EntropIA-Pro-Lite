<script module lang="ts">
  export type ResearchTab = 'corpus' | 'zotero' | 'notes' | 'agent' | 'export'
</script>

<script lang="ts">
  import { TabButton, TabList } from '@entropia/ui'
  import { t, type I18nKey } from '$lib/i18n'
  import WritingCorpusTab from './WritingCorpusTab.svelte'
  import WritingNotesTab from './WritingNotesTab.svelte'
  import WritingZoteroTab from './WritingZoteroTab.svelte'
  import WritingAgentTab from './WritingAgentTab.svelte'
  import WritingExportTab from './WritingExportTab.svelte'
  import { DEFAULT_EXPORT_PREFERENCES, type ExportPreferences } from '$lib/export-preferences'
  import type { Node } from '$lib/export-document'
  import type { SuggestionRow } from '$lib/writing-agent'
  import type { CitationDraft, CitationEditSession } from './WritingCitationEditor.svelte'

  /**
   * The right-hand panel of the three-panel shell (plan-editor.md §6.3).
   *
   * The shell is this unit's work; the bodies belong to the units that own each
   * source of evidence — Corpus to Unit 4, Notas to 5, Zotero to 6, Agente to
   * 7. Each tab therefore names the work it is waiting on: an empty tab that
   * says nothing reads as a defect rather than as something not built yet.
   *
   * Exportar is the odd one out: not evidence but how the manuscript leaves the
   * application (§17.2). It sits last, after the four sources, and holds only
   * preferences — the download button in the bar is what exports.
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
    /** Draft shown inside Zotero while one citation is being adjusted. */
    citationDraft?: CitationEditSession | null
    /** Persists incomplete fields while another research tab is visible. */
    oncitationdraftchange?: (draft: CitationDraft) => void
    onapplycitation?: (attrs: Record<string, unknown>) => void
    oncancelcitation?: () => void
    /** The manuscript the agent is assisting with, when one is open (§14). */
    documentId?: string | null
    /** Whether a chat model is configured, so the agent can be asked (§14.1). */
    hasChat?: boolean
    /** Whether corpus retrieval can run, for the four evidence actions (§14.1). */
    hasRetrieval?: boolean
    /** The revision an agent proposal is made against, recorded with it (§14). */
    sourceRevision?: number
    /** Whether a proposal's target is still in the manuscript, word for word. */
    passagePresent?: (passage: string) => boolean
    /** Raised to put an accepted proposal into the manuscript (§14.2). */
    onapplysuggestion?: (suggestion: SuggestionRow, text: string, below: boolean) => void
    /** How the manuscript is exported, shown and changed in the Export tab. */
    exportPreferences?: ExportPreferences
    onexportpreferences?: (next: ExportPreferences) => void
    /** The manuscript on screen, for the Export tab's statistics. */
    manuscript?: Node | null
  }

  let {
    tab = $bindable<ResearchTab>('corpus'),
    oninsertcitation,
    oncopynote,
    onlinknote,
    selection,
    oncitezotero,
    citationDraft = null,
    oncitationdraftchange,
    onapplycitation,
    oncancelcitation,
    documentId = null,
    hasChat = false,
    hasRetrieval = false,
    sourceRevision = 0,
    passagePresent,
    onapplysuggestion,
    exportPreferences = DEFAULT_EXPORT_PREFERENCES,
    onexportpreferences = () => {},
    manuscript = null,
  }: Props = $props()

  const TABS: { id: ResearchTab; label: I18nKey; pending: I18nKey }[] = [
    { id: 'corpus', label: 'writing.tab.corpus', pending: 'writing.tabPending.corpus' },
    { id: 'zotero', label: 'writing.tab.zotero', pending: 'writing.tabPending.zotero' },
    { id: 'notes', label: 'writing.tab.notes', pending: 'writing.tabPending.notes' },
    { id: 'agent', label: 'writing.tab.agent', pending: 'writing.tabPending.agent' },
    { id: 'export', label: 'writing.tab.export', pending: 'writing.tabPending.export' },
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

  <!-- Only the chosen body is rendered. Five panels with four hidden would
       still be five panels to a screen reader walking the tree. -->
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
        <WritingZoteroTab
          oncite={oncitezotero}
          {citationDraft}
          {oncitationdraftchange}
          {onapplycitation}
          {oncancelcitation}
        />
      {:else if active.id === 'notes'}
        <WritingNotesTab oncopy={oncopynote} onlink={onlinknote} {selection} />
      {:else if active.id === 'agent'}
        <WritingAgentTab
          {documentId}
          {selection}
          {hasChat}
          {hasRetrieval}
          {sourceRevision}
          {passagePresent}
          onapply={onapplysuggestion}
        />
      {:else if active.id === 'export'}
        <WritingExportTab
          preferences={exportPreferences}
          onchange={onexportpreferences}
          doc={manuscript}
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
    flex: 1;
    width: 100%;
    min-height: 0;
    overflow: hidden;
  }

  /* `TabList` is `inline-flex` with no wrapping and no width of its own, which
     suits the two or three tabs its other callers have. Four of them in a fixed
     280px column overflow the panel, so here the row is told to fill the column
     and share it out. Scoped to this section rather than changed in the shared
     primitive, which the item and collection panels also use.

     `flex: none` because this section is a flex column with a definite height,
     and a row that may shrink is a row that pays for the body's overflow out of
     its own padding. The Agent tab is the only body taller than the column, and
     there the row was squeezed from 39px down to the 32px floor `TabList` sets:
     4px of padding above and below the tabs became 1px, so the tab row looked
     like it had tightened around its buttons on that one tab. The body has
     `overflow-y: auto` and is what should absorb the squeeze. */
  /* The gap is halved from TabList's own: five tabs in a 280px column need the
     8px it gives back more than the row needs the air. */
  .research :global(.research__tabs) {
    display: flex;
    flex: none;
    gap: 2px;
    width: 100%;
    box-sizing: border-box;
  }

  /* The last tab once sat outside the row's own border: the labels, their
     padding and the gaps came to a few pixels more than the row, and a tab
     with the default `min-width: auto` refuses to shrink below its label.
     `min-width: 0` is what holds the row in — a tab that may shrink to nothing
     cannot push past its container.

     The base is the label (`auto`), not an equal share (`0`). With five tabs an
     equal share of the default 280px is about 50px, narrower than `Exportar`,
     while `Notas` needs far less: starting from each label and growing them
     together fits all five at the default width, and only a column dragged
     narrower than that ends a label with an ellipsis. `nowrap` keeps a squeezed
     tab one line high instead of two. */
  .research :global(.research__tabs > button) {
    flex: 1 1 auto;
    min-width: 0;
    padding: 0 var(--space-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .research__body {
    flex: 1;
    width: 100%;
    box-sizing: border-box;
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
