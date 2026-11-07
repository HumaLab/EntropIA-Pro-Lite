import { describe, expect, it } from 'vitest'
import {
  buildEntityTargets,
  buildNodeTargets,
  buildRelationTargets,
  locateOccurrences,
} from './relation-occurrences'

const context = { collectionId: 'c', collectionName: 'Collection', itemTitle: 'Document' }
const entity = { kind: 'entity' as const, value: 'José', itemId: 'i', assetId: 'a', ...context }
const relation = {
  kind: 'relation' as const,
  subject: 'José',
  predicate: 'visita',
  object: 'Lima',
  itemId: 'i',
  assetId: 'a',
  ...context,
}

describe('occurrence targets', () => {
  it('normalizes labels, deduplicates item/asset pairs and orders deterministically', () => {
    const rows = [
      { itemId: 'z', assetId: null, value: ' JOSÉ ', startOffset: 0, endOffset: 0 },
      { itemId: 'i', assetId: 'a', value: 'José', startOffset: 0, endOffset: 4 },
      { itemId: 'i', assetId: 'a', value: 'José', startOffset: 10, endOffset: 14 },
      { itemId: 'i', assetId: 'b', value: 'Other', startOffset: 0, endOffset: 5 },
    ]
    const targets = buildEntityTargets('josé', rows, () => context)
    expect(targets.map(({ itemId, assetId }) => [itemId, assetId])).toEqual([
      ['i', 'a'],
      ['z', null],
    ])
    expect(buildEntityTargets('josé', [...rows].reverse(), () => context)).toEqual(targets)
    const triples = [
      { id: '2', itemId: 'i', assetId: 'a', subject: 'José', predicate: 'visita', object: 'Lima' },
      {
        id: '1',
        itemId: 'i',
        assetId: 'a',
        subject: ' JOSÉ ',
        predicate: 'conoce',
        object: 'LIMA',
      },
      { id: '3', itemId: 'z', assetId: null, subject: 'Lima', predicate: 'recibe', object: 'José' },
    ]
    expect(buildRelationTargets('josé', 'lima', triples, () => context)).toHaveLength(1)
    expect(buildRelationTargets('josé', 'lima', [...triples].reverse(), () => context)).toEqual(
      buildRelationTargets('josé', 'lima', triples, () => context)
    )
  })
})

describe('buildNodeTargets', () => {
  it('adds the documents of the relations naming the node when NER has no entity there', () => {
    const entities = [{ itemId: 'i', assetId: 'a', value: 'José', startOffset: 0, endOffset: 4 }]
    const triples = [
      { id: '1', itemId: 'i', assetId: 'a', subject: 'José', predicate: 've', object: 'Lima' },
      { id: '2', itemId: 'k', assetId: null, subject: 'Lima', predicate: 've', object: ' JOSÉ ' },
      { id: '3', itemId: 'm', assetId: 'b', subject: 'Ana', predicate: 've', object: 'Lima' },
    ]
    const targets = buildNodeTargets('josé', entities, triples, () => context)
    expect(targets.map(({ itemId, assetId }) => [itemId, assetId])).toEqual([
      ['i', 'a'],
      ['k', null],
    ])
    // The NER row keeps its offset hint; the triple-only document searches.
    expect(targets[0]).toMatchObject({ kind: 'entity', offsets: [{ start: 0, end: 4 }] })
    expect(targets[1]).toMatchObject({ kind: 'entity', value: 'josé', offsets: [] })
    expect(buildNodeTargets('nadie', entities, triples, () => context)).toEqual([])
  })
})

describe('locateOccurrences', () => {
  it('accepts valid UTF-16 offsets including zero, and still exposes every match', () => {
    const result = locateOccurrences('José y José', { ...entity, offsets: [{ start: 0, end: 4 }] })
    expect(result.map(({ start, end }) => [start, end])).toEqual([
      [0, 4],
      [7, 11],
    ])
  })
  it.each([
    [0, 0],
    [1, 5],
    [-1, 3],
    [0, 100],
  ])('rejects invalid offsets %s/%s and searches', (start, end) => {
    const found = locateOccurrences('😀 José', { ...entity, offsets: [{ start, end }] })
    expect(found[0]).toMatchObject({ start: 3, end: 7, approximate: false })
  })
  it('matches diacritics and case, flexible whitespace and whole words without changing raw coordinates', () => {
    const found = locateOccurrences('Josef JOSE Jose\u0301', entity)
    expect(found.map(({ start, end }) => [start, end])).toEqual([
      [6, 10],
      [11, 16],
    ])
    const spaced = locateOccurrences('Nueva\n  York', { ...entity, value: 'nueva york' })
    expect(spaced[0]).toMatchObject({ start: 0, end: 12 })
    expect(locateOccurrences('Josef', entity)).toEqual([])
  })
  it('locates a bounded relation passage and labeled subject, predicate and object', () => {
    const text = 'José visita Lima. José vive en Lima.'
    const found = locateOccurrences(text, relation)
    expect(found).toHaveLength(2)
    expect(found[0]).toMatchObject({
      start: 0,
      end: 16,
      approximate: false,
      ranges: [
        { role: 'subject', start: 0, end: 4 },
        { role: 'predicate', start: 5, end: 11 },
        { role: 'object', start: 12, end: 16 },
      ],
    })
    expect(found[1]?.ranges.map(({ role }) => role)).toEqual(['subject', 'object'])
    expect(found.every(({ start, end }) => end - start <= 400)).toBe(true)
  })
  it('flags entity-only fallback as approximate, and returns none when neither exists', () => {
    expect(locateOccurrences('Solo Lima.', relation)[0]).toMatchObject({
      approximate: true,
      ranges: [{ role: 'object', start: 5, end: 9 }],
    })
    expect(locateOccurrences('Nada aquí.', relation)).toEqual([])
    const distant = locateOccurrences(`José ${'x'.repeat(401)} Lima`, relation)
    expect(distant.every((hit) => hit.approximate)).toBe(true)
  })
})
