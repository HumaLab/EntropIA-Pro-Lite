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
  it('locates the sentence holding a literal relation and marks its three parts as literal', () => {
    const found = locateOccurrences('José visita Lima. José vive en Lima.', relation)
    // The second sentence only names both ends; the first also holds the link.
    expect(found).toHaveLength(1)
    expect(found[0]).toMatchObject({
      start: 0,
      end: 17,
      approximate: false,
      evidence: { subject: 'literal', predicate: 'literal', object: 'literal' },
      ranges: [
        { role: 'subject', start: 0, end: 4, literal: true },
        { role: 'predicate', start: 5, end: 11, literal: true },
        { role: 'object', start: 12, end: 16, literal: true },
      ],
    })
  })

  it('finds a triple the model rebuilt from context and marks the inferred part', () => {
    const text =
      'Que se tenga en cuenta al personal de la Federación de Obreros en Construcciones Navales.\n\n' +
      'Sin local, sin asambleas, la Federación mantuvo su actividad, continuó editando su ' +
      'periódico y mantuvo cierto control sobre el trabajo de los talleres navales. Producida la ' +
      'revolución, el gremio se reorganizó.'
    const target = {
      ...relation,
      subject: 'la Federación de Obreros en Construcciones Navales',
      predicate: 'mantuvo cierto control sobre',
      object: 'el trabajo de los talleres navales',
    }
    const found = locateOccurrences(text, target)
    expect(found).toHaveLength(1)
    const sentence = found[0]!
    expect(text.slice(sentence.start, sentence.end)).toMatch(/^Sin local.*talleres navales\.$/)
    expect(sentence).toMatchObject({
      approximate: true,
      evidence: { subject: 'partial', predicate: 'literal', object: 'literal' },
    })
    const marked = sentence.ranges.map((range) => [
      range.role,
      text.slice(range.start, range.end),
      range.literal,
    ])
    expect(marked).toContainEqual(['subject', 'Federación', false])
    expect(marked).toContainEqual(['predicate', 'mantuvo cierto control sobre', true])
    expect(marked).toContainEqual(['object', 'el trabajo de los talleres navales', true])
    // A word already inside a literal part is not marked again for another part.
    expect(marked.filter(([, words]) => words === 'navales')).toEqual([])
  })

  it('never steps to a sentence that only names one end of the relation', () => {
    const found = locateOccurrences(
      'José nació en Cusco. Lima es grande. José visita Lima.',
      relation
    )
    expect(found.map(({ start }) => start)).toEqual([37])
  })

  it('falls back to one approximate passage when a single part is found, and none when nothing is', () => {
    const found = locateOccurrences('Lima es grande. Solo Lima.', relation)
    expect(found).toHaveLength(1)
    expect(found[0]).toMatchObject({
      approximate: true,
      evidence: { subject: 'missing', predicate: 'missing', object: 'literal' },
      ranges: [{ role: 'object', start: 0, end: 4, literal: true }],
    })
    expect(locateOccurrences('Nada aquí.', relation)).toEqual([])
  })

  it('finds the parts the model took from elsewhere in the document', () => {
    const text =
      'Presentación dirigida al Interventor de la Confederación General de Trabajadores.\n\n' +
      'Señor Interventor: los gremios abajo firmantes exponen su queja.\n\n' +
      'firmados por:\n\n' +
      'FEDERACION DE EMPLEADOS DE COMERCIO\n\n' +
      'FEDERACION DE OBREOS DE LA INDUSTRIA DEL VESTIDO\n\n' +
      '(es copia)'
    const target = {
      ...relation,
      subject: 'FEDERACION DE OBREOS DE LA INDUSTRIA DEL VESTIDO',
      predicate: 'firma',
      object: 'la presentación dirigida al Interventor de la Confederación General de Trabajadores',
    }
    const found = locateOccurrences(text, target)
    expect(found).toHaveLength(1)
    const [hit] = found
    expect(text.slice(hit!.start, hit!.end)).toBe(
      'FEDERACION DE OBREOS DE LA INDUSTRIA DEL VESTIDO'
    )
    expect(hit!.evidence).toEqual({
      subject: 'literal',
      predicate: 'elsewhere',
      object: 'elsewhere',
    })
    const marked = hit!.ranges.map((range) => [range.role, text.slice(range.start, range.end)])
    expect(marked).toContainEqual(['predicate', 'firmados'])
    expect(marked).toContainEqual(['object', 'Presentación'])
    expect(marked).toContainEqual(['object', 'Trabajadores'])
  })

  it('tolerates OCR misspellings when a part is not literal', () => {
    const text = 'La Federación de Obreos Navales firmó el acta.'
    const target = {
      ...relation,
      subject: 'Federación de Obreros Navales',
      predicate: 'firmó',
      object: 'el acta',
    }
    const [hit] = locateOccurrences(text, target)
    const marked = hit!.ranges.map((range) => [
      range.role,
      text.slice(range.start, range.end),
      range.literal,
    ])
    expect(marked).toContainEqual(['subject', 'Obreos', false])
    expect(hit!.evidence).toEqual({ subject: 'partial', predicate: 'literal', object: 'literal' })
  })

  it('splits sentences too long to read as one passage', () => {
    const text = `José ${'palabra '.repeat(80)}Lima`
    const found = locateOccurrences(text, relation)
    expect(found.every(({ start, end }) => end - start <= 400)).toBe(true)
    expect(found.every((hit) => hit.approximate)).toBe(true)
  })
})

describe('stored triple evidence', () => {
  const text = 'José visita Lima. Años después, José visita Lima con su hermano.'
  const second = 'Años después, José visita Lima con su hermano.'
  const secondStart = text.indexOf(second)
  const row = {
    id: '1',
    itemId: 'i',
    assetId: 'a',
    subject: 'José',
    predicate: 'visita',
    object: 'Lima',
  }

  it('carries the stored sentences of every row of an address, without repeats', () => {
    const rows = [
      { ...row, evidenceText: second, evidenceStart: secondStart, evidenceEnd: text.length },
      {
        ...row,
        id: '2',
        evidenceText: second,
        evidenceStart: secondStart,
        evidenceEnd: text.length,
      },
      { ...row, id: '3', predicate: 'recorre', evidenceText: 'José recorre Lima.' },
      { ...row, id: '4', evidenceText: null, evidenceStart: null, evidenceEnd: null },
    ]
    const [target] = buildRelationTargets('josé', 'lima', rows, () => context)
    expect(target).toMatchObject({
      evidence: [
        { text: second, start: secondStart, end: text.length },
        { text: 'José recorre Lima.', start: null, end: null },
      ],
    })
    const [withoutEvidence] = buildRelationTargets('josé', 'lima', [row], () => context)
    expect(withoutEvidence).not.toHaveProperty('evidence')
  })

  it('shows the stored sentence, not the best scored one, and marks its parts', () => {
    const found = locateOccurrences(text, {
      ...relation,
      evidence: [{ text: second, start: secondStart, end: text.length }],
    })
    expect(found).toHaveLength(1)
    expect(found[0]).toMatchObject({
      start: secondStart,
      end: text.length,
      approximate: false,
      evidence: { subject: 'literal', predicate: 'literal', object: 'literal' },
    })
    expect(found[0]!.ranges.map(({ role, start }) => [role, start])).toEqual([
      ['subject', text.indexOf('José', secondStart)],
      ['predicate', text.indexOf('visita', secondStart)],
      ['object', text.indexOf('Lima', secondStart)],
    ])
  })

  it('finds the sentence by its words when the stored span no longer matches the text', () => {
    const edited = `Nota. ${text.replace('Años después, ', 'Años  después,\n')}`
    const found = locateOccurrences(edited, {
      ...relation,
      evidence: [{ text: second, start: secondStart, end: text.length }],
    })
    expect(found).toHaveLength(1)
    expect(edited.slice(found[0]!.start, found[0]!.end)).toBe(
      'Años  después,\nJosé visita Lima con su hermano.'
    )
  })

  it('falls back to the scored sentences when the stored sentence is not in the text', () => {
    const withMissing = locateOccurrences(text, {
      ...relation,
      evidence: [{ text: 'Pedro viajó a Quito.', start: 0, end: 20 }],
    })
    expect(withMissing).toEqual(locateOccurrences(text, relation))
  })

  it('still looks elsewhere for a part the stored sentence lacks', () => {
    const doc = 'Acta de Lima. José visita la ciudad con su hermano.'
    const found = locateOccurrences(doc, {
      ...relation,
      evidence: [{ text: 'José visita la ciudad con su hermano.', start: null, end: null }],
    })
    expect(found).toHaveLength(1)
    expect(found[0]).toMatchObject({
      start: doc.indexOf('José'),
      approximate: true,
      evidence: { subject: 'literal', predicate: 'literal', object: 'elsewhere' },
    })
  })
})

describe('entity aliases (T2)', () => {
  // Same fixture as S1/S3: "Mdme. Cuevas = Carmela Cuevas".
  const resolveAlias = (value: string) =>
    value.trim().replace(/\s+/g, ' ').toLowerCase() === 'mdme. cuevas' ? 'Carmela Cuevas' : value

  it('groups the NER rows of every alias form under the unified node', () => {
    const entityRows = [
      { itemId: 'i', assetId: 'a', value: 'Mdme. Cuevas', startOffset: 0, endOffset: 12 },
      { itemId: 'i', assetId: 'a', value: 'Carmela Cuevas', startOffset: 20, endOffset: 33 },
    ]

    const targets = buildNodeTargets('Carmela Cuevas', entityRows, [], () => context, resolveAlias)

    expect(targets).toHaveLength(1)
    expect(targets[0]).toMatchObject({
      kind: 'entity',
      value: 'Carmela Cuevas',
      offsets: [
        { start: 0, end: 12 },
        { start: 20, end: 33 },
      ],
    })
    const first = targets[0]
    expect(first?.kind).toBe('entity')
    if (first?.kind !== 'entity') throw new Error('expected an entity target')
    expect(first.surfaceForms).toEqual(['Mdme. Cuevas'])
  })

  it('reaches the documents whose triple endpoints use an alias form', () => {
    const tripleRows = [
      {
        id: '1',
        itemId: 'k',
        assetId: null,
        subject: 'Mdme. Cuevas',
        predicate: 've',
        object: 'Lima',
      },
    ]

    const targets = buildNodeTargets('Carmela Cuevas', [], tripleRows, () => context, resolveAlias)

    expect(targets.map(({ itemId, assetId }) => [itemId, assetId])).toEqual([['k', null]])
    expect(targets[0]).toMatchObject({ kind: 'entity', value: 'Carmela Cuevas' })
  })

  it('matches relation endpoints through alias forms, keeping rows as written', () => {
    const rows = [
      {
        id: '1',
        itemId: 'i',
        assetId: 'a',
        subject: 'Mdme. Cuevas',
        predicate: 'visita',
        object: 'Lima',
      },
    ]

    const targets = buildRelationTargets(
      'Carmela Cuevas',
      'Lima',
      rows,
      () => context,
      resolveAlias
    )

    expect(targets).toHaveLength(1)
    expect(targets[0]).toMatchObject({
      kind: 'relation',
      subject: 'Mdme. Cuevas',
      predicate: 'visita',
      object: 'Lima',
    })
    // Without a resolver nothing changes: the alias form does not match.
    expect(buildRelationTargets('Carmela Cuevas', 'Lima', rows, () => context)).toEqual([])
  })

  it('locates the mentions of every alias form, each with its own offsets', () => {
    const found = locateOccurrences('Mdme. Cuevas y Carmela Cuevas', {
      ...entity,
      value: 'Carmela Cuevas',
      offsets: [{ start: 0, end: 12 }],
      surfaceForms: ['Mdme. Cuevas'],
    })

    expect(found.map(({ start, end }) => [start, end])).toEqual([
      [0, 12],
      [15, 29],
    ])
  })
})
