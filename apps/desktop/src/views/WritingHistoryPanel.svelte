<script lang="ts">
  import { Button, IconButton, ActionIcon, Panel, WritingEditor } from '@entropia/ui'
  import type { CanonicalDocument } from '@entropia/ui'
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
      preview = await writing.readVersion(versionNumber)
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
      await load()
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
      {#if versions === null}
        <p class="history-panel__hint">…</p>
      {:else if versions.length === 0}
        <p class="history-panel__hint">{t('writing.historyEmpty')}</p>
      {:else}
        <ul>
          {#each versions as version (version.version_number)}
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
      {:else}
        <p class="history-panel__hint">{t('writing.historyPick')}</p>
      {/if}
      {#if error}<p class="history-panel__error" role="alert">{error}</p>{/if}
    </section>
  </div>
</Panel>

<style>
  .history-panel {
    position: relative;
    display: grid;
    grid-template-columns: minmax(12rem, 1fr) 2fr;
    gap: var(--space-4);
  }

  .history-panel > :global(button:first-child) {
    position: absolute;
    top: 0;
    right: 0;
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
