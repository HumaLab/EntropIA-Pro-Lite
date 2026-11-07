/**
 * What «Volver» needs to bring Relaciones 3D back exactly as it was left: the
 * analysis tab, every node where it sat, the camera, and the selection. It
 * travels on the collection's own history entry (navigation is in memory, per
 * pane), so leaving for a passage and coming back is one Back, and nothing is
 * stored once the entry is gone.
 */
import type { GraphLink, GraphNode, RelationGraph } from './relation-graph'

export type Vec3 = [number, number, number]

export type GraphSelection =
  { kind: 'node'; id: string } | { kind: 'link'; source: string; target: string }

export type ResolvedSelection =
  { kind: 'node'; node: GraphNode } | { kind: 'link'; link: GraphLink }

export interface GraphCamera {
  position: Vec3
  /** The orbit centre the camera looks at. */
  target: Vec3
  up: Vec3
  zoom: number
}

export interface GraphViewSnapshot {
  /** Identity of the graph the layout was computed for (settings and expanded entities). */
  structureKey: string
  positions: Record<string, Vec3>
  camera: GraphCamera | null
  selection: GraphSelection | null
  /** Entities the user had expanded past the caps. */
  expanded?: string[]
}

export type AnalysisTab = 'viz' | 'graph' | 'settings'

/** Saved on the collection view entry when the user leaves from the analysis panel. */
export interface CollectionAnalysisRestore {
  tab: AnalysisTab
  graph: GraphViewSnapshot | null
}

export type SeededNode = GraphNode & {
  x?: number
  y?: number
  z?: number
  fx?: number
  fy?: number
  fz?: number
}

/**
 * Copies of the graph's nodes for the renderer, pinned at the saved positions
 * when the snapshot belongs to this very structure. Anything else (filters
 * changed in between, a node without a position) lays out fresh: a partial
 * restore would put the new nodes on top of the old ones.
 */
export function seedNodePositions(
  graph: RelationGraph,
  snapshot: GraphViewSnapshot | null,
  structureKey: string
): { nodes: SeededNode[]; restored: boolean } {
  const positions = snapshot?.structureKey === structureKey ? snapshot.positions : null
  const restored = !!positions && graph.nodes.every((node) => positions[node.id] !== undefined)
  return {
    restored,
    nodes: graph.nodes.map((node) => {
      if (!restored) return { ...node }
      const [x, y, z] = positions![node.id]!
      return { ...node, x, y, z, fx: x, fy: y, fz: z }
    }),
  }
}

/** The saved selection in the rebuilt graph, or null when it is no longer shown. */
export function resolveSelection(
  graph: RelationGraph,
  selection: GraphSelection | null
): ResolvedSelection | null {
  if (!selection) return null
  if (selection.kind === 'node') {
    const node = graph.nodes.find(({ id }) => id === selection.id)
    return node ? { kind: 'node', node } : null
  }
  const link = graph.links.find(
    ({ source, target }) => source === selection.source && target === selection.target
  )
  return link ? { kind: 'link', link } : null
}

/** An endpoint id, whether the renderer still holds the id or has swapped in the node. */
const endpointId = (end: unknown): string =>
  typeof end === 'object' && end !== null ? String((end as { id: unknown }).id) : String(end)

/** Whether a link belongs to the selection: any link of a selected node, or the selected link. */
export function linkTouchesSelection(
  link: { source: unknown; target: unknown },
  selection: GraphSelection | null
): boolean {
  if (!selection) return false
  const source = endpointId(link.source)
  const target = endpointId(link.target)
  return selection.kind === 'node'
    ? source === selection.id || target === selection.id
    : source === selection.source && target === selection.target
}
