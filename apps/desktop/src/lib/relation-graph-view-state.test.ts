import { describe, expect, it } from 'vitest'
import {
  linkTouchesSelection,
  resolveSelection,
  seedNodePositions,
  type GraphViewSnapshot,
} from './relation-graph-view-state'
import type { RelationGraph } from './relation-graph'

const graph: RelationGraph = {
  nodes: [
    { id: 'a', label: 'A', degree: 1 },
    { id: 'b', label: 'B', degree: 1 },
  ],
  links: [{ source: 'a', target: 'b', predicates: ['ve'] }],
  hiddenNodes: 0,
} as RelationGraph

const snapshot: GraphViewSnapshot = {
  structureKey: 'k1',
  positions: { a: [1, 2, 3], b: [4, 5, 6] },
  camera: { position: [0, 0, 100], target: [1, 1, 1], up: [0, 1, 0], zoom: 1 },
  selection: { kind: 'node', id: 'b' },
}

describe('seedNodePositions', () => {
  it('pins every node at its saved position when the structure is the same', () => {
    const seeded = seedNodePositions(graph, snapshot, 'k1')
    expect(seeded.restored).toBe(true)
    expect(seeded.nodes).toEqual([
      { id: 'a', label: 'A', degree: 1, x: 1, y: 2, z: 3, fx: 1, fy: 2, fz: 3 },
      { id: 'b', label: 'B', degree: 1, x: 4, y: 5, z: 6, fx: 4, fy: 5, fz: 6 },
    ])
    // The built graph is never mutated: the library gets copies.
    expect(graph.nodes[0]).toEqual({ id: 'a', label: 'A', degree: 1 })
  })

  it('lays the graph out fresh when the structure changed or a node has no saved position', () => {
    expect(seedNodePositions(graph, snapshot, 'k2')).toEqual({
      restored: false,
      nodes: [
        { id: 'a', label: 'A', degree: 1 },
        { id: 'b', label: 'B', degree: 1 },
      ],
    })
    const partial = { ...snapshot, positions: { a: [1, 2, 3] as [number, number, number] } }
    expect(seedNodePositions(graph, partial, 'k1').restored).toBe(false)
    expect(seedNodePositions(graph, null, 'k1').restored).toBe(false)
  })
})

describe('linkTouchesSelection', () => {
  it('lights the links of a selected node, or the selected link alone', () => {
    const ab = { source: 'a', target: 'b' }
    const bc = { source: { id: 'b' }, target: { id: 'c' } }
    expect(linkTouchesSelection(ab, { kind: 'node', id: 'a' })).toBe(true)
    expect(linkTouchesSelection(bc, { kind: 'node', id: 'a' })).toBe(false)
    // The renderer swaps endpoint ids for node objects once laid out.
    expect(linkTouchesSelection(bc, { kind: 'node', id: 'c' })).toBe(true)
    expect(linkTouchesSelection(ab, { kind: 'link', source: 'a', target: 'b' })).toBe(true)
    expect(linkTouchesSelection(ab, { kind: 'link', source: 'b', target: 'a' })).toBe(false)
    expect(linkTouchesSelection(ab, null)).toBe(false)
  })
})

describe('resolveSelection', () => {
  it('finds a saved node or link in the rebuilt graph, and drops one that is gone', () => {
    expect(resolveSelection(graph, { kind: 'node', id: 'b' })).toEqual({
      kind: 'node',
      node: graph.nodes[1],
    })
    expect(resolveSelection(graph, { kind: 'link', source: 'a', target: 'b' })).toEqual({
      kind: 'link',
      link: graph.links[0],
    })
    expect(resolveSelection(graph, { kind: 'node', id: 'zz' })).toBeNull()
    expect(resolveSelection(graph, { kind: 'link', source: 'b', target: 'a' })).toBeNull()
    expect(resolveSelection(graph, null)).toBeNull()
  })
})
