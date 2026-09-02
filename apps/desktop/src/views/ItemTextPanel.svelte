<script lang="ts">
  import { ActionIcon, Button, StatusBadge, type StatusBadgeVariant } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { Asset } from '@entropia/store'
  import type { AssetOcrState, OcrMode } from '$lib/ocr'
  import type { AssetTranscriptionState } from '$lib/transcription'
  import type { ItemLlmState } from '$lib/llm'
  import { getAssetPathLabel } from '$lib/item-metadata'

  let {
    selectedAsset,
    assetsCount,
    allAssetsAreImages,
    selectedAssetIndex,
    ocrState,
    ocrEditedText,
    transcriptionState,
    transcriptionEditedText,
    llmState,
    llmAvailable,
    ocrCorrectionAvailable,
    localOcrAvailable,
    isOcrCorrected,
    ocrEditingDisabled,
    isSummarizing,
    currentSummary,
    translate,
    onExtractText,
    onCorrectOcr,
    onSummarize,
    onTranscribeAudio,
    onOcrTextInput,
    onTranscriptionTextInput,
  }: {
    selectedAsset: Asset | null
    assetsCount: number
    allAssetsAreImages: boolean
    selectedAssetIndex: number
    ocrState: AssetOcrState | null
    ocrEditedText: string
    transcriptionState: AssetTranscriptionState | null
    transcriptionEditedText: string
    llmState: ItemLlmState
    llmAvailable: boolean
    ocrCorrectionAvailable: boolean
    localOcrAvailable: boolean
    isOcrCorrected: boolean
    ocrEditingDisabled: boolean
    currentSummary: string | null
    isSummarizing: boolean
    translate: (key: I18nKey, params?: I18nParams) => string
    onExtractText: (asset: Asset, mode: OcrMode) => void | Promise<void>
    onCorrectOcr: () => void | Promise<void>
    onSummarize: () => void | Promise<void>
    onTranscribeAudio: (asset: Asset) => void | Promise<void>
    onOcrTextInput: (assetId: string, value: string) => void
    onTranscriptionTextInput: (assetId: string, value: string) => void
  } = $props()

  function getJobStatusBadgeVariant(status: string): StatusBadgeVariant {
    if (status === 'done') return 'success'
    if (status === 'running') return 'warning'
    if (status === 'pending') return 'info'
    if (status === 'error') return 'danger'
    return 'neutral'
  }

  function getCorrectionActionLabel(assetType: Asset['type']) {
    return assetType === 'pdf'
      ? translate('item.pdfCorrectAction')
      : translate('item.ocrCorrectAction')
  }

  function getSummaryActionLabel(assetType: Asset['type']) {
    if (assetType === 'pdf') {
      return translate('item.summaryPdfAction')
    }

    if (assetType === 'audio') {
      return translate('item.summaryAudioAction')
    }

    return translate('item.summaryAction')
  }

  function getTranscriptionActionLabel(busy: boolean) {
    return busy ? translate('item.transcribeBusyAction') : translate('item.transcribeShortAction')
  }

  function getTranscriptionStageLabel(stage?: string) {
    if (!stage) return ''

    switch (stage) {
      case 'uploading':
        return translate('item.transcriptionStage.uploading')
      case 'submitting_remote':
        return translate('item.transcriptionStage.submitting_remote')
      case 'polling_remote':
        return translate('item.transcriptionStage.polling_remote')
      default:
        return ''
    }
  }

  function getOcrStageLabel(stage?: string) {
    if (!stage || stage === 'done' || stage === 'error') return ''

    switch (stage) {
      case 'reading':
        return translate('item.ocrStage.reading')
      case 'extracting_native':
        return translate('item.ocrStage.extracting_native')
      case 'ocr_inference':
        return translate('item.ocrStage.ocr_inference')
      case 'paddlevl_detection':
        return translate('item.ocrStage.paddlevl_detection')
      case 'submitting_glm_ocr':
        return translate('item.ocrStage.submitting_glm_ocr')
      case 'waiting_glm_ocr':
        return translate('item.ocrStage.waiting_glm_ocr')
      case 'parsing_glm_ocr':
        return translate('item.ocrStage.parsing_glm_ocr')
      default:
        return ''
    }
  }

  function getAssetFilename(asset: Asset) {
    return getAssetPathLabel(asset.path)
  }
</script>

{#if selectedAsset && selectedAsset.type !== 'audio' && ocrState}
  {@const busy = ocrState.status === 'pending' || ocrState.status === 'running'}
  {@const isPdfAsset = selectedAsset.type === 'pdf'}
  <section class="section">
    <h3>
      {translate('item.textExtraction')}{#if assetsCount > 1}
        {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
    </h3>
    <div class="ocr-item">
      <div class="ocr-item-header">
        <span class="ocr-filename">
          {assetsCount > 1 && allAssetsAreImages
            ? translate('item.assetPageLabel', { page: selectedAssetIndex + 1 })
            : (getAssetFilename(selectedAsset) ?? translate('item.assetNoSelection'))}
        </span>
        <StatusBadge
          variant={getJobStatusBadgeVariant(ocrState.status)}
          size="sm"
          class="ocr-status-badge"
        >
          {ocrState.status}
        </StatusBadge>
        <div class="ocr-btn-group">
          {#if !isPdfAsset && localOcrAvailable}
            <Button
              variant="secondary"
              size="sm"
              class="ocr-btn"
              disabled={busy}
              onclick={() => onExtractText(selectedAsset, 'light')}
              title={busy ? translate('item.ocrFastBusyTitle') : translate('item.ocrFastTitle')}
            >
              {translate('item.ocrFastAction')}
            </Button>
          {/if}
          <Button
            variant="secondary"
            size="sm"
            class="ocr-btn"
            disabled={busy}
            onclick={() => onExtractText(selectedAsset, 'high')}
            title={busy ? translate('item.ocrHighBusyTitle') : translate('item.ocrHighTitle')}
          >
            {translate('item.ocrHighAction')}
          </Button>
          {#if ocrCorrectionAvailable && !isOcrCorrected}
            <Button
              variant="secondary"
              size="sm"
              class="ocr-btn"
              disabled={ocrEditingDisabled ||
                llmState.status === 'running' ||
                ocrState.status !== 'done'}
              onclick={onCorrectOcr}
              title={!ocrCorrectionAvailable
                ? translate('item.ocrCorrectUnavailable')
                : ocrState.status !== 'done'
                  ? translate('item.ocrCorrectNeedsText')
                  : isPdfAsset
                    ? translate('item.pdfCorrectTitle')
                    : translate('item.ocrCorrectTitle')}
            >
              {getCorrectionActionLabel(selectedAsset.type)}
            </Button>
          {/if}
          {#if llmAvailable}
            <Button
              variant="secondary"
              size="sm"
              class="ocr-btn"
              disabled={llmState.status === 'running' || ocrState.status !== 'done'}
              onclick={onSummarize}
              title={!llmAvailable
                ? translate('item.summaryUnavailable')
                : ocrState.status !== 'done'
                  ? translate('item.summaryNeedsText')
                  : translate('item.summaryTitle')}
            >
              {getSummaryActionLabel(selectedAsset.type)}
            </Button>
          {/if}
        </div>
        {#if !llmAvailable}
          <p class="ocr-llm-hint">{translate('item.llmUnavailableHint')}</p>
        {/if}
      </div>

      {#if ocrState.status === 'running'}
        {@const ocrStageLabel = getOcrStageLabel(ocrState.stage)}
        <progress class="ocr-progress" value={ocrState.progress} max="100">
          {ocrState.progress}%
        </progress>
        <p class="ocr-status-text">
          {ocrStageLabel
            ? translate('item.extractionRunningStage', {
                progress: ocrState.progress,
                stage: ocrStageLabel,
              })
            : translate('item.extractionRunning', { progress: ocrState.progress })}
        </p>
      {:else if ocrState.status === 'pending'}
        <p class="ocr-status-text">{translate('item.extractionStarting')}</p>
      {:else if ocrState.status === 'error'}
        <p class="ocr-error">
          {translate('item.extractionFailed', { error: ocrState.error ?? '' })}
        </p>
        {#if ocrEditedText.trim()}
          {@const displayLength = ocrEditedText.length}
          <details class="ocr-result" open>
            <summary>
              {translate('item.extractedText')}
              <span class="ocr-meta">
                via {ocrState.method ?? translate('item.ocrMethodUnknown')} · {translate(
                  'item.characters',
                  { count: displayLength }
                )}
              </span>
            </summary>
            <textarea
              class="ocr-result-body ocr-textarea"
              rows="8"
              disabled={ocrEditingDisabled}
              oninput={(event) => onOcrTextInput(selectedAsset.id, event.currentTarget.value)}
              >{ocrEditedText}</textarea
            >
          </details>
        {/if}
      {:else if ocrState.status === 'done'}
        {#if ocrState.warning}
          <p class="ocr-warning">{ocrState.warning}</p>
        {/if}
        {@const displayLength = ocrEditedText.length}
        <details class="ocr-result">
          <summary>
            {translate('item.extractedText')}
            <span class="ocr-meta">
              via {ocrState.method ?? translate('item.ocrMethodUnknown')} · {translate(
                'item.characters',
                { count: displayLength }
              )}
            </span>
          </summary>
          <textarea
            class="ocr-result-body ocr-textarea"
            rows="8"
            disabled={ocrEditingDisabled}
            oninput={(event) => onOcrTextInput(selectedAsset.id, event.currentTarget.value)}
            >{ocrEditedText}</textarea
          >
        </details>
      {/if}
    </div>
  </section>
{/if}

{#if selectedAsset && selectedAsset.type === 'audio' && transcriptionState}
  {@const busy = transcriptionState.status === 'pending' || transcriptionState.status === 'running'}
  <section class="section">
    <h3>
      {translate('item.audioTranscription')}{#if assetsCount > 1}
        {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
    </h3>
    <div class="ocr-item">
      <div class="ocr-item-header">
        <span class="ocr-filename"
          ><ActionIcon name="volume" size={16} />
          <span class="ocr-filename__text">
            {getAssetFilename(selectedAsset) ?? translate('item.audioLabel')}
          </span></span
        >
        <StatusBadge
          variant={getJobStatusBadgeVariant(transcriptionState.status)}
          size="sm"
          class="ocr-status-badge"
        >
          {transcriptionState.status}
        </StatusBadge>
        <div class="ocr-btn-group">
          <Button
            variant="secondary"
            size="sm"
            class="ocr-btn"
            disabled={busy}
            onclick={() => onTranscribeAudio(selectedAsset)}
            title={busy ? translate('item.transcribeBusyTitle') : translate('item.transcribeTitle')}
          >
            {getTranscriptionActionLabel(busy)}
          </Button>
          {#if llmAvailable}
            <Button
              variant="secondary"
              size="sm"
              class="ocr-btn"
              disabled={llmState.status === 'running' || transcriptionState.status !== 'done'}
              onclick={onSummarize}
              title={!llmAvailable
                ? translate('item.summaryUnavailable')
                : transcriptionState.status !== 'done'
                  ? translate('item.summaryNeedsText')
                  : translate('item.summaryTitle')}
            >
              {getSummaryActionLabel(selectedAsset.type)}
            </Button>
          {/if}
        </div>
      </div>

      {#if transcriptionState.status === 'running'}
        <progress class="ocr-progress" value={transcriptionState.progress} max="100">
          {transcriptionState.progress}%
        </progress>
        <p class="ocr-status-text">
          {translate('item.transcriptionRunning', { progress: transcriptionState.progress })}
          {#if getTranscriptionStageLabel(transcriptionState.stage)}
            · {getTranscriptionStageLabel(transcriptionState.stage)}
          {/if}
        </p>
      {:else if transcriptionState.status === 'pending'}
        <p class="ocr-status-text">{translate('item.transcriptionStarting')}</p>
      {:else if transcriptionState.status === 'error'}
        <p class="ocr-error">
          {translate('item.transcriptionFailed', { error: transcriptionState.error ?? '' })}
        </p>
      {:else if transcriptionState.status === 'done'}
        {@const displayLength = transcriptionEditedText.length}
        <details class="ocr-result">
          <summary>
            {translate('item.transcription')}
            <span class="ocr-meta">
              {#if transcriptionState.language}{transcriptionState.language} &middot;
              {/if}{translate('item.characters', { count: displayLength })}
              {#if transcriptionState.durationMs}
                &middot; {translate('item.audioDurationSeconds', {
                  count: Math.round(transcriptionState.durationMs / 1000),
                })}{/if}
            </span>
          </summary>
          <textarea
            class="ocr-result-body ocr-textarea"
            rows="8"
            oninput={(event) =>
              onTranscriptionTextInput(selectedAsset.id, event.currentTarget.value)}
            >{transcriptionEditedText}</textarea
          >
        </details>
      {/if}
    </div>
  </section>
{/if}

{#if selectedAsset && (currentSummary || isSummarizing)}
  <section class="section">
    <h3>
      {translate('item.summary')}{#if assetsCount > 1}
        {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
    </h3>
    {#if isSummarizing}
      <p class="summary-status">{translate('item.generatingSummary')}</p>
    {:else if currentSummary}
      <div class="summary-result">
        <pre class="summary-text">{currentSummary}</pre>
      </div>
    {/if}
  </section>
{/if}

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    box-shadow: var(--shadow-surface);
  }

  .section h3 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    margin-bottom: var(--space-1);
  }

  .summary-result {
    margin-top: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--surface-input);
  }

  .summary-status {
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
    font-style: italic;
  }

  .summary-text {
    margin: 0;
    font-size: var(--font-size-sm);
    font-family: var(--font-sans);
    white-space: pre-wrap;
    word-wrap: break-word;
    max-height: 300px;
    overflow-y: auto;
    line-height: 1.6;
    color: var(--color-text-secondary);
  }

  .ocr-item {
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    background: var(--surface-card);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.025);
    container-type: inline-size;
    container-name: ocr-item;
  }

  .ocr-item-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .ocr-filename {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1 1 120px;
    min-width: 0;
  }

  .ocr-filename :global(svg) {
    flex-shrink: 0;
  }

  .ocr-filename__text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  :global(.ocr-status-badge) {
    flex-shrink: 0;
    text-transform: uppercase;
  }

  /* Surface, states and geometry all come from Button; only how the control
     shares the row belongs to this screen. */
  .ocr-btn-group :global(.ocr-btn) {
    flex-shrink: 0;
  }

  .ocr-btn-group {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-left: auto;
    max-width: 100%;
  }
  @media (max-width: 420px) {
    .ocr-btn-group {
      flex: 1 1 100%;
      justify-content: flex-start;
    }
    .ocr-btn-group :global(.ocr-btn) {
      flex: 1 1 auto;
      min-width: 64px;
    }
  }

  @container ocr-item (max-width: 420px) {
    .ocr-btn-group {
      flex: 1 1 100%;
      justify-content: flex-start;
    }
    .ocr-btn-group :global(.ocr-btn) {
      flex: 1 1 auto;
      min-width: 64px;
    }
  }

  .ocr-llm-hint {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    margin: var(--space-1) 0 0;
    font-style: italic;
  }

  .ocr-progress {
    width: 100%;
    height: 6px;
    border-radius: var(--radius-sm);
    appearance: none;
  }

  .ocr-status-text {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .ocr-error {
    font-size: var(--font-size-xs);
    color: var(--color-danger);
  }

  .ocr-meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .ocr-result {
    font-size: var(--font-size-sm);
  }
  .ocr-result summary {
    cursor: pointer;
    color: var(--color-text-secondary);
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
  }
  .ocr-result-body {
    margin-top: var(--space-1);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .ocr-textarea {
    width: 100%;
    min-height: 7rem;
    padding: var(--space-1) var(--space-2);
    font-family: var(--font-mono, ui-monospace, SFMono-Regular, monospace);
    font-size: var(--font-size-sm);
    line-height: 1.5;
    color: var(--color-text-secondary);
    background: var(--surface-input);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    resize: vertical;
    white-space: pre-wrap;
    word-break: break-word;
    outline: none;
    transition:
      border-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .ocr-textarea:focus {
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .ocr-textarea:hover:not(:focus) {
    border-color: var(--border-panel);
  }
</style>
