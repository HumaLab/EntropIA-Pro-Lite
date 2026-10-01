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
  import { ConfirmDialog } from '@entropia/ui'
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
    onclose,
    onopenitem,
  }: {
    capture: { id: string; title: string }
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
      {$currentLocale && t('navegador.copy.intro', { title: capture.title })}
    </p>
    <fieldset class="copy-dialog__destination">
      <legend class="copy-dialog__legend">
        {$currentLocale && t('navegador.copy.destination')}
      </legend>

      {#if loadError}
        <p class="copy-dialog__problem" role="alert">{loadError}</p>
      {:else if loading}
        <p class="copy-dialog__status">{$currentLocale && t('navegador.copy.loading')}</p>
      {:else}
        <div class="copy-dialog__options">
          {#each collections as option (option.id)}
            <label class="copy-dialog__option">
              <input type="radio" name="copy-destination" value={option.id} bind:group={choice} />
              <span class="copy-dialog__option-name">{option.name}</span>
            </label>
          {/each}

          <label class="copy-dialog__option">
            <input
              type="radio"
              name="copy-destination"
              value={NEW_COLLECTION}
              bind:group={choice}
            />
            <span class="copy-dialog__option-name copy-dialog__option-name--fixed">
              {$currentLocale && t('navegador.copy.newCollection')}
            </span>
            <input
              type="text"
              class="copy-dialog__new-name"
              placeholder={$currentLocale && t('navegador.copy.newCollectionPlaceholder')}
              aria-label={$currentLocale && t('navegador.copy.newCollectionAriaLabel')}
              bind:value={newName}
              disabled={!isNew}
              onfocus={() => (choice = NEW_COLLECTION)}
            />
          </label>
        </div>
      {/if}
    </fieldset>
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
    color: var(--color-danger, var(--color-text-primary));
  }

  .copy-dialog__destination {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: none;
  }

  .copy-dialog__legend {
    padding: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .copy-dialog__options {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 240px;
    overflow-y: auto;
  }

  .copy-dialog__option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .copy-dialog__option:hover {
    background: var(--color-accent-faint);
  }

  .copy-dialog__option-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .copy-dialog__option-name--fixed {
    flex: 0 0 auto;
  }

  .copy-dialog__new-name {
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
  }

  .copy-dialog__new-name:focus,
  .copy-dialog__new-name:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .copy-dialog__new-name:disabled {
    cursor: not-allowed;
    opacity: 0.56;
  }
</style>
