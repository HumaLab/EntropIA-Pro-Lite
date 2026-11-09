import { describe, expect, it } from 'vitest'
import {
  agentParametersForm,
  agentParametersJson,
  frozenResearchModel,
  frozenResearchScope,
  type ResearchArtifact,
  type ResearchCreateRequest,
  type ResearchCitation,
  stripProjectContext,
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
      context_collection_ids: [],
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

describe('agentParametersForm', () => {
  // La forma de la respuesta del motor: los campos y sus valores por defecto
  // (`ParametrosAgente::default()`). El formulario no repite ninguno.
  const DEFAULTS = {
    max_tokens_salida: 16384,
    temperatura: 0.3,
    esperas_reintento_s: [2, 6],
    lotes_archivo_bytes: [48000, 24000, 12000],
    max_consultas: { general: 20, trayectorias: 30, cronologia: 20 },
  }

  it('shows every engine default, lists as comma-separated text', () => {
    const form = agentParametersForm(DEFAULTS)
    expect(form.values.max_tokens_salida).toBe('16384')
    expect(form.values.temperatura).toBe('0.3')
    expect(form.values.esperas_reintento_s).toBe('2, 6')
    expect(form.values.lotes_archivo_bytes).toBe('48000, 24000, 12000')
    expect(form.maxConsultas).toEqual({ general: '20', trayectorias: '30', cronologia: '20' })
  })

  it('reads the saved parameters over the defaults and keeps what is missing', () => {
    const form = agentParametersForm(
      DEFAULTS,
      JSON.stringify({ max_tokens_salida: 2048, max_consultas: { general: 25 } })
    )
    expect(form.values.max_tokens_salida).toBe('2048')
    expect(form.values.temperatura).toBe('0.3')
    expect(form.maxConsultas).toEqual({ general: '25', trayectorias: '30', cronologia: '20' })
  })

  it('falls back to the defaults when the saved value is not readable', () => {
    const form = agentParametersForm(DEFAULTS, 'no es json')
    expect(form.values.max_tokens_salida).toBe('16384')
    expect(form.values.lotes_archivo_bytes).toBe('48000, 24000, 12000')
  })

  it('builds the parameters JSON the engine validates, lists as arrays', () => {
    const form = agentParametersForm(
      DEFAULTS,
      JSON.stringify({ temperatura: 1.1, esperas_reintento_s: [], max_consultas: { general: 25 } })
    )
    expect(agentParametersJson(form)).toEqual({
      max_tokens_salida: 16384,
      temperatura: 1.1,
      esperas_reintento_s: [],
      lotes_archivo_bytes: [48000, 24000, 12000],
      max_consultas: { general: 25, trayectorias: 30, cronologia: 20 },
    })
  })

  it('sends a non-numeric text as null so the engine answers with its own message', () => {
    const form = agentParametersForm(DEFAULTS)
    form.values.max_tokens_salida = 'muchos'
    form.values.lotes_archivo_bytes = '48000, tanto'
    const parameters = agentParametersJson(form)
    expect(parameters.max_tokens_salida).toBeNull()
    expect(parameters.lotes_archivo_bytes).toEqual([48000, null])
  })
})

describe('stripProjectContext', () => {
  it('removes the leading project-context block the backend prepends', () => {
    const block = `Contexto del proyecto (escrito por quien investiga; usalo para interpretar nombres):
- «Prensa»: 1890\n\n`
    expect(stripProjectContext(block + '¿Qué pasó?')).toBe('¿Qué pasó?')
  })

  it('leaves any other question untouched', () => {
    expect(stripProjectContext('¿Qué pasó?')).toBe('¿Qué pasó?')
    expect(stripProjectContext('')).toBe('')
    expect(stripProjectContext('Contexto del proyecto (incompleto')).toBe(
      'Contexto del proyecto (incompleto'
    )
  })
})
