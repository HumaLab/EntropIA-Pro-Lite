<script lang="ts">
  import {
    ActionIcon,
    EntityViewer,
    StatusBadge,
    type MapMarker,
    type StatusBadgeVariant,
  } from '@entropia/ui'
  import type { Entity } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { ItemNlpState } from '$lib/nlp'
  import ItemMapViewer from './ItemMapViewer.svelte'

  type EditableEntityType = 'person' | 'organization' | 'place' | 'misc' | 'date'
  type SemanticTriple = { id: string; subject: string; predicate: string; object: string }
  type TripleDraft = { subject: string; predicate: string; object: string }

  const EDITABLE_ENTITY_TYPES: EditableEntityType[] = [
    'person',
    'organization',
    'place',
    'misc',
    'date',
  ]

  let {
    assetsCount,
    selectedAsset,
    nlpState,
    llmAvailable,
    geoMarkers,
    visible,
    entities,
    editingEntityId,
    editingEntityValue,
    newEntityType,
    newEntityValue,
    entityActionError,
    triples,
    tripleActionError,
    translate,
    onIndexFts,
    onEmbedAsset,
    onExtractEntities,
    onExtractTriples,
    onEntityClick,
    onEditValueChange,
    onSaveEntity,
    onCancelEntityEdit,
    onDeleteEntity,
    onNewEntityTypeChange,
    onNewEntityValueChange,
    onCreateEntity,
    onCreateTriple,
    onSaveTriple,
    onDeleteTriple,
    onSaveMapLocation,
    onResetMapLocation,
  }: {
    assetsCount: number
    selectedAsset: boolean
    nlpState: ItemNlpState
    llmAvailable: boolean
    geoMarkers: MapMarker[]
    visible: boolean
    entities: Entity[]
    editingEntityId: string | null
    editingEntityValue: string
    newEntityType: EditableEntityType
    newEntityValue: string
    entityActionError: string | null
    triples: SemanticTriple[]
    tripleActionError: string | null
    translate: (key: I18nKey, params?: I18nParams) => string
    onIndexFts: () => void | Promise<void>
    onEmbedAsset: () => void | Promise<void>
    onExtractEntities: () => void | Promise<void>
    onExtractTriples: () => void | Promise<void>
    onEntityClick: (entity: Entity) => void
    onEditValueChange: (value: string) => void
    onSaveEntity: (entityId: string, value: string) => void | Promise<void>
    onCancelEntityEdit: () => void
    onDeleteEntity: (entityId: string) => void | Promise<void>
    onNewEntityTypeChange: (type: EditableEntityType) => void
    onNewEntityValueChange: (value: string) => void
    onCreateEntity: () => void | Promise<void>
    /** Resolves to `true` only when the row was persisted, so the draft can close. */
    onCreateTriple: (draft: TripleDraft) => Promise<boolean>
    /** Resolves to `true` only when the edit was persisted, so the row can close. */
    onSaveTriple: (tripleId: string, draft: TripleDraft) => Promise<boolean>
    onDeleteTriple: (tripleId: string) => void | Promise<void>
    onSaveMapLocation: (entityId: string, latitude: number, longitude: number) => void | Promise<void>
    onResetMapLocation: (entityId: string) => void | Promise<void>
  } = $props()

  // Sesión de edición de tripletas. Vive acá y no en ItemView porque es estado
  // de UI puramente transitorio: solo el guardado y el borrado cruzan hacia la
  // base. Misma mecánica que EntityViewer para las entidades NER: edición
  // inline y borrado en dos pasos con ventana de confirmación.
  const TRIPLE_DELETE_CONFIRM_WINDOW_MS = 1800

  let editingTripleId = $state<string | null>(null)
  let editingTriple = $state<TripleDraft>({ subject: '', predicate: '', object: '' })
  let editingTripleInput = $state<HTMLInputElement | null>(null)
  let pendingDeleteTripleId = $state<string | null>(null)
  let pendingDeleteTripleTimer = $state<ReturnType<typeof setTimeout> | null>(null)

  const editingTripleIsComplete = $derived(
    Boolean(
      editingTriple.subject.trim() && editingTriple.predicate.trim() && editingTriple.object.trim()
    )
  )

  function startEditingTriple(triple: SemanticTriple) {
    cancelPendingTripleDelete()
    editingTripleId = triple.id
    editingTriple = {
      subject: triple.subject,
      predicate: triple.predicate,
      object: triple.object,
    }
  }

  function cancelEditingTriple() {
    editingTripleId = null
  }

  async function saveEditingTriple() {
    const tripleId = editingTripleId
    if (!tripleId || !editingTripleIsComplete) return

    const saved = await onSaveTriple(tripleId, {
      subject: editingTriple.subject.trim(),
      predicate: editingTriple.predicate.trim(),
      object: editingTriple.object.trim(),
    })
    // Un guardado fallido deja la fila abierta con lo escrito: el error se
    // muestra abajo y el trabajo del usuario no se pierde.
    if (saved && editingTripleId === tripleId) {
      editingTripleId = null
    }
  }

  function handleTripleEditKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing && event.keyCode !== 229) {
      event.preventDefault()
      void saveEditingTriple()
      return
    }

    if (event.key === 'Escape') {
      event.preventDefault()
      cancelEditingTriple()
    }
  }

  // Alta manual de tripletas. Sesión propia y separada de la de edición: una
  // cosa no cancela a la otra, así nadie pierde lo que estaba tipeando.
  let creatingTriple = $state(false)
  let newTriple = $state<TripleDraft>({ subject: '', predicate: '', object: '' })
  let newTripleInput = $state<HTMLInputElement | null>(null)

  const newTripleIsComplete = $derived(
    Boolean(newTriple.subject.trim() && newTriple.predicate.trim() && newTriple.object.trim())
  )

  function startCreatingTriple() {
    cancelPendingTripleDelete()
    newTriple = { subject: '', predicate: '', object: '' }
    creatingTriple = true
  }

  function cancelCreatingTriple() {
    // Cancelar no escribe nada: la fila borrador solo existió en memoria.
    creatingTriple = false
    newTriple = { subject: '', predicate: '', object: '' }
  }

  async function saveNewTriple() {
    if (!newTripleIsComplete) return

    const created = await onCreateTriple({
      subject: newTriple.subject.trim(),
      predicate: newTriple.predicate.trim(),
      object: newTriple.object.trim(),
    })
    // Un alta fallida deja el borrador abierto con lo escrito.
    if (created) {
      cancelCreatingTriple()
    }
  }

  function handleNewTripleKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing && event.keyCode !== 229) {
      event.preventDefault()
      void saveNewTriple()
      return
    }

    if (event.key === 'Escape') {
      event.preventDefault()
      cancelCreatingTriple()
    }
  }

  $effect(() => {
    if (creatingTriple && newTripleInput) {
      newTripleInput.focus()
    }
  })

  function clearPendingTripleDeleteTimer() {
    if (pendingDeleteTripleTimer) {
      clearTimeout(pendingDeleteTripleTimer)
      pendingDeleteTripleTimer = null
    }
  }

  function cancelPendingTripleDelete(tripleId?: string) {
    if (!tripleId || pendingDeleteTripleId === tripleId) {
      pendingDeleteTripleId = null
    }
    clearPendingTripleDeleteTimer()
  }

  async function handleTripleDeleteRequest(tripleId: string) {
    // Primer click arma la confirmación, segundo borra. La ventana se cierra
    // sola para que un click olvidado no quede armado indefinidamente.
    if (pendingDeleteTripleId !== tripleId) {
      pendingDeleteTripleId = tripleId
      clearPendingTripleDeleteTimer()
      pendingDeleteTripleTimer = setTimeout(() => {
        pendingDeleteTripleId = null
        pendingDeleteTripleTimer = null
      }, TRIPLE_DELETE_CONFIRM_WINDOW_MS)
      return
    }

    cancelPendingTripleDelete(tripleId)
    await onDeleteTriple(tripleId)
  }

  function handleTripleDeleteKeydown(event: KeyboardEvent, tripleId: string) {
    if (event.key === 'Escape') {
      event.preventDefault()
      cancelPendingTripleDelete(tripleId)
    }
  }

  $effect(() => {
    if (editingTripleId && editingTripleInput) {
      editingTripleInput.focus()
      editingTripleInput.select()
    }
  })

  $effect(() => {
    // Una recarga de tripletas (otra página, nueva extracción) invalida la
    // sesión abierta: la fila que se estaba editando puede ya no existir.
    if (editingTripleId && !triples.some((triple) => triple.id === editingTripleId)) {
      editingTripleId = null
    }
    if (pendingDeleteTripleId && !triples.some((triple) => triple.id === pendingDeleteTripleId)) {
      cancelPendingTripleDelete()
    }
  })

  function handleNewEntityKeydown(event: KeyboardEvent) {
    // keyCode 229 cubre WKWebView, donde isComposing puede no reportarse durante IME.
    if (event.key === 'Enter' && !event.isComposing && event.keyCode !== 229) {
      void onCreateEntity()
    }
  }

  function getJobStatusBadgeVariant(status: string): StatusBadgeVariant {
    if (status === 'done') return 'success'
    if (status === 'running') return 'warning'
    if (status === 'pending') return 'info'
    if (status === 'error') return 'danger'
    return 'neutral'
  }
</script>

{#if assetsCount > 0}
  <section class="section">
    <div class="analysis-panel analysis-panel--tabbed">
      <div class="nlp-actions">
        <button
          class="nlp-btn"
          disabled={nlpState.fts === 'pending' || nlpState.fts === 'running'}
          onclick={onIndexFts}
        >
          {translate('item.indexAction')}
          <StatusBadge variant={getJobStatusBadgeVariant(nlpState.fts)} size="sm" class="nlp-badge">{nlpState.fts}</StatusBadge>
        </button>

        <button
          class="nlp-btn"
          disabled={!selectedAsset || nlpState.embed === 'pending' || nlpState.embed === 'running'}
          onclick={onEmbedAsset}
        >
          {translate('item.embedAction')}
          <StatusBadge variant={getJobStatusBadgeVariant(nlpState.embed)} size="sm" class="nlp-badge">{nlpState.embed}</StatusBadge>
        </button>

        <button
          class="nlp-btn"
          disabled={nlpState.ner === 'pending' || nlpState.ner === 'running'}
          onclick={onExtractEntities}
        >
          {translate('item.nerAction')}
          <StatusBadge variant={getJobStatusBadgeVariant(nlpState.ner)} size="sm" class="nlp-badge">{nlpState.ner === 'done' && nlpState.entityCount === 0 ? `${nlpState.ner} · 0` : nlpState.ner}</StatusBadge>
        </button>

        <button
          class="nlp-btn"
          disabled={!llmAvailable || nlpState.triples === 'pending' || nlpState.triples === 'running'}
          onclick={onExtractTriples}
        >
          {translate('item.triplesAction')}
          <StatusBadge variant={getJobStatusBadgeVariant(nlpState.triples)} size="sm" class="nlp-badge">{nlpState.triples}</StatusBadge>
        </button>
      </div>

      {#if nlpState.errors?.embed}
        <p class="ocr-error">
          {translate('item.embeddingError', { error: nlpState.errors.embed })}
        </p>
      {/if}

      {#if !selectedAsset}
        <p class="empty-text">
          {translate('item.analysisNeedAsset')}
        </p>
      {/if}

      <div class="geo-section">
        <ItemMapViewer
          {entities}
          {geoMarkers}
          height="280px"
          {visible}
          {translate}
          {onSaveMapLocation}
          {onResetMapLocation}
        />
      </div>

      <div class="entities-section">
        <h4>{translate('item.entities')}</h4>
        <EntityViewer
          {entities}
          {editingEntityId}
          editingValue={editingEntityValue}
          labels={{
            editValueAria: translate('item.entityEditValueAria'),
            deleteEntityAria: (value: string) => translate('item.entityDeleteAria', { value }),
          }}
          onentityclick={onEntityClick}
          oneditvaluechange={onEditValueChange}
          onsaveentity={onSaveEntity}
          oncancelentityedit={onCancelEntityEdit}
          ondeleteentity={onDeleteEntity}
        />

        <div class="entity-editor">
          <h5>{translate('item.manualEntities')}</h5>
          <p class="entity-editor__hint">
            {translate('item.entityHint')}
          </p>

          <div class="entity-editor__create">
            <select
              value={newEntityType}
              aria-label={translate('item.newEntityType')}
              onchange={(event) => {
                onNewEntityTypeChange(event.currentTarget.value as EditableEntityType)
              }}
            >
              {#each EDITABLE_ENTITY_TYPES as type (type)}
                <option value={type}>{type.toUpperCase()}</option>
              {/each}
            </select>
            <input
              value={newEntityValue}
              type="text"
              placeholder={translate('item.newEntityValue')}
              aria-label={translate('item.newEntityValue')}
              oninput={(event) => onNewEntityValueChange(event.currentTarget.value)}
              onkeydown={handleNewEntityKeydown}
            />
            <button
              type="button"
              class="nlp-btn nlp-btn--icon"
              aria-label={translate('item.addEntity')}
              title={translate('item.addEntity')}
              data-testid="entity-add"
              onclick={onCreateEntity}
            >
              <ActionIcon name="add" size={16} />
            </button>
          </div>

          {#if entityActionError}
            <p class="error">{entityActionError}</p>
          {/if}
        </div>
      </div>

      <div class="triples-section">
        <h4>{translate('item.semanticTriples')}</h4>
        {#if triples.length === 0 && !creatingTriple}
          <p class="empty-text">
            {translate('item.noTriples', {
              suffix: assetsCount > 1 ? translate('item.noTriplesPageSuffix') : '',
            })}
          </p>
        {/if}

        {#if triples.length > 0 || creatingTriple}
          <ul class="triples-list">
            {#each triples as triple (triple.id)}
              <li
                class="triple-item"
                class:triple-item--editing={editingTripleId === triple.id}
                class:triple-item--pending={pendingDeleteTripleId === triple.id}
                data-testid={`triple-row-${triple.id}`}
                onmouseleave={() => cancelPendingTripleDelete(triple.id)}
              >
                {#if editingTripleId === triple.id}
                  <input
                    bind:this={editingTripleInput}
                    class="triple-cell triple-input"
                    type="text"
                    aria-label={translate('item.tripleSubjectAria')}
                    bind:value={editingTriple.subject}
                    onkeydown={handleTripleEditKeydown}
                  />
                  <input
                    class="triple-cell triple-cell--predicate triple-input"
                    type="text"
                    aria-label={translate('item.triplePredicateAria')}
                    bind:value={editingTriple.predicate}
                    onkeydown={handleTripleEditKeydown}
                  />
                  <input
                    class="triple-cell triple-input"
                    type="text"
                    aria-label={translate('item.tripleObjectAria')}
                    bind:value={editingTriple.object}
                    onkeydown={handleTripleEditKeydown}
                  />
                  <div class="triple-actions">
                    <button
                      type="button"
                      class="triple-action"
                      disabled={!editingTripleIsComplete}
                      aria-label={translate('item.tripleSaveAria')}
                      title={translate('item.tripleSaveTitle')}
                      data-testid={`triple-save-${triple.id}`}
                      onclick={() => void saveEditingTriple()}
                    >
                      <ActionIcon name="check" size={12} />
                    </button>
                    <button
                      type="button"
                      class="triple-action"
                      aria-label={translate('item.tripleCancelAria')}
                      title={translate('item.tripleCancelTitle')}
                      data-testid={`triple-cancel-${triple.id}`}
                      onclick={cancelEditingTriple}
                    >
                      <ActionIcon name="close" size={12} />
                    </button>
                  </div>
                {:else}
                  <span class="triple-cell">{triple.subject}</span>
                  <span class="triple-cell triple-cell--predicate">{triple.predicate}</span>
                  <span class="triple-cell">{triple.object}</span>
                  <div class="triple-actions">
                    <button
                      type="button"
                      class="triple-action"
                      aria-label={translate('item.tripleEditAria', { subject: triple.subject })}
                      title={translate('item.tripleEditTitle')}
                      data-testid={`triple-edit-${triple.id}`}
                      onclick={() => startEditingTriple(triple)}
                    >
                      <ActionIcon name="edit" size={12} />
                    </button>
                    <button
                      type="button"
                      class="triple-action triple-action--delete"
                      class:triple-action--pending={pendingDeleteTripleId === triple.id}
                      aria-label={pendingDeleteTripleId === triple.id
                        ? translate('item.tripleConfirmDeleteAria', { subject: triple.subject })
                        : translate('item.tripleDeleteAria', { subject: triple.subject })}
                      title={pendingDeleteTripleId === triple.id
                        ? translate('item.tripleConfirmDeleteTitle')
                        : translate('item.tripleDeleteTitle')}
                      data-testid={`triple-delete-${triple.id}`}
                      onclick={() => void handleTripleDeleteRequest(triple.id)}
                      onkeydown={(event) => handleTripleDeleteKeydown(event, triple.id)}
                    >
                      {#if pendingDeleteTripleId === triple.id}
                        <span aria-hidden="true" class="triple-action__label"
                          >{translate('item.tripleDeletePrompt')}</span
                        >
                      {:else}
                        <ActionIcon name="delete" size={12} />
                      {/if}
                    </button>
                  </div>
                {/if}
              </li>
            {/each}

            {#if creatingTriple}
              <li class="triple-item triple-item--editing" data-testid="triple-new-row">
                <input
                  bind:this={newTripleInput}
                  class="triple-cell triple-input"
                  type="text"
                  aria-label={translate('item.newTripleSubjectAria')}
                  bind:value={newTriple.subject}
                  onkeydown={handleNewTripleKeydown}
                />
                <input
                  class="triple-cell triple-cell--predicate triple-input"
                  type="text"
                  aria-label={translate('item.newTriplePredicateAria')}
                  bind:value={newTriple.predicate}
                  onkeydown={handleNewTripleKeydown}
                />
                <input
                  class="triple-cell triple-input"
                  type="text"
                  aria-label={translate('item.newTripleObjectAria')}
                  bind:value={newTriple.object}
                  onkeydown={handleNewTripleKeydown}
                />
                <div class="triple-actions">
                  <button
                    type="button"
                    class="triple-action"
                    disabled={!newTripleIsComplete}
                    aria-label={translate('item.newTripleSaveAria')}
                    title={translate('item.tripleSaveTitle')}
                    data-testid="triple-new-save"
                    onclick={() => void saveNewTriple()}
                  >
                    <ActionIcon name="check" size={12} />
                  </button>
                  <button
                    type="button"
                    class="triple-action"
                    aria-label={translate('item.newTripleCancelAria')}
                    title={translate('item.tripleCancelTitle')}
                    data-testid="triple-new-cancel"
                    onclick={cancelCreatingTriple}
                  >
                    <ActionIcon name="close" size={12} />
                  </button>
                </div>
              </li>
            {/if}
          </ul>
        {/if}

        <button
          type="button"
          class="nlp-btn nlp-btn--icon triples-add"
          disabled={creatingTriple}
          aria-label={translate('item.addTriple')}
          title={translate('item.addTriple')}
          data-testid="triple-add"
          onclick={startCreatingTriple}
        >
          <ActionIcon name="add" size={16} />
        </button>

        {#if tripleActionError}
          <p class="error">{tripleActionError}</p>
        {/if}
      </div>
    </div>
  </section>
{/if}

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    box-shadow: var(--shadow-surface);
    container-type: inline-size;
  }

  .analysis-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-top: none;
    border-radius: 0 0 var(--radius-surface) var(--radius-surface);
    overflow: hidden;
    background: var(--surface-card);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.025);
  }

  .analysis-panel--tabbed {
    border-top: 1px solid var(--color-border);
    border-radius: var(--radius-surface);
  }

  .nlp-actions {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--space-1);
  }

  .nlp-btn {
    display: inline-flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    flex: 1 1 25%;
    min-width: 0;
    padding: 6px var(--space-1);
    font-size: var(--font-size-xs);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-card);
    cursor: pointer;
    color: var(--color-text-primary);
    font-family: var(--font-sans);
    text-align: center;
    white-space: nowrap;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .nlp-btn:hover:not(:disabled) {
    border-color: var(--border-panel);
    background: var(--color-accent-faint);
  }

  .nlp-btn:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .nlp-btn:disabled {
    opacity: 0.48;
    cursor: not-allowed;
  }

  :global(.nlp-badge) {
    text-transform: uppercase;
  }

  .entities-section,
  .triples-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .entities-section h4,
  .triples-section h4 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .entity-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-3);
    min-width: 0;
  }

  .entity-editor h5 {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.075em;
  }

  .entity-editor__hint {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .entity-editor__create {
    display: grid;
    grid-template-columns: minmax(0, 35fr) minmax(0, 50fr) max-content;
    gap: var(--space-2);
    align-items: center;
    padding-bottom: var(--space-2);
    min-width: 0;
  }

  .entity-editor__create select,
  .entity-editor__create input {
    box-sizing: border-box;
    width: 100%;
    max-width: 100%;
    min-width: 0;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .entity-editor__create select:focus-visible,
  .entity-editor__create input:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .entity-editor__create .nlp-btn {
    width: 100%;
    flex-direction: row;
    justify-content: center;
    font-size: var(--font-size-sm);
    padding: var(--space-2) var(--space-3);
  }

  .empty-text {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .error,
  .ocr-error {
    color: var(--color-danger);
  }

  .ocr-error {
    font-size: var(--font-size-xs);
  }

  .triples-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .triple-item {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
    /* Ancla de los controles: en lectura flotan sobre el borde derecho, así el
       grid de tres columnas conserva exactamente sus anchos actuales. */
    position: relative;
  }

  /* Editando SÍ hace falta una pista propia para guardar/cancelar: superponer
     controles sobre tres inputs los volvería inalcanzables. */
  .triple-item--editing {
    grid-template-columns: 1fr 1fr 1fr auto;
  }

  .triple-cell {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Solo el predicado se centra: la columna del medio ancla visualmente la
     lectura sujeto → predicado → objeto. Sujeto y objeto siguen alineados a
     la izquierda. */
  .triple-cell--predicate {
    text-align: center;
  }

  .triple-input {
    min-width: 0;
    padding: 0 var(--space-1);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-xs);
    white-space: normal;
  }

  .triple-input:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .triple-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  /* Discretos por defecto: siguen en el DOM y son enfocables con teclado —
     por eso `opacity` y no `display` — pero solo se ven al recorrer la fila. */
  .triple-item:not(.triple-item--editing) .triple-actions {
    position: absolute;
    inset-block: 1px;
    inset-inline-end: var(--space-1);
    padding-inline-start: var(--space-2);
    background: linear-gradient(
      to right,
      transparent,
      var(--color-surface-raised) var(--space-2)
    );
    opacity: 0;
    transition: opacity 120ms ease-out;
  }

  .triple-item:hover .triple-actions,
  .triple-item:focus-within .triple-actions,
  .triple-item--pending .triple-actions {
    opacity: 1;
  }

  .triple-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 var(--space-1);
    min-width: 20px;
    height: 20px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .triple-action:hover:not(:disabled) {
    border-color: var(--color-hairline);
    color: var(--color-text-primary);
  }

  .triple-action:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .triple-action--delete:hover:not(:disabled),
  .triple-action--pending {
    border-color: var(--color-danger);
    color: var(--color-danger);
  }

  /* Controles de agregado: solo el icono, cuadrados y del mismo tamaño que el
     resto de los botones de acción. `.nlp-btn` reparte ancho dentro de la
     grilla NLP, así que acá hay que soltarlo de ese reparto. */
  .nlp-btn--icon {
    flex: 0 0 auto;
    padding: 6px;
    min-width: 32px;
  }

  .triples-add {
    align-self: flex-start;
  }

  .triple-action__label {
    font-size: var(--font-size-xs);
    white-space: nowrap;
  }

  .geo-section {
    margin-top: var(--space-4);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-hairline);
  }

  @container (max-width: 30rem) {
    .nlp-actions {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .entity-editor__create {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr);
    }

    .entity-editor__create .nlp-btn {
      grid-column: 1 / -1;
      justify-self: start;
    }
  }

  @container (max-width: 16rem) {
    .nlp-actions,
    .entity-editor__create {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
