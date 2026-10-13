<script lang="ts">
  /**
   * The collection's relations as a 3D graph (T-46). The renderer (three.js,
   * ~1 MB) loads only the first time the tab is shown, and the graph is built
   * from the triples the batch already extracted; nothing new is computed.
   */
  import { onDestroy } from 'svelte'
  import { getStore } from '$lib/db'
  import { locale, t } from '$lib/i18n'
  import {
    buildRelationGraph,
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

  async function loadTriples(): Promise<RelationGraph> {
    const store = getStore()
    const items = await store.items.findByCollection(collectionId)
    // ponytail: one query per item, like the JSON export; a collection-wide
    // query in the store if large collections make this slow.
    const perItem = await Promise.all(
      items.map((item) => store.triples?.findByItemId?.(item.id) ?? Promise.resolve([]))
    )
    return buildRelationGraph(perItem.flat())
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
  <div
    class="relation-graph__canvas"
    class:is-empty={status !== 'ready' || !graph?.nodes.length}
    bind:this={container}
  ></div>
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
