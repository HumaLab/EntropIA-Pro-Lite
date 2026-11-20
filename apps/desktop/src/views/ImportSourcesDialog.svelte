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
  import { getNavigation } from '$lib/pane-context'
  import { pickFiles } from '$lib/file-import'
  import {
    importClassifiedPathsIntoCollection,
    buildImportSummary,
    type ImportProgress,
    type ImportSummary,
  } from '$lib/collection-import'
  import { notifyDocumentExplorerCollectionChanged } from '$lib/document-explorer'
  import { ConfirmDialog, Button, Input, ProgressBar, RadioCard, RadioGroup } from '@entropia/ui'
  import type { Collection } from '@entropia/store'

  const navigation = getNavigation()

  let {
    onClose,
    onOpenInternetArchive,
  }: { onClose: () => void; onOpenInternetArchive?: () => void } = $props()

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

  /**
   * Hands over to the Internet Archive dialog: the parent owns that dialog,
   * so this one closes itself first and then asks the parent to open it.
   * The action is only rendered when the parent passed the callback (26b).
   */
  function handleOpenInternetArchive() {
    onClose()
    onOpenInternetArchive?.()
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
        <ProgressBar
          value={importProgress.completed}
          max={importProgress.total}
          label={t('collection.importSummary.progressBar')}
        />
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
    {#if loadError}
      <p class="surface-message surface-message--error" role="alert">{loadError}</p>
    {:else}
      <div class="import-dialog__options">
        <RadioGroup
          variant="list"
          name="import-destination"
          label={t('home.importDialog.destination')}
          bind:value={destinationChoice}
        >
          {#each collections as collection (collection.id)}
            <RadioCard value={collection.id} label={collection.name}>
              {#snippet description()}
                <span class="import-dialog__option-count" aria-hidden="true">
                  {formatCount(collection.count)}
                </span>
                <span class="sr-only">{itemCountLabel(collection.count)}</span>
              {/snippet}
            </RadioCard>
          {/each}

          <RadioCard value={NEW_COLLECTION} label={t('home.importDialog.newCollection')}>
            {#snippet embedded()}
              <Input
                size="sm"
                placeholder={t('home.importDialog.newCollectionPlaceholder')}
                aria-label={t('home.importDialog.newCollectionAriaLabel')}
                bind:value={newCollectionName}
              />
            {/snippet}
          </RadioCard>
        </RadioGroup>
      </div>
    {/if}
    {#if onOpenInternetArchive}
      <div class="import-dialog__ia">
        <Button variant="secondary" onclick={handleOpenInternetArchive}>
          {t('home.importDialog.fromInternetArchive')}
        </Button>
      </div>
    {/if}
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
    letter-spacing: var(--letter-spacing-caps);
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

  /* A long list scrolls inside the dialog; the padding keeps the options'
     focus ring clear of the scroll box's edge. */
  .import-dialog__options {
    max-height: 240px;
    padding: var(--space-0-5);
    overflow-y: auto;
  }

  .import-dialog__option-count {
    font-variant-numeric: tabular-nums;
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

  .import-dialog__ia {
    display: flex;
    justify-content: flex-start;
    margin-top: var(--space-2);
  }
</style>
