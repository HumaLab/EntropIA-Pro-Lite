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
