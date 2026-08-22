import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import CollectionView from './CollectionView.svelte'
import { locale } from '$lib/i18n'
import { DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT } from '$lib/document-explorer'

const { storeRef, navigationRef, fileImportRef, dragDropRef } = vi.hoisted(() => ({
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
    current: { name: 'collection', collectionName: 'Colección' } as const,
    navigate: vi.fn(),
  },
  fileImportRef: {
    pickFiles: vi.fn(),
    classifyFiles: vi.fn(),
    importSingleFile: vi.fn(),
    splitPdfPages: vi.fn(),
    generateImageThumbnail: vi.fn(),
  },
  dragDropRef: {
    onDragDropEvent: vi.fn(),
    handler: undefined as
      | ((event: { payload: { type: string; paths?: string[] } }) => void)
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
      getCollectionStats: vi
        .fn()
        .mockResolvedValue({ items: 0, assets: 0, ocr: 0, embeddings: 0, ner: 0, triples: 0 }),
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

vi.mock('$lib/navigation', () => ({
  navigation: navigationRef,
}))

vi.mock('$lib/file-import', () => ({
  pickFiles: fileImportRef.pickFiles,
  classifyFiles: fileImportRef.classifyFiles,
  importSingleFile: fileImportRef.importSingleFile,
  splitPdfPages: fileImportRef.splitPdfPages,
  pickAndImportFiles: vi.fn().mockResolvedValue([]),
  importFilesFromPaths: vi
    .fn()
    .mockResolvedValue({ imported: [], rejected: [], skippedDuplicatePaths: 0 }),
  getAssetUrl: vi.fn().mockImplementation((p: string) => `asset://localhost${p}`),
  generateImageThumbnail: fileImportRef.generateImageThumbnail,
  deleteAssetFile: vi.fn().mockResolvedValue(undefined),
  deleteImageThumbnail: vi.fn().mockResolvedValue(undefined),
  generatePdfThumbnail: vi.fn().mockResolvedValue('asset://localhost/thumbnails/asset-1.png'),
  deletePdfThumbnail: vi.fn().mockResolvedValue(undefined),
}))

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
  fileImportRef.importSingleFile.mockReset()
  fileImportRef.splitPdfPages.mockReset()
  fileImportRef.generateImageThumbnail.mockReset()
  fileImportRef.pickFiles.mockResolvedValue([])
  fileImportRef.classifyFiles.mockReturnValue({ classified: [], rejected: [] })
  fileImportRef.splitPdfPages.mockResolvedValue([])
  fileImportRef.generateImageThumbnail.mockResolvedValue('asset://localhost/thumbs/image-asset-1.png')
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
        getCollectionStats: vi
          .fn()
          .mockResolvedValue({ items: 3, assets: 16, ocr: 13, embeddings: 13, ner: 8, triples: 2 }),
      },
    } as typeof storeRef.current

    render(CollectionView, { collectionId: 'col-1' })

    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(0)

    const metricsGroup = screen.getByText('3 items').closest('.collection-view__pipeline')
    expect(metricsGroup).not.toBeNull()
    const metrics = within(metricsGroup as HTMLElement)
    const expectedMetrics = [
      '3 items',
      '16 assets',
      '13 con OCR',
      '13 con Embed',
      '8 con NER',
      '2 con Triplets',
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

    locale.set('en')

    await vi.advanceTimersByTimeAsync(0)
    const metricsGroup = screen.getByText('0 items').closest('.collection-view__pipeline')
    expect(metricsGroup).not.toBeNull()
    const metrics = within(metricsGroup as HTMLElement)
    for (const metric of [
      '0 items',
      '0 assets',
      '0 with OCR',
      '0 with Embed',
      '0 with NER',
      '0 with Triplets',
    ]) {
      expect(metrics.getByText(metric)).toBeInTheDocument()
    }

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
    storeRef.current.items.create = vi.fn().mockResolvedValue({ id: 'item-new' })
    storeRef.current.items.update = vi.fn().mockResolvedValue(undefined)
    storeRef.current.items.delete = vi.fn().mockResolvedValue(undefined)
    storeRef.current.assets.create = vi.fn().mockResolvedValue({ id: 'asset-new' })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  function mockImageImport(sourcePath = 'C:\\tmp\\photo.png') {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath, name: 'photo.png', type: 'image' }],
      rejected: [],
    })
    fileImportRef.importSingleFile.mockResolvedValue({
      originalName: 'photo.png',
      originalPath: sourcePath,
      destPath: 'C:\\app-data\\assets\\col-1\\item-new\\photo.png',
      type: 'image',
      size: 123,
      originalMetadata: {
        originalName: 'photo.png',
        originalPath: sourcePath,
        importedAt: '2026-06-02T00:00:00.000Z',
        sizeBytes: 123,
      },
    })
  }

  function mockPdfImport(pageCount: number) {
    const sourcePath = 'C:\\tmp\\doc.pdf'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath, name: 'doc.pdf', type: 'pdf' }],
      rejected: [],
    })
    fileImportRef.importSingleFile.mockResolvedValue({
      originalName: 'doc.pdf',
      originalPath: sourcePath,
      destPath: 'C:\\app-data\\assets\\col-1\\item-new\\doc.pdf',
      type: 'pdf',
      size: 9999,
      originalMetadata: {
        originalName: 'doc.pdf',
        originalPath: sourcePath,
        importedAt: '2026-06-02T00:00:00.000Z',
        sizeBytes: 9999,
      },
    })
    fileImportRef.splitPdfPages.mockResolvedValue(
      Array.from({ length: pageCount }, (_, i) => ({
        page_number: i + 1,
        pdf_path: `C:\\app-data\\assets\\col-1\\item-new\\doc_page_${i + 1}.pdf`,
      }))
    )
  }

  it('splits a multi-page PDF into single-page PDF assets linked to a kept parent', async () => {
    mockPdfImport(3)

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(fileImportRef.splitPdfPages).toHaveBeenCalledWith(
        'C:\\app-data\\assets\\col-1\\item-new\\doc.pdf',
        expect.any(String),
        'doc'
      )
    })

    const createCalls = storeRef.current.assets.create.mock.calls
    // The original PDF is kept as the parent asset (no rasterization, no deletion).
    expect(createCalls[0]?.[0]).toMatchObject({
      itemId: 'item-new',
      path: 'C:\\app-data\\assets\\col-1\\item-new\\doc.pdf',
      type: 'pdf',
      size: 9999,
      sortIndex: 0,
    })
    // One single-page PDF child per page, ordered, linked to the parent.
    expect(createCalls).toHaveLength(4)
    expect(createCalls[1]?.[0]).toMatchObject({
      type: 'pdf',
      parentAssetId: 'asset-new',
      pageNumber: 1,
      sortIndex: 0,
    })
    expect(createCalls[2]?.[0]).toMatchObject({
      type: 'pdf',
      parentAssetId: 'asset-new',
      pageNumber: 2,
      sortIndex: 1,
    })
    expect(createCalls[3]?.[0]).toMatchObject({
      type: 'pdf',
      parentAssetId: 'asset-new',
      pageNumber: 3,
      sortIndex: 2,
    })
  })

  it('creates a page asset for a single-page PDF through the same split pipeline', async () => {
    mockPdfImport(1)

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => expect(fileImportRef.splitPdfPages).toHaveBeenCalled())
    expect(storeRef.current.assets.create).toHaveBeenCalledTimes(2)
    expect(storeRef.current.assets.create.mock.calls[0]?.[0]).toMatchObject({
      type: 'pdf',
      sortIndex: 0,
    })
    expect(storeRef.current.assets.create.mock.calls[1]?.[0]).toMatchObject({
      type: 'pdf',
      parentAssetId: 'asset-new',
      pageNumber: 1,
      sortIndex: 0,
    })
  })

  it('fails the import instead of retaining a processable PDF parent when splitting fails', async () => {
    mockPdfImport(2)
    fileImportRef.splitPdfPages.mockRejectedValueOnce(new Error('PDF split failed'))

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(storeRef.current.items.delete).toHaveBeenCalledWith('item-new')
    })
    expect(storeRef.current.assets.create).toHaveBeenCalledTimes(1)
    expect(navigationRef.navigate).not.toHaveBeenCalled()
    expect(screen.getAllByText(/PDF split failed/)).toHaveLength(2)
  })

  it('imports picker-selected paths through the shared item/asset workflow', async () => {
    const sourcePath = 'C:\\tmp\\photo.png'
    const explorerRefreshes: CustomEvent[] = []
    const handleExplorerRefresh = (event: Event) => {
      explorerRefreshes.push(event as CustomEvent)
    }
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handleExplorerRefresh)
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    mockImageImport(sourcePath)

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(fileImportRef.classifyFiles).toHaveBeenCalledWith([sourcePath])
      expect(storeRef.current.items.create).toHaveBeenCalledWith({
        title: 'photo',
        collectionId: 'col-1',
        metadata: null,
      })
      expect(fileImportRef.importSingleFile).toHaveBeenCalledWith(sourcePath, 'col-1', 'item-new')
      expect(storeRef.current.assets.create).toHaveBeenCalledWith({
        itemId: 'item-new',
        path: 'C:\\app-data\\assets\\col-1\\item-new\\photo.png',
        type: 'image',
        size: 123,
        sortIndex: 0,
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
    const firstImport = deferred<{
      originalName: string
      originalPath: string
      destPath: string
      type: 'image'
      size: number
      originalMetadata: { originalName: string; originalPath: string; importedAt: string; sizeBytes: number }
    }>()
    const secondImport = deferred<{
      originalName: string
      originalPath: string
      destPath: string
      type: 'image'
      size: number
      originalMetadata: { originalName: string; originalPath: string; importedAt: string; sizeBytes: number }
    }>()
    let createdItems = 0
    fileImportRef.pickFiles.mockResolvedValue([firstPath, secondPath])
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [
        { sourcePath: firstPath, name: 'first.png', type: 'image' },
        { sourcePath: secondPath, name: 'second.png', type: 'image' },
      ],
      rejected: [],
    })
    storeRef.current.items.create = vi.fn().mockImplementation(async () => ({
      id: `item-${++createdItems}`,
    }))
    fileImportRef.importSingleFile.mockImplementation((sourcePath: string) =>
      sourcePath === firstPath ? firstImport.promise : secondImport.promise
    )

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(fileImportRef.importSingleFile).toHaveBeenCalledWith(firstPath, 'col-1', 'item-1')
    })

    const progress = screen.getByRole('progressbar', { name: 'Progreso de la importación' })
    expect(progress).toHaveAttribute('value', '0')
    expect(progress).toHaveAttribute('max', '2')
    expect(progress).toHaveAttribute('aria-describedby', 'collection-import-progress-description')
    expect(screen.getByText('Archivo actual: first.png.')).toBeInTheDocument()

    firstImport.resolve({
      originalName: 'first.png',
      originalPath: firstPath,
      destPath: 'C:\\app-data\\assets\\col-1\\item-1\\first.png',
      type: 'image',
      size: 1,
      originalMetadata: {
        originalName: 'first.png',
        originalPath: firstPath,
        importedAt: '2026-06-02T00:00:00.000Z',
        sizeBytes: 1,
      },
    })

    await waitFor(() => {
      expect(fileImportRef.importSingleFile).toHaveBeenCalledWith(secondPath, 'col-1', 'item-2')
      expect(progress).toHaveAttribute('value', '1')
    })
    expect(screen.getByText('Archivo actual: second.png.')).toBeInTheDocument()
    expect(document.getElementById('collection-import-progress-description')).toHaveTextContent(
      '1 de 2 archivos procesados.'
    )

    secondImport.resolve({
      originalName: 'second.png',
      originalPath: secondPath,
      destPath: 'C:\\app-data\\assets\\col-1\\item-2\\second.png',
      type: 'image',
      size: 1,
      originalMetadata: {
        originalName: 'second.png',
        originalPath: secondPath,
        importedAt: '2026-06-02T00:00:00.000Z',
        sizeBytes: 1,
      },
    })

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Cerrar resumen' })).toBeInTheDocument()
    })

    expect(navigationRef.navigate).not.toHaveBeenCalled()
    expect(screen.getByRole('region', { name: 'Resumen de importación' })).toBeInTheDocument()
    expect(screen.queryByRole('progressbar', { name: 'Progreso de la importación' })).not.toBeInTheDocument()
  })

  it('summarizes skipped unsupported files without creating items', async () => {
    const sourcePath = 'C:\\tmp\\notes.exe'
    fileImportRef.pickFiles.mockResolvedValue([sourcePath])
    fileImportRef.classifyFiles.mockReturnValue({ classified: [], rejected: ['notes.exe'] })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(screen.getByRole('region', { name: 'Resumen de importación' })).toBeInTheDocument()
      expect(screen.getByText('Omitidos: notes.exe')).toBeInTheDocument()
      expect(storeRef.current.items.create).not.toHaveBeenCalled()
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
    storeRef.current.items.create = vi
      .fn()
      .mockResolvedValueOnce({ id: 'item-ok' })
      .mockResolvedValueOnce({ id: 'item-broken' })
    fileImportRef.importSingleFile.mockImplementation(async (sourcePath: string) => {
      if (sourcePath === brokenPath) throw new Error('disk full')
      return {
        originalName: 'photo.png',
        originalPath: sourcePath,
        destPath: 'C:\\app-data\\assets\\col-1\\item-ok\\photo.png',
        type: 'image',
        size: 123,
        originalMetadata: {
          originalName: 'photo.png',
          originalPath: sourcePath,
          importedAt: '2026-06-02T00:00:00.000Z',
          sizeBytes: 123,
        },
      }
    })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(
        screen.getByText(/importing broken\.png.*disk full/)
      ).toBeInTheDocument()
    })

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Cerrar resumen' })).toBeInTheDocument()
    })

    const summary = screen.getByRole('region', { name: 'Resumen de importación' })
    expect(summary.querySelectorAll('.import-summary__counts')).toHaveLength(1)
    expect(screen.queryByText('Fallidos')).not.toBeInTheDocument()
    expect(screen.getByText('Errores').parentElement).toHaveTextContent('1')
    expect(summary).not.toHaveAttribute('aria-live')

    // Both files were attempted; the failed item was cleaned up.
    expect(fileImportRef.importSingleFile).toHaveBeenCalledTimes(2)
    expect(storeRef.current.items.delete).toHaveBeenCalledWith('item-broken')

    // Partial failure → no auto-navigation, summary stays visible.
    expect(navigationRef.navigate).not.toHaveBeenCalled()
    expect(
      screen.getByText('Algunos archivos no se pudieron importar. Revisá el detalle antes de continuar.')
    ).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar resumen' }))

    await waitFor(() => {
      expect(screen.queryByRole('region', { name: 'Resumen de importación' })).not.toBeInTheDocument()
    })
  })

  it('keeps importing remaining files and lists every per-file error', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['C:\\tmp\\a.png', 'C:\\tmp\\b.png', 'C:\\tmp\\c.png'])
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [
        { sourcePath: 'C:\\tmp\\a.png', name: 'a.png', type: 'image' },
        { sourcePath: 'C:\\tmp\\b.png', name: 'b.png', type: 'image' },
        { sourcePath: 'C:\\tmp\\c.png', name: 'c.png', type: 'image' },
      ],
      rejected: [],
    })
    let createCount = 0
    storeRef.current.items.create = vi.fn().mockImplementation(async () => ({
      id: `item-${++createCount}`,
    }))
    fileImportRef.importSingleFile.mockImplementation(async (sourcePath: string) => {
      if (sourcePath.endsWith('a.png')) throw new Error('copy failed a')
      if (sourcePath.endsWith('b.png')) throw new Error('copy failed b')
      return {
        originalName: 'c.png',
        originalPath: sourcePath,
        destPath: 'C:\\app-data\\assets\\col-1\\item-3\\c.png',
        type: 'image',
        size: 1,
        originalMetadata: {
          originalName: 'c.png',
          originalPath: sourcePath,
          importedAt: '2026-06-02T00:00:00.000Z',
          sizeBytes: 1,
        },
      }
    })

    render(CollectionView, { collectionId: 'col-1' })

    await fireEvent.click(screen.getByRole('button', { name: /Importar documento/ }))

    await waitFor(() => {
      expect(screen.getByText(/importing a\.png.*copy failed a/)).toBeInTheDocument()
      expect(screen.getByText(/importing b\.png.*copy failed b/)).toBeInTheDocument()
    })

    // The third file still imported despite the earlier failures.
    expect(storeRef.current.assets.create).toHaveBeenCalledWith(
      expect.objectContaining({ itemId: 'item-3' })
    )
    expect(navigationRef.navigate).not.toHaveBeenCalled()
  })

  it('imports dropped paths through the same item/asset workflow', async () => {
    const sourcePath = 'C:\\tmp\\photo.png'
    mockImageImport(sourcePath)

    render(CollectionView, { collectionId: 'col-1' })

    await waitFor(() => {
      expect(dragDropRef.handler).toBeDefined()
    })

    dragDropRef.handler?.({ payload: { type: 'drop', paths: [sourcePath] } })

    await waitFor(() => {
      expect(fileImportRef.classifyFiles).toHaveBeenCalledWith([sourcePath])
      expect(storeRef.current.items.create).toHaveBeenCalledWith({
        title: 'photo',
        collectionId: 'col-1',
        metadata: null,
      })
      expect(fileImportRef.importSingleFile).toHaveBeenCalledWith(sourcePath, 'col-1', 'item-new')
      expect(storeRef.current.assets.create).toHaveBeenCalledWith({
        itemId: 'item-new',
        path: 'C:\\app-data\\assets\\col-1\\item-new\\photo.png',
        type: 'image',
        size: 123,
        sortIndex: 0,
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
    expect(screen.getByText(/ítem Acta/)).toBeInTheDocument()
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

  it('deletes entire item when last asset is removed — card disappears from grid', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')
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

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar ítem' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(deleteAssetFile).toHaveBeenCalledWith(sampleAsset.path)
      // Last asset → entire item is deleted, not just the asset
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

  it('keeps the dialog and warning visible when DB cleanup fails', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')
    // Simulate DB failure
    storeRef.current.items.deleteWithCascade = vi.fn().mockRejectedValueOnce(new Error('DB locked'))

    await renderAndWaitForItems()

    expect(screen.getByText('Acta')).toBeInTheDocument()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar ítem' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      // File was still attempted
      expect(deleteAssetFile).toHaveBeenCalledWith(sampleAsset.path)
      // DB failed but...
    })

    // Card stays visible because DB cleanup is the authoritative state.
    await waitFor(() => {
      expect(screen.getByText('Acta')).toBeInTheDocument()
    })

    // Modal stays open and explains the partial failure instead of pretending success.
    await waitFor(() => {
      expect(screen.getByRole('dialog')).toBeInTheDocument()
      expect(screen.getByText(/DB locked/)).toBeInTheDocument()
    })
  })

  it('does NOT call findById — uses cached path for file deletion', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')

    await renderAndWaitForItems()

    const deleteBtn = screen.getByRole('button', { name: 'Delete Acta' })
    await fireEvent.click(deleteBtn)

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar ítem' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(deleteAssetFile).toHaveBeenCalled()
      // findById should NOT be called — path comes from cache
      expect(storeRef.current.assets.findById).not.toHaveBeenCalled()
    })
  })

  it('routes image asset deletion through delete_asset_files to remove versioned siblings', async () => {
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

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar ítem' })
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      // Image files go through the backend GC so edited versions
      // (foto_v2.png, foto_v3.png, …) are deleted together.
      expect(invoke).toHaveBeenCalledWith('delete_asset_files', { assetPath: imageAsset.path })
      // The plain single-file deletion must NOT be used for images.
      expect(deleteAssetFile).not.toHaveBeenCalled()
      // DB cascade is preserved: last asset → entire item removed.
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

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar ítem' })
    expect(confirmBtn.querySelector('svg')).toBeInTheDocument()
    await fireEvent.click(confirmBtn)

    await waitFor(() => {
      expect(deletePdfThumbnail).toHaveBeenCalledWith(pdfAsset.id)
    })
  })

  it('deletes a source PDF, its generated pages, and the generated directory as one lifecycle', async () => {
    const { deleteAssetFile, deleteImageThumbnail } = await import('$lib/file-import')
    const { remove } = await import('@tauri-apps/plugin-fs')
    const { invoke } = await import('@tauri-apps/api/core')
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
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar ítem' }))

    await waitFor(() => {
      expect(deleteAssetFile).toHaveBeenCalledWith(pdfAsset.path)
      expect(invoke).toHaveBeenCalledWith('delete_asset_files', { assetPath: pageAsset.path })
      expect(deleteImageThumbnail).toHaveBeenCalledWith(pageAsset.id)
      expect(remove).toHaveBeenCalledWith(
        '/app-data/assets/col-1/item-1/uuid_doc.pages',
        { recursive: true }
      )
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
    })
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
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar ítem' }))

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
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar ítem' }))

    await waitFor(() => {
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-empty')
      expect(storeRef.current.assets.deleteWithCascade).not.toHaveBeenCalled()
      expect(screen.queryByText('Empty Document')).not.toBeInTheDocument()
    })
  })

  it('continues item deletion when the full asset cleanup lookup fails', async () => {
    const { deleteAssetFile } = await import('$lib/file-import')

    await renderAndWaitForItems()
    storeRef.current.assets.findByItem.mockRejectedValueOnce(new Error('DB locked'))
    await fireEvent.click(screen.getByRole('button', { name: 'Delete PDF Document' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar ítem' }))

    await waitFor(() => {
      expect(deleteAssetFile).toHaveBeenCalledWith(pdfAsset.path)
      expect(storeRef.current.items.deleteWithCascade).toHaveBeenCalledWith('item-1')
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  it('renders the confirm delete action as the shared trash icon button', async () => {
    await renderAndWaitForItems()

    await fireEvent.click(screen.getByRole('button', { name: 'Delete PDF Document' }))

    const confirmBtn = screen.getByRole('button', { name: 'Eliminar ítem' })
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

    expect(
      screen.getByRole('button', { name: 'Mostrar análisis textual' })
    ).toBeInTheDocument()
    expect(storeRef.current.extractions.findTextByCollection).not.toHaveBeenCalled()
    expect(storeRef.current.transcriptions.findTextByCollection).not.toHaveBeenCalled()
  })

  it('loads the corpus lazily on open and renders cloud and bar chart', async () => {
    storeRef.current.extractions.findTextByCollection = vi.fn().mockResolvedValue([
      { assetId: 'asset-1', textContent: 'fábrica fábrica huelga conserva', createdAt: 100 },
    ])
    storeRef.current.transcriptions.findTextByCollection = vi.fn().mockResolvedValue([
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
    storeRef.current.extractions.findTextByCollection = vi.fn().mockResolvedValue([
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
    expect(stored).toEqual({ cloudTermCount: 20, customStopwords: ['fábrica'] })

    // Out-of-range input clamps to the configured maximum.
    await fireEvent.change(termInput, { target: { value: '999' } })
    expect((termInput as HTMLInputElement).value).toBe('100')

    // Back to the visualization: cloud title reflects the term count.
    await fireEvent.click(screen.getByRole('tab', { name: 'Visualización' }))
    expect(await screen.findByText('Top 100 palabras')).toBeInTheDocument()
  })
})
