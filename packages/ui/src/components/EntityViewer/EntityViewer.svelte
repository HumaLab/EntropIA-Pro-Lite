<script lang="ts">
  import { tick } from 'svelte'
  import { ActionIcon } from '../Button'
  import type { Entity, EntityType, EntityViewerLabels } from './EntityViewer.types'
  import { ENTITY_TYPE_TAGS } from './EntityViewer.types'

  interface Props {
    entities: Entity[]
    editingEntityId?: string | null
    editingValue?: string
    onhighlight?: (detail: { startOffset: number; endOffset: number }) => void
    onentityclick?: (entity: Entity) => void
    oneditvaluechange?: (value: string) => void
    onsaveentity?: (entityId: string, value: string) => void | Promise<void>
    oncancelentityedit?: () => void
    ondeleteentity?: (entityId: string) => void | Promise<void>
    labels?: Partial<EntityViewerLabels>
  }

  let {
    entities,
    editingEntityId = null,
    editingValue = '',
    onhighlight,
    onentityclick,
    oneditvaluechange,
    onsaveentity,
    oncancelentityedit,
    ondeleteentity,
    labels: labelsProp = {},
  }: Props = $props()

  const defaultLabels: EntityViewerLabels = {
    emptyText: 'No entities extracted yet.',
    editValueAria: 'Edit entity value',
    entityAriaLabel: (value: string) => `Entity ${value}`,
    editEntityAria: (value: string) => `Edit entity ${value}`,
    editEntityTitle: 'Edit entity',
    saveEntityAria: 'Save entity',
    saveEntityTitle: 'Save changes',
    cancelEntityEditAria: 'Cancel entity edit',
    cancelEntityEditTitle: 'Cancel',
    deleteEntityAria: (value: string) => `Delete entity ${value}`,
    confirmDeleteEntityAria: (value: string) => `Confirm delete entity ${value}`,
    deleteEntityTitle: 'Delete entity',
    confirmDeleteEntityTitle: 'Press again to confirm delete',
    deletePrompt: 'Delete?',
  }

  const labels = $derived({ ...defaultLabels, ...labelsProp })

  let hoveredEntityId = $state<string | null>(null)
  let focusedEntityId = $state<string | null>(null)
  let editingInput = $state<HTMLInputElement | null>(null)
  let pendingDeleteEntityId = $state<string | null>(null)
  let pendingDeleteTimer = $state<ReturnType<typeof setTimeout> | null>(null)

  const DELETE_CONFIRM_WINDOW_MS = 1800

  // No headings anymore: type survives as chip color and as the short NER tag.
  // Sorting by the same order keeps same-type chips adjacent, so the palette
  // still reads as grouping without spending a line on a label.
  const TYPE_ORDER: EntityType[] = [
    'person',
    'organization',
    'institution',
    'place',
    'date',
    'misc',
    'custom',
  ]

  const sortedEntities = $derived(
    [...entities].sort(
      (a, b) => TYPE_ORDER.indexOf(a.entityType) - TYPE_ORDER.indexOf(b.entityType)
    )
  )

  const editingIsComplete = $derived(getNormalizedValue(editingValue).length > 0)

  function handlePillClick(entity: Entity) {
    if (entity.startOffset != null && entity.endOffset != null) {
      onhighlight?.({ startOffset: entity.startOffset, endOffset: entity.endOffset })
    }
    onentityclick?.(entity)
  }

  function getNormalizedValue(value: string) {
    return value
      .trim()
      .replace(/^["'“”‘’«»\-–—\s]+|["'“”‘’«»\-–—\s]+$/g, '')
      .trim()
  }

  function shouldSaveOnBlur(entity: Entity, nextValue: string) {
    const normalized = getNormalizedValue(nextValue)
    if (!normalized) return false
    return normalized !== entity.value
  }

  async function saveEntity(entity: Entity, nextValue: string) {
    const normalized = getNormalizedValue(nextValue)
    if (!normalized) {
      oncancelentityedit?.()
      return
    }
    await onsaveentity?.(entity.id, normalized)
  }

  async function handleInputBlur(entity: Entity, event: FocusEvent) {
    // Save/cancel live next to the input, so tabbing onto them must not let the
    // blur handler resolve the edit before their own click does.
    const nextTarget = event.relatedTarget
    const chip = (event.currentTarget as HTMLElement | null)?.closest('.entity-viewer__chip')
    if (nextTarget instanceof Node && chip?.contains(nextTarget)) return

    await tick()
    if (shouldSaveOnBlur(entity, editingValue)) {
      await saveEntity(entity, editingValue)
      return
    }
    oncancelentityedit?.()
  }

  function clearPendingDeleteTimer() {
    if (pendingDeleteTimer) {
      clearTimeout(pendingDeleteTimer)
      pendingDeleteTimer = null
    }
  }

  function cancelPendingDelete(entityId?: string) {
    if (!entityId || pendingDeleteEntityId === entityId) {
      pendingDeleteEntityId = null
    }
    clearPendingDeleteTimer()
  }

  function armDeleteConfirmation(entityId: string) {
    pendingDeleteEntityId = entityId
    clearPendingDeleteTimer()
    pendingDeleteTimer = setTimeout(() => {
      pendingDeleteEntityId = null
      pendingDeleteTimer = null
    }, DELETE_CONFIRM_WINDOW_MS)
  }

  async function handleDeleteRequest(entityId: string) {
    if (pendingDeleteEntityId !== entityId) {
      armDeleteConfirmation(entityId)
      return
    }

    cancelPendingDelete(entityId)
    await ondeleteentity?.(entityId)
  }

  function handleDeleteKeydown(event: KeyboardEvent, entityId: string) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      void handleDeleteRequest(entityId)
      return
    }

    if (event.key === 'Escape') {
      event.preventDefault()
      cancelPendingDelete(entityId)
    }
  }

  $effect(() => {
    if (editingEntityId && editingInput) {
      editingInput.focus()
      editingInput.select()
    }
  })

  $effect(() => {
    if (editingEntityId && pendingDeleteEntityId === editingEntityId) {
      cancelPendingDelete(editingEntityId)
    }
  })
</script>

{#if entities.length === 0}
  <div class="entity-viewer__empty" data-testid="entity-viewer-empty">
    <span class="entity-viewer__empty-icon" aria-hidden="true">
      <ActionIcon name="search-x" size={24} />
    </span>
    <p class="entity-viewer__empty-text">{labels.emptyText}</p>
  </div>
{:else}
  <div class="entity-viewer">
    {#each sortedEntities as entity (entity.id)}
      <div
        class="entity-viewer__chip entity-viewer__chip--{entity.entityType}"
        class:entity-viewer__chip--editing={editingEntityId === entity.id}
        data-testid={`entity-chip-${entity.id}`}
        role="group"
        aria-label={labels.entityAriaLabel(entity.value)}
        onmouseenter={() => {
          hoveredEntityId = entity.id
        }}
        onfocusin={() => {
          focusedEntityId = entity.id
        }}
        onmouseleave={() => {
          if (hoveredEntityId === entity.id) hoveredEntityId = null
          if (pendingDeleteEntityId === entity.id) cancelPendingDelete(entity.id)
        }}
        onfocusout={(event) => {
          if (focusedEntityId === entity.id) focusedEntityId = null
          const nextTarget = event.relatedTarget
          if (nextTarget instanceof Node && event.currentTarget.contains(nextTarget)) {
            return
          }
          if (pendingDeleteEntityId === entity.id) cancelPendingDelete(entity.id)
        }}
      >
        {#if editingEntityId === entity.id}
          <span class="entity-viewer__tag">{ENTITY_TYPE_TAGS[entity.entityType]}</span>
          <input
            bind:this={editingInput}
            class="entity-viewer__input"
            type="text"
            aria-label={labels.editValueAria}
            value={editingValue}
            oninput={(event) => oneditvaluechange?.(event.currentTarget.value)}
            onkeydown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                void saveEntity(entity, editingValue)
              }

              if (event.key === 'Escape') {
                event.preventDefault()
                oncancelentityedit?.()
              }
            }}
            onblur={(event) => {
              void handleInputBlur(entity, event)
            }}
          />
          <div class="entity-viewer__actions entity-viewer__actions--inline">
            <button
              type="button"
              class="entity-viewer__action"
              disabled={!editingIsComplete}
              aria-label={labels.saveEntityAria}
              title={labels.saveEntityTitle}
              data-testid={`entity-save-${entity.id}`}
              onmousedown={(event) => event.preventDefault()}
              onclick={() => void saveEntity(entity, editingValue)}
            >
              <ActionIcon name="check" size={12} />
            </button>
            <button
              type="button"
              class="entity-viewer__action"
              aria-label={labels.cancelEntityEditAria}
              title={labels.cancelEntityEditTitle}
              data-testid={`entity-cancel-${entity.id}`}
              onmousedown={(event) => event.preventDefault()}
              onclick={() => oncancelentityedit?.()}
            >
              <ActionIcon name="close" size={12} />
            </button>
          </div>
        {:else}
          <button
            type="button"
            class="entity-viewer__pill"
            onclick={() => handlePillClick(entity)}
            title={entity.value}
          >
            <span class="entity-viewer__tag">{ENTITY_TYPE_TAGS[entity.entityType]}</span>
            <span class="entity-viewer__value">{entity.value}</span>
          </button>

          {#if hoveredEntityId === entity.id || focusedEntityId === entity.id || pendingDeleteEntityId === entity.id}
            <div
              class="entity-viewer__actions"
              class:entity-viewer__actions--pending={pendingDeleteEntityId === entity.id}
            >
              <button
                type="button"
                class="entity-viewer__action"
                aria-label={labels.editEntityAria(entity.value)}
                title={labels.editEntityTitle}
                data-testid={`entity-edit-${entity.id}`}
                onclick={(event) => {
                  event.stopPropagation()
                  onentityclick?.(entity)
                }}
              >
                <ActionIcon name="edit" size={12} />
              </button>
              <button
                type="button"
                class="entity-viewer__action entity-viewer__action--delete"
                class:entity-viewer__action--pending={pendingDeleteEntityId === entity.id}
                aria-label={pendingDeleteEntityId === entity.id
                  ? labels.confirmDeleteEntityAria(entity.value)
                  : labels.deleteEntityAria(entity.value)}
                data-testid={`entity-delete-${entity.id}`}
                title={pendingDeleteEntityId === entity.id
                  ? labels.confirmDeleteEntityTitle
                  : labels.deleteEntityTitle}
                onclick={(event) => {
                  event.stopPropagation()
                  void handleDeleteRequest(entity.id)
                }}
                onkeydown={(event) => handleDeleteKeydown(event, entity.id)}
              >
                {#if pendingDeleteEntityId === entity.id}
                  <span aria-hidden="true" class="entity-viewer__delete-label"
                    >{labels.deletePrompt}</span
                  >
                {:else}
                  <ActionIcon name="delete" size={12} />
                {/if}
              </button>
            </div>
          {/if}
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .entity-viewer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }

  .entity-viewer__empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-4);
    color: var(--color-text-muted);
    text-align: center;
  }

  .entity-viewer__empty-icon {
    display: inline-flex;
    opacity: 0.4;
  }

  .entity-viewer__empty-text {
    font-size: var(--font-size-sm);
    margin: 0;
  }

  .entity-viewer__chip {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 24px;
    padding: 2px var(--space-2);
    border-radius: var(--radius-control);
    background: var(--entity-chip-bg);
    color: var(--entity-chip-fg);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    font-family: var(--font-sans);
    transition: box-shadow 0.15s ease;
  }

  .entity-viewer__chip--editing {
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--color-accent) 22%, transparent);
  }

  .entity-viewer__pill {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
    transition: opacity 0.15s ease;
  }

  /* Solo la píldora se atenúa al pasar por encima: los controles aparecen
     justo en ese momento y tienen que leerse a pleno contraste. */
  .entity-viewer__pill:hover {
    opacity: 0.85;
  }

  .entity-viewer__pill:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }

  .entity-viewer__input {
    min-width: 7rem;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    padding: 0;
    outline: none;
  }

  /* Mismo gesto que en las tripletas: las acciones aparecen al recorrer el
     chip y se superponen a su borde derecho, así el wrap no se reacomoda. */
  .entity-viewer__actions {
    position: absolute;
    inset-block: 1px;
    inset-inline-end: var(--space-1);
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-inline-start: var(--space-2);
    background: linear-gradient(to right, transparent, var(--entity-chip-bg) var(--space-2));
  }

  /* Editando no: superponer los controles sobre el input lo volvería
     inalcanzable, así que ahí van en el flujo. */
  .entity-viewer__actions--inline {
    position: static;
    padding-inline-start: 0;
    background: none;
  }

  .entity-viewer__action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 var(--space-1);
    min-width: 20px;
    height: 20px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .entity-viewer__action:hover:not(:disabled) {
    border-color: currentColor;
  }

  .entity-viewer__action:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .entity-viewer__action--delete:hover:not(:disabled),
  .entity-viewer__action--pending {
    border-color: var(--color-danger);
    color: var(--color-danger);
  }

  .entity-viewer__delete-label {
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-bold);
    letter-spacing: 0.01em;
    line-height: 1;
    white-space: nowrap;
  }

  /* Color-coded chips per entity type */
  .entity-viewer__chip--person {
    --entity-chip-bg: color-mix(in srgb, var(--color-info) 12%, var(--surface-card));
    --entity-chip-fg: var(--color-info);
  }

  .entity-viewer__chip--place {
    --entity-chip-bg: color-mix(in srgb, var(--color-success) 12%, var(--surface-card));
    --entity-chip-fg: var(--color-success);
  }

  .entity-viewer__chip--date {
    --entity-chip-bg: color-mix(in srgb, var(--color-warning) 12%, var(--surface-card));
    --entity-chip-fg: var(--color-warning);
  }

  .entity-viewer__chip--institution {
    --entity-chip-bg: color-mix(in srgb, var(--color-danger) 11%, var(--surface-card));
    --entity-chip-fg: var(--color-danger);
  }

  /* Misma fórmula opaca que el resto: `--color-accent-faint` es un 6% de alfa,
     así que dejaba ver el texto de atrás y los controles inline se mezclaban
     con el contenido del chip. */
  .entity-viewer__chip--organization {
    --entity-chip-bg: color-mix(in srgb, var(--color-accent) 12%, var(--surface-card));
    --entity-chip-fg: var(--color-accent-hover);
  }

  .entity-viewer__chip--misc {
    --entity-chip-bg: var(--surface-card);
    --entity-chip-fg: var(--color-text-secondary);
  }

  .entity-viewer__chip--custom {
    --entity-chip-bg: var(--color-surface-raised);
    --entity-chip-fg: var(--color-text-secondary);
  }

  .entity-viewer__tag {
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-bold);
    letter-spacing: 0.04em;
    opacity: 0.9;
  }

  .entity-viewer__value {
    white-space: nowrap;
  }
</style>
