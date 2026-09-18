<script lang="ts">
  import { ActionIcon, IconButton, Panel } from '@entropia/ui'
  import { t, type I18nKey } from '$lib/i18n'
  import { CITATION_CHOICES } from '$lib/export-preferences'
  import type { FidelityWarning } from '$lib/export-fidelity'
  import type { DownloadOutcome } from '$lib/writing-export'

  /**
   * What a download reports (plan-editor.md §17.4), under the writing bar.
   *
   * The download menu closes the moment a format is chosen, so what the export
   * has to say — where the file went, what the format could not carry, or why
   * nothing was written — needs a place of its own. It stays until dismissed:
   * a warning that disappears on a timer is one the writer may never read.
   *
   * A cancelled download is not reported: closing the file dialog is an answer,
   * not a problem.
   */

  interface Props {
    outcome: Exclude<DownloadOutcome, { kind: 'cancelled' }>
    ondismiss: () => void
  }

  let { outcome, ondismiss }: Props = $props()

  const REPRESENTATION_LABEL = new Map<string, I18nKey>(
    CITATION_CHOICES.map((choice) => [choice.id, choice.label])
  )

  function describe(warning: FidelityWarning): string {
    const label = REPRESENTATION_LABEL.get(warning.element)
    const name =
      warning.kind === 'citation' && label
        ? t(label)
        : t(`writing.exportElement.${warning.element}` as I18nKey)
    return t(
      warning.support === 'unsupported' ? 'writing.exportDropped' : 'writing.exportSubstituted',
      { element: name, count: String(warning.count) }
    )
  }
</script>

<Panel padding="md">
  <div class="export-notice">
    <div class="export-notice__body">
      {#if outcome.kind === 'refused'}
        <!-- Not a caveat. §17.4 forbids DOCX dropping these, so nothing was
             written at all. -->
        <p class="export-notice__error" role="alert">
          {t('writing.exportRefused', {
            elements: outcome.elements
              .map((element) => t(`writing.exportElement.${element}` as I18nKey))
              .join(', '),
          })}
        </p>
      {:else if outcome.kind === 'failed'}
        <p class="export-notice__error" role="alert">
          {t('writing.exportFailed', { message: outcome.message })}
        </p>
      {:else}
        <p class="export-notice__saved" role="status">
          {t('writing.exportSaved', { path: outcome.path })}
        </p>
        {#if outcome.trouble}
          <p class="export-notice__error" role="alert">
            {t('writing.exportTrouble', { message: outcome.trouble })}
          </p>
        {/if}
        {#if outcome.warnings.length > 0}
          <ul class="export-notice__warnings">
            {#each outcome.warnings as warning (`${warning.kind}:${warning.element}:${warning.support}`)}
              <li class="export-notice__warning">{describe(warning)}</li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>
    <IconButton
      size="sm"
      variant="ghost"
      label={t('writing.exportClose')}
      title={t('writing.exportClose')}
      onclick={ondismiss}
    >
      <ActionIcon name="close" size={14} />
    </IconButton>
  </div>
</Panel>

<style>
  .export-notice {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .export-notice__body {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .export-notice__saved,
  .export-notice__error {
    margin: 0;
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .export-notice__saved {
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .export-notice__error {
    color: var(--color-danger);
  }

  .export-notice__warnings {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .export-notice__warning {
    color: var(--color-warning);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }
</style>
