<script lang="ts">
  import { getStore } from '$lib/db'
  import { navigation } from '$lib/navigation'
  import { locale, t } from '$lib/i18n'
  import { getFocusableElements, getNextFocusTrapTarget } from '$lib/modal-focus'
  import { CollectionCard, SearchBar, Button, Input, Card } from '@entropia/ui'
  import { onMount, onDestroy } from 'svelte'
  import type { Collection } from '@entropia/store'

  let collections = $state<Collection[]>([])
  let searchQuery = $state('')
  let showCreate = $state(false)
  let newName = $state('')
  let newDescription = $state('')
  let loading = $state(true)
  let error = $state<string | null>(null)
  let itemCounts = $state<Record<string, number>>({})
  let editingId = $state<string | null>(null)
  let editName = $state('')
  let editDescription = $state('')
  let deletingId = $state<string | null>(null)
  let deletingName = $state('')
  let deleting = $state(false)
  let deleteDialogEl: HTMLElement | undefined = $state()
  let previousFocusedElement: HTMLElement | null = null
  const currentLocale = locale

  let filtered = $derived(
    searchQuery
      ? collections.filter((c) => c.name.toLowerCase().includes(searchQuery.toLowerCase()))
      : collections
  )

  let visibleCountLabel = $derived.by(() => {
    $currentLocale
    return filtered.length === 1
      ? t('collections.visibleCount.one', { count: filtered.length })
      : t('collections.visibleCount.other', { count: filtered.length })
  })

  async function loadCollections() {
    try {
      loading = true
      error = null
      const store = getStore()
      // Load ALL collections (including newly created ones with 0 items)
      collections = await store.collections.findAll()

      // Load item counts
      const counts: Record<string, number> = {}
      for (const c of collections) {
        counts[c.id] = await store.collections.countItems(c.id)
      }
      itemCounts = counts
    } catch (e) {
      error = e instanceof Error ? e.message : t('collections.error.load')
    } finally {
      loading = false
    }
  }

  async function handleCreate() {
    if (!newName.trim()) return
    try {
      const store = getStore()
      const collection = await store.collections.create({
        name: newName.trim(),
        description: newDescription.trim() || null,
      })
      console.log('[Collections] created collection:', collection.id, collection.name)
      newName = ''
      newDescription = ''
      showCreate = false
      await loadCollections()
    } catch (e) {
      console.log('[Collections] ERROR creating collection:', e)
      error = e instanceof Error ? e.message : t('collections.error.create')
    }
  }

  function handleEdit(collection: Collection) {
    editingId = collection.id
    editName = collection.name
    editDescription = collection.description ?? ''
  }

  function handleCancelEdit() {
    editingId = null
    editName = ''
    editDescription = ''
  }

  async function handleSaveEdit() {
    if (!editingId || !editName.trim()) return
    try {
      const store = getStore()
      await store.collections.update(editingId, {
        name: editName.trim(),
        description: editDescription.trim() || null,
      })
      editingId = null
      editName = ''
      editDescription = ''
      await loadCollections()
    } catch (e) {
      error = e instanceof Error ? e.message : t('collections.error.update')
    }
  }

  function handleDeleteRequest(id: string, name: string) {
    previousFocusedElement = document.activeElement instanceof HTMLElement ? document.activeElement : null
    deletingId = id
    deletingName = name
    deleting = false
  }

  function handleCancelDelete() {
    if (deleting) return
    deletingId = null
    deletingName = ''
    deleting = false
    previousFocusedElement?.focus()
    previousFocusedElement = null
  }

  async function handleConfirmDelete() {
    if (!deletingId) return
    console.log('[Collections] deleting collection:', deletingId, deletingName)
    try {
      deleting = true
      const store = getStore()
      await store.collections.delete(deletingId)
      console.log('[Collections] deleted successfully')
      deletingId = null
      deletingName = ''
      deleting = false
      previousFocusedElement = null
      await loadCollections()
    } catch (e) {
      console.error('[Collections] ERROR deleting collection:', e)
      error = e instanceof Error ? e.message : String(e)
      deletingId = null
      deletingName = ''
      deleting = false
      previousFocusedElement = null
    }
  }

  function handleDeleteDialogKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      handleCancelDelete()
      return
    }

    if (event.key !== 'Tab') return

    const target = getNextFocusTrapTarget(
      getFocusableElements(deleteDialogEl ?? null),
      event.target instanceof HTMLElement ? event.target : null,
      event.shiftKey,
      deleteDialogEl ?? null
    )

    if (target) {
      event.preventDefault()
      target.focus()
    }
  }

  $effect(() => {
    if (!deletingId || !deleteDialogEl) return

    setTimeout(() => {
      getFocusableElements(deleteDialogEl ?? null)[0]?.focus()
    }, 0)
  })

  function handleExternalCreate() {
    showCreate = true
    setTimeout(() => {
      document.querySelector<HTMLInputElement>('.create-form input')?.focus()
    }, 100)
  }

  function handleExternalFilter(e: Event) {
    const detail = (e as CustomEvent<string>).detail
    searchQuery = detail || ''
  }

  onMount(() => {
    loadCollections()
    window.addEventListener('entropia:create-collection', handleExternalCreate)
    window.addEventListener('entropia:filter-collections', handleExternalFilter)
  })

  onDestroy(() => {
    window.removeEventListener('entropia:create-collection', handleExternalCreate)
    window.removeEventListener('entropia:filter-collections', handleExternalFilter)
  })
</script>

<div class="collections-view page-shell">
  <section class="page-header">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('collections.eyebrow')}</span>
      <h1>{$currentLocale && t('collections.title')}</h1>
      <p>{$currentLocale && t('collections.subtitle')}</p>
      <span class="page-header__meta">{visibleCountLabel}</span>
    </div>

    <div class="page-toolbar collections-toolbar">
      <SearchBar
        placeholder={$currentLocale && t('collections.searchPlaceholder')}
        onsearch={(q) => (searchQuery = q)}
        onclear={() => (searchQuery = '')}
      />
      <Button variant="primary" onclick={() => (showCreate = !showCreate)}>
        {showCreate
          ? $currentLocale && t('collections.cancel')
          : $currentLocale && t('collections.new')}
      </Button>
    </div>
  </section>

  {#if showCreate}
    <Card>
      <form
        class="create-form"
        onsubmit={(e) => {
          e.preventDefault()
          handleCreate()
        }}
      >
        <div class="section-copy">
          <h2>{t('collections.createTitle')}</h2>
          <p>{t('collections.createDescription')}</p>
        </div>
        <Input type="text" placeholder={t('collections.namePlaceholder')} bind:value={newName} />
        <Input
          type="text"
          placeholder={t('collections.descriptionPlaceholder')}
          bind:value={newDescription}
        />
        <div class="create-form__actions">
          <Button variant="primary" type="submit" disabled={!newName.trim()}
            >{t('collections.createAction')}</Button
          >
          <Button variant="ghost" onclick={() => (showCreate = false)}
            >{t('collections.cancel')}</Button
          >
        </div>
      </form>
    </Card>
  {/if}

  {#if error}
    <p class="surface-message surface-message--error">{error}</p>
  {/if}

  {#if loading}
    <p class="surface-message surface-message--center">{t('collections.loading')}</p>
  {:else if filtered.length === 0}
    <div class="surface-message surface-message--center empty">
      <p>
        {searchQuery ? t('collections.emptySearch') : t('collections.empty')}
      </p>
    </div>
  {:else}
    <div class="grid">
      {#each filtered as collection (collection.id)}
        {#if editingId === collection.id}
          <Card>
            <form
              class="edit-form"
              onsubmit={(e) => {
                e.preventDefault()
                handleSaveEdit()
              }}
            >
              <Input
                type="text"
                placeholder={t('collections.editNamePlaceholder')}
                bind:value={editName}
              />
              <Input
                type="text"
                placeholder={t('collections.descriptionPlaceholder')}
                bind:value={editDescription}
              />
              <div class="edit-form__actions">
                <Button variant="primary" type="submit" disabled={!editName.trim()}
                  >{t('collections.save')}</Button
                >
                <Button variant="ghost" onclick={handleCancelEdit}>{t('collections.cancel')}</Button
                >
              </div>
            </form>
          </Card>
        {:else}
          <CollectionCard
            id={collection.id}
            name={collection.name}
            description={collection.description ?? undefined}
            itemCount={itemCounts[collection.id] ?? 0}
            updatedAt={new Date(collection.updatedAt).getTime()}
            onclick={() =>
              navigation.navigate({
                name: 'collection',
                id: collection.id,
                collectionName: collection.name,
              })}
            onedit={() => handleEdit(collection)}
            ondelete={() => handleDeleteRequest(collection.id, collection.name)}
          />
        {/if}
      {/each}
    </div>
  {/if}

  {#if deletingId}
    <div class="confirm-overlay" role="presentation" onclick={handleCancelDelete}>
      <Card>
        <div
          bind:this={deleteDialogEl}
          class="confirm-dialog"
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="delete-collection-title"
          aria-describedby="delete-collection-description"
          tabindex="-1"
          onkeydown={handleDeleteDialogKeydown}
          onclick={(event) => event.stopPropagation()}
        >
          <h3 id="delete-collection-title" class="confirm-dialog__title">{t('collections.deleteTitle')}</h3>
          <p id="delete-collection-description" class="confirm-dialog__message">
            {t('collections.deleteMessage', { name: deletingName })}
          </p>
          <div class="confirm-dialog__actions">
            <button
              type="button"
              class="confirm-dialog__delete-button"
              aria-label={t('collections.deleteAria')}
              title={deleting ? t('collections.deletingTitle') : t('collections.deleteAria')}
              aria-busy={deleting}
              onclick={handleConfirmDelete}
              disabled={deleting}
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M3 6h18" />
                <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
                <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
                <line x1="10" y1="11" x2="10" y2="17" />
                <line x1="14" y1="11" x2="14" y2="17" />
              </svg>
            </button>
            <Button variant="ghost" onclick={handleCancelDelete} disabled={deleting}
              >{t('collections.cancel')}</Button
            >
          </div>
        </div>
      </Card>
    </div>
  {/if}
</div>

<style>
  .collections-view {
    min-height: 100%;
  }

  .collections-toolbar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex: 1;
  }

  .collections-toolbar :global(.search-bar) {
    min-width: min(100%, 320px);
    flex: 1 1 260px;
  }

  .create-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
  }

  .create-form__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .section-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .section-copy h2 {
    font-size: var(--font-size-lg);
  }

  .section-copy p {
    max-width: 56ch;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-3);
  }

  .empty {
    min-height: 220px;
  }

  .edit-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
  }

  .edit-form__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .confirm-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: var(--color-overlay);
    z-index: 100;
  }
  .confirm-dialog {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
    min-width: min(100vw - 32px, 440px);
  }

  .confirm-dialog__title {
    margin: 0;
  }

  .confirm-dialog__message {
    margin: 0;
    font-size: var(--font-size-base, 1rem);
    color: var(--color-text-primary);
  }

  .confirm-dialog__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    justify-content: flex-end;
  }

  .confirm-dialog__delete-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-sm);
    height: var(--control-height-sm);
    padding: 0;
    border: 1px solid var(--color-danger);
    border-radius: var(--radius-md);
    background-color: var(--color-danger);
    color: var(--color-bg);
    cursor: pointer;
    transition:
      background-color var(--transition-smooth),
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      transform var(--transition-smooth);
    box-shadow: 0 8px 18px color-mix(in srgb, var(--color-danger) 18%, transparent);
  }

  .confirm-dialog__delete-button:hover:not(:disabled) {
    background-color: var(--color-danger-hover);
    border-color: var(--color-danger-hover);
    transform: translateY(-1px);
  }

  .confirm-dialog__delete-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .confirm-dialog__delete-button:disabled {
    opacity: 0.48;
    cursor: not-allowed;
    transform: none;
  }

  @media (max-width: 720px) {
    .collections-toolbar {
      width: 100%;
      justify-content: stretch;
    }

    .collections-toolbar :global(.search-bar),
    .collections-toolbar :global(.btn) {
      width: 100%;
    }

    .create-form__actions :global(.btn),
    .edit-form__actions :global(.btn),
    .confirm-dialog__actions :global(.btn) {
      width: 100%;
    }
  }
</style>
