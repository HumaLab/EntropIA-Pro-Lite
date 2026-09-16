<script lang="ts">
  import { untrack } from 'svelte'
  import { ActionIcon, Button, Checkbox, IconButton, Panel } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import {
    DEFAULT_STYLE,
    isCslError,
    renderCluster,
    type StyleSource,
  } from '$lib/writing-csl'

  /**
   * Adjusting a citation (plan-editor.md §11.5).
   *
   * A citation is a *cluster*: one pair of brackets holding one or more works.
   * So this edits a list, not a single reference — each work gets its own
   * locator and its own author suppression, while the affixes belong to the
   * cluster, because "see" is said once before the whole citation rather than
   * once per source.
   *
   * Everything here is CSL-equivalent data. The preview is produced from those
   * fields each time they change, which is also what proves the stored data is
   * enough to reproduce the citation: if the preview reads right, the fields
   * are complete.
   */

  interface ClusterWork {
    itemKey: string
    title: string
    snapshot: string
    locator: string
    locatorType: string
    suppressAuthor: boolean
  }

  interface Props {
    /**
     * The works cited together, taken once.
     *
     * A copy on purpose: the dialog stays open while the writer types in it, so
     * re-reading the node would wipe what they were in the middle of. The
     * parent remounts this per citation, which keeps the copy from belonging to
     * the wrong one.
     */
    items: ClusterWork[]
    affixes: { prefix: string; suffix: string }
    style?: StyleSource
    onapply: (attrs: Record<string, unknown>) => void
    onclose: () => void
  }

  let { items, affixes, style = DEFAULT_STYLE, onapply, onclose }: Props = $props()

  const LOCATOR_KINDS = [
    { value: 'page', label: 'writing.locatorPage' },
    { value: 'chapter', label: 'writing.locatorChapter' },
    { value: 'section', label: 'writing.locatorSection' },
    { value: 'paragraph', label: 'writing.locatorParagraph' },
    { value: 'volume', label: 'writing.locatorVolume' },
    { value: 'line', label: 'writing.locatorLine' },
    { value: 'folio', label: 'writing.locatorFolio' },
    { value: 'note', label: 'writing.locatorNote' },
  ] as const

  // `untrack` says the quiet part out loud: these are taken once and then
  // belong to the writer.
  let works = $state<ClusterWork[]>(untrack(() => items.map((item) => ({ ...item }))))
  let prefix = $state(untrack(() => affixes.prefix))
  let suffix = $state(untrack(() => affixes.suffix))

  let preview = $state('')
  let renderError = $state<string | null>(null)
  /** Works that asked for suppression and did not get it, by index. */
  let unsuppressed = $state<number[]>([])

  /** Reads a work's title out of its snapshot, for naming the row. */
  function nameOf(work: ClusterWork): string {
    if (work.title) return work.title
    try {
      const item = JSON.parse(work.snapshot) as Record<string, unknown>
      return String(item.title ?? work.itemKey)
    } catch {
      return work.itemKey
    }
  }

  $effect(() => {
    const cluster = works.map((work, index) => ({
      csl_json: work.snapshot,
      locator: work.locator || null,
      locator_kind: work.locatorType,
      prefix: index === 0 ? prefix || null : null,
      suffix: index === 0 ? suffix || null : null,
      suppress_author: work.suppressAuthor,
    }))
    let cancelled = false
    void renderCluster(cluster, style).then((result) => {
      if (cancelled) return
      if (isCslError(result)) {
        renderError = t('writing.citeRenderFailed', { message: result.message })
        return
      }
      renderError = null
      preview = result.text
      // The engine reports suppression for the cluster as a whole, so the
      // answer is only attributable when exactly one work asked for it.
      // Pointing at a particular work on weaker evidence would be inventing one.
      const asked = works.map((w, i) => (w.suppressAuthor ? i : -1)).filter((i) => i >= 0)
      unsuppressed = asked.length === 1 && !result.author_suppressed ? asked : []
    })
    return () => {
      cancelled = true
    }
  })

  function removeWork(index: number) {
    works = works.filter((_, i) => i !== index)
  }

  function apply() {
    onapply({
      items: works.map((work) => ({
        itemKey: work.itemKey,
        metadataSnapshot: work.snapshot,
        locator: work.locator || null,
        locatorType: work.locatorType,
        suppressAuthor: work.suppressAuthor,
      })),
      prefix: prefix || null,
      suffix: suffix || null,
      renderedText: preview || null,
    })
    onclose()
  }
</script>

<Panel padding="md">
  <div class="cite">
    <p class="cite__title">{t('writing.citeDialogTitle')}</p>

    <!-- One row per work. A citation of three sources is one citation, and each
         of its works still has its own page and its own author. -->
    {#each works as work, index (work.itemKey + index)}
      <div class="cite__work">
        <div class="cite__work-head">
          <span class="cite__work-name">{nameOf(work)}</span>
          {#if works.length > 1}
            <IconButton
              size="sm"
              variant="ghost"
              label={t('writing.citeRemoveWork')}
              onclick={() => removeWork(index)}
            >
              <ActionIcon name="close" size={12} />
            </IconButton>
          {/if}
        </div>

        <div class="cite__row">
          <label class="cite__field">
            <span class="cite__label">{t('writing.citeLocator')}</span>
            <input
              class="cite__input"
              type="text"
              bind:value={work.locator}
              placeholder={t('writing.citeLocatorPlaceholder')}
            />
          </label>
          <label class="cite__field">
            <span class="cite__label">{t('writing.citeLocatorKind')}</span>
            <select class="cite__input" bind:value={work.locatorType}>
              {#each LOCATOR_KINDS as kind (kind.value)}
                <option value={kind.value}>{t(kind.label)}</option>
              {/each}
            </select>
          </label>
        </div>

        <Checkbox bind:checked={work.suppressAuthor}>{t('writing.citeSuppress')}</Checkbox>
        {#if unsuppressed.includes(index)}
          <p class="cite__warning" role="status">{t('writing.citeSuppressFailed')}</p>
        {/if}
      </div>
    {/each}

    <p class="cite__help">{t('writing.citeSuppressHelp')}</p>

    <!-- Affixes wrap the whole citation: "see" is said once before all of it,
         not once per source. -->
    <label class="cite__field">
      <span class="cite__label">{t('writing.citePrefix')}</span>
      <input
        class="cite__input"
        type="text"
        bind:value={prefix}
        placeholder={t('writing.citePrefixPlaceholder')}
      />
    </label>

    <label class="cite__field">
      <span class="cite__label">{t('writing.citeSuffix')}</span>
      <input
        class="cite__input"
        type="text"
        bind:value={suffix}
        placeholder={t('writing.citeSuffixPlaceholder')}
      />
    </label>

    <p class="cite__label">{t('writing.citePreview')}</p>
    {#if renderError}
      <p class="cite__warning" role="alert">{renderError}</p>
    {:else}
      <p class="cite__preview">{preview}</p>
    {/if}

    <div class="cite__actions">
      <Button variant="secondary" size="sm" onclick={apply}>{t('writing.citeDone')}</Button>
    </div>
  </div>
</Panel>

<style>
  .cite {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .cite__title {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .cite__work {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
  }

  .cite__work-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .cite__work-name {
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cite__row {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .cite__field {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1 1 12ch;
    min-width: 0;
  }

  .cite__label {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
  }

  .cite__input {
    width: 100%;
    min-height: 28px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-xs);
  }

  .cite__input:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .cite__preview {
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .cite__help {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  .cite__warning {
    margin: 0;
    color: var(--color-warning);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  .cite__actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
