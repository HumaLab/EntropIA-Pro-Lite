<script lang="ts">
  import { onMount } from 'svelte'
  import { ActionIcon, ToolbarMenu, type ToolbarMenuItem } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { testOpenrouterConnection, type ModelInfo } from '$lib/settings'
  import { requestSettingsTab } from '$lib/settings-tab-request'
  import { workspace } from '$lib/workspace'

  /**
   * A model picker fed by the OpenRouter catalog, the same suggestions
   * Configuración shows. It loads the catalog with the saved credential;
   * without one, the field is replaced by a note pointing at Configuración.
   * No free-text input: the list is the only way to pick.
   */
  let {
    selected = '',
    onchoose,
    label,
    emptyLabel,
  }: {
    selected?: string
    onchoose: (id: string) => void
    /** Field label, shown next to the menu. */
    label: string
    /** The "no model of its own" option (the general model applies). */
    emptyLabel: string
  } = $props()

  const currentLocale = locale
  let models = $state<ModelInfo[]>([])
  let noKey = $state(false)
  const uid = `openrouter-model-${Math.random().toString(36).slice(2, 8)}`

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

  const items = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio',
      id: '',
      label: emptyLabel,
      checked: selected === '',
      onselect: () => onchoose(''),
    },
    ...suggested.map((m) => ({
      kind: 'radio' as const,
      id: m.id,
      label: `${m.id} · ${Math.round(m.context_length / 1024)}k`,
      checked: selected === m.id,
      onselect: () => onchoose(m.id),
    })),
  ])

  function openApiSettings() {
    requestSettingsTab('api')
    workspace.navigateActive({ name: 'settings' })
  }
</script>

<div class="model-picker">
  <span class="model-picker__label" id="{uid}-label">{label}</span>
  {#if noKey}
    <p class="model-picker__nokey">
      {$currentLocale && t('settings.modelList.noKey')}
      <button type="button" class="model-picker__link" onclick={openApiSettings}>
        {$currentLocale && t('settings.modelList.openSettings')}
      </button>
    </p>
  {:else}
    <ToolbarMenu {label} {items}>
      {#snippet trigger(props, { open })}
        <button
          type="button"
          class="menu-select"
          class:menu-select--open={open}
          aria-labelledby="{uid}-label {uid}-value"
          disabled={suggested.length === 0}
          {...props}
        >
          <span id="{uid}-value">{selected || emptyLabel}</span>
          <ActionIcon name="chevron-down" size={12} />
        </button>
      {/snippet}
    </ToolbarMenu>
  {/if}
</div>

<style>
  /* Same shape as the Input next to it (label above, control the width of
     the column, `control-height-md` tall), so swapping the text field for
     the menu moves nothing. */
  .model-picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1 1 0;
    min-width: 0;
  }

  .model-picker__label {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    user-select: none;
  }

  .model-picker__nokey {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .model-picker__link {
    border: none;
    background: transparent;
    padding: 0;
    font: inherit;
    color: var(--color-accent);
    text-decoration: underline;
    cursor: pointer;
  }

  /* Same trigger as the schema menu above it: the app's own menu, never a
     native <select>, whose popup the operating system paints. */
  .menu-select {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    min-height: var(--control-height-md);
    max-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    text-align: left;
    cursor: pointer;
  }

  .menu-select span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .menu-select:hover,
  .menu-select--open {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  .menu-select:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .menu-select:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
</style>
