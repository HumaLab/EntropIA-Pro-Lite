<script lang="ts">
  /**
   * Alias dialog (entity aliases, T3, S8): «Es la misma persona que…» from an
   * item's entity list. The mention arrives as the alias, prefilled; the
   * owner picks or types the canonical name and confirms (S6: no automatic
   * merge). Saving writes only to `entity_aliases` through the repo (S2:
   * mentions are never edited) and refreshes the shared resolver, so the
   * entity list and Relaciones 3D show the change without restarting. Repo
   * validation errors show in plain language and leave the dialog open.
   */
  import { normalizeEntityAliasKey } from '@entropia/store'
  import { ConfirmDialog, Input } from '@entropia/ui'
  import { getStore } from '$lib/db'
  import { describeEntityAliasError, refreshEntityAliases } from '$lib/entity-aliases'
  import { t } from '$lib/i18n'

  interface Props {
    /** The mention's surface form: the alias, prefilled and never rewritten. */
    alias: string
    /** The item's other mention values, offered as canonical candidates. */
    suggestions?: string[]
    onclose: () => void
    /** Runs after the save and the refresh, so the opener can re-read. */
    onsaved?: () => void
  }

  let { alias, suggestions = [], onclose, onsaved }: Props = $props()

  let canonical = $state('')
  let saving = $state(false)
  let error = $state<string | null>(null)

  /** Other mentions, deduplicated and never the alias itself (normalized). */
  const candidates = $derived.by(() => {
    const key = normalizeEntityAliasKey(alias)
    const seen = new Set<string>()
    return suggestions.filter((suggestion) => {
      const candidate = suggestion.trim()
      if (!candidate) return false
      const candidateKey = normalizeEntityAliasKey(candidate)
      if (candidateKey === key || seen.has(candidateKey)) return false
      seen.add(candidateKey)
      return true
    })
  })

  async function save(): Promise<void> {
    if (saving) return
    saving = true
    error = null
    try {
      await getStore().entityAliases.upsert({ alias, canonical: canonical.trim() })
      await refreshEntityAliases()
      onsaved?.()
      onclose()
    } catch (saveError) {
      error = describeEntityAliasError(saveError, alias, canonical.trim(), t)
    } finally {
      saving = false
    }
  }
</script>

<ConfirmDialog
  title={t('entityAliases.dialog.title')}
  titleId="entity-alias-dialog-title"
  message={t('entityAliases.dialog.message')}
  cancelLabel={t('entityAliases.dialog.cancel')}
  confirmLabel={t('entityAliases.dialog.save')}
  dismissOnOverlay={false}
  confirming={saving}
  {error}
  oncancel={onclose}
  onconfirm={() => void save()}
>
  <div class="entity-alias-dialog__fields">
    <Input label={t('entityAliases.dialog.aliasLabel')} value={alias} disabled />
    <Input
      label={t('entityAliases.dialog.canonicalLabel')}
      placeholder={t('entityAliases.dialog.canonicalPlaceholder')}
      bind:value={canonical}
    />
    {#if candidates.length > 0}
      <div class="entity-alias-dialog__suggestions">
        <p class="entity-alias-dialog__suggestions-label">
          {t('entityAliases.dialog.suggestionsLabel')}
        </p>
        <p class="entity-alias-dialog__suggestions-hint">
          {t('entityAliases.dialog.suggestionsHint')}
        </p>
        <ul class="entity-alias-dialog__suggestions-list">
          {#each candidates as candidate (candidate)}
            <li>
              <button
                type="button"
                class="entity-alias-dialog__suggestion"
                onclick={() => {
                  canonical = candidate
                  error = null
                }}
              >
                {candidate}
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>
</ConfirmDialog>

<style>
  .entity-alias-dialog__fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .entity-alias-dialog__suggestions {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .entity-alias-dialog__suggestions-label {
    margin: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .entity-alias-dialog__suggestions-hint {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .entity-alias-dialog__suggestions-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: var(--space-1) 0 0;
    padding: 0;
    list-style: none;
  }

  .entity-alias-dialog__suggestion {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-xs);
    cursor: pointer;
  }

  .entity-alias-dialog__suggestion:hover {
    border-color: var(--color-accent);
    color: var(--color-accent-hover);
  }
</style>
