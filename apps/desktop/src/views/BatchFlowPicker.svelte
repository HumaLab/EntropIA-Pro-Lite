<script lang="ts">
  import { ActionIcon, Button, IconButton, Input } from '@entropia/ui'
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
  <label class="batch-field__legend" for="batch-flow-select">{t('batch.flow')}</label>
  <select
    id="batch-flow-select"
    value={chosen}
    onchange={(event) => choose(event.currentTarget.value)}
  >
    <option value="">{t('batch.flowNone')}</option>
    {#each flows as flow (flow.name)}
      <option value={flow.name}>{flow.name}</option>
    {/each}
  </select>
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

  .flow-picker__error {
    flex-basis: 100%;
    color: var(--color-danger, #c0392b);
  }
</style>
