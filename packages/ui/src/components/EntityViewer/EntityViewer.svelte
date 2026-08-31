<script lang="ts">
  import { tick } from 'svelte'
  import { ActionIcon } from '../Button'
  import type { Entity, EntityType, EntityViewerLabels } from './EntityViewer.types'
  import { ENTITY_TYPE_LABELS, ENTITY_TYPE_TAGS } from './EntityViewer.types'

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
    deleteEntityAria: (value: string) => `Delete entity ${value}`,
    confirmDeleteEntityAria: (value: string) => `Confirm delete entity ${value}`,
    deleteEntityTitle: 'Delete entity',
    confirmDeleteEntityTitle: 'Press again to confirm delete',
    deletePrompt: 'Delete?',
    typeLabels: ENTITY_TYPE_LABELS,
  }

  const labels = $derived({ ...defaultLabels, ...labelsProp })

  let hoveredEntityId = $state<string | null>(null)
  let focusedEntityId = $state<string | null>(null)
  let editingInput = $state<HTMLInputElement | null>(null)
  let pendingDeleteEntityId = $state<string | null>(null)
  let pendingDeleteTimer = $state<ReturnType<typeof setTimeout> | null>(null)

  const DELETE_CONFIRM_WINDOW_MS = 1800

  // Group entities by type, preserving order for core NER labels first.
  const TYPE_ORDER: EntityType[] = [
    'person',
    'organization',
    'institution',
    'place',
    'date',
    'misc',
    'custom',
  ]

  const grouped = $derived(
    TYPE_ORDER.reduce<Map<EntityType, Entity[]>>((acc, type) => {
      const group = entities.filter((e) => e.entityType === type)
      if (group.length > 0) acc.set(type, group)
      return acc
    }, new Map())
  )

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

  async function handleInputBlur(entity: Entity) {
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
    {#each TYPE_ORDER as type (type)}
      {#if grouped.has(type)}
        <div class="entity-viewer__group" data-testid="entity-group">
          <span class="entity-viewer__group-label entity-viewer__group-label--{type}">
            {labels.typeLabels[type]}
          </span>
          <div class="entity-viewer__pills">
            {#each grouped.get(type) ?? [] as entity (entity.id)}
              <div
                class="entity-viewer__chip entity-viewer__chip--{type}"
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
                  <div
                    class="entity-viewer__pill entity-viewer__pill--editing entity-viewer__pill--{type}"
                  >
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
                      onblur={() => {
                        void handleInputBlur(entity)
                      }}
                    />
                  </div>
                {:else}
                  <button
                    type="button"
                    class="entity-viewer__pill entity-viewer__pill--{type}"
                    onclick={() => handlePillClick(entity)}
                    title={entity.value}
                  >
                    <span class="entity-viewer__tag">{ENTITY_TYPE_TAGS[entity.entityType]}</span>
                    <span class="entity-viewer__value">{entity.value}</span>
                  </button>
                {/if}

                {#if (hoveredEntityId === entity.id || focusedEntityId === entity.id || pendingDeleteEntityId === entity.id) && editingEntityId !== entity.id}
                  <button
                    type="button"
                    class="entity-viewer__delete"
                    class:entity-viewer__delete--pending={pendingDeleteEntityId === entity.id}
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
                      <ActionIcon name="close" size={12} />
                    {/if}
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .entity-viewer {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
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

  .entity-viewer__group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .entity-viewer__group-label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    text-transform: uppercase;
    letter-spacing: 0.075em;
    color: var(--color-text-muted);
  }

  .entity-viewer__pills {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .entity-viewer__chip {
    position: relative;
    display: inline-flex;
    align-items: center;
  }

  .entity-viewer__pill {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px var(--space-2);
    border-radius: var(--radius-control);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    font-family: var(--font-sans);
    cursor: pointer;
    border: none;
    transition:
      opacity 0.15s ease,
      box-shadow 0.15s ease;
  }

  .entity-viewer__pill:hover {
    opacity: 0.85;
  }

  .entity-viewer__pill:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }

  .entity-viewer__pill--editing {
    cursor: text;
    opacity: 1;
    transform: none;
    padding-right: var(--space-2);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--color-accent) 22%, transparent);
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

  .entity-viewer__delete {
    position: absolute;
    top: 50%;
    right: 4px;
    transform: translateY(-50%);
    min-width: 18px;
    height: 18px;
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-control);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--color-surface-elevated) 86%, transparent);
    color: inherit;
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.15s ease,
      transform 0.15s ease,
      background 0.15s ease;
  }

  .entity-viewer__chip:hover .entity-viewer__delete,
  .entity-viewer__delete:focus-visible {
    opacity: 1;
    transform: translateY(-50%) scale(1);
  }

  .entity-viewer__delete:hover {
    background: var(--color-surface-elevated);
  }

  .entity-viewer__delete--pending {
    background: var(--color-danger-soft);
    color: var(--color-danger);
    opacity: 1;
    transform: translateY(-50%) scale(1);
  }

  .entity-viewer__delete--pending:hover,
  .entity-viewer__delete--pending:focus-visible {
    background: var(--color-danger-hover);
  }

  .entity-viewer__delete-label {
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-bold);
    letter-spacing: 0.01em;
    line-height: 1;
    white-space: nowrap;
  }

  /* Color-coded pills per entity type */
  .entity-viewer__pill--person {
    background-color: color-mix(in srgb, var(--color-info) 12%, var(--surface-card));
    color: var(--color-info);
  }

  .entity-viewer__pill--place {
    background-color: color-mix(in srgb, var(--color-success) 12%, var(--surface-card));
    color: var(--color-success);
  }

  .entity-viewer__pill--date {
    background-color: color-mix(in srgb, var(--color-warning) 12%, var(--surface-card));
    color: var(--color-warning);
  }

  .entity-viewer__pill--institution {
    background-color: color-mix(in srgb, var(--color-danger) 11%, var(--surface-card));
    color: var(--color-danger);
  }

  .entity-viewer__pill--organization {
    background-color: var(--color-accent-faint);
    color: var(--color-accent-hover);
  }

  .entity-viewer__pill--misc {
    background-color: var(--surface-card);
    color: var(--color-text-secondary);
  }

  .entity-viewer__pill--custom {
    background-color: var(--color-surface-raised);
    color: var(--color-text-secondary);
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
