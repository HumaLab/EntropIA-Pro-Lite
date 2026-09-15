<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { ActionIcon, Button, Panel, StatusBadge, WritingEditor } from '@entropia/ui'
  import type { CanonicalDocument, StatusBadgeVariant } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { navigation } from '$lib/navigation'
  import { writing, type SaveStatus, type WritingDocumentRow } from '$lib/writing'

  const store = writing
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  onMount(async () => {
    const ready = await store.init()
    if (!ready) return
    // The store is a module singleton and outlives this view, so a remount can
    // arrive with a document still open. Navigation decides what is showing;
    // the store follows it, never the other way round.
    const requested = navigationDocumentId()
    if (requested) {
      if (store.snapshot.open?.id !== requested) await store.openDocument(requested)
    } else {
      store.closeDocument()
      await store.listDocuments()
    }
  })

  function navigationDocumentId(): string | null {
    const current = navigation.current
    return current.name === 'writing' ? (current.documentId ?? null) : null
  }

  onDestroy(() => {
    unsubscribe()
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
    if (id) await open(id)
  }

  async function open(id: string) {
    await store.openDocument(id)
    const title = snapshot.open?.title ?? null
    navigation.replace({ name: 'writing', documentId: id, documentTitle: title })
  }

  async function backToList() {
    await store.flush()
    store.closeDocument()
    navigation.replace({ name: 'writing', documentId: null, documentTitle: null })
    await store.listDocuments()
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
      <div class="writing__bar-end">
        <span class="writing__revision">
          {t('writing.revision', { revision: String(snapshot.revision) })}
        </span>
        <StatusBadge variant={statusVariant(snapshot.status)}>
          {t(STATUS_LABEL[snapshot.status])}
        </StatusBadge>
      </div>
    </header>

    <div class="writing__editor">
      {#if snapshot.content}
        <WritingEditor
          document={snapshot.content}
          onchange={onEditorChange}
          placeholder={t('writing.placeholder')}
        />
      {:else if snapshot.refusal}
        <WritingEditor document={{ schemaVersion: 1, doc: { type: 'doc' } }} />
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
          <li>
            <button type="button" class="writing__card" onclick={() => open(doc.id)}>
              <span class="writing__card-title">{doc.title}</span>
              <span class="writing__card-meta">{formatDate(doc.updated_at)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

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

  .writing__editor {
    flex: 1;
    min-height: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    overflow: hidden;
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

  .writing__card {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    width: 100%;
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
