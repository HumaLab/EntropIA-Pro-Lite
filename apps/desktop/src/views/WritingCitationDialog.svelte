<script lang="ts">
  import { untrack } from 'svelte'
  import { Button, Checkbox, Panel } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import {
    DEFAULT_STYLE,
    isCslError,
    renderCluster,
    type StyleSource,
  } from '$lib/writing-csl'

  /**
   * Adjusting one bibliographic citation (plan-editor.md §11.5).
   *
   * Everything here is CSL-equivalent data: a locator and its kind, the affixes
   * and whether the author is already named in the writer's own sentence. None
   * of it is the rendered string, which §11.5 forbids storing — the preview is
   * produced from these fields every time they change, which is also what
   * proves the stored data is enough to reproduce the citation.
   *
   * The suppression checkbox is worded as the situation rather than the
   * mechanism. "Suppress author" describes what the software does; "I already
   * named the author in my sentence" describes when to tick it, which is the
   * thing the writer actually knows.
   */

  interface Props {
    /** The work being cited, as CSL-JSON. Never edited here, only rendered from. */
    snapshot: string
    /**
     * The citation's current settings, taken once.
     *
     * A copy on purpose: the dialog stays open while the writer types in it, so
     * re-reading the node would wipe what they were in the middle of. The
     * parent remounts this component per citation, which is what keeps the copy
     * from belonging to the wrong one.
     */
    initial: {
      locator: string
      locatorType: string
      prefix: string
      suffix: string
      suppressAuthor: boolean
    }
    style?: StyleSource
    /** Applies the changes to the node, found by its identity. */
    onapply: (attrs: Record<string, unknown>) => void
    onclose: () => void
  }

  let { snapshot, initial, style = DEFAULT_STYLE, onapply, onclose }: Props = $props()

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
  // belong to the writer. Following the prop would overwrite what they are in
  // the middle of typing every time the manuscript changed underneath.
  let locator = $state(untrack(() => initial.locator))
  let locatorKind = $state(untrack(() => initial.locatorType) || 'page')
  let prefix = $state(untrack(() => initial.prefix))
  let suffix = $state(untrack(() => initial.suffix))
  let suppressAuthor = $state(untrack(() => initial.suppressAuthor))

  let preview = $state('')
  let renderError = $state<string | null>(null)
  /** False when suppression was asked for and the style would not allow it. */
  let suppressionHonoured = $state(true)

  /**
   * Re-renders on every change, which is the point: the preview is derived from
   * the stored fields, so seeing it right is seeing that the fields are enough.
   */
  $effect(() => {
    const items = [
      {
        csl_json: snapshot,
        locator: locator || null,
        locator_kind: locatorKind,
        prefix: prefix || null,
        suffix: suffix || null,
        suppress_author: suppressAuthor,
      },
    ]
    let cancelled = false
    void renderCluster(items, style).then((result) => {
      if (cancelled) return
      if (isCslError(result)) {
        renderError = t('writing.citeRenderFailed', { message: result.message })
        return
      }
      renderError = null
      preview = result.text
      suppressionHonoured = !suppressAuthor || result.author_suppressed
    })
    return () => {
      cancelled = true
    }
  })

  function apply() {
    onapply({
      locator: locator || null,
      locatorType: locatorKind,
      prefix: prefix || null,
      suffix: suffix || null,
      suppressAuthor,
      renderedText: preview || null,
    })
    onclose()
  }
</script>

<Panel padding="md">
  <div class="cite">
    <p class="cite__title">{t('writing.citeDialogTitle')}</p>

    <label class="cite__field">
      <span class="cite__label">{t('writing.citeLocator')}</span>
      <input
        class="cite__input"
        type="text"
        bind:value={locator}
        placeholder={t('writing.citeLocatorPlaceholder')}
      />
    </label>

    <label class="cite__field">
      <span class="cite__label">{t('writing.citeLocatorKind')}</span>
      <select class="cite__input" bind:value={locatorKind}>
        {#each LOCATOR_KINDS as kind (kind.value)}
          <option value={kind.value}>{t(kind.label)}</option>
        {/each}
      </select>
    </label>

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

    <!-- Worded as the situation, not the mechanism: "suppress author" says what
         the software does, this says when to tick it. -->
    <Checkbox bind:checked={suppressAuthor}>{t('writing.citeSuppress')}</Checkbox>
    <p class="cite__help">{t('writing.citeSuppressHelp')}</p>

    {#if !suppressionHonoured}
      <!-- Said out loud rather than left as a checkbox that does nothing. -->
      <p class="cite__warning" role="status">{t('writing.citeSuppressFailed')}</p>
    {/if}

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

  .cite__field {
    display: flex;
    flex-direction: column;
    gap: 2px;
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
