<script lang="ts">
  import { untrack } from 'svelte'
  import { Button } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import type { WritingDocumentRow } from '$lib/writing'

  // The manuscripts discarded from the list (finding 26). Trashing keeps every
  // record, so the trash only lists them and moves one back. Loaded when it
  // opens, so the list view never pays for it.
  let {
    listTrashed,
    onRestore,
    listed = [],
  }: {
    listTrashed: () => Promise<WritingDocumentRow[]>
    /** Moves the manuscript back to the list; rejects when it could not. */
    onRestore: (id: string) => Promise<void>
    /** The workspace list: when it changes, an open trash reads itself again. */
    listed?: WritingDocumentRow[]
  } = $props()

  let open = $state(false)
  let documents = $state<WritingDocumentRow[] | null>(null)
  let error = $state<string | null>(null)
  let notice = $state<string | null>(null)
  let restoringId = $state<string | null>(null)
  let restoredId: string | null = null

  // A manuscript trashed from the list while the trash is open shows up here
  // at once, and a notice about restoring it goes away once it is back.
  $effect(() => {
    void listed
    if (untrack(() => open)) void reload()
  })

  async function reload() {
    await load()
    if (restoredId && documents?.some((doc) => doc.id === restoredId)) {
      notice = null
      restoredId = null
    }
  }

  function messageOf(cause: unknown): string {
    if (typeof cause === 'object' && cause !== null && 'message' in cause) {
      return String((cause as { message: unknown }).message)
    }
    return String(cause)
  }

  async function load() {
    try {
      documents = await listTrashed()
    } catch (cause) {
      error = t('writing.trash.loadFailed', { error: messageOf(cause) })
    }
  }

  async function toggle() {
    open = !open
    error = null
    notice = null
    if (open) await load()
  }

  async function restore(doc: WritingDocumentRow) {
    if (restoringId) return
    restoringId = doc.id
    error = null
    notice = null
    try {
      await onRestore(doc.id)
      notice = t('writing.trash.restored', { title: doc.title })
      restoredId = doc.id
      await load()
    } catch (cause) {
      error = t('writing.trash.restoreFailed', { error: messageOf(cause) })
    } finally {
      restoringId = null
    }
  }
</script>

<div class="writing-trash">
  <Button variant="secondary" size="sm" aria-expanded={open} onclick={toggle}>
    {open ? t('writing.trash.hide') : t('writing.trash.show')}
  </Button>
  {#if open}
    {#if error}
      <p class="writing-trash__error" role="alert">{error}</p>
    {/if}
    {#if notice}
      <p class="writing-trash__notice" role="status">{notice}</p>
    {/if}
    {#if documents && documents.length === 0}
      <p class="writing-trash__empty">{t('writing.trash.empty')}</p>
    {:else if documents}
      <ul class="writing-trash__list">
        {#each documents as doc (doc.id)}
          <li class="writing-trash__item">
            <span class="writing-trash__text">
              <span class="writing-trash__title">{doc.title}</span>
              <span class="writing-trash__meta">
                {t('writing.trash.trashedAt', { date: new Date(doc.updated_at).toLocaleString() })}
              </span>
            </span>
            <Button
              variant="secondary"
              size="sm"
              aria-label={t('writing.trash.restoreAria', { title: doc.title })}
              disabled={restoringId !== null}
              onclick={() => restore(doc)}
            >
              {t('writing.trash.restore')}
            </Button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  /* The trash sits under the document grid, which is the page's scroll area:
     it keeps its own size and scrolls its own list, so a long trash never
     squeezes the grid out of the page. */
  .writing-trash {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .writing-trash__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
    max-width: 720px;
    max-height: 40vh;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .writing-trash__item {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
  }

  .writing-trash__text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .writing-trash__title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .writing-trash__meta,
  .writing-trash__empty,
  .writing-trash__notice {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .writing-trash__error {
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
</style>
