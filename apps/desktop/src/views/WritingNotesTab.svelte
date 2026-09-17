<script lang="ts">
  import { onDestroy } from 'svelte'
  import { ActionIcon, Button, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { plainTextOf, previewTextOf } from '$lib/note-text'
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
    /** The passage selected in the manuscript, for writing it down (§13.1). */
    selection?: () => string
  }

  let { oncopy, onlink, selection }: Props = $props()

  const store = writingNotes
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  let inserted = $state(false)

  /**
   * Writing a passage of the manuscript down as a note (§13.1).
   *
   * The item picker is not a convenience, it is the requirement: `notes.item_id`
   * is NOT NULL and §13.1 forbids relaxing it or minting a fictitious item to
   * hold a manuscript-only note. So the destination is chosen, explicitly, and
   * the sentence under the field says why rather than leaving it looking like
   * needless friction.
   */
  let creating = $state(false)
  let passage = $state('')
  let targetQuery = $state('')
  let created = $state<string | null>(null)
  /** Set only after an attempt, so the hint answers a question that was asked. */
  let needsSelection = $state(false)

  function startNote() {
    passage = selection?.() ?? ''
    targetQuery = ''
    created = null
    void store.findTargets('')
    creating = passage.length > 0
    needsSelection = !creating
  }

  async function chooseTarget(itemId: string, itemTitle: string) {
    const note = await store.createFromSelection({ text: passage, itemId })
    if (!note) return
    created = itemTitle
    creating = false
    void store.findTargets('')
  }

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
    // The note's words, not its markup: this goes straight into the article
    // and §13 calls it a copy, which a copy of `<p>…</p>` is not.
    inserted = oncopy(plainTextOf(note.content))
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

    <p class="notes__body">{plainTextOf(snapshot.open.content)}</p>

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
            <!-- One line, so a note of four paragraphs does not break the row. -->
            <span class="notes__row-text">{previewTextOf(note.content)}</span>
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

  {#if creating}
    <div class="notes__create">
      <blockquote class="notes__passage">{passage}</blockquote>
      <p class="notes__help">{t('writing.noteCreateWhy')}</p>
      <SearchBar
        value={targetQuery}
        debounceMs={200}
        emitSearch={false}
        ariaLabel={t('writing.noteCreateTarget')}
        placeholder={t('writing.noteCreateTarget')}
        onvaluechange={(query) => {
          targetQuery = query
          void store.findTargets(query)
        }}
      />
      {#if snapshot.targets.length > 0}
        <ul class="notes__list">
          {#each snapshot.targets as item (item.id)}
            <li>
              <button
                type="button"
                class="notes__row"
                onclick={() => chooseTarget(item.id, item.title)}
              >
                <span class="notes__row-text">{item.title}</span>
              </button>
            </li>
          {/each}
        </ul>
      {:else if targetQuery.trim()}
        <p class="notes__notice">{t('writing.noteCreateNoTargets')}</p>
      {/if}
      <Button variant="ghost" size="sm" onclick={() => (creating = false)}>
        {t('writing.noteCreateCancel')}
      </Button>
    </div>
  {:else if selection}
    <div class="notes__create">
      <Button variant="ghost" size="sm" onclick={startNote}>
        <ActionIcon name="add" size={14} />
        {t('writing.noteCreate')}
      </Button>
      {#if created}
        <p class="notes__notice" role="status">{t('writing.noteCreated', { item: created })}</p>
      {:else if needsSelection}
        <p class="notes__help">{t('writing.noteCreateNoSelection')}</p>
      {/if}
    </div>
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

  .notes__create {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border-subtle);
  }

  .notes__passage {
    max-height: 18vh;
    margin: 0;
    padding-left: var(--space-3);
    border-left: 2px solid var(--border-subtle);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-style: italic;
    overflow-y: auto;
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
