<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import {
    bibliographyReprocessCandidates,
    bibliographyReprocessCandidatesCancel,
    bibliographyReprocessConfirm,
    bibliographyReprocessPreview,
    bibliographyReprocessPreviewCancel,
    formatEstimatedUsd,
    onBibliographyReprocessCandidatesProgress,
    onBibliographyReprocessPreviewProgress,
    previewUnitPercent,
    type ReprocessCandidatesProgress,
    type ReprocessConfirm,
    type ReprocessPreview,
    type ReprocessPreviewProgress,
  } from '$lib/bibliography-reprocess'
  import { ConfirmDialog } from '@entropia/ui'

  /**
   * The owner's text reprocess flow (plan-texto-nativo-parte-b 2.5): scan →
   * preview → costed summary → one explicit confirm. Built from ConfirmDialog
   * alone, with the preview progress as text and the confirm action absent
   * until there is something to approve. Nothing here spends money except the
   * confirm button, which sends exactly `{attachmentId, planHash}` entries.
   */
  interface Props {
    /** 'library' scans for damaged text; 'work' previews the given PDFs. */
    mode: 'library' | 'work'
    /** Work mode: the catalog ids of the work's PDF attachments. */
    attachmentIds?: string[]
    onclose: () => void
  }

  let { mode, attachmentIds = [], onclose }: Props = $props()

  const currentLocale = locale

  type Phase = 'loading' | 'previewing' | 'summary' | 'no-candidates' | 'done' | 'error'

  // The props are fixed for the dialog's lifetime: the phase starts where the
  // mode starts and the ids are the ones the view opened it with.
  // svelte-ignore state_referenced_locally
  let phase = $state<Phase>(mode === 'work' ? 'previewing' : 'loading')
  // svelte-ignore state_referenced_locally
  let progress = $state<ReprocessPreviewProgress>({
    done: 0,
    total: attachmentIds.length,
    unitsDone: 0,
    unitsTotal: 0,
  })
  let preview = $state<ReprocessPreview | null>(null)
  // The scan counter; the plain loading line stands until the first event.
  let candidatesProgress = $state<ReprocessCandidatesProgress>({ done: 0, total: 0 })
  let doneReport = $state<ReprocessConfirm | null>(null)
  let errorMessage = $state<string | null>(null)
  let confirming = $state(false)
  let alive = true

  /** Entries the confirm may queue: they have a plan and no live task. */
  const confirmable = $derived(
    (preview?.attachments ?? []).filter(
      (entry) => entry.planHash !== null && !entry.busy && !entry.unreadable
    )
  )
  /** Listed but never queued: busy or unreadable attachments. */
  const skipped = $derived(
    (preview?.attachments ?? []).filter((entry) => entry.busy || entry.unreadable)
  )
  const usdLabel = $derived.by(() => {
    $currentLocale
    return formatEstimatedUsd(preview?.totals.estimatedUsd ?? 0, $currentLocale)
  })
  const confirmLabel = $derived.by(() => {
    $currentLocale
    return t('bibliography.reprocess.confirm', { usd: usdLabel })
  })
  const cancelLabel = $derived.by(() => {
    $currentLocale
    return phase === 'previewing'
      ? t('bibliography.reprocess.cancel')
      : t('bibliography.reprocess.close')
  })
  // The counter of the attachment being read; its percentage appears only
  // while the reader knows that attachment's unit total.
  const previewProgressLabel = $derived.by(() => {
    $currentLocale
    if (progress.unitsTotal > 0) {
      return t('bibliography.reprocess.previewProgressUnits', {
        done: progress.done,
        total: progress.total,
        percent: previewUnitPercent(progress),
      })
    }
    return t('bibliography.reprocess.previewProgress', {
      done: progress.done,
      total: progress.total,
    })
  })
  // The scan counter, once the scan has told the dialog its total.
  const candidatesProgressLabel = $derived.by(() => {
    $currentLocale
    if (candidatesProgress.total > 0) {
      return t('bibliography.reprocess.candidatesProgress', {
        done: candidatesProgress.done,
        total: candidatesProgress.total,
      })
    }
    return t('bibliography.reprocess.loadingCandidates')
  })
  const queuedCount = $derived(
    (doneReport?.results ?? []).filter((result) => result.status === 'queued').length
  )
  // "How many busy" spans both sources: attachments the preview refused and
  // entries the confirm refused in the meantime.
  const busyCount = $derived(
    (doneReport?.results ?? []).filter((result) => result.status === 'busy').length +
      (preview?.attachments ?? []).filter((entry) => entry.busy).length
  )
  // Entries the confirm could not even look up (the attachment vanished or
  // is not a PDF anymore) are their own count, never dropped silently.
  const notQueuedCount = $derived(
    (doneReport?.results ?? []).filter((result) => result.status === 'unknown_attachment').length
  )

  function errorText(error: unknown): string {
    if (typeof error === 'string') return error
    if (error instanceof Error) return error.message
    return ''
  }

  function unreadableText(reason: string): string {
    switch (reason) {
      case 'attachment_missing':
        return t('bibliography.reprocess.unreadableAttachmentMissing')
      case 'file_missing':
        return t('bibliography.reprocess.unreadableFileMissing')
      case 'file_too_large':
        return t('bibliography.reprocess.unreadableFileTooLarge')
      case 'not_a_pdf':
        return t('bibliography.reprocess.unreadableNotPdf')
      case 'read_failed':
        return t('bibliography.reprocess.unreadableReadFailed')
      default:
        return t('bibliography.reprocess.unreadableUnknown', { reason })
    }
  }

  /** Work mode skips the scan; library mode gates on it (2.5 paso 1). */
  async function start(): Promise<void> {
    try {
      let ids = attachmentIds
      if (mode === 'library') {
        const unlisten = await onBibliographyReprocessCandidatesProgress((next) => {
          if (alive) candidatesProgress = next
        })
        try {
          const candidates = await bibliographyReprocessCandidates()
          if (!alive) return
          ids = candidates.map((entry) => entry.attachmentId)
        } finally {
          unlisten()
        }
      }
      if (ids.length === 0) {
        phase = 'no-candidates'
        return
      }
      await runPreview(ids)
    } catch (error) {
      if (!alive) return
      errorMessage = t('bibliography.reprocess.error', { reason: errorText(error) })
      phase = 'error'
    }
  }

  async function runPreview(ids: string[]): Promise<void> {
    phase = 'previewing'
    progress = { done: 0, total: ids.length, unitsDone: 0, unitsTotal: 0 }
    const unlisten = await onBibliographyReprocessPreviewProgress((next) => {
      if (alive) progress = next
    })
    try {
      const result = await bibliographyReprocessPreview(ids)
      if (!alive) return
      preview = result
      phase = result.attachments.length > 0 ? 'summary' : 'no-candidates'
    } finally {
      unlisten()
    }
  }

  async function confirm(): Promise<void> {
    if (!preview || confirming || confirmable.length === 0) return
    confirming = true
    errorMessage = null
    try {
      const result = await bibliographyReprocessConfirm(
        confirmable.map((entry) => ({
          attachmentId: entry.attachmentId,
          planHash: entry.planHash ?? '',
        }))
      )
      if (!alive) return
      doneReport = result
      phase = 'done'
    } catch (error) {
      if (!alive) return
      errorMessage = t('bibliography.reprocess.error', { reason: errorText(error) })
    } finally {
      if (alive) confirming = false
    }
  }

  /** Closing drops any in-flight result; a running preview or scan is stopped too. */
  function handleCancel(): void {
    if (phase === 'previewing') {
      void bibliographyReprocessPreviewCancel()
    }
    if (phase === 'loading') {
      void bibliographyReprocessCandidatesCancel()
    }
    onclose()
  }

  onMount(() => {
    void start()
  })

  onDestroy(() => {
    alive = false
  })
</script>

<ConfirmDialog
  title={$currentLocale && t('bibliography.reprocess.title')}
  titleId="bibliography-reprocess-title"
  cancelLabel={$currentLocale && cancelLabel}
  confirmLabel={phase === 'summary' ? confirmLabel : undefined}
  confirmDisabled={confirmable.length === 0}
  {confirming}
  dismissOnOverlay={false}
  error={errorMessage}
  oncancel={handleCancel}
  onconfirm={() => void confirm()}
>
  {#if phase === 'loading'}
    <p class="reprocess-dialog__note" role="status">
      {$currentLocale && candidatesProgressLabel}
    </p>
  {:else if phase === 'previewing'}
    <p class="reprocess-dialog__note" role="status" aria-live="polite">
      {$currentLocale && previewProgressLabel}
    </p>
  {:else if phase === 'summary' && preview}
    <dl class="reprocess-dialog__summary">
      <div>
        <dt>{$currentLocale && t('bibliography.reprocess.summaryAttachments')}</dt>
        <dd>{preview.totals.attachments}</dd>
      </div>
      <div>
        <dt>{$currentLocale && t('bibliography.reprocess.summaryPages')}</dt>
        <dd>{preview.totals.pages}</dd>
      </div>
      <div>
        <dt>{$currentLocale && t('bibliography.reprocess.summaryOcrPages')}</dt>
        <dd>{preview.totals.ocrPages}</dd>
      </div>
      <div>
        <dt>{$currentLocale && t('bibliography.reprocess.summaryReusedOcr')}</dt>
        <dd>{preview.totals.reusedOcrPages}</dd>
      </div>
      <div>
        <dt>{$currentLocale && t('bibliography.reprocess.summaryFixedWithoutOcr')}</dt>
        <dd>{preview.totals.fixedWithoutOcr}</dd>
      </div>
      <div>
        <dt>{$currentLocale && t('bibliography.reprocess.summaryCost')}</dt>
        <dd>
          {$currentLocale &&
            (preview.totals.ocrPages === 0
              ? t('bibliography.reprocess.noOcrCost')
              : t('bibliography.reprocess.estimated', { usd: usdLabel }))}
        </dd>
      </div>
    </dl>
    <p class="reprocess-dialog__note">
      {$currentLocale && t('bibliography.reprocess.estimateNote')}
    </p>
    {#if skipped.length > 0}
      <div class="reprocess-dialog__skipped">
        <p class="reprocess-dialog__note">
          {$currentLocale && t('bibliography.reprocess.skippedTitle')}
        </p>
        <ul class="reprocess-dialog__skipped-list">
          {#each skipped as entry (entry.attachmentId)}
            <li class="reprocess-dialog__skipped-row">
              <span class="reprocess-dialog__skipped-name">
                {entry.title || entry.filename || entry.attachmentId}
              </span>
              <span class="reprocess-dialog__skipped-reason">
                {entry.busy
                  ? $currentLocale && t('bibliography.reprocess.busy')
                  : $currentLocale && unreadableText(entry.unreadable ?? 'read_failed')}
              </span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  {:else if phase === 'done' && doneReport}
    <p class="reprocess-dialog__note" role="status" aria-live="polite">
      {$currentLocale && t('bibliography.reprocess.doneQueued', { queued: queuedCount })}
    </p>
    <p class="reprocess-dialog__note">
      {$currentLocale && t('bibliography.reprocess.doneBusy', { busy: busyCount })}
    </p>
    <p class="reprocess-dialog__note">
      {$currentLocale && t('bibliography.reprocess.doneNotQueued', { notQueued: notQueuedCount })}
    </p>
    <p class="reprocess-dialog__note">
      {$currentLocale && t('bibliography.reprocess.doneBatch', { tab: t('settings.batchTab') })}
    </p>
  {:else if phase === 'no-candidates'}
    <p class="reprocess-dialog__note" role="status">
      {$currentLocale && t('bibliography.reprocess.noCandidates')}
    </p>
  {/if}
</ConfirmDialog>

<style>
  .reprocess-dialog__note {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
  }

  /* The summary is one definition list of counts, in the labels-over-numbers
     language of ImportSourcesDialog's summary counts. */
  .reprocess-dialog__summary {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2) var(--space-4);
    margin: 0;
  }

  .reprocess-dialog__summary dt {
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.075em;
    color: var(--color-text-muted);
  }

  .reprocess-dialog__summary dd {
    margin: 0;
    font-family: var(--font-reading);
    font-size: var(--font-size-lg);
    color: var(--color-text-primary);
  }

  .reprocess-dialog__skipped {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .reprocess-dialog__skipped-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 160px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .reprocess-dialog__skipped-row {
    display: flex;
    gap: var(--space-2);
    justify-content: space-between;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .reprocess-dialog__skipped-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .reprocess-dialog__skipped-reason {
    flex: none;
    color: var(--color-text-secondary);
  }
</style>
