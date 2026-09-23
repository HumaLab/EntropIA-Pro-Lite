import { fireEvent, render, screen } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import BibliographySearchTab from './BibliographySearchTab.svelte'

const mockInvoke = vi.mocked(invoke)

function hybridResponse() {
  return {
    hits: [
      {
        itemId: 'item-1',
        itemKey: 'AAAA1111',
        libraryId: 'lib-1',
        title: 'Obra A',
        method: 'hybrid',
        lexicalScore: -3.2,
        vectorScore: 0.87,
        fusedScore: 0.033,
        contractHash: 'contract-1',
        generationId: 'gen-1',
      },
      {
        itemId: 'item-2',
        itemKey: 'BBBB2222',
        libraryId: 'lib-1',
        title: 'Obra B',
        method: 'lexical',
        lexicalScore: -1.1,
        vectorScore: null,
        fusedScore: -1.1,
        contractHash: null,
        generationId: null,
      },
    ],
    vectorAvailable: true,
    activeGenerationId: 'gen-1',
    contractHash: 'contract-1',
  }
}

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('BibliographySearchTab', () => {
  it('searches on submit and renders hits with method badges', async () => {
    mockInvoke.mockResolvedValue(hybridResponse())
    render(BibliographySearchTab)

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'revoluciones' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar' }))

    expect(mockInvoke).toHaveBeenCalledWith(
      'bibliography_search_works',
      expect.objectContaining({
        request: expect.objectContaining({ text: 'revoluciones' }),
      })
    )
    expect(await screen.findByText('Obra A')).toBeInTheDocument()
    expect(screen.getByText('Obra B')).toBeInTheDocument()
    expect(screen.getByText('Híbrido')).toBeInTheDocument()
    expect(screen.getByText('Léxico')).toBeInTheDocument()
  })

  it('labels a vector-unavailable answer as lexical-only', async () => {
    mockInvoke.mockResolvedValue({ ...hybridResponse(), vectorAvailable: false })
    render(BibliographySearchTab)

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'botánica' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar' }))

    expect(
      await screen.findByText('Solo búsqueda léxica: sin espacio vectorial activo.')
    ).toBeInTheDocument()
  })

  it('searches the manuscript selection verbatim on explicit click', async () => {
    mockInvoke.mockResolvedValue(hybridResponse())
    render(BibliographySearchTab, { props: { getSelection: () => '  revoluciones agrarias  ' } })

    expect(screen.getByText(/envía tu consulta/)).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar desde la selección' }))

    expect(mockInvoke).toHaveBeenCalledWith(
      'bibliography_search_works',
      expect.objectContaining({
        request: expect.objectContaining({ text: 'revoluciones agrarias' }),
      })
    )
    expect(await screen.findByText('Obra A')).toBeInTheDocument()
  })

  it('does not search when the selection is empty', async () => {
    mockInvoke.mockResolvedValue(hybridResponse())
    render(BibliographySearchTab, { props: { getSelection: () => '   ' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar desde la selección' }))

    expect(mockInvoke).not.toHaveBeenCalled()
    expect(
      await screen.findByText('No hay texto seleccionado en el manuscrito.')
    ).toBeInTheDocument()
  })

  it('shows empty and error states honestly', async () => {
    mockInvoke.mockResolvedValue({ ...hybridResponse(), hits: [] })
    render(BibliographySearchTab)

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'zzz' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar' }))
    expect(await screen.findByText('Sin resultados para esta consulta.')).toBeInTheDocument()

    mockInvoke.mockRejectedValue(new Error('engine exploded'))
    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'otra' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar' }))
    expect(await screen.findByText(/engine exploded/)).toBeInTheDocument()
  })
})
