<script lang="ts">
  import { Checkbox, RadioCard, RadioGroup } from '@entropia/ui'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import { CITATION_CHOICES, type ExportPreferences } from '$lib/export-preferences'
  import type { CitationRepresentation } from '$lib/export-fidelity'
  import type { Node } from '$lib/export-document'
  import { documentStats, type DocumentStats } from '$lib/document-stats'

  /**
   * The Export tab of the research panel (plan-editor.md §17.2): how the
   * manuscript is exported, chosen once.
   *
   * # What is not here
   *
   * The format, and any button that exports. Both belong to the download
   * button in the bar, which asks for the format each time and exports at
   * once with what is chosen here. Keeping the two apart is what lets a
   * download take two clicks: the questions that do not change between
   * downloads are not asked again.
   *
   * # Why every choice is offered whatever the format
   *
   * There is no format here to disable a choice against. The one gap — Markdown
   * has no comment — is said beside the choice when it is made, and the
   * download that meets it says so again.
   *
   * # The statistics
   *
   * Informative only, under the choices: the counts a writer checks against a
   * word limit before sending the manuscript out (document-stats.ts says what
   * is counted). They follow the manuscript on screen, and are computed only
   * while this tab is open.
   */

  interface Props {
    preferences: ExportPreferences
    onchange: (next: ExportPreferences) => void
    /** The manuscript on screen, for the statistics. None, no statistics. */
    doc?: Node | null
  }

  let { preferences, onchange, doc = null }: Props = $props()

  const stats = $derived(doc ? documentStats(doc) : null)
  const numbers = $derived(new Intl.NumberFormat($locale))

  const STATS: { key: keyof DocumentStats; label: I18nKey }[] = [
    { key: 'words', label: 'writing.stats.words' },
    { key: 'characters', label: 'writing.stats.characters' },
    { key: 'charactersNoSpaces', label: 'writing.stats.charactersNoSpaces' },
    { key: 'paragraphs', label: 'writing.stats.paragraphs' },
    { key: 'footnotes', label: 'writing.stats.footnotes' },
  ]

  const uid = $props.id()

  function chooseCitations(citations: CitationRepresentation) {
    onchange({ ...preferences, citations })
  }

  function chooseBibliography(bibliography: boolean) {
    onchange({ ...preferences, bibliography })
  }
</script>

<div class="export-tab">
  <div class="export-tab__group">
    <!-- The typography cards, compacted: native radios, so arrow keys move the
         choice and the group is announced as one. The 120px floor is the Agent
         tab's grid (WritingAgentTab.svelte): across the panel's 200–560 range it
         steps 1 / 2 / 3 columns, two at the default 280. -->
    <RadioGroup
      label={t('writing.exportCitations')}
      value={preferences.citations}
      columnMin="120px"
      onchange={(id) => chooseCitations(id as CitationRepresentation)}
    >
      {#each CITATION_CHOICES as choice (choice.id)}
        <RadioCard value={choice.id} label={t(choice.label)} />
      {/each}
    </RadioGroup>
    {#if preferences.citations === 'comment'}
      <p class="export-tab__note">{t('writing.exportCiteMarkdownComment')}</p>
    {/if}
  </div>

  <Checkbox checked={preferences.bibliography} onchange={chooseBibliography}>
    {t('writing.exportWithBibliography')}
  </Checkbox>

  {#if stats}
    <section class="export-tab__stats" aria-labelledby="{uid}-stats">
      <p class="export-tab__legend" id="{uid}-stats">{t('writing.stats.title')}</p>
      <dl class="export-tab__stats-list">
        {#each STATS as stat (stat.key)}
          <div class="export-tab__stat">
            <dt class="export-tab__stat-label">{t(stat.label)}</dt>
            <dd class="export-tab__stat-value">{numbers.format(stats[stat.key])}</dd>
          </div>
        {/each}
      </dl>
    </section>
  {/if}
</div>

<style>
  .export-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .export-tab__group {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .export-tab__legend {
    margin: 0 0 var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: var(--letter-spacing-caps);
    text-transform: uppercase;
  }

  .export-tab__note {
    margin: var(--space-1) 0 0;
    padding: 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  /* A quiet panel, not a card competing with the choices above: the same
     subtle border, and the rows read label to value across it. */
  .export-tab__stats {
    display: flex;
    flex-direction: column;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
  }

  .export-tab__stats-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
  }

  .export-tab__stat {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .export-tab__stat-label {
    min-width: 0;
    color: var(--color-text-muted);
  }

  /* Tabular, so the digits line up as the manuscript grows. */
  .export-tab__stat-value {
    margin: 0;
    color: var(--color-text-primary);
    font-variant-numeric: tabular-nums;
    font-weight: var(--font-weight-medium);
  }
</style>
