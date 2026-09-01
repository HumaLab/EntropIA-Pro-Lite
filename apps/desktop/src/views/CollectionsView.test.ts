import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CollectionsView from './CollectionsView.svelte'
import { locale } from '$lib/i18n'

const { storeRef, navigationRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      collections: {
        findAll: vi.fn(),
        findAllNonEmpty: vi.fn(),
        countItems: vi.fn(),
        create: vi.fn(),
        delete: vi.fn(),
      },
    },
  },
  navigationRef: {
    navigate: vi.fn(),
  },
}))

type CollectionRow = {
  id: string
  name: string
  description: string | null
  createdAt: number
  updatedAt: number
}

function createStore(collections: CollectionRow[], count = 0) {
  return {
    collections: {
      findAll: vi.fn().mockResolvedValue(collections),
      findAllNonEmpty: vi.fn().mockResolvedValue(collections),
      countItems: vi.fn().mockResolvedValue(count),
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn(),
    },
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('$lib/navigation', () => ({
  navigation: navigationRef,
}))

describe('CollectionsView consumer compatibility', () => {
  beforeEach(() => {
    locale.set('es')
    navigationRef.navigate.mockReset()
    storeRef.current = createStore(
      [
        {
          id: 'col-1',
          name: 'Historia',
          description: 'Colección histórica',
          createdAt: Date.now(),
          updatedAt: Date.now() - 11 * 60 * 60 * 1000,
        },
      ],
      7
    )
  })

  it('exposes the localized clear action for a non-empty search', async () => {
    render(CollectionsView)

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'historia' } })

    const clearButton = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    expect(clearButton).toHaveAttribute('title', 'Limpiar búsqueda')
    await fireEvent.click(clearButton)
  })

  it('keeps the search field accessible without a redundant external label', async () => {
    const { container } = render(CollectionsView)

    expect(
      await screen.findByRole('searchbox', { name: 'Buscar colecciones...' })
    ).toBeInTheDocument()
    expect(container.querySelector('.collections-controls__label')).not.toBeInTheDocument()
  })

  it('renders new collection as a folder-only control and opens the create form', async () => {
    render(CollectionsView)

    const newCollection = await screen.findByRole('button', { name: 'Nueva colección' })

    expect(newCollection).toHaveAttribute('title', 'Nueva colección')
    expect(newCollection.querySelector('svg')).not.toBeNull()
    expect(newCollection.textContent?.trim()).toBe('')

    await fireEvent.click(newCollection)

    expect(screen.getByPlaceholderText('Nombre de la colección')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: 'Cancelar' })).toHaveLength(2)
  })

  it('passes CollectionCard props and preserves onclick navigation contract', async () => {
    const { container } = render(CollectionsView)

    expect(await screen.findByText('Historia')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Colecciones' })).toBeInTheDocument()
    expect(container.querySelector('.page-header')).not.toBeInTheDocument()
    expect(container.querySelector('.page-toolbar')).not.toBeInTheDocument()
    expect(
      screen.getByText('Gestioná tus espacios de trabajo y organizá el archivo por tema.')
    ).toBeInTheDocument()
    const count = screen.getByText('1 colección')
    expect(count.tagName).toBe('SPAN')
    expect(count).toHaveClass('collections-intro__meta')
    expect(screen.queryByRole('button', { name: '1 colección' })).not.toBeInTheDocument()
    expect(await screen.findByText('7 items')).toBeInTheDocument()
    expect(await screen.findByText('Colección histórica')).toBeInTheDocument()

    const card = (await screen.findByRole('button', { name: /Historia/i })) as HTMLButtonElement

    await fireEvent.click(card)

    await waitFor(() => {
      expect(navigationRef.navigate).toHaveBeenCalledWith({
        name: 'collection',
        id: 'col-1',
        collectionName: 'Historia',
      })
    })
  })

  it('shows the empty-state guidance when there are no collections', async () => {
    storeRef.current = createStore([], 0)

    render(CollectionsView)

    expect(screen.getByRole('heading', { name: 'Colecciones' })).toBeInTheDocument()
    expect(screen.getByText('0 colecciones')).toBeInTheDocument()

    expect(
      await screen.findByText(
        'Todavía no hay colecciones. Creá una para empezar a ordenar el material.'
      )
    ).toBeInTheDocument()
  })

  it('uses the collection name as the description when no description is available', async () => {
    storeRef.current = createStore(
      [
        {
          id: 'col-without-description',
          name: 'Archivo 1930',
          description: null,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        },
      ],
      0
    )

    render(CollectionsView)

    expect(await screen.findByTestId('collection-description')).toHaveTextContent('Archivo 1930')
  })

  it('renders the confirm delete action as an icon-only trash button', async () => {
    render(CollectionsView)

    await fireEvent.click(await screen.findByRole('button', { name: 'Delete collection' }))

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar colección' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    expect(confirmBtn).not.toHaveTextContent('Eliminar')
  })

  it('updates critical collection copy when locale changes', async () => {
    render(CollectionsView)

    expect(await screen.findByRole('heading', { name: 'Colecciones' })).toBeInTheDocument()
    expect(await screen.findByTestId('collection-date')).toHaveTextContent('hace 11 horas')
    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'historia' } })
    expect(screen.getByRole('button', { name: 'Limpiar búsqueda' })).toHaveAttribute(
      'title',
      'Limpiar búsqueda'
    )

    locale.set('en')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Collections' })).toBeInTheDocument()
      expect(screen.getByText('1 collection')).toBeInTheDocument()
      expect(screen.getByTestId('collection-date')).toHaveTextContent('11 hours ago')
      const clearButton = screen.getByRole('button', { name: 'Clear search' })
      expect(clearButton).toHaveAttribute('title', 'Clear search')
    })
  })

  it('tells the document explorer to reload after creating, renaming, or deleting', async () => {
    const listener = vi.fn()
    window.addEventListener('entropia:document-explorer-collections-changed', listener)

    try {
      render(CollectionsView)

      await fireEvent.click(await screen.findByRole('button', { name: 'Nueva colección' }))
      await fireEvent.input(screen.getByPlaceholderText('Nombre de la colección'), {
        target: { value: 'Prensa' },
      })
      await fireEvent.click(screen.getByRole('button', { name: 'Crear colección' }))

      await waitFor(() => expect(listener).toHaveBeenCalledTimes(1))

      await fireEvent.click(await screen.findByRole('button', { name: 'Edit collection' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))

      await waitFor(() => expect(listener).toHaveBeenCalledTimes(2))

      await fireEvent.click(await screen.findByRole('button', { name: 'Delete collection' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar colección' }))

      await waitFor(() => expect(listener).toHaveBeenCalledTimes(3))
    } finally {
      window.removeEventListener('entropia:document-explorer-collections-changed', listener)
    }
  })

  it('does not notify the document explorer when the write fails', async () => {
    const listener = vi.fn()
    storeRef.current.collections.create.mockRejectedValue(new Error('disco lleno'))
    window.addEventListener('entropia:document-explorer-collections-changed', listener)

    try {
      render(CollectionsView)

      await fireEvent.click(await screen.findByRole('button', { name: 'Nueva colección' }))
      await fireEvent.input(screen.getByPlaceholderText('Nombre de la colección'), {
        target: { value: 'Prensa' },
      })
      await fireEvent.click(screen.getByRole('button', { name: 'Crear colección' }))

      expect(await screen.findByText('disco lleno')).toBeInTheDocument()
      expect(listener).not.toHaveBeenCalled()
    } finally {
      window.removeEventListener('entropia:document-explorer-collections-changed', listener)
    }
  })

  it('ignores stale collection loads that resolve after a newer refresh', async () => {
    const firstLoad = deferred<CollectionRow[]>()
    const secondLoad = deferred<CollectionRow[]>()
    const oldCollection: CollectionRow = {
      id: 'col-old',
      name: 'Historia vieja',
      description: null,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    }
    const newCollection: CollectionRow = {
      id: 'col-new',
      name: 'Historia nueva',
      description: null,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    }
    const findAll = vi
      .fn()
      .mockReturnValueOnce(firstLoad.promise)
      .mockReturnValueOnce(secondLoad.promise)

    storeRef.current = {
      collections: {
        findAll,
        findAllNonEmpty: vi.fn(),
        countItems: vi.fn().mockResolvedValue(0),
        create: vi.fn().mockResolvedValue(newCollection),
        delete: vi.fn(),
      },
    }

    render(CollectionsView)

    await fireEvent.click(screen.getByRole('button', { name: 'Nueva colección' }))
    await fireEvent.input(screen.getByPlaceholderText('Nombre de la colección'), {
      target: { value: 'Historia nueva' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Crear colección' }))

    secondLoad.resolve([newCollection])

    expect(await screen.findByRole('heading', { name: 'Historia nueva' })).toBeInTheDocument()

    firstLoad.resolve([oldCollection])

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Historia nueva' })).toBeInTheDocument()
      expect(screen.queryByRole('heading', { name: 'Historia vieja' })).not.toBeInTheDocument()
    })
  })
})
