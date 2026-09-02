<script lang="ts">
  import { tick } from 'svelte'
  import { ActionIcon } from '../Button'
  import type { Entity, EntityType, EntityViewerLabels } from './EntityViewer.types'
  import { ENTITY_TYPE_TAGS } from './EntityViewer.types'

  interface Props {
    entities: Entity[]
    editingEntityId?: string | null
    editingValue?: string
    creatableTypes?: EntityType[]
    newEntityType?: EntityType
    newEntityValue?: string
    onhighlight?: (detail: { startOffset: number; endOffset: number }) => void
    onentityclick?: (entity: Entity) => void
    oneditvaluechange?: (value: string) => void
    onsaveentity?: (entityId: string, value: string) => void | Promise<void>
    oncancelentityedit?: () => void
    ondeleteentity?: (entityId: string) => void | Promise<void>
    onnewentitytypechange?: (entityType: EntityType) => void
    onnewentityvaluechange?: (value: string) => void
    oncreateentity?: () => boolean | Promise<boolean>
    labels?: Partial<EntityViewerLabels>
  }

  let {
    entities,
    editingEntityId = null,
    editingValue = '',
    creatableTypes = [],
    newEntityType,
    newEntityValue = '',
    onhighlight,
    onentityclick,
    oneditvaluechange,
    onsaveentity,
    oncancelentityedit,
    ondeleteentity,
    onnewentitytypechange,
    onnewentityvaluechange,
    oncreateentity,
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
    addEntity: 'Add entity',
    newEntityTypeAria: 'New entity type',
    newEntityValueAria: 'New entity value',
    newEntityValuePlaceholder: 'New entity value',
    saveNewEntityAria: 'Save the new entity',
    cancelNewEntityAria: 'Discard the new entity',
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

  // Alta manual. Ofrecer al menos un tipo es lo que enciende el chip final;
  // el borrador en sí vive con quien es dueño de las entidades.
  const canCreate = $derived(creatableTypes.length > 0)
  const draftType = $derived(newEntityType ?? creatableTypes[0] ?? 'misc')
  const newEntityIsComplete = $derived(getNormalizedValue(newEntityValue).length > 0)

  let creatingEntity = $state(false)
  let newEntityInput = $state<HTMLInputElement | null>(null)

  // Selector de tipo propio en vez de un <select>: el popup nativo lo dibuja el
  // sistema operativo, con fondo claro y tipografía ajena, y no hay CSS que lo
  // alcance. Este es el mismo mecanismo (elegir uno de una lista) pintado con
  // los tokens de la app.
  let typeMenuOpen = $state(false)
  let typeMenuIndex = $state(0)
  let typeTrigger = $state<HTMLButtonElement | null>(null)
  let typeMenu = $state<HTMLDivElement | null>(null)
  let typeWrap = $state<HTMLDivElement | null>(null)

  function openTypeMenu() {
    typeMenuIndex = Math.max(0, creatableTypes.indexOf(draftType))
    typeMenuOpen = true
  }

  function closeTypeMenu(returnFocus = true) {
    typeMenuOpen = false
    if (returnFocus) typeTrigger?.focus()
  }

  function selectDraftType(type: EntityType) {
    onnewentitytypechange?.(type)
    closeTypeMenu()
  }

  function handleTypeTriggerKeydown(event: KeyboardEvent) {
    // Enter y Espacio ya disparan el click nativo del botón; acá solo hace
    // falta la apertura por flechas.
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      openTypeMenu()
    }
  }

  function handleTypeMenuKeydown(event: KeyboardEvent) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const step = event.key === 'ArrowDown' ? 1 : -1
      typeMenuIndex = (typeMenuIndex + step + creatableTypes.length) % creatableTypes.length
      return
    }

    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault()
      typeMenuIndex = event.key === 'Home' ? 0 : creatableTypes.length - 1
      return
    }

    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      const type = creatableTypes[typeMenuIndex]
      if (type) selectDraftType(type)
      return
    }

    if (event.key === 'Escape') {
      // Cierra solo el menú: el borrador de la entidad sigue abierto.
      event.stopPropagation()
      event.preventDefault()
      closeTypeMenu()
    }
  }

  function handleWindowMouseDown(event: MouseEvent) {
    if (!typeMenuOpen) return
    const target = event.target
    if (target instanceof Node && typeWrap?.contains(target)) return
    typeMenuOpen = false
  }

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

  function startCreatingEntity() {
    cancelPendingDelete()
    onnewentityvaluechange?.('')
    creatingEntity = true
  }

  function cancelCreatingEntity() {
    // Cancelar no escribe nada: el chip borrador solo existió en memoria.
    creatingEntity = false
    typeMenuOpen = false
    onnewentityvaluechange?.('')
  }

  async function saveNewEntity() {
    if (!newEntityIsComplete) return

    const created = await oncreateentity?.()
    // Un alta fallida deja el borrador abierto con lo escrito.
    if (created) {
      creatingEntity = false
      typeMenuOpen = false
    }
  }

  function handleNewEntityKeydown(event: KeyboardEvent) {
    // keyCode 229 cubre WKWebView, donde isComposing puede no reportarse durante IME.
    if (event.key === 'Enter' && !event.isComposing && event.keyCode !== 229) {
      event.preventDefault()
      void saveNewEntity()
      return
    }

    if (event.key === 'Escape') {
      event.preventDefault()
      cancelCreatingEntity()
    }
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
    if (creatingEntity && newEntityInput) {
      newEntityInput.focus()
    }
  })

  $effect(() => {
    if (typeMenuOpen && typeMenu) {
      typeMenu.focus()
    }
  })

  $effect(() => {
    if (editingEntityId && pendingDeleteEntityId === editingEntityId) {
      cancelPendingDelete(editingEntityId)
    }
  })
</script>

<svelte:window onmousedown={handleWindowMouseDown} />

{#if entities.length === 0 && !canCreate}
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
            <div class="entity-viewer__actions">
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

    {#if canCreate}
      {#if creatingEntity}
        <div
          class="entity-viewer__chip entity-viewer__chip--editing entity-viewer__chip--{draftType}"
          data-testid="entity-new-chip"
        >
          <div class="entity-viewer__type" bind:this={typeWrap}>
            <button
              bind:this={typeTrigger}
              type="button"
              class="entity-viewer__type-trigger"
              aria-label={labels.newEntityTypeAria}
              aria-haspopup="listbox"
              aria-expanded={typeMenuOpen}
              data-testid="entity-new-type"
              onclick={() => (typeMenuOpen ? closeTypeMenu(false) : openTypeMenu())}
              onkeydown={handleTypeTriggerKeydown}
            >
              <span class="entity-viewer__tag">{ENTITY_TYPE_TAGS[draftType]}</span>
              <ActionIcon name="chevron-down" size={12} />
            </button>

            {#if typeMenuOpen}
              <div
                bind:this={typeMenu}
                class="entity-viewer__type-menu"
                role="listbox"
                tabindex="-1"
                aria-label={labels.newEntityTypeAria}
                aria-activedescendant={`entity-type-option-${creatableTypes[typeMenuIndex]}`}
                data-testid="entity-new-type-menu"
                onkeydown={handleTypeMenuKeydown}
              >
                {#each creatableTypes as type, index (type)}
                  <button
                    id={`entity-type-option-${type}`}
                    type="button"
                    class="entity-viewer__type-option entity-viewer__type-option--{type}"
                    class:entity-viewer__type-option--active={index === typeMenuIndex}
                    role="option"
                    tabindex="-1"
                    aria-selected={type === draftType}
                    data-testid={`entity-new-type-${type}`}
                    onmouseenter={() => (typeMenuIndex = index)}
                    onclick={() => selectDraftType(type)}
                  >
                    <span class="entity-viewer__type-dot" aria-hidden="true"></span>
                    <span>{ENTITY_TYPE_TAGS[type]}</span>
                  </button>
                {/each}
              </div>
            {/if}
          </div>
          <input
            bind:this={newEntityInput}
            class="entity-viewer__input"
            type="text"
            aria-label={labels.newEntityValueAria}
            placeholder={labels.newEntityValuePlaceholder}
            value={newEntityValue}
            oninput={(event) => onnewentityvaluechange?.(event.currentTarget.value)}
            onkeydown={handleNewEntityKeydown}
          />
          <div class="entity-viewer__actions entity-viewer__actions--inline">
            <button
              type="button"
              class="entity-viewer__action"
              disabled={!newEntityIsComplete}
              aria-label={labels.saveNewEntityAria}
              title={labels.saveEntityTitle}
              data-testid="entity-new-save"
              onmousedown={(event) => event.preventDefault()}
              onclick={() => void saveNewEntity()}
            >
              <ActionIcon name="check" size={12} />
            </button>
            <button
              type="button"
              class="entity-viewer__action"
              aria-label={labels.cancelNewEntityAria}
              title={labels.cancelEntityEditTitle}
              data-testid="entity-new-cancel"
              onmousedown={(event) => event.preventDefault()}
              onclick={cancelCreatingEntity}
            >
              <ActionIcon name="close" size={12} />
            </button>
          </div>
        </div>
      {:else}
        <button
          type="button"
          class="entity-viewer__chip entity-viewer__chip--add"
          aria-label={labels.addEntity}
          title={labels.addEntity}
          data-testid="entity-add"
          onclick={startCreatingEntity}
        >
          <ActionIcon name="add" size={12} />
        </button>
      {/if}
    {/if}
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

  /* El borde transparente no se ve, pero iguala la geometría con el chip de
     alta, que sí lleva uno punteado. Así todas las etiquetas miden lo mismo. */
  .entity-viewer__chip {
    position: relative;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 24px;
    padding: 2px var(--space-2);
    border: 1px solid transparent;
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

  .entity-viewer__type {
    position: relative;
    display: inline-flex;
  }

  /* El disparador vive dentro del chip, así que hereda su color y solo se
     insinúa con un borde al recorrerlo, igual que las acciones de la fila. */
  .entity-viewer__type-trigger {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 2px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font-family: inherit;
    cursor: pointer;
  }

  .entity-viewer__type-trigger:hover,
  .entity-viewer__type-trigger[aria-expanded='true'],
  .entity-viewer__type-trigger:focus-visible {
    outline: none;
    border-color: currentColor;
  }

  .entity-viewer__type-menu {
    position: absolute;
    inset-inline-start: 0;
    top: calc(100% + 4px);
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 5.5rem;
    padding: 2px;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-surface);
    outline: none;
  }

  .entity-viewer__type-option {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    width: 100%;
    padding: 3px var(--space-1);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: start;
    color: var(--color-text-secondary);
    font-family: var(--font-sans);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-bold);
    letter-spacing: 0.04em;
    cursor: pointer;
  }

  /* Hover y foco de teclado son el mismo estado: un realce oscuro, nunca un
     fondo claro. */
  .entity-viewer__type-option--active {
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface-raised));
    color: var(--color-text-primary);
  }

  /* La opción vigente se distingue por su propio color de tipo, no por fondo. */
  .entity-viewer__type-option[aria-selected='true'] {
    color: var(--entity-option-color);
  }

  .entity-viewer__type-dot {
    flex: 0 0 auto;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--entity-option-color);
  }

  .entity-viewer__type-option--person {
    --entity-option-color: var(--color-info);
  }

  .entity-viewer__type-option--place {
    --entity-option-color: var(--color-success);
  }

  .entity-viewer__type-option--date {
    --entity-option-color: var(--color-warning);
  }

  .entity-viewer__type-option--institution {
    --entity-option-color: var(--color-danger);
  }

  .entity-viewer__type-option--organization {
    --entity-option-color: var(--color-accent-hover);
  }

  .entity-viewer__type-option--misc,
  .entity-viewer__type-option--custom {
    --entity-option-color: var(--color-text-muted);
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

  /* Una etiqueta más del conjunto, no un botón al costado: el punteado es lo
     único que la distingue de una entidad ya cargada. */
  .entity-viewer__chip--add {
    --entity-chip-bg: var(--surface-card);
    --entity-chip-fg: var(--color-text-muted);
    border-style: dashed;
    border-color: var(--color-hairline);
    cursor: pointer;
    transition:
      border-color 0.15s ease,
      color 0.15s ease;
  }

  .entity-viewer__chip--add:hover {
    border-color: var(--color-accent);
    color: var(--color-accent-hover);
  }

  .entity-viewer__chip--add:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
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
