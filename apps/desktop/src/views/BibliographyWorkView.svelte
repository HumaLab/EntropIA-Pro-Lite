<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { convertFileSrc } from '@tauri-apps/api/core'
  import { locale, t } from '$lib/i18n'
  import {
    bibliographyOpenWorkAttachment,
    bibliographyWorkDetail,
    type BibliographyWorkAttachmentRef,
    type BibliographyWorkCreator,
    type BibliographyWorkDetail,
    type BibliographyWorkOpen,
  } from '$lib/bibliography-library'
  import { workLine } from '$lib/rag-scope'
  import { renderOcrMarkup, sanitizeOcrHtml } from '$lib/ocr-rich-text'
  import { isPdfAttachment } from '$lib/bibliography-reprocess'
  import BibliographyReprocessDialog from '../components/BibliographyReprocessDialog.svelte'
  import BibliographyAnnotationsPanel from '../components/BibliographyAnnotationsPanel.svelte'
  import {
    createAnnotation,
    deleteAnnotation,
    listAnnotations,
    onAnnotationsChanged,
    resolveAnnotationConflict,
    restoreAnnotation,
    syncAnnotations,
    updateAnnotation,
    type AnnotationList,
    type AnnotationSyncReason,
    type AnnotationSyncResult,
    type AnnotationView,
    type ConflictDecision,
  } from '$lib/bibliography-annotations'
  import {
    draftToData,
    viewToMarkup,
    withField,
    withGeometry,
  } from '$lib/bibliography-annotation-markups'
  import {
    ActionIcon,
    Button,
    ConfirmDialog,
    DocumentViewer,
    ZOTERO_COLORS,
    type MarkupTool,
    type PdfMarkup,
    type PdfMarkupDraft,
    type PdfMarkupGeometryChange,
    IconButton,
    TabButton,
    TabList,
    ToolbarMenu,
    type ToolbarMenuItem,
  } from '@entropia/ui'

  /**
   * One Zotero work as a document-like page (P2): the header of the ficha,
   * then three sections in the ItemView tab language — "Original" (the
   * attachment's PDF in the in-app viewer, or the stored text of an HTML
   * snapshot), "Texto" (the extracted page texts, read-only) and "Metadatos"
   * (the catalog projection plus the attachments list). Everything here is
   * a read; the only privileged step is the backend's one-file grant behind
   * the Original tab.
   */

  interface Props {
    itemId: string
    itemKey: string
    libraryRowId: string
    title: string
  }

  let { itemId, itemKey: _itemKey, libraryRowId, title }: Props = $props()

  const currentLocale = locale

  type Tab = 'original' | 'text' | 'metadata'

  let detail = $state<BibliographyWorkDetail | null>(null)
  let detailFailed = $state(false)
  let opened = $state<BibliographyWorkOpen | null>(null)
  let openLoading = $state(false)
  let openFailed = $state(false)
  let activeTab = $state<Tab>('original')
  // Same condition as the PDF branch of the Original tab: only while that tab
  // is visible and showing a PDF does the view need the pane's full height.
  const showsPdf = $derived(
    activeTab === 'original' &&
      !openLoading &&
      !openFailed &&
      !opened?.openError &&
      opened?.originalKind === 'pdf' &&
      Boolean(opened.originalPath)
  )
  let activeKey = $state<string | null>(null)
  let attachmentMenuOpen = $state(false)
  /** The owner's text reprocess dialog over the work's PDF attachments. */
  let reprocessOpen = $state(false)
  /** The page shown and how many there are, as the viewer reports them. */
  let page = $state(1)
  let total = $state(1)

  const displayTitle = $derived(detail?.title || title)
  const metaLine = $derived(detail ? workLine({ authors: detail.authors, year: detail.year }) : '')
  const attachments = $derived(detail?.item.attachments ?? [])
  /** PDFs only, by the backend's own rule: the reprocess action names these. */
  const pdfAttachments = $derived(
    attachments.filter((entry) => isPdfAttachment(entry.contentType, entry.filename))
  )
  const currentAttachment = $derived(
    attachments.find((entry) => entry.attachmentKey === activeKey) ?? null
  )

  /** PDFs open first: that is what the Original tab is for. */
  function preferredAttachment(
    entries: BibliographyWorkAttachmentRef[]
  ): BibliographyWorkAttachmentRef | null {
    return (
      entries.find((entry) => entry.contentType?.toLowerCase().startsWith('application/pdf')) ??
      entries[0] ??
      null
    )
  }

  async function loadDetail(): Promise<void> {
    detailFailed = false
    try {
      detail = await bibliographyWorkDetail(itemId)
      const first = preferredAttachment(detail.item.attachments)
      if (first) await openAttachment(first.attachmentKey)
    } catch {
      detailFailed = true
    }
  }

  async function openAttachment(attachmentKey: string): Promise<void> {
    activeKey = attachmentKey
    attachmentMenuOpen = false
    page = 1
    total = 1
    openLoading = true
    openFailed = false
    try {
      opened = await bibliographyOpenWorkAttachment(itemId, attachmentKey)
    } catch {
      opened = null
      openFailed = true
    } finally {
      openLoading = false
    }
  }

  onMount(() => {
    void loadDetail()
  })

  // ── Zotero annotations (docs/zotero-annotations-plan.md) ────────────
  /** The Biblioteca's tools: never crop, erase or rotate Zotero's file. */
  const ANNOTATION_TOOLS: readonly MarkupTool[] = [
    'select',
    'highlight',
    'underline',
    'note',
    'image',
    'text',
  ]
  const EDIT_SYNC_DELAY_MS = 1500

  let annotationViews = $state<AnnotationView[]>([])
  let annotationStatus = $state<AnnotationList['status']>('ready')
  let syncResult = $state<AnnotationSyncResult | null>(null)
  let syncing = $state(false)
  let annotationError = $state<string | null>(null)
  let selectedMarkupId = $state<string | null>(null)
  let otherFormatCount = $state(0)
  let pendingDelete = $state<AnnotationView | null>(null)
  let editSyncTimer: ReturnType<typeof setTimeout> | null = null
  let unlistenAnnotations: (() => void) | null = null
  let destroyed = false

  /** The PDF attachment on screen, whose annotations the panel lists. */
  const annotatedKey = $derived(
    opened?.originalKind === 'pdf' && opened.originalPath ? activeKey : null
  )
  const markups = $derived(
    annotationViews.map(viewToMarkup).filter((markup): markup is PdfMarkup => markup !== null)
  )

  async function loadAnnotations(parentKey: string | null = annotatedKey): Promise<void> {
    if (!parentKey) {
      annotationViews = []
      return
    }
    try {
      const list = await listAnnotations(libraryRowId, parentKey)
      if (destroyed || parentKey !== annotatedKey) return
      annotationStatus = list.status
      annotationViews = list.annotations
    } catch (error) {
      annotationError = String(error)
    }
  }

  /** Counts annotations on the work's non-PDF attachments (shown, never edited). */
  async function countOtherFormats(): Promise<void> {
    const others = attachments.filter(
      (entry) => !isPdfAttachment(entry.contentType, entry.filename)
    )
    let count = 0
    for (const entry of others) {
      try {
        const list = await listAnnotations(libraryRowId, entry.attachmentKey)
        count += list.annotations.length
      } catch {
        // Best effort: the count is informational.
      }
    }
    if (!destroyed) otherFormatCount = count
  }

  async function runSync(reason: AnnotationSyncReason): Promise<void> {
    if (syncing) return
    syncing = true
    try {
      syncResult = await syncAnnotations(libraryRowId, reason)
    } catch (error) {
      annotationError = String(error)
    } finally {
      syncing = false
    }
    if (!destroyed) {
      await loadAnnotations()
      void countOtherFormats()
    }
  }

  function scheduleEditSync() {
    if (editSyncTimer) clearTimeout(editSyncTimer)
    editSyncTimer = setTimeout(() => {
      editSyncTimer = null
      void runSync('edit')
    }, EDIT_SYNC_DELAY_MS)
  }

  async function mutate(change: () => Promise<unknown>): Promise<void> {
    annotationError = null
    try {
      await change()
    } catch (error) {
      annotationError = t('bibliographyAnnotations.saveFailed', { error: String(error) })
    }
    await loadAnnotations()
    scheduleEditSync()
  }

  function viewByKey(key: string) {
    return annotationViews.find((view) => view.key === key) ?? null
  }

  function handleMarkupCreate(draft: PdfMarkupDraft) {
    const parentKey = annotatedKey
    if (!parentKey) return
    void mutate(async () => {
      const created = await createAnnotation(libraryRowId, parentKey, draftToData(draft))
      selectedMarkupId = created.key
    })
  }

  function handleMarkupChange(change: PdfMarkupGeometryChange) {
    const view = viewByKey(change.id)
    if (!view?.data) return
    const data = withGeometry(view.data, change)
    void mutate(() => updateAnnotation(libraryRowId, change.id, data))
  }

  function handleCommentChange(key: string, comment: string) {
    const view = viewByKey(key)
    if (!view?.data) return
    const data = withField(view.data, 'annotationComment', comment)
    void mutate(() => updateAnnotation(libraryRowId, key, data))
  }

  function requestDelete(view: AnnotationView) {
    if (view.syncState === 'conflict') {
      pendingDelete = view
      return
    }
    void mutate(async () => {
      try {
        await deleteAnnotation(libraryRowId, view.key)
      } catch (error) {
        if (String(error).includes('needs_confirmation')) {
          pendingDelete = view
          return
        }
        throw error
      }
      if (selectedMarkupId === view.key) selectedMarkupId = null
    })
  }

  function handleMarkupDelete(key: string) {
    const view = viewByKey(key)
    if (view) requestDelete(view)
  }

  function confirmDelete() {
    const view = pendingDelete
    pendingDelete = null
    if (!view) return
    void mutate(() => deleteAnnotation(libraryRowId, view.key, true))
  }

  function handleRestore(key: string) {
    void mutate(() => restoreAnnotation(libraryRowId, key))
  }

  function handleResolve(key: string, decisions: Record<string, ConflictDecision>) {
    void mutate(() => resolveAnnotationConflict(libraryRowId, key, decisions))
  }

  function handlePanelSelect(key: string, targetPage: number | null) {
    selectedMarkupId = key
    if (targetPage !== null) page = targetPage
  }

  const annotationStatusMessage = $derived.by(() => {
    $currentLocale
    if (annotationError) return annotationError
    if (annotationStatus === 'no_key') return t('bibliographyAnnotations.statusNoKey')
    if (annotationStatus === 'not_personal') return t('bibliographyAnnotations.statusNotPersonal')
    switch (syncResult?.status) {
      case 'no_key':
        return t('bibliographyAnnotations.statusNoKey')
      case 'not_personal':
        return t('bibliographyAnnotations.statusNotPersonal')
      case 'other_account':
        return t('bibliographyAnnotations.statusOtherAccount')
      case 'account_unknown':
        return t('bibliographyAnnotations.statusAccountUnknown')
      case 'invalid_key':
        return t('bibliographyAnnotations.statusInvalidKey')
      case 'no_write':
        return t('bibliographyAnnotations.statusNoWrite')
      case 'unreachable':
        return t('bibliographyAnnotations.statusUnreachable')
      default:
        return syncResult?.report?.paused ? t('bibliographyAnnotations.statusPaused') : null
    }
  })

  const markupLabels = $derived.by(() => {
    $currentLocale
    return {
      toolbarAriaLabel: t('bibliographyAnnotations.toolbar'),
      toolSelect: t('bibliographyAnnotations.toolSelect'),
      toolHighlight: t('bibliographyAnnotations.toolHighlight'),
      toolUnderline: t('bibliographyAnnotations.toolUnderline'),
      toolNote: t('bibliographyAnnotations.toolNote'),
      toolImage: t('bibliographyAnnotations.toolImage'),
      toolText: t('bibliographyAnnotations.toolText'),
      colorAriaLabel: (color: string) => t('bibliographyAnnotations.color', { color }),
      popoverAriaLabel: t('bibliographyAnnotations.popover'),
      popoverHighlight: t('bibliographyAnnotations.toolHighlight'),
      popoverUnderline: t('bibliographyAnnotations.toolUnderline'),
      deleteMarkup: t('bibliographyAnnotations.delete'),
    }
  })

  // A new PDF attachment on screen: list its annotations, then pull.
  let annotationsLoadedFor: string | null = null
  $effect(() => {
    const parentKey = annotatedKey
    if (parentKey === annotationsLoadedFor) return
    annotationsLoadedFor = parentKey
    selectedMarkupId = null
    if (!parentKey) {
      annotationViews = []
      return
    }
    void loadAnnotations(parentKey).then(() => runSync('open'))
  })

  onMount(() => {
    void onAnnotationsChanged((changedLibrary) => {
      if (changedLibrary === libraryRowId) void loadAnnotations()
    }).then((unlisten) => {
      if (destroyed) unlisten()
      else unlistenAnnotations = unlisten
    })
  })

  onDestroy(() => {
    destroyed = true
    unlistenAnnotations?.()
    if (editSyncTimer) {
      clearTimeout(editSyncTimer)
      void syncAnnotations(libraryRowId, 'edit').catch(() => {})
    }
  })

  const attachmentItems = $derived<ToolbarMenuItem[]>(
    attachments.map((entry) => ({
      kind: 'radio' as const,
      id: entry.attachmentKey,
      label: entry.filename ?? entry.attachmentKey,
      checked: entry.attachmentKey === activeKey,
      onselect: () => void openAttachment(entry.attachmentKey),
    }))
  )

  const viewerLabels = $derived.by(() => {
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

  /**
   * One page's stored text as safe rich HTML: new pages are Markdown (pipe
   * tables), legacy GLM-OCR pages raw HTML tables — the shared OCR renderer
   * turns both into tables and its sanitizer drops anything executable.
   * No region images: bibliography pages carry no region references. The
   * raw text is never rendered unsanitized.
   */
  function pageRichHtml(text: string): string {
    return sanitizeOcrHtml(renderOcrMarkup(text).html)
  }

  function creatorName(creator: BibliographyWorkCreator): string {
    if (creator.name?.trim()) return creator.name.trim()
    return [creator.firstName, creator.lastName]
      .map((part) => part?.trim() ?? '')
      .filter(Boolean)
      .join(' ')
  }

  const creatorLine = $derived(
    (detail?.item.creators ?? []).map(creatorName).filter(Boolean).join(', ')
  )
</script>

<div class="work-view" class:work-view--fill={showsPdf}>
  {#if detailFailed}
    <div class="surface-message surface-message--center empty">
      <p>{$currentLocale && t('bibliographyWork.error')}</p>
      <Button variant="secondary" size="sm" onclick={() => void loadDetail()}>
        {$currentLocale && t('bibliographyWork.retry')}
      </Button>
    </div>
  {:else if detail === null}
    <p class="surface-message surface-message--center">
      {$currentLocale && t('bibliographyWork.loading')}
    </p>
  {:else}
    <header class="work-header">
      <span class="work-header__eyebrow">{$currentLocale && t('biblioteca.eyebrow')}</span>
      <h2 class="work-header__title">{displayTitle}</h2>
      <div class="work-header__row">
        {#if metaLine}
          <p class="work-header__meta">{metaLine}</p>
        {/if}
        {#if detail.libraryName}
          <span class="work-header__chip">{detail.libraryName}</span>
        {/if}
        {#if pdfAttachments.length > 0}
          <Button variant="secondary" onclick={() => (reprocessOpen = true)}>
            <ActionIcon name="refresh" size={16} />
            {$currentLocale && t('bibliography.reprocess.action')}
          </Button>
        {/if}
      </div>
    </header>

    <TabList class="work-tabs" aria-label={$currentLocale && t('bibliographyWork.tabs')}>
      <TabButton active={activeTab === 'original'} onclick={() => (activeTab = 'original')}>
        {$currentLocale && t('bibliographyWork.tabOriginal')}
      </TabButton>
      <TabButton active={activeTab === 'text'} onclick={() => (activeTab = 'text')}>
        {$currentLocale && t('bibliographyWork.tabText')}
      </TabButton>
      <TabButton active={activeTab === 'metadata'} onclick={() => (activeTab = 'metadata')}>
        {$currentLocale && t('bibliographyWork.tabMetadata')}
      </TabButton>
    </TabList>

    <!-- The Original tab stays mounted (hidden) while the reading tabs show:
         unmounting it reopened the 128 MB document on every return. -->
    <section
      class="work-section work-section--original"
      class:is-hidden={activeTab !== 'original'}
      hidden={activeTab !== 'original'}
      aria-label={$currentLocale && t('bibliographyWork.tabOriginal')}
    >
      {#if attachments.length > 1}
        <ToolbarMenu
          label={$currentLocale && t('bibliographyWork.attachmentMenu')}
          items={attachmentItems}
          bind:open={attachmentMenuOpen}
        >
          {#snippet trigger(props, { open })}
            <button
              type="button"
              class="work-attachment-trigger"
              class:work-attachment-trigger--open={open}
              aria-label={$currentLocale && t('bibliographyWork.attachmentMenu')}
              {...props}
            >
              <span class="work-attachment-trigger__label">
                {currentAttachment?.filename ?? currentAttachment?.attachmentKey ?? ''}
              </span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
        </ToolbarMenu>
      {/if}

      {#if openLoading}
        <p class="surface-message surface-message--center">
          {$currentLocale && t('bibliographyWork.opening')}
        </p>
      {:else if openFailed || opened?.openError}
        <p class="surface-message surface-message--error" role="alert">
          {opened?.openError ?? t('bibliographyWork.error')}
        </p>
      {:else if opened?.originalKind === 'pdf' && opened.originalPath}
        <div class="work-original-layout">
          <div class="work-viewer" data-testid="work-original-viewer">
            <DocumentViewer
              path={opened.originalPath}
              assetUrl={convertFileSrc(opened.originalPath)}
              type="pdf"
              readOnly
              pauseWhenHidden
              labels={viewerLabels}
              currentPage={page}
              {markups}
              allowedTools={ANNOTATION_TOOLS}
              palette={ZOTERO_COLORS}
              textSelection
              {selectedMarkupId}
              {markupLabels}
              onMarkupCreate={handleMarkupCreate}
              onMarkupChange={handleMarkupChange}
              onMarkupDelete={handleMarkupDelete}
              onMarkupSelect={(id) => (selectedMarkupId = id)}
              onPageChange={(next, count) => {
                total = count
                page = next
              }}
            />
          </div>
          <BibliographyAnnotationsPanel
            annotations={annotationViews}
            selectedKey={selectedMarkupId}
            statusMessage={annotationStatusMessage}
            {syncing}
            canSync={annotationStatus === 'ready'}
            {otherFormatCount}
            onSelect={handlePanelSelect}
            onCommentChange={handleCommentChange}
            onDelete={requestDelete}
            onRestore={handleRestore}
            onResolve={handleResolve}
            onSync={() => void runSync('edit')}
          />
        </div>
        {#if total > 1}
          <div class="work-viewer-pager">
            <IconButton
              size="sm"
              variant="ghost"
              label={$currentLocale && t('item.previousPage')}
              title={$currentLocale && t('item.previousPage')}
              disabled={page <= 1}
              onclick={() => (page = Math.max(1, page - 1))}
            >
              <ActionIcon name="chevron-left" size={14} />
            </IconButton>
            <span class="work-viewer-pager__count">
              {$currentLocale && t('navegador.pdf.page', { page, total })}
            </span>
            <IconButton
              size="sm"
              variant="ghost"
              label={$currentLocale && t('item.nextPage')}
              title={$currentLocale && t('item.nextPage')}
              disabled={page >= total}
              onclick={() => (page = Math.min(total, page + 1))}
            >
              <ActionIcon name="chevron-right" size={14} />
            </IconButton>
          </div>
        {/if}
        <p class="work-note">{$currentLocale && t('bibliographyWork.originalNote')}</p>
      {:else if opened?.originalKind === 'html'}
        <p class="work-note">{$currentLocale && t('bibliographyWork.snapshotNote')}</p>
        <p class="work-snapshot">{opened.snapshotText}</p>
      {:else}
        <p class="surface-message surface-message--center">
          {$currentLocale && t('bibliographyWork.originalEmpty')}
        </p>
      {/if}
    </section>

    {#if activeTab === 'text'}
      <section class="work-section" aria-label={$currentLocale && t('bibliographyWork.tabText')}>
        <h3 class="work-section__title">{$currentLocale && t('bibliographyWork.textTitle')}</h3>
        {#if openLoading}
          <p class="surface-message surface-message--center">
            {$currentLocale && t('bibliographyWork.opening')}
          </p>
        {:else if opened?.extracted && opened.pages.length > 0}
          {#each opened.pages as page (page.pageNumber)}
            <h4 class="work-page__title">
              {$currentLocale && t('bibliographyWork.textPage', { page: page.pageNumber })}
            </h4>
            <div class="work-page__rich">
              <!-- eslint-disable-next-line svelte/no-at-html-tags -- renderOcrMarkup output passes sanitizeOcrHtml -->
              {@html pageRichHtml(page.text)}
            </div>
          {/each}
        {:else if opened?.extracted && opened.snapshotText}
          <p class="work-page__text">{opened.snapshotText}</p>
        {:else}
          <p class="surface-message surface-message--center">
            {$currentLocale && t('bibliographyWork.textEmpty')}
          </p>
        {/if}
      </section>
    {:else if activeTab === 'metadata'}
      <section
        class="work-section"
        aria-label={$currentLocale && t('bibliographyWork.tabMetadata')}
      >
        <h3 class="work-section__title">
          {$currentLocale && t('bibliographyWork.tabMetadata')}
        </h3>
        <dl class="work-meta">
          {#if detail.item.itemType}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailType')}</dt>
              <dd>{detail.item.itemType}</dd>
            </div>
          {/if}
          {#if creatorLine}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailCreators')}</dt>
              <dd>{creatorLine}</dd>
            </div>
          {/if}
          {#if detail.item.publicationTitle}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailPublication')}</dt>
              <dd>{detail.item.publicationTitle}</dd>
            </div>
          {/if}
          {#if detail.item.publisher}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailPublisher')}</dt>
              <dd>{detail.item.publisher}</dd>
            </div>
          {/if}
          {#if detail.item.date}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailDate')}</dt>
              <dd>{detail.item.date}</dd>
            </div>
          {/if}
          {#if detail.item.doi}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailDoi')}</dt>
              <dd>{detail.item.doi}</dd>
            </div>
          {/if}
          {#if detail.item.isbn}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailIsbn')}</dt>
              <dd>{detail.item.isbn}</dd>
            </div>
          {/if}
          {#if detail.item.abstract}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailAbstract')}</dt>
              <dd>{detail.item.abstract}</dd>
            </div>
          {/if}
          {#if detail.item.language}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailLanguage')}</dt>
              <dd>{detail.item.language}</dd>
            </div>
          {/if}
          {#if detail.item.url}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailUrl')}</dt>
              <dd>{detail.item.url}</dd>
            </div>
          {/if}
          {#if detail.item.collections.length > 0}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailCollections')}</dt>
              <dd>{detail.item.collections.join(', ')}</dd>
            </div>
          {/if}
          {#if detail.item.tags.length > 0}
            <div class="work-meta__row">
              <dt>{$currentLocale && t('writing.zoteroDetailTags')}</dt>
              <dd>{detail.item.tags.join(', ')}</dd>
            </div>
          {/if}
        </dl>

        <h3 class="work-section__title">
          {$currentLocale && t('bibliographyWork.attachmentsTitle')}
        </h3>
        {#if attachments.length === 0}
          <p class="surface-message">{$currentLocale && t('bibliographyWork.attachmentsEmpty')}</p>
        {:else}
          <ul class="work-attachments">
            {#each attachments as entry (entry.attachmentKey)}
              <li class="work-attachments__row">
                <span class="work-attachments__name">{entry.filename ?? entry.attachmentKey}</span>
                {#if entry.contentType}
                  <span class="work-attachments__type">{entry.contentType}</span>
                {/if}
                <button
                  type="button"
                  class="work-attachments__open"
                  aria-label={$currentLocale && t('bibliographyWork.attachmentOpen')}
                  onclick={() => {
                    activeTab = 'original'
                    void openAttachment(entry.attachmentKey)
                  }}
                >
                  <ActionIcon name="eye" size={14} />
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}
  {/if}

  {#if pendingDelete}
    <ConfirmDialog
      title={$currentLocale && t('bibliographyAnnotations.deleteConfirmTitle')}
      titleId="bibliography-annotation-delete-title"
      cancelLabel={$currentLocale && t('bibliographyAnnotations.cancel')}
      confirmLabel={$currentLocale && t('bibliographyAnnotations.confirmDelete')}
      oncancel={() => (pendingDelete = null)}
      onconfirm={confirmDelete}
    >
      <p>
        {$currentLocale &&
          t('bibliographyAnnotations.deleteConflictConfirm', {
            fields: pendingDelete.conflictFields.join(', '),
          })}
      </p>
    </ConfirmDialog>
  {/if}

  {#if reprocessOpen}
    <BibliographyReprocessDialog
      mode="work"
      attachmentIds={pdfAttachments.map((entry) => entry.attachmentId)}
      onclose={() => (reprocessOpen = false)}
    />
  {/if}
</div>

<style>
  .work-view {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 1100px;
  }

  /* While a PDF is shown, the view takes the WorkPane body's full height,
     like ItemView's `height: 100%`. A min-height is not a definite height:
     the viewer below measured ~0, so the page stayed hidden behind a
     clipped toolbar. The reading tabs keep their natural flow and the pane
     scrolls them. */
  .work-view--fill {
    height: 100%;
    min-height: 0;
  }

  .work-header {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .work-header__eyebrow {
    color: var(--color-accent);
    font-family: var(--font-mono);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .work-header__title {
    margin: 0;
    font-size: var(--font-size-xl);
    line-height: 1.2;
  }

  .work-header__row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .work-header__meta {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .work-header__chip {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    white-space: nowrap;
  }

  .work-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  /* The Original tab owns the remaining height of the fill-height view:
     `flex: 1; min-height: 0` lets the viewer chain below shrink with the
     pane instead of pushing it to scroll. */
  .work-section--original {
    flex: 1;
    min-height: 0;
  }

  /* The hidden Original tab must beat `.work-section`'s display: flex (and the
     UA rule behind the `hidden` attribute) or it would stay laid out. */
  .work-section--original.is-hidden {
    display: none;
  }

  .work-section__title {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .work-attachment-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    align-self: flex-start;
    max-width: 320px;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: var(--color-surface-glass);
    color: var(--color-text-primary);
    cursor: pointer;
  }

  .work-attachment-trigger:hover,
  .work-attachment-trigger--open {
    border-color: color-mix(in srgb, var(--color-accent) 40%, var(--color-hairline));
  }

  .work-attachment-trigger:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .work-attachment-trigger__label {
    overflow: hidden;
    font-size: var(--font-size-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The viewer and the "Anotaciones" panel side by side; the panel scrolls on
     its own and the viewer keeps the remaining width. */
  .work-original-layout {
    display: grid;
    flex: 1;
    grid-template-columns: minmax(0, 1fr) minmax(240px, 320px);
    gap: var(--space-3);
    min-height: 0;
  }

  .work-viewer {
    /* The DocumentViewer is a flex child of this frame (the exact pattern of
       ItemAssetPanel's .left-panel-pane--document): its fit scale reads the
       container rect, which a min-height-only frame never made definite.
       The view's definite height (.work-view--fill) flows down to it; a
       min-height floor here would push the pager and note out of the pane. */
    display: flex;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--surface-app);
  }

  .work-viewer :global(.document-viewer) {
    flex: 1;
    min-height: 0;
  }

  .work-viewer-pager {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
  }

  .work-viewer-pager__count {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .work-note {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .work-snapshot {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .work-page__title {
    margin: var(--space-2) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  /* The same reading surface the Colecciones OCR text has (OcrRichText's
     .ocr-rich-text): pages render through the shared safe renderer, so a
     legacy GLM-OCR HTML table and a new Markdown pipe table show the same
     table. {@html} content is unscoped, hence :global. */
  .work-page__rich {
    min-width: 0;
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.6;
    overflow-wrap: anywhere;
  }

  .work-page__rich :global(p),
  .work-page__rich :global(ul),
  .work-page__rich :global(ol),
  .work-page__rich :global(blockquote),
  .work-page__rich :global(table),
  .work-page__rich :global(pre) {
    margin: 0 0 var(--space-3);
  }

  .work-page__rich :global(h1),
  .work-page__rich :global(h2),
  .work-page__rich :global(h3),
  .work-page__rich :global(h4),
  .work-page__rich :global(h5),
  .work-page__rich :global(h6) {
    margin: var(--space-4) 0 var(--space-2);
    color: var(--color-text-primary);
    line-height: 1.25;
  }

  .work-page__rich :global(ul),
  .work-page__rich :global(ol) {
    padding-inline-start: var(--space-6);
  }

  .work-page__rich :global(blockquote) {
    padding-inline-start: var(--space-3);
    border-inline-start: 2px solid var(--border-subtle);
    color: var(--color-text-secondary);
  }

  .work-page__rich :global(table) {
    width: 100%;
    border-collapse: collapse;
    font-size: inherit;
  }

  .work-page__rich :global(th),
  .work-page__rich :global(td) {
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    text-align: start;
    vertical-align: top;
  }

  .work-page__rich :global(code),
  .work-page__rich :global(pre) {
    font-family: var(--font-mono);
  }

  .work-page__rich :global(a) {
    color: var(--color-accent);
  }

  .work-page__text {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  /* The ItemMetadataPanel label/value row, read-only. */
  .work-meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
  }

  .work-meta__row {
    display: grid;
    grid-template-columns: minmax(0, 0.45fr) minmax(0, 0.55fr);
    gap: var(--space-3);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--color-border-subtle);
  }

  .work-meta__row:last-child {
    border-bottom: none;
  }

  .work-meta dt {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .work-meta dd {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    overflow-wrap: anywhere;
  }

  .work-attachments {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* The same row as the Zotero tabs: text on the left, one action right. */
  .work-attachments__row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-control);
  }

  .work-attachments__row:hover {
    background: var(--color-accent-faint);
  }

  .work-attachments__name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    font-size: var(--font-size-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .work-attachments__type {
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    white-space: nowrap;
  }

  .work-attachments__open {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: var(--control-height-md);
    min-height: var(--control-height-md);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .work-attachments__open:hover {
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    color: var(--color-text-primary);
  }

  .work-attachments__open:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
</style>
