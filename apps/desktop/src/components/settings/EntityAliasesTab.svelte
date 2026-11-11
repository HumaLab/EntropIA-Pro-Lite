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
  import { normalizeEntityAliasKey } from '@entropia/store'
  import { ActionIcon, Card, ConfirmDialog, Input, tooltip } from '@entropia/ui'
  import { getStore } from '$lib/db'
  import { describeEntityAliasError, refreshEntityAliases } from '$lib/entity-aliases'
  import { suggestEntityAliases, type EntityAliasSuggestion } from '$lib/entity-alias-suggestions'
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
  let suggestions = $state<EntityAliasSuggestion[]>([])
  let suggestionsLoading = $state(false)
  let suggestionsError = $state<string | null>(null)
  let dismissedKeys = $state<Set<string>>(new Set())
  let acceptingKey = $state<string | null>(null)

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
    await loadSuggestions()
  }

  /**
   * Loads rule-based suggestions (T5, S5) from the archive's distinct
   * person-type values. A store failure only empties the section, never the
   * alias list above it.
   */
  async function loadSuggestions(): Promise<void> {
    suggestionsLoading = true
    suggestionsError = null
    try {
      const values = await getStore().entities.listDistinctValues({ types: ['person'] })
      suggestions = suggestEntityAliases({ values, aliases, dismissedKeys })
    } catch (suggestionsFailure) {
      const detail =
        suggestionsFailure instanceof Error
          ? suggestionsFailure.message
          : String(suggestionsFailure ?? '')
      suggestionsError = t('entityAliases.suggestions.loadError', { error: detail })
      suggestions = []
    } finally {
      suggestionsLoading = false
    }
  }

  /**
   * Accepts one suggestion (S5): one alias row per variant. On any refusal
   * the readable message is shown and the suggestion stays, so the owner can
   * fix the canonical and retry from «Es la misma persona que…».
   */
  async function acceptSuggestion(suggestion: EntityAliasSuggestion): Promise<void> {
    const key = normalizeEntityAliasKey(suggestion.canonical)
    if (acceptingKey) return
    acceptingKey = key
    suggestionsError = null
    let failedVariant = suggestion.variants[0] ?? suggestion.canonical
    try {
      for (const variant of suggestion.variants) {
        failedVariant = variant
        await getStore().entityAliases.upsert({ alias: variant, canonical: suggestion.canonical })
      }
      await refreshEntityAliases()
      suggestions = suggestions.filter((item) => item !== suggestion)
      await load()
    } catch (acceptFailure) {
      suggestionsError = describeEntityAliasError(
        acceptFailure,
        failedVariant,
        suggestion.canonical,
        t
      )
    } finally {
      acceptingKey = null
    }
  }

  /** Hides a suggestion for this session only (S6: rejected, never merged). */
  function dismissSuggestion(suggestion: EntityAliasSuggestion): void {
    dismissedKeys.add(normalizeEntityAliasKey(suggestion.canonical))
    suggestions = suggestions.filter((item) => item !== suggestion)
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

<Card>
  <section class="entity-aliases" aria-labelledby="entity-alias-suggestions-title">
    <div class="entity-aliases__copy">
      <h2 id="entity-alias-suggestions-title">{t('entityAliases.suggestions.title')}</h2>
      <p>{t('entityAliases.suggestions.description')}</p>
    </div>

    {#if suggestionsError}
      <p class="surface-message surface-message--error" role="alert">{suggestionsError}</p>
    {/if}

    {#if suggestionsLoading}
      <p class="entity-aliases__note">{t('entityAliases.suggestions.loading')}</p>
    {:else if suggestions.length === 0}
      <p class="entity-aliases__note">{t('entityAliases.suggestions.empty')}</p>
    {:else}
      <ul class="entity-aliases__list">
        {#each suggestions as suggestion, index (normalizeEntityAliasKey(suggestion.canonical))}
          {@const suggestionKey = normalizeEntityAliasKey(suggestion.canonical)}
          <li class="entity-aliases__row" data-testid={`entity-alias-suggestion-${index}`}>
            <span class="entity-aliases__pair">
              <span class="entity-aliases__canonical">{suggestion.canonical}</span>
              <span class="entity-aliases__arrow" aria-hidden="true">←</span>
              <span class="entity-aliases__alias">{suggestion.variants.join(', ')}</span>
            </span>
            <span class="entity-aliases__note entity-aliases__meta">
              {t('entityAliases.suggestions.mentions', { count: suggestion.totalMentions })}
            </span>
            <span class="entity-aliases__row-actions">
              <button
                type="button"
                class="entity-aliases__icon-action entity-aliases__icon-action--primary"
                aria-label={t('entityAliases.suggestions.acceptAria', {
                  canonical: suggestion.canonical,
                })}
                use:tooltip={t('entityAliases.suggestions.acceptAria', {
                  canonical: suggestion.canonical,
                })}
                data-testid={`entity-alias-suggestion-accept-${index}`}
                disabled={acceptingKey !== null}
                onclick={() => void acceptSuggestion(suggestion)}
              >
                <ActionIcon name="check" size={14} />
              </button>
              <button
                type="button"
                class="entity-aliases__icon-action"
                aria-label={t('entityAliases.suggestions.discardAria', {
                  canonical: suggestion.canonical,
                })}
                use:tooltip={t('entityAliases.suggestions.discardAria', {
                  canonical: suggestion.canonical,
                })}
                data-testid={`entity-alias-suggestion-discard-${index}`}
                disabled={acceptingKey !== null}
                onclick={() => dismissSuggestion(suggestion)}
              >
                <ActionIcon name="close" size={14} />
              </button>
            </span>
            {#if acceptingKey === suggestionKey}
              <span class="entity-aliases__note" role="status">
                {t('entityAliases.suggestions.accepting')}
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

  .entity-aliases__meta {
    flex: none;
    margin-left: auto;
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

  .entity-aliases__icon-action--primary {
    color: var(--color-accent-hover);
  }

  .entity-aliases__icon-action--primary:hover {
    border-color: var(--color-accent);
    color: var(--color-accent-hover);
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
