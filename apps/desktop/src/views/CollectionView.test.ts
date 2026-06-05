import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import CollectionView from './CollectionView.svelte'
import { locale } from '$lib/i18n'

const { storeRef, navigationRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      items: {
        findByCollection: vi.fn(),
        searchByText: vi.fn(),
        create: vi.fn(),
        update: vi.fn(),
        delete: vi.fn(),
        deleteWithCascade: vi.fn(),
      },
      assets: {
        create: vi.fn(),
        findByItem: vi.fn(),
        findById: vi.fn(),
        deleteWithCascade: vi.fn(),
      },
    },
  },
  navigationRef: {
    current: { name: 'collection', collectionName: 'Colección' } as const,
    navigate: vi.fn(),
  },
}))

type ItemRow = {
  id: string
  title: string
  createdAt: number
  updatedAt: number
  collectionId: string
  metadata: string | null
}

type AssetRow = {
  id: string
  itemId: string
  path: string
  type: string
  size: number | null
  createdAt: number
}

function createStore(items: ItemRow[], assets: AssetRow[] = []) {
  return {
    items: {
      findByCollection: vi.fn().mockResolvedValue(items),
      searchByText: vi.fn().mockResolvedValue(items),
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn(),
      deleteWithCascade: vi.fn().mockResolvedValue(undefined),
    },
    assets: {
      create: vi.fn(),
      findByItem: vi.fn().mockResolvedValue(assets),
      findById: vi.fn().mockResolvedValue(assets[0] ?? null),
      deleteWithCascade: vi.fn().mockResolvedValue(undefined),
    },
  }
}

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('$lib/navigation', () => ({
  navigation: navigationRef,
}))

vi.mock('$lib/file-import', () => ({
  pickAndImportFiles: vi.fn().mockResolvedValue([]),
  importFilesFromPaths: vi
    .fn()
    .mockResolvedValue({ imported: [], rejected: [], skippedDuplicatePaths: 0 }),
  getAssetUrl: vi.fn().mockImplementation((p: string) => `asset://localhost${p}`),
  deleteAssetFile: vi.fn().mockResolvedValue(undefined),
  generatePdfThumbnail: vi.fn().mockResolvedValue('asset://localhost/thumbnails/asset-1.png'),
  deletePdfThumbnail: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('$lib/export', () => ({
  exportCollectionById: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: vi.fn(() => ({
    onDragDropEvent: vi.fn().mockResolvedValue(vi.fn()),
  })),
}))

describe('CollectionView consumer compatibility', () => {
  beforeEach(() => {
    locale.set('es')
    vi.useFakeTimers()
    navigationRef.navigate.mockReset()
    storeRef.current = createStore([
      {
        id: 'item-1',
        title: 'Acta',
        createdAt: Date.now(),
        updatedAt: Date.now(),
        collectionId: 'col-1',
        metadata: null,
      },
    ])
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('uses SearchBar onsearch/onclear contract to call collection queries', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalledWith('col-1')
    })

    expect(screen.getByRole('heading', { name: 'Colección' })).toBeInTheDocument()
    expect(
      screen.getByText('Importá, explorá y mantené ordenados los assets de esta colección.')
    ).toBeInTheDocument()
    expect(screen.getByText('1 documento visible')).toBeInTheDocument()

    const searchInput = screen.getByRole('searchbox')
    await fireEvent.input(searchInput, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(storeRef.current.items.searchByText).toHaveBeenCalledWith('col-1', 'acta')
    })

    await fireEvent.click(screen.getByRole('button', { name: /clear search/i }))

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalledTimes(2)
    })
  })

  it('shows the empty-state guidance when there are no items', async () => {
    storeRef.current = createStore([])

    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalledWith('col-1')
    })

    expect(screen.getByText('0 documentos visibles')).toBeInTheDocument()
    expect(
      screen.getByText(
        'Todavía no hay documentos en esta colección. Importá archivos para empezar a trabajar.'
      )
    ).toBeInTheDocument()
  })

  it('updates translated collection copy when locale changes', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    expect(await screen.findByRole('heading', { name: 'Colección' })).toBeInTheDocument()

    locale.set('en')

    await waitFor(() => {
      expect(screen.getByText('1 visible document')).toBeInTheDocument()
      expect(
        screen.getByText('Import, browse, and keep this collection assets organized.')
      ).toBeInTheDocument()
    })
  })
})

describe('CollectionView asset deletion', () => {
  const sampleAsset: AssetRow = {
    id: 'asset-1',
    itemId: 'item-1',
    path: '/app-data/assets/col-1/item-1/uuid_acta.pdf',
    type: 'pdf',
    size: 1024,
    createdAt: Date.now(),
  }

  beforeEach(() => {
    locale.set('es')
    vi.useFakeTimers()
    navigationRef.navigate.mockReset()
    storeRef.current = createStore(
      [
        {
          id: 'item-1',
          title: 'Acta',
          createdAt: Date.now(),
          updatedAt: Date.now(),
          collectionId: 'col-1',
          metadata: null,
        },
      ],
      [sampleAsset]
    )
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  async function renderAndWaitForItems() {
    render(CollectionView, { collectionId: 'col-1' })

    // Wait for the async load to complete
    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalled()
    })

    // Advance timers to let the promise resolution propagate to Svelte state
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)
  }

  it('shows delete confirmation modal when delete button is clicked', async () => {
    await renderAndWaitForItems()

    // Find and click the delete button
    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    // Modal should appear with destructive dialog semantics
    const dialog = screen.getByRole('alertdialog')
    expect(dialog).toBeInTheDocument()
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleDescription(/uuid_acta\.pdf/)
    expect(screen.getByText(/¿Seguro que querés eliminar/)).toBeInTheDocument()
    expect(screen.getByText(/uuid_acta\.pdf/)).toBeInTheDocument()
  })

  it('cancels deletion when Cancel is clicked', async () => {
    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    expect(screen.getByRole('alertdialog')).toBeInTheDocument()

    const cancelBtn = screen.getByRole('button', { name: 'Cancelar' })
    await fireEvent.click(cancelBtn)

    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
  })

  it('deletes entire item when last asset is removed — card disappears from grid', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')

    await renderAndWaitForItems()

    // Verify the card is visible
    expect(screen.getByText('Acta')).toBeInTheDocument()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar asset' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(deleteAssetFile).toHaveBeenCalledWith(sampleAsset.path)
      // Last asset → entire item is deleted, not just the asset
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })

    // Card should be removed from the grid (no ghost card)
    await waitFor(() => {
      expect(screen.queryByText('Acta')).not.toBeInTheDocument()
    })

    // Modal should close after successful deletion
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
  })

  it('still removes card even when DB cleanup fails — resilient deletion', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')
    // Simulate DB failure
    storeRef.current.items.deleteWithCascade = vi.fn().mockRejectedValueOnce(new Error('DB locked'))

    await renderAndWaitForItems()

    expect(screen.getByText('Acta')).toBeInTheDocument()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar asset' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      // File was still attempted
      expect(deleteAssetFile).toHaveBeenCalledWith(sampleAsset.path)
      // DB failed but...
    })

    // Card is STILL removed — UI update is not blocked by DB error
    await waitFor(() => {
      expect(screen.queryByText('Acta')).not.toBeInTheDocument()
    })

    // Modal closes even on DB failure
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
  })

  it('closes delete confirmation with Escape and restores focus to the trigger', async () => {
    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    deleteBtn.focus()
    await fireEvent.click(deleteBtn)
    await vi.advanceTimersByTimeAsync(0)

    const dialog = screen.getByRole('alertdialog')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Cancelar' })).toHaveFocus()
    })

    await fireEvent.keyDown(dialog, { key: 'Escape' })

    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
    expect(deleteBtn).toHaveFocus()
  })

  it('does NOT call findById — uses cached path for file deletion', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')

    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar asset' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(deleteAssetFile).toHaveBeenCalled()
      // findById should NOT be called — path comes from cache
      expect(storeRef.current.assets.findById).not.toHaveBeenCalled()
    })
  })
})

describe('CollectionView PDF thumbnail', () => {
  const pdfAsset: AssetRow = {
    id: 'asset-pdf-1',
    itemId: 'item-1',
    path: '/app-data/assets/col-1/item-1/uuid_doc.pdf',
    type: 'pdf',
    size: 2048,
    createdAt: Date.now(),
  }

  beforeEach(() => {
    locale.set('es')
    vi.useFakeTimers()
    navigationRef.navigate.mockReset()
    storeRef.current = createStore(
      [
        {
          id: 'item-1',
          title: 'PDF Document',
          createdAt: Date.now(),
          updatedAt: Date.now(),
          collectionId: 'col-1',
          metadata: null,
        },
      ],
      [pdfAsset]
    )
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  async function renderAndWaitForItems() {
    render(CollectionView, { collectionId: 'col-1' })

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalled()
    })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)
  }

  it('does not generate thumbnails for PDF assets during initial exploration', async () => {
    const { generatePdfThumbnail } = await import('$lib/file-import')

    await renderAndWaitForItems()

    expect(generatePdfThumbnail).not.toHaveBeenCalled()
  })

  it('cleans up PDF thumbnail when deleting a PDF asset', async () => {
    const { deletePdfThumbnail } = await import('$lib/file-import')

    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete PDF Document' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar asset' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(deletePdfThumbnail).toHaveBeenCalledWith(pdfAsset.id)
    })
  })

  it('renders the confirm delete action as the shared trash icon button', async () => {
    await renderAndWaitForItems()

    await fireEvent.click(screen.getByRole('button', { name: 'Delete PDF Document' }))

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar asset' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    expect(confirmBtn).not.toHaveTextContent('Eliminar')
  })
})
