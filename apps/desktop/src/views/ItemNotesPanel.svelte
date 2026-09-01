<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { appendLog, type AppLogLevel } from '$lib/logs'
  import {
    ActionIcon,
    ConfirmDialog,
    IconButton,
    normalizeNoteContentForRender,
    normalizeNoteLinkHref,
  } from '@entropia/ui'
  import { NoteEditor } from '@entropia/ui/components/NoteEditor'
  import { TopicEditor } from '@entropia/ui'
  import type { NoteEditorProps, TopicEditorProps } from '@entropia/ui'

  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { Note } from '@entropia/store'

  let {
    itemTopics,
    topicSuggestions,
    assetsCount,
    selectedAssetIndex,
    notes,
    editingNoteId,
    expandedNoteId,
    pendingDeleteNoteId,
    deletingNote,
    noteEditorLabels,
    translate,
    onTopicsChange,
    onSaveNote,
    onTranscribeDictation,
    onSaveEdit,
    onCancelEdit,
    onEditNote,
    onOpenDeleteNoteConfirm,
    onDeleteNoteCancel,
    onDeleteNoteConfirm,
    onToggleNoteExpanded,
  }: {
    itemTopics: string[]
    topicSuggestions: string[]
    assetsCount: number
    selectedAssetIndex: number
    notes: Note[]
    editingNoteId: string | null
    expandedNoteId: string | null
    pendingDeleteNoteId: string | null
    deletingNote: boolean
    noteEditorLabels: NoteEditorProps['labels']
    translate: (key: I18nKey, params?: I18nParams) => string
    onTopicsChange: NonNullable<TopicEditorProps['onchange']>
    onSaveNote: NonNullable<NoteEditorProps['onsave']>
    onTranscribeDictation: NonNullable<NoteEditorProps['ondictate']>
    onSaveEdit: (noteId: string, content: string) => void | Promise<void>
    onCancelEdit: () => void
    onEditNote: (note: Note) => void
    onOpenDeleteNoteConfirm: (noteId: string) => void
    onDeleteNoteCancel: () => void
    onDeleteNoteConfirm: () => void | Promise<void>
    onToggleNoteExpanded: (noteId: string) => void
  } = $props()

  function handleNoteRowClick(noteId: string, event: MouseEvent) {
    const target = event.target
    if (target instanceof Element && target.closest('a, button')) {
      return
    }
    onToggleNoteExpanded(noteId)
  }

  function handleNoteRowKeydown(noteId: string, event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    onToggleNoteExpanded(noteId)
  }

  function getRenderedNoteContent(content: string): string {
    return normalizeNoteContentForRender(content)
  }

  function shouldOpenNoteLinkViaDesktopBridge(url: string): boolean {
    if (url.startsWith('#')) return false

    try {
      const protocol = new URL(url).protocol.toLowerCase()
      return protocol === 'http:' || protocol === 'https:'
    } catch {
      return false
    }
  }

  async function handleExpandedNoteContentClick(event: MouseEvent) {
    const target = event.target
    if (!(target instanceof Element)) return

    const link = target.closest('a')
    if (!(link instanceof HTMLAnchorElement)) return

    const url = normalizeNoteLinkHref(link.getAttribute('href') ?? link.href)
    if (!url) return
    if (!shouldOpenNoteLinkViaDesktopBridge(url)) return

    event.preventDefault()
    event.stopPropagation()

    try {
      await invoke('open_external_url', { url })
    } catch (error) {
      console.error(`[ItemNotesPanel] ${translate('item.noteOpenLinkError')}`, error)
    }
  }

  function expandedNoteContentLinkHandler(node: HTMLElement) {
    const handleClick = (event: MouseEvent) => {
      void handleExpandedNoteContentClick(event)
    }

    node.addEventListener('click', handleClick)

    return {
      destroy() {
        node.removeEventListener('click', handleClick)
      },
    }
  }

  function getPlainTextFromNote(content: string): string {
    const rendered = getRenderedNoteContent(content)
    if (!rendered) return ''

    const withSeparators = rendered
      .replace(/<br\s*\/?>/gi, '\n')
      .replace(/<li\b[^>]*>/gi, '• ')
      .replace(/<\/(?:p|h1|h2|h3|li|blockquote|pre|ul|ol)>/gi, '\n')

    if (typeof document === 'undefined') {
      return withSeparators
        .replace(/<[^>]+>/g, ' ')
        .replace(/\s+/g, ' ')
        .trim()
    }

    const container = document.createElement('div')
    container.innerHTML = withSeparators
    return (container.textContent ?? '').replace(/\s+/g, ' ').trim()
  }

  function getNotePreview(content: string): string {
    return getPlainTextFromNote(content)
  }

  function logDictationDiagnostic(level: AppLogLevel, message: string) {
    void appendLog(level, 'dictation', message).catch((error) => {
      console.error('[ItemNotesPanel] Failed to append dictation diagnostic log:', error)
    })
  }

  function formatNoteDate(timestamp: number): string {
    return new Date(timestamp).toLocaleDateString()
  }
</script>

<section class="section">
  <h3>{translate('item.topics')}</h3>
  <TopicEditor topics={itemTopics} suggestions={topicSuggestions} onchange={onTopicsChange} />
</section>

<section class="section">
  <h3>
    {translate('item.addNote')}{#if assetsCount > 1}
      {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
  </h3>
  <NoteEditor
    onsave={onSaveNote}
    ondictate={onTranscribeDictation}
    ondictationlog={logDictationDiagnostic}
    clearOnSave={true}
    placeholder={translate('item.writeNote')}
    saveLabel={translate('item.saveNote')}
    labels={noteEditorLabels}
  />
</section>

<section class="section">
  <h3>
    {translate('item.notes')} ({notes.length}){#if assetsCount > 1}
      {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
  </h3>
  {#if notes.length === 0}
    <p class="empty-text">{translate('item.noNotes')}</p>
  {:else}
    <div class="notes-list">
      {#each notes as note (note.id)}
        <div class="note-card">
          {#if editingNoteId === note.id}
            <div class="note-edit">
              <NoteEditor
                content={note.content}
                onsave={(content) => onSaveEdit(note.id, content)}
                oncancel={onCancelEdit}
                ondictate={onTranscribeDictation}
                ondictationlog={logDictationDiagnostic}
                clearOnSave={false}
                saveLabel={translate('item.saveNote')}
                cancelLabel={translate('item.cancelEdit')}
                labels={noteEditorLabels}
              />
            </div>
          {:else}
            <div
              class="note-row"
              role="button"
              tabindex="0"
              aria-label={getNotePreview(note.content)}
              aria-expanded={expandedNoteId === note.id}
              onclick={(event) => handleNoteRowClick(note.id, event)}
              onkeydown={(event) => handleNoteRowKeydown(note.id, event)}
            >
              <span class="note-preview" title={getNotePreview(note.content)}>
                {getNotePreview(note.content)}
              </span>
              <p class="note-date note-date--inline">{formatNoteDate(note.createdAt)}</p>
              <div class="note-actions">
                <IconButton
                  class="note-action-button note-action-button--edit"
                  variant="ghost"
                  size="sm"
                  label={translate('item.editNote')}
                  onclick={(event) => {
                    event.stopPropagation()
                    onEditNote(note)
                  }}
                >
                  <ActionIcon name="edit" />
                </IconButton>
                <IconButton
                  class="note-action-button note-action-button--delete"
                  variant="ghost"
                  size="sm"
                  label={translate('item.deleteNote')}
                  onclick={(event) => {
                    event.stopPropagation()
                    onOpenDeleteNoteConfirm(note.id)
                  }}
                >
                  <ActionIcon name="delete" />
                </IconButton>
              </div>
            </div>
            {#if expandedNoteId === note.id}
              <div
                class="note-expanded note-content note-content--rich"
                use:expandedNoteContentLinkHandler
              >
                <!-- eslint-disable-next-line svelte/no-at-html-tags -- content is sanitized by normalizeNoteContentForRender (sanitizeNoteHtml) -->
                {@html getRenderedNoteContent(note.content)}
              </div>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</section>

{#if pendingDeleteNoteId}
  <ConfirmDialog
    title={translate('item.deleteNoteTitle')}
    titleId="delete-note-modal-title"
    message={translate('item.deleteNoteMessage')}
    cancelLabel={translate('collections.cancel')}
    confirmIcon="delete"
    confirmAriaLabel={translate('item.confirmDeleteNote')}
    confirmTitle={translate('item.confirmDeleteNote')}
    variant="destructive"
    confirming={deletingNote}
    cancelDisabled={deletingNote}
    oncancel={onDeleteNoteCancel}
    onconfirm={onDeleteNoteConfirm}
  />
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

  .notes-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .note-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--surface-card);
  }

  .note-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    cursor: pointer;
  }

  .note-row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    border-radius: var(--radius-sm);
  }

  .note-preview {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-primary);
    line-height: 1.35;
  }

  .note-content {
    color: var(--color-text-primary);
    line-height: 1.6;
    word-break: break-word;
  }

  .note-expanded {
    padding-top: var(--space-1);
  }

  .note-content--rich :global(p:first-child),
  .note-content--rich :global(h1:first-child),
  .note-content--rich :global(h2:first-child),
  .note-content--rich :global(h3:first-child),
  .note-content--rich :global(blockquote:first-child) {
    margin-top: 0;
  }

  .note-content--rich :global(p:last-child),
  .note-content--rich :global(h1:last-child),
  .note-content--rich :global(h2:last-child),
  .note-content--rich :global(h3:last-child),
  .note-content--rich :global(blockquote:last-child),
  .note-content--rich :global(ul:last-child),
  .note-content--rich :global(ol:last-child) {
    margin-bottom: 0;
  }

  .note-content--rich :global(a) {
    color: var(--color-accent-hover);
    text-decoration: underline;
  }

  .note-content--rich :global(blockquote) {
    margin: var(--space-3) 0;
    padding-left: var(--space-3);
    border-left: 3px solid color-mix(in srgb, var(--color-accent) 45%, var(--color-border));
    color: var(--color-text-secondary);
  }

  .note-content--rich :global(code) {
    background: color-mix(in srgb, var(--color-border) 65%, transparent);
    border-radius: var(--radius-sm);
    padding: 0.1rem 0.3rem;
    font-size: 0.95em;
  }

  .note-content--rich :global(pre) {
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    overflow-x: auto;
  }

  .note-content--rich :global(ul),
  .note-content--rich :global(ol) {
    padding-left: 1.25rem;
  }

  .note-date {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    margin: 0;
  }

  .note-date--inline {
    margin-top: 0;
    white-space: nowrap;
  }

  .note-actions {
    display: flex;
    gap: var(--space-1);
    margin-top: 0;
    align-items: center;
    justify-self: end;
  }

  :global(.icon-button.note-action-button) {
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-sm);
    background: transparent;
    box-shadow: none;
    color: var(--color-text-muted);
    transition:
      color var(--transition-base),
      opacity var(--transition-base);
  }

  :global(.note-action-button:hover) {
    background: transparent;
    box-shadow: none;
    transform: none;
  }

  :global(.note-action-button--edit) {
    color: var(--color-text-secondary);
  }

  :global(.note-action-button--edit:hover) {
    color: var(--color-text-primary);
    opacity: 1;
  }

  :global(.note-action-button--delete) {
    color: var(--color-text-muted);
    opacity: 0.9;
  }

  :global(.note-action-button--delete:hover) {
    color: var(--color-danger);
    opacity: 1;
  }

  .note-edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .empty-text {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
</style>
