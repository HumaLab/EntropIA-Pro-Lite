import { describe, expect, it } from 'vitest'
import { buildRelationGraph, describeNode, linkLabelText, splitRelations } from './relation-graph'

describe('linkLabelText', () => {
  it('names each predicate with how often it is stated when more than once', () => {
    expect(
      linkLabelText({ source: 'a', target: 'b', predicates: ['firma', 'convoca'], counts: [7, 1] })
    ).toBe('firma ×7 · convoca')
  })
})

describe('graph structure options', () => {
  const triples = [
    { subject: 'A', predicate: 'p', object: 'B' },
    { subject: ' a ', predicate: 'q', object: 'b' },
    { subject: 'A', predicate: 'p', object: 'C' },
    { subject: 'D', predicate: 'p', object: 'E' },
  ]
  const ids = (graph: { nodes: { id: string }[] }) => graph.nodes.map((n) => n.id).sort()
  const pairs = (graph: { links: { source: string; target: string }[] }) =>
    graph.links.map((l) => `${l.source}>${l.target}`).sort()
  /** No node may lose every link to a filter: isolation must come from the data. */
  const noFilterMadeIsolates = (graph: ReturnType<typeof buildRelationGraph>) => {
    const linked = new Set(graph.links.flatMap((l) => [l.source, l.target]))
    expect(graph.nodes.filter((n) => !linked.has(n.id))).toEqual([])
  }

  it('admits an object by its threshold with every subject of its triples, however rare', () => {
    const signed = ['F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7'].map((subject) => ({
      subject,
      predicate: 'firma',
      object: 'La presentación',
    }))
    const graph = buildRelationGraph([...signed, { subject: 'X', predicate: 'p', object: 'Y' }], {
      minSubjectFrequency: 10,
      minObjectFrequency: 5,
    })
    expect(ids(graph)).toEqual(['f1', 'f2', 'f3', 'f4', 'f5', 'f6', 'f7', 'la presentación'])
    expect(graph.links).toHaveLength(7)
    expect(graph.hiddenNodes).toBe(2)
    noFilterMadeIsolates(graph)
  })
  it('admits a subject by its threshold with every object of its triples', () => {
    const signs = ['O1', 'O2', 'O3', 'O4', 'O5', 'O6'].map((object) => ({
      subject: 'Federación',
      predicate: 'firma',
      object,
    }))
    const graph = buildRelationGraph(signs, { minSubjectFrequency: 5, minObjectFrequency: 10 })
    expect(ids(graph)).toEqual(['federación', 'o1', 'o2', 'o3', 'o4', 'o5', 'o6'])
    noFilterMadeIsolates(graph)
  })
  it("admits by distinct-pair degree and keeps the admitted node's counterparts", () => {
    const graph = buildRelationGraph(triples, { minDegree: 2 })
    expect(ids(graph)).toEqual(['a', 'b', 'c'])
    expect(pairs(graph)).toEqual(['a>b', 'a>c'])
    expect(graph.hiddenNodes).toBe(2)
  })
  it('drops weak relations, and with them nodes that have no other relation, not as isolates', () => {
    const graph = buildRelationGraph(triples, { minRelationWeight: 2 })
    expect(graph.links).toEqual([
      { source: 'a', target: 'b', predicates: ['p', 'q'], counts: [1, 1] },
    ])
    expect(ids(graph)).toEqual(['a', 'b'])
    noFilterMadeIsolates(graph)
  })
  it('caps visible entities by admitting whole relation stars, never half of one', () => {
    // A needs three nodes (a, b, c); the best star that fits two is b with a.
    const graph = buildRelationGraph(triples, { maxEntities: 2 })
    expect(ids(graph)).toEqual(['a', 'b'])
    expect(pairs(graph)).toEqual(['a>b'])
    expect(graph).toEqual(buildRelationGraph(triples, 2))
    noFilterMadeIsolates(graph)
  })
  it('caps relations by admitting whole stars as well', () => {
    const graph = buildRelationGraph(triples, { maxRelations: 1 })
    expect(pairs(graph)).toEqual(['a>b'])
    expect(ids(graph)).toEqual(['a', 'b'])
    noFilterMadeIsolates(graph)
  })
  it('keeps relations between admitted entities even when neither was the one admitted', () => {
    const graph = buildRelationGraph(
      [
        { subject: 'Hub', predicate: 'p', object: 'B' },
        { subject: 'Hub', predicate: 'p', object: 'C' },
        { subject: 'Hub', predicate: 'p', object: 'D' },
        { subject: 'B', predicate: 'q', object: 'C' },
      ],
      { minSubjectFrequency: 3, minObjectFrequency: 10 }
    )
    expect(pairs(graph)).toEqual(['b>c', 'hub>b', 'hub>c', 'hub>d'])
  })
  describe('graph and relation list agree', () => {
    // F links two hubs: drawn as their counterpart, its own star of five
    // relations does not fit in five visible entities.
    const hubs = [
      { subject: 'A', predicate: 'p', object: 'A1' },
      { subject: 'A', predicate: 'p', object: 'F' },
      { subject: 'F', predicate: 'q', object: 'F1' },
      { subject: 'F', predicate: 'q', object: 'F2' },
      { subject: 'F', predicate: 'q', object: 'F3' },
      { subject: 'F', predicate: 'r', object: 'F3' },
    ]
    it('counts, per node, the relations a cap left undrawn', () => {
      const graph = buildRelationGraph(hubs, { maxEntities: 3 })
      const f = graph.nodes.find((n) => n.id === 'f')!
      expect(f.hiddenRelations).toBe(4)
      expect(graph.nodes.find((n) => n.id === 'a')!.hiddenRelations).toBe(0)
      const details = describeNode('f', hubs)
      const { shown, hidden } = splitRelations(details.relations, graph)
      expect(shown.length + hidden.length).toBe(details.relations.length)
      expect(hidden).toHaveLength(f.hiddenRelations)
    })
    it("draws an expanded entity's whole star, past the caps", () => {
      const graph = buildRelationGraph(hubs, { maxEntities: 3 }, ['f'])
      expect(graph.nodes.find((n) => n.id === 'f')!.hiddenRelations).toBe(0)
      expect(pairs(graph)).toEqual(expect.arrayContaining(['f>f1', 'f>f2', 'f>f3', 'a>f']))
      expect(splitRelations(describeNode('f', hubs).relations, graph).hidden).toEqual([])
    })
  })

  it('says which setting hid each name: the thresholds or the caps', () => {
    // With the default thresholds every name may enter: only the cap hides.
    expect(buildRelationGraph(triples, { maxEntities: 2 }).hiddenBy).toEqual({
      thresholds: 0,
      caps: 3,
    })
    // a (degree 2) enters with b and c; d and e never reach the threshold.
    expect(buildRelationGraph(triples, { minDegree: 2 }).hiddenBy).toEqual({
      thresholds: 2,
      caps: 0,
    })
  })

  it('shows truly isolated entities only when asked, never ones a filter isolated', () => {
    const input = [...triples, { subject: 'Solo', predicate: 'se nombra', object: 'solo' }]
    expect(ids(buildRelationGraph(input))).toContain('solo')
    expect(ids(buildRelationGraph(input, { includeIsolated: false }))).not.toContain('solo')
    noFilterMadeIsolates(buildRelationGraph(input, { includeIsolated: false }))
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

    // «Nadie es nadie» names an entity the data never relates to another one:
    // the only isolation the graph shows, and only while isolates are included.
    expect(graph.nodes.map((n) => [n.label, n.degree])).toEqual([
      ['Perón', 2],
      ['Partido Justicialista', 1],
      ['Evita', 1],
      ['Nadie', 0],
    ])
    expect(graph.links).toEqual([
      // «fundó» is read twice: the link shows how often, not a thicker line.
      {
        source: 'perón',
        target: 'partido justicialista',
        predicates: ['fundó', 'presidió'],
        counts: [2, 1],
      },
      { source: 'evita', target: 'perón', predicates: ['acompañó'], counts: [1] },
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
    expect(graph.links).toEqual([{ source: 'a', target: 'b', predicates: ['p'], counts: [1] }])
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
        { subject: 'Marini', predicate: 'visita', object: 'Mar del Plata', count: 2 },
        { subject: 'Verón', predicate: 'reclamó a', object: 'Marini', count: 1 },
      ],
      itemIds: ['d1', 'd2'],
    })
  })
})
