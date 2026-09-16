<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import {
    ActionIcon,
    Button,
    ConfirmDialog,
    IconButton,
    Panel,
    StatusBadge,
    WritingEditor,
    outlineDepth,
    outlineFromDocument,
  } from '@entropia/ui'
  import type { CanonicalDocument, StatusBadgeVariant } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { navigation, type View } from '$lib/navigation'
  import { writing, type SaveStatus, type WritingDocumentRow } from '$lib/writing'
  import WritingResearchPanel, { type ResearchTab } from './WritingResearchPanel.svelte'

  const store = writing
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  // `subscribe` fires synchronously, so this is populated before first use.
  let navSnapshot = $state<{ current: View; canGoBack: boolean } | null>(null)
  const unsubscribeNav = navigation.subscribe((value) => {
    navSnapshot = value
  })

  /**
   * The one place the store's open document is decided.
   *
   * Every route into this section — a card, the Back button, the section
   * crumb, the top-bar icon — changes navigation and nothing else. This
   * reconciles the store to it. Anything that also mutated the store directly
   * would race against this and lose.
   */
  let reconciling = false
  $effect(() => {
    const view = navSnapshot?.current
    const requested = view?.name === 'writing' ? (view.documentId ?? null) : null
    const openId = snapshot.open?.id ?? null
    if (!snapshot.ready || reconciling) return
    if (requested === openId) return

    reconciling = true
    void (async () => {
      try {
        if (requested) {
          await store.openDocument(requested)
        } else {
          store.closeDocument()
          await store.listDocuments()
        }
      } finally {
        reconciling = false
      }
    })()
  })

  onMount(async () => {
    // Only the gate and the list. Which document is open is the effect's
    // business, including on a remount that arrives with one still held: the
    // store is a module singleton and outlives this view.
    if (await store.init()) await store.listDocuments()
  })

  onDestroy(() => {
    unsubscribe()
    unsubscribeNav()
    // Persist whatever is pending, then release the timer. The document stays
    // open in the store on purpose: navigating away and back should return to
    // it, and onMount reconciles against navigation.
    void store.flush().finally(() => store.dispose())
  })

  const STATUS_LABEL: Record<SaveStatus, string> = {
    saved: 'writing.status.saved',
    saving: 'writing.status.saving',
    pending: 'writing.status.pending',
    error: 'writing.status.error',
    'recovery-available': 'writing.status.recovery',
  }

  function statusVariant(status: SaveStatus): StatusBadgeVariant {
    if (status === 'saved') return 'success'
    if (status === 'error') return 'danger'
    if (status === 'pending' || status === 'recovery-available') return 'warning'
    return 'neutral'
  }

  async function createDocument() {
    const id = await store.createDocument(t('writing.newDocumentTitle'))
    if (id) open(id)
  }

  /**
   * Opening is a navigation, nothing else.
   *
   * The reconciling effect below owns the store: if this also called
   * `openDocument` the two would race — the store would change first, the
   * effect would see a route with no document yet and close it again.
   *
   * Pushed, not replaced: the list has to stay in history so the shell's Back
   * button returns to it instead of leaving the section entirely.
   */
  function open(id: string) {
    navigation.navigate({
      name: 'writing',
      documentId: id,
      documentTitle: documents.find((d) => d.id === id)?.title ?? null,
    })
  }

  async function backToList() {
    await store.flush()
    if (navigationCanGoBack()) {
      navigation.back()
    } else {
      navigation.replace({ name: 'writing', documentId: null, documentTitle: null })
    }
  }

  function navigationCanGoBack(): boolean {
    return navSnapshot?.canGoBack ?? false
  }

  async function commitTitle(value: string) {
    const current = snapshot.open
    if (!current || value.trim() === current.title) return
    await store.renameDocument(current.id, value)
    navigation.replace({
      name: 'writing',
      documentId: current.id,
      documentTitle: snapshot.open?.title ?? null,
    })
  }

  function onTitleKeydown(event: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (event.key === 'Enter') {
      event.preventDefault()
      event.currentTarget.blur()
    }
    if (event.key === 'Escape') {
      event.currentTarget.value = snapshot.open?.title ?? ''
      event.currentTarget.blur()
    }
  }

  function onEditorChange(next: CanonicalDocument) {
    store.applyEdit(next)
  }

  function formatDate(ms: number): string {
    return new Date(ms).toLocaleString()
  }

  let editorRef = $state<{ goToPosition: (position: number) => void } | undefined>(undefined)
  /**
   * Which side panels are showing. §6.3 asks for both to be foldable so the
   * editor can take the full width for a concentrated session, so the two are
   * remembered the same way rather than one being a special case.
   */
  const OUTLINE_PREFERENCE = 'entropia-writing-outline'
  const RESEARCH_PREFERENCE = 'entropia-writing-research'

  function readPanelPreference(key: string): boolean {
    try {
      return localStorage.getItem(key) !== 'closed'
    } catch {
      // A blocked storage is not a reason to hide a panel.
      return true
    }
  }

  function writePanelPreference(key: string, open: boolean) {
    try {
      localStorage.setItem(key, open ? 'open' : 'closed')
    } catch {
      // Nor is it a reason to refuse the toggle.
    }
  }

  let outlineOpen = $state(readPanelPreference(OUTLINE_PREFERENCE))
  let researchOpen = $state(readPanelPreference(RESEARCH_PREFERENCE))
  let researchTab = $state<ResearchTab>('corpus')

  /** Derived from the document, never kept as a second copy (§6.1). */
  const outline = $derived(outlineFromDocument(snapshot.content))

  function toggleOutline() {
    outlineOpen = !outlineOpen
    writePanelPreference(OUTLINE_PREFERENCE, outlineOpen)
  }

  function toggleResearch() {
    researchOpen = !researchOpen
    writePanelPreference(RESEARCH_PREFERENCE, researchOpen)
  }

  /** The document a discard was asked for, held until it is confirmed. */
  let pendingDiscard = $state<WritingDocumentRow | null>(null)

  async function confirmDiscard() {
    const target = pendingDiscard
    if (!target) return
    pendingDiscard = null
    await store.trashDocument(target.id)
  }

  /** Recomputed from the snapshot so it tracks every status change. */
  const canRetrySave = $derived(snapshot.status === 'error' && store.canRetrySave)

  const openDocument = $derived(snapshot.open)
  const documents = $derived(snapshot.documents as WritingDocumentRow[])
</script>

<section class="writing">
  {#if !snapshot.ready}
    <Panel padding="lg">
      <p class="writing__notice" role="status">{t('writing.notReady')}</p>
    </Panel>
  {:else if openDocument && (snapshot.content || snapshot.refusal)}
    <header class="writing__bar">
      <Button variant="ghost" size="sm" onclick={backToList}>
        <ActionIcon name="chevron-left" size={14} />
        {t('writing.backToList')}
      </Button>
      <input
        class="writing__doc-title"
        type="text"
        value={openDocument.title}
        aria-label={t('writing.titleLabel')}
        placeholder={t('writing.newDocumentTitle')}
        onblur={(event) => commitTitle(event.currentTarget.value)}
        onkeydown={onTitleKeydown}
      />
      <IconButton
        size="sm"
        variant="ghost"
        label={t('writing.toggleOutline')}
        active={outlineOpen}
        onclick={toggleOutline}
      >
        <ActionIcon name="list" size={14} />
      </IconButton>
      <IconButton
        size="sm"
        variant="ghost"
        label={t('writing.toggleResearch')}
        active={researchOpen}
        onclick={toggleResearch}
      >
        <ActionIcon name="search" size={14} />
      </IconButton>
      <div class="writing__bar-end">
        <span class="writing__revision">
          {t('writing.revision', { revision: String(snapshot.revision) })}
        </span>
        <StatusBadge variant={statusVariant(snapshot.status)}>
          {t(STATUS_LABEL[snapshot.status])}
        </StatusBadge>
      </div>
    </header>

    {#if snapshot.status === 'error' && snapshot.error}
      <Panel padding="md">
        <div class="writing__save-error" role="alert">
          <p class="writing__error">
            {canRetrySave
              ? t('writing.saveFailed', { message: snapshot.error.message })
              : t('writing.saveConflict')}
          </p>
          {#if canRetrySave}
            <Button variant="secondary" size="sm" onclick={() => void store.retrySave()}>
              <ActionIcon name="refresh" size={14} />
              {t('writing.retrySave')}
            </Button>
          {/if}
        </div>
      </Panel>
    {/if}

    {#if snapshot.repair}
      <Panel padding="md">
        <p class="writing__notice" role="status">
          {t('writing.repaired', {
            count: String(snapshot.repair.orphanFootnoteReferences),
          })}
        </p>
      </Panel>
    {/if}

    <div class="writing__workspace">
      {#if outlineOpen}
        <nav class="writing__outline" aria-label={t('writing.outline')}>
          <p class="writing__outline-title">{t('writing.outline')}</p>
          {#if outline.length === 0}
            <p class="writing__outline-empty">{t('writing.outlineEmpty')}</p>
          {:else}
            <ul class="writing__outline-list">
              {#each outline as entry (entry.index)}
                <li>
                  <button
                    type="button"
                    class="writing__outline-item"
                    style:padding-left="calc(var(--space-2) + {outlineDepth(outline, entry)} * var(--space-3))"
                    onclick={() => editorRef?.goToPosition(entry.position)}
                  >
                    {entry.text || t('writing.outlineUntitled')}
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </nav>
      {/if}

      <div class="writing__editor">
        {#if snapshot.content}
          <WritingEditor
            bind:this={editorRef}
            document={snapshot.content}
            onchange={onEditorChange}
            placeholder={t('writing.placeholder')}
          />
        {:else if snapshot.refusal}
          <WritingEditor document={{ schemaVersion: 1, doc: { type: 'doc' } }} toolbar={false} />
        {/if}
      </div>

      {#if researchOpen}
        <aside class="writing__research">
          <WritingResearchPanel bind:tab={researchTab} />
        </aside>
      {/if}
    </div>
  {:else}
    <header class="writing__header">
      <div>
        <p class="writing__eyebrow">{t('writing.eyebrow')}</p>
        <h1 class="writing__title">{t('writing.title')}</h1>
        <p class="writing__subtitle">{t('writing.subtitle')}</p>
      </div>
      <Button variant="primary" size="md" onclick={createDocument}>
        <ActionIcon name="add" size={16} />
        {t('writing.newDocument')}
      </Button>
    </header>

    {#if snapshot.error}
      <Panel padding="md">
        <p class="writing__error" role="alert">{snapshot.error.message}</p>
      </Panel>
    {/if}

    {#if documents.length === 0}
      <Panel padding="lg">
        <p class="writing__notice">{t('writing.empty')}</p>
      </Panel>
    {:else}
      <ul class="writing__list">
        {#each documents as doc (doc.id)}
          <li class="writing__row">
            <button type="button" class="writing__card" onclick={() => open(doc.id)}>
              <span class="writing__card-title">{doc.title}</span>
              <span class="writing__card-meta">{formatDate(doc.updated_at)}</span>
            </button>
            <IconButton
              size="sm"
              variant="ghost"
              label={t('writing.discard', { title: doc.title })}
              onclick={() => (pendingDiscard = doc)}
            >
              <ActionIcon name="delete" size={14} />
            </IconButton>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

{#if pendingDiscard}
  <ConfirmDialog
    variant="destructive"
    title={t('writing.discardTitle')}
    message={t('writing.discardMessage', { title: pendingDiscard.title })}
    confirmLabel={t('writing.discardConfirm')}
    cancelLabel={t('writing.discardCancel')}
    onconfirm={confirmDiscard}
    oncancel={() => (pendingDiscard = null)}
  />
{/if}

<style>
  .writing {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-height: 0;
    height: 100%;
    padding: var(--space-5);
  }

  .writing__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .writing__eyebrow {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .writing__title {
    margin: var(--space-1) 0 0;
    font-family: var(--font-display);
    font-size: var(--font-size-xl);
  }

  .writing__subtitle {
    margin: var(--space-1) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .writing__bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  /* Reads as the heading it replaces until you put the caret in it. */
  .writing__doc-title {
    flex: 1;
    min-width: 0;
    min-height: 32px;
    margin: 0;
    padding: 0 var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-primary);
    font-family: var(--font-display);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
  }

  .writing__doc-title:hover {
    border-color: var(--border-subtle);
  }

  .writing__doc-title:focus {
    outline: none;
    border-color: var(--border-focus);
    background: var(--color-surface-raised);
  }

  .writing__doc-title:focus-visible {
    box-shadow: var(--focus-ring);
  }

  .writing__bar-end {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .writing__revision {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
  }

  .writing__workspace {
    display: flex;
    flex: 1;
    min-height: 0;
    gap: var(--space-3);
  }

  .writing__editor {
    flex: 1;
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    overflow: hidden;
  }

  /* Mirrors the outline's fixed column. With both folded away the editor
     panel takes the whole width; the text column inside it stays at its own
     measure, because a 200-character line is not a wider editor, it is an
     unreadable one. */
  .writing__research {
    display: flex;
    flex-direction: column;
    flex: 0 0 280px;
    min-height: 0;
    padding: var(--space-3) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-panel);
    overflow-y: auto;
  }

  .writing__outline {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 0 0 240px;
    min-height: 0;
    padding: var(--space-3) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-panel);
    overflow-y: auto;
  }

  .writing__outline-title {
    margin: 0 0 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .writing__outline-empty {
    margin: 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .writing__outline-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .writing__outline-item {
    display: block;
    width: 100%;
    min-height: 28px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font: inherit;
    font-size: var(--font-size-sm);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .writing__outline-item:hover {
    background: var(--color-surface-elevated);
    color: var(--color-text-primary);
  }

  .writing__outline-item:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .writing__notice,
  .writing__error {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .writing__error {
    color: var(--color-danger);
  }

  .writing__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }

  /* The delete control is a sibling of the card, never inside it: a button
     nested in a button is invalid, and the browser would give the outer one
     the click either way. */
  .writing__save-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .writing__row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .writing__card {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    flex: 1;
    min-width: 0;
    min-height: 44px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background var(--transition-base), border-color var(--transition-base);
  }

  .writing__card:hover {
    background: var(--color-surface-elevated);
    border-color: var(--color-border-strong);
  }

  .writing__card:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .writing__card-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .writing__card-meta {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
  }
</style>
