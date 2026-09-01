<script lang="ts">
  import { ActionIcon, Button } from '../Button'
  import type { MetadataEditorProps } from './MetadataEditor.types'

  type MetadataEditorResolvedLabels = NonNullable<MetadataEditorProps['labels']> & {
    keyPlaceholder: string
    valuePlaceholder: string
    removeFieldAria: string
    addField: string
    fieldLabel: string
    valueLabel: string
    emptyText: string
  }

  let { value, onchange, labels: labelsProp = {} }: MetadataEditorProps = $props()

  const defaultLabels: MetadataEditorResolvedLabels = {
    keyPlaceholder: 'Key',
    valuePlaceholder: 'Value',
    removeFieldAria: 'Remove field',
    addField: 'Add field',
    fieldLabel: 'Field',
    valueLabel: 'Value',
    emptyText: '',
  }

  const labels = $derived.by(
    (): MetadataEditorResolvedLabels => ({ ...defaultLabels, ...labelsProp })
  )

  interface MetadataRow {
    key: string
    value: string
  }

  function valueToRows(nextValue: Record<string, string> | undefined): MetadataRow[] {
    return nextValue ? Object.entries(nextValue).map(([key, val]) => ({ key, value: val })) : []
  }

  function rowsToValue(nextRows: MetadataRow[]): Record<string, string> {
    const result: Record<string, string> = {}
    for (const row of nextRows) {
      if (row.key || row.value) {
        result[row.key] = row.value
      }
    }
    return result
  }

  function signatureFromValue(nextValue: Record<string, string> | undefined): string {
    if (!nextValue) {
      return ''
    }
    const orderedEntries = Object.entries(nextValue).sort(([a], [b]) => a.localeCompare(b))
    return JSON.stringify(orderedEntries)
  }

  let rows = $state<MetadataRow[]>([])
  let lastExternalSignature = $state<string | undefined>(undefined)

  $effect(() => {
    const incomingSignature = signatureFromValue(value)
    if (lastExternalSignature === undefined) {
      lastExternalSignature = incomingSignature
      rows = valueToRows(value)
      return
    }

    if (incomingSignature === lastExternalSignature) {
      return
    }

    lastExternalSignature = incomingSignature
    rows = valueToRows(value)
  })

  function serializeRows(): Record<string, string> {
    return rowsToValue(rows)
  }

  function handleKeyInput(index: number, e: Event) {
    const target = e.target as HTMLInputElement
    rows[index]!.key = target.value
    onchange?.(serializeRows())
  }

  function handleValueInput(index: number, e: Event) {
    const target = e.target as HTMLInputElement
    rows[index]!.value = target.value
    onchange?.(serializeRows())
  }

  function addRow() {
    rows.push({ key: '', value: '' })
  }

  function deleteRow(index: number) {
    rows.splice(index, 1)
    onchange?.(serializeRows())
  }
</script>

<div class="metadata-editor">
  {#if rows.length > 0}
    <div class="metadata-editor__header" aria-hidden="true">
      <span class="metadata-editor__header-label">{labels.fieldLabel}</span>
      <span class="metadata-editor__header-label">{labels.valueLabel}</span>
      <span class="metadata-editor__header-spacer"></span>
    </div>

    {#each rows as row, index (index)}
      <div class="metadata-editor__row">
        <input
          class="metadata-editor__input metadata-editor__key"
          type="text"
          placeholder={labels.keyPlaceholder}
          value={row.key}
          oninput={(e: Event) => handleKeyInput(index, e)}
        />
        <input
          class="metadata-editor__input metadata-editor__value"
          type="text"
          placeholder={labels.valuePlaceholder}
          value={row.value}
          oninput={(e: Event) => handleValueInput(index, e)}
        />
        <Button
          variant="ghost"
          size="sm"
          iconOnly
          data-testid="metadata-delete"
          onclick={() => deleteRow(index)}
          aria-label={labels.removeFieldAria}
        >
          <ActionIcon name="delete" />
        </Button>
      </div>
    {/each}
  {:else if labels.emptyText}
    <p class="metadata-editor__empty">{labels.emptyText}</p>
  {/if}

  <button
    class="metadata-editor__add"
    type="button"
    aria-label={labels.addField}
    title={labels.addField}
    data-testid="metadata-add"
    onclick={addRow}
  >
    <ActionIcon name="add" size={16} />
  </button>
</div>

<style>
  .metadata-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .metadata-editor__header,
  .metadata-editor__row {
    display: grid;
    grid-template-columns: minmax(0, 0.4fr) minmax(0, 0.6fr) auto;
    align-items: center;
    gap: var(--space-2);
  }

  .metadata-editor__header {
    padding: 0 var(--space-2);
  }

  .metadata-editor__header-label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .metadata-editor__header-spacer {
    width: 32px;
  }

  .metadata-editor__input {
    width: 100%;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background-color: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    outline: none;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth);
  }

  .metadata-editor__input::placeholder {
    color: var(--color-text-muted);
  }

  .metadata-editor__input:focus {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .metadata-editor__key {
    font-weight: var(--font-weight-medium);
  }

  .metadata-editor__empty {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }

  .metadata-editor__add {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-1);
    min-width: 32px;
    border: 1px dashed var(--color-border);
    border-radius: var(--radius-md);
    background-color: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    transition:
      border-color 0.15s ease,
      color 0.15s ease;
  }

  .metadata-editor__add:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }
</style>
