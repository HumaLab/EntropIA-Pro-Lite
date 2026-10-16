import { describe, expect, it } from 'vitest'
import { buildRelationGraph, describeNode } from './relation-graph'

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
