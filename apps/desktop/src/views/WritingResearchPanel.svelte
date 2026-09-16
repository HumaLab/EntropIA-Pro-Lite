<script module lang="ts">
  export type ResearchTab = 'corpus' | 'zotero' | 'notes' | 'agent'
</script>

<script lang="ts">
  import { TabButton, TabList } from '@entropia/ui'
  import { t, type I18nKey } from '$lib/i18n'

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
  }

  let { tab = $bindable<ResearchTab>('corpus') }: Props = $props()

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
      <p class="research__pending">{t(active.pending)}</p>
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
