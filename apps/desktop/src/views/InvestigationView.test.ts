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
      question: 'Realizar una investigación sobre la organización y la lucha de los y las obreras del pescado en los años 1960-1970',
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
      { id: 'gate-round', kind: 'clarification_round', artifact_id: 'art-round', status: 'pending' },
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
        }),
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
      expect(
        screen.getByText('¿Qué recorte temporal delimita el informe?'),
      ).toBeInTheDocument()
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
      screen.getByText('Respondé al menos una pregunta: el encuadre del informe depende de esto.'),
    ).toBeInTheDocument()
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
    expect(
      screen.getAllByText('Conflicto SOIP 1965-66 · 65-04-12-b · 1965-04-12'),
    ).toHaveLength(2)

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
      screen.getByText('Elegí una cita del informe para ver su fuente acá.'),
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
      screen.queryByText('Elegí una cita del informe para ver su fuente acá.'),
    ).not.toBeInTheDocument()
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
})
