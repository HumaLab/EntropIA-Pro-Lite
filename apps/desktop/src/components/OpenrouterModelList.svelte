<script lang="ts">
  import { onMount } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import { testOpenrouterConnection, type ModelInfo } from '$lib/settings'
  import { requestSettingsTab } from '$lib/settings-tab-request'
  import { workspace } from '$lib/workspace'

  /**
   * The suggested OpenRouter models, the same list Configuración shows. It
   * loads the catalog with the saved credential; without one, or offline,
   * nothing shows and the model field stays editable by hand.
   */
  let { selected = '', onchoose }: { selected?: string; onchoose: (id: string) => void } = $props()

  const currentLocale = locale
  let models = $state<ModelInfo[]>([])
  /** No saved credential: the list cannot load, so say where to add one. */
  let noKey = $state(false)

  onMount(() => {
    let alive = true
    testOpenrouterConnection('')
      .then((list) => {
        if (alive) models = Array.isArray(list) ? list : []
      })
      .catch((err: unknown) => {
        if (alive) noKey = String(err).includes('No protected credential')
      })
    return () => {
      alive = false
    }
  })

  const suggested = $derived(
    models
      .filter((m) =>
        ['gemma', 'llama', 'mistral', 'qwen', 'claude', 'gpt'].some((k) => m.id.includes(k))
      )
      .slice(0, 15)
  )

  function openApiSettings() {
    requestSettingsTab('api')
    workspace.navigateActive({ name: 'settings' })
  }
</script>

{#if noKey}
  <p class="settings__model-list-nokey">
    {$currentLocale && t('settings.modelList.noKey')}
    <button type="button" class="settings__model-list-link" onclick={openApiSettings}>
      {$currentLocale && t('settings.modelList.openSettings')}
    </button>
  </p>
{:else if suggested.length > 0}
  <div class="settings__model-list">
    <p class="settings__model-list-title">
      {$currentLocale && t('settings.suggestedModels')}
    </p>
    {#each suggested as m (m.id)}
      <button
        class="settings__model-option"
        type="button"
        class:selected={selected === m.id}
        onclick={() => onchoose(m.id)}
      >
        <span class="settings__model-id">{m.id}</span>
        <span class="settings__model-ctx">{Math.round(m.context_length / 1024)}k ctx</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  /* The same classes and tokens as Configuración (`settings__model-*` in
     SettingsView), so both lists look the same. */
  .settings__model-list {
    max-height: 240px;
    overflow-y: auto;
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-surface);
    background: color-mix(in srgb, var(--color-surface-glass) 72%, transparent);
  }

  .settings__model-list-nokey {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }
  .settings__model-list-link {
    border: none;
    background: transparent;
    padding: 0;
    font: inherit;
    color: var(--color-accent);
    text-decoration: underline;
    cursor: pointer;
  }

  .settings__model-list-title {
    padding: var(--space-2) var(--space-3);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    border-bottom: 1px solid color-mix(in srgb, var(--color-hairline) 72%, transparent);
  }
  .settings__model-option {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    text-align: left;
    transition: background-color var(--transition-smooth);
  }
  .settings__model-option:hover {
    background: color-mix(in srgb, var(--color-surface-glass) 82%, transparent);
  }

  .settings__model-option.selected {
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface-glass));
    font-weight: var(--font-weight-medium);
  }

  .settings__model-option + .settings__model-option {
    border-top: 1px solid var(--color-border-subtle);
  }

  .settings__model-id {
    color: var(--color-text-primary);
  }

  .settings__model-ctx {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }
</style>
