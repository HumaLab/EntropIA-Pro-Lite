import { describe, expect, it } from 'vitest'
import { buildRelationGraph, describeNode } from './relation-graph'

describe('graph structure options', () => {
  const triples = [
    { subject: 'A', predicate: 'p', object: 'B' },
    { subject: ' a ', predicate: 'q', object: 'b' },
    { subject: 'A', predicate: 'p', object: 'C' },
    { subject: 'D', predicate: 'p', object: 'E' },
  ]
  it('filters by original distinct-pair degree without recursive pruning', () => {
    expect(buildRelationGraph(triples, { minDegree: 2 }).nodes.map((n) => n.id)).toEqual(['a'])
  })
  it('counts entity frequency across triple occurrences, not unique pairs', () => {
    expect(buildRelationGraph(triples, { minFrequency: 2 }).nodes.map((n) => n.id)).toEqual([
      'a',
      'b',
    ])
  })
  it('filters by directed pair weight while preserving merged predicates', () => {
    expect(buildRelationGraph(triples, { minRelationWeight: 2 }).links).toEqual([
      { source: 'a', target: 'b', predicates: ['p', 'q'] },
    ])
  })
  it('caps entities by existing degree ranking and first-seen tie order', () => {
    expect(buildRelationGraph(triples, { maxEntities: 2 })).toEqual(buildRelationGraph(triples, 2))
  })
  it('caps links by weight with first-seen ties, retaining original output order', () => {
    const input = [...triples.slice(2), ...triples.slice(0, 2)]
    expect(buildRelationGraph(input, { maxRelations: 1 }).links.map((l) => l.target)).toEqual(['b'])
    expect(buildRelationGraph(input, { maxRelations: 2 }).links.map((l) => l.target)).toEqual([
      'c',
      'b',
    ])
  })
  it('includes isolated nodes by default and can remove them after caps', () => {
    expect(buildRelationGraph(triples, { maxEntities: 1 }).nodes.map((n) => n.id)).toEqual(['a'])
    expect(buildRelationGraph(triples, { maxEntities: 1, includeIsolated: false })).toEqual({
      nodes: [],
      links: [],
      hiddenNodes: 5,
    })
  })
  it('does not mutate triples or options', () => {
    const input = Object.freeze(triples.map((t) => Object.freeze({ ...t })))
    const options = Object.freeze({ maxRelations: 1 })
    buildRelationGraph(input, options)
    expect(input).toEqual(triples)
    expect(options).toEqual({ maxRelations: 1 })
  })
})

describe('buildRelationGraph', () => {
  it('merges names that differ only in case or spacing and joins predicates per pair', () => {
    const graph = buildRelationGraph([
      { subject: 'Perón', predicate: 'fundó', object: 'Partido Justicialista' },
      { subject: ' perón ', predicate: 'presidió', object: 'partido  justicialista' },
      { subject: 'Perón', predicate: 'fundó', object: 'Partido Justicialista' },
      { subject: 'Evita', predicate: 'acompañó', object: 'Perón' },
      { subject: 'Nadie', predicate: 'es', object: 'nadie' },
      { subject: '', predicate: 'x', object: 'Evita' },
    ])

    expect(graph.nodes.map((n) => [n.label, n.degree])).toEqual([
      ['Perón', 2],
      ['Partido Justicialista', 1],
      ['Evita', 1],
    ])
    expect(graph.links).toEqual([
      { source: 'perón', target: 'partido justicialista', predicates: ['fundó', 'presidió'] },
      { source: 'evita', target: 'perón', predicates: ['acompañó'] },
    ])
    expect(graph.hiddenNodes).toBe(0)
  })

  it('keeps the best-connected nodes and drops links to the rest', () => {
    const graph = buildRelationGraph(
      [
        { subject: 'A', predicate: 'p', object: 'B' },
        { subject: 'A', predicate: 'p', object: 'C' },
        { subject: 'D', predicate: 'p', object: 'E' },
      ],
      2
    )

    expect(graph.nodes.map((n) => n.id)).toEqual(['a', 'b'])
    expect(graph.links).toEqual([{ source: 'a', target: 'b', predicates: ['p'] }])
    expect(graph.hiddenNodes).toBe(3)
  })

  it('describes a node with its relations, both directions, and their documents', () => {
    const triples = [
      { subject: 'Marini', predicate: 'visita', object: 'Mar del Plata', itemId: 'd1' },
      { subject: ' marini', predicate: 'visita', object: 'mar del plata', itemId: 'd2' },
      { subject: 'Verón', predicate: 'reclamó a', object: 'Marini', itemId: 'd2' },
      { subject: 'Otro', predicate: 'p', object: 'Nadie', itemId: 'd3' },
    ]

    expect(describeNode('marini', triples)).toEqual({
      relations: [
        { subject: 'Marini', predicate: 'visita', object: 'Mar del Plata' },
        { subject: 'Verón', predicate: 'reclamó a', object: 'Marini' },
      ],
      itemIds: ['d1', 'd2'],
    })
  })
})
