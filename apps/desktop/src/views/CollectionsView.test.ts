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
      delete: vi.fn(),
    },
  }
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
          updatedAt: 1711000000000,
        },
      ],
      7
    )
  })

  it('passes CollectionCard props and preserves onclick navigation contract', async () => {
    render(CollectionsView)

    expect(await screen.findByText('Historia')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Colecciones' })).toBeInTheDocument()
    expect(
      screen.getByText('Gestioná tus espacios de trabajo y organizá el archivo por tema.')
    ).toBeInTheDocument()
    expect(screen.getByText('1 colección visible')).toBeInTheDocument()
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
    expect(screen.getByText('0 colecciones visibles')).toBeInTheDocument()

    expect(
      await screen.findByText(
        'Todavía no hay colecciones. Creá una para empezar a ordenar el material.'
      )
    ).toBeInTheDocument()
  })

  it('renders the confirm delete action as an icon-only trash button', async () => {
    render(CollectionsView)

    await fireEvent.click(await screen.findByRole('button', { name: 'Delete collection' }))

    const dialog = screen.getByRole('alertdialog', { name: 'Eliminar colección' })
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleDescription(
      '¿Estás seguro que querés eliminar la colección "Historia"? Se eliminarán todos sus items y datos asociados.'
    )

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar colección' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    expect(confirmBtn).not.toHaveTextContent('Eliminar')
  })

  it('closes the confirm delete dialog with Escape', async () => {
    render(CollectionsView)

    await fireEvent.click(await screen.findByRole('button', { name: 'Delete collection' }))
    const dialog = screen.getByRole('alertdialog', { name: 'Eliminar colección' })

    await fireEvent.keyDown(dialog, { key: 'Escape' })

    expect(screen.queryByRole('alertdialog', { name: 'Eliminar colección' })).not.toBeInTheDocument()
  })

  it('updates critical collection copy when locale changes', async () => {
    render(CollectionsView)

    expect(await screen.findByRole('heading', { name: 'Colecciones' })).toBeInTheDocument()

    locale.set('en')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Collections' })).toBeInTheDocument()
      expect(screen.getByText('1 visible collection')).toBeInTheDocument()
    })
  })
})
