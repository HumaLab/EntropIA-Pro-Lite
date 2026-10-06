/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'

const { invokeMock, navigateMock, forgetResearchMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  navigateMock: vi.fn(),
  forgetResearchMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => ({
    navigate: navigateMock,
  }),
  getPaneId: () => 'pane-test',
}))

vi.mock('$lib/workspace', () => ({
  workspace: {
    forgetResearch: forgetResearchMock,
  },
}))

vi.mock('@entropia/ui', async () => {
  const actual = await vi.importActual<typeof import('@entropia/ui')>('@entropia/ui')
  const MockButton = (await import('./__mocks__/MockButton.svelte')).default
  const MockActionIcon = (await import('./__mocks__/MockActionIcon.svelte')).default
  return {
    ...actual,
    Button: MockButton,
    IconButton: MockButton,
    ActionIcon: MockActionIcon,
  }
})

import ResearchView from './ResearchView.svelte'

function listPayload() {
  return {
    jobs: [],
    collections: [
      {
        id: 'c-conflicto',
        name: 'Conflicto SOIP 1965-66',
        items: 148,
        items_with_chunks: 12,
        chunks: 40,
      },
      { id: 'c-voces', name: 'Voces', items: 12, items_with_chunks: 7, chunks: 709 },
    ],
    modalidades: [{ id: 'general', name: 'Informe general' }],
  }
}

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe('ResearchView', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset()
    invokeMock.mockResolvedValue(listPayload())
  })

  it('arranca con las colecciones que tienen material procesado', async () => {
    render(ResearchView)
    await waitFor(() => {
      expect(screen.getByText('Conflicto SOIP 1965-66')).toBeInTheDocument()
    })
    const casillas = screen.getAllByRole('checkbox') as HTMLInputElement[]
    expect(casillas.every((c) => c.checked)).toBe(true)
  })

  it('deseleccionar todo sobrevive al refresco periódico', async () => {
    vi.useFakeTimers()
    render(ResearchView)

    await vi.advanceTimersByTimeAsync(0)
    const casillas = () => screen.getAllByRole('checkbox') as HTMLInputElement[]
    expect(casillas().every((c) => c.checked)).toBe(true)

    // El botón alterna: con todo seleccionado, deselecciona.
    await fireEvent.click(screen.getByRole('button', { name: 'Deseleccionar todas' }))
    expect(casillas().some((c) => c.checked)).toBe(false)

    // El polling corre cada 1,5 s y antes volvía a seleccionar todo: un
    // alcance vacío que el investigador eligió es una decisión, no un estado
    // a corregir.
    await vi.advanceTimersByTimeAsync(5000)
    expect(casillas().some((c) => c.checked)).toBe(false)
  })

  it('el botón alterna entre seleccionar y deseleccionar', async () => {
    render(ResearchView)
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Deseleccionar todas' })).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Deseleccionar todas' }))
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Seleccionar todas' })).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Seleccionar todas' }))
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Deseleccionar todas' })).toBeInTheDocument()
    })
  })
})

function jobFixture() {
  return {
    id: 'job-1',
    title: 'Pregunta de prueba',
    question: 'Pregunta de prueba',
    status: 'done' as const,
    phase: 'report' as const,
    llm_calls: 5,
    max_llm_calls: 40,
    cost: null,
    max_cost: null,
    close_reason: null,
  }
}

function listPayloadWithJob() {
  return { ...listPayload(), jobs: [jobFixture()] }
}

/**
 * Regression: `back()` could land on a deleted investigation's own screen
 * once it no longer existed. Only a successful delete prunes history — a
 * rejected one leaves it alone, matching that it also leaves the job listed.
 */
describe('ResearchView job deletion', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset()
    navigateMock.mockReset()
    forgetResearchMock.mockReset()
  })

  async function openDeleteConfirm() {
    render(ResearchView)
    await waitFor(() => {
      expect(screen.getByText('Pregunta de prueba')).toBeInTheDocument()
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Borrar la investigación' }))
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  }

  it('prunes history for the job once the delete succeeds', async () => {
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      const op = (args as { request?: { op?: string } } | undefined)?.request?.op
      if (command === 'research_request' && op === 'delete') return {}
      return listPayloadWithJob()
    })

    await openDeleteConfirm()
    await fireEvent.click(screen.getByRole('button', { name: 'Borrar' }))

    await waitFor(() => {
      expect(forgetResearchMock).toHaveBeenCalledWith('job-1')
    })
  })

  it('does not prune history when the delete fails', async () => {
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      const op = (args as { request?: { op?: string } } | undefined)?.request?.op
      if (command === 'research_request' && op === 'delete') {
        throw new Error('backend unavailable')
      }
      return listPayloadWithJob()
    })

    await openDeleteConfirm()
    await fireEvent.click(screen.getByRole('button', { name: 'Borrar' }))

    await waitFor(() => {
      expect(screen.getByText('backend unavailable')).toBeInTheDocument()
    })
    expect(forgetResearchMock).not.toHaveBeenCalled()
  })
})

describe('ResearchView alcance de la investigación', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset()
    navigateMock.mockReset()
  })

  const LIBRARIES = [
    { libraryType: 'user', libraryId: '0', name: 'Mi biblioteca', works: 12, passages: 340 },
    { libraryType: 'group', libraryId: '77', name: 'Grupo Anales', works: 4, passages: 90 },
  ]

  /** El backend de la vista: estado de bibliotecas + create + list. */
  function backend({ libraries = LIBRARIES }: { libraries?: typeof LIBRARIES } = {}) {
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      const request = (args as { request?: { op?: string } } | undefined)?.request
      if (command === 'bibliography_library_status') {
        return { libraries, vectorReady: true }
      }
      if (command === 'research_request' && request?.op === 'create') {
        return { job: jobFixture() }
      }
      return listPayload()
    })
  }

  function questionBox() {
    return screen.getByPlaceholderText('¿Qué querés investigar?') as HTMLTextAreaElement
  }

  async function writeQuestion() {
    await fireEvent.input(questionBox(), { target: { value: '¿Qué pasó en el plenario?' } })
  }

  function createCalls() {
    return invokeMock.mock.calls.filter(
      ([command, args]) =>
        command === 'research_request' &&
        (args as { request?: { op?: string } } | undefined)?.request?.op === 'create'
    )
  }

  it('manda alcance y bibliotecas cuando el alcance es Biblioteca', async () => {
    backend()
    render(ResearchView)
    await waitFor(() => {
      expect(screen.getByText('Conflicto SOIP 1965-66')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('tab', { name: 'Biblioteca' }))
    const trigger = await screen.findByRole('button', { name: /Todas las bibliotecas/ })

    // El menú de bibliotecas es el mismo ToolbarMenu del chat: se estrecha
    // desmarcando una, y las refs viajan como «user:0» / «group:77».
    await fireEvent.click(trigger)
    const grupo = await screen.findByRole('menuitemcheckbox', { name: /Grupo Anales/ })
    await fireEvent.click(grupo)
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /1 de 2 bibliotecas/ })).toBeInTheDocument()
    )

    await writeQuestion()
    await fireEvent.click(screen.getByRole('button', { name: 'Investigar' }))

    await waitFor(() => expect(createCalls()).toHaveLength(1))
    expect(createCalls()[0]![1]).toEqual({
      request: expect.objectContaining({
        op: 'create',
        alcance: 'biblioteca',
        bibliotecas: ['user:0'],
      }),
    })
  })

  it('un alcance de corpus manda bibliotecas vacías y conserva las colecciones', async () => {
    backend()
    render(ResearchView)
    await waitFor(() => {
      expect(screen.getByText('Conflicto SOIP 1965-66')).toBeInTheDocument()
    })

    await writeQuestion()
    await fireEvent.click(screen.getByRole('button', { name: 'Investigar' }))

    await waitFor(() => expect(createCalls()).toHaveLength(1))
    expect(createCalls()[0]![1]).toEqual({
      request: expect.objectContaining({
        op: 'create',
        alcance: 'corpus',
        bibliotecas: [],
        collection_ids: ['c-conflicto', 'c-voces'],
      }),
    })
  })

  it('sin bibliotecas sincronizadas el alcance bibliográfico no se envía', async () => {
    backend({ libraries: [] })
    render(ResearchView)
    await waitFor(() => {
      expect(screen.getByText('Conflicto SOIP 1965-66')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('tab', { name: 'Ambos' }))
    await waitFor(() =>
      expect(
        screen.getByText(/Todavía no hay ninguna biblioteca de Zotero sincronizada/)
      ).toBeVisible()
    )

    await writeQuestion()
    const submit = screen.getByRole('button', { name: 'Investigar' })
    expect(submit).toBeDisabled()
    await fireEvent.click(submit)
    expect(createCalls()).toHaveLength(0)
  })
})
