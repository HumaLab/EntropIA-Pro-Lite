<script lang="ts">
  import {
    ActionIcon,
    DocumentViewer,
    IconButton,
    TabButton,
    TabList,
    type AnnotationTool,
    type DocumentViewerProps,
    type EditTool,
    type ViewerAnnotation,
    type ViewerLayoutRegion,
    type ViewerType,
  } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import { buildExportDefaultName } from '$lib/item-metadata'
  import { loadAudioPreviewBlob } from '$lib/file-import'
  import { exportOcrText, type OcrExportFormat } from '$lib/ocr-export'
  import type { Asset } from '@entropia/store'
  import type { AssetOcrState } from '$lib/ocr'
  import type { AssetTranscriptionState } from '$lib/transcription'
  import OcrRichText from '../components/OcrRichText.svelte'
  import { onDestroy } from 'svelte'

  let leftPanelTab = $state<'document' | 'text'>('document')
  let currentAssetId = $state<string | null>(null)
  let downloadMenuOpen = $state(false)
  let copyFeedback = $state<'idle' | 'success' | 'error'>('idle')
  let exportingFormat = $state<OcrExportFormat | null>(null)
  let exportError = $state(false)
  let downloadContainerEl = $state<HTMLElement | null>(null)
  let feedbackTimer: ReturnType<typeof setTimeout> | undefined
  let exportGeneration = 0
  let copyGeneration = 0
  const exportMenuId = 'left-panel-extracted-text-export-menu'

  let {
    selectedAsset,
    viewerSrc,
    viewerType,
    annotations,
    layoutRegions,
    showLayoutOverlay,
    hoveredLayoutRegionId,
    selectedLayoutRegionId,
    layoutReferenceWidth,
    layoutReferenceHeight,
    selectedAnnotationId,
    annotationTool,
    annotationColor,
    editTool,
    canUndo,
    canRedo,
    viewerPage,
    annotationSaveError,
    ocrState,
    ocrEditedText,
    transcriptionState,
    transcriptionEditedText,
    documentViewerLabels,
    annotationToolbarLabels,
    translate,
    onAnnotationsChange,
    onSelectedAnnotationIdChange,
    onAnnotationToolChange,
    onAnnotationColorChange,
    onLayoutRegionHoverChange,
    onLayoutRegionSelect,
    onEditSelect,
    onEditToolChange,
    onRotateLeft,
    onRotateRight,
    onFineRotateCommit,
    onUndo,
    onRedo,
    onDuplicateAsset,
    duplicateAssetDisabled,
    onPageChange,
    onDimensionsChange,
  }: {
    selectedAsset: Asset | null
    viewerSrc: string
    viewerType: ViewerType
    annotations: ViewerAnnotation[]
    layoutRegions: ViewerLayoutRegion[]
    showLayoutOverlay: boolean
    hoveredLayoutRegionId: string | null
    selectedLayoutRegionId: string | null
    layoutReferenceWidth: number
    layoutReferenceHeight: number
    selectedAnnotationId: string | null
    annotationTool: AnnotationTool
    annotationColor: string
    editTool: EditTool
    canUndo: boolean
    canRedo: boolean
    viewerPage: number
    annotationSaveError: string | null
    ocrState: AssetOcrState | null
    ocrEditedText: string
    transcriptionState: AssetTranscriptionState | null
    transcriptionEditedText: string
    documentViewerLabels: DocumentViewerProps['labels']
    annotationToolbarLabels: DocumentViewerProps['annotationToolbarLabels']
    translate: (key: I18nKey, params?: I18nParams) => string
    onAnnotationsChange: (annotations: ViewerAnnotation[]) => void
    onSelectedAnnotationIdChange: (annotationId: string | null) => void
    onAnnotationToolChange: (tool: AnnotationTool) => void
    onAnnotationColorChange: (color: string) => void
    onLayoutRegionHoverChange: (regionId: string | null) => void
    onLayoutRegionSelect: (regionId: string) => void
    onEditSelect: (region: { x: number; y: number; width: number; height: number }) => void | Promise<void>
    onEditToolChange: (tool: EditTool) => void
    onRotateLeft: () => void | Promise<void>
    onRotateRight: () => void | Promise<void>
    onFineRotateCommit: (degrees: number) => void | Promise<void>
    onUndo: () => void | Promise<void>
    onRedo: () => void | Promise<void>
    onDuplicateAsset: () => void | Promise<void>
    duplicateAssetDisabled: boolean
    onPageChange: (page: number, totalPages: number) => void
    onDimensionsChange: (dimensions: { width: number; height: number }) => void
  } = $props()

  $effect(() => {
    const nextAssetId = selectedAsset?.id ?? null

    if (nextAssetId !== currentAssetId) {
      currentAssetId = nextAssetId
      leftPanelTab = 'document'
      downloadMenuOpen = false
      copyFeedback = 'idle'
      exportingFormat = null
      exportError = false
      exportGeneration += 1
      copyGeneration += 1
      if (feedbackTimer) {
        clearTimeout(feedbackTimer)
        feedbackTimer = undefined
      }
    }
  })

  async function loadAudioFallbackBlob(nativePath: string): Promise<Blob> {
    return loadAudioPreviewBlob(nativePath)
  }

  function showCopyFeedback(next: 'success' | 'error') {
    copyFeedback = next
    if (feedbackTimer) clearTimeout(feedbackTimer)
    feedbackTimer = setTimeout(() => {
      copyFeedback = 'idle'
      feedbackTimer = undefined
    }, 2200)
  }

  async function handleCopyExtractedText() {
    if (!selectedAsset || selectedAsset.type === 'audio') return

    const assetId = selectedAsset.id
    const generation = ++copyGeneration

    try {
      await navigator.clipboard.writeText(ocrEditedText)
      if (generation === copyGeneration && assetId === currentAssetId) {
        showCopyFeedback('success')
      }
    } catch {
      if (generation === copyGeneration && assetId === currentAssetId) {
        showCopyFeedback('error')
      }
    }
  }

  function closeDownloadMenu(restoreFocus = false) {
    downloadMenuOpen = false
    if (restoreFocus) {
      downloadContainerEl?.querySelector<HTMLButtonElement>('[data-export-trigger]')?.focus()
    }
  }

  function handleWindowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && downloadMenuOpen) {
      event.preventDefault()
      closeDownloadMenu(true)
    }
  }

  function handleWindowPointerdown(event: PointerEvent) {
    const target = event.target
    if (downloadMenuOpen && target instanceof Node && !downloadContainerEl?.contains(target)) {
      closeDownloadMenu()
    }
  }


  async function handleExport(format: OcrExportFormat) {
    const asset = selectedAsset
    const source = ocrEditedText
    if (!asset || asset.type === 'audio' || !source.trim() || exportingFormat) return

    const generation = ++exportGeneration
    exportingFormat = format
    exportError = false
    closeDownloadMenu()

    try {
      await exportOcrText(
        {
          source,
          assetUrl: viewerSrc,
          sourceType: viewerType === 'pdf' ? 'pdf' : 'image',
          referenceWidth: layoutReferenceWidth,
          referenceHeight: layoutReferenceHeight,
        },
        format,
        `${buildExportDefaultName(asset.path)}.${format === 'markdown' ? 'md' : format}`
      )
    } catch {
      if (generation === exportGeneration) exportError = true
    } finally {
      if (generation === exportGeneration) exportingFormat = null
    }
  }

  onDestroy(() => {
    if (feedbackTimer) clearTimeout(feedbackTimer)
  })
</script>

<svelte:window onkeydown={handleWindowKeydown} onpointerdown={handleWindowPointerdown} />

{#if selectedAsset}
  <TabList class="left-panel-tabs" aria-label={translate('item.assetPanel')}>
    <TabButton
      id="left-panel-tab-document"
      active={leftPanelTab === 'document'}
      class="left-panel-tab"
      aria-controls="left-panel-document"
      onclick={() => {
        leftPanelTab = 'document'
      }}
    >
      {translate('item.documentTab')}
    </TabButton>
    <TabButton
      id="left-panel-tab-text"
      active={leftPanelTab === 'text'}
      class="left-panel-tab"
      aria-controls="left-panel-text"
      onclick={() => {
        leftPanelTab = 'text'
      }}
    >
      {translate('item.extractedTextTab')}
    </TabButton>
  </TabList>

  <div class="left-panel-content">
    <div
      id="left-panel-document"
      role="tabpanel"
      aria-labelledby="left-panel-tab-document"
      class="left-panel-pane left-panel-pane--document"
      class:is-hidden={leftPanelTab !== 'document'}
    >
      <DocumentViewer
        path={selectedAsset.path}
        assetUrl={viewerSrc}
        type={viewerType}
        {annotations}
        {layoutRegions}
        {showLayoutOverlay}
        {hoveredLayoutRegionId}
        {selectedLayoutRegionId}
        {layoutReferenceWidth}
        {layoutReferenceHeight}
        {selectedAnnotationId}
        {annotationTool}
        {annotationColor}
        {editTool}
        {canUndo}
        {canRedo}
        currentPage={viewerPage}
        {onAnnotationsChange}
        {onSelectedAnnotationIdChange}
        {onAnnotationToolChange}
        {onAnnotationColorChange}
        {onLayoutRegionHoverChange}
        {onLayoutRegionSelect}
        {onEditSelect}
        {onEditToolChange}
        {onRotateLeft}
        {onRotateRight}
        {onFineRotateCommit}
        {onUndo}
        {onRedo}
        {onDuplicateAsset}
        {duplicateAssetDisabled}
        {onPageChange}
        {onDimensionsChange}
        audioFallbackBlobLoader={loadAudioFallbackBlob}
        labels={documentViewerLabels}
        {annotationToolbarLabels}
      />

      {#if annotationSaveError}
        <p class="error">{annotationSaveError}</p>
      {/if}
    </div>

    <div
      id="left-panel-text"
      role="tabpanel"
      aria-labelledby="left-panel-tab-text"
      class="left-panel-pane left-panel-pane--text"
      class:is-hidden={leftPanelTab !== 'text'}
      hidden={leftPanelTab !== 'text'}
    >
      {#if selectedAsset.type !== 'audio'}
        <section class="left-text-panel-section">
          <div class="left-text-panel-card">
            {#if ocrEditedText.trim()}
              <div class="left-text-panel-meta">
                <div class="left-text-panel-meta__details">
                  <span>{translate('item.extractedText')}</span>
                  <span class="ocr-meta">
                    via {ocrState?.method ?? translate('item.ocrMethodUnknown')} · {translate(
                      'item.characters',
                      { count: ocrEditedText.length }
                    )}
                  </span>
                </div>

                <div
                  bind:this={downloadContainerEl}
                  class="left-text-panel-actions"
                  aria-live="polite"
                >
                  <IconButton
                    size="sm"
                    variant="ghost"
                    label={translate('item.copyExtractedTextAria')}
                    title={translate('item.copyExtractedText')}
                    disabled={exportingFormat !== null}
                    onclick={() => void handleCopyExtractedText()}
                  >
                    <ActionIcon name="copy" size={14} />
                  </IconButton>

                  <IconButton
                    data-export-trigger
                    size="sm"
                    variant="ghost"
                    label={translate('item.downloadExtractedTextAria')}
                    title={translate('item.downloadExtractedText')}
                    active={downloadMenuOpen}
                    disabled={exportingFormat !== null}
                    aria-haspopup="menu"
                    aria-expanded={downloadMenuOpen ? 'true' : 'false'}
                    aria-controls={downloadMenuOpen ? exportMenuId : undefined}
                    onclick={() => {
                      downloadMenuOpen = !downloadMenuOpen
                    }}
                  >
                    <ActionIcon name="download" size={14} />
                  </IconButton>

                  {#if downloadMenuOpen}
                    <div
                      id={exportMenuId}
                      class="left-text-panel-export-menu"
                      role="menu"
                      aria-label={translate('item.downloadExtractedTextMenu')}
                    >
                      <button
                        type="button"
                        role="menuitem"
                        disabled={exportingFormat !== null}
                        onclick={() => void handleExport('markdown')}
                      >
                        {translate('item.exportExtractedTextMarkdown')}
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        disabled={exportingFormat !== null}
                        onclick={() => void handleExport('pdf')}
                      >
                        {translate('item.exportExtractedTextPdf')}
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        disabled={exportingFormat !== null}
                        onclick={() => void handleExport('docx')}
                      >
                        {translate('item.exportExtractedTextDocx')}
                      </button>
                    </div>
                  {/if}

                  {#if copyFeedback !== 'idle'}
                    <span class="sr-only" role="status">
                      {translate(
                        copyFeedback === 'success'
                          ? 'item.copyExtractedTextSuccess'
                          : 'item.copyExtractedTextError'
                      )}
                    </span>
                  {/if}

                  {#if exportingFormat !== null}
                    <span class="sr-only" role="status">
                      {translate('item.exportExtractedTextWorking')}
                    </span>
                  {/if}

                  {#if exportError}
                    <span class="sr-only" role="alert">
                      {translate('item.exportExtractedTextError')}
                    </span>
                  {/if}
                </div>
              </div>
              <div class="left-text-panel-body">
                <OcrRichText
                  text={ocrEditedText}
                  assetUrl={viewerSrc}
                  sourceType={viewerType === 'pdf' ? 'pdf' : 'image'}
                  referenceWidth={layoutReferenceWidth}
                  referenceHeight={layoutReferenceHeight}
                />
              </div>
            {:else}
              <p class="empty-text">{translate('item.noExtractedText')}</p>
            {/if}
          </div>
        </section>
      {:else}
        <section class="left-text-panel-section">
          <div class="left-text-panel-card">
            {#if transcriptionEditedText.trim()}
              <div class="left-text-panel-meta">
                <span>{translate('item.transcription')}</span>
                <span class="ocr-meta">
                  {#if transcriptionState?.language}{transcriptionState.language} &middot;
                  {/if}{translate('item.characters', { count: transcriptionEditedText.length })}
                  {#if transcriptionState?.durationMs}
                    &middot; {translate('item.audioDurationSeconds', {
                      count: Math.round(transcriptionState.durationMs / 1000),
                    })}{/if}
                </span>
              </div>
              <div class="left-text-panel-body left-text-panel-body--plain">
                {transcriptionEditedText}
              </div>
            {:else}
              <p class="empty-text">{translate('item.noExtractedText')}</p>
            {/if}
          </div>
        </section>
      {/if}
    </div>
  </div>
{:else}
  <div class="empty-viewer">
    <p>{translate('item.noAssets')}</p>
  </div>
{/if}

<style>
  :global(.left-panel-tabs) {
    display: flex;
    width: 100%;
    flex-shrink: 0;
    border-color: var(--border-subtle);
    background: var(--surface-input);
  }

  :global(.left-panel-tab) {
    flex: 1;
    min-width: 0;
  }

  .left-panel-content {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
    overflow: hidden;
  }

  .left-panel-pane {
    min-height: 0;
  }

  .left-panel-pane--document {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .left-panel-pane--document :global(.document-viewer) {
    flex: 1;
    min-height: 0;
  }

  .left-panel-pane--text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    padding: 0 var(--space-2);
  }

  .left-panel-pane.is-hidden {
    display: none;
  }

  .left-text-panel-section {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }

  .left-text-panel-card {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-panel);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.025);
  }

  .left-text-panel-meta {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    flex: 0 0 auto;
    min-width: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .left-text-panel-meta__details {
    display: flex;
    min-width: 0;
    align-items: baseline;
    gap: var(--space-2);
    overflow: hidden;
  }

  .left-text-panel-meta__details > :first-child {
    flex: 0 0 auto;
  }

  .left-text-panel-meta__details .ocr-meta {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .left-text-panel-actions {
    position: relative;
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    gap: var(--space-1);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  .left-text-panel-export-menu {
    position: absolute;
    z-index: 5;
    top: calc(100% + var(--space-1));
    right: 0;
    display: grid;
    min-width: 13rem;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    box-shadow: var(--shadow-lg);
  }

  .left-text-panel-export-menu button {
    display: flex;
    width: 100%;
    align-items: center;
    padding: var(--space-2) var(--space-3);
    border: 0;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--color-text-secondary);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }

  .left-text-panel-export-menu button:hover:not(:disabled),
  .left-text-panel-export-menu button:focus-visible {
    background: var(--color-accent-faint);
    color: var(--color-text-primary);
  }

  .left-text-panel-export-menu button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .left-text-panel-export-menu button:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .left-text-panel-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
    scrollbar-color: color-mix(in srgb, var(--color-text-muted) 58%, transparent) transparent;
    scrollbar-width: thin;
    padding: var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.6;
    word-break: break-word;
  }

  .left-text-panel-body--plain {
    white-space: pre-wrap;
  }

  .left-text-panel-body::-webkit-scrollbar {
    width: 8px;
  }

  .left-text-panel-body::-webkit-scrollbar-track {
    background: transparent;
  }

  .left-text-panel-body::-webkit-scrollbar-thumb {
    border: 2px solid transparent;
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--color-text-muted) 52%, transparent);
    background-clip: padding-box;
  }

  .empty-viewer {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 300px;
    color: var(--color-text-secondary);
    border: 1px dashed var(--color-hairline);
    border-radius: var(--radius-md);
  }

  .empty-text {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .error {
    color: var(--color-danger);
  }

  .ocr-meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
</style>
