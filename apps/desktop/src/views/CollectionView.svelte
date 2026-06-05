<script lang="ts">
  import { getStore } from '$lib/db'
  import { navigation } from '$lib/navigation'
  import { locale, t } from '$lib/i18n'
  import { getFocusableElements, getNextFocusTrapTarget } from '$lib/modal-focus'
  import {
    pickFiles,
    classifyFiles,
    importSingleFile,
    isScannedPdf,
    renderPdfPages,
    type ImportedFile,
  } from '$lib/file-import'
  import {
    getAssetUrl,
    deleteAssetFile,
    deletePdfThumbnail,
  } from '$lib/file-import'
  import { appDataDir, join } from '@tauri-apps/api/path'
  import { stat } from '@tauri-apps/plugin-fs'
  import { exportCollectionById } from '$lib/export'
  import { ItemCard, SearchBar, Button } from '@entropia/ui'
  import { onMount, onDestroy } from 'svelte'
  import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview'
  import { listen, type UnlistenFn } from '@tauri-apps/api/event'
  import type { Item, Asset } from '@entropia/store'

  let { collectionId }: { collectionId: string } = $props()

  let items = $state<Item[]>([])
  let searchQuery = $state('')
  let loading = $state(true)
  let error = $state<string | null>(null)
  let importing = $state(false)
  let exporting = $state(false)
  let importNotice = $state<string | null>(null)
  let dragActive = $state(false)
  let unlistenDragDrop: (() => void) | null = null
  let unlistenAssetUpdate: (() => void) | null = null
  const currentLocale = locale

  let visibleCountLabel = $derived.by(() => {
    $currentLocale
    return items.length === 1
      ? t('collection.visibleCount.one', { count: items.length })
      : t('collection.visibleCount.other', { count: items.length })
  })

  let collectionTitle = $derived.by(() => {
    $currentLocale
    return navigation.current.name === 'collection'
      ? navigation.current.collectionName
      : t('collection.documentsFallback')
  })

  // Cache itemId → { assetCount, thumbnailUrl, primaryAssetId, primaryAssetPath, primaryAssetType }
  let itemAssetMeta = $state<
    Map<
      string,
      {
        assetCount: number
        thumbnailUrl: string | null
        primaryAssetId: string | null
        primaryAssetPath: string | null
        primaryAssetType: string | null
      }
    >
  >(new Map())

  // Delete confirmation state
  let showDeleteConfirm = $state(false)
  let pendingDeleteAssetId = $state<string | null>(null)
  let pendingDeleteItemId = $state<string | null>(null)
  let pendingDeleteFilename = $state<string | null>(null)
  let deleting = $state(false)
  let deleteError = $state<string | null>(null)
  let deleteDialogEl: HTMLElement | undefined = $state()
  let deleteTriggerEl: HTMLElement | null = null

  function getItemAssetMeta(itemId: string): {
    assetCount: number
    thumbnailUrl: string | null
    primaryAssetId: string | null
    primaryAssetPath: string | null
    primaryAssetType: string | null
  } {
    return (
      itemAssetMeta.get(itemId) ?? {
        assetCount: 0,
        thumbnailUrl: null,
        primaryAssetId: null,
        primaryAssetPath: null,
        primaryAssetType: null,
      }
    )
  }

  async function loadItemAssets(itemIds: string[]) {
    if (itemIds.length === 0) return
    const store = getStore()
    const newMeta = new Map(itemAssetMeta)
    for (const itemId of itemIds) {
      try {
        const assets: Asset[] = await store.assets.findByItem(itemId)
        const imageAsset = assets.find((a) => a.type === 'image')
        // For PDFs, keep exploration lightweight: ItemCard shows the PDF icon.
        const pdfAsset = assets.find((a) => a.type === 'pdf')

        let thumbnailUrl: string | null = null
        let primaryAssetType: string | null = null

        if (imageAsset) {
          thumbnailUrl = getAssetUrl(imageAsset.path)
          primaryAssetType = imageAsset.type
        } else if (pdfAsset) {
          thumbnailUrl = null
          primaryAssetType = pdfAsset.type
        } else {
          const thumbAsset = assets[0]
          const isAudio = thumbAsset?.type === 'audio'
          thumbnailUrl = !isAudio && thumbAsset ? getAssetUrl(thumbAsset.path) : null
          primaryAssetType = thumbAsset?.type ?? null
        }

        newMeta.set(itemId, {
          assetCount: assets.length,
          thumbnailUrl,
          primaryAssetId: imageAsset?.id ?? pdfAsset?.id ?? assets[0]?.id ?? null,
          primaryAssetPath: imageAsset?.path ?? pdfAsset?.path ?? assets[0]?.path ?? null,
          primaryAssetType,
        })
      } catch (e) {
        console.error('[CollectionView] Failed to load assets for item', itemId, e)
        // Non-fatal: item card shows placeholder
      }
    }
    itemAssetMeta = newMeta
  }

  let filtered = $derived(
    searchQuery ? items : items // search is handled by repo call below
  )

  async function loadItems() {
    try {
      loading = true
      error = null
      const store = getStore()
      items = searchQuery
        ? await store.items.searchByText(collectionId, searchQuery)
        : await store.items.findByCollection(collectionId)
      // Load asset metadata (count + thumbnail) for each item
      await loadItemAssets(items.map((i) => i.id))
    } catch (e) {
      error = e instanceof Error ? e.message : t('collection.error.load')
    } finally {
      loading = false
    }
  }

  async function handleSearch(query: string) {
    searchQuery = query
    await loadItems()
  }

  async function handleClearSearch() {
    searchQuery = ''
    await loadItems()
  }

  async function finalizeImportedItem(itemId: string, imported: ImportedFile) {
    const store = getStore()

    // For scanned PDFs, convert to per-page image assets instead of a single PDF asset
    if (imported.type === 'pdf') {
      try {
        const isScanned = await isScannedPdf(imported.destPath)
        if (isScanned) {
          const pages = await convertScannedPdfToPages(imported, collectionId, itemId, store)
          if (pages.length > 0) {
            // Delete the original PDF file — we only keep the page images
            try {
              await deleteAssetFile(imported.destPath)
            } catch (e) {
              console.warn('[CollectionView] Failed to delete original scanned PDF:', e)
            }
            return // Pages created, no PDF asset needed
          }
          // If page conversion failed, fall through to create a regular PDF asset
        }
      } catch (e) {
        console.warn('[CollectionView] Scanned PDF detection failed, treating as text PDF:', e)
        // Fall through to create a regular PDF asset
      }
    }

    // Default: create a single asset for the imported file
    await store.assets.create({
      itemId,
      path: imported.destPath,
      type: imported.type,
      size: imported.size,
      sortIndex: 0,
    })
  }

  const IMPORTED_FILE_METADATA_KEY = '__entropia_file_metadata'

  function buildImportedItemMetadata(imported: ImportedFile): string {
    return JSON.stringify({
      [IMPORTED_FILE_METADATA_KEY]: imported.originalMetadata,
    })
  }

  async function readAssetSize(path: string): Promise<number | null> {
    try {
      const metadata = await stat(path)
      const size = Number(metadata.size ?? 0)
      return Number.isFinite(size) ? size : null
    } catch (e) {
      console.warn('[CollectionView] Failed to read rendered page size:', e)
      return null
    }
  }

  /**
   * Convert a scanned PDF to per-page PNG image assets.
   * Returns the list of created asset IDs, or empty array on failure.
   */
  async function convertScannedPdfToPages(
    imported: ImportedFile,
    collId: string,
    itemId: string,
    store: ReturnType<typeof getStore>
  ): Promise<string[]> {
    try {
      const dataDir = await appDataDir()
      const outputDir = await join(dataDir, 'assets', collId, itemId)

      // Render all PDF pages as PNGs using the backend command
      const baseName = imported.originalName.replace(/\.[^.]+$/, '')
      const pages = await renderPdfPages(imported.destPath, outputDir, baseName)

      // Create an image asset for each page, with sort_index for ordering
      const assetIds: string[] = []
      for (const page of pages) {
        const asset = await store.assets.create({
          itemId,
          path: page.png_path,
          type: 'image',
          sortIndex: page.page_number - 1, // 0-indexed
          size: await readAssetSize(page.png_path),
        })
        assetIds.push(asset.id)
      }

      console.log(`[CollectionView] Converted scanned PDF to ${pages.length} page assets`)
      return assetIds
    } catch (e) {
      console.error('[CollectionView] Failed to convert scanned PDF to pages:', e)
      return []
    }
  }

  function getErrorDetails(e: unknown): string {
    return e instanceof Error ? e.message : String(e)
  }

  function formatImportStageError(baseMessage: string, stage: string, e: unknown): string {
    return `${baseMessage} (${stage}): ${getErrorDetails(e)}`
  }

  async function handleImport() {
    importing = true
    error = null
    importNotice = null
    const store = getStore()

    // Step 1: Open file picker — get raw paths BEFORE creating any items
    let selectedPaths: string[]
    try {
      selectedPaths = await pickFiles()
    } catch (e) {
      error = formatImportStageError('Failed to import files', 'selecting files', e)
      importing = false
      return
    }

    if (selectedPaths.length === 0) {
      importing = false
      return
    }

    // Step 2: Classify files (no DB or FS side effects yet)
    const { classified, rejected } = classifyFiles(selectedPaths)

    if (classified.length === 0) {
      if (rejected.length > 0) {
        error = `Unsupported format: ${rejected.join(', ')}`
      }
      importing = false
      return
    }

    // Step 3: Create one item per file, copy file, create asset
    const createdItemIds: string[] = []
    let importError: string | null = null

    for (const file of classified) {
      let itemId: string
      try {
        const item = await store.items.create({
          title: file.name.replace(/\.[^.]+$/, ''),
          collectionId,
          metadata: null,
        })
        itemId = item.id
      } catch (e) {
        importError = formatImportStageError('Failed to import files', 'creating item', e)
        break
      }

      try {
        const imported = await importSingleFile(file.sourcePath, collectionId, itemId)
        await store.items.update(itemId, { metadata: buildImportedItemMetadata(imported) })
        await finalizeImportedItem(itemId, imported)
        createdItemIds.push(itemId)
      } catch (e) {
        // Clean up the item if file copy failed
        try {
          await store.items.delete(itemId)
        } catch {
          // ignore cleanup errors
        }
        importError = formatImportStageError('Failed to import files', `importing ${file.name}`, e)
        break
      }
    }

    await loadItems()

    // Navigate to the last created item
    if (createdItemIds.length > 0) {
      const lastItemId = createdItemIds[createdItemIds.length - 1]!
      const lastFile = classified[classified.length - 1]!
      navigation.navigate({
        name: 'item',
        collectionId,
        collectionName:
          navigation.current.name === 'collection'
            ? (navigation.current as { collectionName: string }).collectionName
            : '',
        itemId: lastItemId,
        itemTitle: lastFile.name.replace(/\.[^.]+$/, ''),
      })
    }

    // Build notice for partial imports
    const noticeParts: string[] = []
    if (rejected.length > 0) {
      noticeParts.push(`${rejected.length} unsupported skipped: ${rejected.join(', ')}`)
    }
    if (importError) {
      noticeParts.push(`error: ${importError}`)
    }
    importNotice =
      noticeParts.length > 0
        ? `${createdItemIds.length} imported (${noticeParts.join(' · ')})`
        : null

    if (importError && createdItemIds.length === 0) {
      error = importError
    }

    importing = false
  }

  async function handleImportFromDroppedPaths(paths: string[]) {
    importing = true
    error = null
    importNotice = null
    const store = getStore()

    // Step 1: Classify dropped files (no DB or FS side effects yet)
    const { classified, rejected } = classifyFiles(paths)

    if (classified.length === 0) {
      if (rejected.length > 0) {
        error = `Unsupported format: ${rejected.join(', ')}`
      }
      importing = false
      dragActive = false
      return
    }

    // Step 2: Create one item per file, copy file, create asset
    const createdItemIds: string[] = []
    let importError: string | null = null

    for (const file of classified) {
      let itemId: string
      try {
        const item = await store.items.create({
          title: file.name.replace(/\.[^.]+$/, ''),
          collectionId,
          metadata: null,
        })
        itemId = item.id
      } catch (e) {
        importError = formatImportStageError('Failed to import dropped files', 'creating item', e)
        break
      }

      try {
        const imported = await importSingleFile(file.sourcePath, collectionId, itemId)
        await store.items.update(itemId, { metadata: buildImportedItemMetadata(imported) })
        await finalizeImportedItem(itemId, imported)
        createdItemIds.push(itemId)
      } catch (e) {
        try {
          await store.items.delete(itemId)
        } catch {
          // ignore cleanup errors
        }
        importError = formatImportStageError(
          'Failed to import dropped files',
          `importing ${file.name}`,
          e
        )
        break
      }
    }

    await loadItems()

    // Navigate to the last created item
    if (createdItemIds.length > 0) {
      const lastItemId = createdItemIds[createdItemIds.length - 1]!
      const lastFile = classified[classified.length - 1]!
      navigation.navigate({
        name: 'item',
        collectionId,
        collectionName:
          navigation.current.name === 'collection'
            ? (navigation.current as { collectionName: string }).collectionName
            : '',
        itemId: lastItemId,
        itemTitle: lastFile.name.replace(/\.[^.]+$/, ''),
      })
    }

    // Build notice
    const noticeParts: string[] = []
    if (rejected.length > 0) {
      noticeParts.push(`${rejected.length} unsupported skipped: ${rejected.join(', ')}`)
    }
    if (importError) {
      noticeParts.push(`error: ${importError}`)
    }
    importNotice =
      noticeParts.length > 0
        ? `${createdItemIds.length} imported (${noticeParts.join(' · ')})`
        : null

    if (importError && createdItemIds.length === 0) {
      error = importError
    }

    importing = false
    dragActive = false
  }

  async function handleExportJson() {
    try {
      exporting = true
      error = null
      const store = getStore()
      await exportCollectionById(store, collectionId)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to export collection'
    } finally {
      exporting = false
    }
  }

  // ---------------------------------------------------------------------------
  // Asset deletion flow
  // ---------------------------------------------------------------------------

  /**
   * Extract just the filename from a full native path.
   */
  function extractFilename(nativePath: string): string {
    return nativePath.split(/[/\\]/).pop() ?? t('collection.unknownFile')
  }

  /**
   * Open the delete confirmation dialog for the primary asset of an item.
   */
  function handleDeleteClick(itemId: string) {
    const meta = getItemAssetMeta(itemId)
    if (!meta.primaryAssetId || !meta.primaryAssetPath) {
      error = t('collection.error.noAssetToDelete')
      return
    }
    deleteTriggerEl = document.activeElement instanceof HTMLElement ? document.activeElement : null
    pendingDeleteAssetId = meta.primaryAssetId
    pendingDeleteItemId = itemId
    pendingDeleteFilename = extractFilename(meta.primaryAssetPath)
    showDeleteConfirm = true
    deleteError = null
  }

  /**
   * Cancel the delete confirmation dialog.
   */
  function handleDeleteCancel() {
    showDeleteConfirm = false
    pendingDeleteAssetId = null
    pendingDeleteItemId = null
    pendingDeleteFilename = null
    deleteError = null
    deleteTriggerEl?.focus()
    deleteTriggerEl = null
  }

  function handleDeleteDialogKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      handleDeleteCancel()
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

  /**
   * Execute the asset deletion: remove file from FS, then cascade delete from DB.
   * If the deleted asset is the item's last one, the entire item is removed
   * (with all associated metadata) and the card disappears from the grid.
   *
   * Resilient: DB errors do NOT block file deletion or UI update.
   * The file is always removed and the UI is always refreshed.
   */
  async function handleDeleteConfirm() {
    if (!pendingDeleteAssetId || !pendingDeleteItemId) return

    deleting = true
    deleteError = null

    const store = getStore()
    const meta = getItemAssetMeta(pendingDeleteItemId)
    const assetPath = meta.primaryAssetPath
    const isLastAsset = meta.assetCount <= 1

    // Step 1: Always delete the file from filesystem (ENOENT is OK)
    // Use the cached path — do NOT depend on a DB lookup
    if (assetPath) {
      try {
        await deleteAssetFile(assetPath)
      } catch (e) {
        // Log but continue — file deletion should not block UI update
        console.warn('[CollectionView] File deletion warning:', e)
      }
    }

    // Step 2: Try DB cleanup — non-blocking
    try {
      if (isLastAsset) {
        await store.items.deleteWithCascade(pendingDeleteItemId)
      } else {
        await store.assets.deleteWithCascade(pendingDeleteAssetId)
      }
    } catch (e) {
      // Log DB error but do NOT block UI update
      const message = e instanceof Error ? e.message : String(e)
      console.error('[CollectionView] DB cleanup failed (UI will still update):', message)
      // Show a subtle warning in the error field but still close the dialog
      deleteError = t('collection.error.fileRemovedDbFailed', { message })
    }

    // Step 2b: Clean up cached PDF thumbnail if the asset was a PDF
    if (meta.primaryAssetType === 'pdf' && pendingDeleteAssetId) {
      try {
        await deletePdfThumbnail(pendingDeleteAssetId)
      } catch (e) {
        console.warn('[CollectionView] Failed to delete PDF thumbnail:', e)
        // Non-fatal: thumbnail cache cleanup is best-effort
      }
    }

    // Step 3: Always update UI — remove card or refresh meta
    if (isLastAsset) {
      items = items.filter((i) => i.id !== pendingDeleteItemId)
      const newMeta = new Map(itemAssetMeta)
      newMeta.delete(pendingDeleteItemId)
      itemAssetMeta = newMeta
    } else {
      await loadItemAssets([pendingDeleteItemId])
    }

    // Step 4: Close dialog (even if DB failed — file is gone, UI is updated)
    deleteTriggerEl = null
    handleDeleteCancel()
    deleting = false
  }

  $effect(() => {
    if (!showDeleteConfirm || !deleteDialogEl) return

    setTimeout(() => {
      getFocusableElements(deleteDialogEl ?? null)[0]?.focus()
    }, 0)
  })

  onMount(() => {
    loadItems()

    getCurrentWebview()
      .onDragDropEvent((event: { payload: DragDropEvent }) => {
        if (event.payload.type === 'enter') {
          dragActive = true
          return
        }

        if (event.payload.type === 'over') {
          dragActive = true
          return
        }

        if (event.payload.type === 'leave') {
          dragActive = false
          return
        }

        if (event.payload.type !== 'drop') {
          return
        }

        dragActive = false
        void handleImportFromDroppedPaths(event.payload.paths)
      })
      .then((unlisten: () => void) => {
        unlistenDragDrop = unlisten
      })

    // Listen for asset image updates from ItemView (crop, rotate, erase, undo).
    // When an image is edited, the asset path changes to a new versioned file.
    // We must invalidate the cached thumbnail URL so the card shows the latest
    // version instead of a stale browser-cached image.
    listen<{ itemId: string; assetId: string; path: string }>('asset:image-updated', (event) => {
      const { itemId: updatedItemId } = event.payload
      // Invalidate the cached metadata for this item so the thumbnail
      // is regenerated with the new path (which includes a cache-busting
      // version number since edits create new files).
      void loadItemAssets([updatedItemId])
    })
      .then((unlisten) => {
        unlistenAssetUpdate = unlisten
      })
      .catch((e: unknown) => {
        console.warn('[CollectionView] Failed to subscribe to asset:image-updated:', e)
      })
  })

  onDestroy(() => {
    unlistenDragDrop?.()
    unlistenAssetUpdate?.()
  })
</script>

<div class="collection-view page-shell" class:drag-active={dragActive}>
  <section class="page-header collection-view__header">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('collection.active')}</span>
      <h1>{collectionTitle}</h1>
      <p>{$currentLocale && t('collection.subtitle')}</p>
      <span class="page-header__meta">{visibleCountLabel}</span>
    </div>

    <div class="page-toolbar collection-toolbar">
      <SearchBar
        placeholder={$currentLocale && t('collection.searchPlaceholder')}
        onsearch={handleSearch}
        onclear={handleClearSearch}
      />
      <Button variant="primary" onclick={handleImport} disabled={importing}>
        {importing
          ? $currentLocale && t('collection.importing')
          : $currentLocale && t('collection.import')}
      </Button>
      <Button variant="secondary" onclick={handleExportJson} disabled={exporting}>
        {exporting
          ? $currentLocale && t('collection.exporting')
          : $currentLocale && t('collection.export')}
      </Button>
    </div>
  </section>

  {#if error}
    <p class="surface-message surface-message--error">{error}</p>
  {/if}

  {#if importNotice}
    <p class="surface-message surface-message--success">{importNotice}</p>
  {/if}

  {#if dragActive}
    <div class="drop-hint">{t('collection.dropHint')}</div>
  {/if}

  {#if loading}
    <p class="surface-message surface-message--center">{t('collection.loading')}</p>
  {:else if items.length === 0}
    <div class="surface-message surface-message--center empty">
      <p>
        {searchQuery ? t('collection.emptySearch') : t('collection.empty')}
      </p>
    </div>
  {:else}
    <div class="grid">
      {#each items as item (item.id)}
        {@const meta = getItemAssetMeta(item.id)}
        <ItemCard
          id={item.id}
          title={item.title}
          assetCount={meta.assetCount}
          thumbnailPath={meta.thumbnailUrl ?? undefined}
          primaryAssetType={(meta.primaryAssetType as 'image' | 'pdf' | 'audio' | undefined) ??
            undefined}
          onclick={() =>
            navigation.navigate({
              name: 'item',
              collectionId,
              collectionName:
                navigation.current.name === 'collection'
                  ? (navigation.current as { collectionName: string }).collectionName
                  : '',
              itemId: item.id,
              itemTitle: item.title,
            })}
          onDelete={() => handleDeleteClick(item.id)}
        />
      {/each}
    </div>
  {/if}

  <!-- Delete confirmation modal -->
  {#if showDeleteConfirm}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="modal-overlay" onclick={handleDeleteCancel} role="presentation">
      <div
        bind:this={deleteDialogEl}
        class="modal"
        tabindex="-1"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="delete-modal-title"
        aria-describedby="delete-modal-description"
        onclick={(e) => e.stopPropagation()}
        onkeydown={handleDeleteDialogKeydown}
      >
        <h3 id="delete-modal-title" class="modal-title">{t('collection.deleteAssetTitle')}</h3>
        <p id="delete-modal-description" class="modal-message">
          {t('collection.deleteAssetMessage', { name: pendingDeleteFilename ?? '' })}
        </p>

        {#if deleteError}
          <p class="modal-error">{deleteError}</p>
        {/if}

        <div class="modal-actions">
          <Button variant="secondary" onclick={handleDeleteCancel} disabled={deleting}>
            {t('collections.cancel')}
          </Button>
          <button
            type="button"
            class="modal-delete-button"
            aria-label={t('collection.deleteAssetAria')}
            title={deleting ? t('collection.deletingAssetTitle') : t('collection.deleteAssetAria')}
            aria-busy={deleting}
            onclick={handleDeleteConfirm}
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
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .collection-view {
    min-height: 100%;
  }

  .collection-view__header {
    align-items: center;
  }

  .collection-toolbar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex: 1;
  }

  .collection-toolbar :global(.search-bar) {
    min-width: min(100%, 340px);
    flex: 1 1 280px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-3);
  }

  .empty {
    min-height: 220px;
  }

  .drop-hint {
    padding: var(--space-4);
    border: 1px dashed color-mix(in srgb, var(--color-accent) 44%, transparent);
    border-radius: var(--radius-lg);
    color: var(--color-text-secondary);
    text-align: center;
    background:
      linear-gradient(180deg, var(--color-accent-faint), transparent),
      var(--color-surface-sunken);
  }

  .collection-view.drag-active {
    outline: 1px dashed var(--color-primary);
    outline-offset: 6px;
    border-radius: var(--radius-md);
  }

  /* Delete confirmation modal */
  .modal-overlay {
    position: fixed;
    inset: 0;
    background-color: var(--color-overlay);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: var(--space-4);
  }

  .modal {
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent 58%),
      var(--color-surface-glass);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-lg);
    padding: var(--space-6);
    max-width: 420px;
    width: 100%;
    box-shadow: var(--shadow-lg);
  }

  .modal-title {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
    margin: 0 0 var(--space-3) 0;
  }

  .modal-message {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    margin: 0 0 var(--space-4) 0;
    line-height: 1.5;
  }

  .modal-error {
    font-size: var(--font-size-sm);
    color: var(--color-danger);
    margin: 0 0 var(--space-4) 0;
    padding: var(--space-2) var(--space-3);
    background-color: var(--color-danger-soft);
    border-radius: var(--radius-sm);
  }

  .modal-actions {
    display: flex;
    gap: var(--space-3);
    justify-content: flex-end;
  }

  .modal-delete-button {
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

  .modal-delete-button:hover:not(:disabled) {
    background-color: var(--color-danger-hover);
    border-color: var(--color-danger-hover);
    transform: translateY(-1px);
  }

  .modal-delete-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .modal-delete-button:disabled {
    opacity: 0.48;
    cursor: not-allowed;
    transform: none;
  }

  @media (max-width: 720px) {
    .collection-toolbar {
      width: 100%;
      justify-content: stretch;
    }

    .collection-toolbar :global(.search-bar),
    .collection-toolbar :global(.btn),
    .modal-actions :global(.btn) {
      width: 100%;
    }

    .modal-actions {
      flex-direction: column-reverse;
    }
  }
</style>
