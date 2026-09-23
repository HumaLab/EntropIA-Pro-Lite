<script lang="ts">
  /**
   * "Importar fuentes" dialog (T4, home-view): choose an existing collection
   * or create a new one, then run the same file picker and import pipeline
   * CollectionView uses (`$lib/collection-import`), and navigate to the
   * result. Opened from Inicio's header action and its first-run block.
   *
   * The destination collection is only created after files were actually
   * picked (see `handleConfirm`): cancelling the OS file picker leaves no
   * empty collection behind.
   */
  import { onMount } from 'svelte'
  import { getStore } from '$lib/db'
  import { locale, t } from '$lib/i18n'
  import { navigation } from '$lib/navigation'
  import { pickFiles } from '$lib/file-import'
  import {
    importClassifiedPathsIntoCollection,
    buildImportSummary,
    type ImportProgress,
    type ImportSummary,
  } from '$lib/collection-import'
  import { notifyDocumentExplorerCollectionChanged } from '$lib/document-explorer'
  import { ConfirmDialog } from '@entropia/ui'
  import type { Collection } from '@entropia/store'

  let { onClose }: { onClose: () => void } = $props()

  const NEW_COLLECTION = '__new__'

  const currentLocale = locale
  const NUMBER_LOCALE = { es: 'es-AR', en: 'en-US' } as const
  const numberFormatter = $derived.by(() => new Intl.NumberFormat(NUMBER_LOCALE[$currentLocale]))

  let collections = $state<Array<Collection & { count: number }>>([])
  let loading = $state(true)
  let loadError = $state<string | null>(null)

  let destinationChoice = $state<string>('')
  let newCollectionName = $state('')
  let confirming = $state(false)
  let confirmError = $state<string | null>(null)

  /**
   * 'choosing': picking a destination. 'importing': files were picked and the
   * engine is running — Cancel/Escape are disabled (see `handleCancel`) since
   * there is nothing safe to undo mid-import. 'summary': the import finished
   * with something the user should see (a rejection, a duplicate, a per-file
   * error, or nothing classified at all) — reusing CollectionView's own
   * import-summary copy, so it reads the same in both places.
   */
  let phase = $state<'choosing' | 'importing' | 'summary'>('choosing')
  let importProgress = $state<ImportProgress | null>(null)
  let summary = $state<ImportSummary | null>(null)
  let targetCollectionId = $state<string | null>(null)
  let targetCollectionName = $state<string | null>(null)

  const isNewCollection = $derived(destinationChoice === NEW_COLLECTION)
  const isValidDestination = $derived(
    destinationChoice !== '' && (!isNewCollection || newCollectionName.trim() !== '')
  )

  async function loadCollections() {
    loading = true
    loadError = null
    try {
      const store = getStore()
      const found = await store.collections.findAll()
      const withCounts: Array<Collection & { count: number }> = []
      for (const collection of found) {
        withCounts.push({ ...collection, count: await store.collections.countItems(collection.id) })
      }
      collections = withCounts
    } catch (e) {
      loadError = e instanceof Error ? e.message : t('home.importDialog.errorLoad')
    } finally {
      loading = false
    }
  }

  onMount(() => {
    void loadCollections()
  })

  function formatCount(value: number): string {
    return numberFormatter.format(value)
  }

  function itemCountLabel(count: number): string {
    return count === 1
      ? t('home.recent.itemCount.one', { count })
      : t('home.recent.itemCount.other', { count })
  }

  function focusNewCollection() {
    destinationChoice = NEW_COLLECTION
  }

  async function handleConfirm() {
    if (!isValidDestination || confirming) return
    confirming = true
    confirmError = null

    try {
      const paths = await pickFiles()
      if (paths.length === 0) {
        confirming = false
        return
      }

      let targetId: string
      let targetName: string
      if (isNewCollection) {
        const store = getStore()
        const created = await store.collections.create({
          name: newCollectionName.trim(),
          description: null,
        })
        targetId = created.id
        targetName = created.name
      } else {
        const collection = collections.find((c) => c.id === destinationChoice)
        if (!collection) {
          confirming = false
          return
        }
        targetId = collection.id
        targetName = collection.name
      }
      targetCollectionId = targetId
      targetCollectionName = targetName

      phase = 'importing'
      importProgress = null

      const result = await importClassifiedPathsIntoCollection(paths, targetId, {
        baseErrorMessage: t('home.importDialog.title'),
        onProgress: (progress) => {
          importProgress = progress
        },
      })

      if (result.createdItems.length > 0) {
        notifyDocumentExplorerCollectionChanged(targetId)
      }

      const hasIssues =
        result.classifiedCount === 0 ||
        result.rejected.length > 0 ||
        result.importErrors.length > 0 ||
        result.alreadyImported.length > 0

      if (!hasIssues) {
        onClose()
        navigation.navigate({ name: 'collection', id: targetId, collectionName: targetName })
        return
      }

      // Something needs the user's attention (a rejection, a duplicate, a
      // per-file error, or nothing classified at all): stay open and let
      // them choose, instead of silently navigating away from it.
      summary = buildImportSummary(result)
      phase = 'summary'
    } catch (e) {
      confirmError = e instanceof Error ? e.message : String(e)
      phase = 'choosing'
    } finally {
      confirming = false
    }
  }

  /** Guarded so Cancel/Escape/overlay-click are inert while an import runs. */
  function handleCancel() {
    if (confirming) return
    onClose()
  }

  function handleGoToCollection() {
    if (!targetCollectionId || !targetCollectionName) return
    onClose()
    navigation.navigate({
      name: 'collection',
      id: targetCollectionId,
      collectionName: targetCollectionName,
    })
  }
</script>

<ConfirmDialog
  title={t('home.importDialog.title')}
  titleId="import-sources-dialog-title"
  cancelLabel={phase === 'summary' ? t('home.importDialog.close') : t('home.importDialog.cancel')}
  cancelDisabled={phase === 'importing'}
  dismissOnOverlay={false}
  confirmLabel={phase === 'summary'
    ? t('home.importDialog.goToCollection')
    : t('home.importDialog.confirm')}
  confirmDisabled={phase === 'summary' ? false : !isValidDestination || loading}
  confirming={phase === 'importing'}
  error={confirmError}
  oncancel={phase === 'summary' ? onClose : handleCancel}
  onconfirm={phase === 'summary' ? handleGoToCollection : handleConfirm}
>
  {#if phase === 'importing'}
    <div class="import-dialog__progress" role="status" aria-live="polite">
      <p class="import-dialog__progress-title">{t('collection.importSummary.importingTitle')}</p>
      {#if importProgress}
        <progress
          value={importProgress.completed}
          max={importProgress.total}
          aria-label={t('collection.importSummary.progressBar')}
        ></progress>
        <p class="import-dialog__progress-detail">
          {t('collection.importSummary.progressDescription', {
            completed: importProgress.completed,
            total: importProgress.total,
          })}
        </p>
      {/if}
    </div>
  {:else if phase === 'summary' && summary}
    <div class="import-dialog__summary">
      <p>
        {#if summary.errors.length > 0 || summary.rejected.length > 0}
          {t('collection.importSummary.partialFailure')}
        {:else if summary.alreadyImported.length > 0}
          {t('collection.importSummary.alreadyInCollection')}
        {:else}
          {t('collection.importSummary.reviewCollection')}
        {/if}
      </p>
      <dl class="import-dialog__summary-counts">
        <div>
          <dt>{t('collection.importSummary.imported')}</dt>
          <dd>{summary.imported}</dd>
        </div>
        <div>
          <dt>{t('collection.importSummary.skipped')}</dt>
          <dd>{summary.skipped}</dd>
        </div>
        <div>
          <dt>{t('collection.importSummary.errors')}</dt>
          <dd>{summary.errors.length}</dd>
        </div>
      </dl>
      {#if summary.rejected.length > 0}
        <p class="import-dialog__summary-detail">
          {t('collection.importSummary.skippedFiles', { files: summary.rejected.join(', ') })}
        </p>
      {/if}
      {#if summary.alreadyImported.length > 0}
        <p class="import-dialog__summary-detail">
          {t('collection.importSummary.alreadyImported', {
            files: summary.alreadyImported.join(', '),
          })}
        </p>
      {/if}
      {#if summary.errors.length > 0}
        <ul class="import-dialog__summary-errors">
          {#each summary.errors as importErrorLine, index (index)}
            <li class="import-dialog__summary-detail">{importErrorLine}</li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else}
    <fieldset class="import-dialog__destination">
      <legend class="import-dialog__legend">{t('home.importDialog.destination')}</legend>

      {#if loadError}
        <p class="surface-message surface-message--error" role="alert">{loadError}</p>
      {:else}
        <div class="import-dialog__options">
          {#each collections as collection (collection.id)}
            <label class="import-dialog__option">
              <input
                type="radio"
                name="import-destination"
                value={collection.id}
                bind:group={destinationChoice}
              />
              <span class="import-dialog__option-name">{collection.name}</span>
              <span class="import-dialog__option-count" aria-hidden="true"
                >{formatCount(collection.count)}</span
              >
              <span class="sr-only">{itemCountLabel(collection.count)}</span>
            </label>
          {/each}

          <label class="import-dialog__option import-dialog__option--new">
            <input
              type="radio"
              name="import-destination"
              value={NEW_COLLECTION}
              bind:group={destinationChoice}
            />
            <span class="import-dialog__option-name">{t('home.importDialog.newCollection')}</span>
            <input
              type="text"
              class="import-dialog__new-name"
              placeholder={t('home.importDialog.newCollectionPlaceholder')}
              aria-label={t('home.importDialog.newCollectionAriaLabel')}
              bind:value={newCollectionName}
              disabled={!isNewCollection}
              onfocus={focusNewCollection}
            />
          </label>
        </div>
      {/if}
    </fieldset>
  {/if}
</ConfirmDialog>

<style>
  .import-dialog__progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .import-dialog__progress-title {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .import-dialog__progress-detail {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .import-dialog__summary {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .import-dialog__summary-counts {
    display: flex;
    gap: var(--space-4);
  }

  .import-dialog__summary-counts dt {
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.075em;
    color: var(--color-text-muted);
  }

  .import-dialog__summary-counts dd {
    margin: 0;
    font-family: var(--font-reading);
    font-size: var(--font-size-lg);
    color: var(--color-text-primary);
  }

  .import-dialog__summary-detail {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .import-dialog__summary-errors {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 160px;
    overflow-y: auto;
    list-style: none;
  }

  .import-dialog__destination {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: none;
  }

  .import-dialog__legend {
    padding: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .import-dialog__options {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 240px;
    overflow-y: auto;
  }

  .import-dialog__option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2);
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .import-dialog__option:hover {
    background: var(--color-accent-faint);
  }

  .import-dialog__option-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .import-dialog__option-count {
    flex: 0 0 auto;
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
    color: var(--color-text-muted);
  }

  .import-dialog__option--new .import-dialog__option-name {
    flex: 0 0 auto;
  }

  .import-dialog__new-name {
    flex: 1;
    min-width: 0;
    height: var(--control-height-sm);
    padding: 0 var(--space-2);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    outline: none;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth);
  }

  .import-dialog__new-name:focus,
  .import-dialog__new-name:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .import-dialog__new-name:disabled {
    cursor: not-allowed;
    opacity: 0.56;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
</style>
