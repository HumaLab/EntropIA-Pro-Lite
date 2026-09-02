<script lang="ts">
  import {
    ActionIcon,
    DocumentViewer,
    type DocumentViewerProps,
    type ViewerType,
  } from '@entropia/ui'
  import OcrRichText from '../components/OcrRichText.svelte'
  import { getAssetUrl, loadAudioPreviewBlob } from '$lib/file-import'
  import { getAssetPathLabel, getAssetTypeLabel } from '$lib/item-metadata'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { OcrSourceType } from '$lib/ocr-rich-text'
  import type { SimilarAsset } from '$lib/nlp'

  const FOCUSABLE_SELECTOR =
    'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

  let {
    asset,
    translate,
    documentViewerLabels,
    loadFullText,
    onclose,
  }: {
    asset: SimilarAsset
    translate: (key: I18nKey, params?: I18nParams) => string
    documentViewerLabels: DocumentViewerProps['labels']
    loadFullText: (assetId: string) => Promise<string | null>
    onclose: () => void
  } = $props()

  let titleId = `similar-asset-preview-title-${Math.random().toString(36).slice(2)}`
  let dialogEl: HTMLDivElement | undefined = $state()
  let previouslyFocused: HTMLElement | null = null

  let assetTitle = $derived(asset.title || getAssetPathLabel(asset.assetPath) || asset.itemId)
  let viewerType = $derived<ViewerType>(
    asset.assetType.toLowerCase() === 'pdf'
      ? 'pdf'
      : asset.assetType.toLowerCase() === 'audio'
        ? 'audio'
        : 'image'
  )
  // El renderer solo distingue imagen y PDF para recortar regiones OCR; el
  // audio no trae ninguna, así que cae del lado de imagen sin consecuencias.
  let ocrSourceType = $derived<OcrSourceType>(viewerType === 'pdf' ? 'pdf' : 'image')
  let similarityPercent = $derived((asset.similarity * 100).toFixed(1))
  let fullTextPromise = $derived(loadFullText(asset.assetId))

  function getDisplayText(fullText: string | null | undefined) {
    return fullText?.trim() || asset.textPreview?.trim() || ''
  }

  function getFocusableElements(): HTMLElement[] {
    if (!dialogEl) return []
    return Array.from(dialogEl.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (element) => !element.hasAttribute('disabled') && !element.hasAttribute('hidden')
    )
  }

  function handleWindowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      requestClose()
      return
    }

    if (event.key !== 'Tab') return

    const focusableElements = getFocusableElements()
    if (focusableElements.length === 0) {
      event.preventDefault()
      dialogEl?.focus()
      return
    }

    const currentElement =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    const currentIndex = currentElement ? focusableElements.indexOf(currentElement) : -1
    const nextElement = event.shiftKey
      ? currentIndex <= 0
        ? focusableElements[focusableElements.length - 1]
        : focusableElements[currentIndex - 1]
      : currentIndex === -1 || currentIndex === focusableElements.length - 1
        ? focusableElements[0]
        : focusableElements[currentIndex + 1]

    event.preventDefault()
    ;(nextElement ?? dialogEl)?.focus()
  }

  function handleOverlayClick(event: MouseEvent) {
    if (event.target === event.currentTarget) requestClose()
  }

  function requestClose() {
    const returnFocusTarget = previouslyFocused
    onclose()
    queueMicrotask(() => {
      if (returnFocusTarget?.isConnected) returnFocusTarget.focus()
    })
  }

  async function loadAudioFallbackBlob(nativePath: string): Promise<Blob> {
    return loadAudioPreviewBlob(nativePath)
  }

  $effect(() => {
    if (!dialogEl) return
    previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    const initialFocusTarget =
      dialogEl?.querySelector<HTMLElement>('[data-autofocus]') ??
      getFocusableElements()[0] ??
      dialogEl

    initialFocusTarget?.focus()
    window.addEventListener('keydown', handleWindowKeydown, true)

    return () => {
      window.removeEventListener('keydown', handleWindowKeydown, true)
    }
  })
</script>

<div class="asset-preview__overlay" role="presentation" onclick={handleOverlayClick}>
  <div
    bind:this={dialogEl}
    class="asset-preview"
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
  >
    <header class="asset-preview__header">
      <div class="asset-preview__heading">
        <span class="asset-preview__eyebrow">{translate('item.similarAssetPreview.eyebrow')}</span>
        <h2 id={titleId}>{assetTitle}</h2>
        <p>{getAssetTypeLabel(asset.assetType)} · {getAssetPathLabel(asset.assetPath)}</p>
      </div>

      <div class="asset-preview__header-actions">
        <span class="asset-preview__score">
          {translate('item.similarAssetPreview.similarity', { value: similarityPercent })}
        </span>
        <button
          type="button"
          class="asset-preview__close"
          aria-label={translate('item.similarAssetPreview.close')}
          title={translate('item.similarAssetPreview.close')}
          data-autofocus
          onclick={requestClose}
        >
          <ActionIcon name="close" size={20} />
        </button>
      </div>
    </header>

    <div class="asset-preview__body">
      <div class="asset-preview__viewer" data-testid="similar-asset-preview-viewer">
        <DocumentViewer
          path={asset.assetPath}
          assetUrl={getAssetUrl(asset.assetPath)}
          type={viewerType}
          readOnly
          audioFallbackBlobLoader={loadAudioFallbackBlob}
          labels={{ ...documentViewerLabels, imageAlt: assetTitle }}
          annotationToolbarLabels={{
            toolbarAriaLabel: translate('item.similarAssetPreview.viewerTools'),
          }}
        />
      </div>

      <aside class="asset-preview__context">
        <h3>{translate('item.similarAssetPreview.relatedText')}</h3>
        {#await fullTextPromise}
          <p class="asset-preview__empty">{translate('item.similarAssetPreview.loadingText')}</p>
        {:then fullText}
          {@const displayText = getDisplayText(fullText)}
          {#if displayText}
            <OcrRichText
              text={displayText}
              assetUrl={getAssetUrl(asset.assetPath)}
              sourceType={ocrSourceType}
              referenceWidth={0}
              referenceHeight={0}
            />
          {:else}
            <p class="asset-preview__empty">{translate('item.similarAssetsNoPreview')}</p>
          {/if}
        {:catch}
          {@const fallbackText = getDisplayText(null)}
          {#if fallbackText}
            <OcrRichText
              text={fallbackText}
              assetUrl={getAssetUrl(asset.assetPath)}
              sourceType={ocrSourceType}
              referenceWidth={0}
              referenceHeight={0}
            />
          {:else}
            <p class="asset-preview__empty">{translate('item.similarAssetsNoPreview')}</p>
          {/if}
        {/await}
      </aside>
    </div>
  </div>
</div>

<style>
  .asset-preview__overlay {
    position: fixed;
    inset: 0;
    z-index: 1100;
    display: grid;
    place-items: center;
    padding: var(--space-4);
    background: color-mix(in srgb, var(--color-overlay) 88%, transparent);
    backdrop-filter: blur(8px);
  }

  .asset-preview {
    display: flex;
    flex-direction: column;
    width: min(1120px, 100%);
    height: min(820px, calc(100vh - var(--space-8)));
    min-height: 0;
    overflow: hidden;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-dialog);
    background: var(--color-surface-glass);
    box-shadow: var(--shadow-lg);
    color: var(--color-text-primary);
  }

  .asset-preview:focus-visible {
    outline: none;
    box-shadow: var(--shadow-lg), var(--focus-ring);
  }

  .asset-preview__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--color-hairline);
    background: color-mix(in srgb, var(--surface-panel) 88%, transparent);
  }

  .asset-preview__heading {
    min-width: 0;
  }

  .asset-preview__eyebrow {
    display: block;
    margin-bottom: var(--space-1);
    color: var(--color-accent);
    font-family: var(--font-mono);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .asset-preview__heading h2,
  .asset-preview__heading p,
  .asset-preview__context h3,
  .asset-preview__context p {
    margin: 0;
  }

  .asset-preview__heading h2 {
    overflow: hidden;
    color: var(--color-text-primary);
    font-size: var(--font-size-lg);
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .asset-preview__heading p {
    margin-top: var(--space-1);
    overflow: hidden;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .asset-preview__header-actions {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: var(--space-2);
  }

  .asset-preview__score {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
  }

  .asset-preview__close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    padding: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .asset-preview__close:hover {
    border-color: var(--color-accent);
    color: var(--color-text-primary);
  }

  .asset-preview__close:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .asset-preview__body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(240px, 30%);
    flex: 1;
    min-height: 0;
  }

  .asset-preview__viewer {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border-right: 1px solid var(--color-hairline);
    background: var(--surface-app);
  }

  /* El cuerpo tipográfico va en el panel, no en un selector de `p`: el HTML lo
     genera OcrRichText y los estilos con hash de Svelte no lo alcanzan. Estos
     son los mismos valores que usa el panel de Texto extraído. */
  .asset-preview__context {
    min-width: 0;
    overflow-y: auto;
    padding: var(--space-4);
    background: var(--surface-card);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    word-break: break-word;
  }

  .asset-preview__context h3 {
    margin-bottom: var(--space-3);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .asset-preview__context .asset-preview__empty {
    color: var(--color-text-secondary);
    font-style: italic;
  }

  @media (max-width: 760px) {
    .asset-preview__overlay {
      padding: var(--space-2);
    }

    .asset-preview {
      height: calc(100vh - var(--space-4));
    }

    .asset-preview__header {
      padding: var(--space-3);
    }

    .asset-preview__score {
      display: none;
    }

    .asset-preview__body {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 2fr) minmax(140px, 1fr);
    }

    .asset-preview__viewer {
      border-right: 0;
      border-bottom: 1px solid var(--color-hairline);
    }

    .asset-preview__context {
      padding: var(--space-3);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .asset-preview__close {
      transition: none;
    }
  }
</style>
