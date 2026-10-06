<script lang="ts">
  import {
    ActionIcon,
    Button,
    IconButton,
    Input,
    ToolbarMenu,
    type ToolbarMenuItem,
  } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { settingsGet, settingsSet } from '$lib/settings'
  import {
    BATCH_FLOWS_SETTING,
    parseFlows,
    upsertFlow,
    type BatchFlow,
    type BatchFlowSteps,
  } from '$lib/batch-flows'

  /**
   * Saved batch flows (T-52), above the operation toggles: choosing one ticks
   * its operations and schema; "Guardar como flujo" remembers the current
   * ticks under a name.
   */

  interface Props {
    steps: BatchFlowSteps
    onapply: (steps: BatchFlowSteps) => void
  }

  let { steps, onapply }: Props = $props()

  let flows = $state<BatchFlow[]>([])
  let chosen = $state('')
  let naming = $state(false)
  let name = $state('')
  let error = $state('')

  const items = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio',
      id: '',
      label: t('batch.flowNone'),
      checked: chosen === '',
      onselect: () => choose(''),
    },
    ...flows.map((flow) => ({
      kind: 'radio' as const,
      id: flow.name,
      label: flow.name,
      checked: chosen === flow.name,
      onselect: () => choose(flow.name),
    })),
  ])

  async function load() {
    try {
      flows = parseFlows(await settingsGet(BATCH_FLOWS_SETTING))
    } catch {
      flows = []
    }
  }

  $effect(() => {
    load()
  })

  async function persist(next: BatchFlow[]) {
    try {
      await settingsSet(BATCH_FLOWS_SETTING, JSON.stringify(next))
      flows = next
      error = ''
    } catch (failure) {
      error = String(failure)
    }
  }

  function choose(value: string) {
    chosen = value
    const flow = flows.find((entry) => entry.name === value)
    if (flow) onapply({ ...flow.steps })
  }

  async function save() {
    const trimmed = name.trim()
    if (!trimmed) return
    await persist(upsertFlow(flows, { name: trimmed, steps: { ...steps } }))
    chosen = trimmed
    naming = false
    name = ''
  }

  async function remove() {
    if (!chosen) return
    await persist(flows.filter((flow) => flow.name !== chosen))
    chosen = ''
  }
</script>

<div class="flow-picker">
  <span class="batch-field__legend" id="batch-flow-label">{t('batch.flow')}</span>
  <ToolbarMenu label={t('batch.flow')} {items}>
    {#snippet trigger(props, { open })}
      <button
        type="button"
        class="menu-select"
        class:menu-select--open={open}
        aria-labelledby="batch-flow-label batch-flow-value"
        {...props}
      >
        <span id="batch-flow-value">{chosen || t('batch.flowNone')}</span>
        <ActionIcon name="chevron-down" size={12} />
      </button>
    {/snippet}
  </ToolbarMenu>
  {#if chosen}
    <IconButton size="sm" variant="ghost" label={t('batch.flowDelete')} onclick={remove}>
      <ActionIcon name="delete" size={14} />
    </IconButton>
  {/if}
  {#if naming}
    <Input bind:value={name} label={t('batch.flowName')} />
    <Button size="sm" disabled={!name.trim()} onclick={save}>{t('batch.flowSave')}</Button>
    <Button variant="ghost" size="sm" onclick={() => (naming = false)}>
      {t('batch.schemaCancel')}
    </Button>
  {:else}
    <Button variant="ghost" size="sm" onclick={() => (naming = true)}>
      <ActionIcon name="save" size={14} />
      {t('batch.flowSaveAs')}
    </Button>
  {/if}
  {#if error}<p class="flow-picker__error" role="alert">{error}</p>{/if}
</div>

<style>
  .flow-picker {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2);
  }

  /* Same trigger as the batch tab's state filter: the app's own menu, never a
     native <select>, whose popup the operating system paints. */
  .menu-select {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    cursor: pointer;
  }

  .menu-select:hover,
  .menu-select--open {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  .menu-select:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .flow-picker__error {
    flex-basis: 100%;
    color: var(--color-danger, #c0392b);
  }
</style>
