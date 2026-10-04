<script lang="ts">
  import { onMount } from 'svelte'
  import { convertFileSrc } from '@tauri-apps/api/core'
  import { ActionIcon, DocumentViewer, IconButton, portal } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import type { BibliographyPassageContext } from '$lib/bibliography-search'
  import { citedPageNumber, passageMarks, passageWindow } from '$lib/rag-scope'

  /**
   * The cited original, inside the app: a PDF at the cited page (the corpus's
   * own viewer) with the page text beside it and the passage marked there, or
   * an HTML snapshot as its stored text with the passage marked and scrolled
   * into view. The PDF's pixels carry no word boxes, so the mark lives in the
   * text panel, never as a guessed rectangle on the page.
   *
   * The file is read through the asset protocol after the backend granted that
   * one path: this component never builds or accepts a path of its own.
   */
  interface Props {
    context: BibliographyPassageContext
    title: string
    heading: string
    onclose: () => void
  }

  let { context, title, heading, onclose }: Props = $props()

  const currentLocale = locale
  /** A snapshot can be megabytes: the window around the passage is bounded. */
  const SNAPSHOT_RADIUS = 30_000
  const PAGE_RADIUS = 8_000

  const isPdf = $derived(context.originalKind === 'pdf' && !!context.originalPath)
  const marks = $derived(passageMarks(context.pages, context.text))
  const firstPage = $derived(citedPageNumber(context.pages))
  let page = $derived(firstPage)
  let panel: HTMLElement | undefined = $state()
  let dialog: HTMLElement | undefined = $state()

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

  function onKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return
    event.preventDefault()
    event.stopPropagation()
    onclose()
  }

  onMount(() => {
    dialog?.focus()
    // The first marked passage into view: a snapshot's cited paragraph can be
    // far down a long text.
    panel?.querySelector('mark')?.scrollIntoView?.({ block: 'center' })
  })
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="passage-original__overlay" {@attach portal} role="presentation">
  <div
    bind:this={dialog}
    class="passage-original"
    role="dialog"
    aria-modal="true"
    aria-labelledby="passage-original-title"
    tabindex="-1"
  >
    <header class="passage-original__header">
      <div class="passage-original__heading">
        <span class="passage-original__eyebrow">
          {$currentLocale && t('ragChat.passageOriginalEyebrow')}
        </span>
        <h2 id="passage-original-title">{title}</h2>
        <p>{heading}</p>
      </div>
      <IconButton
        size="sm"
        variant="ghost"
        label={$currentLocale && t('ragChat.passageOriginalClose')}
        title={$currentLocale && t('ragChat.passageOriginalClose')}
        onclick={onclose}
      >
        <ActionIcon name="close" size={20} />
      </IconButton>
    </header>

    <div class="passage-original__body" class:passage-original__body--single={!isPdf}>
      {#if isPdf && context.originalPath}
        <div class="passage-original__viewer" data-testid="passage-original-viewer">
          <DocumentViewer
            path={context.originalPath}
            assetUrl={convertFileSrc(context.originalPath)}
            type="pdf"
            readOnly
            {labels}
            currentPage={page}
            onPageChange={(next) => {
              page = next
            }}
          />
        </div>
      {/if}
      <aside class="passage-original__text" bind:this={panel}>
        <p class="passage-original__note">
          {$currentLocale &&
            t(isPdf ? 'ragChat.passageOriginalNote' : 'ragChat.passageOriginalSnapshotNote')}
        </p>
        {#each context.pages as sourcePage, pageIndex (sourcePage.pageNumber)}
          {@const view = passageWindow(
            sourcePage.text,
            marks[pageIndex] ?? [],
            isPdf ? PAGE_RADIUS : SNAPSHOT_RADIUS
          )}
          <h3>
            {$currentLocale &&
              (isPdf
                ? t('ragChat.passageOriginalText', { page: sourcePage.pageNumber })
                : t('ragChat.passageOriginalSnapshot'))}
          </h3>
          <p class="passage-original__page">
            {#if view.truncatedBefore}<span>… </span>{/if}
            {#each view.segments as segment, segmentIndex (segmentIndex)}
              {#if segment.marked}
                <mark class="passage-original__hit">{segment.text}</mark>
              {:else}
                <span>{segment.text}</span>
              {/if}
            {/each}
            {#if view.truncatedAfter}<span> …</span>{/if}
          </p>
        {/each}
      </aside>
    </div>
  </div>
</div>

<style>
  .passage-original__overlay {
    position: fixed;
    inset: 0;
    z-index: 1100;
    display: grid;
    place-items: center;
    padding: var(--space-4);
    background: color-mix(in srgb, var(--color-overlay) 88%, transparent);
    backdrop-filter: blur(8px);
  }

  .passage-original {
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

  .passage-original:focus-visible {
    outline: none;
    box-shadow: var(--shadow-lg), var(--focus-ring);
  }

  .passage-original__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--color-hairline);
    background: color-mix(in srgb, var(--surface-panel) 88%, transparent);
  }

  .passage-original__heading {
    min-width: 0;
  }

  .passage-original__eyebrow {
    display: block;
    margin-bottom: var(--space-1);
    color: var(--color-accent);
    font-family: var(--font-mono);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .passage-original__heading h2,
  .passage-original__heading p,
  .passage-original__text h3 {
    margin: 0;
  }

  .passage-original__heading h2 {
    overflow: hidden;
    font-size: var(--font-size-lg);
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .passage-original__heading p {
    margin-top: var(--space-1);
    overflow: hidden;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .passage-original__body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(260px, 34%);
    flex: 1;
    min-height: 0;
  }

  .passage-original__body--single {
    grid-template-columns: minmax(0, 1fr);
  }

  .passage-original__viewer {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border-right: 1px solid var(--color-hairline);
    background: var(--surface-app);
  }

  .passage-original__text {
    min-width: 0;
    overflow-y: auto;
    padding: var(--space-4);
    background: var(--surface-card);
    font-size: var(--font-size-sm);
  }

  .passage-original__text h3 {
    margin: var(--space-3) 0 var(--space-2);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .passage-original__note {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .passage-original__page {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--color-text-secondary);
  }

  /* The cited range, marked the way the reader and the item view mark it. */
  .passage-original__hit {
    border-radius: var(--radius-xs);
    background: var(--color-warning-soft);
    box-shadow: inset 0 -2px 0 var(--color-warning);
    color: inherit;
  }

  @media (max-width: 760px) {
    .passage-original__overlay {
      padding: var(--space-2);
    }

    .passage-original__body {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 2fr) minmax(140px, 1fr);
    }

    .passage-original__viewer {
      border-right: 0;
      border-bottom: 1px solid var(--color-hairline);
    }
  }
</style>
