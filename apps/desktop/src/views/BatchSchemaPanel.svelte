<script lang="ts">
  import { save } from '@tauri-apps/plugin-dialog'
  import { writeFile } from '@tauri-apps/plugin-fs'
  import {
    ActionIcon,
    Button,
    Checkbox,
    IconButton,
    Input,
    ToolbarMenu,
    type ToolbarMenuItem,
  } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import {
    cellText,
    deleteSchema,
    listRecords,
    listSchemas,
    recordsCsv,
    saveSchema,
    type ExtractionRecordRow,
    type ExtractionSchema,
  } from '$lib/extraction-schemas'

  /**
   * Extraction with a user-defined schema (T-51), under the batch operations.
   *
   * Pick the schema the next batch runs (or none), write a new one, and look
   * at what earlier batches extracted with it, as a table or a CSV file.
   */

  interface Props {
    /** The schema the next batch runs; '' runs none. */
    schemaId: string
  }

  let { schemaId = $bindable('') }: Props = $props()

  let schemas = $state<ExtractionSchema[]>([])
  let editing = $state<ExtractionSchema | null>(null)
  let records = $state<ExtractionRecordRow[] | null>(null)
  let error = $state('')
  let busy = $state(false)

  const current = $derived(schemas.find((schema) => schema.id === schemaId) ?? null)

  const schemaItems = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio',
      id: '',
      label: t('batch.schemaNone'),
      checked: schemaId === '',
      onselect: () => (schemaId = ''),
    },
    ...schemas.map((schema) => ({
      kind: 'radio' as const,
      id: schema.id,
      label: schema.name,
      checked: schemaId === schema.id,
      onselect: () => (schemaId = schema.id),
    })),
  ])

  async function reload() {
    try {
      schemas = await listSchemas()
      if (schemaId && !schemas.some((schema) => schema.id === schemaId)) schemaId = ''
    } catch (failure) {
      error = String(failure)
    }
  }

  $effect(() => {
    reload()
  })

  // A different schema shows its own records, after "Ver resultados".
  $effect(() => {
    void schemaId
    records = null
  })

  function blankSchema(): ExtractionSchema {
    return {
      id: '',
      name: '',
      model: '',
      fields: [{ name: '', description: '', repeatable: false }],
    }
  }

  async function run(action: () => Promise<void>) {
    if (busy) return
    busy = true
    error = ''
    try {
      await action()
    } catch (failure) {
      error = failure instanceof Error ? failure.message : String(failure)
    } finally {
      busy = false
    }
  }

  function saveEditing() {
    const draft = editing
    if (!draft) return
    return run(async () => {
      const saved = await saveSchema({
        ...draft,
        fields: draft.fields.filter((field) => field.name.trim()),
      })
      editing = null
      await reload()
      schemaId = saved.id
    })
  }

  function removeCurrent() {
    const target = current
    if (!target) return
    return run(async () => {
      await deleteSchema(target.id)
      schemaId = ''
      await reload()
    })
  }

  function showRecords() {
    const target = current
    if (!target) return
    return run(async () => {
      records = await listRecords(target.id)
    })
  }

  function downloadCsv() {
    const target = current
    if (!target || !records) return
    const rows = records
    return run(async () => {
      const path = await save({
        defaultPath: `${target.name}.csv`,
        filters: [{ name: 'CSV', extensions: ['csv'] }],
      })
      if (!path) return
      const csv = recordsCsv(target, rows, t('batch.schemaDocument'))
      await writeFile(path, new TextEncoder().encode(csv))
    })
  }
</script>

<div class="schema-panel" role="group" aria-labelledby="schema-panel-label">
  <div class="schema-panel__row">
    <span class="batch-field__legend" id="schema-panel-label">{t('batch.schema')}</span>
    <ToolbarMenu label={t('batch.schema')} items={schemaItems}>
      {#snippet trigger(props, { open })}
        <button
          type="button"
          class="menu-select"
          class:menu-select--open={open}
          aria-labelledby="schema-panel-label schema-panel-value"
          disabled={busy}
          {...props}
        >
          <span id="schema-panel-value">{current?.name ?? t('batch.schemaNone')}</span>
          <ActionIcon name="chevron-down" size={12} />
        </button>
      {/snippet}
    </ToolbarMenu>
    <Button variant="ghost" size="sm" disabled={busy} onclick={() => (editing = blankSchema())}>
      <ActionIcon name="add" size={14} />
      {t('batch.schemaNew')}
    </Button>
    {#if current}
      <Button
        variant="ghost"
        size="sm"
        disabled={busy}
        onclick={() => (editing = structuredClone($state.snapshot(current)))}
      >
        <ActionIcon name="edit" size={14} />
        {t('batch.schemaEdit')}
      </Button>
      <Button variant="ghost" size="sm" disabled={busy} onclick={showRecords}>
        <ActionIcon name="table" size={14} />
        {t('batch.schemaResults')}
      </Button>
      <IconButton
        size="sm"
        variant="ghost"
        label={t('batch.schemaDelete')}
        disabled={busy}
        onclick={removeCurrent}
      >
        <ActionIcon name="delete" size={14} />
      </IconButton>
    {/if}
  </div>
  <p class="schema-panel__hint">{t('batch.schemaHint')}</p>

  {#if editing}
    <div class="schema-panel__editor">
      <div class="schema-panel__row">
        <Input bind:value={editing.name} label={t('batch.schemaName')} />
        <Input
          bind:value={editing.model}
          label={t('batch.schemaModel')}
          placeholder={t('batch.schemaModelPlaceholder')}
          hint={t('batch.schemaModelHint')}
        />
      </div>
      {#each editing.fields as field, index (index)}
        <div class="schema-panel__field">
          <Input bind:value={field.name} label={t('batch.schemaFieldName')} />
          <Input bind:value={field.description} label={t('batch.schemaFieldDescription')} />
          <Checkbox bind:checked={field.repeatable}>{t('batch.schemaFieldRepeatable')}</Checkbox>
          <IconButton
            size="sm"
            variant="ghost"
            label={t('batch.schemaFieldRemove')}
            onclick={() => editing && editing.fields.splice(index, 1)}
          >
            <ActionIcon name="close" size={14} />
          </IconButton>
        </div>
      {/each}
      <div class="schema-panel__row">
        <Button
          variant="ghost"
          size="sm"
          onclick={() =>
            editing && editing.fields.push({ name: '', description: '', repeatable: false })}
        >
          <ActionIcon name="add" size={14} />
          {t('batch.schemaFieldAdd')}
        </Button>
        <Button size="sm" disabled={busy || !editing.name.trim()} onclick={saveEditing}>
          {t('batch.schemaSave')}
        </Button>
        <Button variant="ghost" size="sm" onclick={() => (editing = null)}>
          {t('batch.schemaCancel')}
        </Button>
      </div>
    </div>
  {/if}

  {#if records && current}
    {#if records.length === 0}
      <p class="schema-panel__hint">{t('batch.schemaNoResults')}</p>
    {:else}
      <div class="schema-panel__row">
        <span>{t('batch.schemaResultCount', { count: String(records.length) })}</span>
        <Button variant="ghost" size="sm" disabled={busy} onclick={downloadCsv}>
          <ActionIcon name="download" size={14} />
          {t('batch.schemaDownload')}
        </Button>
      </div>
      <div class="schema-panel__table">
        <table>
          <thead>
            <tr>
              <th>{t('batch.schemaDocument')}</th>
              {#each current.fields as field (field.name)}<th>{field.name}</th>{/each}
            </tr>
          </thead>
          <tbody>
            {#each records as row, index (index)}
              <tr>
                <td>{row.item_title}</td>
                {#each current.fields as field (field.name)}
                  <td>{cellText(row.record[field.name])}</td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/if}

  {#if error}<p class="schema-panel__error" role="alert">{error}</p>{/if}
</div>

<style>
  .schema-panel {
    display: grid;
    gap: var(--space-2);
  }

  .schema-panel__row,
  .schema-panel__field {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2);
  }

  .schema-panel__editor {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--color-border, rgba(127, 127, 127, 0.3));
    border-radius: var(--radius-sm);
  }

  .schema-panel__hint {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: 0.9em;
  }

  .schema-panel__table {
    max-height: 320px;
    overflow: auto;
  }

  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 0.9em;
  }

  th,
  td {
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--color-border, rgba(127, 127, 127, 0.3));
    text-align: left;
    vertical-align: top;
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

  .schema-panel__error {
    color: var(--color-danger, #c0392b);
  }
</style>
