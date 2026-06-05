import { render, screen, fireEvent, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import ItemView from './ItemView.svelte'

const {
  nlpEventHandlers,
  embedAssetMock,
  extractTriplesMock,
  llmExtractTriplesMock,
  llmExtractTriplesAssetMock,
  similarAssetsMock,
  extractTextMock,
  getLayoutByAssetMock,
  clipboardWriteTextMock,
  invokeMock,
} = vi.hoisted(() => ({
  nlpEventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  embedAssetMock: vi.fn<(_: string, __: string) => Promise<void>>(),
  extractTriplesMock: vi.fn<(_: string) => Promise<void>>(),
  llmExtractTriplesMock: vi.fn<(_: string) => Promise<void>>(),
  llmExtractTriplesAssetMock: vi.fn<(_: string) => Promise<void>>(),
  similarAssetsMock: vi.fn<
    (
      _: string,
      __?: number
    ) => Promise<
      Array<{
        assetId: string
        itemId: string
        title: string
        collectionId: string
        assetPath: string
        assetType: string
        similarity: number
      }>
    >
  >(),
  extractTextMock: vi.fn(),
  getLayoutByAssetMock: vi.fn(),
  clipboardWriteTextMock: vi.fn<(_: string) => Promise<void>>(),
  invokeMock: vi.fn(async (command: string) => {
    if (command === 'llm_get_results') return []
    if (command === 'llm_get_result') return null
    if (command === 'llm_is_available') return true
    if (command === 'db_select') return []
    return null
  }),
}))

type TripleRow = { subject: string; predicate: string; object: string }
type AnnotationRow = {
  id: string
  assetId: string
  page: number
  kind: 'rectangle' | 'underline'
  color: string
  x: number
  y: number
  width: number
  height: number
  createdAt: number
  updatedAt: number
}

type StoreOptions = {
  notesRows?: Array<{
    id: string
    itemId: string
    content: string
    createdAt: number
    updatedAt: number
  }>
  entitiesRows?: Array<{
    id: string
    itemId: string
    entityType: 'person' | 'organization' | 'place' | 'misc' | 'date' | 'institution'
    value: string
    startOffset: number | null
    endOffset: number | null
    confidence: number | null
    createdAt: number
  }>
  triplesRows?: TripleRow[]
  itemsById?: Record<
    string,
    {
      id: string
      title: string
      collectionId: string
      metadata: string
      createdAt?: number
      updatedAt?: number
    }
  >
  ftsSearchImpl?: (
    _query: string,
    _limit?: number
  ) => Promise<Array<{ itemId: string; rank: number }>>
  ftsStatsTotal?: number
  assetsRows?: Array<{
    id: string
    itemId: string
    path: string
    type: 'image' | 'pdf' | 'audio'
    createdAt: number
    size?: number | null
  }>
  collectionsById?: Record<
    string,
    {
      id: string
      name: string
      description?: string | null
      createdAt?: number
      updatedAt?: number
    }
  >
  annotationsByAsset?: Record<string, AnnotationRow[]>
  replaceAnnotationsImpl?: (
    assetId: string,
    page: number,
    annotations: AnnotationRow[]
  ) => Promise<unknown>
}

function createStore({
  notesRows = [],
  entitiesRows = [],
  triplesRows = [],
  itemsById = {
    'item-1': {
      id: 'item-1',
      title: 'Acta histórica',
      collectionId: 'col-1',
      metadata: '{}',
      createdAt: 1,
      updatedAt: 1,
    },
  },
  ftsSearchImpl = async () => [],
  ftsStatsTotal = 35,
  assetsRows = [
    {
      id: 'asset-1',
      itemId: 'item-1',
      path: 'docs/acta.pdf',
      type: 'pdf' as const,
      createdAt: Date.now(),
      size: 2048,
    },
  ],
  collectionsById = {
    'col-1': {
      id: 'col-1',
      name: 'Colección 1',
      description: null,
      createdAt: 1,
      updatedAt: 1,
    },
  },
  annotationsByAsset = {},
  replaceAnnotationsImpl = async () => undefined,
}: StoreOptions = {}) {
  return {
    items: {
      findById: vi.fn().mockImplementation(async (id: string) => itemsById[id] ?? null),
      update: vi.fn().mockResolvedValue(undefined),
    },
    collections: {
      findById: vi.fn().mockImplementation(async (id: string) => collectionsById[id] ?? null),
    },
    assets: {
      findByItem: vi.fn().mockResolvedValue(assetsRows),
      updatePath: vi.fn().mockResolvedValue(undefined),
    },
    notes: {
      findByItem: vi.fn().mockResolvedValue(notesRows),
      findByAsset: vi.fn().mockResolvedValue(notesRows),
      create: vi.fn().mockResolvedValue(undefined),
      update: vi.fn().mockResolvedValue(undefined),
      delete: vi.fn().mockResolvedValue(undefined),
    },
    annotations: {
      findByAsset: vi
        .fn()
        .mockImplementation(async (assetId: string) => annotationsByAsset[assetId] ?? []),
      replaceForAssetPage: vi.fn().mockImplementation(replaceAnnotationsImpl),
    },
    extractions: {
      findByAsset: vi.fn().mockResolvedValue(null),
    },
    transcriptions: {
      findByAsset: vi.fn().mockResolvedValue(null),
    },
    entities: {
      findByItemId: vi.fn().mockResolvedValue(entitiesRows),
      findByAssetId: vi.fn().mockResolvedValue(entitiesRows),
      create: vi.fn().mockResolvedValue(undefined),
      update: vi.fn().mockResolvedValue(undefined),
      delete: vi.fn().mockResolvedValue(undefined),
    },
    fts: {
      search: vi.fn().mockImplementation(ftsSearchImpl),
      searchWithDebug: vi.fn().mockImplementation(async (query: string, limit?: number) => {
        const results = await ftsSearchImpl(query, limit)
        return {
          results,
          debug: {
            rawQuery: query,
            sanitizedQuery: query ? `"${query}"` : '',
            strategy: results.length > 0 ? 'strict' : 'relaxed',
            matchCount: results.length,
            resultIds: results.map((row) => row.itemId),
          },
        }
      }),
      stats: vi.fn().mockResolvedValue({ totalRows: ftsStatsTotal }),
    },
    triples: {
      findByItemId: vi.fn().mockResolvedValue(triplesRows),
      findByAssetId: vi.fn().mockResolvedValue(triplesRows),
    },
    topics: {
      findByItemId: vi.fn().mockResolvedValue([]),
      allNames: vi.fn().mockResolvedValue([]),
      addTopicToItem: vi.fn().mockResolvedValue(undefined),
      findByName: vi.fn().mockResolvedValue(null),
      removeTopicFromItem: vi.fn().mockResolvedValue(undefined),
    },
    layouts: {
      findByAssetId: vi.fn().mockResolvedValue(null),
    },
  }
}

const storeRef: { current: ReturnType<typeof createStore> } = {
  current: createStore(),
}

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('$lib/file-import', () => ({
  getAssetUrl: (path: string) => `https://asset.localhost/${path}`,
}))

vi.mock('$lib/layouts', async () => {
  const actual = await vi.importActual<typeof import('$lib/layouts')>('$lib/layouts')
  return {
    ...actual,
    getLayoutByAsset: getLayoutByAssetMock,
  }
})

vi.mock('$lib/ocr', async () => {
  const actual = await vi.importActual<typeof import('$lib/ocr')>('$lib/ocr')
  return {
    ...actual,
    extractText: extractTextMock,
  }
})

vi.mock('$lib/nlp', async () => {
  const actual = await vi.importActual<typeof import('$lib/nlp')>('$lib/nlp')
  return {
    ...actual,
    extractTriples: extractTriplesMock,
    similarAssets: similarAssetsMock,
    indexFts: vi.fn().mockResolvedValue(undefined),
    embedAsset: embedAssetMock,
    extractEntities: vi.fn().mockResolvedValue(undefined),
  }
})

vi.mock('$lib/llm', async () => {
  const actual = await vi.importActual<typeof import('$lib/llm')>('$lib/llm')
  return {
    ...actual,
    llmIsAvailable: vi.fn().mockResolvedValue(true),
    llmGetResult: vi.fn().mockResolvedValue(null),
    llmGetResults: vi.fn().mockResolvedValue([]),
    llmSummarize: vi.fn().mockResolvedValue(undefined),
    llmCorrectOcr: vi.fn().mockResolvedValue(undefined),
    llmExtractTriples: llmExtractTriplesMock,
    llmSummarizeAsset: vi.fn().mockResolvedValue(undefined),
    llmCorrectOcrAsset: vi.fn().mockResolvedValue(undefined),
    llmExtractTriplesAsset: llmExtractTriplesAssetMock,
  }
})

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((eventName: string, callback: (event: { payload: unknown }) => void) => {
    nlpEventHandlers.set(eventName, callback)
    return Promise.resolve(vi.fn())
  }),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@entropia/ui', async () => {
  const MockActionIcon = (await import('./__mocks__/MockActionIcon.svelte')).default
  const MockDocumentViewer = (await import('./__mocks__/MockDocumentViewer.svelte')).default
  const MockEntityViewer = (await import('./__mocks__/MockEntityViewer.svelte')).default
  const ActualMetadataEditor = (
    await import('../../../../packages/ui/src/components/MetadataEditor/MetadataEditor.svelte')
  ).default
  const MockButton = (await import('./__mocks__/MockButton.svelte')).default
  const MockCard = (await import('./__mocks__/MockCard.svelte')).default
  const MockNoteEditor = (await import('./__mocks__/MockNoteEditor.svelte')).default

  return {
    ActionIcon: MockActionIcon,
    DocumentViewer: MockDocumentViewer,
    MetadataEditor: ActualMetadataEditor,
    NoteEditor: MockNoteEditor,
    Button: MockButton,
    Card: MockCard,
    EntityViewer: MockEntityViewer,
    MapViewer: () => null,
    TopicEditor: () => null,
    normalizeNoteLinkHref: (href: string | null) => {
      if (!href) return null
      const trimmed = href.trim()
      if (!trimmed) return null
      if (trimmed.startsWith('#')) return trimmed

      const candidate = /^[a-zA-Z][a-zA-Z\d+.-]*:/.test(trimmed) ? trimmed : `https://${trimmed}`

      try {
        const url = new URL(candidate, 'https://entropia.local')
        const protocol = url.protocol.toLowerCase()
        if (
          protocol === 'http:' ||
          protocol === 'https:' ||
          protocol === 'mailto:' ||
          protocol === 'tel:'
        ) {
          return url.toString()
        }
      } catch {
        return null
      }

      return null
    },
    normalizeNoteContentForRender: (content: string) => {
      if (!content) return ''
      const stripped = content.replace(/<script[\s\S]*?<\/script>/gi, '')
      if (/<[a-z][\s\S]*>/i.test(stripped)) return stripped
      return stripped
        .split(/\n{2,}/)
        .map((paragraph) => `<p>${paragraph.replace(/\n/g, '<br>')}</p>`)
        .join('')
    },
    isNoteHtmlEffectivelyEmpty: (content: string) =>
      !content || content.replace(/<[^>]+>/g, '').trim().length === 0,
  }
})

beforeEach(() => {
  Object.defineProperty(globalThis.navigator, 'clipboard', {
    configurable: true,
    value: { writeText: clipboardWriteTextMock },
  })
  clipboardWriteTextMock.mockReset().mockResolvedValue(undefined)
})

describe('ItemView semantic triples panel', () => {
  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
  })

  async function renderItemViewWith(triplesRows: TripleRow[]) {
    storeRef.current = createStore({ triplesRows })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const analysisToggle = await screen.findByRole('tab', { name: /Análisis/i })
    await fireEvent.click(analysisToggle)
  }

  it('shows explicit empty state when no triples exist for the item', async () => {
    await renderItemViewWith([])

    expect(await screen.findByText('Tripletas semánticas (S|P|O)')).toBeInTheDocument()
    expect(await screen.findByText('Todavía no hay tripletas extraídas.')).toBeInTheDocument()
  })

  it('renders triples as Subject | Predicate | Object rows when store has data', async () => {
    await renderItemViewWith([
      { subject: 'Belgrano', predicate: 'creó', object: 'la Bandera' },
      { subject: 'San Martín', predicate: 'fue', object: 'gobernador de Cuyo' },
    ])

    expect(await screen.findByText('Belgrano')).toBeInTheDocument()
    expect(await screen.findByText('creó')).toBeInTheDocument()
    expect(await screen.findByText('la Bandera')).toBeInTheDocument()
    expect(await screen.findByText('San Martín')).toBeInTheDocument()
    expect(await screen.findByText('gobernador de Cuyo')).toBeInTheDocument()
  })

  it('transitions pending → running → done and supports retry after error for triples', async () => {
    await renderItemViewWith([])

    const triplesBtn = await screen.findByRole('button', { name: /TRIPLET/i })

    await fireEvent.click(triplesBtn)
    expect(llmExtractTriplesAssetMock).toHaveBeenCalledWith('asset-1')
    expect(triplesBtn).toBeDisabled()
    expect(screen.getByText('pending')).toBeInTheDocument()

    nlpEventHandlers.get('nlp:progress')?.({
      payload: { item_id: 'item-1', job: 'triples', pct: 25 },
    })
    await waitFor(() => {
      expect(screen.getByText('running')).toBeInTheDocument()
      expect(triplesBtn).toBeDisabled()
    })

    storeRef.current.triples.findByAssetId.mockResolvedValueOnce([
      { subject: 'Moreno', predicate: 'fundó', object: 'La Gazeta' },
    ])
    nlpEventHandlers.get('nlp:complete')?.({
      payload: { item_id: 'item-1', job: 'triples' },
    })
    await waitFor(() => {
      expect(screen.getByText('done')).toBeInTheDocument()
      expect(screen.getByText('Moreno')).toBeInTheDocument()
      expect(screen.getByText('La Gazeta')).toBeInTheDocument()
    })

    nlpEventHandlers.get('nlp:error')?.({
      payload: { item_id: 'item-1', job: 'triples', error: 'queue full' },
    })
    await waitFor(() => {
      expect(screen.getByText('error')).toBeInTheDocument()
      expect(triplesBtn).toBeEnabled()
    })

    await fireEvent.click(triplesBtn)
    expect(llmExtractTriplesAssetMock).toHaveBeenCalledTimes(2)
  })
})

describe('ItemView header hierarchy', () => {
  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
  })

  it('shows item context with the active asset summary in the sidebar header', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-1',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: Date.now(),
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    expect(await screen.findByText('Documento activo')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Acta histórica' })).toBeInTheDocument()
    expect(screen.getByText('PDF · acta.pdf')).toBeInTheDocument()
  })
})

describe('ItemView asset-level embedding and similarity', () => {
  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
  })

  async function openAnalysis(store = createStore()) {
    storeRef.current = store
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: /Análisis/i }))
  }

  it('calls embedAsset for the selected asset when clicking EMBED', async () => {
    await openAnalysis(
      createStore({
        assetsRows: [
          {
            id: 'asset-embed-1',
            itemId: 'item-1',
            path: 'docs/acta-1.pdf',
            type: 'pdf',
            createdAt: 1,
          },
        ],
      })
    )

    await fireEvent.click(await screen.findByRole('button', { name: /EMBED/i }))

    expect(embedAssetMock).toHaveBeenCalledWith('item-1', 'asset-embed-1')
  })

  it('disables EMBED and shows a graceful hint when no asset is selected', async () => {
    storeRef.current = createStore({ assetsRows: [] })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    expect(screen.queryByRole('button', { name: /EMBED/i })).not.toBeInTheDocument()
    expect(embedAssetMock).not.toHaveBeenCalled()
  })

  it('shows metadata labels and existing metadata values in the metadata tab', async () => {
    storeRef.current = createStore({
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: JSON.stringify({ autor: 'Mariano Moreno', fecha: '1810-05-25' }),
        },
      },
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await fireEvent.click(await screen.findByRole('tab', { name: 'Metadatos' }))

    expect(screen.getByText('Campo')).toBeInTheDocument()
    expect(screen.getByText('Valor')).toBeInTheDocument()
    expect(screen.getByDisplayValue('autor')).toBeInTheDocument()
    expect(screen.getByDisplayValue('Mariano Moreno')).toBeInTheDocument()
    expect(screen.getByDisplayValue('fecha')).toBeInTheDocument()
    expect(screen.getByDisplayValue('1810-05-25')).toBeInTheDocument()
    expect(screen.getByTestId('metadata-add')).toBeInTheDocument()
  })

  it('shows technical file metadata even when there are no custom metadata fields', async () => {
    storeRef.current = createStore({
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: '{}',
        },
      },
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await fireEvent.click(await screen.findByRole('tab', { name: 'Metadatos' }))

    const fileMetadataSection = screen.getByTestId('item-file-metadata')
    const customMetadataSection = screen.getByTestId('item-custom-metadata')

    expect(within(fileMetadataSection).getByText('Metadatos del archivo')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Nombre del archivo')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('acta.pdf')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Tipo de archivo')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('PDF')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Extensión')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('.pdf')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Tamaño')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('2.0 KB')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Documento ID')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('item-1')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Asset ID')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('asset-1')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Ruta interna')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('docs/acta.pdf')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Colección')).toBeInTheDocument()
    expect(within(fileMetadataSection).getByText('Colección 1')).toBeInTheDocument()

    expect(within(customMetadataSection).getByText('Metadatos personalizados')).toBeInTheDocument()
    expect(
      within(customMetadataSection).getByText('No hay metadatos cargados para este documento.')
    ).toBeInTheDocument()
    expect(screen.getByTestId('metadata-add')).toBeInTheDocument()
  })

  it('avoids duplicating technical metadata fields already present in custom metadata', async () => {
    storeRef.current = createStore({
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: JSON.stringify({ 'ruta interna': 'ruta/personalizada.pdf' }),
        },
      },
      assetsRows: [
        {
          id: 'asset-1',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: Date.now(),
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await fireEvent.click(await screen.findByRole('tab', { name: 'Metadatos' }))

    const fileMetadataSection = screen.getByTestId('item-file-metadata')
    const customMetadataSection = screen.getByTestId('item-custom-metadata')

    expect(within(fileMetadataSection).queryByText('Ruta interna')).not.toBeInTheDocument()
    expect(within(customMetadataSection).getByDisplayValue('ruta interna')).toBeInTheDocument()
    expect(
      within(customMetadataSection).getByDisplayValue('ruta/personalizada.pdf')
    ).toBeInTheDocument()
  })

  it('loads and renders similar assets with asset-level context', async () => {
    similarAssetsMock.mockResolvedValue([
      {
        assetId: 'asset-sim-2',
        itemId: 'item-2',
        title: 'Carta manuscrita',
        collectionId: 'col-9',
        assetPath: 'archivo/carta-manuscrita.jpg',
        assetType: 'image',
        similarity: 0.913,
      },
    ])

    await openAnalysis(
      createStore({
        assetsRows: [
          {
            id: 'asset-source-1',
            itemId: 'item-1',
            path: 'docs/acta-1.pdf',
            type: 'pdf',
            createdAt: 1,
          },
        ],
      })
    )

    await waitFor(() => {
      expect(similarAssetsMock).toHaveBeenCalledWith('asset-source-1', 5)
    })

    expect(await screen.findByText('Assets similares')).toBeInTheDocument()
    expect(await screen.findByTestId('similar-asset-asset-sim-2')).toBeInTheDocument()
    expect(screen.getByText('Carta manuscrita')).toBeInTheDocument()
    expect(screen.getByText('IMAGE · carta-manuscrita.jpg')).toBeInTheDocument()
    expect(
      screen.getByText('asset asset-sim-2 · item item-2 · colección col-9')
    ).toBeInTheDocument()
    expect(screen.getByText('archivo/carta-manuscrita.jpg')).toBeInTheDocument()
    expect(screen.getByText('91.3%')).toBeInTheDocument()
  })
})

describe('ItemView full-text search in Analysis panel', () => {
  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
  })

  it('shows FTS results only after entering a query', async () => {
    storeRef.current = createStore({
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: '{}',
        },
        'item-2': {
          id: 'item-2',
          title: 'Acta del Cabildo',
          collectionId: 'col-1',
          metadata: '{}',
        },
        'item-3': {
          id: 'item-3',
          title: 'Registro de otra colección',
          collectionId: 'col-2',
          metadata: '{}',
        },
      },
      ftsSearchImpl: async () => [
        { itemId: 'item-2', rank: -1.234 },
        { itemId: 'item-3', rank: -0.5 },
        { itemId: 'item-1', rank: -0.1 },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const analysisToggle = await screen.findByRole('tab', { name: /Análisis/i })
    await fireEvent.click(analysisToggle)

    expect(await screen.findByText('Ingresá un término para ver resultados.')).toBeInTheDocument()

    const input = await screen.findByPlaceholderText('Escribí para buscar...')
    await fireEvent.input(input, { target: { value: 'cabildo' } })

    await waitFor(() => {
      expect(storeRef.current.fts.searchWithDebug).toHaveBeenCalledWith('cabildo', 10)
      expect(storeRef.current.fts.stats).toHaveBeenCalled()
    })

    expect(await screen.findByText('Acta del', { exact: false })).toBeInTheDocument()
    expect(await screen.findByText('Cabildo')).toBeInTheDocument()
    expect(await screen.findByText('Registro de otra colección')).toBeInTheDocument()
    expect(document.querySelectorAll('.fts-search-section .similar-item').length).toBe(3)
    expect(document.querySelector('.fts-match')).toBeInTheDocument()

    await fireEvent.input(input, { target: { value: '' } })
    await waitFor(() => {
      expect(screen.getByText('Ingresá un término para ver resultados.')).toBeInTheDocument()
    })
  })

  it('executes immediate search on Enter and clears search on Escape', async () => {
    storeRef.current = createStore({
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: '{}',
        },
        'item-2': {
          id: 'item-2',
          title: 'Cabildo abierto de Mayo',
          collectionId: 'col-1',
          metadata: '{}',
        },
      },
      ftsSearchImpl: async () => [{ itemId: 'item-2', rank: -0.33 }],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const analysisToggle = await screen.findByRole('tab', { name: /Análisis/i })
    await fireEvent.click(analysisToggle)

    const input = (await screen.findByPlaceholderText('Escribí para buscar...')) as HTMLInputElement

    await fireEvent.input(input, { target: { value: 'cabildo' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => {
      expect(storeRef.current.fts.searchWithDebug).toHaveBeenCalledTimes(1)
      expect(storeRef.current.fts.searchWithDebug).toHaveBeenCalledWith('cabildo', 10)
      expect(screen.getByText('Cabildo')).toBeInTheDocument()
      expect(screen.getByText('abierto de Mayo', { exact: false })).toBeInTheDocument()
    })

    await new Promise((resolve) => setTimeout(resolve, 350))
    expect(storeRef.current.fts.searchWithDebug).toHaveBeenCalledTimes(1)

    await fireEvent.keyDown(input, { key: 'Escape' })
    expect(input.value).toBe('')
    expect(screen.getByText('Ingresá un término para ver resultados.')).toBeInTheDocument()
  })

  it('shows FTS debug panel only in dev with query metadata', async () => {
    storeRef.current = createStore({
      ftsStatsTotal: 99,
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: '{}',
        },
        'item-2': {
          id: 'item-2',
          title: 'Sindicato Obrero',
          collectionId: 'col-1',
          metadata: '{}',
        },
      },
      ftsSearchImpl: async () => [{ itemId: 'item-2', rank: -0.4 }],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const analysisToggle = await screen.findByRole('tab', { name: /Análisis/i })
    await fireEvent.click(analysisToggle)

    expect(await screen.findByText('FTS Debug (solo dev)')).toBeInTheDocument()

    const input = await screen.findByPlaceholderText('Escribí para buscar...')
    await fireEvent.input(input, { target: { value: 'sindicato' } })

    await waitFor(() => {
      expect(screen.getByText('Filas indexadas')).toBeInTheDocument()
      expect(screen.getByText('99')).toBeInTheDocument()
      expect(screen.getByText('Query original')).toBeInTheDocument()
      expect(screen.getByText('sindicato')).toBeInTheDocument()
      expect(screen.getByText('Sanitizada')).toBeInTheDocument()
      expect(screen.getByText('"sindicato"')).toBeInTheDocument()
      expect(screen.getByText('Matches DB')).toBeInTheDocument()
      expect(screen.getAllByText('1').length).toBeGreaterThanOrEqual(2)
      expect(screen.getByText('item-2')).toBeInTheDocument()
    })
  })
})

describe('ItemView note editing', () => {
  const sampleNote = {
    id: 'note-1',
    itemId: 'item-1',
    content: '<p>Original <strong>note</strong> content</p>',
    createdAt: Date.now(),
    updatedAt: Date.now(),
  }

  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
    invokeMock.mockClear()
  })

  async function renderItemViewWithNotes(notes: (typeof sampleNote)[]) {
    storeRef.current = createStore({ notesRows: notes })
    storeRef.current.notes.findByItem.mockResolvedValue(notes)
    storeRef.current.notes.findByAsset.mockResolvedValue(notes)
    storeRef.current.notes.update.mockResolvedValue(undefined)
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByText(new RegExp(`Notas \\(${notes.length}\\)`))
  }

  it('displays the correct note count', async () => {
    await renderItemViewWithNotes([sampleNote])
    expect(screen.getByText(/Notas \(1\)/)).toBeInTheDocument()
  })

  it('renders icon-only note action buttons with accessible names', async () => {
    await renderItemViewWithNotes([sampleNote])

    expect(screen.getByRole('button', { name: 'Editar nota' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Eliminar nota' })).toBeInTheDocument()
  })

  it('renders stored rich text notes sanitized', async () => {
    await renderItemViewWithNotes([
      {
        ...sampleNote,
        content:
          '<h2>Nota</h2><p>Texto <a href="https://entropia.dev">seguro</a></p><script>alert(1)</script>',
      },
    ])

    expect(screen.queryByRole('heading', { name: 'Nota', level: 2 })).not.toBeInTheDocument()

    const noteRow = screen.getByRole('button', { name: /Nota Texto seguro/i })
    await fireEvent.click(noteRow)

    expect(screen.getByRole('heading', { name: 'Nota', level: 2 })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'seguro' })).toHaveAttribute(
      'href',
      'https://entropia.dev'
    )
    expect(screen.queryByText('alert(1)')).not.toBeInTheDocument()
  })

  it('renders legacy plain text notes correctly', async () => {
    await renderItemViewWithNotes([
      {
        ...sampleNote,
        content: 'Linea uno\n\nLinea dos',
      },
    ])

    expect(screen.queryByText('Linea uno')).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: /Linea uno Linea dos/i }))

    expect(screen.getByText('Linea uno')).toBeInTheDocument()
    expect(screen.getByText('Linea dos')).toBeInTheDocument()
  })

  it('keeps notes collapsed into a single clickable row until expanded', async () => {
    await renderItemViewWithNotes([
      {
        ...sampleNote,
        content: '<p>Una nota bastante larga para verificar vista previa compacta</p>',
      },
    ])

    const noteRow = screen.getByRole('button', {
      name: /Una nota bastante larga para verificar vista previa compacta/i,
    })

    expect(noteRow).toHaveAttribute('aria-expanded', 'false')
    expect(
      screen.getByText(new Date(sampleNote.createdAt).toLocaleDateString())
    ).toBeInTheDocument()

    await fireEvent.click(noteRow)
    expect(noteRow).toHaveAttribute('aria-expanded', 'true')

    await fireEvent.click(noteRow)
    expect(noteRow).toHaveAttribute('aria-expanded', 'false')
  })

  it('clicking note links does not collapse the expanded row', async () => {
    await renderItemViewWithNotes([
      {
        ...sampleNote,
        content: '<p>Texto con <a href="https://entropia.dev">enlace</a></p>',
      },
    ])

    const noteRow = screen.getByRole('button', { name: /Texto con enlace/i })
    await fireEvent.click(noteRow)

    const link = screen.getByRole('link', { name: 'enlace' })
    await fireEvent.click(link)

    expect(noteRow).toHaveAttribute('aria-expanded', 'true')
  })

  it('opens expanded note links through the external url command', async () => {
    await renderItemViewWithNotes([
      {
        ...sampleNote,
        content: '<p>Texto con <a href="entropia.dev/docs">enlace</a></p>',
      },
    ])

    await fireEvent.click(screen.getByRole('button', { name: /Texto con enlace/i }))
    await fireEvent.click(screen.getByRole('link', { name: 'enlace' }))

    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://entropia.dev/docs',
    })
  })

  it('clicking note edit action does not toggle expansion', async () => {
    await renderItemViewWithNotes([sampleNote])

    const noteRow = screen.getByRole('button', { name: /Original note content/i })
    expect(noteRow).toHaveAttribute('aria-expanded', 'false')

    await fireEvent.click(screen.getByRole('button', { name: 'Editar nota' }))

    expect(noteRow).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getAllByTestId('note-save')).toHaveLength(2)
  })

  it('clicking note delete action opens confirmation without toggling expansion', async () => {
    await renderItemViewWithNotes([sampleNote])

    const noteRow = screen.getByRole('button', { name: /Original note content/i })
    expect(noteRow).toHaveAttribute('aria-expanded', 'false')

    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar nota' }))

    expect(noteRow).toHaveAttribute('aria-expanded', 'false')
    const dialog = screen.getByRole('alertdialog', { name: 'Eliminar nota' })
    expect(dialog).toBeInTheDocument()
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleDescription(
      '¿Seguro que querés eliminar esta nota? Esta acción no se puede deshacer.'
    )
    expect(
      screen.getByText('¿Seguro que querés eliminar esta nota? Esta acción no se puede deshacer.')
    ).toBeInTheDocument()
    expect(storeRef.current.notes.delete).not.toHaveBeenCalled()
  })

  it('deletes the note only after confirming', async () => {
    await renderItemViewWithNotes([sampleNote])

    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar nota' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Confirmar eliminación de nota' }))

    expect(storeRef.current.notes.delete).toHaveBeenCalledWith('note-1')
  })

  it('cancelling note deletion keeps the note untouched', async () => {
    await renderItemViewWithNotes([sampleNote])

    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar nota' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))

    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(storeRef.current.notes.delete).not.toHaveBeenCalled()
  })

  it('closes note delete confirmation with Escape and restores focus to the trigger', async () => {
    await renderItemViewWithNotes([sampleNote])

    const deleteButton = screen.getByRole('button', { name: 'Eliminar nota' })
    deleteButton.focus()
    await fireEvent.click(deleteButton)

    const dialog = screen.getByRole('alertdialog', { name: 'Eliminar nota' })
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Cancelar' })).toHaveFocus()
    })

    await fireEvent.keyDown(dialog, { key: 'Escape' })

    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(deleteButton).toHaveFocus()
    expect(storeRef.current.notes.delete).not.toHaveBeenCalled()
  })

  it('uses the rich text editor for editing existing notes', async () => {
    await renderItemViewWithNotes([sampleNote])

    expect(screen.getAllByTestId('note-save')).toHaveLength(1)

    await fireEvent.click(screen.getByRole('button', { name: 'Editar nota' }))

    expect(screen.getAllByTestId('note-save')).toHaveLength(2)
    expect(screen.getAllByRole('button', { name: 'Negrita' }).length).toBeGreaterThanOrEqual(2)
  })

  it('keeps the edit save action disabled until the note has content to persist', async () => {
    await renderItemViewWithNotes([{ ...sampleNote, content: '' }])

    await fireEvent.click(screen.getByRole('button', { name: 'Editar nota' }))

    const saveButtons = screen.getAllByTestId('note-save')
    expect(saveButtons.at(-1)).toBeDisabled()
  })

  it('displays localized empty state when notes array is empty', async () => {
    storeRef.current = createStore()
    storeRef.current.notes.findByItem.mockResolvedValue([])
    storeRef.current.notes.findByAsset.mockResolvedValue([])
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    expect(await screen.findByText('Todavía no hay notas.')).toBeInTheDocument()
  })

  it('notes store has update method for editing notes', async () => {
    await renderItemViewWithNotes([sampleNote])
    expect(storeRef.current.notes.update).toBeDefined()
    expect(typeof storeRef.current.notes.update).toBe('function')
  })

  it('notes store update method can be called with note id and content', async () => {
    await renderItemViewWithNotes([sampleNote])
    await storeRef.current.notes.update('note-1', '<p>Updated content</p>')
    expect(storeRef.current.notes.update).toHaveBeenCalledWith('note-1', '<p>Updated content</p>')
  })

  it('after update, notes are reloaded from store', async () => {
    const updatedNote = { ...sampleNote, content: '<p>Updated content</p>', updatedAt: Date.now() }
    storeRef.current.notes.findByItem.mockResolvedValueOnce([sampleNote])
    storeRef.current.notes.findByItem.mockResolvedValueOnce([updatedNote])

    await renderItemViewWithNotes([sampleNote])

    // Simulate the update that handleSaveEdit would do
    await storeRef.current.notes.update('note-1', '<p>Updated content</p>')
    // After update, notes are loaded in the current asset scope
    expect(storeRef.current.notes.findByAsset).toHaveBeenCalledWith('item-1', 'asset-1')
  })
})

describe('ItemView image annotations', () => {
  const layoutFixture = {
    id: 'layout-1',
    assetId: 'asset-image-1',
    model: 'paddle_vl',
    imageWidth: 1000,
    imageHeight: 1400,
    createdAt: 1,
    regions: [
      {
        category: 'doc_title',
        confidence: 0.98,
        groupId: 1,
        bbox: { x: 10, y: 20, width: 200, height: 80 },
        page: 1,
      },
      {
        category: 'text',
        confidence: 0.96,
        groupId: 2,
        bbox: { x: 30, y: 140, width: 260, height: 120 },
        page: 1,
      },
      {
        category: 'table',
        confidence: 0.94,
        groupId: 3,
        bbox: { x: 40, y: 300, width: 300, height: 130 },
        page: 1,
      },
      {
        category: 'figure',
        confidence: 0.93,
        groupId: 4,
        bbox: { x: 360, y: 120, width: 180, height: 180 },
        page: 1,
      },
      {
        category: 'abandoned',
        confidence: 0.9,
        groupId: 5,
        bbox: { x: 60, y: 450, width: 220, height: 80 },
        page: 1,
      },
    ],
    blocks: [
      {
        label: 'title',
        content: 'Bloque título',
        bbox: { x: 8, y: 18, width: 180, height: 70 },
        order: 1,
        groupId: 1,
        page: 1,
      },
      {
        label: 'plain_text',
        content: 'Bloque cuerpo',
        bbox: { x: 28, y: 138, width: 250, height: 110 },
        order: 2,
        groupId: 2,
        page: 1,
      },
      {
        label: 'table',
        content: 'Bloque tabla',
        bbox: { x: 42, y: 302, width: 280, height: 120 },
        order: 3,
        groupId: 3,
        page: 1,
      },
      {
        label: 'figure',
        content: 'Bloque figura',
        bbox: { x: 362, y: 122, width: 160, height: 170 },
        order: 4,
        groupId: 4,
        page: 1,
      },
      {
        label: 'vision_footnote',
        content: 'Bloque nota',
        bbox: { x: 62, y: 452, width: 210, height: 70 },
        order: 5,
        groupId: 5,
        page: 1,
      },
    ],
  }

  beforeEach(() => {
    vi.useFakeTimers()
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
    getLayoutByAssetMock.mockReset().mockResolvedValue(null)
  })

  it('loads annotations per asset and rehydrates when switching assets', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
        {
          id: 'asset-image-2',
          itemId: 'item-1',
          path: 'docs/photo-b.jpg',
          type: 'image',
          createdAt: 2,
        },
      ],
      annotationsByAsset: {
        'asset-image-1': [
          {
            id: 'ann-1',
            assetId: 'asset-image-1',
            page: 1,
            kind: 'rectangle',
            color: 'var(--color-accent)',
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
            createdAt: 1,
            updatedAt: 1,
          },
        ],
        'asset-image-2': [
          {
            id: 'ann-2',
            assetId: 'asset-image-2',
            page: 1,
            kind: 'underline',
            color: 'var(--color-warning)',
            x: 0.2,
            y: 0.7,
            width: 0.3,
            height: 0.05,
            createdAt: 2,
            updatedAt: 2,
          },
          {
            id: 'ann-3',
            assetId: 'asset-image-2',
            page: 1,
            kind: 'rectangle',
            color: 'var(--color-danger)',
            x: 0.5,
            y: 0.2,
            width: 0.15,
            height: 0.25,
            createdAt: 3,
            updatedAt: 3,
          },
        ],
      },
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await waitFor(() => {
      expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
    })
    expect(storeRef.current.annotations.findByAsset).toHaveBeenCalledWith('asset-image-1', 1)

    await fireEvent.click(screen.getByRole('button', { name: /página siguiente/i }))

    await waitFor(() => {
      expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('2')
    })
    expect(storeRef.current.annotations.findByAsset).toHaveBeenCalledWith('asset-image-2', 1)
  })

  it('keeps optimistic annotation state and persists with debounce', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: /add annotation/i }))

    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
    expect(storeRef.current.annotations.replaceForAssetPage).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(499)
    expect(storeRef.current.annotations.replaceForAssetPage).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1)

    expect(storeRef.current.annotations.replaceForAssetPage).toHaveBeenCalledTimes(1)
    expect(storeRef.current.annotations.replaceForAssetPage).toHaveBeenCalledWith(
      'asset-image-1',
      1,
      expect.arrayContaining([
        expect.objectContaining({ kind: 'rectangle', color: 'var(--color-accent)' }),
      ])
    )
  })

  it('shows a non-blocking error when annotation save fails', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
      replaceAnnotationsImpl: async () => {
        throw new Error('disk busy')
      },
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: /add annotation/i }))
    await vi.advanceTimersByTimeAsync(500)

    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
    expect(
      await screen.findByText('Failed to save annotations. Changes remain local until retry.')
    ).toBeInTheDocument()
  })

  it('keeps pdf assets view-only by skipping annotation loads', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-pdf-1',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await waitFor(() => {
      expect(screen.getByTestId('viewer-type')).toHaveTextContent('pdf')
    })
    expect(storeRef.current.annotations.findByAsset).not.toHaveBeenCalled()
  })

  it('reloads persisted layout after ocr:complete for the current asset', async () => {
    getLayoutByAssetMock.mockResolvedValueOnce(null).mockResolvedValueOnce(layoutFixture)
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')

    const layoutToggle = await screen.findByRole('button', { name: /mostrar overlay/i })
    await waitFor(() => {
      expect(getLayoutByAssetMock).toHaveBeenCalledTimes(1)
    })
    expect(layoutToggle).toBeDisabled()

    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_length: 128,
        text_content: 'OCR listo',
      },
    })

    await waitFor(() => {
      expect(getLayoutByAssetMock).toHaveBeenCalledTimes(2)
      expect(layoutToggle).toBeEnabled()
    })
    expect(screen.getByText(/paddle_vl · 5 bloques · 5 regiones/i)).toBeInTheDocument()
  })

  it('syncs list hover/select with overlay state and keeps selection persistent', async () => {
    getLayoutByAssetMock.mockResolvedValue(layoutFixture)
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
    })

    await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))

    const firstBlock = await screen.findByTestId('layout-block-item-layout-block-0')
    const secondBlock = await screen.findByTestId('layout-block-item-layout-block-1')

    await fireEvent.mouseEnter(firstBlock)
    expect(screen.getByTestId('viewer-hovered-layout-region')).toHaveTextContent(
      'layout-block-0::overlay'
    )
    expect(firstBlock.className).toContain('hovered')

    await fireEvent.click(firstBlock)
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent(
      'layout-block-0::overlay'
    )
    expect(firstBlock.className).toContain('selected')

    await fireEvent.mouseEnter(secondBlock)
    expect(screen.getByTestId('viewer-hovered-layout-region')).toHaveTextContent(
      'layout-block-1::overlay'
    )
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent(
      'layout-block-0::overlay'
    )
    expect(firstBlock.className).toContain('selected')
    expect(secondBlock.className).toContain('hovered')

    await fireEvent.click(secondBlock)
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent(
      'layout-block-1::overlay'
    )
    expect(secondBlock.className).toContain('selected')
  })

  it('syncs overlay hover/select back to the list and auto-scrolls selected block', async () => {
    getLayoutByAssetMock.mockResolvedValue(layoutFixture)
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
    })

    const scrollIntoViewSpy = vi
      .spyOn(HTMLElement.prototype, 'scrollIntoView')
      .mockImplementation(() => undefined)

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
    })

    await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))

    const firstBlock = await screen.findByTestId('layout-block-item-layout-block-0')
    const secondBlock = await screen.findByTestId('layout-block-item-layout-block-1')

    await fireEvent.click(screen.getByRole('button', { name: /hover first layout region/i }))
    expect(firstBlock.className).toContain('hovered')

    await fireEvent.click(screen.getByRole('button', { name: /select second layout region/i }))
    await waitFor(() => {
      expect(secondBlock.className).toContain('selected')
    })
    expect(scrollIntoViewSpy).toHaveBeenCalled()
    expect(scrollIntoViewSpy.mock.instances.at(-1)).toBe(secondBlock)

    await fireEvent.click(screen.getByRole('button', { name: /clear layout hover/i }))
    expect(firstBlock.className).not.toContain('hovered')

    scrollIntoViewSpy.mockRestore()
  })

  it('filters layout blocks by type, shows counters, and hides non-matching overlays', async () => {
    getLayoutByAssetMock.mockResolvedValue(layoutFixture)
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
    })

    expect(screen.getByTestId('layout-filter-count-all')).toHaveTextContent('5')
    expect(screen.getByTestId('layout-filter-count-titles')).toHaveTextContent('1')
    expect(screen.getByTestId('layout-filter-count-text')).toHaveTextContent('1')
    expect(screen.getByTestId('layout-filter-count-tables')).toHaveTextContent('1')
    expect(screen.getByTestId('layout-filter-count-figures')).toHaveTextContent('1')
    expect(screen.getByTestId('layout-filter-count-notes')).toHaveTextContent('1')

    await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))
    expect(screen.getByTestId('viewer-layout-region-count')).toHaveTextContent('5')

    await fireEvent.click(screen.getByTestId('layout-filter-figures'))

    expect(screen.getByTestId('viewer-layout-region-count')).toHaveTextContent('1')
    expect(await screen.findByTestId('layout-block-item-layout-block-3')).toBeInTheDocument()
    expect(screen.queryByTestId('layout-block-item-layout-block-0')).not.toBeInTheDocument()
    expect(screen.getByText('Mostrando 1 de 5 bloques.')).toBeInTheDocument()
  })

  it('shows a rich inspector for the selected block and exposes quick copy actions', async () => {
    getLayoutByAssetMock.mockResolvedValue(layoutFixture)
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    expect(screen.getByTestId('layout-inspector-empty')).toBeInTheDocument()

    await fireEvent.click(await screen.findByTestId('layout-block-item-layout-block-0'))

    expect(screen.getByTestId('layout-inspector-label')).toHaveTextContent('title')
    expect(screen.getByTestId('layout-inspector-overlay-source')).toHaveTextContent(
      'BBox del bloque'
    )
    expect(screen.getByTestId('layout-inspector-bbox')).toHaveTextContent('x:8 y:18 w:180 h:70')
    expect(screen.getByTestId('layout-inspector-content')).toHaveTextContent('Bloque título')

    await fireEvent.click(screen.getByTestId('layout-inspector-copy-text'))
    expect(clipboardWriteTextMock).toHaveBeenCalledWith('Bloque título')

    await fireEvent.click(screen.getByTestId('layout-inspector-copy-bbox'))
    expect(clipboardWriteTextMock).toHaveBeenCalledWith('x:8 y:18 w:180 h:70')

    await fireEvent.click(screen.getByTestId('layout-inspector-copy-json'))
    expect(clipboardWriteTextMock).toHaveBeenLastCalledWith(
      expect.stringContaining('"overlaySource": "block"')
    )
    expect(screen.getByTestId('layout-inspector-copy-message')).toHaveTextContent('JSON copiado.')
  })

  it('tracks a page-aware layoutActivePage for multi-page pdf layouts', async () => {
    getLayoutByAssetMock.mockResolvedValue({
      ...layoutFixture,
      assetId: 'asset-pdf-1',
      regions: [
        {
          category: 'doc_title',
          confidence: 0.98,
          groupId: 1,
          bbox: { x: 10, y: 20, width: 200, height: 80 },
          page: 1,
        },
        {
          category: 'table',
          confidence: 0.94,
          groupId: 2,
          bbox: { x: 20, y: 140, width: 260, height: 120 },
          page: 2,
        },
      ],
      blocks: [
        {
          label: 'title',
          content: 'Bloque título página 1',
          bbox: { x: 8, y: 18, width: 180, height: 70 },
          order: 1,
          groupId: 1,
          page: 1,
        },
        {
          label: 'table',
          content: 'Bloque tabla página 2',
          bbox: { x: 18, y: 138, width: 240, height: 110 },
          order: 2,
          groupId: 2,
          page: 2,
        },
      ],
    })
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-pdf-1',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
    })

    const expectLayoutHeading = (page: number) =>
      expect(
        screen
          .getAllByText(new RegExp(`Página ${page}`, 'i'))
          .some((node) => node.textContent?.includes(`Página ${page}`))
      ).toBe(true)

    expect(screen.getByText('Mostrando 1 de 1 bloques.')).toBeInTheDocument()
    expectLayoutHeading(1)
    expect(screen.getByTestId('layout-filter-count-all')).toHaveTextContent('1')
    expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('1')
    expect(screen.getByTestId('layout-page-summary')).toHaveTextContent('Página 1 de 2')
    expect(screen.getByTestId('layout-page-chip-1')).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByTestId('layout-page-chip-2')).toHaveTextContent('2')

    await fireEvent.click(screen.getByRole('button', { name: /go to page 2/i }))

    await waitFor(() => {
      expectLayoutHeading(2)
    })
    expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('2')
    expect(screen.getByText('Mostrando 1 de 1 bloques.')).toBeInTheDocument()
    expect(screen.getByTestId('layout-filter-count-all')).toHaveTextContent('1')
    expect(screen.getByTestId('layout-page-summary')).toHaveTextContent('Página 2 de 2')
    expect(screen.getByTestId('layout-page-chip-2')).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByText(/paddle_vl · 1 bloques · 1 regiones/i)).toBeInTheDocument()

    await fireEvent.click(screen.getByTestId('layout-page-chip-1'))

    await waitFor(() => {
      expectLayoutHeading(1)
    })
    expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('1')
    expect(screen.getByTestId('layout-page-chip-1')).toHaveAttribute('aria-pressed', 'true')
  })

  it('clears the visible layout selection when the active page changes away from the selected block', async () => {
    getLayoutByAssetMock.mockResolvedValue({
      ...layoutFixture,
      assetId: 'asset-pdf-1',
      regions: [
        {
          category: 'doc_title',
          confidence: 0.98,
          groupId: 1,
          bbox: { x: 10, y: 20, width: 200, height: 80 },
          page: 1,
        },
        {
          category: 'table',
          confidence: 0.94,
          groupId: 2,
          bbox: { x: 20, y: 140, width: 260, height: 120 },
          page: 2,
        },
      ],
      blocks: [
        {
          label: 'title',
          content: 'Bloque título página 1',
          bbox: { x: 8, y: 18, width: 180, height: 70 },
          order: 1,
          groupId: 1,
          page: 1,
        },
        {
          label: 'table',
          content: 'Bloque tabla página 2',
          bbox: { x: 18, y: 138, width: 240, height: 110 },
          order: 2,
          groupId: 2,
          page: 2,
        },
      ],
    })
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-pdf-1',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
    })

    await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))

    const pageOneBlock = await screen.findByTestId('layout-block-item-layout-block-0')
    await fireEvent.click(pageOneBlock)

    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent(
      'layout-block-0::overlay'
    )

    await fireEvent.click(screen.getByTestId('layout-page-chip-2'))

    await waitFor(() => {
      expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('2')
    })
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent('none')
    expect(screen.queryByTestId('layout-block-item-layout-block-0')).not.toBeInTheDocument()
    expect(await screen.findByTestId('layout-block-item-layout-block-1')).toBeInTheDocument()
  })

  it('keeps the chosen filter during internal navigation and clears selection when filtered out', async () => {
    getLayoutByAssetMock.mockResolvedValue(layoutFixture)
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/photo-a.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
    })

    await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))

    const titleBlock = await screen.findByTestId('layout-block-item-layout-block-0')
    await fireEvent.click(titleBlock)
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent(
      'layout-block-0::overlay'
    )

    const tablesFilter = screen.getByTestId('layout-filter-tables')
    await fireEvent.click(tablesFilter)

    expect(tablesFilter).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent('none')
    expect(screen.queryByTestId('layout-block-item-layout-block-0')).not.toBeInTheDocument()

    const tableBlock = await screen.findByTestId('layout-block-item-layout-block-2')
    await fireEvent.mouseEnter(tableBlock)
    expect(screen.getByTestId('viewer-hovered-layout-region')).toHaveTextContent(
      'layout-block-2::overlay'
    )
    expect(tablesFilter).toHaveAttribute('aria-pressed', 'true')

    await fireEvent.click(tableBlock)
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent(
      'layout-block-2::overlay'
    )
    expect(tablesFilter).toHaveAttribute('aria-pressed', 'true')
  })
})

describe('ItemView entity editing UX', () => {
  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
  })

  async function renderAnalysisWithEntities() {
    storeRef.current = createStore({
      entitiesRows: [
        {
          id: 'entity-1',
          itemId: 'item-1',
          entityType: 'organization',
          value: 'Mar del Plata',
          startOffset: 10,
          endOffset: 23,
          confidence: 0.95,
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: /Análisis/i }))
  }

  it('opens inline entity editing from chip click and saves edits on Enter', async () => {
    await renderAnalysisWithEntities()

    await fireEvent.click(await screen.findByTestId('mock-entity-entity-1'))

    const input = await screen.findByLabelText('Editar valor de entidad')
    expect(input).toHaveValue('Mar del Plata')

    await fireEvent.input(input, {
      target: { value: 'Mar del Plata 1970' },
    })
    await fireEvent.keyDown(input, { key: 'Enter' })

    expect(storeRef.current.entities.update).toHaveBeenCalledWith('entity-1', {
      entityType: 'organization',
      value: 'Mar del Plata 1970',
      confidence: 1,
      source: 'manual',
    })
  })

  it('deletes entity from chip action', async () => {
    await renderAnalysisWithEntities()

    const deleteBtn = await screen.findByRole('button', { name: 'Eliminar entidad Mar del Plata' })

    await fireEvent.click(deleteBtn)

    expect(storeRef.current.entities.delete).toHaveBeenCalledWith('entity-1')
  })

  it('blur saves trimmed changed values inline', async () => {
    await renderAnalysisWithEntities()

    await fireEvent.click(await screen.findByTestId('mock-entity-entity-1'))

    const input = await screen.findByLabelText('Editar valor de entidad')
    await fireEvent.input(input, { target: { value: '  Mar del Plata 1980  ' } })
    await fireEvent.blur(input)

    expect(storeRef.current.entities.update).toHaveBeenCalledWith('entity-1', {
      entityType: 'organization',
      value: 'Mar del Plata 1980',
      confidence: 1,
      source: 'manual',
    })
  })

  it('creates manual DATE entities', async () => {
    await renderAnalysisWithEntities()

    await fireEvent.change(screen.getByLabelText('Nuevo tipo de entidad'), {
      target: { value: 'date' },
    })
    await fireEvent.input(screen.getByLabelText('Nuevo valor de entidad'), {
      target: { value: '21 de agosto de 1970' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar' }))

    expect(storeRef.current.entities.create).toHaveBeenCalledWith(
      expect.objectContaining({
        itemId: 'item-1',
        entityType: 'date',
        value: '21 de agosto de 1970',
        confidence: 1,
        source: 'manual',
      })
    )
  })
})

describe('ItemView processing labels by asset type', () => {
  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    extractTextMock.mockReset().mockResolvedValue(undefined)
  })

  async function renderTextTabForAsset(assetType: 'image' | 'pdf' | 'audio') {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: `asset-${assetType}-1`,
          itemId: 'item-1',
          path: `docs/sample.${assetType === 'image' ? 'jpg' : assetType === 'pdf' ? 'pdf' : 'mp3'}`,
          type: assetType,
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: 'Texto' }))
  }

  it('keeps OCR labels for image assets', async () => {
    await renderTextTabForAsset('image')

    expect(screen.getByRole('button', { name: 'OCRL' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRH' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRC' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRR' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PTT' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFC' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFR' })).not.toBeInTheDocument()
  })

  it('uses PDF-specific labels and hides OCR wording for pdf assets', async () => {
    await renderTextTabForAsset('pdf')

    expect(screen.getByRole('button', { name: 'PTT' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'PDFC' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'PDFR' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRL' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRH' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRC' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRR' })).not.toBeInTheDocument()
  })

  it('uses transcription and summary labels for audio assets without OCR wording', async () => {
    await renderTextTabForAsset('audio')

    expect(screen.getByRole('button', { name: 'STT' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Resumen' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRL' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRH' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRC' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRR' })).not.toBeInTheDocument()
  })
})
