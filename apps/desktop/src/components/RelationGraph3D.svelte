<script lang="ts">
  /**
   * The collection's relations as a 3D graph (T-46). The renderer (three.js,
   * ~1 MB) loads only the first time the tab is shown, and the graph is built
   * from the triples the batch already extracted; nothing new is computed.
   */
  import { onDestroy } from 'svelte'
  import { ActionIcon, IconButton } from '@entropia/ui'
  import { getStore } from '$lib/db'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import { getNavigation } from '$lib/pane-context'
  import {
    buildRelationGraph,
    describeNode,
    key as nameKey,
    type NodeDetails,
    type RelationTriple,
    type GraphLink,
    type GraphNode,
    type RelationGraph,
  } from '$lib/relation-graph'
  import type { ForceGraph3DInstance } from '3d-force-graph'

  let {
    collectionId,
    active,
    refreshToken,
  }: { collectionId: string; active: boolean; refreshToken: number } = $props()

  const currentLocale = locale
  let status = $state<'idle' | 'loading' | 'ready' | 'error'>('idle')
  let graph = $state<RelationGraph | null>(null)
  let container: HTMLDivElement | undefined = $state()
  let instance: ForceGraph3DInstance | null = null
  let observer: ResizeObserver | null = null
  let loadedFor = ''
  const navigation = getNavigation()

  // What a click needs, kept from the last load: the raw triples, document
  // titles, and the entity type each name was tagged with (first one wins).
  let triples: RelationTriple[] = []
  let titles = $state.raw(new Map<string, string>())
  let types = new Map<string, string>()
  let selected = $state<(NodeDetails & { label: string; type?: string }) | null>(null)

  async function loadTriples(): Promise<RelationGraph> {
    const store = getStore()
    const items = await store.items.findByCollection(collectionId)
    // ponytail: one query per item, like the JSON export; a collection-wide
    // query in the store if large collections make this slow.
    const [perItem, entities] = await Promise.all([
      Promise.all(
        items.map((item) => store.triples?.findByItemId?.(item.id) ?? Promise.resolve([]))
      ),
      Promise.all(
        items.map((item) => store.entities?.findByItemId?.(item.id) ?? Promise.resolve([]))
      ),
    ])
    triples = perItem.flat()
    titles = new Map(items.map((item) => [item.id, item.title]))
    types = new Map()
    for (const entity of entities.flat()) {
      const k = nameKey(entity.value)
      if (!types.has(k)) types.set(k, entity.entityType)
    }
    selected = null
    return buildRelationGraph(triples)
  }

  function select(node: GraphNode) {
    selected = { ...describeNode(node.id, triples), label: node.label, type: types.get(node.id) }
  }

  function typeLabel(type: string): string {
    return t(`item.entityType.${type}` as I18nKey)
  }

  function openDocument(itemId: string) {
    const current = navigation.current
    navigation.navigate({
      name: 'item',
      collectionId,
      collectionName: current.name === 'collection' ? current.collectionName : '',
      itemId,
      itemTitle: titles.get(itemId) ?? '',
    })
  }

  /** Text labels as elements: the library would render a string as HTML, and
   *  these strings come from the documents. */
  function label(text: string): HTMLElement {
    const el = document.createElement('span')
    el.textContent = text
    return el
  }

  function token(name: string, fallback: string): string {
    if (!container) return fallback
    return getComputedStyle(container).getPropertyValue(name).trim() || fallback
  }

  async function render() {
    const key = `${collectionId}:${refreshToken}`
    if (!container || loadedFor === key) return
    loadedFor = key
    status = 'loading'
    try {
      const [{ default: ForceGraph3D }, built] = await Promise.all([
        import('3d-force-graph'),
        loadTriples(),
      ])
      if (loadedFor !== key || !container) return
      graph = built
      status = 'ready'
      if (!instance) {
        instance = new ForceGraph3D(container, { controlType: 'orbit' })
        observer = new ResizeObserver(() => {
          if (container && instance) {
            instance.width(container.clientWidth).height(container.clientHeight)
          }
        })
        observer.observe(container)
      }
      const accent = token('--color-accent', '#4f7cff')
      instance
        .width(container.clientWidth)
        .height(container.clientHeight)
        .backgroundColor(token('--color-surface', '#111'))
        .showNavInfo(false)
        .nodeLabel((n) => label((n as GraphNode).label))
        .nodeVal((n) => 1 + (n as GraphNode).degree)
        .nodeColor(() => accent)
        .onNodeClick((n) => select(n as GraphNode))
        .onBackgroundClick(() => (selected = null))
        .linkLabel((l) => label((l as GraphLink).predicates.join(' · ')))
        .linkColor(() => token('--color-text-secondary', '#888'))
        .linkDirectionalArrowLength(3)
        .linkDirectionalArrowRelPos(1)
        // The library rewrites source/target into node objects: hand it copies.
        .graphData({
          nodes: built.nodes.map((n) => ({ ...n })),
          links: built.links.map((l) => ({ ...l })),
        })
    } catch (failure) {
      console.warn('[RelationGraph3D] failed to build the graph:', failure)
      status = 'error'
      loadedFor = ''
    }
  }

  // Built when the tab is first shown (a hidden container has no size), and
  // again when the collection or its data changes: render() reads both before
  // its first await, so the effect tracks them.
  $effect(() => {
    if (active && container) void render()
  })

  onDestroy(() => {
    observer?.disconnect()
    instance?._destructor()
    instance = null
  })
</script>

<div class="relation-graph">
  {#if status === 'loading'}
    <p class="relation-graph__note">{$currentLocale && t('collectionAnalysis.graphLoading')}</p>
  {:else if status === 'error'}
    <p class="relation-graph__note">{$currentLocale && t('collectionAnalysis.graphError')}</p>
  {:else if status === 'ready' && graph && graph.nodes.length === 0}
    <p class="relation-graph__note">{$currentLocale && t('collectionAnalysis.graphEmpty')}</p>
  {:else if status === 'ready' && graph}
    <p class="relation-graph__note">
      {$currentLocale &&
        t('collectionAnalysis.graphSummary', {
          nodes: graph.nodes.length,
          links: graph.links.length,
        })}
      {#if graph.hiddenNodes > 0}
        {t('collectionAnalysis.graphHidden', { count: graph.hiddenNodes })}
      {/if}
    </p>
  {/if}
  <div class="relation-graph__stage">
    <div
      class="relation-graph__canvas"
      class:is-empty={status !== 'ready' || !graph?.nodes.length}
      bind:this={container}
    ></div>
    {#if selected}
      <section class="relation-graph__details" aria-label={selected.label}>
        <header class="relation-graph__details-head">
          <div>
            <h3>{selected.label}</h3>
            {#if selected.type}
              <span class="relation-graph__type">{typeLabel(selected.type)}</span>
            {/if}
          </div>
          <IconButton
            size="sm"
            variant="ghost"
            label={t('collectionAnalysis.graphClose')}
            onclick={() => (selected = null)}
          >
            <ActionIcon name="close" size={14} />
          </IconButton>
        </header>
        <h4>{t('collectionAnalysis.graphRelations')}</h4>
        <ul>
          {#each selected.relations as relation, index (index)}
            <li>{relation.subject} <em>{relation.predicate}</em> {relation.object}</li>
          {/each}
        </ul>
        {#if selected.itemIds.length > 0}
          <h4>{t('collectionAnalysis.graphDocuments')}</h4>
          <ul>
            {#each selected.itemIds as itemId (itemId)}
              <li>
                <button
                  type="button"
                  class="relation-graph__doc"
                  onclick={() => openDocument(itemId)}
                >
                  {titles.get(itemId) ?? itemId}
                  <ActionIcon name="external-link" size={12} />
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}
  </div>
</div>

<style>
  .relation-graph {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    height: 100%;
    min-height: 420px;
  }

  .relation-graph__note {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .relation-graph__stage {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
  }

  .relation-graph__details {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    width: min(320px, calc(100% - 2 * var(--space-2)));
    max-height: calc(100% - 2 * var(--space-2));
    overflow-y: auto;
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-surface);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-md);
    font-size: var(--font-size-sm);
  }

  .relation-graph__details-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .relation-graph__details h3 {
    font-size: var(--font-size-md);
  }

  .relation-graph__details h4 {
    margin-top: var(--space-3);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
  }

  .relation-graph__details ul {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: var(--space-1) 0 0;
    padding: 0;
    list-style: none;
  }

  .relation-graph__details em {
    color: var(--color-text-secondary);
  }

  .relation-graph__type {
    color: var(--color-text-secondary);
  }

  .relation-graph__doc {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-accent);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .relation-graph__doc:hover {
    text-decoration: underline;
  }

  .relation-graph__canvas {
    flex: 1;
    min-height: 380px;
    border-radius: var(--radius-surface);
    overflow: hidden;
  }

  .relation-graph__canvas.is-empty {
    visibility: hidden;
  }

  /* The library's hover label sits outside Svelte's scope. */
  .relation-graph :global(.scene-tooltip) {
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
  }
</style>
