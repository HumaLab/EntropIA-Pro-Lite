<script lang="ts">
  import { Button, Checkbox, Panel } from '@entropia/ui'
  import { t, type I18nKey } from '$lib/i18n'
  import {
    CITATION_FIDELITY,
    type CitationRepresentation,
    type ExportFormat,
    type FidelityWarning,
  } from '$lib/export-fidelity'
  import { DEFAULT_STYLE, type StyleSource } from '$lib/writing-csl'
  import { isExportFailure, saveExport, type ExportSettings } from '$lib/writing-export'
  import type { Node } from '$lib/export-document'

  /**
   * Exporting a manuscript (plan-editor.md §17).
   *
   * # Why the warnings are shown before the export and not after
   *
   * §17.4 asks a partial export to warn what could not be represented. A
   * warning that arrives after the file is on disk is a report, not a warning:
   * by then the writer has already sent it to someone. So the fidelity of the
   * current choice is on screen while they are still choosing, and changes as
   * they change the format.
   *
   * # Why an option the format cannot do is disabled rather than hidden
   *
   * Markdown has no comment. Hiding the option would leave a writer who used it
   * last time wondering where it went; offering it and quietly substituting a
   * footnote would give them a document they did not ask for. Disabled, with
   * the reason beside it, is the only one of the three that tells the truth.
   *
   * # Nothing here touches the manuscript
   *
   * §17.2's closing line. The settings are read on the way out and never
   * written back, which is also why they are local state and not part of the
   * document.
   */

  interface Props {
    /** The canonical document, read and never written. */
    doc: Node
    title: string
    style?: StyleSource
    onclose: () => void
  }

  let { doc, title, style = DEFAULT_STYLE, onclose }: Props = $props()

  const FORMATS: { id: ExportFormat; label: I18nKey }[] = [
    { id: 'markdown', label: 'writing.exportMarkdown' },
    { id: 'html', label: 'writing.exportHtml' },
    { id: 'docx', label: 'writing.exportDocx' },
  ]

  const REPRESENTATIONS: { id: CitationRepresentation; label: I18nKey }[] = [
    { id: 'footnote', label: 'writing.exportCiteFootnote' },
    { id: 'inline', label: 'writing.exportCiteInline' },
    { id: 'comment', label: 'writing.exportCiteComment' },
    { id: 'quote_with_note', label: 'writing.exportCiteQuote' },
  ]

  let format = $state<ExportFormat>('docx')
  let citations = $state<CitationRepresentation>('footnote')
  let bibliography = $state(true)
  let busy = $state(false)
  let warnings = $state<FidelityWarning[]>([])
  let refused = $state<string[] | null>(null)
  let trouble = $state<string | null>(null)
  /** The export itself failed: unlike `trouble`, no file was written. */
  let failed = $state<string | null>(null)
  let saved = $state<string | null>(null)

  /**
   * A representation the chosen format cannot do falls back to the footnote.
   *
   * Done here rather than left to fail at export time: the writer is choosing,
   * and a choice that silently produces something else is worse than a choice
   * that visibly moves.
   */
  $effect(() => {
    if (CITATION_FIDELITY[citations][format] === 'unsupported') citations = 'footnote'
  })

  const unavailable = $derived(
    (representation: CitationRepresentation) =>
      CITATION_FIDELITY[representation][format] === 'unsupported'
  )

  const REPRESENTATION_LABEL = new Map(
    REPRESENTATIONS.map((option) => [option.id as string, option.label])
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

  async function run() {
    busy = true
    refused = null
    trouble = null
    failed = null
    saved = null
    try {
      const settings: ExportSettings = {
        format,
        citations,
        bibliography,
        style,
        title,
        bibliographyHeading: t('writing.exportBibliography'),
      }
      const out = await saveExport(doc, settings, title || t('writing.untitled'))

      if (isExportFailure(out)) {
        // §17.4: a warning does not discharge an obligatory element in DOCX, so
        // no file was written and the reason is named rather than softened.
        refused = out.elements
        return
      }
      warnings = out.result.warnings
      trouble = out.result.citationTrouble
      if (out.path) saved = out.path
    } catch (error) {
      // Not citation trouble: that is reported on a file that was written. A
      // throw here means no file exists, and saying otherwise would send the
      // writer looking for it.
      failed = error instanceof Error ? error.message : String(error)
    } finally {
      busy = false
    }
  }
</script>

<Panel padding="md">
  <div class="export">
    <p class="export__title">{t('writing.exportTitle')}</p>

    <fieldset class="export__group">
      <legend class="export__legend">{t('writing.exportFormat')}</legend>
      <div class="export__row">
        {#each FORMATS as option (option.id)}
          <Button
            variant={format === option.id ? 'secondary' : 'ghost'}
            size="sm"
            onclick={() => (format = option.id)}
          >
            {t(option.label)}
          </Button>
        {/each}
      </div>
    </fieldset>

    <fieldset class="export__group">
      <legend class="export__legend">{t('writing.exportCitations')}</legend>
      <!-- §17.2's four representations. What this format cannot do is disabled
           with its reason, never hidden and never quietly substituted. -->
      <div class="export__row">
        {#each REPRESENTATIONS as option (option.id)}
          <Button
            variant={citations === option.id ? 'secondary' : 'ghost'}
            size="sm"
            disabled={unavailable(option.id)}
            onclick={() => (citations = option.id)}
          >
            {t(option.label)}
          </Button>
        {/each}
      </div>
      {#if REPRESENTATIONS.some((option) => unavailable(option.id))}
        <p class="export__note">{t('writing.exportCiteUnavailable')}</p>
      {/if}
    </fieldset>

    <Checkbox bind:checked={bibliography} label={t('writing.exportWithBibliography')} />

    {#if refused}
      <!-- Not a caveat. §17.4 forbids DOCX dropping these, so nothing was
           written at all. -->
      <p class="export__error" role="alert">
        {t('writing.exportRefused', {
          elements: refused
            .map((element) => t(`writing.exportElement.${element}` as I18nKey))
            .join(', '),
        })}
      </p>
    {/if}

    {#if warnings.length > 0}
      <ul class="export__warnings">
        {#each warnings as warning (warning.kind + warning.element)}
          <li class="export__warning">{describe(warning)}</li>
        {/each}
      </ul>
    {/if}

    {#if failed}
      <p class="export__error" role="alert">{t('writing.exportFailed', { message: failed })}</p>
    {/if}

    {#if trouble}
      <p class="export__error" role="alert">{t('writing.exportTrouble', { message: trouble })}</p>
    {/if}

    {#if saved}
      <p class="export__note" role="status">{t('writing.exportSaved', { path: saved })}</p>
    {/if}

    <div class="export__actions">
      <Button variant="primary" size="sm" disabled={busy} onclick={run}>
        {busy ? t('writing.exportRunning') : t('writing.exportAction')}
      </Button>
      <Button variant="ghost" size="sm" onclick={onclose}>{t('writing.exportClose')}</Button>
    </div>
  </div>
</Panel>

<style>
  .export {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .export__title {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .export__group {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: 0;
  }

  .export__legend {
    padding: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .export__row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .export__warnings {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .export__warning {
    color: var(--color-warning);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .export__note,
  .export__error {
    margin: 0;
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .export__note {
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .export__error {
    color: var(--color-danger);
  }

  .export__actions {
    display: flex;
    gap: var(--space-2);
    padding-top: var(--space-1);
  }
</style>
