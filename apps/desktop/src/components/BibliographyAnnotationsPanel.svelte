<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte'
  import { ActionIcon, Button, IconButton } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import {
    EXISTENCE_FIELD,
    existenceReason,
    isInZoteroTrash,
    type AnnotationView,
    type ConflictDecision,
    type ZoteroAnnotationType,
  } from '$lib/bibliography-annotations'
  import type { ThumbnailRequest } from '@entropia/ui'
  import { compareViews, thumbnailRequest, viewPage } from '$lib/bibliography-annotation-markups'

  /**
   * The "Anotaciones" panel of the Biblioteca work view
   * (docs/zotero-annotations-plan.md, section 10): the annotations of the
   * attachment on screen grouped by page, each with its comment, its sync
   * state and, in conflict, one decision per disputed entry. Annotations in
   * Zotero's trash sit in a collapsed read-only group with Restore. Every
   * change goes through the callbacks; the parent owns the commands.
   */

  interface Props {
    annotations: AnnotationView[]
    selectedKey?: string | null
    /** A sentence about the sync (missing key, offline…), or null. */
    statusMessage?: string | null
    syncing?: boolean
    canSync?: boolean
    otherFormatCount?: number
    /** False when the PDF is not on this device: comments stay editable. */
    pdfAvailable?: boolean
    /** Renders the preview of an area or ink annotation (a data URL). */
    renderThumbnail?: ((request: ThumbnailRequest) => Promise<string | null>) | null
    onSelect?: (key: string, page: number | null) => void
    onCommentChange?: (key: string, comment: string) => void
    onDelete?: (view: AnnotationView) => void
    onRestore?: (key: string) => void
    onResolve?: (key: string, decisions: Record<string, ConflictDecision>) => void
    onSync?: () => void
  }

  let {
    annotations,
    selectedKey = null,
    statusMessage = null,
    syncing = false,
    canSync = true,
    otherFormatCount = 0,
    pdfAvailable = true,
    renderThumbnail = null,
    onSelect = () => {},
    onCommentChange = () => {},
    onDelete = () => {},
    onRestore = () => {},
    onResolve = () => {},
    onSync = () => {},
  }: Props = $props()

  const currentLocale = locale
  const COMMENT_DEBOUNCE_MS = 600

  const live = $derived(annotations.filter((view) => !isInZoteroTrash(view)).sort(compareViews))
  const trashed = $derived(annotations.filter(isInZoteroTrash).sort(compareViews))

  const groups = $derived.by(() => {
    const byPage = new Map<number | null, AnnotationView[]>()
    for (const view of live) {
      const page = viewPage(view)
      const entries = byPage.get(page) ?? []
      entries.push(view)
      byPage.set(page, entries)
    }
    return [...byPage.entries()].map(([page, entries]) => ({ page, entries }))
  })

  let trashOpen = $state(false)
  let trashSection: HTMLElement | undefined = $state()

  /** Opening the trash group brings it into view: it sits at the panel's end. */
  async function toggleTrash() {
    trashOpen = !trashOpen
    if (!trashOpen) return
    await tick()
    trashSection?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' })
  }

  // ── Area and ink previews, one per annotation revision ─────────────
  const thumbnails = $state<Record<string, string>>({})

  $effect(() => {
    const render = renderThumbnail
    if (!render || !pdfAvailable) return
    for (const view of live) {
      const request = thumbnailRequest(view)
      if (!request || untrack(() => request.id in thumbnails)) continue
      void render(request).then((url) => {
        if (url) thumbnails[request.id] = url
      })
    }
  })

  function thumbnailOf(view: AnnotationView): string | null {
    const request = thumbnailRequest(view)
    return request ? (thumbnails[request.id] ?? null) : null
  }

  // ── Comments: one debounced write per annotation ───────────────────
  const drafts = $state<Record<string, string>>({})
  const timers = new Map<string, ReturnType<typeof setTimeout>>()

  function commentOf(view: AnnotationView): string {
    if (view.key in drafts) return drafts[view.key] ?? ''
    const comment = view.data?.annotationComment
    return typeof comment === 'string' ? comment : ''
  }

  function flush(key: string) {
    const timer = timers.get(key)
    if (timer) clearTimeout(timer)
    timers.delete(key)
    if (!(key in drafts)) return
    const comment = drafts[key] ?? ''
    delete drafts[key]
    onCommentChange(key, comment)
  }

  function editComment(key: string, comment: string) {
    drafts[key] = comment
    const timer = timers.get(key)
    if (timer) clearTimeout(timer)
    timers.set(
      key,
      setTimeout(() => flush(key), COMMENT_DEBOUNCE_MS)
    )
  }

  onDestroy(() => {
    for (const key of [...timers.keys()]) flush(key)
  })

  // ── Conflict decisions ─────────────────────────────────────────────
  const decisions = $state<Record<string, Record<string, ConflictDecision>>>({})

  function decide(key: string, field: string, decision: ConflictDecision) {
    decisions[key] = { ...(decisions[key] ?? {}), [field]: decision }
  }

  function complete(view: AnnotationView) {
    const chosen = decisions[view.key] ?? {}
    return view.conflictFields.every((field) => field in chosen)
  }

  function apply(view: AnnotationView) {
    const chosen = decisions[view.key]
    if (!chosen || !complete(view)) return
    delete decisions[view.key]
    onResolve(view.key, chosen)
  }

  function existenceText(view: AnnotationView): string {
    switch (existenceReason(view)) {
      case 'trashed':
        return t('bibliographyAnnotations.existenceTrashed')
      case 'modified':
        return t('bibliographyAnnotations.existenceModified')
      case 'deleted':
        return t('bibliographyAnnotations.existenceDeleted')
      case 'attachment_deleted':
        return t('bibliographyAnnotations.existenceAttachmentDeleted')
      default:
        return ''
    }
  }

  function fieldLabel(field: string): string {
    switch (field) {
      case 'annotationComment':
        return t('bibliographyAnnotations.fieldComment')
      case 'annotationColor':
        return t('bibliographyAnnotations.fieldColor')
      case 'annotationText':
        return t('bibliographyAnnotations.fieldText')
      case 'annotationPosition':
        return t('bibliographyAnnotations.fieldPosition')
      default:
        return t('bibliographyAnnotations.fieldOther', { field })
    }
  }

  function display(value: unknown): string {
    if (value == null || value === '') return '—'
    return typeof value === 'string' ? value : JSON.stringify(value)
  }

  function kindLabel(type: ZoteroAnnotationType | undefined): string {
    switch (type) {
      case 'highlight':
        return t('bibliographyAnnotations.kindHighlight')
      case 'underline':
        return t('bibliographyAnnotations.kindUnderline')
      case 'note':
        return t('bibliographyAnnotations.kindNote')
      case 'image':
        return t('bibliographyAnnotations.kindImage')
      case 'text':
        return t('bibliographyAnnotations.kindText')
      case 'ink':
        return t('bibliographyAnnotations.kindInk')
      default:
        return ''
    }
  }

  function stateLabel(view: AnnotationView): string | null {
    switch (view.syncState) {
      case 'pending':
        return t('bibliographyAnnotations.statePending')
      case 'conflict':
        return t('bibliographyAnnotations.stateConflict')
      case 'error':
        return t('bibliographyAnnotations.stateError')
      case 'waiting_parent':
        return t('bibliographyAnnotations.stateWaitingParent')
      default:
        return null
    }
  }

  function since(view: AnnotationView): string {
    return new Date(view.updatedAt).toLocaleString($currentLocale)
  }
</script>

<aside class="annotations-panel" aria-label={$currentLocale && t('bibliographyAnnotations.title')}>
  <header class="annotations-panel__header">
    <h3 class="annotations-panel__title">{$currentLocale && t('bibliographyAnnotations.title')}</h3>
    <IconButton
      size="sm"
      variant="ghost"
      label={$currentLocale && (syncing ? t('bibliographyAnnotations.syncing') : t('bibliographyAnnotations.syncNow'))}
      title={$currentLocale && t('bibliographyAnnotations.syncNow')}
      disabled={syncing || !canSync}
      onclick={() => onSync()}
    >
      <ActionIcon name={syncing ? 'loader' : 'refresh'} size={14} />
    </IconButton>
  </header>

  {#if !pdfAvailable}
    <p class="annotations-panel__status" data-testid="annotations-pdf-unavailable">
      {$currentLocale && t('bibliographyAnnotations.pdfUnavailable')}
    </p>
  {/if}

  {#if statusMessage}
    <p class="annotations-panel__status" role="status">{statusMessage}</p>
  {/if}

  {#if live.length === 0}
    <p class="annotations-panel__empty">{$currentLocale && t('bibliographyAnnotations.empty')}</p>
  {/if}

  {#each groups as group (group.page)}
    <section class="annotations-panel__group">
      {#if group.page !== null}
        <h4 class="annotations-panel__page">
          {$currentLocale && t('bibliographyAnnotations.page', { page: group.page })}
        </h4>
      {/if}
      <ul class="annotations-panel__list">
        {#each group.entries as view (view.key)}
          {@const state = stateLabel(view)}
          <li
            class="annotation-entry"
            class:annotation-entry--selected={view.key === selectedKey}
            data-testid="annotation-entry"
            data-annotation-key={view.key}
          >
            <span
              class="annotation-entry__bar"
              style={`background:${typeof view.data?.annotationColor === 'string' ? view.data.annotationColor : 'var(--color-accent)'};`}
              aria-hidden="true"
            ></span>
            <div class="annotation-entry__body">
              <div class="annotation-entry__head">
                <button
                  type="button"
                  class="annotation-entry__jump"
                  onclick={() => onSelect(view.key, viewPage(view))}
                >
                  <span class="annotation-entry__kind">{$currentLocale && kindLabel(view.data?.annotationType)}</span>
                  {#if typeof view.data?.annotationText === 'string' && view.data.annotationText}
                    <q class="annotation-entry__quote">{view.data.annotationText}</q>
                  {/if}
                </button>
                {#if state}
                  <span
                    class="annotation-entry__badge annotation-entry__badge--{view.syncState}"
                    data-testid="annotation-sync-badge"
                  >{state}</span>
                {/if}
                <IconButton
                  size="sm"
                  variant="ghost"
                  label={$currentLocale && t('bibliographyAnnotations.delete')}
                  title={$currentLocale && t('bibliographyAnnotations.delete')}
                  onclick={() => onDelete(view)}
                >
                  <ActionIcon name="delete" size={14} />
                </IconButton>
              </div>

              {#if thumbnailOf(view)}
                <img
                  class="annotation-entry__preview"
                  data-testid="annotation-preview"
                  src={thumbnailOf(view)}
                  alt={$currentLocale && kindLabel(view.data?.annotationType)}
                />
              {/if}

              <textarea
                class="annotation-entry__comment"
                rows="1"
                aria-label={$currentLocale && t('bibliographyAnnotations.commentLabel')}
                placeholder={$currentLocale && t('bibliographyAnnotations.commentPlaceholder')}
                value={commentOf(view)}
                oninput={(event) => editComment(view.key, (event.currentTarget as HTMLTextAreaElement).value)}
                onblur={() => flush(view.key)}
              ></textarea>

              {#if Array.isArray(view.data?.tags) && view.data.tags.length > 0}
                <p class="annotation-entry__tags">
                  {view.data.tags.map((tag) => tag.tag).join(', ')}
                </p>
              {/if}

              {#if view.syncState === 'waiting_parent'}
                <p class="annotation-entry__note">
                  {$currentLocale && t('bibliographyAnnotations.waitingSince', { when: since(view) })}
                </p>
              {:else if view.syncState === 'error' && view.lastError}
                <p class="annotation-entry__note annotation-entry__note--error">{view.lastError}</p>
              {/if}

              {#if view.syncState === 'conflict'}
                <div class="annotation-conflict" data-testid="annotation-conflict">
                  <p class="annotation-conflict__title">{$currentLocale && t('bibliographyAnnotations.conflictTitle')}</p>
                  {#each view.conflictFields as field (field)}
                    {@const chosen = decisions[view.key]?.[field]}
                    {#if field === EXISTENCE_FIELD}
                      <p class="annotation-conflict__reason">{$currentLocale && existenceText(view)}</p>
                      <div class="annotation-conflict__choices" role="radiogroup" aria-label={existenceText(view)}>
                        {#if existenceReason(view) !== 'attachment_deleted'}
                          <Button
                            size="sm"
                            variant={chosen === 'keep' ? 'primary' : 'secondary'}
                            aria-pressed={chosen === 'keep'}
                            onclick={() => decide(view.key, field, 'keep')}
                          >{$currentLocale && t('bibliographyAnnotations.conflictKeep')}</Button>
                        {/if}
                        <Button
                          size="sm"
                          variant={chosen === 'delete' ? 'primary' : 'secondary'}
                          aria-pressed={chosen === 'delete'}
                          onclick={() => decide(view.key, field, 'delete')}
                        >{$currentLocale && t('bibliographyAnnotations.conflictDiscard')}</Button>
                      </div>
                    {:else}
                      <p class="annotation-conflict__field">{$currentLocale && fieldLabel(field)}</p>
                      <div class="annotation-conflict__choices" role="radiogroup" aria-label={fieldLabel(field)}>
                        <Button
                          size="sm"
                          variant={chosen === 'local' ? 'primary' : 'secondary'}
                          aria-pressed={chosen === 'local'}
                          title={t('bibliographyAnnotations.conflictLocalValue', { value: display(view.data?.[field]) })}
                          onclick={() => decide(view.key, field, 'local')}
                        >{$currentLocale && t('bibliographyAnnotations.conflictLocal')}</Button>
                        <Button
                          size="sm"
                          variant={chosen === 'zotero' ? 'primary' : 'secondary'}
                          aria-pressed={chosen === 'zotero'}
                          title={t('bibliographyAnnotations.conflictZoteroValue', { value: display(view.conflictRemote?.[field]) })}
                          onclick={() => decide(view.key, field, 'zotero')}
                        >{$currentLocale && t('bibliographyAnnotations.conflictZotero')}</Button>
                      </div>
                      <p class="annotation-conflict__values">
                        {$currentLocale && t('bibliographyAnnotations.conflictLocalValue', { value: display(view.data?.[field]) })}
                        <br />
                        {$currentLocale && t('bibliographyAnnotations.conflictZoteroValue', { value: display(view.conflictRemote?.[field]) })}
                      </p>
                    {/if}
                  {/each}
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={!complete(view)}
                    onclick={() => apply(view)}
                  >{$currentLocale && t('bibliographyAnnotations.conflictApply')}</Button>
                </div>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  {#if trashed.length > 0}
    <section class="annotations-panel__trash" bind:this={trashSection}>
      <button
        type="button"
        class="annotations-panel__trash-toggle"
        aria-expanded={trashOpen}
        onclick={() => void toggleTrash()}
      >
        <ActionIcon name={trashOpen ? 'chevron-down' : 'chevron-right'} size={12} />
        {$currentLocale && t('bibliographyAnnotations.trashGroup', { count: trashed.length })}
      </button>
      {#if trashOpen}
        <ul class="annotations-panel__list">
          {#each trashed as view (view.key)}
            <li class="annotation-entry annotation-entry--trashed" data-testid="annotation-trashed">
              <span class="annotation-entry__bar" aria-hidden="true"></span>
              <div class="annotation-entry__body">
                <div class="annotation-entry__head">
                  <span class="annotation-entry__kind">{$currentLocale && kindLabel(view.data?.annotationType)}</span>
                  {#if typeof view.data?.annotationText === 'string' && view.data.annotationText}
                    <q class="annotation-entry__quote">{view.data.annotationText}</q>
                  {/if}
                  <Button size="sm" variant="secondary" onclick={() => onRestore(view.key)}>
                    {$currentLocale && t('bibliographyAnnotations.restore')}
                  </Button>
                </div>
                {#if typeof view.data?.annotationComment === 'string' && view.data.annotationComment}
                  <p class="annotation-entry__note">{view.data.annotationComment}</p>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}

  {#if otherFormatCount > 0}
    <p class="annotations-panel__note">
      {$currentLocale && t('bibliographyAnnotations.otherFormat', { count: otherFormatCount })}
    </p>
  {/if}
</aside>

<style>
  .annotations-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
  }

  .annotations-panel__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .annotations-panel__title,
  .annotations-panel__page {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .annotations-panel__page {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
  }

  .annotations-panel__status,
  .annotations-panel__empty,
  .annotations-panel__note {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .annotations-panel__group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .annotations-panel__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .annotation-entry {
    display: flex;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-control);
  }

  .annotation-entry--selected {
    border-color: color-mix(in srgb, var(--color-accent) 55%, var(--color-hairline));
    background: var(--color-accent-faint);
  }

  .annotation-entry--trashed {
    opacity: 0.75;
  }

  .annotation-entry__bar {
    flex: 0 0 3px;
    border-radius: var(--radius-full);
    background: var(--color-border-strong);
  }

  .annotation-entry__body {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .annotation-entry__head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-1);
  }

  .annotation-entry__jump {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    text-align: start;
    cursor: pointer;
  }

  .annotation-entry__jump:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .annotation-entry__kind {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .annotation-entry__quote {
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .annotation-entry__badge {
    flex: 0 0 auto;
    padding: 0 var(--space-1);
    border-radius: var(--radius-sm);
    background: var(--color-info-soft);
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    white-space: nowrap;
  }

  .annotation-entry__badge--conflict {
    background: var(--color-warning-soft);
  }

  .annotation-entry__badge--error {
    background: var(--color-danger-soft);
  }

  /* Sized by the image itself: a column flex child would stretch it to the
     full width and letterbox a narrow region. */
  .annotation-entry__preview {
    display: block;
    align-self: flex-start;
    width: auto;
    height: auto;
    max-width: 100%;
    max-height: 160px;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
  }

  .annotation-entry__comment {
    width: 100%;
    min-height: var(--control-height-md);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-sm);
    /* One line while empty; grows with the comment. */
    field-sizing: content;
    max-height: 12lh;
    resize: none;
  }

  .annotation-entry__comment:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .annotation-entry__tags,
  .annotation-entry__note {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
  }

  .annotation-entry__note--error {
    color: var(--color-danger);
  }

  .annotation-conflict {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-control);
    background: var(--color-warning-soft);
  }

  .annotation-conflict__title,
  .annotation-conflict__field,
  .annotation-conflict__reason {
    margin: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
  }

  .annotation-conflict__values {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    overflow-wrap: anywhere;
  }

  .annotation-conflict__choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .annotations-panel__trash-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    cursor: pointer;
  }

  .annotations-panel__trash-toggle:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
</style>
