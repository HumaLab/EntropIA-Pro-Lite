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
    IconButton: MockButton,
    ActionIcon: MockActionIcon,
  }
})

import ResearchView from './ResearchView.svelte'

function listPayload() {
  return {
    jobs: [],
    collections: [
      { id: 'c-conflicto', name: 'Conflicto SOIP 1965-66', items: 148, items_with_chunks: 12, chunks: 40 },
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
    await fireEvent.click(screen.getByTitle('Deseleccionar todas'))
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
      expect(screen.getByTitle('Deseleccionar todas')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByTitle('Deseleccionar todas'))
    await waitFor(() => {
      expect(screen.getByTitle('Seleccionar todas')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByTitle('Seleccionar todas'))
    await waitFor(() => {
      expect(screen.getByTitle('Deseleccionar todas')).toBeInTheDocument()
    })
  })
})
