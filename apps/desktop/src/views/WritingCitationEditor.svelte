<script module lang="ts">
  export interface CitationWork {
    itemKey: string
    sourceOrigin?: string
    sourceInstanceId?: string | null
    libraryType?: string
    libraryId?: string
    itemVersion?: number | null
    title: string
    snapshot: string
    locator: string
    locatorType: string
    suppressAuthor: boolean
  }

  export interface CitationDraft {
    items: CitationWork[]
    affixes: { prefix: string; suffix: string }
  }

  export interface CitationEditSession extends CitationDraft {
    id: string
  }
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import {
    ActionIcon,
    Button,
    Checkbox,
    IconButton,
    ToolbarMenu,
    tooltip,
    type ToolbarMenuItem,
  } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { DEFAULT_STYLE, isCslError, renderCluster, type StyleSource } from '$lib/writing-csl'

  interface Props extends CitationDraft {
    style?: StyleSource
    ondraftchange?: (draft: CitationDraft) => void
    onapply: (attrs: Record<string, unknown>) => void
    onclose: () => void
  }

  let { items, affixes, style = DEFAULT_STYLE, ondraftchange, onapply, onclose }: Props = $props()

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

  // This local copy keeps incomplete keystrokes inside the editor. Every change
  // is also reported upward so remounting the Zotero tab can restore the draft.
  let works = $state<CitationWork[]>(untrack(() => items.map((item) => ({ ...item }))))
  let prefix = $state(untrack(() => affixes.prefix))
  let suffix = $state(untrack(() => affixes.suffix))
  let locatorKindMenuOpen = $state<boolean[]>(untrack(() => items.map(() => false)))

  let preview = $state('')
  let renderError = $state<string | null>(null)
  /** Works that asked for suppression and did not get it, by index. */
  let unsuppressed = $state<number[]>([])

  function nameOf(work: CitationWork): string {
    if (work.title) return work.title
    try {
      const item = JSON.parse(work.snapshot) as Record<string, unknown>
      return String(item.title ?? work.itemKey)
    } catch {
      return work.itemKey
    }
  }

  function locatorKindLabel(locatorType: string): string {
    const kind = LOCATOR_KINDS.find((candidate) => candidate.value === locatorType)
    return kind ? t(kind.label) : locatorType
  }

  /** The locator-kind picker: the canonical radio pattern (a ToolbarMenu whose
   *  checked entry is the choice), never a native select. */
  function locatorKindItems(work: CitationWork): ToolbarMenuItem[] {
    return LOCATOR_KINDS.map((kind) => ({
      kind: 'radio' as const,
      id: kind.value,
      label: t(kind.label),
      checked: work.locatorType === kind.value,
      onselect: () => {
        work.locatorType = kind.value
        changed()
      },
    }))
  }

  function draft(): CitationDraft {
    return {
      items: works.map((work) => ({ ...work })),
      affixes: { prefix, suffix },
    }
  }

  function changed() {
    ondraftchange?.(draft())
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
      const asked = works
        .map((work, index) => (work.suppressAuthor ? index : -1))
        .filter((i) => i >= 0)
      unsuppressed = asked.length === 1 && !result.author_suppressed ? asked : []
    })
    return () => {
      cancelled = true
    }
  })

  function removeWork(index: number) {
    works = works.filter((_, workIndex) => workIndex !== index)
    changed()
  }

  function identityAttrs(work: CitationWork) {
    const attrs: Partial<
      Pick<
        CitationWork,
        'sourceOrigin' | 'sourceInstanceId' | 'libraryType' | 'libraryId' | 'itemVersion'
      >
    > = {}
    if (work.sourceOrigin !== undefined) attrs.sourceOrigin = work.sourceOrigin
    if (work.sourceInstanceId !== undefined) attrs.sourceInstanceId = work.sourceInstanceId
    if (work.libraryType !== undefined) attrs.libraryType = work.libraryType
    if (work.libraryId !== undefined) attrs.libraryId = work.libraryId
    if (work.itemVersion !== undefined) attrs.itemVersion = work.itemVersion
    return attrs
  }

  function apply() {
    onapply({
      items: works.map((work) => ({
        ...identityAttrs(work),
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

<div class="cite">
  <div class="cite__header">
    <Button variant="ghost" size="sm" onclick={onclose}>
      <ActionIcon name="chevron-left" size={12} />
      {t('writing.zoteroBack')}
    </Button>
    <p class="cite__title">{t('writing.citeDialogTitle')}</p>
  </div>

  <div class="cite__works">
    {#each works as work, index (work.itemKey + index)}
      <section class="cite__work" aria-label={nameOf(work)}>
        <div class="cite__work-head">
          <span class="cite__work-name" use:tooltip={nameOf(work)}>{nameOf(work)}</span>
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
              value={work.locator}
              placeholder={t('writing.citeLocatorPlaceholder')}
              oninput={(event) => {
                work.locator = event.currentTarget.value
                changed()
              }}
            />
          </label>
          <div class="cite__field">
            <span class="cite__label" id="cite-locator-kind-label-{index}"
              >{t('writing.citeLocatorKind')}</span
            >
            <ToolbarMenu
              label={t('writing.citeLocatorKind')}
              items={locatorKindItems(work)}
              bind:open={locatorKindMenuOpen[index]}
            >
              {#snippet trigger(props, { open })}
                <button
                  type="button"
                  class="cite__menu-trigger"
                  class:cite__menu-trigger--open={open}
                  aria-labelledby="cite-locator-kind-label-{index} cite-locator-kind-value-{index}"
                  {...props}
                >
                  <span class="cite__menu-trigger-label" id="cite-locator-kind-value-{index}"
                    >{locatorKindLabel(work.locatorType)}</span
                  >
                  <ActionIcon name="chevron-down" size={12} />
                </button>
              {/snippet}
            </ToolbarMenu>
          </div>
        </div>

        <Checkbox
          checked={work.suppressAuthor}
          onchange={(checked) => {
            work.suppressAuthor = checked
            changed()
          }}>{t('writing.citeSuppress')}</Checkbox
        >
        {#if unsuppressed.includes(index)}
          <p class="cite__warning" role="status">{t('writing.citeSuppressFailed')}</p>
        {/if}
      </section>
    {/each}
  </div>

  <p class="cite__help">{t('writing.citeSuppressHelp')}</p>

  <label class="cite__field">
    <span class="cite__label">{t('writing.citePrefix')}</span>
    <input
      class="cite__input"
      type="text"
      value={prefix}
      placeholder={t('writing.citePrefixPlaceholder')}
      oninput={(event) => {
        prefix = event.currentTarget.value
        changed()
      }}
    />
  </label>

  <label class="cite__field">
    <span class="cite__label">{t('writing.citeSuffix')}</span>
    <input
      class="cite__input"
      type="text"
      value={suffix}
      placeholder={t('writing.citeSuffixPlaceholder')}
      oninput={(event) => {
        suffix = event.currentTarget.value
        changed()
      }}
    />
  </label>

  <div class="cite__preview-block">
    <p class="cite__label">{t('writing.citePreview')}</p>
    {#if renderError}
      <p class="cite__warning" role="alert">{renderError}</p>
    {:else}
      <p class="cite__preview">{preview}</p>
    {/if}
  </div>

  <div class="cite__actions">
    <Button variant="ghost" size="sm" onclick={onclose}>{t('writing.citeCancel')}</Button>
    <Button variant="secondary" size="sm" onclick={apply}>{t('writing.citeDone')}</Button>
  </div>
</div>

<style>
  .cite,
  .cite__works,
  .cite__work,
  .cite__field,
  .cite__preview-block {
    min-width: 0;
  }

  .cite {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .cite__header {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
  }

  .cite__title {
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  .cite__works {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
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
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-1);
    min-width: 0;
  }

  .cite__work-name {
    display: -webkit-box;
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
    overflow-wrap: anywhere;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .cite__row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(11rem, 100%), 1fr));
    gap: var(--space-2);
    min-width: 0;
  }

  .cite__field {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .cite__label {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
  }

  .cite__input {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    max-width: 100%;
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

  /* The locator-kind trigger. It replaces the native select that wore
     .cite__input, so it keeps that field look and adds the canonical
     label/name/chevron shape of the other choice menus. */
  .cite__menu-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    width: 100%;
    min-width: 0;
    max-width: 100%;
    min-height: 28px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-xs);
    cursor: pointer;
  }

  .cite__menu-trigger:hover,
  .cite__menu-trigger--open {
    border-color: color-mix(in srgb, var(--border-focus) 40%, var(--border-subtle));
  }

  .cite__menu-trigger:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .cite__menu-trigger-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-align: start;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cite__preview-block {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .cite__preview {
    margin: 0;
    padding: var(--space-2);
    overflow-wrap: anywhere;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .cite__help,
  .cite__warning {
    margin: 0;
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  .cite__help {
    color: var(--color-text-muted);
  }

  .cite__warning {
    color: var(--color-warning);
  }

  .cite__actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-1);
  }
</style>
