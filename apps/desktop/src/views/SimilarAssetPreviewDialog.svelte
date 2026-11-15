<script lang="ts">
  import {
    ActionIcon,
    Dialog,
    DocumentViewer,
    IconButton,
    type DocumentViewerProps,
    type ViewerType,
  } from '@entropia/ui'
  import OcrRichText from '../components/OcrRichText.svelte'
  import { getAssetUrl, loadAudioPreviewBlob } from '$lib/file-import'
  import { getAssetPathLabel, getAssetTypeLabel } from '$lib/item-metadata'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { OcrSourceType } from '$lib/ocr-rich-text'
  import type { SimilarAsset } from '$lib/nlp'

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

  async function loadAudioFallbackBlob(nativePath: string): Promise<Blob> {
    return loadAudioPreviewBlob(nativePath)
  }
</script>

<!-- Dialog portals itself out of the work pane, which is a size container
     and would clip this full-window overlay. -->
<Dialog
  size="viewer"
  nested
  eyebrow={translate('item.similarAssetPreview.eyebrow')}
  title={assetTitle}
  description="{getAssetTypeLabel(asset.assetType)} · {getAssetPathLabel(asset.assetPath)}"
  {onclose}
>
  {#snippet headerActions()}
    <span class="asset-preview__score">
      {translate('item.similarAssetPreview.similarity', { value: similarityPercent })}
    </span>
    <IconButton
      variant="secondary"
      size="lg"
      label={translate('item.similarAssetPreview.close')}
      title={translate('item.similarAssetPreview.close')}
      data-dialog-initial-focus
      onclick={onclose}
    >
      <ActionIcon name="close" size={20} />
    </IconButton>
  {/snippet}

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
</Dialog>

<style>
  .asset-preview__context h3,
  .asset-preview__context p {
    margin: 0;
  }

  .asset-preview__score {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
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
    letter-spacing: var(--letter-spacing-caps);
    text-transform: uppercase;
  }

  .asset-preview__context .asset-preview__empty {
    color: var(--color-text-secondary);
    font-style: italic;
  }

  @media (max-width: 760px) {
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
</style>
