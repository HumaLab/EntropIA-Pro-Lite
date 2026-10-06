<script lang="ts">
  import { onMount } from 'svelte'
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
  import {
    ActionIcon,
    Button,
    DocumentViewer,
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

  let { itemId, itemKey: _itemKey, libraryRowId: _libraryRowId, title }: Props = $props()

  const currentLocale = locale

  type Tab = 'original' | 'text' | 'metadata'

  let detail = $state<BibliographyWorkDetail | null>(null)
  let detailFailed = $state(false)
  let opened = $state<BibliographyWorkOpen | null>(null)
  let openLoading = $state(false)
  let openFailed = $state(false)
  let activeTab = $state<Tab>('original')
  let activeKey = $state<string | null>(null)
  let attachmentMenuOpen = $state(false)

  const displayTitle = $derived(detail?.title || title)
  const metaLine = $derived(detail ? workLine({ authors: detail.authors, year: detail.year }) : '')
  const attachments = $derived(detail?.item.attachments ?? [])
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

<div class="work-view">
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

    {#if activeTab === 'original'}
      <section
        class="work-section"
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
          <div class="work-viewer" data-testid="work-original-viewer">
            <DocumentViewer
              path={opened.originalPath}
              assetUrl={convertFileSrc(opened.originalPath)}
              type="pdf"
              readOnly
              labels={viewerLabels}
            />
          </div>
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
    {:else if activeTab === 'text'}
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
            <p class="work-page__text">{page.text}</p>
          {/each}
        {:else if opened?.extracted && opened.snapshotText}
          <p class="work-page__text">{opened.snapshotText}</p>
        {:else}
          <p class="surface-message surface-message--center">
            {$currentLocale && t('bibliographyWork.textEmpty')}
          </p>
        {/if}
      </section>
    {:else}
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
</div>

<style>
  .work-view {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 1100px;
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

  .work-viewer {
    min-height: 480px;
    overflow: hidden;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--surface-app);
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
