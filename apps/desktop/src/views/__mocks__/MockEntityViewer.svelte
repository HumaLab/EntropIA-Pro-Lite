<script lang="ts">
  let {
    entities = [],
    editingEntityId = null,
    editingValue = '',
    creatableTypes = [],
    newEntityType,
    newEntityValue = '',
    labels,
    onentityclick = () => {},
    onhighlight = () => {},
    oneditvaluechange = () => {},
    onsaveentity = () => {},
    oncancelentityedit = () => {},
    ondeleteentity = () => {},
    onnewentitytypechange = () => {},
    onnewentityvaluechange = () => {},
    oncreateentity = () => false,
  } = $props()

  // El alta real vive detrás del chip +: el mock reproduce ese gesto para que
  // las pruebas de integración recorran el mismo camino que la interfaz.
  let creating = $state(false)
</script>

<div data-testid="mock-entity-viewer">
  {#each entities as entity (entity.id)}
    {#if editingEntityId === entity.id}
      <div data-testid={`mock-entity-editing-${entity.id}`}>
        <input
          type="text"
          aria-label={labels?.editValueAria ?? 'Edit entity value'}
          value={editingValue}
          oninput={(event) => oneditvaluechange(event.currentTarget.value)}
          onkeydown={(event) => {
            if (event.key === 'Enter') onsaveentity(entity.id, event.currentTarget.value)
            if (event.key === 'Escape') oncancelentityedit()
          }}
          onblur={(event) => onsaveentity(entity.id, event.currentTarget.value)}
        />
      </div>
    {:else}
      <button
        type="button"
        data-testid={`mock-entity-${entity.id}`}
        onclick={() => {
          if (entity.startOffset != null && entity.endOffset != null) {
            onhighlight({ startOffset: entity.startOffset, endOffset: entity.endOffset })
          }
          onentityclick(entity)
        }}
      >
        {entity.value}
      </button>
      <button
        type="button"
        aria-label={(labels?.deleteEntityAria ?? ((value: string) => `Delete entity ${value}`))(
          entity.value
        )}
        data-testid={`mock-entity-delete-${entity.id}`}
        onclick={() => ondeleteentity(entity.id)}
      >
        delete
      </button>
    {/if}
  {/each}

  {#if creatableTypes.length > 0}
    {#if creating}
      <div data-testid="mock-entity-new">
        <select
          aria-label={labels?.newEntityTypeAria ?? 'New entity type'}
          value={newEntityType ?? creatableTypes[0]}
          onchange={(event) => onnewentitytypechange(event.currentTarget.value)}
        >
          {#each creatableTypes as type (type)}
            <option value={type}>{type}</option>
          {/each}
        </select>
        <input
          type="text"
          aria-label={labels?.newEntityValueAria ?? 'New entity value'}
          value={newEntityValue}
          oninput={(event) => onnewentityvaluechange(event.currentTarget.value)}
        />
        <button
          type="button"
          aria-label={labels?.saveNewEntityAria ?? 'Save the new entity'}
          data-testid="mock-entity-new-save"
          onclick={async () => {
            if (await oncreateentity()) creating = false
          }}
        >
          save
        </button>
      </div>
    {:else}
      <button
        type="button"
        aria-label={labels?.addEntity ?? 'Add entity'}
        data-testid="mock-entity-add"
        onclick={() => {
          creating = true
        }}
      >
        add
      </button>
    {/if}
  {/if}
</div>
