<script lang="ts">
  import { EntityViewer, StatusBadge, type MapMarker, type StatusBadgeVariant } from '@entropia/ui'
  import type { Entity } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { ItemNlpState } from '$lib/nlp'
  import ItemMapViewer from './ItemMapViewer.svelte'

  type EditableEntityType = 'person' | 'organization' | 'place' | 'misc' | 'date'
  type SemanticTriple = { subject: string; predicate: string; object: string }

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
    selectedAssetIndex,
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
    onSaveMapLocation,
    onResetMapLocation,
  }: {
    assetsCount: number
    selectedAsset: boolean
    selectedAssetIndex: number
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
    onSaveMapLocation: (entityId: string, latitude: number, longitude: number) => void | Promise<void>
    onResetMapLocation: (entityId: string) => void | Promise<void>
  } = $props()

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
            <button type="button" class="nlp-btn" onclick={onCreateEntity}
              >{translate('item.addEntity')}</button
            >
          </div>

          {#if entityActionError}
            <p class="error">{entityActionError}</p>
          {/if}
        </div>
      </div>

      <div class="triples-section">
        <h4>
          {translate('item.semanticTriples')}{#if assetsCount > 1}
            {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
        </h4>
        {#if triples.length === 0}
          <p class="empty-text">
            {translate('item.noTriples', {
              suffix: assetsCount > 1 ? translate('item.noTriplesPageSuffix') : '',
            })}
          </p>
        {:else}
          <ul class="triples-list">
            {#each triples as triple, i (`${triple.subject}-${triple.predicate}-${triple.object}-${i}`)}
              <li class="triple-item">
                <span class="triple-cell">{triple.subject}</span>
                <span class="triple-cell">{triple.predicate}</span>
                <span class="triple-cell">{triple.object}</span>
              </li>
            {/each}
          </ul>
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

  .entities-section h4 {
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
  }

  .triple-cell {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
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
    }
  }

  @container (max-width: 16rem) {
    .nlp-actions,
    .entity-editor__create {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
