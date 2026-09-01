<script lang="ts">
  import { getStore } from '$lib/db'
  import { navigation } from '$lib/navigation'
  import { locale, t } from '$lib/i18n'
  import {
    ActionIcon,
    CollectionCard,
    SearchBar,
    Button,
    Input,
    Card,
    ConfirmDialog,
  } from '@entropia/ui'
  import { onMount, onDestroy } from 'svelte'
  import { DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT } from '$lib/document-explorer'
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
  const currentLocale = locale
  let collectionsLoadRequestId = 0

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

  /** Announced only after a write actually succeeded, so the sidebar never
   *  reloads for a change that did not happen. */
  function notifyCollectionsChanged() {
    window.dispatchEvent(new CustomEvent(DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT))
  }

  async function loadCollections() {
    const requestId = ++collectionsLoadRequestId
    try {
      loading = true
      error = null
      const store = getStore()
      // Load ALL collections (including newly created ones with 0 items)
      const loadedCollections = await store.collections.findAll()
      if (requestId !== collectionsLoadRequestId) return

      // Load item counts
      const counts: Record<string, number> = {}
      for (const c of loadedCollections) {
        counts[c.id] = await store.collections.countItems(c.id)
        if (requestId !== collectionsLoadRequestId) return
      }
      collections = loadedCollections
      itemCounts = counts
    } catch (e) {
      if (requestId !== collectionsLoadRequestId) return
      error = e instanceof Error ? e.message : t('collections.error.load')
    } finally {
      if (requestId === collectionsLoadRequestId) {
        loading = false
      }
    }
  }

  async function handleCreate() {
    if (!newName.trim()) return
    try {
      const store = getStore()
      await store.collections.create({
        name: newName.trim(),
        description: newDescription.trim() || null,
      })
      newName = ''
      newDescription = ''
      showCreate = false
      notifyCollectionsChanged()
      await loadCollections()
    } catch (e) {
      console.error('[Collections] ERROR creating collection:', e)
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
      notifyCollectionsChanged()
      await loadCollections()
    } catch (e) {
      error = e instanceof Error ? e.message : t('collections.error.update')
    }
  }

  function handleDeleteRequest(id: string, name: string) {
    deletingId = id
    deletingName = name
    deleting = false
  }

  function handleCancelDelete() {
    if (deleting) return
    deletingId = null
    deletingName = ''
    deleting = false
  }

  async function handleConfirmDelete() {
    if (!deletingId) return
    try {
      deleting = true
      const store = getStore()
      await store.collections.delete(deletingId)
      deletingId = null
      deletingName = ''
      deleting = false
      notifyCollectionsChanged()
      await loadCollections()
    } catch (e) {
      console.error('[Collections] ERROR deleting collection:', e)
      error = e instanceof Error ? e.message : String(e)
      deletingId = null
      deletingName = ''
      deleting = false
    }
  }

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
  <section class="collections-intro" aria-labelledby="collections-title">
    <div class="collections-intro__content">
      <span class="collections-intro__eyebrow">{$currentLocale && t('collections.eyebrow')}</span>
      <div class="collections-intro__copy">
        <h1 id="collections-title">{$currentLocale && t('collections.title')}</h1>
        <p>{$currentLocale && t('collections.subtitle')}</p>
      </div>
      <span class="collections-intro__meta">{visibleCountLabel}</span>
    </div>
  </section>

  <section class="collections-controls" aria-label={$currentLocale && t('collections.title')}>
    <div class="collections-controls__search">
      <SearchBar
        placeholder={$currentLocale && t('collections.searchPlaceholder')}
        clearAriaLabel={$currentLocale && t('collections.searchClear')}
        onsearch={(q) => (searchQuery = q)}
        onclear={() => (searchQuery = '')}
      />
    </div>
    <Button
      variant="primary"
      iconOnly={!showCreate}
      aria-label={showCreate ? undefined : $currentLocale && t('collections.createTitle')}
      title={showCreate ? undefined : $currentLocale && t('collections.createTitle')}
      onclick={() => (showCreate = !showCreate)}
    >
      {#if showCreate}
        {$currentLocale && t('collections.cancel')}
      {:else}
        <ActionIcon name="folder-plus" size={16} />
      {/if}
    </Button>
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
            locale={$currentLocale}
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
    <ConfirmDialog
      title={t('collections.deleteTitle')}
      message={t('collections.deleteMessage', { name: deletingName })}
      cancelLabel={t('collections.cancel')}
      confirmIcon="delete"
      confirmAriaLabel={t('collections.deleteAria')}
      confirmTitle={deleting ? t('collections.deletingTitle') : t('collections.deleteAria')}
      variant="destructive"
      confirming={deleting}
      cancelDisabled={deleting}
      confirmFirst
      oncancel={handleCancelDelete}
      onconfirm={handleConfirmDelete}
    />
  {/if}
</div>

<style>
  .collections-view {
    min-height: 100%;
  }

  .collections-intro__content {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 760px;
  }

  .collections-intro__eyebrow {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .collections-intro__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .collections-intro__copy p {
    max-width: 62ch;
  }

  .collections-intro__meta {
    display: inline-flex;
    align-items: center;
    width: fit-content;
    min-height: var(--control-height-sm);
    padding: 0 var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--color-surface-raised) 48%, transparent);
    color: color-mix(in srgb, var(--color-text-secondary) 86%, transparent);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    white-space: nowrap;
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 2%);
  }

  .collections-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) 0;
  }

  .collections-controls__search {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    min-width: min(100%, 360px);
    flex: 1 1 360px;
  }

  .collections-controls :global(.search-bar) {
    min-width: min(100%, 320px);
    max-width: 360px;
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

  @media (max-width: 720px) {
    .collections-controls {
      width: 100%;
      align-items: stretch;
    }

    .collections-controls__search {
      flex-direction: column;
      align-items: stretch;
      gap: var(--space-2);
    }

    .collections-controls :global(.search-bar),
    .collections-controls :global(.btn) {
      width: 100%;
      max-width: none;
    }

    .create-form__actions :global(.btn),
    .edit-form__actions :global(.btn) {
      width: 100%;
    }
  }
</style>
