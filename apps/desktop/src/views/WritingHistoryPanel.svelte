<script lang="ts">
  import {
    Button,
    IconButton,
    ActionIcon,
    Panel,
    WritingEditor,
    ToolbarMenu,
    documentSections,
    sectionChanges,
    type ToolbarMenuItem,
  } from '@entropia/ui'
  import type { CanonicalDocument, DocumentSection, SectionKey } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { writing, type WritingVersionSummary } from '$lib/writing'
  import { writingEditorLabels } from '$lib/writing-editor-labels'

  /**
   * The open manuscript's history: pick a version, read it, bring it back.
   * Opens under the writing bar, like the share panel. Restoring is never
   * destructive — what it replaces becomes a version too — so it needs no
   * confirmation.
   */

  interface Props {
    onclose: () => void
  }

  let { onclose }: Props = $props()

  const labels = $derived(writingEditorLabels())

  let versions = $state<WritingVersionSummary[] | null>(null)
  let selected = $state<number | null>(null)
  let preview = $state<CanonicalDocument | null>(null)
  let busy = $state(false)
  let error = $state('')

  // Per-section history: the sections come from the manuscript as it is now;
  // a chosen one narrows the list to the versions where it changed.
  let current = $state<CanonicalDocument | null>(null)
  let section = $state<SectionKey | null>(null)
  let sectionMenuOpen = $state(false)
  let changedVersions = $state<Set<number> | null>(null)
  const contents = new Map<number, CanonicalDocument | null>()
  // A later choice wins: `section` holds a proxy, so the request is counted instead.
  let sectionRequest = 0

  $effect(() =>
    writing.subscribe((snapshot) => {
      current = snapshot.content
    })
  )

  const sections = $derived<DocumentSection[]>(documentSections(current))
  const sectionItems = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio' as const,
      id: 'all',
      label: t('writing.historySectionAll'),
      checked: section === null,
      onselect: () => chooseSection(null),
    },
    ...sections.map((entry) => ({
      kind: 'radio' as const,
      id: `${entry.level}:${entry.start}`,
      label: `${'  '.repeat(Math.max(0, entry.level - 1))}${entry.title || '…'}`,
      checked:
        section !== null &&
        section.level === entry.level &&
        section.title === entry.title &&
        section.occurrence === entry.occurrence,
      onselect: () =>
        chooseSection({
          title: entry.title,
          level: entry.level,
          occurrence: entry.occurrence,
          ordinal: entry.ordinal,
        }),
    })),
  ])
  const shownVersions = $derived(
    versions && changedVersions
      ? versions.filter((version) => changedVersions!.has(version.version_number))
      : versions
  )

  async function versionContent(versionNumber: number): Promise<CanonicalDocument | null> {
    if (!contents.has(versionNumber))
      contents.set(versionNumber, await writing.readVersion(versionNumber))
    return contents.get(versionNumber) ?? null
  }

  async function chooseSection(next: SectionKey | null) {
    const request = ++sectionRequest
    section = next
    selected = null
    preview = null
    if (!next || !versions) {
      changedVersions = null
      return
    }
    const oldestFirst = [...versions].sort((a, b) => a.version_number - b.version_number)
    const loaded = await Promise.all(
      oldestFirst.map(async (version) => ({
        id: String(version.version_number),
        document: await versionContent(version.version_number),
      }))
    )
    if (request !== sectionRequest) return
    changedVersions = new Set(sectionChanges(loaded, next).map(Number))
  }

  async function restoreOnlySection() {
    if (selected === null || !section) return
    busy = true
    error = ''
    try {
      const restored = await writing.restoreSection(selected, section)
      if (!restored) {
        error = t('writing.historyRestoreSectionMissing')
        return
      }
      selected = null
      preview = null
      contents.clear()
      await load()
      await chooseSection(section)
    } catch (failure) {
      error = String((failure as { message?: string })?.message ?? failure)
    } finally {
      busy = false
    }
  }

  const REASON_KEYS: Record<string, string> = {
    auto: 'writing.historyAuto',
    checkpoint: 'writing.historyBeforeReceive',
    restore: 'writing.historyBeforeRestore',
  }

  async function load() {
    try {
      versions = await writing.listVersions()
    } catch (failure) {
      error = String((failure as { message?: string })?.message ?? failure)
      versions = []
    }
  }

  async function pick(versionNumber: number) {
    selected = versionNumber
    preview = null
    try {
      preview = await versionContent(versionNumber)
    } catch (failure) {
      error = String((failure as { message?: string })?.message ?? failure)
    }
  }

  async function restore() {
    if (selected === null) return
    busy = true
    error = ''
    try {
      await writing.restoreVersion(selected)
      selected = null
      preview = null
      contents.clear()
      await load()
      if (section) await chooseSection(section)
    } catch (failure) {
      error = String((failure as { message?: string })?.message ?? failure)
    } finally {
      busy = false
    }
  }

  const when = (ms: number) =>
    new Date(ms).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' })

  void load()
</script>

<Panel padding="md">
  <div class="history-panel">
    <IconButton size="sm" variant="ghost" label={t('sync.notif.close')} onclick={onclose}>
      <ActionIcon name="close" size={14} />
    </IconButton>

    <section class="history-panel__list">
      <h3>{t('writing.historyTitle')}</h3>
      {#if sections.length > 0}
        <ToolbarMenu
          label={t('writing.historySectionLabel')}
          items={sectionItems}
          bind:open={sectionMenuOpen}
        >
          {#snippet trigger(props, { open })}
            <button type="button" class="history-panel__section" class:is-open={open} {...props}>
              <span class="history-panel__hint">{t('writing.historySectionLabel')}:</span>
              <span class="history-panel__section-name">
                {section ? section.title || '…' : t('writing.historySectionAll')}
              </span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
        </ToolbarMenu>
      {/if}
      {#if versions === null}
        <p class="history-panel__hint">…</p>
      {:else if versions.length === 0}
        <p class="history-panel__hint">{t('writing.historyEmpty')}</p>
      {:else if shownVersions && shownVersions.length === 0}
        <p class="history-panel__hint">{t('writing.historySectionEmpty')}</p>
      {:else}
        <ul>
          {#each shownVersions ?? [] as version (version.version_number)}
            <li>
              <button
                type="button"
                class="history-panel__item"
                class:history-panel__item--selected={selected === version.version_number}
                onclick={() => pick(version.version_number)}
              >
                <span>{when(version.created_at)}</span>
                <span class="history-panel__hint">
                  {t(REASON_KEYS[version.reason] ?? 'writing.historyOther')}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="history-panel__preview">
      {#if preview}
        <div class="history-panel__doc">
          <WritingEditor document={preview} toolbar={false} editable={false} {labels} />
        </div>
        <Button size="sm" disabled={busy} onclick={restore}>
          <ActionIcon name="history" size={14} />
          {t('writing.historyRestore')}
        </Button>
        <p class="history-panel__hint">{t('writing.historyRestoreHint')}</p>
        {#if section}
          <Button size="sm" variant="secondary" disabled={busy} onclick={restoreOnlySection}>
            <ActionIcon name="history" size={14} />
            {t('writing.historyRestoreSection')}
          </Button>
          <p class="history-panel__hint">
            {t('writing.historyRestoreSectionHint', { title: section.title || '…' })}
          </p>
        {/if}
      {:else}
        <p class="history-panel__hint">{t('writing.historyPick')}</p>
      {/if}
      {#if error}<p class="history-panel__error" role="alert">{error}</p>{/if}
    </section>
  </div>
</Panel>

<style>
  .history-panel__section {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    max-width: 100%;
    margin-bottom: var(--space-2);
    padding: var(--space-1) var(--space-2);
    font: inherit;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background: transparent;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    cursor: pointer;
  }

  .history-panel__section.is-open,
  .history-panel__section:focus-visible {
    border-color: var(--color-accent);
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .history-panel__section-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The close button takes its own track instead of floating over the
     preview: absolutely positioned at the top-right corner it sat on the
     preview's bordered editor box and on the hint beside it. */
  .history-panel {
    display: grid;
    grid-template-columns: minmax(min(100%, 12rem), 1fr) minmax(0, 2fr) auto;
    gap: var(--space-4);
  }

  .history-panel > :global(button:first-child) {
    grid-column: 3;
    grid-row: 1;
    align-self: start;
  }

  h3 {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-md, 1rem);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 18rem;
    overflow-y: auto;
  }

  .history-panel__item {
    display: flex;
    flex-direction: column;
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: 0;
    border-radius: var(--radius-sm, 4px);
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .history-panel__item:hover,
  .history-panel__item--selected {
    background: var(--color-surface-hover, var(--color-surface));
  }

  .history-panel__doc {
    max-height: 18rem;
    overflow-y: auto;
    margin-bottom: var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm, 4px);
  }

  .history-panel__hint {
    color: var(--color-text-secondary);
    font-size: 0.9em;
  }

  .history-panel__error {
    color: var(--color-danger, #c0392b);
  }
</style>
