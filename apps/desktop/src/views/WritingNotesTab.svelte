<script lang="ts">
  import { onDestroy } from 'svelte'
  import { ActionIcon, Button, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { FtsSearchController } from '$lib/item-view-search'
  import { writingNotes } from '$lib/writing-notes'
  import { buildNoteLink } from '$lib/note-link'

  /**
   * The Notas tab of the research panel (plan-editor.md §6.3, §13).
   *
   * The two insert actions are deliberately separate buttons with their own
   * sentence of explanation, because §13 asks for the difference to be clear
   * and it is not self-evident: copying takes the words and forgets where they
   * came from, linking keeps a relationship that will report a divergence.
   * Offering one button with a dropdown would bury exactly the decision the
   * writer needs to make.
   */

  interface Props {
    /** Inserts plain text at the caret. Absent when no manuscript is open. */
    oncopy?: (text: string) => boolean
    /** Inserts a live link at the caret. Absent when no manuscript is open. */
    onlink?: (attrs: Record<string, unknown>) => string | null
  }

  let { oncopy, onlink }: Props = $props()

  const store = writingNotes
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  let inserted = $state(false)

  const controller = new FtsSearchController({
    getQuery: () => snapshot.query,
    setQuery: () => {},
    reset: () => void store.search(''),
    search: (query) => store.search(query),
  })

  onDestroy(() => {
    unsubscribe()
    controller.cancel()
  })

  function copyNote() {
    const note = snapshot.open
    if (!note || !oncopy) return
    inserted = oncopy(note.content)
  }

  async function linkNote() {
    const note = snapshot.open
    if (!note || !onlink) return
    inserted = onlink(await buildNoteLink(note)) !== null
  }

  function open(noteId: string) {
    inserted = false
    store.openNote(noteId)
  }
</script>

<div class="notes">
  <SearchBar
    value={snapshot.query}
    debounceMs={0}
    emitSearch={false}
    ariaLabel={t('writing.notesSearch')}
    placeholder={t('writing.notesSearch')}
    onvaluechange={(query) => controller.handleInput(query)}
    onkeydown={(event) => controller.handleKeydown(event)}
  />

  {#if snapshot.scope.length > 0}
    <p class="notes__scope">{t('writing.notesScoped')}</p>
  {/if}

  {#if snapshot.error}
    <p class="notes__error" role="alert">{snapshot.error}</p>
  {/if}

  {#if snapshot.open}
    <div class="notes__open">
      <Button variant="ghost" size="sm" onclick={() => store.closeNote()}>
        <ActionIcon name="chevron-left" size={14} />
        {t('writing.notesBack')}
      </Button>
      <p class="notes__origin">{snapshot.open.itemTitle}</p>
    </div>

    <p class="notes__body">{snapshot.open.content}</p>

    <!-- Two actions, each with its consequence written next to it. The choice
         between copying and linking is the one §13 asks to be made explicit. -->
    <div class="notes__actions">
      <div class="notes__action">
        <Button variant="secondary" size="sm" disabled={!oncopy} onclick={copyNote}>
          <ActionIcon name="copy" size={14} />
          {t('writing.notesCopy')}
        </Button>
        <p class="notes__help">{t('writing.notesCopyHelp')}</p>
      </div>
      <div class="notes__action">
        <Button variant="secondary" size="sm" disabled={!onlink} onclick={linkNote}>
          <ActionIcon name="link" size={14} />
          {t('writing.notesLink')}
        </Button>
        <p class="notes__help">{t('writing.notesLinkHelp')}</p>
      </div>
    </div>

    <p class="notes__notice" role="status">
      {#if inserted}
        {t('writing.notesInserted')}
      {:else if !oncopy && !onlink}
        {t('writing.notesNoDocument')}
      {/if}
    </p>
  {:else if snapshot.searching}
    <p class="notes__notice" role="status">{t('writing.notesSearching')}</p>
  {:else if snapshot.results.length > 0}
    <ul class="notes__list">
      {#each snapshot.results as note (note.id)}
        <li>
          <button type="button" class="notes__row" onclick={() => open(note.id)}>
            <span class="notes__row-text">{note.content}</span>
            <span class="notes__row-origin">{note.itemTitle}</span>
          </button>
        </li>
      {/each}
    </ul>
  {:else if snapshot.query.trim()}
    <p class="notes__notice">{t('writing.notesEmpty')}</p>
  {:else}
    <p class="notes__notice">{t('writing.notesStart')}</p>
  {/if}
</div>

<style>
  .notes {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }

  .notes__open {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
  }

  .notes__origin {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
  }

  .notes__body {
    max-height: 32vh;
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
    white-space: pre-wrap;
    overflow-y: auto;
  }

  .notes__actions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .notes__action {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
  }

  .notes__help {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  .notes__list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .notes__row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background var(--transition-base);
  }

  .notes__row:hover {
    background: var(--color-accent-faint);
  }

  .notes__row-text {
    font-size: var(--font-size-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .notes__row-origin {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
  }

  .notes__scope,
  .notes__notice,
  .notes__error {
    margin: 0;
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .notes__scope,
  .notes__notice {
    color: var(--color-text-muted);
  }

  .notes__error {
    color: var(--color-danger);
  }
</style>
