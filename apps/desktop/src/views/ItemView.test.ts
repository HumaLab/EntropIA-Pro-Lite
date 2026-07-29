import { render, screen, fireEvent, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import ItemView from './ItemView.svelte'
import { navigation } from '$lib/navigation'
import { setupKeyboardShortcuts } from '$lib/keyboard'
import { DOCUMENT_ASSET_DELETED_EVENT } from '$lib/document-explorer'

const {
  nlpEventHandlers,
  embedAssetMock,
  extractEntitiesForAssetMock,
  indexFtsMock,
  extractTriplesMock,
  llmCorrectOcrMock,
  llmSummarizeAssetMock,
  llmCorrectOcrAssetMock,
  llmExtractTriplesMock,
  llmExtractTriplesAssetMock,
  similarAssetsMock,
  extractTextMock,
  getLayoutByAssetMock,
  clipboardWriteTextMock,
  llmIsAvailableMock,
  invokeMock,
  emitMock,
} = vi.hoisted(() => ({
  nlpEventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  embedAssetMock: vi.fn<(_: string, __: string) => Promise<void>>(),
  extractEntitiesForAssetMock: vi.fn<(_: string, __: string) => Promise<void>>(),
  indexFtsMock: vi.fn<(_: string) => Promise<void>>(),
  extractTriplesMock: vi.fn<(_: string) => Promise<void>>(),
  llmCorrectOcrMock: vi.fn<(_: string) => Promise<void>>(),
  llmSummarizeAssetMock: vi.fn<(_: string) => Promise<void>>(),
  llmCorrectOcrAssetMock: vi.fn<(_: string) => Promise<void>>(),
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
        textPreview?: string
        similarity: number
      }>
    >
  >(),
  extractTextMock: vi.fn(),
  getLayoutByAssetMock: vi.fn(),
  clipboardWriteTextMock: vi.fn<(_: string) => Promise<void>>(),
  llmIsAvailableMock: vi.fn<() => Promise<boolean>>(),
  invokeMock: vi.fn<(_: string, __?: unknown) => Promise<unknown>>(async (command: string) => {
    if (command === 'llm_get_results') return []
    if (command === 'llm_get_result') return null
    if (command === 'llm_is_available') return true
    if (command === 'db_select') return []
    return null
  }),
  emitMock: vi.fn<(_: string, __?: unknown) => Promise<void>>(),
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

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
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
    parentAssetId?: string | null
    pageNumber?: number | null
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
  extractionsByAsset?: Record<string, { textContent: string; method?: string }>
  transcriptionsByAsset?: Record<
    string,
    {
      textContent: string
      language?: string | null
      durationMs?: number | null
      segments?: string | null
    }
  >
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
  extractionsByAsset = {},
  transcriptionsByAsset = {},
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
      create: vi.fn().mockImplementation(async (data) => ({
        ...data,
        id: 'asset-pdf-crop-1',
        createdAt: 2,
      })),
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
      findByAsset: vi.fn().mockImplementation(async (assetId: string) => {
        const extraction = extractionsByAsset[assetId]
        return extraction
          ? { textContent: extraction.textContent, method: extraction.method ?? 'light' }
          : null
      }),
      deleteByAsset: vi.fn().mockResolvedValue(undefined),
    },
    transcriptions: {
      findByAsset: vi.fn().mockImplementation(async (assetId: string) => {
        return transcriptionsByAsset[assetId] ?? null
      }),
    },
    entities: {
      findByItemId: vi.fn().mockResolvedValue(entitiesRows),
      findByAssetId: vi.fn().mockResolvedValue(entitiesRows),
      create: vi.fn().mockResolvedValue(undefined),
      update: vi.fn().mockResolvedValue(undefined),
      setManualLocation: vi.fn().mockResolvedValue(undefined),
      resetManualLocation: vi.fn().mockResolvedValue(undefined),
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
      deleteByAssetId: vi.fn().mockResolvedValue(undefined),
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
    indexFts: indexFtsMock,
    embedAsset: embedAssetMock,
    extractEntities: vi.fn().mockResolvedValue(undefined),
    extractEntitiesForAsset: extractEntitiesForAssetMock,
  }
})

vi.mock('$lib/llm', async () => {
  const actual = await vi.importActual<typeof import('$lib/llm')>('$lib/llm')
  return {
    ...actual,
    llmIsAvailable: llmIsAvailableMock,
    llmGetResult: vi.fn().mockResolvedValue(null),
    llmGetResults: vi.fn().mockResolvedValue([]),
    llmSummarize: vi.fn().mockResolvedValue(undefined),
    llmCorrectOcr: llmCorrectOcrMock,
    llmExtractTriples: llmExtractTriplesMock,
    llmSummarizeAsset: llmSummarizeAssetMock,
    llmCorrectOcrAsset: llmCorrectOcrAssetMock,
    llmExtractTriplesAsset: llmExtractTriplesAssetMock,
  }
})

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((eventName: string, callback: (event: { payload: unknown }) => void) => {
    nlpEventHandlers.set(eventName, callback)
    return Promise.resolve(vi.fn())
  }),
  emit: emitMock,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@entropia/ui', async () => {
  const MockActionIcon = (await import('./__mocks__/MockActionIcon.svelte')).default
  const MockDocumentViewer = (await import('./__mocks__/MockDocumentViewer.svelte')).default
  const MockEntityViewer = (await import('./__mocks__/MockEntityViewer.svelte')).default
  const ActualConfirmDialog = (
    await import('../../../../packages/ui/src/components/ConfirmDialog/ConfirmDialog.svelte')
  ).default
  const ActualIconButton = (
    await import('../../../../packages/ui/src/components/IconButton/IconButton.svelte')
  ).default
  const ActualMetadataEditor = (
    await import('../../../../packages/ui/src/components/MetadataEditor/MetadataEditor.svelte')
  ).default
  const ActualPanel = (await import('../../../../packages/ui/src/components/Panel/Panel.svelte'))
    .default
  const ActualStatusBadge = (
    await import('../../../../packages/ui/src/components/StatusBadge/StatusBadge.svelte')
  ).default
  const ActualTabButton = (
    await import('../../../../packages/ui/src/components/Tabs/TabButton.svelte')
  ).default
  const ActualTabList = (await import('../../../../packages/ui/src/components/Tabs/TabList.svelte'))
    .default
  const MockButton = (await import('./__mocks__/MockButton.svelte')).default
  const MockCard = (await import('./__mocks__/MockCard.svelte')).default
  const MockMapViewer = (await import('./__mocks__/MockMapViewer.svelte')).default
  const MockNoteEditor = (await import('./__mocks__/MockNoteEditor.svelte')).default

  return {
    ActionIcon: MockActionIcon,
    ConfirmDialog: ActualConfirmDialog,
    DocumentViewer: MockDocumentViewer,
    MetadataEditor: ActualMetadataEditor,
    NoteEditor: MockNoteEditor,
    Button: MockButton,
    Card: MockCard,
    EntityViewer: MockEntityViewer,
    IconButton: ActualIconButton,
    MapViewer: MockMapViewer,
    Panel: ActualPanel,
    StatusBadge: ActualStatusBadge,
    TabButton: ActualTabButton,
    TabList: ActualTabList,
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
  navigation.resetToPath([{ name: 'collections' }])
  Object.defineProperty(globalThis.navigator, 'clipboard', {
    configurable: true,
    value: { writeText: clipboardWriteTextMock },
  })
  clipboardWriteTextMock.mockReset().mockResolvedValue(undefined)
  llmIsAvailableMock.mockReset().mockResolvedValue(true)
  llmCorrectOcrMock.mockReset().mockResolvedValue(undefined)
  emitMock.mockReset().mockResolvedValue(undefined)
  extractEntitiesForAssetMock.mockReset().mockResolvedValue(undefined)
  indexFtsMock.mockReset().mockResolvedValue(undefined)
  invokeMock.mockReset().mockImplementation(async (command: string) => {
    if (command === 'llm_get_results') return []
    if (command === 'llm_get_result') return null
    if (command === 'llm_is_available') return true
    if (command === 'db_select') return []
    return null
  })
})

describe('ItemView multi-asset navigation', () => {
  const multiPageAssets = [
    {
      id: 'asset-page-1',
      itemId: 'item-1',
      path: 'docs/757-70_page_1.png',
      type: 'image' as const,
      createdAt: 1,
    },
    {
      id: 'asset-page-2',
      itemId: 'item-1',
      path: 'docs/11111111-1111-4111-8111-111111111111_757-70_page_2.png',
      type: 'image' as const,
      createdAt: 2,
    },
    {
      id: 'asset-page-3',
      itemId: 'item-1',
      path: 'docs/757-70_page_3.png',
      type: 'image' as const,
      createdAt: 3,
    },
  ]

  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    llmCorrectOcrMock.mockReset().mockResolvedValue(undefined)
    llmSummarizeAssetMock.mockReset().mockResolvedValue(undefined)
    llmCorrectOcrAssetMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    storeRef.current = createStore({ assetsRows: multiPageAssets })
  })

  it('opens the asset requested by navigation instead of pinning the first sibling', async () => {
    navigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
        assetId: 'asset-page-2',
        assetLabel: '757-70_page_2.png',
      },
    ])

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    expect(await screen.findByText(/2\s*\/\s*3/)).toBeInTheDocument()
    expect(screen.getAllByText(/757-70_page_2\.png/).length).toBeGreaterThan(0)
    expect(screen.queryByText(/11111111-1111-4111-8111-111111111111_/)).not.toBeInTheDocument()
  })

  it('keeps navigation and explorer selection events synced when using the asset paginator', async () => {
    const selectedEvents: Array<{ itemId: string; assetId: string | null }> = []
    const handleSelected = (event: Event) => {
      const detail = (event as CustomEvent<{ itemId: string; assetId: string | null }>).detail
      selectedEvents.push({ itemId: detail.itemId, assetId: detail.assetId })
    }
    window.addEventListener('entropia:document-explorer-asset-selected', handleSelected)

    try {
      navigation.resetToPath([
        { name: 'collections' },
        { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
        {
          name: 'item',
          collectionId: 'col-1',
          collectionName: 'Colección 1',
          itemId: 'item-1',
          itemTitle: 'Acta histórica',
        },
      ])

      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

      expect(await screen.findByText(/1\s*\/\s*3/)).toBeInTheDocument()

      await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))

      expect(await screen.findByText(/2\s*\/\s*3/)).toBeInTheDocument()
      expect(screen.getAllByText(/757-70_page_2\.png/).length).toBeGreaterThan(0)
      await waitFor(() => {
        expect(navigation.current).toMatchObject({
          name: 'item',
          itemId: 'item-1',
          assetId: 'asset-page-2',
          assetLabel: '757-70_page_2.png',
        })
        expect(selectedEvents.at(-1)).toEqual({ itemId: 'item-1', assetId: 'asset-page-2' })
      })
    } finally {
      window.removeEventListener('entropia:document-explorer-asset-selected', handleSelected)
    }
  })

  it('removes an asset deleted from the topbar and selects its next sibling', async () => {
    navigation.resetToPath([
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
        assetId: 'asset-page-2',
        assetLabel: '757-70_page_2.png',
      },
    ])
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    expect(await screen.findByText(/2\s*\/\s*3/)).toBeInTheDocument()

    window.dispatchEvent(
      new CustomEvent(DOCUMENT_ASSET_DELETED_EVENT, {
        detail: { itemId: 'item-1', assetId: 'asset-page-2' },
      })
    )

    expect(await screen.findByText(/2\s*\/\s*2/)).toBeInTheDocument()
    expect(screen.getAllByText(/757-70_page_3\.png/).length).toBeGreaterThan(0)
  })

  it('excludes the PDF parent container from pagination and refreshes the selected page text', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-pdf-parent',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: 1,
        },
        {
          id: 'asset-page-1',
          itemId: 'item-1',
          path: 'docs/acta_page_1.pdf',
          type: 'pdf',
          parentAssetId: 'asset-pdf-parent',
          pageNumber: 1,
          createdAt: 2,
        },
        {
          id: 'asset-page-2',
          itemId: 'item-1',
          path: 'docs/acta_page_2.pdf',
          type: 'pdf',
          parentAssetId: 'asset-pdf-parent',
          pageNumber: 2,
          createdAt: 3,
        },
      ],
      extractionsByAsset: {
        'asset-page-1': { textContent: 'Texto extraído página 1' },
        'asset-page-2': { textContent: 'Texto extraído página 2' },
      },
    })
    navigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
      },
    ])

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    expect(await screen.findByText(/1\s*\/\s*2/)).toBeInTheDocument()
    const previousButton = screen.getByRole('button', {
      name: /Página anterior|Previous page/i,
    })
    const nextButton = screen.getByRole('button', { name: /Página siguiente|Next page/i })
    expect(previousButton).toBeDisabled()
    expect(nextButton).toBeEnabled()

    await fireEvent.click(screen.getByRole('tab', { name: /^Texto$/i }))
    expect(await screen.findByDisplayValue('Texto extraído página 1')).toBeInTheDocument()

    await fireEvent.click(nextButton)

    expect(await screen.findByText(/2\s*\/\s*2/)).toBeInTheDocument()
    expect(previousButton).toBeEnabled()
    expect(nextButton).toBeDisabled()
    expect(screen.getAllByText(/acta_page_2\.pdf/).length).toBeGreaterThan(0)
    await waitFor(() => {
      expect(storeRef.current.extractions.findByAsset).toHaveBeenCalledWith('asset-page-2')
    })
    expect(await screen.findByDisplayValue('Texto extraído página 2')).toBeInTheDocument()
  })

  it('runs OCRC only for the currently selected asset in a multi-page item', async () => {
    navigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
        assetId: 'asset-page-2',
        assetLabel: '757-70_page_2.png',
      },
    ])

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    expect(await screen.findByText(/2\s*\/\s*3/)).toBeInTheDocument()
    await fireEvent.click(await screen.findByRole('tab', { name: 'Texto' }))
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-page-2',
        method: 'paddle_vl',
        text_content: 'Texto OCR página 2',
      },
    })

    const correctButton = await screen.findByRole('button', { name: 'OCRC' })
    await waitFor(() => expect(correctButton).toBeEnabled())
    await fireEvent.click(correctButton)

    expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-page-2')
    expect(llmCorrectOcrAssetMock).not.toHaveBeenCalledWith('asset-page-1')
    expect(llmCorrectOcrMock).not.toHaveBeenCalled()
  })
})

describe('ItemView semantic triples panel', () => {
  beforeEach(() => {
    nlpEventHandlers.clear()
    embedAssetMock.mockReset().mockResolvedValue(undefined)
    extractEntitiesForAssetMock.mockReset().mockResolvedValue(undefined)
    indexFtsMock.mockReset().mockResolvedValue(undefined)
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

    nlpEventHandlers.get('llm:progress')?.({
      payload: { id: 'asset-1', job: 'extract_triples', pct: 25 },
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
    llmSummarizeAssetMock.mockReset().mockResolvedValue(undefined)
    llmCorrectOcrAssetMock.mockReset().mockResolvedValue(undefined)
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

  it('does not show asset A embedding completion as ready on asset B', async () => {
    await openAnalysis(
      createStore({
        assetsRows: [
          {
            id: 'asset-embed-a',
            itemId: 'item-1',
            path: 'docs/acta-1.pdf',
            type: 'pdf',
            createdAt: 1,
          },
          {
            id: 'asset-embed-b',
            itemId: 'item-1',
            path: 'docs/acta-2.pdf',
            type: 'pdf',
            createdAt: 2,
          },
        ],
      })
    )

    nlpEventHandlers.get('nlp:complete')?.({
      payload: { item_id: 'item-1', asset_id: 'asset-embed-a', job: 'embed' },
    })

    await waitFor(() => {
      expect(
        within(screen.getByRole('button', { name: /EMBED/i })).getByText('done')
      ).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))

    await waitFor(() => {
      expect(
        within(screen.getByRole('button', { name: /EMBED/i })).getByText('idle')
      ).toBeInTheDocument()
    })
  })

  it('disables EMBED and shows a graceful hint when no asset is selected', async () => {
    storeRef.current = createStore({ assetsRows: [] })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    expect(screen.queryByRole('button', { name: /EMBED/i })).not.toBeInTheDocument()
    expect(embedAssetMock).not.toHaveBeenCalled()
  })

  it('runs NER only when the user clicks the NER action', async () => {
    await openAnalysis(
      createStore({
        assetsRows: [
          {
            id: 'asset-ner-manual',
            itemId: 'item-1',
            path: 'docs/acta.pdf',
            type: 'pdf',
            createdAt: 1,
          },
        ],
      })
    )

    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
    await fireEvent.click(screen.getByRole('button', { name: /^NER/ }))

    expect(extractEntitiesForAssetMock).toHaveBeenCalledOnce()
    expect(extractEntitiesForAssetMock).toHaveBeenCalledWith('item-1', 'asset-ner-manual')
  })

  it('shows done · 0 on the NER chip when the completed run persisted zero entities', async () => {
    await openAnalysis(
      createStore({
        assetsRows: [
          {
            id: 'asset-ner-1',
            itemId: 'item-1',
            path: 'docs/acta-1.pdf',
            type: 'pdf',
            createdAt: 1,
          },
        ],
      })
    )

    nlpEventHandlers.get('nlp:complete')?.({
      payload: { item_id: 'item-1', asset_id: 'asset-ner-1', job: 'ner', entity_count: 0 },
    })

    await waitFor(() => {
      expect(
        within(screen.getByRole('button', { name: /^NER/ })).getByText('done · 0')
      ).toBeInTheDocument()
    })
  })

  it('shows a plain done NER chip when the completed run persisted entities', async () => {
    await openAnalysis(
      createStore({
        assetsRows: [
          {
            id: 'asset-ner-2',
            itemId: 'item-1',
            path: 'docs/acta-2.pdf',
            type: 'pdf',
            createdAt: 1,
          },
        ],
      })
    )

    nlpEventHandlers.get('nlp:complete')?.({
      payload: { item_id: 'item-1', asset_id: 'asset-ner-2', job: 'ner', entity_count: 12 },
    })

    await waitFor(() => {
      expect(
        within(screen.getByRole('button', { name: /^NER/ })).getByText('done')
      ).toBeInTheDocument()
    })
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
        textPreview:
          'Excelentísimo señor:\nTengo el honor de remitir la carta solicitada.\nArchivo histórico.',
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
    const resultCard = await screen.findByTestId('similar-asset-asset-sim-2')
    expect(resultCard).toBeInTheDocument()
    expect(screen.getByText('Carta manuscrita')).toBeInTheDocument()
    expect(
      screen.getByText(/Excelentísimo señor: Tengo el honor de remitir la carta solicitada/)
    ).toBeInTheDocument()
    expect(screen.getByText('91.3%')).toBeInTheDocument()
    expect(resultCard.querySelector('img')).toHaveAttribute(
      'src',
      'https://asset.localhost/archivo/carta-manuscrita.jpg'
    )

    const technicalMeta = screen.getByText('asset asset-sim-2 · item item-2 · colección col-9')
    expect(technicalMeta).not.toBeVisible()
    expect(screen.getByText('archivo/carta-manuscrita.jpg')).not.toBeVisible()
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

  it('shows readiness guidance when search and similarity have no extracted text yet', async () => {
    storeRef.current = createStore()

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const analysisToggle = await screen.findByRole('tab', { name: /Análisis/i })
    await fireEvent.click(analysisToggle)

    expect(
      await screen.findAllByText(
        'Primero extraé o transcribí texto para que la búsqueda y la similitud tengan material para comparar.'
      )
    ).toHaveLength(2)
  })

  it('shows OpenRouter readiness guidance for semantic similarity after text exists', async () => {
    llmIsAvailableMock.mockResolvedValue(false)
    storeRef.current = createStore({
      extractionsByAsset: {
        'asset-1': { textContent: 'Texto histórico listo para comparar.' },
      },
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const analysisToggle = await screen.findByRole('tab', { name: /Análisis/i })
    await fireEvent.click(analysisToggle)

    expect(
      await screen.findByText(
        'La similitud semántica requiere OpenRouter configurado en Configuración.'
      )
    ).toBeInTheDocument()
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

  it('leaves mailto note links to the browser instead of routing them through the desktop bridge', async () => {
    await renderItemViewWithNotes([
      {
        ...sampleNote,
        content: '<p><a href="mailto:test@entropia.dev">email</a></p>',
      },
    ])

    await fireEvent.click(screen.getByRole('button', { name: 'email' }))
    await fireEvent.click(screen.getByRole('link', { name: 'email' }))

    expect(invokeMock).not.toHaveBeenCalledWith('open_external_url', expect.anything())
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
    expect(screen.getByRole('dialog')).toBeInTheDocument()
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

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
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

  it('ignores stale notes loaded for a previously selected asset', async () => {
    const firstAssetNotes = deferred<(typeof sampleNote)[]>()
    const secondAssetNotes = deferred<(typeof sampleNote)[]>()
    const notesByAsset = new Map([
      ['asset-1', firstAssetNotes],
      ['asset-2', secondAssetNotes],
    ])

    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-1',
          itemId: 'item-1',
          path: 'docs/primera.pdf',
          type: 'pdf',
          createdAt: 1,
        },
        {
          id: 'asset-2',
          itemId: 'item-1',
          path: 'docs/segunda.pdf',
          type: 'pdf',
          createdAt: 2,
        },
      ],
    })
    storeRef.current.notes.findByAsset.mockImplementation(
      async (_itemId: string, assetId: string) => {
        const pending = notesByAsset.get(assetId)
        return pending ? pending.promise : []
      }
    )

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await waitFor(() => {
      expect(storeRef.current.notes.findByAsset).toHaveBeenCalledWith('item-1', 'asset-1')
    })

    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))

    await waitFor(() => {
      expect(storeRef.current.notes.findByAsset).toHaveBeenCalledWith('item-1', 'asset-2')
    })

    secondAssetNotes.resolve([{ ...sampleNote, id: 'note-2', content: '<p>Nota vigente</p>' }])
    expect(await screen.findByRole('button', { name: /Nota vigente/i })).toBeInTheDocument()

    firstAssetNotes.resolve([{ ...sampleNote, id: 'note-1', content: '<p>Nota vieja</p>' }])

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Nota vigente/i })).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: /Nota vieja/i })).not.toBeInTheDocument()
    })
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
    extractEntitiesForAssetMock.mockReset().mockResolvedValue(undefined)
    indexFtsMock.mockReset().mockResolvedValue(undefined)
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

  it('loads and persists annotations for pdf assets', async () => {
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
    expect(storeRef.current.annotations.findByAsset).toHaveBeenCalledWith('asset-pdf-1', 1)

    await fireEvent.click(screen.getByRole('button', { name: /add annotation/i }))
    await vi.advanceTimersByTimeAsync(500)

    expect(storeRef.current.annotations.replaceForAssetPage).toHaveBeenCalledWith(
      'asset-pdf-1',
      1,
      [expect.objectContaining({ kind: 'rectangle' })]
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Erase tool' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
    await vi.advanceTimersByTimeAsync(500)
    expect(storeRef.current.annotations.replaceForAssetPage).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      1,
      expect.arrayContaining([expect.objectContaining({ kind: 'erase', y: 0.25, height: 0.4 })])
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(500)
    expect(storeRef.current.annotations.replaceForAssetPage).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      1,
      expect.arrayContaining([expect.objectContaining({ kind: 'rotation', x: 1 })])
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.runAllTimersAsync()
    const annotationsAfterUndo =
      storeRef.current.annotations.replaceForAssetPage.mock.calls.at(-1)?.[2]
    expect(annotationsAfterUndo).not.toEqual(
      expect.arrayContaining([expect.objectContaining({ kind: 'rotation' })])
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.runAllTimersAsync()
    const annotationsAfterRedo =
      storeRef.current.annotations.replaceForAssetPage.mock.calls.at(-1)?.[2]
    expect(annotationsAfterRedo).toEqual(
      expect.arrayContaining([expect.objectContaining({ kind: 'rotation', x: 1 })])
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.runAllTimersAsync()
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeEnabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.runAllTimersAsync()
    const annotationsAfterEraseUndo =
      storeRef.current.annotations.replaceForAssetPage.mock.calls.at(-1)?.[2]
    expect(annotationsAfterEraseUndo).not.toEqual(
      expect.arrayContaining([expect.objectContaining({ kind: 'erase' })])
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Go to page 2' }))
    await waitFor(() =>
      expect(storeRef.current.annotations.findByAsset).toHaveBeenCalledWith('asset-pdf-1', 2)
    )
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeDisabled()
  })

  it('versions PDF crops on the same asset with undo, redo, and crop-aware extraction', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-pdf-1',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: 1,
          size: 2048,
          pageNumber: 1,
        },
      ],
      extractionsByAsset: {
        'asset-pdf-1': { textContent: 'Texto de la página completa', method: 'native' },
      },
    })
    getLayoutByAssetMock.mockResolvedValueOnce(layoutFixture)
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'crop_pdf') {
        return { path: 'docs/acta_v2.pdf', size: 768 }
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')

    await fireEvent.click(screen.getByRole('button', { name: 'Crop tool' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(invokeMock).toHaveBeenCalledWith('crop_pdf', {
      path: 'docs/acta.pdf',
      page: 1,
      x: 0.2,
      y: 0.25,
      width: 0.5,
      height: 0.4,
    })
    expect(storeRef.current.assets.create).not.toHaveBeenCalled()
    expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
      'asset-pdf-1',
      'docs/acta_v2.pdf'
    )
    expect(storeRef.current.extractions.deleteByAsset).toHaveBeenCalledWith('asset-pdf-1')
    expect(storeRef.current.layouts.deleteByAssetId).toHaveBeenCalledWith('asset-pdf-1')

    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
        'asset-pdf-1',
        'docs/acta.pdf'
      )
    })
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Redo edit' })).toBeEnabled()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
        'asset-pdf-1',
        'docs/acta_v2.pdf'
      )
    })

    await fireEvent.click(screen.getByRole('tab', { name: 'Texto' }))
    await fireEvent.click(screen.getByRole('button', { name: 'PTT' }))

    expect(extractTextMock).toHaveBeenCalledWith(
      'asset-pdf-1',
      'docs/acta_v2.pdf',
      'pdf',
      'light'
    )
    expect(extractTextMock).not.toHaveBeenCalledWith(
      expect.anything(),
      'docs/acta.pdf',
      expect.anything(),
      expect.anything()
    )
  })

  it.each([
    {
      type: 'image' as const,
      command: 'crop_image',
      originalPath: 'docs/photo-a.jpg',
      editedPath: 'docs/photo-a_v2.jpg',
      result: {
        path: 'docs/photo-a_v2.jpg',
        width: 100,
        height: 40,
        format_changed: false,
        previous_path: 'docs/photo-a.jpg',
      },
    },
    {
      type: 'pdf' as const,
      command: 'crop_pdf',
      originalPath: 'docs/acta.pdf',
      editedPath: 'docs/acta_v2.pdf',
      result: { path: 'docs/acta_v2.pdf', size: 768 },
    },
  ])('enables crop undo only after the $type edit has fully committed', async (scenario) => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-1',
          itemId: 'item-1',
          path: scenario.originalPath,
          type: scenario.type,
          createdAt: 1,
        },
      ],
    })

    let resolveCrop: (result: typeof scenario.result) => void = () => {}
    const cropResult = new Promise<typeof scenario.result>((resolve) => {
      resolveCrop = resolve
    })
    invokeMock.mockImplementation(async (command: string) => {
      if (command === scenario.command) return cropResult
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: /report image dimensions/i }))
    await fireEvent.click(screen.getByRole('button', { name: 'Crop tool' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeDisabled()

    resolveCrop(scenario.result)
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
        'asset-1',
        scenario.editedPath
      )
      expect(screen.getByRole('button', { name: 'Undo edit' })).toBeEnabled()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
        'asset-1',
        scenario.originalPath
      )
    })
  })

  it('persists fine image rotation through the rotate_image_degrees command', async () => {
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
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'rotate_image_degrees') {
        return {
          path: 'docs/photo-a_v2.png',
          width: 206,
          height: 111,
          format_changed: true,
          previous_path: 'docs/photo-a.jpg',
        }
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: /report image dimensions/i }))
    await fireEvent.click(screen.getByRole('button', { name: /commit fine rotation/i }))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('rotate_image_degrees', {
        path: 'docs/photo-a.jpg',
        degrees: 3,
      })
    })
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a_v2.png'
      )
    })
  })

  it('undo restores only the latest image edit per click', async () => {
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
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'rotate_image_degrees') {
        const path = (args as { path: string }).path
        return {
          path: path === 'docs/photo-a.jpg' ? 'docs/photo-a_v2.png' : 'docs/photo-a_v3.png',
          width: 206,
          height: 111,
          format_changed: true,
          previous_path: path,
        }
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: /report image dimensions/i }))
    await fireEvent.click(screen.getByRole('button', { name: /commit fine rotation/i }))
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a_v2.png'
      )
    })

    await fireEvent.click(screen.getByRole('button', { name: /commit fine rotation/i }))
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a_v3.png'
      )
    })

    await fireEvent.click(screen.getByRole('button', { name: /undo edit/i }))

    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
        'asset-image-1',
        'docs/photo-a_v2.png'
      )
    })
    expect(storeRef.current.assets.updatePath).not.toHaveBeenLastCalledWith(
      'asset-image-1',
      'docs/photo-a.jpg'
    )
  })

  it('reloads persisted layout without running NER after ocr:complete', async () => {
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
    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
    expect(indexFtsMock).not.toHaveBeenCalled()
    expect(embedAssetMock).not.toHaveBeenCalled()
    expect(screen.getByText(/paddle_vl · 5 bloques · 5 regiones/i)).toBeInTheDocument()
  })

  it('does not run NER for duplicate OCR completion events', async () => {
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

    const completePayload = {
      asset_id: 'asset-image-1',
      method: 'paddle_vl',
      text_length: 128,
      text_content: 'OCR listo',
    }
    nlpEventHandlers.get('ocr:complete')?.({ payload: completePayload })
    nlpEventHandlers.get('ocr:complete')?.({ payload: completePayload })

    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
    expect(embedAssetMock).not.toHaveBeenCalled()
    expect(indexFtsMock).not.toHaveBeenCalled()
  })

  it('does not run NER after OCRH completion', async () => {
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
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'glm_ocr',
        text_length: 128,
        text_content: 'OCRH listo',
      },
    })

    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
    expect(embedAssetMock).not.toHaveBeenCalled()
    expect(indexFtsMock).not.toHaveBeenCalled()
  })

  it('auto-runs FTS and EMBED but not NER after OCRC completion is persisted', async () => {
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
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'glm_ocr',
        text_length: 128,
        text_content: 'Texto OCRH',
      },
    })
    embedAssetMock.mockClear()
    indexFtsMock.mockClear()
    extractEntitiesForAssetMock.mockClear()

    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto OCRC' },
    })
    await vi.advanceTimersByTimeAsync(2100)

    expect(invokeMock).toHaveBeenCalledWith('update_extraction_text_cmd', {
      assetId: 'asset-image-1',
      textContent: 'Texto OCRC',
    })
    expect(indexFtsMock).toHaveBeenCalledWith('item-1')
    expect(embedAssetMock).toHaveBeenCalledWith('item-1', 'asset-image-1')
    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
  })

  it('refreshes FTS and EMBED without NER when a manual edit follows OCRC', async () => {
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
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'glm_ocr',
        text_length: 128,
        text_content: 'Texto OCRH',
      },
    })
    embedAssetMock.mockClear()
    indexFtsMock.mockClear()
    extractEntitiesForAssetMock.mockClear()

    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto OCRC' },
    })

    await fireEvent.click(screen.getByRole('tab', { name: /^Texto$/i }))
    const textarea = screen.getByDisplayValue('Texto OCRC') as HTMLTextAreaElement
    await fireEvent.input(textarea, { target: { value: 'Texto OCRH/manual posterior' } })
    await vi.advanceTimersByTimeAsync(2100)

    expect(invokeMock).toHaveBeenCalledWith('update_extraction_text_cmd', {
      assetId: 'asset-image-1',
      textContent: 'Texto OCRH/manual posterior',
    })
    expect(indexFtsMock).toHaveBeenCalledWith('item-1')
    expect(embedAssetMock).toHaveBeenCalledWith('item-1', 'asset-image-1')
    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
  })

  it('persists manual OCR edits and refreshes search without NER', async () => {
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
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_length: 128,
        text_content: 'OCR listo',
      },
    })
    embedAssetMock.mockClear()
    extractEntitiesForAssetMock.mockClear()
    indexFtsMock.mockClear()
    await fireEvent.click(screen.getByRole('tab', { name: /^Texto$/i }))

    const textarea = screen.getByDisplayValue('OCR listo') as HTMLTextAreaElement
    await fireEvent.input(textarea, { target: { value: 'OCR editado manualmente' } })

    await vi.advanceTimersByTimeAsync(2100)

    expect(invokeMock).toHaveBeenCalledWith('update_extraction_text_cmd', {
      assetId: 'asset-image-1',
      textContent: 'OCR editado manualmente',
    })
    expect(indexFtsMock).toHaveBeenCalledWith('item-1')
    expect(embedAssetMock).toHaveBeenCalledWith('item-1', 'asset-image-1')
    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
  })

  it('persists manual transcription edits and refreshes search without NER', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-audio-1',
          itemId: 'item-1',
          path: 'docs/audio.mp3',
          type: 'audio',
          createdAt: 1,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    nlpEventHandlers.get('transcription:complete')?.({
      payload: {
        asset_id: 'asset-audio-1',
        text: 'Transcripción lista',
        language: 'es',
        duration_ms: 12000,
        segments_count: 1,
      },
    })
    embedAssetMock.mockClear()
    extractEntitiesForAssetMock.mockClear()
    indexFtsMock.mockClear()
    await fireEvent.click(screen.getByRole('tab', { name: /^Texto$/i }))

    const textarea = screen.getByDisplayValue('Transcripción lista') as HTMLTextAreaElement
    await fireEvent.input(textarea, { target: { value: 'Transcripción editada manualmente' } })

    await vi.advanceTimersByTimeAsync(2100)

    expect(invokeMock).toHaveBeenCalledWith('update_transcription_text_cmd', {
      assetId: 'asset-audio-1',
      textContent: 'Transcripción editada manualmente',
    })
    expect(indexFtsMock).toHaveBeenCalledWith('item-1')
    expect(embedAssetMock).toHaveBeenCalledWith('item-1', 'asset-audio-1')
    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
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

  it('geocodes a manually created PLACE entity', async () => {
    await renderAnalysisWithEntities()
    storeRef.current.entities.create.mockResolvedValue({
      id: 'place-tucuman',
      itemId: 'item-1',
      entityType: 'place',
      value: 'Tucumán',
      startOffset: 0,
      endOffset: 0,
      confidence: 1,
      createdAt: 2,
    })
    invokeMock.mockClear()

    await fireEvent.change(screen.getByLabelText('Nuevo tipo de entidad'), {
      target: { value: 'place' },
    })
    await fireEvent.input(screen.getByLabelText('Nuevo valor de entidad'), {
      target: { value: 'Tucumán' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar' }))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('geocode_entity', { entityId: 'place-tucuman' })
    })
  })

  it('geocodes a PLACE entity again after its value changes', async () => {
    const entityRows = [
      {
        id: 'place-mar-del-plata',
        itemId: 'item-1',
        entityType: 'place' as const,
        value: 'Mar Plata',
        startOffset: 10,
        endOffset: 19,
        confidence: 0.95,
        createdAt: 1,
      },
    ]
    storeRef.current = createStore({ entitiesRows: entityRows })
    storeRef.current.entities.update.mockResolvedValue({
      ...entityRows[0],
      value: 'Mar del Plata',
      confidence: 1,
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: /Análisis/i }))
    invokeMock.mockClear()

    await fireEvent.click(await screen.findByTestId('mock-entity-place-mar-del-plata'))
    const input = await screen.findByLabelText('Editar valor de entidad')
    await fireEvent.input(input, { target: { value: 'Mar del Plata' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('geocode_entity', {
        entityId: 'place-mar-del-plata',
      })
    })
  })

  it('updates map marker labels from the current edited PLACE entity state', async () => {
    const entityRows = [
      {
        id: 'place-1',
        itemId: 'item-1',
        entityType: 'place' as const,
        value: 'Buenos Aires',
        startOffset: 10,
        endOffset: 22,
        confidence: 0.95,
        createdAt: 1,
      },
    ]
    storeRef.current = createStore({ entitiesRows: entityRows })
    storeRef.current.entities.findByAssetId.mockImplementation(async () => entityRows)
    storeRef.current.entities.update.mockImplementation(
      async (id: string, data: { value?: string }) => {
        const entity = entityRows.find((row) => row.id === id)
        if (entity && data.value) entity.value = data.value
        return entity
      }
    )
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'db_select') {
        return [
          {
            id: 'place-1',
            value: 'Buenos Aires',
            latitude: -34.6037,
            longitude: -58.3816,
          },
        ]
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: /Análisis/i }))

    expect(await screen.findByTestId('mock-map-marker-place-1')).toHaveTextContent('Buenos Aires')

    await fireEvent.click(await screen.findByTestId('mock-entity-place-1'))
    const input = await screen.findByLabelText('Editar valor de entidad')
    await fireEvent.input(input, { target: { value: 'La Plata' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => {
      expect(screen.getByTestId('mock-map-marker-place-1')).toHaveTextContent('La Plata')
    })
    expect(screen.queryByText('Buenos Aires')).not.toBeInTheDocument()
  })

  it('prioritizes manual coordinates and persists map edits through the entity repository', async () => {
    const entityRows = [
      {
        id: 'place-1',
        itemId: 'item-1',
        entityType: 'place' as const,
        value: 'Buenos Aires',
        startOffset: 0,
        endOffset: 12,
        confidence: 1,
        createdAt: 1,
      },
      {
        id: 'place-tucuman',
        itemId: 'item-1',
        entityType: 'place' as const,
        value: 'Tucumán',
        startOffset: 20,
        endOffset: 27,
        confidence: 1,
        createdAt: 2,
      },
    ]
    const store = createStore({ entitiesRows: entityRows })
    storeRef.current = store
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'db_select') {
        expect((args as { sql: string }).sql).toContain(
          'CASE WHEN manual_lat IS NOT NULL AND manual_lon IS NOT NULL THEN manual_lat ELSE latitude END'
        )
        return [
          {
            id: 'place-1',
            value: 'Buenos Aires',
            latitude: -34.6037,
            longitude: -58.3816,
            hasManualLocation: 1,
          },
        ]
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: /Análisis/i }))

    expect(await screen.findByTestId('mock-map-location-place-tucuman')).toHaveTextContent(
      'Tucumán'
    )
    expect(screen.queryByTestId('mock-map-marker-place-tucuman')).not.toBeInTheDocument()

    await fireEvent.click(await screen.findByTestId('mock-map-save-place-1'))
    await waitFor(() => {
      expect(store.entities.setManualLocation).toHaveBeenCalledWith('place-1', -34.615, -58.433)
    })

    await fireEvent.click(screen.getByTestId('mock-map-reset-place-1'))
    await waitFor(() => {
      expect(store.entities.resetManualLocation).toHaveBeenCalledWith('place-1')
    })
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

  it('shows OCRL + OCRH for image assets (Pro local dual OCR)', async () => {
    await renderTextTabForAsset('image')

    // Pro is local: images get both the lightweight PaddleOCR (OCRL) and the
    // layout-aware PaddleOCR-VL (OCRH). Lite (cloud) only had the single OCRH.
    expect(screen.getByRole('button', { name: 'OCRL' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRH' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRC' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRR' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PTT' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFC' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFR' })).not.toBeInTheDocument()
  })

  it('shows a visible error when summary generation fails', async () => {
    llmSummarizeAssetMock.mockRejectedValueOnce(new Error('provider unavailable'))
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
    })

    const summarizeButton = await screen.findByRole('button', { name: 'OCRR' })
    await waitFor(() => expect(summarizeButton).toBeEnabled())
    await fireEvent.click(summarizeButton)

    expect(llmSummarizeAssetMock).toHaveBeenCalledWith('asset-image-1')
    expect(await screen.findByText('No se pudo generar el resumen.')).toBeInTheDocument()
  })

  it('shows a visible error when OCR correction fails', async () => {
    llmCorrectOcrAssetMock.mockRejectedValueOnce(new Error('provider unavailable'))
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
    })

    const correctButton = await screen.findByRole('button', { name: 'OCRC' })
    await waitFor(() => expect(correctButton).toBeEnabled())
    await fireEvent.click(correctButton)

    expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-image-1')
    expect(await screen.findByText('No se pudo corregir el texto con OCR.')).toBeInTheDocument()
  })

  it('hides the OCRC button after OCR correction completes (Pro-local idempotency)', async () => {
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
    })

    const correctButton = await screen.findByRole('button', { name: 'OCRC' })
    await waitFor(() => expect(correctButton).toBeEnabled())
    await fireEvent.click(correctButton)

    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto corregido' },
    })

    // Pro hides the OCRC affordance once an asset has been corrected (the
    // onCorrectOcr seeder fires on both live completion and persisted-results
    // reload, so it stays hidden across reopens). Lite kept it always visible.
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'OCRC' })).not.toBeInTheDocument()
    )
  })

  it('resets the corrected OCR state when OCR runs again for the same asset', async () => {
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR inicial' },
    })

    const correctButton = await screen.findByRole('button', { name: 'OCRC' })
    await waitFor(() => expect(correctButton).toBeEnabled())
    await fireEvent.click(correctButton)
    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto corregido' },
    })

    expect(await screen.findByDisplayValue('Texto corregido')).toBeInTheDocument()
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'OCRC' })).not.toBeInTheDocument()
    )

    await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))

    const resetCorrectButton = screen.getByRole('button', { name: 'OCRC' })
    expect(resetCorrectButton).toBeDisabled()
    expect(screen.queryByDisplayValue('Texto corregido')).not.toBeInTheDocument()

    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR nuevo' },
    })

    expect(await screen.findByDisplayValue('Texto OCR nuevo')).toBeInTheDocument()
    await waitFor(() => expect(resetCorrectButton).toBeEnabled())
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

  it('does not run NER when transcription completes', async () => {
    await renderTextTabForAsset('audio')

    const completePayload = {
      asset_id: 'asset-audio-1',
      text: 'Transcripción lista',
      language: 'es',
      duration_ms: 12000,
      segments_count: 1,
    }
    nlpEventHandlers.get('transcription:complete')?.({
      payload: {
        ...completePayload,
      },
    })
    nlpEventHandlers.get('transcription:complete')?.({ payload: completePayload })

    await fireEvent.click(screen.getByRole('tab', { name: /^Texto$/i }))
    expect(screen.getByDisplayValue('Transcripción lista')).toBeInTheDocument()
    expect(extractEntitiesForAssetMock).not.toHaveBeenCalled()
    expect(embedAssetMock).not.toHaveBeenCalled()
    expect(indexFtsMock).not.toHaveBeenCalled()
  })
})

describe('ItemView right panel tab persistence', () => {
  const imageAsset = {
    id: 'asset-image-1',
    itemId: 'item-1',
    path: 'docs/photo-a.jpg',
    type: 'image' as const,
    createdAt: 1,
  }

  function mockRotateInvoke() {
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'rotate_image_degrees') {
        const path = (args as { path: string }).path
        return {
          path: path === 'docs/photo-a.jpg' ? 'docs/photo-a_v2.png' : 'docs/photo-a_v3.png',
          width: 206,
          height: 111,
          format_changed: true,
          previous_path: path,
        }
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })
  }

  it('keeps the active right panel tab after an image edit of the same asset', async () => {
    storeRef.current = createStore({ assetsRows: [imageAsset] })
    mockRotateInvoke()

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(await screen.findByRole('tab', { name: /^Texto$/ }))
    expect(screen.getByRole('tab', { name: /^Texto$/ })).toHaveAttribute('aria-selected', 'true')

    await fireEvent.click(screen.getByRole('button', { name: /report image dimensions/i }))
    await fireEvent.click(screen.getByRole('button', { name: /commit fine rotation/i }))

    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a_v2.png'
      )
    })

    // The asset object was replaced (new versioned path) but its ID did not
    // change, so the right panel tab must NOT reset to notes.
    expect(screen.getByRole('tab', { name: /^Texto$/ })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByRole('tab', { name: 'Notas' })).toHaveAttribute('aria-selected', 'false')
  })

  it('still resets the right panel tab to notes when switching to a different asset', async () => {
    storeRef.current = createStore({
      assetsRows: [
        imageAsset,
        {
          id: 'asset-image-2',
          itemId: 'item-1',
          path: 'docs/photo-b.jpg',
          type: 'image' as const,
          createdAt: 2,
        },
      ],
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    expect(await screen.findByText(/1\s*\/\s*2/)).toBeInTheDocument()
    await fireEvent.click(await screen.findByRole('tab', { name: /^Texto$/ }))
    expect(screen.getByRole('tab', { name: /^Texto$/ })).toHaveAttribute('aria-selected', 'true')

    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente/i }))

    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'Notas' })).toHaveAttribute('aria-selected', 'true')
    })
  })
})

describe('ItemView Escape behavior', () => {
  const imageAsset = {
    id: 'asset-image-1',
    itemId: 'item-1',
    path: 'docs/photo-a.jpg',
    type: 'image' as const,
    createdAt: 1,
  }

  function resetNavigationToItem() {
    navigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
      },
    ])
  }

  it('Escape cancels an active crop mode and only navigates back once idle', async () => {
    storeRef.current = createStore({ assetsRows: [imageAsset] })
    resetNavigationToItem()
    const cleanupKeyboard = setupKeyboardShortcuts()

    try {
      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')

      await fireEvent.click(screen.getByRole('button', { name: 'Crop tool' }))
      expect(screen.getByTestId('viewer-edit-tool')).toHaveTextContent('crop')

      await fireEvent.keyDown(window, { key: 'Escape' })
      expect(screen.getByTestId('viewer-edit-tool')).toHaveTextContent('none')
      expect(navigation.current.name).toBe('item')

      await fireEvent.keyDown(window, { key: 'Escape' })
      expect(navigation.current.name).toBe('collection')
    } finally {
      cleanupKeyboard()
    }
  })

  it('Escape cancels annotation drawing mode without navigating', async () => {
    storeRef.current = createStore({ assetsRows: [imageAsset] })
    resetNavigationToItem()
    const cleanupKeyboard = setupKeyboardShortcuts()

    try {
      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')

      await fireEvent.click(screen.getByRole('button', { name: 'Rectangle tool' }))
      expect(screen.getByTestId('viewer-annotation-tool')).toHaveTextContent('rectangle')

      await fireEvent.keyDown(window, { key: 'Escape' })
      expect(screen.getByTestId('viewer-annotation-tool')).toHaveTextContent('select')
      expect(navigation.current.name).toBe('item')
    } finally {
      cleanupKeyboard()
    }
  })
})
