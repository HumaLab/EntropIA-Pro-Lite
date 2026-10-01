<script lang="ts">
  /**
   * A saved web PDF, in the app's own PDF viewer (the one the corpus uses).
   *
   * The view names a capture and the backend answers where its saved file is
   * (`navegador_pdf_file`): this component never builds or accepts a path. The
   * file is read from disk through the asset protocol, so nothing is downloaded
   * and it works offline. The browser's native webview draws above any HTML, so
   * the view hides it while this is open (it is laid over the page area, which
   * is empty then); closing it shows the page again.
   */
  import { onMount } from 'svelte'
  import { convertFileSrc } from '@tauri-apps/api/core'
  import { ActionIcon, DocumentViewer, IconButton } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { navegadorPdfFile, parseSourceError } from '$lib/navegador-sources'

  let {
    captureId,
    title,
    onclose,
  }: {
    captureId: string
    title: string
    onclose: () => void
  } = $props()

  const currentLocale = locale

  let phase = $state<'loading' | 'ready' | 'error'>('loading')
  let path = $state('')
  let problem = $state('')

  /** The corpus viewer's own texts, in the current language. */
  const labels = $derived.by(() => {
    $currentLocale
    return {
      pdfLoading: t('item.viewerPdfLoading'),
      pdfLoadError: t('item.viewerPdfLoadError'),
      pdfRenderError: t('item.viewerPdfRenderError'),
      pdfPreviousPage: t('item.previousPage'),
      pdfNextPage: t('item.nextPage'),
      pdfZoomOut: t('item.toolbar.zoomOut'),
      pdfZoomIn: t('item.toolbar.zoomIn'),
    }
  })

  onMount(() => {
    let disposed = false
    navegadorPdfFile(captureId)
      .then((file) => {
        if (disposed) return
        path = file
        phase = 'ready'
      })
      .catch((reason) => {
        if (disposed) return
        const { code, detail } = parseSourceError(reason)
        problem = t(`navegador.pdf.error.${code}`, { message: detail ?? '' })
        phase = 'error'
      })
    return () => {
      disposed = true
    }
  })
</script>

<div class="pdf" role="region" aria-label={$currentLocale && t('navegador.pdf.region')}>
  <header class="pdf__head">
    <strong class="pdf__title">{title}</strong>
    <IconButton
      size="sm"
      variant="ghost"
      label={$currentLocale && t('navegador.pdf.close')}
      title={$currentLocale && t('navegador.pdf.close')}
      onclick={onclose}
    >
      <ActionIcon name="close" size={14} />
    </IconButton>
  </header>
  {#if phase === 'loading'}
    <p class="pdf__note" role="status">{$currentLocale && t('navegador.pdf.loading')}</p>
  {:else if phase === 'error'}
    <p class="pdf__note pdf__problem" role="alert">{problem}</p>
  {:else}
    <div class="pdf__viewer">
      <DocumentViewer {path} assetUrl={convertFileSrc(path)} type="pdf" readOnly {labels} />
    </div>
  {/if}
</div>

<style>
  /* Laid over the (empty) page area while the native webview is hidden. */
  .pdf {
    position: absolute;
    inset: 0;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    min-height: 0;
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .pdf__head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pdf__title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-primary);
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .pdf__note {
    margin: 0;
  }

  .pdf__problem {
    color: var(--color-danger, var(--color-text-primary));
  }

  .pdf__viewer {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
