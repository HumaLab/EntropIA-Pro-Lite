<script lang="ts">
  /**
   * Aliases tab of Configuración (entity aliases, T3, S8): every alias
   * (alias → canonical, sorted by canonical then alias), with edit (change
   * the canonical name) and delete behind a confirmation. Edits and deletes
   * refresh the shared resolver, like the dialog does, so the entity list
   * and Relaciones 3D show the change without restarting. Follows the
   * AgentParametersTab layout (Card, section copy, inline editing).
   */
  import { onMount } from 'svelte'
  import type { EntityAlias } from '@entropia/store'
  import { ActionIcon, Card, ConfirmDialog, Input, tooltip } from '@entropia/ui'
  import { getStore } from '$lib/db'
  import { describeEntityAliasError, refreshEntityAliases } from '$lib/entity-aliases'
  import { t } from '$lib/i18n'

  let aliases = $state<EntityAlias[]>([])
  let loading = $state(true)
  let loadError = $state<string | null>(null)
  let editingId = $state<string | null>(null)
  let editingCanonical = $state('')
  let saving = $state(false)
  let error = $state<string | null>(null)
  let pendingDelete = $state<EntityAlias | null>(null)
  let deleting = $state(false)

  const sorted = $derived(
    aliases
      .slice()
      .sort((a, b) => a.canonical.localeCompare(b.canonical) || a.alias.localeCompare(b.alias))
  )

  async function load(): Promise<void> {
    loading = true
    loadError = null
    try {
      aliases = await getStore().entityAliases.list()
    } catch (loadFailure) {
      const detail = loadFailure instanceof Error ? loadFailure.message : String(loadFailure ?? '')
      loadError = t('entityAliases.loadError', { error: detail })
    } finally {
      loading = false
    }
  }

  function startEditing(row: EntityAlias): void {
    error = null
    editingId = row.id
    editingCanonical = row.canonical
  }

  function cancelEditing(): void {
    editingId = null
    editingCanonical = ''
    if (!pendingDelete) error = null
  }

  async function saveEditing(row: EntityAlias): Promise<void> {
    if (saving) return
    saving = true
    error = null
    try {
      await getStore().entityAliases.upsert({
        alias: row.alias,
        canonical: editingCanonical.trim(),
      })
      await refreshEntityAliases()
      editingId = null
      editingCanonical = ''
      await load()
    } catch (saveFailure) {
      error = describeEntityAliasError(saveFailure, row.alias, editingCanonical.trim(), t)
    } finally {
      saving = false
    }
  }

  function requestDelete(row: EntityAlias): void {
    error = null
    pendingDelete = row
  }

  async function confirmDelete(): Promise<void> {
    const row = pendingDelete
    if (!row || deleting) return
    deleting = true
    try {
      await getStore().entityAliases.delete(row.id)
      await refreshEntityAliases()
      pendingDelete = null
      await load()
    } catch (deleteFailure) {
      pendingDelete = null
      error = describeEntityAliasError(deleteFailure, row.alias, row.canonical, t)
    } finally {
      deleting = false
    }
  }

  onMount(() => {
    void load()
  })
</script>

<Card>
  <section class="entity-aliases">
    <div class="entity-aliases__copy">
      <h2>{t('entityAliases.title')}</h2>
      <p>{t('entityAliases.description')}</p>
    </div>

    {#if error}
      <p class="surface-message surface-message--error" role="alert">{error}</p>
    {/if}

    {#if loadError}
      <p class="surface-message surface-message--error" role="alert">{loadError}</p>
    {:else if loading}
      <p class="entity-aliases__note">{t('entityAliases.loading')}</p>
    {:else if sorted.length === 0}
      <p class="entity-aliases__note">{t('entityAliases.empty')}</p>
    {:else}
      <ul class="entity-aliases__list">
        {#each sorted as row (row.id)}
          <li class="entity-aliases__row" data-testid={`entity-alias-row-${row.id}`}>
            {#if editingId === row.id}
              <div class="entity-aliases__edit">
                <span class="entity-aliases__alias">{row.alias}</span>
                <span class="entity-aliases__arrow" aria-hidden="true">→</span>
                <Input
                  label={t('entityAliases.dialog.canonicalLabel')}
                  bind:value={editingCanonical}
                />
                <div class="entity-aliases__edit-actions">
                  <button
                    type="button"
                    class="entity-aliases__action entity-aliases__action--primary"
                    disabled={saving}
                    onclick={() => void saveEditing(row)}
                  >
                    {t('entityAliases.save')}
                  </button>
                  <button
                    type="button"
                    class="entity-aliases__action"
                    disabled={saving}
                    onclick={cancelEditing}
                  >
                    {t('entityAliases.cancel')}
                  </button>
                </div>
              </div>
            {:else}
              <span class="entity-aliases__pair">
                <span class="entity-aliases__alias">{row.alias}</span>
                <span class="entity-aliases__arrow" aria-hidden="true">→</span>
                <span class="entity-aliases__canonical">{row.canonical}</span>
              </span>
              <span class="entity-aliases__row-actions">
                <button
                  type="button"
                  class="entity-aliases__icon-action"
                  aria-label={t('entityAliases.editAria', { alias: row.alias })}
                  use:tooltip={t('entityAliases.editAria', { alias: row.alias })}
                  data-testid={`entity-alias-edit-${row.id}`}
                  onclick={() => startEditing(row)}
                >
                  <ActionIcon name="edit" size={14} />
                </button>
                <button
                  type="button"
                  class="entity-aliases__icon-action entity-aliases__icon-action--delete"
                  aria-label={t('entityAliases.deleteAria', { alias: row.alias })}
                  use:tooltip={t('entityAliases.deleteAria', { alias: row.alias })}
                  data-testid={`entity-alias-delete-${row.id}`}
                  onclick={() => requestDelete(row)}
                >
                  <ActionIcon name="delete" size={14} />
                </button>
              </span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</Card>

{#if pendingDelete}
  <ConfirmDialog
    title={t('entityAliases.deleteTitle')}
    titleId="entity-alias-delete-title"
    message={t('entityAliases.deleteMessage', {
      alias: pendingDelete.alias,
      canonical: pendingDelete.canonical,
    })}
    cancelLabel={t('entityAliases.cancel')}
    confirmLabel={t('entityAliases.deleteConfirm')}
    variant="destructive"
    confirming={deleting}
    oncancel={() => (pendingDelete = null)}
    onconfirm={() => void confirmDelete()}
  />
{/if}

<style>
  .entity-aliases {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
  }

  .entity-aliases__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .entity-aliases__copy h2 {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-lg);
    color: var(--color-text-primary);
  }

  .entity-aliases__copy p {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .entity-aliases__note {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .entity-aliases__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .entity-aliases__row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
  }

  .entity-aliases__pair {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .entity-aliases__alias {
    color: var(--color-text-muted);
  }

  .entity-aliases__arrow {
    color: var(--color-text-muted);
  }

  .entity-aliases__canonical {
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .entity-aliases__row-actions {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    flex: none;
  }

  .entity-aliases__icon-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    height: 28px;
    padding: 0 var(--space-1);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .entity-aliases__icon-action:hover {
    border-color: var(--color-hairline);
    color: var(--color-text-primary);
  }

  .entity-aliases__icon-action--delete:hover {
    border-color: var(--color-danger);
    color: var(--color-danger);
  }

  .entity-aliases__edit {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
  }

  .entity-aliases__edit-actions {
    display: inline-flex;
    gap: var(--space-1);
    flex: none;
  }

  .entity-aliases__action {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font: inherit;
    font-size: var(--font-size-xs);
    cursor: pointer;
  }

  .entity-aliases__action--primary {
    border-color: var(--color-accent);
    color: var(--color-accent-hover);
  }

  .entity-aliases__action:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
