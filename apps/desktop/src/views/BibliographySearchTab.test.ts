import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
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

function passage(over: Record<string, unknown> = {}) {
  return {
    chunkId: 'chunk-1',
    itemId: 'item-1',
    itemKey: 'AAAA1111',
    title: 'Obra A',
    authors: 'Bloch',
    year: 1949,
    libraryName: 'Mi biblioteca',
    libraryType: 'user',
    libraryNativeId: '0',
    cslJson: '{"id":"x","title":"Obra A"}',
    snippet: 'El oficio de historiador es duro.',
    location: { kind: 'pages', from: 3, to: 3 },
    score: 0.9,
    ...over,
  }
}

function passagesResponse(passages = [passage()], notice: string | null = null) {
  return { passages, notice }
}

const readableContext = {
  chunkId: 'chunk-1',
  itemId: 'item-1',
  itemKey: 'AAAA1111',
  title: 'Obra A',
  text: 'El oficio',
  spans: [[3, 7, 16]],
  pages: [{ pageNumber: 3, text: 'Antes. El oficio. Después.', highlights: [[7, 16]] }],
  originalKind: null,
  originalPath: null,
  openError: null,
}

/** Answers each command on its own, as the real backend does. */
function backend(
  options: {
    works?: unknown
    passages?: unknown
    context?: unknown
  } = {}
) {
  mockInvoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'bibliography_search_works':
        return options.works ?? hybridResponse()
      case 'bibliography_search_passages':
        if (typeof options.passages === 'function') return (options.passages as () => never)()
        return options.passages ?? passagesResponse()
      case 'bibliography_passage_context':
        return options.context ?? readableContext
      default:
        throw new Error(`unexpected command ${command}`)
    }
  })
}

function callsFor(command: string) {
  return mockInvoke.mock.calls.filter(([name]) => name === command)
}

async function search(text = 'oficio') {
  await fireEvent.input(screen.getByRole('searchbox'), { target: { value: text } })
  await fireEvent.click(screen.getByRole('button', { name: 'Buscar' }))
}

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('BibliographySearchTab', () => {
  it('searches on submit and renders hits with method badges', async () => {
    backend()
    render(BibliographySearchTab)

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'revoluciones' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar' }))

    expect(mockInvoke).toHaveBeenCalledWith(
      'bibliography_search_works',
      expect.objectContaining({
        request: expect.objectContaining({ text: 'revoluciones' }),
      })
    )
    expect(
      await screen.findByText('Obra A', { selector: '.bib-search__title' })
    ).toBeInTheDocument()
    expect(screen.getByText('Obra B')).toBeInTheDocument()
    expect(screen.getByText('Híbrido')).toBeInTheDocument()
    expect(screen.getByText('Léxico')).toBeInTheDocument()
  })

  it('labels a vector-unavailable answer as lexical-only', async () => {
    backend({ works: { ...hybridResponse(), vectorAvailable: false } })
    render(BibliographySearchTab)

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'botánica' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar' }))

    expect(
      await screen.findByText('Solo búsqueda léxica: sin espacio vectorial activo.')
    ).toBeInTheDocument()
  })

  it('searches the manuscript selection verbatim on explicit click', async () => {
    backend()
    render(BibliographySearchTab, { props: { getSelection: () => '  revoluciones agrarias  ' } })

    expect(screen.getByText(/envía tu consulta/)).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar desde la selección' }))

    expect(mockInvoke).toHaveBeenCalledWith(
      'bibliography_search_works',
      expect.objectContaining({
        request: expect.objectContaining({ text: 'revoluciones agrarias' }),
      })
    )
    expect(await screen.findByText('Obra B')).toBeInTheDocument()
  })

  it('does not search when the selection is empty', async () => {
    backend()
    render(BibliographySearchTab, { props: { getSelection: () => '   ' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar desde la selección' }))

    expect(mockInvoke).not.toHaveBeenCalled()
    expect(
      await screen.findByText('No hay texto seleccionado en el manuscrito.')
    ).toBeInTheDocument()
  })

  it('shows empty and error states honestly', async () => {
    backend({ works: { ...hybridResponse(), hits: [] } })
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

describe('BibliographySearchTab passages', () => {
  it('searches passages for the same query, below the works', async () => {
    backend()
    render(BibliographySearchTab)
    await search('oficio')

    const snippet = await screen.findByText('El oficio de historiador es duro.')
    expect(callsFor('bibliography_search_passages')).toEqual([
      ['bibliography_search_passages', { request: { text: 'oficio', topK: 12 } }],
    ])
    const works = screen.getByText('Obra B')
    expect(
      works.compareDocumentPosition(snippet) & Node.DOCUMENT_POSITION_FOLLOWING,
      'passages come after the works'
    ).toBeTruthy()
    expect(screen.getByRole('heading', { name: 'Pasajes' })).toBeInTheDocument()
  })

  it('groups passages under their work with the shared location format', async () => {
    backend({
      passages: passagesResponse([
        passage(),
        passage({
          chunkId: 'chunk-2',
          snippet: 'Segundo pasaje de la misma obra.',
          location: { kind: 'pages', from: 7, to: 8 },
        }),
        passage({
          chunkId: 'chunk-3',
          itemId: 'item-9',
          itemKey: 'ZZZZ9999',
          title: 'Página web',
          authors: '',
          year: null,
          snippet: 'Texto de un snapshot.',
          location: { kind: 'paragraphs', from: 2, to: 3 },
        }),
      ]),
    })
    render(BibliographySearchTab)
    await search()

    await screen.findByText('Segundo pasaje de la misma obra.')
    const groups = screen.getAllByRole('group')
    expect(groups).toHaveLength(2)
    const first = groups[0]!
    expect(within(first).getByText('p. 3')).toBeInTheDocument()
    expect(within(first).getByText('pp. 7–8')).toBeInTheDocument()
    expect(within(first).getAllByText('Obra A')).toHaveLength(1)
    expect(within(groups[1]!).getByText('párr. 2–3')).toBeInTheDocument()
  })

  it('opens a passage in the shared reader, original only on request', async () => {
    backend()
    render(BibliographySearchTab)
    await search()
    await fireEvent.click(await screen.findByRole('button', { name: 'Abrir pasaje' }))

    const dialog = await screen.findByRole('dialog')
    await waitFor(() => expect(within(dialog).getByText('El oficio').tagName).toBe('MARK'))
    expect(callsFor('bibliography_passage_context')).toEqual([
      ['bibliography_passage_context', { chunkId: 'chunk-1' }],
    ])
    expect(callsFor('bibliography_open_passage')).toHaveLength(0)

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cerrar' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })

  it('cites the work at the passage location', async () => {
    backend()
    const oncite = vi.fn(() => 'node-1')
    render(BibliographySearchTab, { props: { oncite } })
    await search()
    await fireEvent.click(await screen.findByRole('button', { name: 'Citar pasaje' }))

    expect(oncite).toHaveBeenCalledWith({
      sourceOrigin: 'local',
      sourceInstanceId: null,
      itemKey: 'AAAA1111',
      itemVersion: null,
      libraryType: 'user',
      libraryId: '0',
      metadataSnapshot: '{"id":"x","title":"Obra A"}',
      locator: '3',
      locatorType: 'page',
    })
    expect(await screen.findByText('Cita insertada en el manuscrito.')).toBeInTheDocument()
  })

  it('cites a snapshot passage by paragraph and an unlocated one without locator', async () => {
    backend({
      passages: passagesResponse([
        passage({ location: { kind: 'paragraphs', from: 2, to: 3 } }),
        passage({ chunkId: 'chunk-2', snippet: 'Sin lugar.', location: null }),
      ]),
    })
    const oncite = vi.fn((_attrs: Record<string, unknown>) => 'node-1')
    render(BibliographySearchTab, { props: { oncite } })
    await search()
    const buttons = await screen.findAllByRole('button', { name: 'Citar pasaje' })

    await fireEvent.click(buttons[0]!)
    expect(oncite).toHaveBeenLastCalledWith(
      expect.objectContaining({ locator: '2-3', locatorType: 'paragraph' })
    )
    await fireEvent.click(buttons[1]!)
    const unlocated = oncite.mock.calls.at(-1)![0]
    expect(unlocated).not.toHaveProperty('locator')
    expect(unlocated).not.toHaveProperty('locatorType')
  })

  it('cannot cite without an open manuscript and says so', async () => {
    backend()
    render(BibliographySearchTab)
    await search()

    expect(await screen.findByRole('button', { name: 'Citar pasaje' })).toBeDisabled()
    expect(screen.getByText('Abrí un documento para insertar la cita.')).toBeInTheDocument()
  })

  it('does not offer to cite a passage whose work has no CSL data', async () => {
    backend({ passages: passagesResponse([passage({ cslJson: '' })]) })
    render(BibliographySearchTab, { props: { oncite: vi.fn() } })
    await search()

    expect(await screen.findByRole('button', { name: 'Citar pasaje' })).toBeDisabled()
  })

  it.each([
    ['no_embeddings', /todavía no tiene vectores/],
    ['no_library_synced', /no hay ninguna biblioteca sincronizada/],
    ['embedding_unavailable', /no se pudo calcular el vector/],
    ['failed', /La búsqueda de pasajes falló/],
  ])('says why there are no passages (%s), never an empty list', async (notice, text) => {
    backend({ passages: passagesResponse([], notice) })
    render(BibliographySearchTab)
    await search()

    expect(await screen.findByText(text)).toBeInTheDocument()
    expect(screen.queryByText('Sin pasajes para esta consulta.')).toBeNull()
    expect(screen.getByText('Obra A')).toBeInTheDocument()
  })

  it('names the cause when the backend says why the passage search failed', async () => {
    backend({
      passages: { passages: [], notice: 'failed', noticeDetail: 'sql_error: sin tabla' },
    })
    render(BibliographySearchTab)
    await search()

    expect(
      await screen.findByText(/La búsqueda de pasajes falló.*sql_error: sin tabla/)
    ).toBeInTheDocument()
  })

  it('says plainly when the search ran and found no passages', async () => {
    backend({ passages: passagesResponse([]) })
    render(BibliographySearchTab)
    await search()

    expect(await screen.findByText('Sin pasajes para esta consulta.')).toBeInTheDocument()
  })

  it('keeps the works when the passage search fails', async () => {
    backend({
      passages: () => {
        throw new Error('boom')
      },
    })
    render(BibliographySearchTab)
    await search()

    expect(await screen.findByText('Obra A')).toBeInTheDocument()
    expect(await screen.findByText(/La búsqueda de pasajes falló/)).toBeInTheDocument()
  })

  it('searches passages from the manuscript selection too', async () => {
    backend()
    render(BibliographySearchTab, { props: { getSelection: () => ' oficio de historiador ' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar desde la selección' }))

    await screen.findByText('El oficio de historiador es duro.')
    expect(callsFor('bibliography_search_passages')[0]![1]).toEqual({
      request: { text: 'oficio de historiador', topK: 12 },
    })
  })
})
