/**
 * Layout algorithms for Relaciones 3D, all on the renderer already in use
 * (3d-force-graph / d3-force-3d), so none adds a dependency:
 *
 * - `force`: today's free forces, the Fruchterman-Reingold family found in
 *   networkx and igraph: links pull, every node repels the same.
 * - `communities`: ForceAtlas2-style (graphology, Gephi): repulsion grows
 *   with a node's connections, so hubs push each other apart and their
 *   neighbours gather around them, which shows groups.
 * - `radial`: 3d-force-graph's `radialout` DAG mode: rings outward from the
 *   subjects that start relations to the objects that receive them. Cycles
 *   are broken for the rings only; every link is still drawn.
 */
import type { GraphRenderSettings } from './analysis-settings'

export const GRAPH_LAYOUTS = ['force', 'communities', 'radial'] as const
export type GraphLayout = (typeof GRAPH_LAYOUTS)[number]

export interface LayoutConfig {
  dagMode: 'radialout' | null
  /** Distance between rings in the radial layout. */
  dagLevelDistance: number
  /** Charge per node (negative repels). */
  charge: (node: { degree: number }) => number
  linkDistance: number
}

export function layoutConfig(layout: GraphLayout, render: GraphRenderSettings): LayoutConfig {
  const base = {
    dagMode: null,
    dagLevelDistance: render.linkDistance * 1.5,
    charge: () => render.chargeStrength,
    linkDistance: render.linkDistance,
  }
  switch (layout) {
    case 'communities':
      return {
        ...base,
        charge: (node) => render.chargeStrength * (1 + node.degree),
        linkDistance: render.linkDistance * 0.6,
      }
    case 'radial':
      return { ...base, dagMode: 'radialout' }
    default:
      return base
  }
}
