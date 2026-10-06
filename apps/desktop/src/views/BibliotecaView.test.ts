import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import BibliotecaView from './BibliotecaView.svelte'

const mockInvoke = vi.mocked(invoke)

const { navigationRef, workspaceRef } = vi.hoisted(() => ({
  navigationRef: { navigate: vi.fn() },
  workspaceRef: { navigateActive: vi.fn() },
}))

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => navigationRef,
  getPaneId: () => 'pane-test',
}))

vi.mock('$lib/workspace', () => ({
  workspace: workspaceRef,
}))

function work(over: Record<string, unknown> = {}) {
  return {
    itemId: 'item-1',
    itemKey: 'AAAA1111',
    libraryRowId: 'lib-row-1',
    title: 'El oficio de historiador',
    authors: 'Bloch',
    year: 1949,
    libraryName: 'Mi biblioteca',
    libraryType: 'user',
    libraryNativeId: '0',
    cslJson: '{"id":"x"}',
    ...over,
  }
}

function library(over: Record<string, unknown> = {}) {
  return {
    libraryType: 'user',
    libraryId: '0',
    name: 'Mi biblioteca',
    works: 2,
    passages: 0,
    ...over,
  }
}

/** Answers each command on its own, as the real backend does. */
function backend(
  options: {
    libraries?: unknown[]
    page?:
      | { works: unknown[]; total: number }
      | ((offset: number) => { works: unknown[]; total: number })
    pageError?: boolean
    search?: unknown
  } = {}
) {
  mockInvoke.mockImplementation(async (command: string, payload?: unknown) => {
    switch (command) {
      case 'bibliography_library_status':
        return { libraries: options.libraries ?? [library()], vectorReady: false }
      case 'bibliography_list_works': {
        if (options.pageError) throw new Error('boom')
        const offset = (payload as { request: { offset: number } }).request.offset
        const page =
          typeof options.page === 'function'
            ? options.page(offset)
            : (options.page ?? { works: [work()], total: 1 })
        return page
      }
      case 'bibliography_search_works':
        return (
          options.search ?? {
            hits: [work({ itemId: 'item-2', title: 'La sociedad' })],
            vectorAvailable: false,
            activeGenerationId: null,
            contractHash: 'hash',
            librarySynced: true,
          }
        )
      default:
        throw new Error(`unexpected command ${command}`)
    }
  })
}

function callsFor(command: string) {
  return mockInvoke.mock.calls.filter(([name]) => name === command)
}

beforeEach(() => {
  mockInvoke.mockReset()
  navigationRef.navigate.mockReset()
  workspaceRef.navigateActive.mockReset()
  localStorage.clear()
  locale.set('es')
})

describe('BibliotecaView', () => {
  it('lists the works of the synced libraries as rows', async () => {
    backend({
      page: {
        works: [
          work(),
          work({ itemId: 'item-2', title: 'La sociedad', authors: 'Bloch', year: 1949 }),
        ],
        total: 2,
      },
    })
    render(BibliotecaView)

    expect(await screen.findByText('El oficio de historiador')).toBeInTheDocument()
    expect(screen.getByText('La sociedad')).toBeInTheDocument()
    // Reading line under the title: authors · year.
    expect(screen.getAllByText('Bloch · 1949')).toHaveLength(2)
    expect(callsFor('bibliography_list_works')[0]?.[1]).toEqual({
      request: {
        offset: 0,
        limit: 50,
        query: null,
        zoteroLibraryType: null,
        zoteroLibraryId: null,
        sort: 'title',
      },
    })
  })

  it('opens the work view when a row is clicked', async () => {
    backend()
    render(BibliotecaView)

    await fireEvent.click(await screen.findByText('El oficio de historiador'))

    expect(navigationRef.navigate).toHaveBeenCalledWith({
      name: 'bibliography-work',
      libraryRowId: 'lib-row-1',
      itemId: 'item-1',
      itemKey: 'AAAA1111',
      title: 'El oficio de historiador',
    })
  })

  it('shows the empty state and a way to the Zotero tab when no library is synced', async () => {
    backend({ libraries: [] })
    render(BibliotecaView)

    expect(await screen.findByText('Todavía no hay obras sincronizadas.')).toBeInTheDocument()
    expect(
      screen.getByText('Sincronizá una biblioteca de Zotero desde Escritura › Zotero.')
    ).toBeInTheDocument()
    expect(callsFor('bibliography_list_works')).toEqual([])

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir Escritura' }))
    expect(workspaceRef.navigateActive).toHaveBeenCalledWith({ name: 'writing' })
  })

  it('offers the library picker only when more than one library is synced', async () => {
    backend({
      libraries: [library(), library({ libraryType: 'group', libraryId: '7', name: 'Grupo' })],
    })
    render(BibliotecaView)

    const trigger = await screen.findByRole('button', { name: /Elegir biblioteca/ })
    await fireEvent.click(trigger)
    await fireEvent.click(await screen.findByRole('menuitemradio', { name: /Grupo/ }))

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_list_works', {
        request: {
          offset: 0,
          limit: 50,
          query: null,
          zoteroLibraryType: 'group',
          zoteroLibraryId: '7',
          sort: 'title',
        },
      })
    })
  })

  it('hides the picker for a single synced library', async () => {
    backend()
    render(BibliotecaView)

    await screen.findByText('El oficio de historiador')
    expect(screen.queryByRole('button', { name: /Elegir biblioteca/ })).toBeNull()
  })

  it('searches through the works search and renders the hits as the same rows', async () => {
    backend()
    render(BibliotecaView)

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'sociedad' } })

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_search_works', expect.anything())
    })
    expect(await screen.findByText('La sociedad')).toBeInTheDocument()
  })

  it('loads the next page of works on demand', async () => {
    backend({
      page: (offset) =>
        offset === 0
          ? { works: [work()], total: 2 }
          : { works: [work({ itemId: 'item-2', title: 'La sociedad' })], total: 2 },
    })
    render(BibliotecaView)

    await screen.findByText('El oficio de historiador')
    await fireEvent.click(screen.getByRole('button', { name: 'Cargar más obras' }))

    expect(await screen.findByText('La sociedad')).toBeInTheDocument()
    // The next page starts after the works already listed.
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_list_works', {
        request: {
          offset: 1,
          limit: 50,
          query: null,
          zoteroLibraryType: null,
          zoteroLibraryId: null,
          sort: 'title',
        },
      })
    })
  })

  it('sends the chosen order to the works listing and remembers it', async () => {
    backend()
    render(BibliotecaView)
    await screen.findByText('El oficio de historiador')

    await fireEvent.click(screen.getByRole('button', { name: 'Elegir orden' }))
    await fireEvent.click(await screen.findByRole('menuitemradio', { name: 'Recientes' }))

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_list_works', {
        request: {
          offset: 0,
          limit: 50,
          query: null,
          zoteroLibraryType: null,
          zoteroLibraryId: null,
          sort: 'recent',
        },
      })
    })
    expect(localStorage.getItem('entropia:biblioteca:sort')).toBe('recent')
  })

  it('lists in the remembered order on the next mount', async () => {
    localStorage.setItem('entropia:biblioteca:sort', 'recent')
    backend()
    render(BibliotecaView)

    await screen.findByText('El oficio de historiador')
    expect(callsFor('bibliography_list_works')[0]?.[1]).toEqual({
      request: {
        offset: 0,
        limit: 50,
        query: null,
        zoteroLibraryType: null,
        zoteroLibraryId: null,
        sort: 'recent',
      },
    })
  })

  it('says when the works could not be loaded, with a retry', async () => {
    backend({ pageError: true })
    render(BibliotecaView)

    expect(await screen.findByText('No se pudieron cargar las obras.')).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))
    await waitFor(() => {
      expect(callsFor('bibliography_list_works').length).toBeGreaterThan(1)
    })
  })
})
