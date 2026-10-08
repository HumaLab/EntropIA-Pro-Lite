import { describe, expect, it } from 'vitest'
import {
  frozenResearchModel,
  frozenResearchScope,
  type ResearchArtifact,
  type ResearchCreateRequest,
  type ResearchCitation,
} from './research'

function artifact(kind: string, content: unknown, obsolete = false, version = 1): ResearchArtifact {
  return { id: `art-${kind}-${version}`, kind, version, obsolete, content }
}

describe('frozenResearchScope', () => {
  it('reads the scope and the library refs frozen at create', () => {
    const scope = frozenResearchScope([
      artifact('request', {
        title: '¿Pregunta?',
        alcance: 'ambos',
        bibliotecas: ['user:123', 'group:456'],
      }),
    ])
    expect(scope).toEqual({ alcance: 'ambos', bibliotecas: ['user:123', 'group:456'] })
  })

  it('defaults to corpus when the request predates the scope, and says so', () => {
    expect(frozenResearchScope([artifact('request', { title: 'Viejo' })])).toEqual({
      alcance: 'corpus',
      bibliotecas: [],
    })
  })

  it('reads the latest live request, never an obsolete one', () => {
    const scope = frozenResearchScope([
      artifact('request', { alcance: 'biblioteca', bibliotecas: ['user:1'] }, true, 1),
      artifact('request', { alcance: 'corpus', bibliotecas: [] }, false, 2),
    ])
    expect(scope).toEqual({ alcance: 'corpus', bibliotecas: [] })
  })

  it('has no frozen scope when the job carries no request artifact', () => {
    expect(frozenResearchScope([])).toBeNull()
    expect(frozenResearchScope([artifact('report', {})])).toBeNull()
  })
})

describe('frozenResearchModel', () => {
  it('reads the model the job froze at create, from the same snapshot as the scope', () => {
    expect(
      frozenResearchModel([
        artifact('request', {
          title: '¿Pregunta?',
          alcance: 'ambos',
          bibliotecas: ['user:123'],
          modelo: 'meta/llama-3.3-70b',
        }),
      ])
    ).toBe('meta/llama-3.3-70b')
  })

  it('reads the latest live request, never an obsolete one', () => {
    expect(
      frozenResearchModel([
        artifact('request', { modelo: 'viejo/modelo' }, true, 1),
        artifact('request', { modelo: 'nuevo/modelo' }, false, 2),
      ])
    ).toBe('nuevo/modelo')
  })

  it('is null for jobs that froze no model, and never invents one', () => {
    expect(frozenResearchModel([])).toBeNull()
    expect(frozenResearchModel([artifact('report', {})])).toBeNull()
    // Un job anterior al selector de modelo no trae `modelo`.
    expect(frozenResearchModel([artifact('request', { alcance: 'corpus' })])).toBeNull()
    // Un modelo en blanco no es un modelo congelado.
    expect(frozenResearchModel([artifact('request', { modelo: '   ' })])).toBeNull()
  })
})

describe('research request shapes', () => {
  it('serializes the library refs in the one format both sides speak', () => {
    // The bridge in the desktop and the engine both name a library
    // `user:123` / `group:456`: the same string travels in `bibliotecas` and
    // comes back in each citation's `biblioteca`.
    const request: ResearchCreateRequest = {
      question: '¿Pregunta?',
      project: 'investigación',
      collection_ids: ['c-1'],
      max_llm_calls: 10,
      max_cost: null,
      context: null,
      alcance: 'biblioteca',
      bibliotecas: ['user:123'],
    }
    expect(request.alcance).toBe('biblioteca')
    expect(request.bibliotecas).toEqual(['user:123'])
  })

  it('types the bibliography fields a citation of a passage carries', () => {
    const citation: ResearchCitation = {
      n: 1,
      evidence_id: 'bib:user:123:chunk-1',
      item_id: '',
      chunk_id: 'chunk-1',
      title: 'Historia de los vencidos · pp. 3–4',
      text: 'un pasaje citable',
      start: 0,
      end: 17,
      provenance: 'zotero',
      biblioteca: 'user:123',
      item_key: 'ABCD1234',
      autores: 'Bloch, Febvre',
      anio: 1949,
      ubicacion: { tipo: 'paginas', desde: 3, hasta: 4 },
    }
    expect(citation.provenance).toBe('zotero')
    expect(citation.ubicacion).toEqual({ tipo: 'paginas', desde: 3, hasta: 4 })
  })
})
