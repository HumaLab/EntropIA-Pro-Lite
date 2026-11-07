<script lang="ts">
  /**
   * The collection's relations as a 3D graph (T-46). The renderer (three.js,
   * ~1 MB) loads only the first time the tab is shown, and the graph is built
   * from the triples the batch already extracted; nothing new is computed.
   *
   * Structure settings rebuild the graph from the triples already loaded (no
   * query); render settings are applied to the live instance. Opening a
   * passage saves a snapshot (camera, node positions, selection) on the
   * collection's history entry so «Volver» brings this view back as it was.
   */
  import { onDestroy } from 'svelte'
  import { ActionIcon, IconButton, tooltip } from '@entropia/ui'
  import type { Entity, Triple } from '@entropia/store'
  import { getStore } from '$lib/db'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import { getNavigation } from '$lib/pane-context'
  import {
    buildRelationGraph,
    describeNode,
    linkLabelText,
    key as nameKey,
    type NodeDetails,
    type GraphLink,
    type GraphNode,
    type RelationGraph,
  } from '$lib/relation-graph'
  import { graphStructureKey, type GraphSettings } from '$lib/analysis-settings'
  import {
    buildNodeTargets,
    buildRelationTargets,
    type OccurrenceContext,
    type OccurrenceTarget,
  } from '$lib/relation-occurrences'
  import {
    linkTouchesSelection,
    resolveSelection,
    seedNodePositions,
    type GraphCamera,
    type GraphSelection,
    type GraphViewSnapshot,
    type ResolvedSelection,
    type Vec3,
  } from '$lib/relation-graph-view-state'
  import type { ForceGraph3DInstance } from '3d-force-graph'

  let {
    collectionId,
    active,
    refreshToken,
    settings,
    snapshot = null,
  }: {
    collectionId: string
    active: boolean
    refreshToken: number
    settings: GraphSettings
    /** Saved when the user left for a passage; consumed by the first build. */
    snapshot?: GraphViewSnapshot | null
  } = $props()

  const currentLocale = locale
  let status = $state<'idle' | 'loading' | 'ready' | 'error'>('idle')
  let graph = $state<RelationGraph | null>(null)
  let container: HTMLDivElement | undefined = $state()
  let instance: ForceGraph3DInstance | null = null
  let observer: ResizeObserver | null = null
  let loadedFor = ''
  let builtStructureKey = ''
  // a snapshot only restores the first build
  // svelte-ignore state_referenced_locally
  let pendingSnapshot: GraphViewSnapshot | null = snapshot
  let appliedPhysics = ''
  const navigation = getNavigation()

  // What a click needs, kept from the last load: the raw rows (with their
  // item and asset, so a click can reach the passage), document titles, and
  // the entity type each name was tagged with (first one wins).
  let tripleRows: Triple[] = []
  let entityRows: Entity[] = []
  let titles = new Map<string, string>()
  let types = $state.raw(new Map<string, string>())
  let selected = $state<ResolvedSelection | null>(null)
  const nodeDetails = $derived<NodeDetails | null>(
    selected?.kind === 'node' ? describeNode(selected.node.id, tripleRows) : null
  )

  async function loadRows(): Promise<void> {
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
    tripleRows = perItem.flat()
    entityRows = entities.flat()
    titles = new Map(items.map((item) => [item.id, item.title]))
    types = new Map()
    for (const entity of entityRows) {
      const k = nameKey(entity.value)
      if (!types.has(k)) types.set(k, entity.entityType)
    }
  }

  function typeLabel(type: string): string {
    return t(`item.entityType.${type}` as I18nKey)
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

  const endpointId = (end: unknown): string =>
    typeof end === 'object' && end !== null ? (end as GraphNode).id : String(end)

  function selectLink(link: object) {
    const source = endpointId((link as GraphLink).source)
    const target = endpointId((link as GraphLink).target)
    selected = graph ? resolveSelection(graph, { kind: 'link', source, target }) : null
  }

  function nodeLabel(id: string): string {
    return graph?.nodes.find((node) => node.id === id)?.label ?? id
  }

  function selectionHeading(current: ResolvedSelection): string {
    return current.kind === 'node'
      ? current.node.label
      : `${nodeLabel(current.link.source)} → ${nodeLabel(current.link.target)}`
  }

  function selection(): GraphSelection | null {
    if (selected?.kind === 'node') return { kind: 'node', id: selected.node.id }
    if (selected?.kind === 'link') {
      return { kind: 'link', source: selected.link.source, target: selected.link.target }
    }
    return null
  }

  // ── Snapshot ─────────────────────────────────────────────────────────────
  type OrbitControls = {
    target: { x: number; y: number; z: number; set: (...v: number[]) => void }
    update?: () => void
    autoRotate?: boolean
  }

  function captureSnapshot(): GraphViewSnapshot | null {
    if (!instance || !graph) return null
    const positions: Record<string, Vec3> = {}
    for (const node of instance.graphData().nodes as Array<
      GraphNode & Partial<Record<'x' | 'y' | 'z', number>>
    >) {
      if (node.x === undefined || node.y === undefined || node.z === undefined) return null
      positions[node.id] = [node.x, node.y, node.z]
    }
    const camera = instance.camera() as unknown as {
      position: { x: number; y: number; z: number }
      up: { x: number; y: number; z: number }
      zoom: number
    }
    const controls = instance.controls() as unknown as OrbitControls
    return {
      structureKey: builtStructureKey,
      positions,
      camera: {
        position: [camera.position.x, camera.position.y, camera.position.z],
        target: [controls.target.x, controls.target.y, controls.target.z],
        up: [camera.up.x, camera.up.y, camera.up.z],
        zoom: camera.zoom,
      },
      selection: selection(),
    }
  }

  function restoreCamera(saved: GraphCamera) {
    if (!instance) return
    const camera = instance.camera() as unknown as {
      up: { set: (...v: number[]) => void }
      zoom: number
      updateProjectionMatrix: () => void
    }
    const controls = instance.controls() as unknown as OrbitControls
    camera.up.set(...saved.up)
    camera.zoom = saved.zoom
    camera.updateProjectionMatrix()
    controls.target.set(...saved.target)
    const [x, y, z] = saved.position
    instance.cameraPosition(
      { x, y, z },
      { x: saved.target[0], y: saved.target[1], z: saved.target[2] },
      0
    )
    controls.update?.()
  }

  // ── Opening passages ─────────────────────────────────────────────────────
  function occurrenceContext(itemId: string): OccurrenceContext {
    const current = navigation.current
    return {
      collectionId,
      collectionName: current.name === 'collection' ? current.collectionName : '',
      itemTitle: titles.get(itemId) ?? '',
    }
  }

  function nodeTargets(id: string, labelText: string): OccurrenceTarget[] {
    const rows = tripleRows.filter(
      (row) => nameKey(row.subject) === id || nameKey(row.object) === id
    )
    return buildNodeTargets(labelText, entityRows, rows, occurrenceContext)
  }

  function relationTargets(
    subject: string,
    object: string,
    predicate?: string
  ): OccurrenceTarget[] {
    const rows = predicate
      ? tripleRows.filter((row) => nameKey(row.predicate) === nameKey(predicate))
      : tripleRows
    return buildRelationTargets(subject, object, rows, occurrenceContext)
  }

  function openOccurrences(kind: 'entity' | 'relation', targets: OccurrenceTarget[]) {
    const first = targets[0]
    if (!first) return
    const current = navigation.current
    if (current.name === 'collection') {
      navigation.replace({ ...current, analysis: { tab: 'graph', graph: captureSnapshot() } })
    }
    navigation.navigate({
      name: 'item',
      collectionId: first.collectionId,
      collectionName: first.collectionName,
      itemId: first.itemId,
      itemTitle: first.itemTitle,
      assetId: first.assetId,
      occurrenceSession: { kind, targets, index: 0 },
    })
  }

  // ── Building and drawing ─────────────────────────────────────────────────
  // ── Highlighting the selection ─────────────────────────────────────────
  /** The renderer's sphere for a node: a three.js mesh, typed by what is used. */
  interface NodeMesh {
    geometry: unknown
    material: { constructor: new (params: Record<string, unknown>) => unknown }
    constructor: new (geometry: unknown, material: unknown) => OutlineMesh
    add: (child: OutlineMesh) => void
    remove: (child: OutlineMesh) => void
  }
  interface OutlineMesh {
    scale: { setScalar: (value: number) => void }
  }
  let outline: { parent: NodeMesh; mesh: OutlineMesh } | null = null

  function removeOutline() {
    outline?.parent.remove(outline.mesh)
    outline = null
  }

  /**
   * A contrasting rim around the selected node: a slightly larger copy of its
   * sphere drawn from the back faces only. Built from the sphere's own
   * constructors, so the view needs no direct three.js dependency.
   */
  /**
   * Keeps the rim on the selected node's current sphere. The renderer builds
   * node objects a few frames after the data arrives (and may rebuild them),
   * notably when a restored graph comes back, so a single attempt on the next
   * frame is not enough: this rechecks every frame for about two seconds.
   */
  let outlineWatch = 0
  function watchOutline(color: string) {
    cancelAnimationFrame(outlineWatch)
    let frames = 0
    const step = () => {
      if (!instance) return
      ensureOutline(color)
      if (++frames < 120) outlineWatch = requestAnimationFrame(step)
    }
    outlineWatch = requestAnimationFrame(step)
  }

  function ensureOutline(color: string) {
    if (!instance || selected?.kind !== 'node') {
      removeOutline()
      return
    }
    const id = selected.node.id
    const node = instance.graphData().nodes.find((n) => (n as GraphNode).id === id) as
      { __threeObj?: NodeMesh } | undefined
    const sphere = node?.__threeObj
    if (outline && outline.parent === sphere) return
    outlineSelected(sphere, color)
  }

  function outlineSelected(sphere: NodeMesh | undefined, color: string) {
    removeOutline()
    if (!sphere?.geometry || !sphere.material) return
    const Material = sphere.material.constructor
    const Mesh = sphere.constructor
    // side 1 = THREE.BackSide: only the rim around the sphere shows.
    const rim = new Mesh(
      sphere.geometry,
      new Material({ color, side: 1, transparent: true, opacity: 0.95 })
    )
    rim.scale.setScalar(1.35)
    sphere.add(rim)
    outline = { parent: sphere, mesh: rim }
  }

  /**
   * The selected node turns bright with a contrasting rim, and its links (or
   * the selected link) take the contrasting colour, a little width and moving
   * particles, which stay visible whatever the global link opacity is.
   */
  function applyHighlight() {
    if (!instance) return
    const accent = token('--color-accent', '#4f7cff')
    const bright = token('--color-text-primary', '#ffffff')
    const contrast = token('--color-warning', '#f5a524')
    const muted = token('--color-text-secondary', '#888')
    const current = selection()
    const nodeId = current?.kind === 'node' ? current.id : null
    const lit = (link: object) =>
      linkTouchesSelection(link as { source: unknown; target: unknown }, current)
    instance
      .nodeColor((n) => ((n as GraphNode).id === nodeId ? bright : accent))
      .linkColor((l) => (lit(l) ? contrast : muted))
      .linkWidth((l) => (lit(l) ? 1.2 : 0))
      .linkDirectionalParticles((l) => (lit(l) ? 2 : 0))
      .linkDirectionalParticleColor(() => contrast)
    watchOutline(contrast)
  }

  function draw() {
    if (!instance || !graph) return
    const structureKey = graphStructureKey(settings)
    const restore = pendingSnapshot
    pendingSnapshot = null
    const seeded = seedNodePositions(graph, restore, structureKey)
    builtStructureKey = structureKey
    // A restored layout must not move: no ticks until the pins are released.
    instance.cooldownTicks(seeded.restored ? 0 : Infinity)
    let released = !seeded.restored
    instance.onEngineStop(() => {
      if (!instance) return
      if (!released) {
        released = true
        for (const node of instance.graphData().nodes as Array<Record<string, unknown>>) {
          delete node.fx
          delete node.fy
          delete node.fz
        }
        instance.cooldownTicks(Infinity)
      }
      // Node objects are settled now: make sure the selection is still styled.
      applyHighlight()
    })
    // The library rewrites source/target into node objects: hand it copies.
    instance.graphData({ nodes: seeded.nodes, links: graph.links.map((l) => ({ ...l })) })
    selected = resolveSelection(graph, restore?.selection ?? selection())
    if (seeded.restored && restore?.camera) restoreCamera(restore.camera)
    applyHighlight()
  }

  function rebuild() {
    graph = buildRelationGraph(tripleRows, settings.structure)
    draw()
  }

  async function render() {
    const loadKey = `${collectionId}:${refreshToken}`
    if (!container || loadedFor === loadKey) return
    loadedFor = loadKey
    status = 'loading'
    try {
      const [{ default: ForceGraph3D }] = await Promise.all([import('3d-force-graph'), loadRows()])
      if (loadedFor !== loadKey || !container) return
      selected = null
      graph = buildRelationGraph(tripleRows, settings.structure)
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
      instance
        .width(container.clientWidth)
        .height(container.clientHeight)
        .backgroundColor(token('--color-surface', '#111'))
        .showNavInfo(false)
        .nodeLabel((n) => label((n as GraphNode).label))
        .onNodeClick((n) => {
          selected = graph
            ? resolveSelection(graph, { kind: 'node', id: (n as GraphNode).id })
            : null
        })
        .onLinkClick((l) => selectLink(l))
        .onBackgroundClick(() => (selected = null))
        .linkDirectionalArrowRelPos(1)
        .linkDirectionalParticleWidth(2)
      appliedPhysics = ''
      applyRender(settings)
      draw()
    } catch (failure) {
      console.warn('[RelationGraph3D] failed to build the graph:', failure)
      status = 'error'
      loadedFor = ''
    }
  }

  /** Render settings change the live graph only; physics changes reheat it. */
  function applyRender(current: GraphSettings) {
    if (!instance) return
    const render = current.render
    instance
      .nodeVal((n) => render.nodeSizeScale * (1 + (n as GraphNode).degree))
      .linkOpacity(render.linkOpacity)
      .linkDirectionalArrowLength(render.showArrows ? 3 : 0)
      .linkLabel((l) => (render.showLinkLabels ? label(linkLabelText(l as GraphLink)) : ''))
      .cooldownTime(render.cooldownTime)
    const controls = instance.controls() as unknown as OrbitControls
    controls.autoRotate = render.autoRotate
    const physics = `${render.linkDistance}:${render.chargeStrength}`
    if (physics !== appliedPhysics) {
      const first = appliedPhysics === ''
      appliedPhysics = physics
      ;(instance.d3Force('link') as { distance?: (d: number) => unknown } | undefined)?.distance?.(
        render.linkDistance
      )
      ;(
        instance.d3Force('charge') as { strength?: (s: number) => unknown } | undefined
      )?.strength?.(render.chargeStrength)
      // The first application precedes the layout; a later one asks for it.
      if (!first) instance.d3ReheatSimulation()
    }
  }

  // Built when the tab is first shown (a hidden container has no size), and
  // again when the collection or its data changes: render() reads both before
  // its first await, so the effect tracks them.
  $effect(() => {
    if (active && container) void render()
  })

  // Structure settings: rebuild from the loaded rows, never re-query.
  $effect(() => {
    const structureKey = graphStructureKey(settings)
    if (status === 'ready' && instance && structureKey !== builtStructureKey) rebuild()
  })

  // Render settings: applied to the live instance.
  $effect(() => {
    const current = settings
    void current.render
    if (status === 'ready') applyRender(current)
  })

  // Selection changes (clicks, closing the panel) restyle the live graph.
  $effect(() => {
    void selected
    if (status === 'ready') applyHighlight()
  })

  onDestroy(() => {
    cancelAnimationFrame(outlineWatch)
    removeOutline()
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
      {@const heading = selectionHeading(selected)}
      <section class="relation-graph__details" aria-label={heading}>
        <header class="relation-graph__details-head">
          <div class="relation-graph__details-title">
            <h3>{heading}</h3>
            {#if selected.kind === 'node' && types.get(selected.node.id)}
              <span class="relation-graph__type">{typeLabel(types.get(selected.node.id)!)}</span>
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

        {#if selected.kind === 'node'}
          {@const node = selected.node}
          {@const targets = nodeTargets(node.id, node.label)}
          <button
            type="button"
            class="relation-graph__open"
            disabled={targets.length === 0}
            onclick={() => openOccurrences('entity', targets)}
          >
            <ActionIcon name="external-link" size={12} />
            {t('collectionAnalysis.graphOpenEntity', { count: targets.length })}
          </button>
          <h4>{t('collectionAnalysis.graphRelations')}</h4>
          <ul>
            {#each nodeDetails?.relations ?? [] as relation, index (index)}
              {@const relationTargetsFor = relationTargets(
                relation.subject,
                relation.object,
                relation.predicate
              )}
              <li>
                <button
                  type="button"
                  class="relation-graph__relation"
                  disabled={relationTargetsFor.length === 0}
                  use:tooltip={t('collectionAnalysis.graphOpenRelation')}
                  onclick={() => openOccurrences('relation', relationTargetsFor)}
                >
                  <span class="relation-graph__subject">{relation.subject}</span>
                  <em>{relation.predicate}</em>
                  <span class="relation-graph__object">{relation.object}</span>
                  {#if relation.count > 1}
                    <span class="relation-graph__count"
                      >{t('collectionAnalysis.graphRelationCount', { count: relation.count })}</span
                    >
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          {@const link = selected.link}
          {@const sourceLabel = nodeLabel(link.source)}
          {@const targetLabel = nodeLabel(link.target)}
          {@const allTargets = relationTargets(sourceLabel, targetLabel)}
          <button
            type="button"
            class="relation-graph__open"
            disabled={allTargets.length === 0}
            onclick={() => openOccurrences('relation', allTargets)}
          >
            <ActionIcon name="external-link" size={12} />
            {t('collectionAnalysis.graphOpenLink', { count: allTargets.length })}
          </button>
          <h4>{t('collectionAnalysis.graphRelations')}</h4>
          <ul>
            {#each link.predicates as predicate, index (predicate)}
              {@const predicateTargets = relationTargets(sourceLabel, targetLabel, predicate)}
              {@const count = link.counts[index] ?? 1}
              <li>
                <button
                  type="button"
                  class="relation-graph__relation"
                  disabled={predicateTargets.length === 0}
                  use:tooltip={t('collectionAnalysis.graphOpenRelation')}
                  onclick={() => openOccurrences('relation', predicateTargets)}
                >
                  <span class="relation-graph__subject">{sourceLabel}</span>
                  <em>{predicate}</em>
                  <span class="relation-graph__object">{targetLabel}</span>
                  {#if count > 1}
                    <span class="relation-graph__count"
                      >{t('collectionAnalysis.graphRelationCount', { count })}</span
                    >
                  {/if}
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
  /* Below 20rem the panel, the summary and the canvas no longer fit side by
     side usefully; the pane scrolls horizontally instead of squeezing them. */
  .relation-graph {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    height: 100%;
    min-height: 420px;
    min-width: 20rem;
  }

  .relation-graph__note {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .relation-graph__stage {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  /* Half the visualization, on a half-transparent background so the graph
     stays visible behind it. */
  .relation-graph__details {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    box-sizing: border-box;
    width: 50%;
    max-height: calc(100% - 2 * var(--space-2));
    overflow-y: auto;
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-surface);
    background: color-mix(in srgb, var(--color-surface-raised) 50%, transparent);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  /* Under 30rem half the width is too narrow to read: the panel spans the
     stage at the bottom and leaves the top half of the graph uncovered. */
  @container (max-width: 30rem) {
    .relation-graph__details {
      top: auto;
      right: var(--space-2);
      bottom: var(--space-2);
      width: auto;
      max-height: 50%;
    }
  }

  .relation-graph__details-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .relation-graph__details-title {
    min-width: 0;
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

  .relation-graph__open,
  .relation-graph__relation {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .relation-graph__open {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-top: var(--space-2);
    color: var(--color-accent);
  }

  .relation-graph__relation:hover:not(:disabled),
  .relation-graph__open:hover:not(:disabled) {
    text-decoration: underline;
  }

  .relation-graph__relation:focus-visible,
  .relation-graph__open:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }

  .relation-graph__open:disabled,
  .relation-graph__relation:disabled {
    cursor: default;
    opacity: 0.6;
  }

  /* How often the relation is stated: read, never drawn as a thicker link. */
  .relation-graph__count {
    margin-left: var(--space-1);
    padding: 0 var(--space-1);
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--color-text-secondary) 20%, transparent);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .relation-graph__subject,
  .relation-graph__object {
    font-weight: var(--font-weight-semibold);
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
