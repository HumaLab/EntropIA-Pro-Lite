import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import CollectionView from './CollectionView.svelte'
import { locale } from '$lib/i18n'
import {
  DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
  DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
} from '$lib/document-explorer'
import { exportCollectionById } from '$lib/export'
import { exportCollectionOcrText } from '$lib/collection-ocr-export'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
// Real module (not mocked): CollectionView.svelte's Task 3.5 pane-targeting
// guard reads this same registry via `currentPaneRects()`.
import { registerPaneRect, unregisterPaneRect } from '$lib/pane-rects'
import { normalizeGraphSettings } from '$lib/analysis-settings'

const { storeRef, navigationRef, workspaceRef, fileImportRef, dragDropRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      items: {
        // CollectionView feature-detects this method: undefined exercises the
        // findByCollection/searchByText fallback path most tests rely on.
        findCardSummariesByCollection: undefined as Mock | undefined,
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
        findByParentAssetId: vi.fn().mockResolvedValue([]),
        deleteWithCascade: vi.fn(),
      },
      extractions: {
        findTextByCollection: vi.fn(),
      },
      transcriptions: {
        findTextByCollection: vi.fn(),
      },
    },
  },
  navigationRef: {
    current: { name: 'collection', collectionName: 'Colección' } as {
      name: string
      collectionName?: string
    },
    navigate: vi.fn(),
    // A real store, as the app's NavigationStore is: a view that reads
    // `$navigation` has to hear about a change, not only see the first value.
    subscribers: new Set<(value: unknown) => void>(),
    subscribe(run: (value: unknown) => void) {
      this.subscribers.add(run)
      run({ current: this.current })
      return () => this.subscribers.delete(run)
    },
    go(view: { name: string; collectionName?: string }) {
      this.current = view
      for (const run of this.subscribers) run({ current: view })
    },
  },
  workspaceRef: {
    forgetItem: vi.fn(),
    // Task 3.5's pane-targeting guard reads `workspace.activeTabId` as the
    // fallback pane; defaults to this view's own mocked pane id (see the
    // `$lib/pane-context` mock below) so tests that never touch drag-drop
    // targeting are unaffected.
    activeTabId: 'pane-test' as string,
  },
  fileImportRef: {
    pickFiles: vi.fn(),
    classifyFiles: vi.fn(),
    generateImageThumbnail: vi.fn(),
  },
  dragDropRef: {
    onDragDropEvent: vi.fn(),
    handler: undefined as
      | ((event: {
          payload: { type: string; paths?: string[]; position?: { x: number; y: number } }
        }) => void)
      | undefined,
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
  parentAssetId?: string | null
  pageNumber?: number | null
  createdAt: number
}

function createStore(items: ItemRow[], assets: AssetRow[] = []) {
  return {
    items: {
      findCardSummariesByCollection: undefined,
      findByCollection: vi.fn().mockResolvedValue(items),
      searchByText: vi.fn().mockResolvedValue(items),
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn(),
      deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      findImportedFromSource: vi.fn().mockResolvedValue(null),
      getCollectionStats: vi.fn().mockResolvedValue({
        items: 0,
        assets: 0,
        ocr: 0,
        embeddings: 0,
        ner: 0,
        triples: 0,
        pdfPages: 0,
        images: 0,
        audios: 0,
        stt: 0,
      }),
    },
    assets: {
      create: vi.fn(),
      findByItem: vi.fn().mockResolvedValue(assets),
      findById: vi.fn().mockResolvedValue(assets[0] ?? null),
      findByParentAssetId: vi.fn().mockResolvedValue([]),
      deleteWithCascade: vi.fn().mockResolvedValue(undefined),
    },
    extractions: {
      findTextByCollection: vi.fn().mockResolvedValue([]),
    },
    transcriptions: {
      findTextByCollection: vi.fn().mockResolvedValue([]),
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

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => navigationRef,
  getPaneId: () => 'pane-test',
}))

vi.mock('$lib/workspace', () => ({
  workspace: workspaceRef,
}))

vi.mock('$lib/file-import', () => ({
  pickFiles: fileImportRef.pickFiles,
  classifyFiles: fileImportRef.classifyFiles,
  getAssetUrl: vi.fn().mockImplementation((p: string) => `asset://localhost${p}`),
  generateImageThumbnail: fileImportRef.generateImageThumbnail,
  deleteAssetFile: vi.fn().mockResolvedValue(undefined),
  deleteImageThumbnail: vi.fn().mockResolvedValue(undefined),
  generatePdfThumbnail: vi.fn().mockResolvedValue('asset://localhost/thumbnails/asset-1.png'),
  deletePdfThumbnail: vi.fn().mockResolvedValue(undefined),
  isOutsideDataDir: () => false,
  resolveStoredAssetPath: (path: string) => path,
}))

vi.mock('$lib/collection-ocr-export', () => {
  class CollectionOcrExportEmptyError extends Error {}
  return {
    CollectionOcrExportEmptyError,
    exportCollectionOcrText: vi.fn().mockResolvedValue(null),
  }
})

vi.mock('$lib/export', () => ({
  exportCollectionById: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: vi.fn(() => ({
    onDragDropEvent: dragDropRef.onDragDropEvent,
  })),
}))

beforeEach(() => {
  fileImportRef.pickFiles.mockReset()
  fileImportRef.classifyFiles.mockReset()
  fileImportRef.generateImageThumbnail.mockReset()
  fileImportRef.pickFiles.mockResolvedValue([])
  fileImportRef.classifyFiles.mockReturnValue({ classified: [], rejected: [] })
  fileImportRef.generateImageThumbnail.mockResolvedValue(
    'asset://localhost/thumbs/image-asset-1.png'
  )
  dragDropRef.handler = undefined
  dragDropRef.onDragDropEvent.mockReset()
  dragDropRef.onDragDropEvent.mockImplementation((handler) => {
    dragDropRef.handler = handler
    return Promise.resolve(vi.fn())
  })
})

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

  // Clicking another collection in the explorer reuses this view with a new
  // id. The heading has to follow the navigation, not keep the first name.
  it('retitles itself when another collection is opened in place', async () => {
    navigationRef.go({ name: 'collection', collectionName: 'Voces' })
    const { rerender } = render(CollectionView, { collectionId: 'col-1' })
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Voces')

    navigationRef.go({ name: 'collection', collectionName: 'SOIP 1963-1965' })
    await rerender({ collectionId: 'col-2' })
    await vi.advanceTimersByTimeAsync(0)

    try {
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('SOIP 1963-1965')
    } finally {
      navigationRef.go({ name: 'collection', collectionName: 'Colección' })
    }
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
      screen.getByText('Importá, explorá y mantené ordenados los documentos de esta colección.')
    ).toBeInTheDocument()

    const searchInput = screen.getByRole('searchbox')
    await fireEvent.input(searchInput, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(storeRef.current.items.searchByText).toHaveBeenCalledWith('col-1', 'acta')
    })

    const clearButton = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    expect(clearButton).toHaveAttribute('data-tooltip', 'Limpiar búsqueda')
    await fireEvent.click(clearButton)

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalledTimes(2)
    })
  })

  it('renders import and JSON export as icon-only controls without changing their actions', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    const importAction = screen.getByRole('button', { name: 'Importar documento' })
    const exportAction = screen.getByRole('button', { name: 'Exportar JSON' })

    expect(importAction).toHaveAttribute('data-tooltip', 'Importar documento')
    expect(importAction.querySelector('svg')).not.toBeNull()
    expect(importAction.textContent?.trim()).toBe('')
    expect(exportAction).toHaveAttribute('data-tooltip', 'Exportar JSON')
    expect(exportAction.querySelector('svg')).not.toBeNull()
    expect(exportAction.textContent?.trim()).toBe('')

    await fireEvent.click(importAction)
    expect(fileImportRef.pickFiles).toHaveBeenCalledOnce()

    await fireEvent.click(exportAction)
    expect(exportCollectionById).toHaveBeenCalledWith(storeRef.current, 'col-1')
  })

  it('downloads the collection text in the chosen format from the text export menu', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    const trigger = screen.getByRole('button', { name: 'Descargar el texto de la colección' })
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu')
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByRole('menu')).toBeNull()

    await fireEvent.click(trigger)
    const items = screen.getAllByRole('menuitem').map((item) => item.textContent?.trim())
    expect(items).toEqual([
      'Texto (.txt)',
      'Markdown (.md)',
      'PDF (.pdf)',
      'Word (.docx)',
      'CSV (.csv)',
    ])

    await fireEvent.click(screen.getByRole('menuitem', { name: 'CSV (.csv)' }))
    expect(exportCollectionOcrText).toHaveBeenCalledWith(storeRef.current, 'col-1', 'csv')
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('tells the user when the collection has no extracted text to export', async () => {
    const { CollectionOcrExportEmptyError } = await import('$lib/collection-ocr-export')
    vi.mocked(exportCollectionOcrText).mockRejectedValueOnce(new CollectionOcrExportEmptyError())
    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    await fireEvent.click(
      screen.getByRole('button', { name: 'Descargar el texto de la colección' })
    )
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Texto (.txt)' }))

    expect(
      await screen.findByText('Ningún documento de esta colección tiene texto extraído.')
    ).toBeInTheDocument()
  })

  it('closes the text export menu with Escape', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    await fireEvent.click(
      screen.getByRole('button', { name: 'Descargar el texto de la colección' })
    )
    expect(screen.getByRole('menu')).toBeInTheDocument()
    const callsBefore = vi.mocked(exportCollectionOcrText).mock.calls.length

    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(vi.mocked(exportCollectionOcrText).mock.calls.length).toBe(callsBefore)
  })

  it('shows the empty-state guidance when there are no items', async () => {
    storeRef.current = createStore([])

    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalledWith('col-1')
    })

    expect(
      screen.getByText(
        'Todavía no hay documentos en esta colección. Importá archivos para empezar a trabajar.'
      )
    ).toBeInTheDocument()
  })

  it('renders collection-wide stats below the subtitle', async () => {
    const rows: ItemRow[] = ['Acta', 'Mapa', 'Carta'].map((title, index) => ({
      id: `item-${index + 1}`,
      title,
      createdAt: Date.now(),
      updatedAt: Date.now(),
      collectionId: 'col-1',
      metadata: null,
    }))
    storeRef.current = {
      ...createStore(rows),
      items: {
        ...createStore(rows).items,
        getCollectionStats: vi.fn().mockResolvedValue({
          items: 3,
          assets: 16,
          ocr: 13,
          embeddings: 13,
          ner: 8,
          triples: 2,
          pdfPages: 12,
          images: 3,
          audios: 1,
          stt: 1,
        }),
      },
    } as typeof storeRef.current

    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    const metricsGroup = screen.getByText('3 documentos').closest('.collection-view__pipeline')
    expect(metricsGroup).not.toBeNull()
    const metrics = within(metricsGroup as HTMLElement)
    const expectedMetrics = [
      '3 documentos',
      '12 páginas',
      '3 imágenes',
      '1 audio',
      '13 OCR',
      '1 STT',
      '13 Embed',
      '8 NER',
      '2 Triplets',
    ]

    for (const metric of expectedMetrics) {
      expect(metrics.getByText(metric)).toHaveClass(
        'status-badge',
        'status-badge--neutral',
        'status-badge--sm'
      )
    }

    expect(
      metrics.queryByText(
        '3 items | 16 assets | 13 con OCR | 13 con Embed | 8 con NER | 2 con Triplets'
      )
    ).not.toBeInTheDocument()
  })

  it('uses card summaries without per-item asset lookups and renders cached image thumbnails', async () => {
    const { generateImageThumbnail } = await import('$lib/file-import')
    const originalAssetUrl = 'asset://localhost/app-data/assets/col-1/item-1/original.jpg'
    const thumbnailUrl = 'asset://localhost/app-data/thumbnails/image-asset-1.png'
    fileImportRef.generateImageThumbnail.mockResolvedValueOnce(thumbnailUrl)
    storeRef.current = {
      items: {
        findCardSummariesByCollection: vi.fn().mockResolvedValue([
          {
            id: 'item-1',
            title: 'Imagen grande',
            createdAt: 1,
            updatedAt: 2,
            collectionId: 'col-1',
            metadata: null,
            assetCount: 1,
            pdfPageCount: 0,
            imageCount: 1,
            audioCount: 0,
            primaryAssetId: 'asset-1',
            primaryAssetPath: '/app-data/assets/col-1/item-1/original.jpg',
            primaryAssetType: 'image',
          },
        ]),
        findByCollection: vi.fn(),
        searchByText: vi.fn(),
        create: vi.fn(),
        update: vi.fn(),
        delete: vi.fn(),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      assets: {
        create: vi.fn(),
        findByItem: vi.fn(),
        findById: vi.fn(),
        findByParentAssetId: vi.fn().mockResolvedValue([]),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      extractions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
      transcriptions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
    }

    render(CollectionView, { collectionId: 'col-1' })

    expect(await screen.findByText('Imagen grande')).toBeInTheDocument()
    // Terminology: a card is worded by its media — an image document says
    // "imagen", never the media-blind "asset" nor the PDF-only "página".
    expect(screen.getByText('1 imagen')).toBeInTheDocument()
    expect(screen.queryByText('1 asset')).not.toBeInTheDocument()
    expect(screen.queryByText('1 página')).not.toBeInTheDocument()

    await waitFor(() => {
      expect(generateImageThumbnail).toHaveBeenCalledWith(
        '/app-data/assets/col-1/item-1/original.jpg',
        'asset-1'
      )
      expect(storeRef.current.items.findCardSummariesByCollection).toHaveBeenCalledWith('col-1', '')
      expect(storeRef.current.items.findByCollection).not.toHaveBeenCalled()
      expect(storeRef.current.assets.findByItem).not.toHaveBeenCalled()
    })

    const image = await screen.findByRole('img', { name: 'Imagen grande' })
    expect(image).toHaveAttribute('src', thumbnailUrl)
    expect(image).not.toHaveAttribute('src', originalAssetUrl)
  })

  it('generates image thumbnails with limited concurrency and renders each chunk', async () => {
    const summaries = Array.from({ length: 6 }, (_, index) => {
      const itemNumber = index + 1
      return {
        id: `item-${itemNumber}`,
        title: `Imagen ${itemNumber}`,
        createdAt: itemNumber,
        updatedAt: itemNumber,
        collectionId: 'col-1',
        metadata: null,
        assetCount: 1,
        primaryAssetId: `asset-${itemNumber}`,
        primaryAssetPath: `/app-data/assets/col-1/item-${itemNumber}/original.jpg`,
        primaryAssetType: 'image',
      }
    })
    const thumbnailLoads: Array<{ assetId: string; resolve: (value: string) => void }> = []
    let activeThumbnailLoads = 0
    let maxActiveThumbnailLoads = 0
    fileImportRef.generateImageThumbnail.mockImplementation((_path: string, assetId: string) => {
      activeThumbnailLoads++
      maxActiveThumbnailLoads = Math.max(maxActiveThumbnailLoads, activeThumbnailLoads)
      const load = deferred<string>()
      thumbnailLoads.push({
        assetId,
        resolve: (value) => {
          activeThumbnailLoads--
          load.resolve(value)
        },
      })
      return load.promise
    })
    storeRef.current = {
      items: {
        findCardSummariesByCollection: vi.fn().mockResolvedValue(summaries),
        findByCollection: vi.fn(),
        searchByText: vi.fn(),
        create: vi.fn(),
        update: vi.fn(),
        delete: vi.fn(),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      assets: {
        create: vi.fn(),
        findByItem: vi.fn(),
        findById: vi.fn(),
        findByParentAssetId: vi.fn().mockResolvedValue([]),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      extractions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
      transcriptions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
    }

    render(CollectionView, { collectionId: 'col-1' })

    expect(await screen.findByText('Imagen 1')).toBeInTheDocument()

    await waitFor(() => {
      expect(fileImportRef.generateImageThumbnail).toHaveBeenCalledTimes(4)
    })
    expect(maxActiveThumbnailLoads).toBe(4)

    for (const load of thumbnailLoads.slice(0, 4)) {
      load.resolve(`asset://localhost/app-data/thumbnails/${load.assetId}.png`)
    }

    await waitFor(() => {
      expect(fileImportRef.generateImageThumbnail).toHaveBeenCalledTimes(6)
    })

    for (const load of thumbnailLoads.slice(4)) {
      load.resolve(`asset://localhost/app-data/thumbnails/${load.assetId}.png`)
    }

    const image = await screen.findByRole('img', { name: 'Imagen 6' })
    expect(image).toHaveAttribute('src', 'asset://localhost/app-data/thumbnails/asset-6.png')
  })

  it('updates translated collection copy when locale changes', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    expect(await screen.findByRole('heading', { name: 'Colección' })).toBeInTheDocument()
    const searchInput = screen.getByRole('searchbox')
    await fireEvent.input(searchInput, { target: { value: 'acta' } })
    expect(screen.getByRole('button', { name: 'Limpiar búsqueda' })).toHaveAttribute(
      'data-tooltip',
      'Limpiar búsqueda'
    )

    locale.set('en')

    await vi.advanceTimersByTimeAsync(0)
    const clearButton = screen.getByRole('button', { name: 'Clear search' })
    expect(clearButton).toHaveAttribute('data-tooltip', 'Clear search')
    const metricsGroup = screen.getByText('0 documents').closest('.collection-view__pipeline')
    expect(metricsGroup).not.toBeNull()
    const metrics = within(metricsGroup as HTMLElement)
    // A collection with no known media yet shows no media chip at all — only
    // the always-on processing chips (OCR, STT, Embed, NER, Triplets).
    for (const metric of ['0 documents', '0 OCR', '0 STT', '0 Embed', '0 NER', '0 Triplets']) {
      expect(metrics.getByText(metric)).toBeInTheDocument()
    }
    expect(metrics.queryByText('0 pages')).not.toBeInTheDocument()

    expect(
      metrics.queryByText(
        '0 items | 0 assets | 0 with OCR | 0 with Embed | 0 with NER | 0 with Triplets'
      )
    ).not.toBeInTheDocument()
  })

  it('ignores stale item loads that resolve after a newer search', async () => {
    const firstLoad = deferred<ItemRow[]>()
    const searchLoad = deferred<ItemRow[]>()
    const oldItem: ItemRow = {
      id: 'item-old',
      title: 'Acta vieja',
      createdAt: Date.now(),
      updatedAt: Date.now(),
      collectionId: 'col-1',
      metadata: null,
    }
    const newItem: ItemRow = {
      id: 'item-new',
      title: 'Acta nueva',
      createdAt: Date.now(),
      updatedAt: Date.now(),
      collectionId: 'col-1',
      metadata: null,
    }

    storeRef.current = {
      items: {
        findCardSummariesByCollection: undefined,
        findByCollection: vi.fn().mockReturnValueOnce(firstLoad.promise),
        searchByText: vi.fn().mockReturnValueOnce(searchLoad.promise),
        create: vi.fn(),
        update: vi.fn(),
        delete: vi.fn(),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      assets: {
        create: vi.fn(),
        findByItem: vi.fn().mockResolvedValue([]),
        findById: vi.fn().mockResolvedValue(null),
        findByParentAssetId: vi.fn().mockResolvedValue([]),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      extractions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
      transcriptions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
    }

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'acta' } })
    await vi.advanceTimersByTimeAsync(300)

    searchLoad.resolve([newItem])

    expect(await screen.findByText('Acta nueva')).toBeInTheDocument()

    firstLoad.resolve([oldItem])

    await waitFor(() => {
      expect(screen.getByText('Acta nueva')).toBeInTheDocument()
      expect(screen.queryByText('Acta vieja')).not.toBeInTheDocument()
    })
  })

  it('reloads and resets collection state when collectionId changes', async () => {
    storeRef.current = {
      items: {
        findCardSummariesByCollection: undefined,
        findByCollection: vi.fn().mockImplementation(async (collectionId: string) =>
          collectionId === 'col-2'
            ? [
                {
                  id: 'item-2',
                  title: 'Contrato nuevo',
                  createdAt: Date.now(),
                  updatedAt: Date.now(),
                  collectionId: 'col-2',
                  metadata: null,
                },
              ]
            : [
                {
                  id: 'item-1',
                  title: 'Acta vieja',
                  createdAt: Date.now(),
                  updatedAt: Date.now(),
                  collectionId: 'col-1',
                  metadata: null,
                },
              ]
        ),
        searchByText: vi.fn().mockResolvedValue([
          {
            id: 'item-1',
            title: 'Acta vieja',
            createdAt: Date.now(),
            updatedAt: Date.now(),
            collectionId: 'col-1',
            metadata: null,
          },
        ]),
        create: vi.fn(),
        update: vi.fn(),
        delete: vi.fn(),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      assets: {
        create: vi.fn(),
        findByItem: vi.fn().mockResolvedValue([]),
        findById: vi.fn().mockResolvedValue(null),
        findByParentAssetId: vi.fn().mockResolvedValue([]),
        deleteWithCascade: vi.fn().mockResolvedValue(undefined),
      },
      extractions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
      transcriptions: {
        findTextByCollection: vi.fn().mockResolvedValue([]),
      },
    }

    const { rerender } = render(CollectionView, { collectionId: 'col-1' })

    expect(await screen.findByText('Acta vieja')).toBeInTheDocument()

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'acta' } })
    await vi.advanceTimersByTimeAsync(300)

    await waitFor(() => {
      expect(storeRef.current.items.searchByText).toHaveBeenCalledWith('col-1', 'acta')
    })

    await rerender({ collectionId: 'col-2' })

    expect(await screen.findByText('Contrato nuevo')).toBeInTheDocument()

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalledWith('col-2')
    })
    expect(storeRef.current.items.searchByText).not.toHaveBeenCalledWith('col-2', 'acta')
    expect(screen.queryByText('Acta vieja')).not.toBeInTheDocument()
  })
})

describe('CollectionView import flow', () => {
  beforeEach(() => {
    locale.set('es')
    vi.useFakeTimers()
    navigationRef.navigate.mockReset()
    navigationRef.current = { name: 'collection', collectionName: 'Colección' }
    storeRef.current = createStore([])
  })

  afterEach(() => {
    vi.useRealTimers()
    // Back to the answer the shared setup gives every other test.
    vi.mocked(invoke).mockReset()
    vi.mocked(invoke).mockImplementation(async (command: string) =>
      command === 'resolve_data_dir' ? '/mock/app-data' : undefined
    )
  })

  /**
   * The import command as the view sees it: one request per classified file.
   * `answers` says what one path's request does — an import (the default), a
   * duplicate, or a failure carrying that message. The copy, the PDF split,
   * the item and its assets are the backend's business now
   * (`odd/tasks/agent-corpus-tools.md` D10); the view only reads the answer.
   */
  function answerImports(answers: Record<string, 'duplicate' | string> = {}) {
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === 'resolve_data_dir') return '/mock/app-data'
      if (command !== 'corpus_import_path') return undefined
      const sourcePath = String(args?.sourcePath)
      const answer = answers[sourcePath] ?? 'imported'
      if (answer === 'duplicate') return { outcome: 'duplicate', itemId: 'item-old' }
      if (answer !== 'imported') throw answer
      const name = sourcePath.split(/[/\\]/).pop() ?? sourcePath
      return {
        outcome: 'imported',
        itemId: 'item-new',
        title: name.replace(/\.[^.]+$/, ''),
        assets: 1,
      }
    }) as never)
  }

  /** One classified file, as `classifyFiles` would report it. */
  function classifyOne(sourcePath: string, name: string, type: 'image' | 'pdf' = 'image') {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath, name, type }],
      rejected: [],
    })
  }

  function importRequests(): unknown[][] {
    return vi
      .mocked(invoke)
      .mock.calls.filter(([command]) => command === 'corpus_import_path')
      .map(([, args]) => [args])
  }

  it('imports picker-selected paths through the backend and opens the item it created', async () => {
    const sourcePath = 'C:\\tmp\\photo.png'
    const explorerRefreshes: CustomEvent[] = []
    const handleExplorerRefresh = (event: Event) => {
      explorerRefreshes.push(event as CustomEvent)
    }
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handleExplorerRefresh)
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'photo.png')
    answerImports()

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(fileImportRef.classifyFiles).toHaveBeenCalledWith([sourcePath])
      expect(invoke).toHaveBeenCalledWith('corpus_import_path', {
        collectionId: 'col-1',
        sourcePath,
        overrides: null,
      })
      expect(navigationRef.navigate).toHaveBeenCalledWith({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección',
        itemId: 'item-new',
        itemTitle: 'photo',
      })
      expect(explorerRefreshes.at(-1)?.detail).toEqual({ collectionId: 'col-1' })
    })

    window.removeEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handleExplorerRefresh)

    expect(screen.getByRole('region', { name: 'Resumen de importación' })).toBeInTheDocument()
    expect(screen.getByText('Abrimos el último documento importado: photo.')).toBeInTheDocument()
    expect(screen.getByText('Importados')).toBeInTheDocument()
    expect(screen.getByText('Omitidos')).toBeInTheDocument()
    expect(screen.getByText('Errores')).toBeInTheDocument()
  })

  it('imports a PDF through the backend and opens the document it created', async () => {
    // The original plus one single-page asset per page is pinned in Rust
    // (`corpus::ingest_tests`); the view only reads the answer.
    const sourcePath = 'C:\\tmp\\doc.pdf'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'doc.pdf', 'pdf')
    answerImports()

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('corpus_import_path', {
        collectionId: 'col-1',
        sourcePath,
        overrides: null,
      })
      expect(navigationRef.navigate).toHaveBeenCalledWith(
        expect.objectContaining({ name: 'item', itemId: 'item-new', itemTitle: 'doc' })
      )
    })
  })

  it('reports a failed import, cleans nothing itself and stays in the collection', async () => {
    // A PDF that cannot be split fails inside the service, which also removes
    // the item and its folder it had already written
    // (`corpus::ingest_tests::a_failed_import_leaves_no_item_and_no_files`).
    const sourcePath = 'C:\\tmp\\doc.pdf'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'doc.pdf', 'pdf')
    answerImports({
      [sourcePath]: 'invalid_file: doc.pdf could not be split into pages: PDF split failed',
    })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(
        screen.getByText(/importing doc\.pdf.*doc\.pdf could not be split into pages/)
      ).toBeInTheDocument()
    })
    expect(screen.getAllByText(/PDF split failed/)).toHaveLength(2)
    expect(navigationRef.navigate).not.toHaveBeenCalled()
    // The renderer no longer deletes anything on a failed import.
    expect(storeRef.current.items.delete).not.toHaveBeenCalled()
    expect(storeRef.current.assets.deleteWithCascade).not.toHaveBeenCalled()
  })

  it('says the collection is gone and refreshes the sidebar when the backend no longer has it', async () => {
    // A collection deleted on this device or applied from a synced one leaves
    // this view holding an id the backend no longer knows.
    const sourcePath = 'C:\\tmp\\foto.png'
    const collectionsRefreshes: CustomEvent[] = []
    const handleCollectionsRefresh = (event: Event) => {
      collectionsRefreshes.push(event as CustomEvent)
    }
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT, handleCollectionsRefresh)
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'foto.png')
    answerImports({ [sourcePath]: 'not_found: no collection col-1' })

    try {
      render(CollectionView, { collectionId: 'col-1' })

      await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

      await waitFor(() => {
        expect(
          screen.getByText(
            'Esta colección ya no existe (se borró en este equipo o en otro sincronizado). Elegí o creá otra colección.'
          )
        ).toBeInTheDocument()
      })
      // Detected by the backend's code, so the raw sentence never reaches the
      // banner.
      expect(screen.queryByText(/no collection col-1/)).not.toBeInTheDocument()
      expect(collectionsRefreshes).toHaveLength(1)
      expect(navigationRef.navigate).not.toHaveBeenCalled()
    } finally {
      window.removeEventListener(
        DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
        handleCollectionsRefresh
      )
    }
  })

  it('says the collection is gone in English too', async () => {
    locale.set('en')
    const sourcePath = 'C:\\tmp\\foto.png'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'foto.png')
    answerImports({ [sourcePath]: 'not_found: no collection col-1' })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Import document/ }))

    await waitFor(() => {
      expect(
        screen.getByText(
          'This collection no longer exists (it was deleted on this or a synced device). Choose or create another collection.'
        )
      ).toBeInTheDocument()
    })
  })

  it('keeps the backend sentence for every other import failure', async () => {
    const sourcePath = 'C:\\tmp\\doc.pdf'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'doc.pdf', 'pdf')
    answerImports({ [sourcePath]: 'not_found: no item item-x' })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(screen.getAllByText(/no item item-x/).length).toBeGreaterThan(0)
    })
    expect(screen.queryByText(/ya no existe/)).not.toBeInTheDocument()
  })

  it('skips a file already imported into this collection', async () => {
    const sourcePath = 'C:\\tmp\\doc.pdf'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'doc.pdf', 'pdf')
    answerImports({ [sourcePath]: 'duplicate' })

    render(CollectionView, { collectionId: 'col-1' })
    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(
        screen.getByText(/Ya estaban importados en esta colección: doc\.pdf/)
      ).toBeInTheDocument()
    })
    expect(navigationRef.navigate).not.toHaveBeenCalled()
  })

  it('does not call a duplicate-only import a failure', async () => {
    const sourcePath = 'C:\\tmp\\doc.pdf'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'doc.pdf', 'pdf')
    answerImports({ [sourcePath]: 'duplicate' })

    render(CollectionView, { collectionId: 'col-1' })
    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(
        screen.getByText(
          /Algunos archivos ya estaban en esta colección y no se volvieron a importar/
        )
      ).toBeInTheDocument()
    })
    expect(screen.queryByText(/no se pudieron importar/)).not.toBeInTheDocument()
  })

  // Before D10 the renderer ran the duplicate check itself and imported a file
  // whose check had failed. The check now runs inside the import's transaction,
  // so a check that cannot run is that file's error and nothing is imported.
  it('reports a duplicate check that could not run as this file’s failure', async () => {
    const sourcePath = 'C:\\tmp\\doc.pdf'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    classifyOne(sourcePath, 'doc.pdf', 'pdf')
    answerImports({
      [sourcePath]: 'unavailable: could not check for an earlier import: database is locked',
    })

    render(CollectionView, { collectionId: 'col-1' })
    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(
        screen.getByText(/could not check for an earlier import: database is locked/)
      ).toBeInTheDocument()
    })
    expect(navigationRef.navigate).not.toHaveBeenCalled()
  })

  it('shows import progress while the picker is pending', async () => {
    const pendingPicker = deferred<string[]>()
    fileImportRef.pickFiles.mockReturnValueOnce(pendingPicker.promise)

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    expect(screen.getByRole('region', { name: 'Resumen de importación' })).toBeInTheDocument()
    expect(screen.getByText('Importando archivos')).toBeInTheDocument()
    expect(screen.getByText('Estamos copiando archivos y creando documentos.')).toBeInTheDocument()

    pendingPicker.resolve([])
    await waitFor(() => {
      expect(screen.queryByText('Importando archivos')).not.toBeInTheDocument()
    })
  })

  it('updates native progress per completed source file and keeps successful batch summaries visible', async () => {
    const firstPath = 'C:\\tmp\\first.png'
    const secondPath = 'C:\\tmp\\second.png'
    const firstImport = deferred<unknown>()
    const secondImport = deferred<unknown>()
    fileImportRef.pickFiles.mockResolvedValue([firstPath, secondPath])
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [
        { sourcePath: firstPath, name: 'first.png', type: 'image' },
        { sourcePath: secondPath, name: 'second.png', type: 'image' },
      ],
      rejected: [],
    })
    vi.mocked(invoke).mockImplementation(((command: string, args?: Record<string, unknown>) => {
      if (command === 'resolve_data_dir') return Promise.resolve('/mock/app-data')
      if (command !== 'corpus_import_path') return Promise.resolve(undefined)
      return String(args?.sourcePath) === firstPath ? firstImport.promise : secondImport.promise
    }) as never)

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(importRequests()).toEqual([
        [{ collectionId: 'col-1', sourcePath: firstPath, overrides: null }],
      ])
    })

    const progress = screen.getByRole('progressbar', { name: 'Progreso de la importación' })
    expect(progress).toHaveAttribute('value', '0')
    expect(progress).toHaveAttribute('max', '2')
    expect(progress).toHaveAttribute(
      'aria-describedby',
      'collection-import-progress-description-pane-test'
    )
    expect(screen.getByText('Archivo actual: first.png.')).toBeInTheDocument()

    firstImport.resolve({ outcome: 'imported', itemId: 'item-1', title: 'first', assets: 1 })

    await waitFor(() => {
      expect(importRequests()).toHaveLength(2)
      expect(progress).toHaveAttribute('value', '1')
    })
    expect(screen.getByText('Archivo actual: second.png.')).toBeInTheDocument()
    expect(
      document.getElementById('collection-import-progress-description-pane-test')
    ).toHaveTextContent('1 de 2 archivos procesados.')

    secondImport.resolve({ outcome: 'imported', itemId: 'item-2', title: 'second', assets: 1 })

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Cerrar resumen' })).toBeInTheDocument()
    })

    expect(navigationRef.navigate).not.toHaveBeenCalled()
    expect(screen.getByRole('region', { name: 'Resumen de importación' })).toBeInTheDocument()
    expect(
      screen.queryByRole('progressbar', { name: 'Progreso de la importación' })
    ).not.toBeInTheDocument()
  })

  it('summarizes skipped unsupported files without asking the backend', async () => {
    const sourcePath = 'C:\\tmp\\notes.exe'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    fileImportRef.classifyFiles.mockReturnValue({ classified: [], rejected: ['notes.exe'] })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(screen.getByRole('region', { name: 'Resumen de importación' })).toBeInTheDocument()
      expect(screen.getByText('Omitidos: notes.exe')).toBeInTheDocument()
      expect(importRequests()).toEqual([])
    })
  })

  it('stays in the collection and shows dismissible per-file errors when an import partially fails', async () => {
    const okPath = 'C:\\tmp\\photo.png'
    const brokenPath = 'C:\\tmp\\broken.png'
    fileImportRef.pickFiles.mockResolvedValue([okPath, brokenPath])
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [
        { sourcePath: okPath, name: 'photo.png', type: 'image' },
        { sourcePath: brokenPath, name: 'broken.png', type: 'image' },
      ],
      rejected: [],
    })
    answerImports({ [brokenPath]: 'unavailable: could not copy broken.png: disk full' })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(screen.getByText(/importing broken\.png.*disk full/)).toBeInTheDocument()
    })

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Cerrar resumen' })).toBeInTheDocument()
    })

    const summary = screen.getByRole('region', { name: 'Resumen de importación' })
    expect(summary.querySelectorAll('.import-summary__counts')).toHaveLength(1)
    expect(screen.queryByText('Fallidos')).not.toBeInTheDocument()
    expect(screen.getByText('Errores').parentElement).toHaveTextContent('1')
    expect(summary).not.toHaveAttribute('aria-live')

    // Both files were attempted; nothing is deleted by the renderer.
    expect(importRequests()).toHaveLength(2)
    expect(storeRef.current.items.delete).not.toHaveBeenCalled()

    // Partial failure → no auto-navigation, summary stays visible.
    expect(navigationRef.navigate).not.toHaveBeenCalled()
    expect(
      screen.getByText(
        'Algunos archivos no se pudieron importar. Revisá el detalle antes de continuar.'
      )
    ).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar resumen' }))

    await waitFor(() => {
      expect(
        screen.queryByRole('region', { name: 'Resumen de importación' })
      ).not.toBeInTheDocument()
    })
  })

  it('keeps importing remaining files and lists every per-file error', async () => {
    fileImportRef.pickFiles.mockResolvedValue([
      'C:\\tmp\\a.png',
      'C:\\tmp\\b.png',
      'C:\\tmp\\c.png',
    ])
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [
        { sourcePath: 'C:\\tmp\\a.png', name: 'a.png', type: 'image' },
        { sourcePath: 'C:\\tmp\\b.png', name: 'b.png', type: 'image' },
        { sourcePath: 'C:\\tmp\\c.png', name: 'c.png', type: 'image' },
      ],
      rejected: [],
    })
    answerImports({
      'C:\\tmp\\a.png': 'unavailable: could not copy a.png: copy failed a',
      'C:\\tmp\\b.png': 'unavailable: could not copy b.png: copy failed b',
    })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(screen.getByText(/importing a\.png.*copy failed a/)).toBeInTheDocument()
      expect(screen.getByText(/importing b\.png.*copy failed b/)).toBeInTheDocument()
    })

    // The third file still imported despite the earlier failures.
    expect(importRequests()).toHaveLength(3)
    expect(navigationRef.navigate).not.toHaveBeenCalled()
  })

  it('imports dropped paths through the same backend workflow', async () => {
    const sourcePath = 'C:\\tmp\\photo.png'
    classifyOne(sourcePath, 'photo.png')
    answerImports()

    render(CollectionView, { collectionId: 'col-1' })

    await waitFor(() => {
      expect(dragDropRef.handler).toBeDefined()
    })

    dragDropRef.handler?.({ payload: { type: 'drop', paths: [sourcePath] } })

    await waitFor(() => {
      expect(fileImportRef.classifyFiles).toHaveBeenCalledWith([sourcePath])
      expect(invoke).toHaveBeenCalledWith('corpus_import_path', {
        collectionId: 'col-1',
        sourcePath,
        overrides: null,
      })
      expect(navigationRef.navigate).toHaveBeenCalledWith({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección',
        itemId: 'item-new',
        itemTitle: 'photo',
      })
    })
  })

  // `onDragDropEvent` resolves only after several IPC round-trips, and the
  // Rust side delivers events as soon as the first one lands. A view torn
  // down before that promise settled used to store the unlisten on a dead
  // component and never call it: its drop handler stayed live for the rest
  // of the session and imported every later drop once more (drop-dup
  // diagnosis, defect 2).
  describe('a subscription that settles after the view is gone', () => {
    it('releases a drag-drop listener that resolves after unmount, and never imports through it', async () => {
      const sourcePath = 'C:\\tmp\\photo.png'
      classifyOne(sourcePath, 'photo.png')
      answerImports()
      let resolveListener: ((unlisten: () => void) => void) | undefined
      dragDropRef.onDragDropEvent.mockImplementation((handler) => {
        dragDropRef.handler = handler
        return new Promise<() => void>((resolve) => {
          resolveListener = resolve
        })
      })

      const { unmount } = render(CollectionView, { collectionId: 'col-1' })
      await vi.advanceTimersByTimeAsync(0)
      expect(resolveListener).toBeDefined()
      unmount()

      const unlisten = vi.fn()
      resolveListener!(unlisten)
      await vi.advanceTimersByTimeAsync(0)
      expect(unlisten).toHaveBeenCalledOnce()

      // Tauri can still deliver an event it already registered before the
      // unlisten lands: the dead view's handler must ignore it.
      dragDropRef.handler?.({ payload: { type: 'drop', paths: [sourcePath] } })
      await vi.advanceTimersByTimeAsync(0)
      expect(fileImportRef.classifyFiles).not.toHaveBeenCalled()
      expect(importRequests()).toEqual([])
    })

    it('releases event listeners that resolve after unmount', async () => {
      const pending: Array<(unlisten: () => void) => void> = []
      vi.mocked(listen).mockImplementation(
        () =>
          new Promise((resolve) => {
            pending.push(resolve as (unlisten: () => void) => void)
          })
      )

      try {
        const { unmount } = render(CollectionView, { collectionId: 'col-1' })
        await vi.advanceTimersByTimeAsync(0)
        expect(pending.length).toBeGreaterThan(0)
        unmount()

        const unlistens = pending.map((resolve) => {
          const unlisten = vi.fn()
          resolve(unlisten)
          return unlisten
        })
        await vi.advanceTimersByTimeAsync(0)
        for (const unlisten of unlistens) expect(unlisten).toHaveBeenCalledOnce()
      } finally {
        vi.mocked(listen).mockImplementation(() => Promise.resolve(vi.fn()))
      }
    })

    // A batch fires one pipeline event per processed page, and each stats
    // load scans the whole collection: reloading per event stalled every
    // other query on a large archive.
    it('coalesces a burst of pipeline events into one delayed stats refresh', async () => {
      const handlers = new Map<string, () => void>()
      vi.mocked(listen).mockImplementation((name, handler) => {
        handlers.set(name as string, handler as () => void)
        return Promise.resolve(vi.fn())
      })

      try {
        render(CollectionView, { collectionId: 'col-1' })
        await vi.advanceTimersByTimeAsync(0)
        const stats = (storeRef.current.items as unknown as { getCollectionStats: Mock })
          .getCollectionStats
        const before = stats.mock.calls.length

        for (let page = 0; page < 20; page++) handlers.get('ocr:complete')?.()
        await vi.advanceTimersByTimeAsync(1999)
        expect(stats.mock.calls.length).toBe(before)
        await vi.advanceTimersByTimeAsync(1)
        expect(stats.mock.calls.length).toBe(before + 1)
      } finally {
        vi.mocked(listen).mockImplementation(() => Promise.resolve(vi.fn()))
      }
    })
  })

  // Task 3.5: Tauri's drag-drop event is webview-wide, so every mounted
  // CollectionView instance receives every drop. These three tests drive
  // the real `$lib/pane-rects` registry (imported unmocked above) around
  // this single rendered instance — registering a rect for a pane other
  // than this one, this pane's own rect, and no rect at all — to prove the
  // guard added in CollectionView.svelte actually gates on
  // `resolveDropPaneId`'s decision, the same way a genuine second pane
  // would, without needing to mount two real instances against this file's
  // single-handler `getCurrentWebview` mock.
  describe('pane-targeting (Task 3.5)', () => {
    afterEach(() => {
      workspaceRef.activeTabId = 'pane-test'
    })

    it("ignores a drop whose position falls inside another pane's rect, not this one", async () => {
      const sourcePath = 'C:\\tmp\\photo.png'
      classifyOne(sourcePath, 'photo.png')
      answerImports()
      workspaceRef.activeTabId = 'other-pane'
      registerPaneRect('other-pane', () => ({ left: 500, top: 0, right: 1000, bottom: 400 }))

      try {
        render(CollectionView, { collectionId: 'col-1' })
        await waitFor(() => {
          expect(dragDropRef.handler).toBeDefined()
        })

        dragDropRef.handler?.({
          payload: { type: 'drop', paths: [sourcePath], position: { x: 700, y: 100 } },
        })

        // Give any (wrongly) started import a tick to run before asserting
        // it never did. This describe block runs under `vi.useFakeTimers()`
        // (see its own `beforeEach` above), so a real `setTimeout` would
        // never fire — advance the fake clock instead.
        await vi.advanceTimersByTimeAsync(0)
        expect(importRequests()).toEqual([])
      } finally {
        unregisterPaneRect('other-pane')
      }
    })

    it("imports a drop whose position falls inside this pane's own registered rect", async () => {
      const sourcePath = 'C:\\tmp\\photo.png'
      classifyOne(sourcePath, 'photo.png')
      answerImports()
      workspaceRef.activeTabId = 'other-pane'
      registerPaneRect('pane-test', () => ({ left: 0, top: 0, right: 500, bottom: 400 }))

      try {
        render(CollectionView, { collectionId: 'col-1' })
        await waitFor(() => {
          expect(dragDropRef.handler).toBeDefined()
        })

        dragDropRef.handler?.({
          payload: { type: 'drop', paths: [sourcePath], position: { x: 100, y: 100 } },
        })

        await waitFor(() => {
          expect(importRequests()).toEqual([
            [{ collectionId: 'col-1', sourcePath, overrides: null }],
          ])
        })
      } finally {
        unregisterPaneRect('pane-test')
      }
    })

    it('imports a drop whose position is outside every registered rect, falling back to the active pane', async () => {
      const sourcePath = 'C:\\tmp\\photo.png'
      classifyOne(sourcePath, 'photo.png')
      answerImports()
      workspaceRef.activeTabId = 'pane-test'

      render(CollectionView, { collectionId: 'col-1' })
      await waitFor(() => {
        expect(dragDropRef.handler).toBeDefined()
      })

      dragDropRef.handler?.({
        payload: { type: 'drop', paths: [sourcePath], position: { x: 5000, y: 5000 } },
      })

      await waitFor(() => {
        expect(importRequests()).toEqual([[{ collectionId: 'col-1', sourcePath, overrides: null }]])
      })
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

    // Modal should appear
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(screen.getByText(/¿Seguro que querés eliminar/)).toBeInTheDocument()
    expect(screen.getByText(/documento Acta/)).toBeInTheDocument()
  })

  it('cancels deletion when Cancel is clicked', async () => {
    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    expect(screen.getByRole('dialog')).toBeInTheDocument()

    const cancelBtn = screen.getByRole('button', { name: 'Cancelar' })
    await fireEvent.click(cancelBtn)

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  it('deletes the whole item through the routed backend — card disappears from grid', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')
    vi.mocked(deleteAssetFile).mockClear()
    const explorerRefreshes: CustomEvent[] = []
    const handleExplorerRefresh = (event: Event) => {
      explorerRefreshes.push(event as CustomEvent)
    }
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handleExplorerRefresh)

    await renderAndWaitForItems()

    // Verify the card is visible
    expect(screen.getByText('Acta')).toBeInTheDocument()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar documento' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      // The routed store call is Rust's corpus_items_delete: it owns the rows
      // and the files, so the renderer must not delete the file first.
      expect(deleteAssetFile).not.toHaveBeenCalled()
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
      expect(explorerRefreshes.at(-1)?.detail).toEqual({ collectionId: 'col-1', itemId: 'item-1' })
    })

    window.removeEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handleExplorerRefresh)

    // Card should be removed from the grid (no ghost card)
    await waitFor(() => {
      expect(screen.queryByText('Acta')).not.toBeInTheDocument()
    })

    // Modal should close after successful deletion
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  async function confirmDeletingActa() {
    await renderAndWaitForItems()
    await fireEvent.click(screen.getByRole('button', { name: 'Delete Acta' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar documento' }))
  }

  it('leaves the item folder and thumbnails to the backend', async () => {
    const { remove } = await import('@tauri-apps/plugin-fs')
    const { deleteAssetFile, deleteImageThumbnail, deletePdfThumbnail } =
      await import('$lib/file-import')
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(remove).mockClear()
    vi.mocked(deleteAssetFile).mockClear()
    vi.mocked(deleteImageThumbnail).mockClear()
    vi.mocked(deletePdfThumbnail).mockClear()
    vi.mocked(invoke).mockClear()

    await confirmDeletingActa()

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)
    expect(remove).not.toHaveBeenCalled()
    expect(deleteAssetFile).not.toHaveBeenCalled()
    expect(deleteImageThumbnail).not.toHaveBeenCalled()
    expect(deletePdfThumbnail).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalledWith('delete_asset_files', expect.anything())
  })

  it('does not attempt renderer file cleanup when the backend delete fails', async () => {
    const { remove } = await import('@tauri-apps/plugin-fs')
    const { deleteAssetFile } = await import('$lib/file-import')
    vi.mocked(remove).mockClear()
    vi.mocked(deleteAssetFile).mockClear()
    storeRef.current.items.deleteWithCascade = vi.fn().mockRejectedValueOnce(new Error('DB locked'))

    await confirmDeletingActa()

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
      expect(screen.getByText(/DB locked/)).toBeInTheDocument()
    })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)
    expect(remove).not.toHaveBeenCalled()
    expect(deleteAssetFile).not.toHaveBeenCalled()
  })

  /**
   * Regression: `back()` could land on a deleted document's own screen —
   * whatever page/asset it had open — once it no longer existed. Only a
   * confirmed DB cascade prunes history; a failed one leaves it alone,
   * matching that it also leaves the item's folder alone.
   */
  it('prunes history for the item once the DB cascade succeeds', async () => {
    workspaceRef.forgetItem.mockClear()

    await confirmDeletingActa()

    await waitFor(() => {
      expect(workspaceRef.forgetItem).toHaveBeenCalledWith('item-1')
    })
  })

  it('does not prune history when the database delete fails', async () => {
    workspaceRef.forgetItem.mockClear()
    storeRef.current.items.deleteWithCascade = vi.fn().mockRejectedValueOnce(new Error('DB locked'))

    await confirmDeletingActa()

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)
    expect(workspaceRef.forgetItem).not.toHaveBeenCalled()
  })

  it('keeps the dialog and warning visible when DB cleanup fails', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')
    // Simulate DB failure
    storeRef.current.items.deleteWithCascade = vi.fn().mockRejectedValueOnce(new Error('DB locked'))

    await renderAndWaitForItems()

    expect(screen.getByText('Acta')).toBeInTheDocument()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar documento' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      // The backend owns the files as well as the rows; a failed delete leaves
      // both alone.
      expect(deleteAssetFile).not.toHaveBeenCalled()
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })

    // Card stays visible because DB cleanup is the authoritative state.
    await waitFor(() => {
      expect(screen.getByText('Acta')).toBeInTheDocument()
    })

    // Modal stays open and explains that nothing was removed.
    await waitFor(() => {
      expect(screen.getByRole('dialog')).toBeInTheDocument()
      expect(screen.getByText(/No se pudo eliminar el documento/)).toBeInTheDocument()
      expect(screen.getByText(/DB locked/)).toBeInTheDocument()
    })
  })

  it('does not read asset paths before deleting — the backend owns them', async () => {
    const { deleteAssetFile, deleteImageThumbnail, deletePdfThumbnail } =
      await import('$lib/file-import')
    vi.mocked(deleteAssetFile).mockClear()
    vi.mocked(deleteImageThumbnail).mockClear()
    vi.mocked(deletePdfThumbnail).mockClear()

    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar documento' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })
    // The routed command reads the asset rows itself; the renderer never asks
    // for one by id nor removes a file or thumbnail.
    expect(storeRef.current.assets.findById).not.toHaveBeenCalled()
    expect(deleteAssetFile).not.toHaveBeenCalled()
    expect(deleteImageThumbnail).not.toHaveBeenCalled()
    expect(deletePdfThumbnail).not.toHaveBeenCalled()
  })

  it('deletes an image item without the renderer asset-file route', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    const { deleteAssetFile } = await import('$lib/file-import')
    vi.mocked(invoke).mockClear()
    vi.mocked(deleteAssetFile).mockClear()

    const imageAsset: AssetRow = {
      id: 'asset-img-1',
      itemId: 'item-1',
      path: '/app-data/assets/col-1/item-1/uuid_foto_v3.png',
      type: 'image',
      size: 2048,
      createdAt: Date.now(),
    }
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
      [imageAsset]
    )

    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar documento' })
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      // The backend removes the whole item folder (versioned siblings
      // included), so the renderer must not call delete_asset_files.
      expect(invoke).not.toHaveBeenCalledWith('delete_asset_files', expect.anything())
      expect(deleteAssetFile).not.toHaveBeenCalled()
      // The routed delete is the only path.
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
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

  it('renders the PDF card from its source file, once', async () => {
    const { generatePdfThumbnail } = await import('$lib/file-import')
    vi.mocked(generatePdfThumbnail).mockClear()

    await renderAndWaitForItems()

    await waitFor(() => {
      expect(generatePdfThumbnail).toHaveBeenCalledWith(pdfAsset.path, pdfAsset.id)
    })
    expect(generatePdfThumbnail).toHaveBeenCalledTimes(1)
  })

  it('leaves PDF thumbnail cleanup to the backend', async () => {
    const { deletePdfThumbnail } = await import('$lib/file-import')
    vi.mocked(deletePdfThumbnail).mockClear()

    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete PDF Document' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar documento' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })
    expect(deletePdfThumbnail).not.toHaveBeenCalled()
  })

  it('deletes a source PDF and its generated pages through the backend without renderer cleanup', async () => {
    const { deleteAssetFile, deleteImageThumbnail } = await import('$lib/file-import')
    const { remove } = await import('@tauri-apps/plugin-fs')
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(deleteAssetFile).mockClear()
    vi.mocked(deleteImageThumbnail).mockClear()
    vi.mocked(remove).mockClear()
    vi.mocked(invoke).mockClear()
    const pageAsset: AssetRow = {
      id: 'pdfpage-asset-pdf-1-0001',
      itemId: 'item-1',
      path: '/app-data/assets/col-1/item-1/uuid_doc.pages/0001.png',
      type: 'image',
      size: 100,
      parentAssetId: pdfAsset.id,
      pageNumber: 1,
      createdAt: Date.now(),
    }
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
      [pageAsset, pdfAsset]
    )
    storeRef.current.assets.findByParentAssetId.mockResolvedValue([pageAsset])

    await renderAndWaitForItems()
    await fireEvent.click(screen.getByRole('button', { name: 'Delete PDF Document' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar documento' }))

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })
    // The backend removes the item folder (the generated .pages directory
    // included) and every thumbnail, so the renderer touches no file.
    expect(deleteAssetFile).not.toHaveBeenCalled()
    expect(deleteImageThumbnail).not.toHaveBeenCalled()
    expect(remove).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalledWith('delete_asset_files', expect.anything())
  })

  it('deletes the item card even when its PDF contains multiple page assets', async () => {
    const pageAssets: AssetRow[] = [1, 2].map((pageNumber) => ({
      id: `pdfpage-asset-pdf-1-${String(pageNumber).padStart(4, '0')}`,
      itemId: 'item-1',
      path: `/app-data/assets/col-1/item-1/page_${pageNumber}.pdf`,
      type: 'pdf',
      size: 100,
      parentAssetId: pdfAsset.id,
      pageNumber,
      createdAt: Date.now() + pageNumber,
    }))
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
      [pdfAsset, ...pageAssets]
    )
    storeRef.current.assets.findByParentAssetId.mockResolvedValue(pageAssets)

    await renderAndWaitForItems()
    await fireEvent.click(screen.getByRole('button', { name: 'Delete PDF Document' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar documento' }))

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
      expect(storeRef.current.assets.deleteWithCascade).not.toHaveBeenCalled()
    })
  })

  it('deletes an item that already has zero assets', async () => {
    storeRef.current = createStore([
      {
        id: 'item-empty',
        title: 'Empty Document',
        createdAt: Date.now(),
        updatedAt: Date.now(),
        collectionId: 'col-1',
        metadata: null,
      },
    ])

    await renderAndWaitForItems()
    await fireEvent.click(screen.getByRole('button', { name: 'Delete Empty Document' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar documento' }))

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-empty')
      expect(storeRef.current.assets.deleteWithCascade).not.toHaveBeenCalled()
      expect(screen.queryByText('Empty Document')).not.toBeInTheDocument()
    })
  })

  it('deletes the item even when a card asset lookup fails', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')
    vi.mocked(deleteAssetFile).mockClear()

    await renderAndWaitForItems()
    storeRef.current.assets.findByItem.mockRejectedValueOnce(new Error('DB locked'))
    await fireEvent.click(screen.getByRole('button', { name: 'Delete PDF Document' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar documento' }))

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
    expect(deleteAssetFile).not.toHaveBeenCalled()
  })

  it('renders the confirm delete action as the shared trash icon button', async () => {
    await renderAndWaitForItems()

    await fireEvent.click(screen.getByRole('button', { name: 'Delete PDF Document' }))

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar documento' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    expect(confirmBtn).not.toHaveTextContent('Eliminar')
  })
})

describe('CollectionView analysis panel', () => {
  beforeEach(() => {
    locale.set('es')
    localStorage.clear()
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

  it('renders the toggle but does not load the corpus while the panel is closed', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await waitFor(() => {
      expect(storeRef.current.items.findByCollection).toHaveBeenCalledWith('col-1')
    })

    expect(screen.getByRole('button', { name: 'Mostrar análisis textual' })).toBeInTheDocument()
    expect(storeRef.current.extractions.findTextByCollection).not.toHaveBeenCalled()
    expect(storeRef.current.transcriptions.findTextByCollection).not.toHaveBeenCalled()
  })

  it('loads the corpus lazily on open and renders cloud and bar chart', async () => {
    storeRef.current.extractions.findTextByCollection = vi
      .fn()
      .mockResolvedValue([
        { assetId: 'asset-1', textContent: 'fábrica fábrica huelga conserva', createdAt: 100 },
      ])
    storeRef.current.transcriptions.findTextByCollection = vi
      .fn()
      .mockResolvedValue([
        { assetId: 'asset-2', textContent: 'Hablante 1: la fábrica de conservas', createdAt: 200 },
      ])

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: 'Mostrar análisis textual' }))

    await waitFor(() => {
      expect(storeRef.current.extractions.findTextByCollection).toHaveBeenCalledWith('col-1')
      expect(storeRef.current.transcriptions.findTextByCollection).toHaveBeenCalledWith('col-1')
    })

    expect(await screen.findByText('Análisis textual')).toBeInTheDocument()
    // "fábrica" appears in the cloud span and in the bar chart x-label.
    const matches = await screen.findAllByText('fábrica')
    expect(matches.length).toBeGreaterThan(0)
    // Speaker labels never reach the frequencies.
    expect(screen.queryByText('hablante')).not.toBeInTheDocument()
    // Distinct words: fábrica, huelga, conserva, conservas → meta line.
    expect(screen.getByText('4 palabras distintas · 6 tokens')).toBeInTheDocument()

    // Closing unmounts the panel.
    await fireEvent.click(screen.getByRole('button', { name: 'Ocultar análisis textual' }))
    await waitFor(() => {
      expect(screen.queryByText('Análisis textual')).not.toBeInTheDocument()
    })
  })

  it('shows the empty state when the collection has no extracted text', async () => {
    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: 'Mostrar análisis textual' }))

    expect(
      await screen.findByText(
        'No hay texto extraído en esta colección. Ejecutá OCR o transcripción en los documentos.'
      )
    ).toBeInTheDocument()
  })

  it('shows the error state with retry when the corpus load fails', async () => {
    storeRef.current.extractions.findTextByCollection = vi
      .fn()
      .mockRejectedValueOnce(new Error('DB locked'))
      .mockResolvedValueOnce([
        { assetId: 'asset-1', textContent: 'fábrica conserva', createdAt: 100 },
      ])

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: 'Mostrar análisis textual' }))

    expect(
      await screen.findByText('No se pudo analizar el texto de la colección.')
    ).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))

    const matches = await screen.findAllByText('fábrica')
    expect(matches.length).toBeGreaterThan(0)
  })

  it('applies term count and custom stopwords reactively and persists them per collection', async () => {
    storeRef.current.extractions.findTextByCollection = vi
      .fn()
      .mockResolvedValue([
        { assetId: 'asset-1', textContent: 'fábrica fábrica huelga conserva', createdAt: 100 },
      ])

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: 'Mostrar análisis textual' }))
    expect((await screen.findAllByText('fábrica')).length).toBeGreaterThan(0)

    await fireEvent.click(screen.getByRole('tab', { name: 'Parámetros' }))

    const termInput = screen.getByLabelText('Términos en la nube')
    await fireEvent.change(termInput, { target: { value: '20' } })

    const stopwordsArea = screen.getByLabelText('Stopwords personalizadas')
    await fireEvent.input(stopwordsArea, { target: { value: 'fábrica' } })

    // Debounced recompute drops the word from cloud and bar chart alike.
    await waitFor(() => {
      expect(screen.queryAllByText('fábrica')).toHaveLength(0)
    })
    expect(screen.queryAllByText('huelga').length).toBeGreaterThan(0)

    const stored = JSON.parse(
      localStorage.getItem('entropia-collection-analysis-settings:col-1') ?? 'null'
    )
    expect(stored).toEqual({
      cloudTermCount: 20,
      customStopwords: ['fábrica'],
      graph: normalizeGraphSettings(undefined),
    })

    // Out-of-range input clamps to the configured maximum.
    await fireEvent.change(termInput, { target: { value: '999' } })
    expect((termInput as HTMLInputElement).value).toBe('100')

    // Back to the visualization: cloud title reflects the term count.
    await fireEvent.click(screen.getByRole('tab', { name: 'Visualización' }))
    expect(await screen.findByText('Top 100 palabras')).toBeInTheDocument()
  })
})
