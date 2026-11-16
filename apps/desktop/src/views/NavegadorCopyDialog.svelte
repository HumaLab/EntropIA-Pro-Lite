<script lang="ts">
  /**
   * "Copiar a colección" for a saved PDF capture: choose an existing collection
   * (or create one), then make an independent corpus item from the saved file.
   *
   * Nothing here is a default or a move: the person picks the destination, the
   * web source stays as it is and the copy lives on its own. When the chosen
   * collection already holds a copy of this capture the dialog asks before it
   * makes a second one. The collection is only created when the copy starts, and
   * is removed again (if still empty) when that copy fails.
   *
   * The work is `$lib/navegador-copy`; this component only holds the choice, the
   * question and the result. The file is named by capture id and found by Rust.
   */
  import { onMount } from 'svelte'
  import { ConfirmDialog, Input, RadioCard, RadioGroup } from '@entropia/ui'
  import type { Collection } from '@entropia/store'
  import { getStore } from '$lib/db'
  import { locale, t } from '$lib/i18n'
  import { notifyDocumentExplorerCollectionChanged } from '$lib/document-explorer'
  import { CopyError, copyCaptureToCollection, findExistingCopy } from '$lib/navegador-copy'

  /** Where the new document is, for the caller to open it. */
  export type CopiedTarget = {
    collectionId: string
    collectionName: string
    itemId: string
    itemTitle: string
  }

  let {
    capture,
    rendered = false,
    onclose,
    onopenitem,
  }: {
    capture: { id: string; title: string }
    /** A page or a selection: the copy is a PDF rendered from its text. */
    rendered?: boolean
    onclose: () => void
    onopenitem: (target: CopiedTarget) => void
  } = $props()

  const NEW_COLLECTION = '__new__'
  const currentLocale = locale

  let collections = $state<Collection[]>([])
  let loading = $state(true)
  let loadError = $state<string | null>(null)
  let choice = $state('')
  let newName = $state('')
  let phase = $state<'choosing' | 'confirmAgain' | 'copying' | 'done'>('choosing')
  let existing = $state<{ id: string; title: string } | null>(null)
  let problem = $state<string | null>(null)
  let target = $state<CopiedTarget | null>(null)

  const isNew = $derived(choice === NEW_COLLECTION)
  const chosenName = $derived(
    isNew ? newName.trim() : (collections.find((c) => c.id === choice)?.name ?? '')
  )
  const valid = $derived(choice !== '' && (!isNew || newName.trim() !== ''))

  onMount(async () => {
    try {
      collections = await getStore().collections.findAll()
    } catch (reason) {
      loadError = t('navegador.copy.loadError', {
        message: reason instanceof Error ? reason.message : String(reason),
      })
    } finally {
      loading = false
    }
  })

  function describeFailure(reason: unknown): string {
    if (reason instanceof CopyError) {
      return t(`navegador.copy.error.${reason.code}`, { message: reason.detail ?? '' })
    }
    return t('navegador.copy.error.unknown', {
      message: reason instanceof Error ? reason.message : String(reason),
    })
  }

  async function copy() {
    phase = 'copying'
    problem = null
    let createdId: string | null = null
    try {
      let collectionId = choice
      let collectionName = chosenName
      if (isNew) {
        const created = await getStore().collections.create({
          name: newName.trim(),
          description: null,
        })
        createdId = created.id
        collectionId = created.id
        collectionName = created.name
      }
      const copied = await copyCaptureToCollection({ captureId: capture.id, collectionId })
      notifyDocumentExplorerCollectionChanged(collectionId, copied.item.id)
      target = {
        collectionId,
        collectionName,
        itemId: copied.item.id,
        itemTitle: copied.item.title,
      }
      phase = 'done'
    } catch (reason) {
      if (createdId) {
        // A collection made only for this copy must not outlive its failure.
        await getStore()
          .collections.deleteIfEmpty(createdId)
          .catch(() => false)
      }
      problem = describeFailure(reason)
      phase = 'choosing'
    }
  }

  async function confirm() {
    if (phase === 'done') {
      if (target) onopenitem(target)
      onclose()
      return
    }
    if (phase === 'confirmAgain') {
      await copy()
      return
    }
    if (!valid || phase !== 'choosing') return
    if (!isNew) {
      const earlier = await findExistingCopy(choice, capture.id)
      if (earlier) {
        existing = earlier
        phase = 'confirmAgain'
        return
      }
    }
    await copy()
  }

  function cancel() {
    if (phase === 'copying') return
    onclose()
  }

  const confirmLabel = $derived(
    $currentLocale &&
      t(
        phase === 'done'
          ? 'navegador.copy.open'
          : phase === 'confirmAgain'
            ? 'navegador.copy.again'
            : 'navegador.copy.confirm'
      )
  )
</script>

<ConfirmDialog
  title={$currentLocale && t('navegador.copy.title')}
  cancelLabel={$currentLocale &&
    t(phase === 'done' ? 'navegador.copy.close' : 'navegador.copy.cancel')}
  cancelDisabled={phase === 'copying'}
  dismissOnOverlay={false}
  {confirmLabel}
  confirmDisabled={phase === 'choosing' ? !valid || loading : false}
  confirming={phase === 'copying'}
  error={problem}
  oncancel={cancel}
  onconfirm={() => void confirm()}
>
  {#if phase === 'copying'}
    <p class="copy-dialog__status" role="status" aria-live="polite">
      {$currentLocale && t('navegador.copy.copying')}
    </p>
  {:else if phase === 'done' && target}
    <p class="copy-dialog__status" role="status" aria-live="polite">
      {$currentLocale &&
        t('navegador.copy.done', { collection: target.collectionName, title: target.itemTitle })}
    </p>
  {:else if phase === 'confirmAgain' && existing}
    <p class="copy-dialog__status" role="alert">
      {$currentLocale &&
        t('navegador.copy.already', { collection: chosenName, title: existing.title })}
    </p>
  {:else}
    <p class="copy-dialog__intro">
      {$currentLocale &&
        t(rendered ? 'navegador.copy.introText' : 'navegador.copy.intro', { title: capture.title })}
    </p>
    {#if loadError}
      <p class="copy-dialog__problem" role="alert">{loadError}</p>
    {:else if loading}
      <p class="copy-dialog__status">{$currentLocale && t('navegador.copy.loading')}</p>
    {:else}
      <div class="copy-dialog__options">
        <RadioGroup
          variant="list"
          name="copy-destination"
          label={$currentLocale && t('navegador.copy.destination')}
          bind:value={choice}
        >
          {#each collections as option (option.id)}
            <RadioCard value={option.id} label={option.name} />
          {/each}

          <RadioCard
            value={NEW_COLLECTION}
            label={$currentLocale && t('navegador.copy.newCollection')}
          >
            {#snippet embedded()}
              <Input
                size="sm"
                placeholder={$currentLocale && t('navegador.copy.newCollectionPlaceholder')}
                aria-label={$currentLocale && t('navegador.copy.newCollectionAriaLabel')}
                bind:value={newName}
              />
            {/snippet}
          </RadioCard>
        </RadioGroup>
      </div>
    {/if}
  {/if}
</ConfirmDialog>

<style>
  .copy-dialog__intro,
  .copy-dialog__status {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    overflow-wrap: anywhere;
  }

  .copy-dialog__problem {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-danger);
  }

  /* A long list scrolls inside the dialog; the padding keeps the options'
     focus ring clear of the scroll box's edge. */
  .copy-dialog__options {
    max-height: 240px;
    padding: var(--space-0-5);
    overflow-y: auto;
  }
</style>
