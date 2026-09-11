/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@entropia/ui', async () => {
  const actual = await vi.importActual<typeof import('@entropia/ui')>('@entropia/ui')
  const MockButton = (await import('./__mocks__/MockButton.svelte')).default
  const MockActionIcon = (await import('./__mocks__/MockActionIcon.svelte')).default
  return {
    ...actual,
    Button: MockButton,
    ActionIcon: MockActionIcon,
  }
})

import InvestigationView from './InvestigationView.svelte'

const HUGE_MARKER = 'UNIQUE-ARCHIVE-MARKER-SHOULD-NOT-PAINT'

function detailPayload() {
  return {
    job: {
      id: 'job-65972-0',
      question:
        'Realizar una investigación sobre la organización y la lucha de los y las obreras del pescado en los años 1960-1970',
      status: 'paused',
      close_reason: null,
      phase: 'verification',
      llm_calls: 21,
      max_llm_calls: 40,
      cost: 0.5,
      max_cost: 2,
    },
    events: [],
    artifacts: [
      {
        id: 'art-archive',
        kind: 'archive',
        version: 1,
        obsolete: false,
        content: {
          summary: 'lote',
          marker: HUGE_MARKER,
          evidence: 'x'.repeat(50_000),
        },
      },
    ],
    gates: [],
    sources: [],
  }
}

/** Un job frenado en la ronda de preguntas, todavía sin responder. */
function rondaAbiertaPayload() {
  return {
    ...detailPayload(),
    job: { ...detailPayload().job, status: 'awaiting_human', phase: 'clarification' },
    artifacts: [
      {
        id: 'art-round',
        kind: 'clarification_round',
        version: 1,
        obsolete: false,
        content: {
          questions: [
            {
              id: 'q1',
              axis: 'Período',
              text: '¿Qué recorte temporal delimita el informe?',
              rationale: 'Cambia el plan',
            },
            {
              id: 'q2',
              axis: 'Fuentes',
              text: '¿Qué tipos documentales deben pesar más?',
              rationale: 'Cambia el plan',
            },
          ],
        },
      },
    ],
    gates: [
      {
        id: 'gate-round',
        kind: 'clarification_round',
        artifact_id: 'art-round',
        status: 'pending',
      },
    ],
  }
}

/** La ronda abierta, con el diseño vigente a la vista. */
function rondaConDisenoPayload() {
  const base = rondaAbiertaPayload()
  return {
    ...base,
    artifacts: [
      {
        id: 'art-design',
        kind: 'design',
        version: 1,
        obsolete: false,
        content: {
          hypothesis: 'La huelga de 1966 reorganizó la comisión interna',
          scope: 'Mar del Plata, 1965-1967',
          closing_criteria: ['Actas del plenario', 'Prensa gremial'],
        },
      },
      ...base.artifacts,
    ],
  }
}

/** Un job frenado en el gate del plan final, después de la ronda. */
function gatePlanPayload() {
  const base = detailPayload()
  return {
    ...base,
    job: { ...base.job, status: 'awaiting_human', phase: 'execution' },
    artifacts: [
      {
        id: 'art-plan-2',
        kind: 'plan',
        version: 2,
        obsolete: false,
        content: {
          queries: ['huelga pescado 1966', 'comisión interna SOIP'],
          bibliography_queries: ['historia sindical Mar del Plata'],
          retrieval_limit: 8,
        },
      },
    ],
    gates: [
      {
        id: 'gate-plan',
        kind: 'plan',
        artifact_id: 'art-plan-2',
        stage_id: 'art-plan-2',
        status: 'pending',
      },
    ],
  }
}

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe('InvestigationView', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset()
  })

  it('shows the job instead of loading when get is slower than the poll interval', async () => {
    vi.useFakeTimers()
    invokeMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          setTimeout(() => resolve(detailPayload()), 2000)
        })
    )

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación sobre obreras' },
    })

    expect(screen.getByRole('heading', { name: 'Investigación sobre obreras' })).toBeInTheDocument()

    await vi.advanceTimersByTimeAsync(10_000)

    expect(screen.queryByText('Preparando…')).not.toBeInTheDocument()
    expect(screen.getByText('Continuar')).toBeInTheDocument()
  })

  it('muestra la ronda de preguntas y la responde en vez de aprobar un gate', async () => {
    invokeMock.mockResolvedValue(rondaAbiertaPayload())

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('¿Qué recorte temporal delimita el informe?')).toBeInTheDocument()
    })
    expect(screen.getByText('¿Qué tipos documentales deben pesar más?')).toBeInTheDocument()

    const campos = screen.getAllByRole('textbox')
    await fireEvent.input(campos[0]!, { target: { value: '1965-1966' } })

    invokeMock.mockClear()
    await fireEvent.click(screen.getByText('Responder y seguir'))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('research_request', {
        request: {
          op: 'answer',
          job_id: 'job-65972-0',
          answers: [
            { id: 'q1', text: '1965-1966' },
            { id: 'q2', text: '' },
          ],
        },
      })
    })
  })

  it('no envía una ronda entera en blanco', async () => {
    invokeMock.mockResolvedValue(rondaAbiertaPayload())

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('Responder y seguir')).toBeInTheDocument()
    })

    invokeMock.mockClear()
    await fireEvent.click(screen.getByText('Responder y seguir'))

    // El encuadre del informe depende de esto: sin ninguna respuesta no se
    // manda nada al motor y se explica por qué.
    expect(invokeMock).not.toHaveBeenCalled()
    expect(
      screen.getByText('Respondé al menos una pregunta: el encuadre del informe depende de esto.')
    ).toBeInTheDocument()
  })

  it('la ronda muestra el diseño y, editado, lo manda junto con las respuestas', async () => {
    invokeMock.mockResolvedValue(rondaConDisenoPayload())

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    // El diseño se lee antes de responder: editarlo es la forma de rechazarlo.
    await waitFor(() => {
      expect(screen.getByText('Diseño de la investigación')).toBeInTheDocument()
    })
    expect(screen.getByText('La huelga de 1966 reorganizó la comisión interna')).toBeInTheDocument()
    expect(screen.getByText('Mar del Plata, 1965-1967')).toBeInTheDocument()
    expect(screen.getByText('Actas del plenario')).toBeInTheDocument()

    await fireEvent.click(screen.getByText('Editar diseño'))
    await fireEvent.input(screen.getByLabelText('Hipótesis'), {
      target: { value: 'La huelga de 1966 dividió a la comisión interna' },
    })
    await fireEvent.input(screen.getByLabelText('Criterios de cierre, uno por línea'), {
      target: { value: 'Actas del plenario\n\n  Prensa gremial  \nTestimonios' },
    })
    await fireEvent.input(screen.getByLabelText(/Qué recorte temporal delimita el informe/), {
      target: { value: '1965-1966' },
    })

    invokeMock.mockClear()
    await fireEvent.click(screen.getByText('Responder y seguir'))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('research_request', {
        request: {
          op: 'answer',
          job_id: 'job-65972-0',
          answers: [
            { id: 'q1', text: '1965-1966' },
            { id: 'q2', text: '' },
          ],
          design: {
            hypothesis: 'La huelga de 1966 dividió a la comisión interna',
            scope: 'Mar del Plata, 1965-1967',
            closing_criteria: ['Actas del plenario', 'Prensa gremial', 'Testimonios'],
          },
        },
      })
    })
  })

  it('un diseño editado con campos vacíos no se manda', async () => {
    invokeMock.mockResolvedValue(rondaConDisenoPayload())

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('Editar diseño')).toBeInTheDocument()
    })
    await fireEvent.click(screen.getByText('Editar diseño'))
    await fireEvent.input(screen.getByLabelText('Alcance'), { target: { value: '   ' } })
    await fireEvent.input(screen.getByLabelText(/Qué recorte temporal delimita el informe/), {
      target: { value: '1965-1966' },
    })

    invokeMock.mockClear()
    await fireEvent.click(screen.getByText('Responder y seguir'))

    expect(invokeMock).not.toHaveBeenCalled()
    expect(
      screen.getByText('Completá la hipótesis, el alcance y al menos un criterio de cierre.')
    ).toBeInTheDocument()
  })

  it('el gate del plan muestra las búsquedas y «Aprobar y buscar» manda la decisión', async () => {
    const base = gatePlanPayload()
    invokeMock.mockResolvedValue({
      ...base,
      // El motor avisa cuando las respuestas no cambiaron el plan.
      events: [
        { id: 'ev-1', kind: 'clarification_answered', payload: {}, timestamp: 1 },
        {
          id: 'ev-2',
          kind: 'role_warning',
          payload: {
            role: 'investigador_principal',
            code: 'plan_unchanged',
            error:
              'la replanificación no cambió el plan: el encuadre respondido no dejó huella en las consultas',
          },
          timestamp: 2,
        },
      ],
    })

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('Búsquedas antes de ir al corpus')).toBeInTheDocument()
    })
    const busquedas = screen.getAllByRole('list', { name: 'Búsquedas' })[0]!
    expect(busquedas.tagName).toBe('OL')
    expect(busquedas).toHaveTextContent('huelga pescado 1966')
    expect(busquedas).toHaveTextContent('comisión interna SOIP')
    expect(screen.getByText('historia sindical Mar del Plata')).toBeInTheDocument()
    expect(screen.getByText(/Tus respuestas no cambiaron el plan/)).toBeInTheDocument()
    // Rechazar sin corregir no lleva a ningún estado útil: no se ofrece.
    expect(screen.queryByText('Rechazar')).not.toBeInTheDocument()

    invokeMock.mockClear()
    await fireEvent.click(screen.getByText('Aprobar y buscar'))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('research_request', {
        request: { op: 'decision', job_id: 'job-65972-0', gate_id: 'gate-plan', approve: true },
      })
    })
  })

  it('«Guardar y buscar» manda las búsquedas editadas y muestra el rechazo del motor', async () => {
    invokeMock.mockImplementation((_cmd: string, args: { request?: { op?: string } }) => {
      if (args?.request?.op === 'revise') {
        // Tauri rechaza con el string del backend; la interfaz no conoce el tope.
        return Promise.reject('Plan fuera de límites: como máximo 2 consultas')
      }
      return Promise.resolve(gatePlanPayload())
    })

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('Editar búsquedas')).toBeInTheDocument()
    })
    await fireEvent.click(screen.getByText('Editar búsquedas'))

    const busquedas = screen.getByLabelText('Búsquedas, una por línea') as HTMLTextAreaElement
    expect(busquedas.value).toBe('huelga pescado 1966\ncomisión interna SOIP')
    await fireEvent.input(busquedas, {
      target: { value: 'huelga pescado 1966\n\n  despidos SOIP  \nplenario de delegadas' },
    })
    await fireEvent.input(screen.getByLabelText('Consultas bibliográficas, una por línea'), {
      target: { value: '' },
    })

    invokeMock.mockClear()
    await fireEvent.click(screen.getByText('Guardar y buscar'))

    // Solo cambian las búsquedas: el límite de recuperación es el del plan vigente.
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('research_request', {
        request: {
          op: 'revise',
          job_id: 'job-65972-0',
          artifact_id: 'art-plan-2',
          content: {
            queries: ['huelga pescado 1966', 'despidos SOIP', 'plenario de delegadas'],
            bibliography_queries: [],
            retrieval_limit: 8,
          },
        },
      })
    })
    await waitFor(() => {
      expect(screen.getByText('Plan fuera de límites: como máximo 2 consultas')).toBeInTheDocument()
    })
  })

  it('pinta cobertura, pasajes y fuentes como estructura, no como markdown aplastado', async () => {
    const base = detailPayload()
    invokeMock.mockResolvedValue({
      ...base,
      job: { ...base.job, status: 'done', phase: 'report' },
      artifacts: [
        {
          id: 'art-report',
          kind: 'report',
          version: 1,
          obsolete: false,
          content: {
            markdown: '# Informe\n\n| Colección | Items |\n|---|---|',
            report: {
              title: 'Organización del conflicto',
              references: [
                {
                  n: 1,
                  evidence_id: 'e1',
                  item_id: 'item-1',
                  chunk_id: 'ragchk-abc',
                  collection: 'Conflicto SOIP 1965-66',
                  title: '65-04-12-b',
                  date: '1965-04-12',
                  start: 0,
                  end: 800,
                },
              ],
              sections: [
                {
                  title: 'Hechos',
                  text: 'El plenario dispuso un paro general.',
                  claim_ids: ['c1'],
                  quotes: [
                    {
                      n: 1,
                      evidence_id: 'e1',
                      item_id: 'item-1',
                      chunk_id: 'ragchk-abc',
                      collection: 'Conflicto SOIP 1965-66',
                      title: '65-04-12-b',
                      date: '1965-04-12',
                      text: 'dispuso un paro general por tres horas',
                      start: 133,
                      end: 171,
                    },
                  ],
                },
              ],
            },
            coverage: {
              collections: [
                {
                  id: 'c-conflicto',
                  name: 'Conflicto SOIP 1965-66',
                  items: 148,
                  items_with_chunks: 12,
                  chunks: 40,
                },
              ],
            },
            coverage_warning: { sufficient: true },
            archive_limitations: [{ text: 'Marzo con cobertura fragmentaria' }],
            role_warnings: [],
            profile: { id: 'general', name: 'Informe general', bias: 'Sin priorización temática.' },
          },
        },
      ],
    })

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('Organización del conflicto')).toBeInTheDocument()
    })

    // La cobertura es una tabla de verdad, no una fila de pipes.
    expect(screen.getByRole('table')).toBeInTheDocument()
    expect(screen.getByRole('columnheader', { name: 'Colección' })).toBeInTheDocument()
    expect(screen.getByRole('rowheader', { name: 'Total' })).toBeInTheDocument()

    // El pasaje se reproduce completo y su referencia queda al lado.
    expect(screen.getByText('dispuso un paro general por tres horas')).toBeInTheDocument()
    expect(screen.getByText('chars 133–171')).toBeInTheDocument()

    // Y las fuentes citadas son elementos accionables, no un párrafo pegado.
    expect(screen.getByText('Fuentes citadas')).toBeInTheDocument()
    // Dos veces: al pie del pasaje y en la lista de fuentes citadas.
    expect(screen.getAllByText('Conflicto SOIP 1965-66 · 65-04-12-b · 1965-04-12')).toHaveLength(2)

    // El sesgo del perfil se declara junto a la cobertura.
    expect(screen.getByText(/Sin priorización temática/)).toBeInTheDocument()
  })

  it('al tocar una cita, la fuente se abre en el panel de la derecha', async () => {
    const base = detailPayload()
    const conInforme = {
      ...base,
      job: { ...base.job, status: 'done', phase: 'report' },
      sources: [{ item_id: 'item-1', title: '65-04-12-b' }],
      artifacts: [
        {
          id: 'art-report',
          kind: 'report',
          version: 1,
          obsolete: false,
          content: {
            report: {
              title: 'Organización del conflicto',
              references: [],
              sections: [
                {
                  title: 'Hechos',
                  text: 'El plenario dispuso un paro general.',
                  claim_ids: ['c1'],
                  quotes: [
                    {
                      n: 1,
                      evidence_id: 'e1',
                      item_id: 'item-1',
                      chunk_id: 'ragchk-abc',
                      collection: 'Conflicto SOIP 1965-66',
                      title: '65-04-12-b',
                      date: '1965-04-12',
                      text: 'dispuso un paro general por tres horas',
                      start: 133,
                      end: 171,
                    },
                  ],
                },
              ],
            },
            coverage: { collections: [] },
            coverage_warning: { sufficient: true },
            archive_limitations: [],
            role_warnings: [],
          },
        },
      ],
    }

    invokeMock.mockImplementation((_cmd: string, args: { request?: { op?: string } }) => {
      if (args?.request?.op === 'source') {
        return Promise.resolve({ sources: [{ path: 'escaneos/65-04-12-b.pdf', page: 2 }] })
      }
      return Promise.resolve(conInforme)
    })

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('dispuso un paro general por tres horas')).toBeInTheDocument()
    })

    // Antes de tocar nada, el panel invita a elegir una cita.
    expect(
      screen.getByText('Elegí una cita del informe para ver su fuente acá.')
    ).toBeInTheDocument()

    await fireEvent.click(screen.getByText('dispuso un paro general por tres horas'))

    // La cita pide su fuente por item_id, no por coincidencia de título.
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('research_request', {
        request: { op: 'source', job_id: 'job-65972-0', item_id: 'item-1' },
      })
    })

    // Y el documento queda accionable en el panel, sin salir de la vista.
    await waitFor(() => {
      expect(screen.getByText('Abrir el documento · p. 2')).toBeInTheDocument()
    })
    expect(
      screen.queryByText('Elegí una cita del informe para ver su fuente acá.')
    ).not.toBeInTheDocument()
  })

  it('un informe viejo, sin item_id en la cita, resuelve la fuente por título', async () => {
    const base = detailPayload()
    // Artefacto anterior a que la cita llevara su item: sin `item_id`.
    const informeViejo = {
      ...base,
      job: { ...base.job, status: 'done', phase: 'report' },
      sources: [{ item_id: 'item-9', title: 'A_y_J_2016-01-13-105442' }],
      artifacts: [
        {
          id: 'art-report',
          kind: 'report',
          version: 1,
          obsolete: false,
          content: {
            report: {
              title: 'Trayectoria',
              references: [],
              sections: [
                {
                  title: 'Hechos',
                  text: 'Testimonio sobre el convenio.',
                  claim_ids: ['c1'],
                  quotes: [
                    {
                      n: 9,
                      evidence_id: 'e9',
                      chunk_id: 'ragchk-2786',
                      collection: 'Voces',
                      title: 'A_y_J_2016-01-13-105442',
                      text: 'Es contra el convenio Crocito',
                      start: 9206,
                      end: 9809,
                    },
                  ],
                },
              ],
            },
            coverage: { collections: [] },
            coverage_warning: { sufficient: true },
            archive_limitations: [],
            role_warnings: [],
          },
        },
      ],
    }

    invokeMock.mockImplementation((_cmd: string, args: { request?: { op?: string } }) => {
      if (args?.request?.op === 'source') {
        return Promise.resolve({ sources: [{ path: 'voces/entrevista.mp3', page: null }] })
      }
      return Promise.resolve(informeViejo)
    })

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('Es contra el convenio Crocito')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByText('Es contra el convenio Crocito'))

    // Sin `item_id` en la cita, el item sale de las fuentes del job por
    // título: un informe viejo no queda con sus fuentes rotas.
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('research_request', {
        request: { op: 'source', job_id: 'job-65972-0', item_id: 'item-9' },
      })
    })
    expect(screen.queryByText('Falta item_id')).not.toBeInTheDocument()
  })

  it('does not dump archive JSON into the document until the artifact is opened', async () => {
    invokeMock.mockResolvedValue(detailPayload())

    render(InvestigationView, {
      props: { jobId: 'job-65972-0', title: 'Investigación' },
    })

    await waitFor(() => {
      expect(screen.getByText('Continuar')).toBeInTheDocument()
    })

    expect(screen.queryByText(HUGE_MARKER, { exact: false })).not.toBeInTheDocument()
  })

  // ==========================================================================
  // Ajustar el presupuesto.
  //
  // Cuando una investigación agota su techo de llamadas, el motor la pasa de
  // `running` a `paused` y deja el error registrado. Pausar y continuar ya
  // estaban en la barra; el eslabón del medio no. Sin él, «Continuar» choca
  // contra el mismo techo y vuelve a pausar, en bucle, con todo lo ya
  // investigado —y pagado— adentro.
  // ==========================================================================
  describe('ajustar el presupuesto', () => {
    function corriendoPayload() {
      return { ...detailPayload(), job: { ...detailPayload().job, status: 'running' } }
    }

    async function abrirEditor() {
      render(InvestigationView, {
        props: { jobId: 'job-65972-0', title: 'Investigación' },
      })
      await waitFor(() => {
        expect(screen.getByText('Ajustar presupuesto')).toBeInTheDocument()
      })
      await fireEvent.click(screen.getByText('Ajustar presupuesto'))
    }

    it('abre el editor con el techo actual y manda el nuevo al motor', async () => {
      invokeMock.mockResolvedValue(detailPayload())
      await abrirEditor()

      const llamadas = screen.getByLabelText('Llamadas LLM') as HTMLInputElement
      const costo = screen.getByLabelText('Costo máximo') as HTMLInputElement
      expect(llamadas.value).toBe('40')
      expect(costo.value).toBe('2')

      await fireEvent.input(llamadas, { target: { value: '80' } })
      invokeMock.mockClear()
      await fireEvent.click(screen.getByText('Guardar presupuesto'))

      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith('research_request', {
          request: {
            op: 'update_budget',
            job_id: 'job-65972-0',
            max_llm_calls: 80,
            max_cost: 2,
          },
        })
      })
    })

    it('un costo máximo vacío viaja como sin límite', async () => {
      invokeMock.mockResolvedValue(detailPayload())
      await abrirEditor()

      await fireEvent.input(screen.getByLabelText('Costo máximo'), { target: { value: '' } })
      invokeMock.mockClear()
      await fireEvent.click(screen.getByText('Guardar presupuesto'))

      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith('research_request', {
          request: {
            op: 'update_budget',
            job_id: 'job-65972-0',
            max_llm_calls: 40,
            max_cost: null,
          },
        })
      })
    })

    it('muestra lo ya consumido, que es el piso del techo nuevo', async () => {
      // El motor rechaza un límite por debajo de lo gastado. Decirlo antes
      // de guardar evita que el investigador lo descubra por un error.
      invokeMock.mockResolvedValue(detailPayload())
      await abrirEditor()

      expect(screen.getByText(/Van 21 llamadas/)).toBeInTheDocument()
    })

    it('no ofrece ajustarlo mientras la investigación corre', async () => {
      // El motor lo rechaza con el trabajo en marcha: ofrecerlo sería
      // ofrecer un error.
      invokeMock.mockResolvedValue(corriendoPayload())
      render(InvestigationView, {
        props: { jobId: 'job-65972-0', title: 'Investigación' },
      })

      await waitFor(() => {
        expect(screen.getByText('Pausar')).toBeInTheDocument()
      })
      expect(screen.queryByText('Ajustar presupuesto')).not.toBeInTheDocument()
    })
  })
})
