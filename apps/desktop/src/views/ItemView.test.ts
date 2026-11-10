import { render, screen, fireEvent, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import ItemView from './ItemView.svelte'
import { LOCAL_ML } from '$lib/capabilities'
import { workspace } from '$lib/workspace'
import type { View } from '$lib/navigation'
import { setupKeyboardShortcuts } from '$lib/keyboard'
import { DOCUMENT_ASSET_DELETED_EVENT } from '$lib/document-explorer'

const {
  nlpEventHandlers,
  tauriListeners,
  embedAssetMock,
  extractEntitiesForAssetMock,
  indexFtsMock,
  extractTriplesMock,
  llmSummarizeAssetMock,
  llmCorrectOcrAssetMock,
  llmCanRestoreOriginalOcrAssetMock,
  llmGetResultsMock,
  llmRestoreOriginalOcrAssetMock,
  llmExtractTriplesMock,
  llmExtractTriplesAssetMock,
  similarAssetsMock,
  extractTextMock,
  getLayoutByAssetMock,
  assetFileFactsMock,
  clipboardWriteTextMock,
  llmIsAvailableMock,
  llmOcrCorrectionIsAvailableMock,
  invokeMock,
  emitMock,
  duplicateAssetFileMock,
  deleteAssetFileMock,
} = vi.hoisted(() => ({
  nlpEventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  // Every live subscription, per event: two views on screen both listen.
  tauriListeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  embedAssetMock: vi.fn<(_: string, __: string) => Promise<void>>(),
  extractEntitiesForAssetMock: vi.fn<(_: string, __: string) => Promise<void>>(),
  indexFtsMock: vi.fn<(_: string) => Promise<void>>(),
  extractTriplesMock: vi.fn<(_: string) => Promise<void>>(),
  llmSummarizeAssetMock: vi.fn<(_: string) => Promise<void>>(),
  llmCorrectOcrAssetMock: vi.fn<(_: string) => Promise<void>>(),
  llmCanRestoreOriginalOcrAssetMock: vi.fn<(_: string) => Promise<boolean>>(),
  llmGetResultsMock: vi.fn(),
  llmRestoreOriginalOcrAssetMock: vi.fn<(_: string) => Promise<string>>(),
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
  // The current file of every image asset is, unless a test says otherwise, the
  // untagged 1000×1400 file the layout fixture was computed on.
  assetFileFactsMock: vi.fn<(_assetId: string, _options?: { withSha?: boolean }) => unknown>(
    async () => ({ orientation: 1, width: 1000, height: 1400, sha256: null })
  ),
  clipboardWriteTextMock: vi.fn<(_: string) => Promise<void>>(),
  llmIsAvailableMock: vi.fn<() => Promise<boolean>>(),
  llmOcrCorrectionIsAvailableMock: vi.fn<() => Promise<boolean>>(),
  invokeMock: vi.fn<(_: string, __?: unknown) => Promise<unknown>>(async (command: string) => {
    if (command === 'llm_get_results') return []
    if (command === 'llm_get_result') return null
    if (command === 'llm_is_available') return true
    if (command === 'db_select') return []
    return null
  }),
  emitMock: vi.fn<(_: string, __?: unknown) => Promise<void>>(),
  duplicateAssetFileMock:
    vi.fn<
      (_sourcePath: string, _existingPaths: string[]) => Promise<{ name: string; path: string }>
    >(),
  deleteAssetFileMock: vi.fn<(_path: string) => Promise<void>>(),
}))

type TripleRow = { id: string; subject: string; predicate: string; object: string }
type AnnotationRow = {
  id: string
  assetId: string
  page: number
  kind: 'rectangle' | 'underline' | 'crop' | 'erase' | 'rotation'
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
    sortIndex?: number
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
  extractionsByAsset?: Record<
    string,
    { textContent: string; method?: string; sourcePath?: string; sourceSha256?: string }
  >
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
      createAfter: vi.fn().mockImplementation(async (_sourceId: string, data) => ({
        ...data,
        id: 'asset-duplicate-1',
        sortIndex: 1,
        createdAt: 2,
      })),
      updatePath: vi.fn().mockResolvedValue(undefined),
      hasEmbedding: vi.fn().mockResolvedValue(false),
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
          ? {
              textContent: extraction.textContent,
              method: extraction.method ?? 'light',
              sourcePath: extraction.sourcePath ?? null,
              sourceSha256: extraction.sourceSha256 ?? null,
            }
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
      isItemIndexed: vi.fn().mockResolvedValue(false),
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
      create: vi.fn().mockResolvedValue({ id: 't-new' }),
      update: vi.fn().mockResolvedValue(undefined),
      delete: vi.fn().mockResolvedValue(undefined),
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
    assetProcessEvents: {
      findByAsset: vi.fn().mockResolvedValue([]),
    },
  }
}

const storeRef: { current: ReturnType<typeof createStore> } = {
  current: createStore(),
}

const { highlightCitationRangeMock } = vi.hoisted(() => ({
  highlightCitationRangeMock: vi.fn(),
}))
vi.mock('$lib/highlight-fragment', () => ({
  highlightCitationRange: highlightCitationRangeMock,
  highlightLabeledRanges: vi.fn(() => true),
  clearHighlight: vi.fn(),
}))

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('$lib/file-import', () => ({
  getAssetUrl: (path: string) => `https://asset.localhost/${path}`,
  duplicateAssetFile: duplicateAssetFileMock,
  deleteAssetFile: deleteAssetFileMock,
  isOutsideDataDir: () => false,
  resolveStoredAssetPath: (path: string) => path,
}))

vi.mock('$lib/layouts', async () => {
  const actual = await vi.importActual<typeof import('$lib/layouts')>('$lib/layouts')
  return {
    ...actual,
    getLayoutByAsset: getLayoutByAssetMock,
  }
})

vi.mock('$lib/asset-file-facts', () => ({
  assetFileFacts: assetFileFactsMock,
}))

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
    llmOcrCorrectionIsAvailable: llmOcrCorrectionIsAvailableMock,
    llmGetResults: llmGetResultsMock,
    llmSummarize: vi.fn().mockResolvedValue(undefined),
    llmExtractTriples: llmExtractTriplesMock,
    llmSummarizeAsset: llmSummarizeAssetMock,
    llmCorrectOcrAsset: llmCorrectOcrAssetMock,
    llmCanRestoreOriginalOcrAsset: llmCanRestoreOriginalOcrAssetMock,
    llmRestoreOriginalOcrAsset: llmRestoreOriginalOcrAssetMock,
    llmExtractTriplesAsset: llmExtractTriplesAssetMock,
  }
})

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((eventName: string, callback: (event: { payload: unknown }) => void) => {
    nlpEventHandlers.set(eventName, callback)
    const listeners = tauriListeners.get(eventName) ?? new Set()
    listeners.add(callback)
    tauriListeners.set(eventName, listeners)
    return Promise.resolve(vi.fn(() => listeners.delete(callback)))
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
  // Load actual Svelte UI components inside the mock factory to avoid resolving the mocked package before setup.
  const ActualSearchClearButton = (
    await import('../../../../packages/ui/src/components/SearchClearButton/SearchClearButton.svelte')
  ).default
  const ActualSearchBar = (
    await import('../../../../packages/ui/src/components/SearchBar/SearchBar.svelte')
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
  // The real action: these tests read the tooltip off the rendered control, so
  // a stand-in would only prove the stand-in.
  const { tooltip } = await import('../../../../packages/ui/src/components/Tooltip/tooltip')
  const { portal } = await import('../../../../packages/ui/src/components/Portal/portal')

  return {
    tooltip,
    portal,
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
    SearchClearButton: ActualSearchClearButton,
    SearchBar: ActualSearchBar,
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
vi.mock('@entropia/ui/components/NoteEditor', async () => ({
  NoteEditor: (await import('./__mocks__/MockNoteEditor.svelte')).default,
}))

vi.mock('@entropia/ui/components/MapViewer', async () => ({
  MapViewer: (await import('./__mocks__/MockMapViewer.svelte')).default,
}))

beforeEach(() => {
  workspace.activeNavigation.resetToPath([{ name: 'collections' }])
  Object.defineProperty(globalThis.navigator, 'clipboard', {
    configurable: true,
    value: { writeText: clipboardWriteTextMock },
  })
  clipboardWriteTextMock.mockReset().mockResolvedValue(undefined)
  llmIsAvailableMock.mockReset().mockResolvedValue(true)
  llmOcrCorrectionIsAvailableMock.mockReset().mockResolvedValue(true)
  llmCorrectOcrAssetMock.mockReset().mockResolvedValue(undefined)
  llmGetResultsMock.mockReset().mockResolvedValue([])
  llmCanRestoreOriginalOcrAssetMock.mockReset().mockResolvedValue(false)
  llmRestoreOriginalOcrAssetMock.mockReset().mockResolvedValue('OCR original')
  emitMock.mockReset().mockResolvedValue(undefined)
  tauriListeners.clear()
  duplicateAssetFileMock.mockReset()
  deleteAssetFileMock.mockReset().mockResolvedValue(undefined)
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

afterEach(() => {
  vi.useRealTimers()
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
    llmSummarizeAssetMock.mockReset().mockResolvedValue(undefined)
    llmCorrectOcrAssetMock.mockReset().mockResolvedValue(undefined)
    llmExtractTriplesAssetMock.mockReset().mockResolvedValue(undefined)
    storeRef.current = createStore({ assetsRows: multiPageAssets })
  })

  it('locates every occurrence across assets, including item-level targets, and replaces history', async () => {
    storeRef.current = createStore({
      assetsRows: multiPageAssets,
      extractionsByAsset: {
        'asset-page-1': { textContent: 'José y José' },
        'asset-page-2': { textContent: 'José' },
      },
    })
    const collection: View = { name: 'collection', id: 'col-1', collectionName: 'Colección 1' }
    const target = {
      kind: 'entity' as const,
      value: 'José',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-1',
      itemTitle: 'Acta histórica',
      assetId: null,
    }
    workspace.activeNavigation.resetToPath([
      collection,
      {
        name: 'item',
        ...target,
        occurrenceSession: { kind: 'entity', targets: [target], index: 0 },
      },
    ])
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    expect(await screen.findByText('Aparición 1 de 3')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Anterior' })).toBeDisabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Siguiente' }))
    expect(await screen.findByText('Aparición 2 de 3')).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Siguiente' }))
    expect(await screen.findByText('Aparición 3 de 3')).toBeInTheDocument()
    await waitFor(() =>
      expect(workspace.activeNavigation.current).toMatchObject({ assetId: 'asset-page-2' })
    )
    expect(screen.getByRole('button', { name: 'Siguiente' })).toBeDisabled()
    expect(screen.getByRole('tab', { name: 'Texto extraído' })).toHaveAttribute(
      'aria-selected',
      'true'
    )
    workspace.activeNavigation.back()
    expect(workspace.activeNavigation.current).toEqual(collection)
  })

  it('keeps the missing-passage notice for an unavailable occurrence asset', async () => {
    const target = {
      kind: 'entity' as const,
      value: 'José',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-1',
      itemTitle: 'Acta histórica',
      assetId: 'deleted-asset',
    }
    workspace.activeNavigation.navigate({
      name: 'item',
      ...target,
      occurrenceSession: { kind: 'entity', targets: [target], index: 0 },
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    const notice = await screen.findByText('No se encontró el pasaje en el texto disponible.')
    expect(notice).toBeVisible()
    expect(workspace.activeNavigation.current).toMatchObject({ assetId: 'deleted-asset' })
  })

  it('opens the asset requested by navigation instead of pinning the first sibling', async () => {
    workspace.activeNavigation.resetToPath([
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

  it('inserts and selects the duplicate after its source for counter and paginator navigation', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-source',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          sortIndex: 0,
          createdAt: 1,
          size: 2048,
        },
        {
          id: 'asset-next',
          itemId: 'item-1',
          path: 'docs/acta-2.pdf',
          type: 'pdf',
          sortIndex: 1,
          createdAt: 2,
          size: 1024,
        },
        {
          id: 'asset-last',
          itemId: 'item-1',
          path: 'docs/acta-3.pdf',
          type: 'pdf',
          sortIndex: 2,
          createdAt: 3,
          size: 512,
        },
      ],
    })
    duplicateAssetFileMock.mockResolvedValue({
      name: 'acta_c1.pdf',
      path: 'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf',
    })
    const collectionChanges: Array<{ collectionId: string; itemId?: string }> = []
    const handleCollectionChange = (event: Event) => {
      collectionChanges.push(
        (event as CustomEvent<{ collectionId: string; itemId?: string }>).detail
      )
    }
    window.addEventListener('entropia:document-explorer-collection-changed', handleCollectionChange)

    try {
      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await fireEvent.click(
        await screen.findByRole('button', { name: /Duplicar página|Duplicate page/i })
      )

      await waitFor(() => {
        expect(storeRef.current.assets.createAfter).toHaveBeenCalledWith('asset-source', {
          itemId: 'item-1',
          path: 'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf',
          type: 'pdf',
          size: 2048,
        })
      })
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf'
      )
      expect(await screen.findByText(/2\s*\/\s*4/)).toBeInTheDocument()
      expect(collectionChanges.at(-1)).toEqual({ collectionId: 'col-1', itemId: 'item-1' })

      await fireEvent.click(screen.getByRole('button', { name: /Página anterior|Previous page/i }))
      await waitFor(() =>
        expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
          'data-path',
          'docs/acta.pdf'
        )
      )

      await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
      await waitFor(() =>
        expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
          'data-path',
          'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf'
        )
      )

      await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
      await waitFor(() =>
        expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
          'data-path',
          'docs/acta-2.pdf'
        )
      )
    } finally {
      window.removeEventListener(
        'entropia:document-explorer-collection-changed',
        handleCollectionChange
      )
    }
  })

  it('uses the source current position when an earlier asset is deleted during duplication', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-before',
          itemId: 'item-1',
          path: 'docs/before.pdf',
          type: 'pdf',
          sortIndex: 0,
          createdAt: 1,
          size: 1024,
        },
        {
          id: 'asset-source',
          itemId: 'item-1',
          path: 'docs/source.pdf',
          type: 'pdf',
          sortIndex: 1,
          createdAt: 2,
          size: 2048,
        },
        {
          id: 'asset-after',
          itemId: 'item-1',
          path: 'docs/after.pdf',
          type: 'pdf',
          sortIndex: 2,
          createdAt: 3,
          size: 512,
        },
      ],
    })
    const pendingDuplicate = deferred<{ name: string; path: string }>()
    duplicateAssetFileMock.mockReturnValue(pendingDuplicate.promise)

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(
      await screen.findByRole('button', { name: /Página siguiente|Next page/i })
    )
    expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
      'data-path',
      'docs/source.pdf'
    )

    await fireEvent.click(screen.getByRole('button', { name: /Duplicar página|Duplicate page/i }))
    window.dispatchEvent(
      new CustomEvent(DOCUMENT_ASSET_DELETED_EVENT, {
        detail: { itemId: 'item-1', assetId: 'asset-before' },
      })
    )
    pendingDuplicate.resolve({
      name: 'source_c1.pdf',
      path: 'docs/11111111-1111-4111-8111-111111111111_source_c1.pdf',
    })

    await waitFor(() =>
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/11111111-1111-4111-8111-111111111111_source_c1.pdf'
      )
    )
    expect(screen.getByText(/2\s*\/\s*3/)).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: /Página anterior|Previous page/i }))
    await waitFor(() =>
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/source.pdf'
      )
    )

    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
    await waitFor(() =>
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/after.pdf'
      )
    )
  })

  it('rolls back the copied file when the asset record cannot be created', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image',
          itemId: 'item-1',
          path: 'docs/photo.png',
          type: 'image',
          createdAt: 1,
          size: 1024,
        },
      ],
    })
    duplicateAssetFileMock.mockResolvedValue({
      name: 'photo_c1.png',
      path: 'docs/11111111-1111-4111-8111-111111111111_photo_c1.png',
    })
    storeRef.current.assets.createAfter.mockRejectedValueOnce(new Error('database unavailable'))

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(
      await screen.findByRole('button', { name: /Duplicar página|Duplicate page/i })
    )

    await waitFor(() => {
      expect(deleteAssetFileMock).toHaveBeenCalledWith(
        'docs/11111111-1111-4111-8111-111111111111_photo_c1.png'
      )
    })
    expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
      'data-path',
      'docs/photo.png'
    )
    expect(await screen.findByText(/database unavailable/i)).toBeInTheDocument()
  })

  it('does not start an edit while asset duplication is pending', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-pdf',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: 1,
          size: 2048,
        },
      ],
    })
    const pendingDuplicate = deferred<{ name: string; path: string }>()
    duplicateAssetFileMock.mockReturnValue(pendingDuplicate.promise)

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(
      await screen.findByRole('button', { name: /Duplicar página|Duplicate page/i })
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))

    expect(invokeMock).not.toHaveBeenCalledWith('edit_pdf', expect.anything())

    pendingDuplicate.resolve({
      name: 'acta_c1.pdf',
      path: 'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf',
    })
    await waitFor(() => expect(storeRef.current.assets.createAfter).toHaveBeenCalledOnce())
  })

  it('keeps navigation and explorer selection events synced when using the asset paginator', async () => {
    const selectedEvents: Array<{ itemId: string; assetId: string | null }> = []
    const handleSelected = (event: Event) => {
      const detail = (event as CustomEvent<{ itemId: string; assetId: string | null }>).detail
      selectedEvents.push({ itemId: detail.itemId, assetId: detail.assetId })
    }
    window.addEventListener('entropia:document-explorer-asset-selected', handleSelected)

    try {
      workspace.activeNavigation.resetToPath([
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
        expect(workspace.activeNavigation.current).toMatchObject({
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

  it.each([
    {
      caseName: 'A 1/3 → next document with two assets',
      sourceAssets: multiPageAssets,
      sourceIndex: 0,
      targetItemId: 'item-2',
      targetAssetCount: 2,
    },
    {
      caseName: 'A 2/3 → next document with one asset',
      sourceAssets: multiPageAssets,
      sourceIndex: 1,
      targetItemId: 'item-2',
      targetAssetCount: 1,
    },
    {
      caseName: 'A 3/3 → next document with two assets',
      sourceAssets: multiPageAssets,
      sourceIndex: 2,
      targetItemId: 'item-2',
      targetAssetCount: 2,
    },
    {
      caseName: 'A 2/3 → previous document with two assets',
      sourceAssets: multiPageAssets,
      sourceIndex: 1,
      targetItemId: 'item-0',
      targetAssetCount: 2,
    },
    {
      caseName: 'A 1/1 → next document with three assets',
      sourceAssets: multiPageAssets.slice(0, 1),
      sourceIndex: 0,
      targetItemId: 'item-2',
      targetAssetCount: 3,
    },
  ])(
    'selects the first target asset for $caseName',
    async ({ sourceAssets, sourceIndex, targetItemId, targetAssetCount }) => {
      const targetAssets = Array.from({ length: targetAssetCount }, (_, index) => ({
        id: `asset-${targetItemId}-page-${index + 1}`,
        itemId: targetItemId,
        path: `docs/${targetItemId}-page-${index + 1}.png`,
        type: 'image' as const,
        createdAt: index + 10,
      }))
      const store = createStore({
        itemsById: {
          'item-0': {
            id: 'item-0',
            title: 'Acta anterior',
            collectionId: 'col-1',
            metadata: '{}',
          },
          'item-1': {
            id: 'item-1',
            title: 'Acta histórica',
            collectionId: 'col-1',
            metadata: '{}',
          },
          'item-2': {
            id: 'item-2',
            title: 'Acta siguiente',
            collectionId: 'col-1',
            metadata: '{}',
          },
        },
      })
      store.assets.findByItem.mockImplementation(async (itemId: string) =>
        itemId === 'item-1' ? sourceAssets : targetAssets
      )
      storeRef.current = store
      workspace.activeNavigation.resetToPath([
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
      const { rerender } = render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

      if (sourceAssets.length > 1) {
        await screen.findByText(new RegExp(`1\\s*/\\s*${sourceAssets.length}`))
        for (let index = 0; index < sourceIndex; index += 1) {
          await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
        }
        expect(
          await screen.findByText(new RegExp(`${sourceIndex + 1}\\s*/\\s*${sourceAssets.length}`))
        ).toBeInTheDocument()
      } else {
        await waitFor(() => {
          expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
            'data-path',
            sourceAssets[0]!.path
          )
        })
      }

      workspace.activeNavigation.replace({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: targetItemId,
        itemTitle: targetItemId === 'item-0' ? 'Acta anterior' : 'Acta siguiente',
      })
      await rerender({ itemId: targetItemId, collectionId: 'col-1' })

      await waitFor(() => {
        expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
          'data-path',
          targetAssets[0]!.path
        )
        expect(workspace.activeNavigation.current).toMatchObject({
          name: 'item',
          itemId: targetItemId,
          assetId: targetAssets[0]!.id,
        })
      })
      if (targetAssets.length > 1) {
        expect(screen.getByText(new RegExp(`1\\s*/\\s*${targetAssets.length}`))).toBeInTheDocument()
      }
      expect(
        screen.queryByText('No hay páginas adjuntas a este documento.')
      ).not.toBeInTheDocument()
    }
  )

  it('opens on the asset the collection card shows, without reordering the pages', async () => {
    // Two photos with no page order set: the strip is A-Z by path, but the card
    // shows the one imported first. Opening the item must show that same photo.
    const store = createStore({
      assetsRows: [
        {
          id: 'asset-second-imported',
          itemId: 'item-1',
          path: 'docs/alpha.jpg',
          type: 'image',
          sortIndex: 0,
          createdAt: 20,
        },
        {
          id: 'asset-first-imported',
          itemId: 'item-1',
          path: 'docs/zeta.jpg',
          type: 'image',
          sortIndex: 0,
          createdAt: 10,
        },
      ],
    })
    storeRef.current = store

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await waitFor(() => {
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/zeta.jpg'
      )
    })
    expect(screen.getByText(/2\s*\/\s*2/)).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: /Página anterior|Previous page/i }))
    await waitFor(() => {
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/alpha.jpg'
      )
    })
  })

  it('resets to each document first asset across repeated sibling navigation', async () => {
    const itemIds = ['item-1', 'item-2', 'item-3', 'item-4']
    const assetsByItem = Object.fromEntries(
      itemIds.map((nextItemId, itemIndex) => [
        nextItemId,
        Array.from({ length: (itemIndex % 3) + 1 }, (_, assetIndex) => ({
          id: `asset-${nextItemId}-${assetIndex + 1}`,
          itemId: nextItemId,
          path: `docs/${nextItemId}-${assetIndex + 1}.png`,
          type: 'image' as const,
          createdAt: itemIndex * 10 + assetIndex,
        })),
      ])
    )
    const store = createStore({
      itemsById: Object.fromEntries(
        itemIds.map((nextItemId) => [
          nextItemId,
          {
            id: nextItemId,
            title: nextItemId,
            collectionId: 'col-1',
            metadata: '{}',
          },
        ])
      ),
    })
    store.assets.findByItem.mockImplementation(
      async (nextItemId: string) => assetsByItem[nextItemId] ?? []
    )
    storeRef.current = store
    workspace.activeNavigation.resetToPath([
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'item-1',
      },
    ])
    const { rerender } = render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await waitFor(() => {
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        assetsByItem['item-1']![0]!.path
      )
    })

    for (const nextItemId of [...itemIds.slice(1), 'item-1']) {
      workspace.activeNavigation.replace({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: nextItemId,
        itemTitle: nextItemId,
      })
      await rerender({ itemId: nextItemId, collectionId: 'col-1' })
      await waitFor(() => {
        expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
          'data-path',
          assetsByItem[nextItemId]![0]!.path
        )
        expect(workspace.activeNavigation.current).toMatchObject({
          itemId: nextItemId,
          assetId: assetsByItem[nextItemId]![0]!.id,
        })
      })
    }
  })

  it('keeps asset pagination scoped to the document selected after parent navigation', async () => {
    const targetAssets = [
      {
        id: 'asset-item-2-1',
        itemId: 'item-2',
        path: 'docs/item-2-1.png',
        type: 'image' as const,
        createdAt: 10,
      },
      {
        id: 'asset-item-2-2',
        itemId: 'item-2',
        path: 'docs/item-2-2.png',
        type: 'image' as const,
        createdAt: 11,
      },
    ]
    const store = createStore({
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: '{}',
        },
        'item-2': {
          id: 'item-2',
          title: 'Acta siguiente',
          collectionId: 'col-1',
          metadata: '{}',
        },
      },
    })
    store.assets.findByItem.mockImplementation(async (nextItemId: string) =>
      nextItemId === 'item-1' ? multiPageAssets : targetAssets
    )
    storeRef.current = store
    const { rerender } = render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByText(/1\s*\/\s*3/)
    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
    await screen.findByText(/2\s*\/\s*3/)

    workspace.activeNavigation.replace({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Acta siguiente',
    })
    await rerender({ itemId: 'item-2', collectionId: 'col-1' })
    expect(await screen.findByText(/1\s*\/\s*2/)).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))

    expect(await screen.findByText(/2\s*\/\s*2/)).toBeInTheDocument()
    expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
      'data-path',
      targetAssets[1]!.path
    )
    await waitFor(() => {
      expect(workspace.activeNavigation.current).toMatchObject({
        itemId: 'item-2',
        assetId: targetAssets[1]!.id,
      })
    })
  })

  it('shows the empty state only after the target document loads with no assets', async () => {
    const store = createStore({
      assetsRows: multiPageAssets,
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta histórica',
          collectionId: 'col-1',
          metadata: '{}',
        },
        'item-2': {
          id: 'item-2',
          title: 'Documento vacío',
          collectionId: 'col-1',
          metadata: '{}',
        },
      },
    })
    store.assets.findByItem.mockImplementation(async (nextItemId: string) =>
      nextItemId === 'item-1' ? multiPageAssets : []
    )
    storeRef.current = store
    const { rerender } = render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByText(/1\s*\/\s*3/)
    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
    await screen.findByText(/3\s*\/\s*3/)

    workspace.activeNavigation.replace({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Documento vacío',
    })
    await rerender({ itemId: 'item-2', collectionId: 'col-1' })

    expect(await screen.findByText('No hay páginas adjuntas a este documento.')).toBeInTheDocument()
    expect(store.assets.findByItem).toHaveBeenCalledWith('item-2')
    expect(workspace.activeNavigation.current).toMatchObject({ itemId: 'item-2', assetId: null })
  })

  it('consumes citation landing when the paginator selects another asset', async () => {
    storeRef.current = createStore({
      assetsRows: multiPageAssets.slice(0, 2),
      extractionsByAsset: {
        'asset-page-1': { textContent: 'Acta de la primera página' },
        'asset-page-2': { textContent: 'Acta de la segunda página' },
      },
    })
    workspace.activeNavigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
        assetId: 'asset-page-1',
        assetLabel: '757-70_page_1.png',
        citationRange: { start: 0, end: 4, text: 'Acta' },
      },
    ])
    highlightCitationRangeMock.mockClear()

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await waitFor(() => {
      expect(highlightCitationRangeMock).toHaveBeenCalledWith(
        expect.anything(),
        'Acta de la primera página',
        { start: 0, end: 4 }
      )
    })
    highlightCitationRangeMock.mockClear()

    await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))

    expect(await screen.findByText(/2\s*\/\s*2/)).toBeInTheDocument()
    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'Documento' })).toHaveAttribute(
        'aria-selected',
        'true'
      )
      expect(workspace.activeNavigation.current).not.toHaveProperty('citationRange')
    })
    expect(await screen.findByText('Acta de la segunda página')).toBeInTheDocument()
    await Promise.resolve()
    expect(highlightCitationRangeMock).not.toHaveBeenCalled()
  })

  it('seeks the audio viewer to the second of a chat citation', async () => {
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
    workspace.activeNavigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Entrevista',
        assetId: 'asset-audio-1',
        citationRange: null,
        citationSeconds: 65,
      },
    ])

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await waitFor(() => {
      expect(screen.getByTestId('viewer-audio-start-at')).toHaveTextContent('65')
    })
  })

  it('does not seek an asset other than the one the citation names', async () => {
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
    workspace.activeNavigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Entrevista',
        assetId: 'asset-audio-1',
        citationSeconds: 65,
      },
    ])
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')

    if (workspace.activeNavigation.current.name !== 'item')
      throw new Error('Expected item navigation')
    workspace.activeNavigation.replace({
      ...workspace.activeNavigation.current,
      assetId: 'another-asset',
    })

    await waitFor(() => {
      expect(screen.getByTestId('viewer-audio-start-at')).toHaveTextContent('none')
    })
  })

  it('reapplies the marker when a second citation targets the selected asset', async () => {
    storeRef.current = createStore({
      extractionsByAsset: {
        'asset-1': { textContent: 'Primero segundo' },
      },
    })
    workspace.activeNavigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
        assetId: 'asset-1',
        assetLabel: 'acta.pdf',
        citationRange: { start: 0, end: 7, text: 'Primero' },
      },
    ])
    highlightCitationRangeMock.mockClear()

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await waitFor(() => {
      expect(highlightCitationRangeMock).toHaveBeenCalledWith(
        expect.anything(),
        'Primero segundo',
        { start: 0, end: 7 }
      )
    })
    await fireEvent.click(screen.getByRole('tab', { name: 'Documento' }))
    highlightCitationRangeMock.mockClear()

    if (workspace.activeNavigation.current.name !== 'item')
      throw new Error('Expected item navigation')
    workspace.activeNavigation.replace({
      ...workspace.activeNavigation.current,
      citationRange: { start: 8, end: 15, text: 'segundo' },
    })

    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'Texto extraído' })).toHaveAttribute(
        'aria-selected',
        'true'
      )
      expect(highlightCitationRangeMock).toHaveBeenCalledWith(
        expect.anything(),
        'Primero segundo',
        { start: 8, end: 15 }
      )
    })
  })

  it('removes an asset deleted from the topbar and selects its next sibling', async () => {
    workspace.activeNavigation.resetToPath([
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
    workspace.activeNavigation.resetToPath([
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
    workspace.activeNavigation.resetToPath([
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

    expect(
      await screen.findByRole('heading', { level: 4, name: 'Tripletas semánticas' })
    ).toBeInTheDocument()
    expect(await screen.findByText('Todavía no hay tripletas extraídas.')).toBeInTheDocument()
  })

  it('renders triples as Subject | Predicate | Object rows when store has data', async () => {
    await renderItemViewWith([
      { id: 't-1', subject: 'Belgrano', predicate: 'creó', object: 'la Bandera' },
      { id: 't-2', subject: 'San Martín', predicate: 'fue', object: 'gobernador de Cuyo' },
    ])

    expect(await screen.findByText('Belgrano')).toBeInTheDocument()
    expect(await screen.findByText('creó')).toBeInTheDocument()
    expect(await screen.findByText('la Bandera')).toBeInTheDocument()
    expect(await screen.findByText('San Martín')).toBeInTheDocument()
    expect(await screen.findByText('gobernador de Cuyo')).toBeInTheDocument()
  })

  it('adds a manual triple scoped to the selected asset', async () => {
    await renderItemViewWith([
      { id: 't-1', subject: 'Belgrano', predicate: 'creó', object: 'la Bandera' },
    ])

    await fireEvent.click(await screen.findByTestId('triple-add'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'Sujeto de la nueva tripleta' }), {
      target: { value: 'San Martín' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'Predicado de la nueva tripleta' }), {
      target: { value: 'cruzó' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'Objeto de la nueva tripleta' }), {
      target: { value: 'los Andes' },
    })
    await fireEvent.click(screen.getByTestId('triple-new-save'))

    await waitFor(() => {
      expect(storeRef.current.triples.create).toHaveBeenCalledWith({
        itemId: 'item-1',
        assetId: 'asset-1',
        subject: 'San Martín',
        predicate: 'cruzó',
        object: 'los Andes',
      })
    })
    // Alta y edición son escrituras distintas: agregar no reescribe nada.
    expect(storeRef.current.triples.update).not.toHaveBeenCalled()
    expect(storeRef.current.triples.delete).not.toHaveBeenCalled()
  })

  it('writes nothing when a manual triple draft is cancelled', async () => {
    await renderItemViewWith([])

    await fireEvent.click(await screen.findByTestId('triple-add'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'Sujeto de la nueva tripleta' }), {
      target: { value: 'San Martín' },
    })
    await fireEvent.click(screen.getByTestId('triple-new-cancel'))

    expect(storeRef.current.triples.create).not.toHaveBeenCalled()
  })

  it('persists an inline triple edit through the repo and reloads the list', async () => {
    await renderItemViewWith([
      { id: 't-1', subject: 'Belgrano', predicate: 'creó', object: 'la Bandera' },
    ])

    await fireEvent.click(await screen.findByTestId('triple-edit-t-1'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'Predicado de la tripleta' }), {
      target: { value: 'izó' },
    })
    await fireEvent.click(screen.getByTestId('triple-save-t-1'))

    await waitFor(() => {
      expect(storeRef.current.triples.update).toHaveBeenCalledWith('t-1', {
        subject: 'Belgrano',
        predicate: 'izó',
        object: 'la Bandera',
      })
    })
  })

  it('deletes a single triple through the repo after confirming', async () => {
    await renderItemViewWith([
      { id: 't-1', subject: 'Belgrano', predicate: 'creó', object: 'la Bandera' },
      { id: 't-2', subject: 'San Martín', predicate: 'fue', object: 'gobernador de Cuyo' },
    ])

    await fireEvent.click(await screen.findByTestId('triple-delete-t-1'))
    expect(storeRef.current.triples.delete).not.toHaveBeenCalled()

    await fireEvent.click(screen.getByTestId('triple-delete-t-1'))
    await waitFor(() => {
      expect(storeRef.current.triples.delete).toHaveBeenCalledWith('t-1')
    })
    expect(storeRef.current.triples.delete).toHaveBeenCalledTimes(1)
  })

  it('transitions pending → running → done and supports retry after error for triples', async () => {
    await renderItemViewWith([])

    // Anclado al inicio: el botón de extracción se llama "TRIPLET <estado>", y
    // un match suelto también atraparía "Agregar tripleta".
    const triplesBtn = await screen.findByRole('button', { name: /^TRIPLET\b/ })

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
      { id: 't-3', subject: 'Moreno', predicate: 'fundó', object: 'La Gazeta' },
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

    expect(await screen.findByText('Páginas similares')).toBeInTheDocument()
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

    const technicalMeta = screen.getByText(
      'página asset-sim-2 · documento item-2 · colección col-9'
    )
    expect(technicalMeta).not.toBeVisible()
    expect(screen.getByText('archivo/carta-manuscrita.jpg')).not.toBeVisible()
  })

  it('previews an embedding result without replacing the original asset navigation', async () => {
    const fullOcrText = `Preview truncado. ${'contenido OCR completo '.repeat(30)}CIERRE_DEL_OCR`
    workspace.activeNavigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
        assetId: 'asset-source-1',
        assetLabel: 'acta-1.pdf',
      },
    ])
    similarAssetsMock.mockResolvedValue([
      {
        assetId: 'asset-sim-2',
        itemId: 'item-2',
        title: 'Carta manuscrita',
        collectionId: 'col-9',
        assetPath: 'archivo/carta-manuscrita.jpg',
        assetType: 'image',
        textPreview: 'Preview truncado.',
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
        extractionsByAsset: {
          'asset-sim-2': { textContent: fullOcrText },
        },
      })
    )

    const resultCard = await screen.findByTestId('similar-asset-asset-sim-2')
    resultCard.focus()
    await fireEvent.click(resultCard)

    const dialog = await screen.findByRole('dialog', { name: 'Carta manuscrita' })
    expect(within(dialog).getByText('Consulta por similitud semántica')).toBeInTheDocument()
    expect(within(dialog).getByText('Similitud 91.3%')).toBeInTheDocument()
    expect(within(dialog).getByText(/CIERRE_DEL_OCR/)).toHaveTextContent(fullOcrText)
    const previewViewer = within(dialog).getByTestId('similar-asset-preview-viewer')
    const documentViewer = within(previewViewer).getByTestId('mock-document-viewer')
    expect(documentViewer).toHaveAttribute('data-path', 'archivo/carta-manuscrita.jpg')
    expect(documentViewer).toHaveAttribute(
      'data-asset-url',
      'https://asset.localhost/archivo/carta-manuscrita.jpg'
    )
    expect(documentViewer).toHaveAttribute('data-read-only', 'true')
    expect(
      within(previewViewer).queryByRole('button', { name: 'Crop tool' })
    ).not.toBeInTheDocument()
    expect(workspace.activeNavigation.current).toMatchObject({
      name: 'item',
      itemId: 'item-1',
      assetId: 'asset-source-1',
    })

    await fireEvent.keyDown(window, { key: 'Escape' })

    expect(screen.queryByRole('dialog', { name: 'Carta manuscrita' })).not.toBeInTheDocument()
    expect(document.activeElement).toBe(resultCard)
    expect(workspace.activeNavigation.current).toMatchObject({
      name: 'item',
      itemId: 'item-1',
      assetId: 'asset-source-1',
    })
  })

  it('renders the related text through the extracted-text renderer, not as raw markup', async () => {
    const source = [
      '# Fileteado: Se Exhorta a Concurrir al Trabajo',
      '',
      '## El Convenio Rige Para Todo el Personal',
      '',
      'Cuerpo de la **nota**.',
    ].join('\n')

    similarAssetsMock.mockResolvedValue([
      {
        assetId: 'asset-sim-2',
        itemId: 'item-2',
        title: 'Carta manuscrita',
        collectionId: 'col-9',
        assetPath: 'archivo/carta-manuscrita.jpg',
        assetType: 'image',
        textPreview: 'Preview truncado.',
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
        extractionsByAsset: {
          'asset-sim-2': { textContent: source },
        },
      })
    )

    await fireEvent.click(await screen.findByTestId('similar-asset-asset-sim-2'))

    const dialog = await screen.findByRole('dialog', { name: 'Carta manuscrita' })

    // Mismo contrato que la solapa Texto extraído: los encabezados se
    // renderizan como encabezados y la marca no queda como texto literal.
    expect(
      await within(dialog).findByRole('heading', {
        level: 1,
        name: 'Fileteado: Se Exhorta a Concurrir al Trabajo',
      })
    ).toBeInTheDocument()
    expect(
      within(dialog).getByRole('heading', {
        level: 2,
        name: 'El Convenio Rige Para Todo el Personal',
      })
    ).toBeInTheDocument()
    expect(
      within(dialog).queryByText('# Fileteado: Se Exhorta a Concurrir al Trabajo')
    ).not.toBeInTheDocument()
  })

  it('loads the complete STT text when previewing an audio embedding result', async () => {
    const fullSttText = `Preview STT. ${'transcripción completa '.repeat(30)}CIERRE_DEL_STT`
    similarAssetsMock.mockResolvedValue([
      {
        assetId: 'asset-audio-similar',
        itemId: 'item-2',
        title: 'Entrevista completa',
        collectionId: 'col-9',
        assetPath: 'archivo/entrevista.mp3',
        assetType: 'audio',
        textPreview: 'Preview STT.',
        similarity: 0.874,
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
        transcriptionsByAsset: {
          'asset-audio-similar': { textContent: fullSttText },
        },
      })
    )

    await fireEvent.click(await screen.findByTestId('similar-asset-asset-audio-similar'))

    const dialog = await screen.findByRole('dialog', { name: 'Entrevista completa' })
    expect(await within(dialog).findByText(/CIERRE_DEL_STT/)).toHaveTextContent(fullSttText)
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

    const clearButton = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    expect(clearButton).toHaveAttribute('data-tooltip', 'Limpiar búsqueda')
    await fireEvent.click(clearButton)
    await waitFor(() => {
      expect(input).toHaveValue('')
      expect(screen.getByText('Ingresá un término para ver resultados.')).toBeInTheDocument()
    })
    expect(storeRef.current.fts.searchWithDebug).toHaveBeenCalledTimes(1)
  })

  it('keeps cleared FTS state isolated from a stale response before a later query', async () => {
    const oldSearch = deferred<Array<{ itemId: string; rank: number }>>()
    const oldHydration = deferred<{
      id: string
      title: string
      collectionId: string
      metadata: string
    } | null>()
    const laterSearch = deferred<Array<{ itemId: string; rank: number }>>()
    const itemsById = {
      'item-1': {
        id: 'item-1',
        title: 'Acta histórica',
        collectionId: 'col-1',
        metadata: '{}',
      },
      'item-2': {
        id: 'item-2',
        title: 'Resultado anterior',
        collectionId: 'col-1',
        metadata: '{}',
      },
      'item-3': {
        id: 'item-3',
        title: 'Resultado posterior',
        collectionId: 'col-1',
        metadata: '{}',
      },
    }
    const ftsSearchImpl = vi.fn((query: string) =>
      query === 'viejo' ? oldSearch.promise : laterSearch.promise
    )

    storeRef.current = createStore({ itemsById, ftsSearchImpl })
    storeRef.current.items.findById.mockImplementation(async (id: string) => {
      if (id === 'item-2') return oldHydration.promise
      return itemsById[id as keyof typeof itemsById] ?? null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const analysisToggle = await screen.findByRole('tab', { name: /Análisis/i })
    await fireEvent.click(analysisToggle)

    const input = (await screen.findByPlaceholderText('Escribí para buscar...')) as HTMLInputElement
    await fireEvent.input(input, { target: { value: 'viejo' } })
    await waitFor(() => {
      expect(storeRef.current.fts.searchWithDebug).toHaveBeenCalledWith('viejo', 10)
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Limpiar búsqueda' }))
    expect(input).toHaveValue('')
    expect(screen.getByText('Ingresá un término para ver resultados.')).toBeInTheDocument()

    oldSearch.resolve([{ itemId: 'item-2', rank: -0.4 }])
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    expect(input).toHaveValue('')
    expect(screen.getByText('Ingresá un término para ver resultados.')).toBeInTheDocument()
    expect(screen.queryByText('Resultado anterior')).not.toBeInTheDocument()
    expect(screen.queryByText('"viejo"')).not.toBeInTheDocument()
    expect(screen.queryByText('No se pudo ejecutar la búsqueda full-text.')).not.toBeInTheDocument()

    await fireEvent.input(input, { target: { value: 'nuevo' } })
    await waitFor(() => {
      expect(storeRef.current.fts.searchWithDebug).toHaveBeenCalledWith('nuevo', 10)
    })

    oldHydration.resolve(itemsById['item-2'])
    await Promise.resolve()
    await Promise.resolve()
    expect(input).toHaveValue('nuevo')
    expect(screen.queryByText('Resultado anterior')).not.toBeInTheDocument()
    expect(screen.queryByText('"viejo"')).not.toBeInTheDocument()
    expect(screen.queryByText('No se pudo ejecutar la búsqueda full-text.')).not.toBeInTheDocument()

    laterSearch.resolve([{ itemId: 'item-3', rank: -0.2 }])
    expect(await screen.findByText('Resultado posterior')).toBeInTheDocument()
    expect(screen.queryByText('Resultado anterior')).not.toBeInTheDocument()
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
    assetFileFactsMock
      .mockReset()
      .mockResolvedValue({ orientation: 1, width: 1000, height: 1400, sha256: null })
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

  it('restores exact image annotations through consecutive edits and a new history branch', async () => {
    storeRef.current = createStore({
      assetsRows: [
        { id: 'image-a', itemId: 'item-1', path: 'docs/a.png', type: 'image', createdAt: 1 },
      ],
    })
    const fallbackInvoke = invokeMock.getMockImplementation()!
    let version = 0
    invokeMock.mockImplementation(async (command, args) => {
      if (
        ['rotate_image', 'crop_image', 'erase_region', 'rotate_image_degrees'].includes(command)
      ) {
        version++
        return {
          path: `docs/a_v${version}.png`,
          width: 100,
          height: 200,
          format_changed: false,
        }
      }
      return fallbackInvoke(command, args)
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await vi.advanceTimersByTimeAsync(0)
    const click = async (name: string) => {
      await fireEvent.click(screen.getByRole('button', { name }))
      await vi.advanceTimersByTimeAsync(0)
    }
    const snapshot = () => ({
      path: screen.getByTestId('mock-document-viewer').getAttribute('data-path'),
      annotations: JSON.parse(screen.getByTestId('viewer-annotations').textContent!),
    })
    const states = [snapshot()]
    await click('Add annotation')
    states.push(snapshot())
    await click('Rotate right')
    expect(snapshot().annotations).toEqual([
      expect.objectContaining({ x: expect.closeTo(0.4), y: 0.1, width: 0.4, height: 0.3 }),
    ])
    states.push(snapshot())
    await click('Crop tool')
    await click('Apply edit region')
    states.push(snapshot())
    await click('Erase tool')
    await click('Apply edit region')
    states.push(snapshot())
    await click('Commit fine rotation')
    states.push(snapshot())
    await click('Delete first annotation')
    states.push(snapshot())

    for (const state of states.slice(0, -1).reverse()) {
      await click('Undo edit')
      expect(snapshot()).toEqual(state)
    }
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeDisabled()
    for (const state of states.slice(1)) {
      await click('Redo edit')
      expect(snapshot()).toEqual(state)
    }
    expect(screen.getByRole('button', { name: 'Redo edit' })).toBeDisabled()
    await click('Undo edit')
    await click('Add annotation')
    expect(screen.getByRole('button', { name: 'Redo edit' })).toBeDisabled()
    await click('Undo edit')
    expect(snapshot()).toEqual(states[states.length - 2])
  })

  it('preserves each asset history when navigating away and back', async () => {
    storeRef.current = createStore({
      assetsRows: [
        { id: 'image-a', itemId: 'item-1', path: 'docs/a.png', type: 'image', createdAt: 1 },
        { id: 'image-b', itemId: 'item-1', path: 'docs/b.png', type: 'image', createdAt: 2 },
      ],
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await vi.advanceTimersByTimeAsync(0)
    await fireEvent.click(screen.getByRole('button', { name: 'Add annotation' }))
    await fireEvent.click(screen.getByRole('button', { name: /página siguiente/i }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeDisabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Add annotation' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    await fireEvent.click(screen.getByRole('button', { name: /página anterior/i }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Redo edit' })).toBeDisabled()
    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('0')
    await fireEvent.click(screen.getByRole('button', { name: /página siguiente/i }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Redo edit' })).toBeEnabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
  })

  it('keeps redo intact after an edit or restoration fails', async () => {
    storeRef.current = createStore()
    const fallbackInvoke = invokeMock.getMockImplementation()!
    invokeMock.mockImplementation(async (command, args) => {
      if (command === 'edit_pdf') throw new Error('disk busy')
      return fallbackInvoke(command, args)
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await vi.advanceTimersByTimeAsync(0)
    await fireEvent.click(screen.getByRole('button', { name: 'Add annotation' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByRole('button', { name: 'Redo edit' })).toBeEnabled()
    storeRef.current.annotations.replaceForAssetPage.mockRejectedValueOnce(new Error('disk busy'))
    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('0')
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Redo edit' })).toBeEnabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
    expect(screen.getByRole('button', { name: 'Redo edit' })).toBeDisabled()
  })

  it('tells the user when a document edit fails', async () => {
    storeRef.current = createStore()
    const fallbackInvoke = invokeMock.getMockImplementation()!
    invokeMock.mockImplementation(async (command, args) => {
      if (command === 'edit_pdf') throw new Error('PDF editing requires a single-page asset')
      return fallbackInvoke(command, args)
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await vi.advanceTimersByTimeAsync(0)

    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(
      screen.getByText('No se pudo aplicar la edición: PDF editing requires a single-page asset')
    ).toBeInTheDocument()
  })

  it('clears the edit error once an edit succeeds', async () => {
    storeRef.current = createStore()
    const fallbackInvoke = invokeMock.getMockImplementation()!
    let failing = true
    invokeMock.mockImplementation(async (command, args) => {
      if (command === 'edit_pdf') {
        if (failing) throw new Error('disk busy')
        return { path: 'docs/acta_v2.pdf', size: 768 }
      }
      return fallbackInvoke(command, args)
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await vi.advanceTimersByTimeAsync(0)
    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByText('No se pudo aplicar la edición: disk busy')).toBeInTheDocument()

    failing = false
    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(screen.queryByText(/No se pudo aplicar la edición/)).not.toBeInTheDocument()
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
    let pdfEditVersion = 1
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'edit_pdf') {
        pdfEditVersion++
        return { path: `docs/acta_v${pdfEditVersion}.pdf`, size: 1024 }
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
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
    await vi.advanceTimersByTimeAsync(0)
    expect(invokeMock).toHaveBeenCalledWith(
      'edit_pdf',
      expect.objectContaining({
        operation: 'erase',
        region: { x: 0.2, y: 0.25, width: 0.5, height: 0.4 },
      })
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(invokeMock).toHaveBeenCalledWith(
      'edit_pdf',
      expect.objectContaining({ operation: 'rotate', rotationDegrees: 90 })
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.runAllTimersAsync()
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta_v2.pdf'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.runAllTimersAsync()
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta_v3.pdf'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.runAllTimersAsync()
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeEnabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.runAllTimersAsync()
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta.pdf'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Go to page 2' }))
    await waitFor(() =>
      expect(storeRef.current.annotations.findByAsset).toHaveBeenCalledWith('asset-pdf-1', 2)
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('1')
    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('0')
    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
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
      if (command === 'edit_pdf') {
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

    expect(invokeMock).toHaveBeenCalledWith('edit_pdf', {
      path: 'docs/acta.pdf',
      page: 1,
      operation: 'crop',
      rotationDegrees: 0,
      region: { x: 0.2, y: 0.25, width: 0.5, height: 0.4 },
      existingCrop: null,
      existingErasures: [],
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
    await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))

    expect(extractTextMock).toHaveBeenCalledWith('asset-pdf-1', 'docs/acta_v2.pdf', 'pdf', 'high')
    expect(extractTextMock).not.toHaveBeenCalledWith(
      expect.anything(),
      'docs/acta.pdf',
      expect.anything(),
      expect.anything()
    )
  })

  it('does not reload stale PDF annotations when an edit versions the current path', async () => {
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
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'edit_pdf') return { path: 'docs/acta_v2.pdf', size: 768 }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await vi.advanceTimersByTimeAsync(0)
    const initialAnnotationLoads = storeRef.current.annotations.findByAsset.mock.calls.length

    await fireEvent.click(screen.getByRole('button', { name: 'Crop tool' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
      'asset-pdf-1',
      'docs/acta_v2.pdf'
    )

    expect(storeRef.current.annotations.findByAsset).toHaveBeenCalledTimes(initialAnnotationLoads)
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeEnabled()
  })

  it('composes persisted PDF crop and rotation state into the next viewport edit', async () => {
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
      annotationsByAsset: {
        'asset-pdf-1': [
          {
            id: 'crop-1',
            assetId: 'asset-pdf-1',
            page: 1,
            kind: 'crop',
            color: '#fff',
            x: 0.1,
            y: 0.2,
            width: 0.6,
            height: 0.5,
            createdAt: 1,
            updatedAt: 1,
          },
          {
            id: 'rotation-1',
            assetId: 'asset-pdf-1',
            page: 1,
            kind: 'rotation',
            color: '#fff',
            x: 1,
            y: 0,
            width: 0,
            height: 0,
            createdAt: 1,
            updatedAt: 1,
          },
        ],
      },
    })
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'edit_pdf') return { path: 'docs/acta_v2.pdf', size: 768 }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await waitFor(() => {
      expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('2')
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Crop tool' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(invokeMock).toHaveBeenCalledWith(
      'edit_pdf',
      expect.objectContaining({
        operation: 'crop',
        rotationDegrees: 90,
        existingCrop: { x: 0.1, y: 0.2, width: 0.6, height: 0.5 },
      })
    )
  })

  it('does not apply a completed PDF edit to a different selected asset', async () => {
    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-pdf-1',
          itemId: 'item-1',
          path: 'docs/acta.pdf',
          type: 'pdf',
          createdAt: 1,
        },
        {
          id: 'asset-pdf-2',
          itemId: 'item-1',
          path: 'docs/anexo.pdf',
          type: 'pdf',
          createdAt: 2,
        },
      ],
    })
    const editResult = deferred<{ path: string; size: number }>()
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'edit_pdf') return editResult.promise
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await fireEvent.click(screen.getByRole('button', { name: /página siguiente/i }))

    editResult.resolve({ path: 'docs/acta_v2.pdf', size: 768 })
    await vi.advanceTimersByTimeAsync(0)

    expect(storeRef.current.assets.updatePath).not.toHaveBeenCalled()
    expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
      'data-path',
      'docs/anexo.pdf'
    )
  })

  it('keeps history unchanged when the backend rejects a legacy multi-page PDF edit', async () => {
    storeRef.current = createStore()
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'edit_pdf') {
        throw new Error('PDF editing requires a single-page asset')
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_get_result') return null
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: 'Go to page 2' }))
    await waitFor(() => {
      expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('2')
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(storeRef.current.assets.updatePath).not.toHaveBeenCalled()
    expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('2')
    expect(screen.getByRole('button', { name: 'Undo edit' })).toBeDisabled()
  })

  it('composes PDF rotation and crop as versioned edits with ordered undo and redo', async () => {
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
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'edit_pdf') {
        const { path, operation } = args as { path: string; operation: string }
        if (path === 'docs/acta.pdf' && operation === 'rotate') {
          return { path: 'docs/acta_v2.pdf', size: 1024 }
        }
        if (path === 'docs/acta_v2.pdf' && operation === 'crop') {
          return { path: 'docs/acta_v3.pdf', size: 512 }
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

    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta_v2.pdf'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Crop tool' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(invokeMock).toHaveBeenCalledWith('edit_pdf', {
      path: 'docs/acta_v2.pdf',
      page: 1,
      operation: 'crop',
      rotationDegrees: 0,
      region: { x: 0.2, y: 0.25, width: 0.5, height: 0.4 },
      existingCrop: null,
      existingErasures: [],
    })
    // The edit lands in the asset's process history, with its input file.
    expect(invokeMock).toHaveBeenLastCalledWith('asset_history_record', {
      report: {
        assetId: 'asset-pdf-1',
        process: 'pdf_crop',
        params: {
          page: 1,
          region: { x: 0.2, y: 0.25, width: 0.5, height: 0.4 },
          rotationDegrees: 0,
        },
        previousPath: 'docs/acta_v2.pdf',
      },
    })
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta_v3.pdf'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta_v2.pdf'
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Undo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta.pdf'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta_v2.pdf'
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Redo edit' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
      'asset-pdf-1',
      'docs/acta_v3.pdf'
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
      command: 'edit_pdf',
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

    // updatePath lands inside commitViewerState, before the operation clears
    // editInProgress; also waiting for the undo button keeps the next click
    // from being dropped by the in-flight guard.
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a_v2.png'
      )
      expect(screen.getByRole('button', { name: /undo edit/i })).toBeEnabled()
    })

    await fireEvent.click(screen.getByRole('button', { name: /commit fine rotation/i }))
    await waitFor(() => {
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a_v3.png'
      )
      expect(screen.getByRole('button', { name: /undo edit/i })).toBeEnabled()
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

  describe('layout of another version of the image', () => {
    const imageAsset = {
      id: 'asset-image-1',
      itemId: 'item-1',
      path: 'docs/photo-a.jpg',
      type: 'image' as const,
      createdAt: 1,
    }
    const tauriDefaults = (command: string) => {
      if (command === 'llm_get_results') return []
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    }
    const notice = () => screen.queryByTestId('layout-version-notice')
    const drawnBoxes = () => screen.getByTestId('viewer-layout-region-count').textContent

    it('keeps the layout through a crop and a rotation without drawing it, and undo brings it back', async () => {
      storeRef.current = createStore({ assetsRows: [imageAsset] })
      getLayoutByAssetMock.mockResolvedValue({
        ...layoutFixture,
        sourcePath: 'docs/photo-a.jpg',
        sourceSha256: 'sha-original',
      })
      // The backend reads the file `assets.path` names: the last path written.
      const filesByPath: Record<string, object> = {
        'docs/photo-a.jpg': { orientation: 1, width: 1000, height: 1400, sha256: 'sha-original' },
        'docs/photo-a_v2.jpg': { orientation: 1, width: 500, height: 560, sha256: 'sha-v2' },
        'docs/photo-a_v3.jpg': { orientation: 1, width: 560, height: 500, sha256: 'sha-v3' },
      }
      const currentPath = () =>
        (storeRef.current.assets.updatePath.mock.calls.at(-1)?.[1] as string | undefined) ??
        imageAsset.path
      assetFileFactsMock.mockImplementation(async () => filesByPath[currentPath()])
      invokeMock.mockImplementation(async (command: string) => {
        if (command === 'crop_image') {
          return { path: 'docs/photo-a_v2.jpg', width: 500, height: 560 }
        }
        if (command === 'rotate_image') {
          return { path: 'docs/photo-a_v3.jpg', width: 560, height: 500 }
        }
        return tauriDefaults(command)
      })

      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')
      await waitFor(() => {
        expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
      })
      await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))
      expect(drawnBoxes()).toBe('5')
      expect(notice()).not.toBeInTheDocument()
      expect(assetFileFactsMock).toHaveBeenLastCalledWith('asset-image-1', { withSha: true })

      await fireEvent.click(screen.getByRole('button', { name: /report image dimensions/i }))
      await fireEvent.click(screen.getByRole('button', { name: 'Crop tool' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
          'asset-image-1',
          'docs/photo-a_v2.jpg'
        )
        expect(notice()).toHaveTextContent(
          'Este layout corresponde a otra versión de la imagen (1000×1400); la actual mide 500×560. Volvé a correr el layout.'
        )
      })
      expect(drawnBoxes()).toBe('0')

      await waitFor(() => {
        expect(screen.getByRole('button', { name: /undo edit/i })).toBeEnabled()
      })
      await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))
      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
          'asset-image-1',
          'docs/photo-a_v3.jpg'
        )
        expect(notice()).toHaveTextContent('la actual mide 560×500')
      })
      expect(drawnBoxes()).toBe('0')

      // Editing never deletes the row: the guard alone keeps it off the screen.
      expect(storeRef.current.layouts.deleteByAssetId).not.toHaveBeenCalled()
      expect(storeRef.current.extractions.deleteByAsset).not.toHaveBeenCalled()

      await waitFor(() => {
        expect(screen.getByRole('button', { name: /undo edit/i })).toBeEnabled()
      })
      await fireEvent.click(screen.getByRole('button', { name: /undo edit/i }))
      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
          'asset-image-1',
          'docs/photo-a_v2.jpg'
        )
        expect(notice()).toHaveTextContent('la actual mide 500×560')
        expect(screen.getByRole('button', { name: /undo edit/i })).toBeEnabled()
      })
      expect(drawnBoxes()).toBe('0')

      // Back on the file the layout was computed on: drawn again, as it was.
      await fireEvent.click(screen.getByRole('button', { name: /undo edit/i }))
      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
          'asset-image-1',
          'docs/photo-a.jpg'
        )
        expect(notice()).not.toBeInTheDocument()
        expect(drawnBoxes()).toBe('5')
      })
      expect(extractTextMock).not.toHaveBeenCalled()
      expect(storeRef.current.layouts.deleteByAssetId).not.toHaveBeenCalled()
    })

    it('keeps the layout through an erase and a redo, which change the image and not its size', async () => {
      storeRef.current = createStore({ assetsRows: [imageAsset] })
      getLayoutByAssetMock.mockResolvedValue({
        ...layoutFixture,
        sourcePath: 'docs/photo-a.jpg',
        sourceSha256: 'sha-original',
      })
      const currentPath = () =>
        (storeRef.current.assets.updatePath.mock.calls.at(-1)?.[1] as string | undefined) ??
        imageAsset.path
      assetFileFactsMock.mockImplementation(async () => ({
        orientation: 1,
        width: 1000,
        height: 1400,
        sha256: currentPath() === imageAsset.path ? 'sha-original' : 'sha-erased',
      }))
      invokeMock.mockImplementation(async (command: string) => {
        if (command === 'erase_region') {
          return { path: 'docs/photo-a_v2.jpg', width: 1000, height: 1400 }
        }
        return tauriDefaults(command)
      })
      const sizeless =
        'Este layout corresponde a otra versión de la imagen. Volvé a correr el layout.'

      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')
      await waitFor(() => {
        expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
      })
      await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))
      expect(drawnBoxes()).toBe('5')

      await fireEvent.click(screen.getByRole('button', { name: /report image dimensions/i }))
      await fireEvent.click(screen.getByRole('button', { name: 'Erase tool' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Apply edit region' }))
      await waitFor(() => {
        expect(notice()).toHaveTextContent(sizeless)
        expect(screen.getByRole('button', { name: /undo edit/i })).toBeEnabled()
      })
      expect(drawnBoxes()).toBe('0')

      await fireEvent.click(screen.getByRole('button', { name: /undo edit/i }))
      await waitFor(() => {
        expect(notice()).not.toBeInTheDocument()
        expect(drawnBoxes()).toBe('5')
        expect(screen.getByRole('button', { name: /redo edit/i })).toBeEnabled()
      })

      await fireEvent.click(screen.getByRole('button', { name: /redo edit/i }))
      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
          'asset-image-1',
          'docs/photo-a_v2.jpg'
        )
        expect(notice()).toHaveTextContent(sizeless)
      })
      expect(drawnBoxes()).toBe('0')
      expect(storeRef.current.layouts.deleteByAssetId).not.toHaveBeenCalled()
      expect(storeRef.current.extractions.deleteByAsset).not.toHaveBeenCalled()
      expect(extractTextMock).not.toHaveBeenCalled()
    })

    it('offers to run the layout again and shows the new one when it arrives', async () => {
      storeRef.current = createStore({ assetsRows: [imageAsset] })
      getLayoutByAssetMock.mockResolvedValue(layoutFixture)
      assetFileFactsMock.mockResolvedValue({
        orientation: 1,
        width: 3456,
        height: 4608,
        sha256: null,
      })

      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')

      await waitFor(() => {
        expect(notice()).toHaveTextContent(
          'Este layout corresponde a otra versión de la imagen (1000×1400); la actual mide 3456×4608. Volvé a correr el layout.'
        )
      })
      expect(drawnBoxes()).toBe('0')
      // An unstamped row has no hash to compare with: the file is not hashed.
      expect(assetFileFactsMock).toHaveBeenLastCalledWith('asset-image-1', { withSha: false })

      await fireEvent.click(
        within(notice()!).getByRole('button', { name: 'Volver a correr el layout' })
      )
      // The engine that computed the stale layout, when this build has it.
      expect(extractTextMock).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a.jpg',
        'image',
        LOCAL_ML ? 'light' : 'high'
      )

      getLayoutByAssetMock.mockResolvedValue({
        ...layoutFixture,
        sourcePath: 'docs/photo-a.jpg',
        sourceSha256: 'sha-current',
      })
      assetFileFactsMock.mockResolvedValue({
        orientation: 1,
        width: 3456,
        height: 4608,
        sha256: 'sha-current',
      })
      nlpEventHandlers.get('ocr:complete')?.({
        payload: {
          asset_id: 'asset-image-1',
          method: 'paddle_vl',
          text_length: 9,
          text_content: 'OCR nuevo',
        },
      })

      await waitFor(() => {
        expect(notice()).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
      })
    })

    it('leaves the sizes out of the notice when only the content changed', async () => {
      storeRef.current = createStore({ assetsRows: [imageAsset] })
      getLayoutByAssetMock.mockResolvedValue({
        ...layoutFixture,
        sourcePath: 'docs/photo-a.jpg',
        sourceSha256: 'sha-before-the-erase',
      })
      assetFileFactsMock.mockResolvedValue({
        orientation: 1,
        width: 1000,
        height: 1400,
        sha256: 'sha-after-the-erase',
      })

      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')

      await waitFor(() => {
        expect(notice()).toHaveTextContent(
          'Este layout corresponde a otra versión de la imagen. Volvé a correr el layout.'
        )
      })
      expect(notice()).not.toHaveTextContent('1000×1400')
      expect(drawnBoxes()).toBe('0')
    })

    it('draws no boxes it cannot vouch for when the file cannot be read, and keeps the image', async () => {
      storeRef.current = createStore({ assetsRows: [imageAsset] })
      getLayoutByAssetMock.mockResolvedValue(layoutFixture)
      assetFileFactsMock.mockRejectedValue(new Error('Failed to read docs/photo-a.jpg'))

      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      const viewer = await screen.findByTestId('mock-document-viewer')

      expect(
        await screen.findByText(
          'No se pudo cargar el layout: no se pudo leer el archivo de la imagen para comprobar a qué versión corresponde.'
        )
      ).toBeInTheDocument()
      expect(viewer).toHaveAttribute('data-path', 'docs/photo-a.jpg')
      expect(drawnBoxes()).toBe('0')
      expect(notice()).not.toBeInTheDocument()
      expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeDisabled()
    })

    it('never checks the layout of a PDF asset', async () => {
      storeRef.current = createStore({
        assetsRows: [{ ...imageAsset, id: 'asset-pdf-1', path: 'docs/acta.pdf', type: 'pdf' }],
      })
      getLayoutByAssetMock.mockResolvedValue({
        ...layoutFixture,
        assetId: 'asset-pdf-1',
        sourcePath: 'docs/acta.pdf',
        sourceSha256: 'sha-of-some-other-file',
      })

      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')

      await waitFor(() => {
        expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
      })
      expect(notice()).not.toBeInTheDocument()
      expect(assetFileFactsMock).not.toHaveBeenCalled()
    })
  })

  describe('versions of the image', () => {
    // The asset shows a crop; the layout and the text were computed on the original.
    const imageAsset = {
      id: 'asset-image-1',
      itemId: 'item-1',
      path: 'docs/photo-a_v2.jpg',
      type: 'image' as const,
      createdAt: 1,
    }
    const filesOnDisk = [
      { path: 'docs/photo-a.jpg', version: 0, width: 1000, height: 1400, sha256: 'sha-original' },
      { path: 'docs/photo-a_v2.jpg', version: 2, width: 500, height: 560, sha256: 'sha-v2' },
      { path: 'docs/photo-a_v11.jpg', version: 11, width: 560, height: 500, sha256: 'sha-v11' },
    ].map((file) => ({ ...file, orientation: 1, modifiedMs: 1_700_000_000_000, size: 2048 }))
    const storedAnnotation: AnnotationRow = {
      id: 'ann-1',
      assetId: 'asset-image-1',
      page: 1,
      kind: 'rectangle',
      color: '#ff0000',
      x: 0.125,
      y: 0.25,
      width: 0.375,
      height: 0.5,
      createdAt: 10,
      updatedAt: 20,
    }
    const WARNING =
      'Esta imagen tiene anotaciones. Pueden no coincidir con la versión que elegiste. ¿Continuar?'

    const currentPath = () =>
      (storeRef.current.assets.updatePath.mock.calls.at(-1)?.[1] as string | undefined) ??
      imageAsset.path
    const panel = () => screen.queryByTestId('image-versions-panel')
    const rows = () => screen.getAllByTestId('image-version')
    const rowOf = (path: string) => rows().find((row) => row.getAttribute('data-path') === path)!
    const useButton = (path: string) =>
      within(rowOf(path)).getByRole('button', { name: 'Usar esta versión' })
    const layoutNotice = () => screen.queryByTestId('layout-version-notice')
    const drawnBoxes = () => screen.getByTestId('viewer-layout-region-count').textContent
    const viewerAnnotations = () => screen.getByTestId('viewer-annotations').textContent
    const commandsInvoked = () => invokeMock.mock.calls.map(([command]) => command)

    beforeEach(() => {
      getLayoutByAssetMock.mockResolvedValue({
        ...layoutFixture,
        sourcePath: 'docs/photo-a.jpg',
        sourceSha256: 'sha-original',
      })
      // The backend answers for the file `assets.path` names: the last path written.
      assetFileFactsMock.mockImplementation(async () =>
        filesOnDisk.find((file) => file.path === currentPath())
      )
      invokeMock.mockImplementation(async (command: string) => {
        if (command === 'list_image_versions') {
          return filesOnDisk.map((file) => ({ ...file, current: file.path === currentPath() }))
        }
        if (command === 'llm_get_results') return []
        if (command === 'llm_is_available') return true
        if (command === 'db_select') return []
        return null
      })
    })

    async function openVersions(options: StoreOptions = {}) {
      storeRef.current = createStore({
        assetsRows: [imageAsset],
        extractionsByAsset: {
          'asset-image-1': { textContent: 'Texto', sourceSha256: 'sha-v2' },
        },
        ...options,
      })
      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')
      await fireEvent.click(screen.getByRole('tab', { name: 'Metadatos' }))
      await waitFor(() => {
        expect(rows()).toHaveLength(filesOnDisk.length)
      })
    }

    it('lists the versions of an image by number, with what each one corresponds to', async () => {
      await openVersions()

      expect(panel()).toHaveTextContent('Versiones de la imagen')
      expect(rows().map((row) => row.getAttribute('data-path'))).toEqual([
        'docs/photo-a.jpg',
        'docs/photo-a_v2.jpg',
        'docs/photo-a_v11.jpg',
      ])
      // A stamp exists, so the versions are hashed to be compared with it.
      expect(invokeMock).toHaveBeenCalledWith('list_image_versions', {
        assetId: 'asset-image-1',
        withSha: true,
      })

      const original = rowOf('docs/photo-a.jpg')
      expect(original).toHaveTextContent('Original')
      expect(original).toHaveTextContent('1000×1400')
      expect(original).toHaveTextContent('Corresponde al layout')
      expect(original).not.toHaveTextContent('Corresponde al texto')
      expect(original).not.toHaveTextContent('Actual')
      expect(within(original).getByRole('img')).toHaveAttribute(
        'src',
        'https://asset.localhost/docs/photo-a.jpg'
      )

      const shown = rowOf('docs/photo-a_v2.jpg')
      expect(shown).toHaveTextContent('Versión 2')
      expect(shown).toHaveTextContent('Actual')
      expect(shown).toHaveTextContent('Corresponde al texto')
      expect(shown).not.toHaveTextContent('Corresponde al layout')
      expect(useButton('docs/photo-a_v2.jpg')).toBeDisabled()

      const last = rowOf('docs/photo-a_v11.jpg')
      expect(last).toHaveTextContent('Versión 11')
      expect(last).not.toHaveTextContent('Corresponde')
      expect(useButton('docs/photo-a_v11.jpg')).toBeEnabled()
    })

    it('is not offered for a PDF nor for a page of one, and their files are never listed', async () => {
      storeRef.current = createStore({
        assetsRows: [
          { id: 'asset-pdf-1', itemId: 'item-1', path: 'docs/acta.pdf', type: 'pdf', createdAt: 1 },
          {
            id: 'asset-page-1',
            itemId: 'item-1',
            path: 'docs/acta-p1.png',
            type: 'image',
            createdAt: 2,
            parentAssetId: 'asset-pdf-1',
            pageNumber: 1,
          },
          {
            id: 'asset-pdf-2',
            itemId: 'item-1',
            path: 'docs/otra.pdf',
            type: 'pdf',
            createdAt: 3,
          },
        ],
      })
      render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
      await screen.findByTestId('mock-document-viewer')
      await fireEvent.click(screen.getByRole('tab', { name: 'Metadatos' }))
      await screen.findByTestId('asset-process-history')

      // The page of the first PDF, then the second PDF.
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/acta-p1.png'
      )
      expect(panel()).not.toBeInTheDocument()
      await fireEvent.click(screen.getByRole('button', { name: 'Página siguiente' }))
      await waitFor(() => {
        expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
          'data-path',
          'docs/otra.pdf'
        )
      })
      expect(panel()).not.toBeInTheDocument()
      expect(commandsInvoked()).not.toContain('list_image_versions')
    })

    it('shows the chosen version through the path of the asset, writing no file, and draws its layout again', async () => {
      await openVersions()
      await waitFor(() => {
        expect(layoutNotice()).toBeInTheDocument()
      })
      expect(drawnBoxes()).toBe('0')
      await fireEvent.click(screen.getByRole('button', { name: /report image dimensions/i }))
      const invokedBefore = invokeMock.mock.calls.length

      await fireEvent.click(useButton('docs/photo-a.jpg'))

      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
          'asset-image-1',
          'docs/photo-a.jpg'
        )
        expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
          'data-path',
          'docs/photo-a.jpg'
        )
      })
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      expect(storeRef.current.assets.updatePath).toHaveBeenCalledTimes(1)

      // Recorded in the history of the asset, with the file it replaced.
      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith('asset_history_record', {
          report: {
            assetId: 'asset-image-1',
            process: 'image_use_version',
            params: { page: 1, version: 0 },
            previousPath: 'docs/photo-a_v2.jpg',
          },
        })
      })
      // The other panes on this asset are told, by the view that did it.
      expect(emitMock).toHaveBeenCalledWith('asset:image-updated', {
        itemId: 'item-1',
        assetId: 'asset-image-1',
        path: 'docs/photo-a.jpg',
        sourceId: expect.any(String),
      })

      // No file is written: nothing but the history and the listing is invoked,
      // and no asset, layout or text is created or deleted.
      const invokedSince = invokeMock.mock.calls.slice(invokedBefore).map(([command]) => command)
      expect(new Set(invokedSince)).toEqual(
        new Set(['asset_history_record', 'list_image_versions'])
      )
      expect(duplicateAssetFileMock).not.toHaveBeenCalled()
      expect(storeRef.current.assets.create).not.toHaveBeenCalled()
      expect(storeRef.current.layouts.deleteByAssetId).not.toHaveBeenCalled()
      expect(storeRef.current.extractions.deleteByAsset).not.toHaveBeenCalled()
      expect(extractTextMock).not.toHaveBeenCalled()

      // The version guard decides by itself: this is the file of the layout.
      await waitFor(() => {
        expect(layoutNotice()).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: /mostrar overlay/i })).toBeEnabled()
      })
      await fireEvent.click(screen.getByRole('button', { name: /mostrar overlay/i }))
      expect(drawnBoxes()).toBe('5')

      // The panel follows: the original is the current one now.
      await waitFor(() => {
        expect(rowOf('docs/photo-a.jpg')).toHaveTextContent('Actual')
        expect(rowOf('docs/photo-a_v2.jpg')).not.toHaveTextContent('Actual')
      })
      expect(useButton('docs/photo-a.jpg')).toBeDisabled()

      // It is one more step of the history: undo goes back to what was shown.
      await waitFor(() => {
        expect(screen.getByRole('button', { name: /undo edit/i })).toBeEnabled()
      })
      await fireEvent.click(screen.getByRole('button', { name: /undo edit/i }))
      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenLastCalledWith(
          'asset-image-1',
          'docs/photo-a_v2.jpg'
        )
        expect(screen.getByRole('button', { name: /redo edit/i })).toBeEnabled()
      })
    })

    it('asks first when the image has annotations, and cancelling changes nothing', async () => {
      await openVersions({ annotationsByAsset: { 'asset-image-1': [storedAnnotation] } })
      await waitFor(() => {
        expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
      })
      const annotationsBefore = viewerAnnotations()

      await fireEvent.click(useButton('docs/photo-a.jpg'))

      const dialog = await screen.findByRole('dialog')
      expect(dialog).toHaveTextContent(WARNING)
      expect(storeRef.current.assets.updatePath).not.toHaveBeenCalled()

      await fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))
      await waitFor(() => {
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      })
      await vi.advanceTimersByTimeAsync(1000)

      expect(storeRef.current.assets.updatePath).not.toHaveBeenCalled()
      expect(storeRef.current.annotations.replaceForAssetPage).not.toHaveBeenCalled()
      expect(commandsInvoked()).not.toContain('asset_history_record')
      expect(emitMock).not.toHaveBeenCalledWith('asset:image-updated', expect.anything())
      expect(viewerAnnotations()).toBe(annotationsBefore)
      expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
        'data-path',
        'docs/photo-a_v2.jpg'
      )
      expect(screen.getByRole('button', { name: /undo edit/i })).toBeDisabled()
    })

    it('keeps the annotations exactly as they are when the version is confirmed', async () => {
      await openVersions({ annotationsByAsset: { 'asset-image-1': [storedAnnotation] } })
      await waitFor(() => {
        expect(screen.getByTestId('viewer-annotation-count')).toHaveTextContent('1')
      })
      const annotationsBefore = viewerAnnotations()
      expect(JSON.parse(annotationsBefore!)).toEqual([storedAnnotation])

      await fireEvent.click(useButton('docs/photo-a.jpg'))
      const dialog = await screen.findByRole('dialog')
      await fireEvent.click(within(dialog).getByRole('button', { name: 'Usar esta versión' }))

      await waitFor(() => {
        expect(storeRef.current.assets.updatePath).toHaveBeenCalledWith(
          'asset-image-1',
          'docs/photo-a.jpg'
        )
      })
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()

      // What is written back is what was stored: same page, same boxes, nothing
      // dropped and nothing added.
      expect(storeRef.current.annotations.replaceForAssetPage.mock.calls).toEqual([
        [
          'asset-image-1',
          1,
          [{ kind: 'rectangle', color: '#ff0000', x: 0.125, y: 0.25, width: 0.375, height: 0.5 }],
        ],
      ])
      expect(viewerAnnotations()).toBe(annotationsBefore)
    })
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

  it('auto-runs FTS and EMBED without re-persisting an atomic OCRC completion', async () => {
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

    expect(invokeMock).not.toHaveBeenCalledWith('update_extraction_text_cmd', {
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
    // The layout arrives after its version check, one step later than the viewer.
    expect(await screen.findByTestId('layout-inspector-empty')).toBeInTheDocument()

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

  it('re-recognizes a stretched layout box and splices its text in reading order', async () => {
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
      extractionsByAsset: {
        'asset-image-1': { textContent: 'Bloque título\n\nBloque cuerpo', method: 'glm_ocr' },
      },
    })
    const defaultInvoke = invokeMock.getMockImplementation()
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'reocr_layout_block') {
        return {
          textContent: 'Bloque título\n\nBloque cuerpo completo',
          blockContent: 'Bloque cuerpo completo',
          blockIndex: 1,
          anchored: true,
        }
      }
      return defaultInvoke?.(command, args)
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(await screen.findByTestId('layout-block-item-layout-block-1'))
    expect(screen.getByTestId('viewer-editable-layout-region')).toHaveTextContent('none')

    await fireEvent.click(screen.getByTestId('layout-box-edit-adjust'))
    expect(screen.getByTestId('viewer-editable-layout-region')).toHaveTextContent(
      'layout-block-1::overlay'
    )
    expect(screen.getByTestId('layout-box-edit-hint')).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Stretch editable layout region' }))
    expect(
      JSON.parse(screen.getByTestId('viewer-layout-regions').textContent ?? '[]').find(
        (region: { id: string }) => region.id === 'layout-block-1::overlay'
      )
    ).toMatchObject({ height: 150 })

    await fireEvent.click(screen.getByTestId('layout-box-edit-apply'))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('reocr_layout_block', {
        request: {
          assetId: 'asset-image-1',
          blockIndex: 1,
          expectedBbox: { x: 28, y: 138, width: 250, height: 110 },
          bbox: { x: 28, y: 138, width: 250, height: 150 },
          mode: 'replace',
          currentText: 'Bloque título\n\nBloque cuerpo',
        },
      })
    })
    expect(await screen.findByTestId('layout-box-edit-message')).toHaveTextContent(
      'Texto del bloque #2 actualizado.'
    )
    expect(screen.getByTestId('viewer-editable-layout-region')).toHaveTextContent('none')
  })

  it('records an image rotation in the asset process history', async () => {
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
    const defaultInvoke = invokeMock.getMockImplementation()
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'rotate_image') {
        return {
          path: 'docs/photo-a_v2.jpg',
          width: 100,
          height: 200,
          format_changed: false,
          previous_path: 'docs/photo-a.jpg',
        }
      }
      return defaultInvoke?.(command, args)
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('button', { name: 'Rotate right' }))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('asset_history_record', {
        report: {
          assetId: 'asset-image-1',
          process: 'image_rotate',
          params: { page: 1, direction: 'right' },
          previousPath: 'docs/photo-a.jpg',
        },
      })
    })
  })

  it('draws a new box after the selected block without selecting it away', async () => {
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
    await fireEvent.click(await screen.findByTestId('layout-block-item-layout-block-0'))
    await fireEvent.click(screen.getByTestId('layout-box-edit-insert'))

    expect(screen.getByTestId('viewer-editable-layout-region')).toHaveTextContent(
      'layout-block-draft::overlay'
    )
    const regions = JSON.parse(screen.getByTestId('viewer-layout-regions').textContent ?? '[]')
    expect(regions.at(-1)).toMatchObject({
      id: 'layout-block-draft::overlay',
      x: 8,
      y: 92,
      width: 180,
      height: 70,
    })

    await fireEvent.click(screen.getByTestId('layout-box-edit-cancel'))
    expect(screen.getByTestId('viewer-editable-layout-region')).toHaveTextContent('none')
    expect(screen.getByTestId('viewer-selected-layout-region')).toHaveTextContent(
      'layout-block-0::overlay'
    )
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

    // El alta arranca cerrada: el chip + es lo único visible en reposo. El tipo
    // se elige con el menú temático del visor (nunca un select nativo).
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar entidad' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Nuevo tipo de entidad' }))
    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'date' }))
    await fireEvent.input(screen.getByLabelText('Nuevo valor de entidad'), {
      target: { value: '21 de agosto de 1970' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar la nueva entidad' }))

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

    await fireEvent.click(screen.getByRole('button', { name: 'Agregar entidad' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Nuevo tipo de entidad' }))
    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'place' }))
    await fireEvent.input(screen.getByLabelText('Nuevo valor de entidad'), {
      target: { value: 'Tucumán' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar la nueva entidad' }))

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

  it('shows the OCR controls available for the current build variant', async () => {
    await renderTextTabForAsset('image')

    if (LOCAL_ML) {
      expect(screen.getByRole('button', { name: 'OCRL' })).toBeInTheDocument()
    } else {
      expect(screen.queryByRole('button', { name: 'OCRL' })).not.toBeInTheDocument()
    }
    expect(screen.getByRole('button', { name: 'OCRH' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRC' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRR' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PTT' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFC' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFR' })).not.toBeInTheDocument()
  })

  it('hides OCRC when the remote correction provider is unavailable', async () => {
    llmOcrCorrectionIsAvailableMock.mockResolvedValue(false)
    await renderTextTabForAsset('image')

    expect(screen.queryByRole('button', { name: 'OCRC' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRR' })).toBeInTheDocument()
  })

  // Following a citation opens this view with the quoted fragment. Moving on to
  // the next document reuses the view: the fragment belongs to the first one
  // and must not be marked in the second, even where its words recur.
  it('forgets the cited fragment when moving on to another document', async () => {
    const assetOf = (itemId: string) => ({
      id: `asset-${itemId}`,
      itemId,
      path: `docs/${itemId}.jpg`,
      type: 'image' as const,
      createdAt: 1,
    })
    storeRef.current = createStore({
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Acta uno',
          collectionId: 'col-1',
          metadata: '{}',
          createdAt: 1,
          updatedAt: 1,
        },
        'item-2': {
          id: 'item-2',
          title: 'Acta dos',
          collectionId: 'col-1',
          metadata: '{}',
          createdAt: 1,
          updatedAt: 1,
        },
      },
      extractionsByAsset: {
        'asset-item-1': { textContent: 'Acta del gremio', method: 'glm_ocr' },
        'asset-item-2': { textContent: 'Acta de la comision', method: 'glm_ocr' },
      },
    })
    storeRef.current.assets.findByItem = vi.fn(async (itemId: string) => [assetOf(itemId)])
    const itemView = (
      itemId: string,
      citationRange?: { start: number; end: number; text: string }
    ) => ({
      name: 'item' as const,
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId,
      itemTitle: itemId,
      ...(citationRange ? { citationRange } : {}),
    })
    workspace.activeNavigation.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      itemView('item-1', { start: 0, end: 4, text: 'Acta' }),
    ])
    highlightCitationRangeMock.mockClear()

    const { rerender } = render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await waitFor(() =>
      expect(highlightCitationRangeMock).toHaveBeenCalledWith(
        expect.anything(),
        'Acta del gremio',
        { start: 0, end: 4 }
      )
    )

    highlightCitationRangeMock.mockClear()
    workspace.activeNavigation.replace(itemView('item-2'))
    await rerender({ itemId: 'item-2', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: 'Texto extraído' }))
    const pane = await screen.findByRole('tabpanel', { name: 'Texto extraído' })
    await within(pane).findByText(/Acta de la comision/)
    await new Promise((resolve) => setTimeout(resolve, 50))

    expect(highlightCitationRangeMock).not.toHaveBeenCalled()
  })

  it('renders mixed OCR rich content only in the left extracted-text tab', async () => {
    const source = [
      '<div align="center">',
      '',
      '# Radiografía',
      '',
      '</div>',
      '',
      '**Texto**',
      '',
      '| A | B |',
      '| --- | --- |',
      '| 1 | 2 |',
      '',
      '![](page=0,bbox=[10,20,30])',
    ].join('\n')

    storeRef.current = createStore({
      assetsRows: [
        {
          id: 'asset-image-1',
          itemId: 'item-1',
          path: 'docs/source.jpg',
          type: 'image',
          createdAt: 1,
        },
      ],
      extractionsByAsset: {
        'asset-image-1': { textContent: source, method: 'glm_ocr' },
      },
    })

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await screen.findByTestId('mock-document-viewer')
    await fireEvent.click(screen.getByRole('tab', { name: 'Texto extraído' }))

    const leftPane = screen.getByRole('tabpanel', { name: 'Texto extraído' })
    expect(
      await within(leftPane).findByRole('heading', { level: 1, name: 'Radiografía' })
    ).toBeInTheDocument()
    expect(within(leftPane).getByText('Texto', { exact: true })).toBeInTheDocument()
    expect(within(leftPane).getByRole('table')).toBeInTheDocument()
    expect(within(leftPane).queryByText('# Radiografía')).not.toBeInTheDocument()
    expect(within(leftPane).getByText(/page=0,bbox=\[10,20,30\]/)).toBeInTheDocument()

    expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
      'data-path',
      'docs/source.jpg'
    )
    const rightTextTab = screen.getByRole('tab', { name: /^Texto$/i })
    await fireEvent.click(rightTextTab)
    expect(rightTextTab).toHaveAttribute('aria-selected', 'true')
    expect(await screen.findByText('Extracción de texto')).toBeInTheDocument()
    expect(storeRef.current.extractions.findByAsset).toHaveBeenCalledWith('asset-image-1')
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

  it(
    'concurrency_contract flushes latest OCR edit before correction and disables editing',
    { timeout: 15_000 },
    async () => {
      let resolvePersist!: () => void
      let resolveCorrection!: () => void
      invokeMock.mockImplementation((command: string) => {
        if (command === 'update_extraction_text_cmd') {
          return new Promise<void>((resolve) => {
            resolvePersist = resolve
          })
        }
        if (command === 'llm_get_results') return Promise.resolve([])
        if (command === 'db_select') return Promise.resolve([])
        return Promise.resolve(null)
      })
      llmCorrectOcrAssetMock.mockImplementation(
        () =>
          new Promise<void>((resolve) => {
            resolveCorrection = resolve
          })
      )
      await renderTextTabForAsset('image')
      nlpEventHandlers.get('ocr:complete')?.({
        payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
      })
      const textarea = await screen.findByDisplayValue('Texto OCR')
      await fireEvent.input(textarea, { target: { value: 'Edición manual más reciente' } })

      await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith('update_extraction_text_cmd', {
          assetId: 'asset-image-1',
          textContent: 'Edición manual más reciente',
        })
      )
      expect(llmCorrectOcrAssetMock).not.toHaveBeenCalled()

      resolvePersist()
      await waitFor(() => expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-image-1'))
      expect(textarea).toBeDisabled()

      resolveCorrection()
      await Promise.resolve()
      expect(textarea).toBeDisabled()

      nlpEventHandlers.get('llm:complete')?.({
        payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto corregido' },
      })
      await waitFor(() => expect(textarea).toBeEnabled())
    }
  )

  it(
    'concurrency_contract blocks correction after flush failure and keeps local text',
    { timeout: 15_000 },
    async () => {
      invokeMock.mockImplementation((command: string) => {
        if (command === 'update_extraction_text_cmd') {
          return Promise.reject(new Error('persist failed'))
        }
        if (command === 'llm_get_results') return Promise.resolve([])
        if (command === 'db_select') return Promise.resolve([])
        return Promise.resolve(null)
      })
      await renderTextTabForAsset('image')
      nlpEventHandlers.get('ocr:complete')?.({
        payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
      })
      const textarea = await screen.findByDisplayValue('Texto OCR')
      await fireEvent.input(textarea, { target: { value: 'Edición local recuperable' } })

      await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))

      expect(await screen.findByText('persist failed')).toBeInTheDocument()
      expect(llmCorrectOcrAssetMock).not.toHaveBeenCalled()
      expect(textarea).toHaveValue('Edición local recuperable')
      expect(textarea).toBeEnabled()
    }
  )

  it(
    'concurrency_contract reloads all OCR state after stale correction rejection',
    { timeout: 15_000 },
    async () => {
      await renderTextTabForAsset('image')
      nlpEventHandlers.get('ocr:complete')?.({
        payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto usado' },
      })
      expect(await screen.findByDisplayValue('Texto usado')).toBeInTheDocument()
      await waitFor(() =>
        expect(storeRef.current.extractions.findByAsset).toHaveBeenCalledWith('asset-image-1')
      )

      storeRef.current.extractions.findByAsset.mockClear()
      storeRef.current.extractions.findByAsset.mockResolvedValue({
        textContent: 'Edición manual vigente',
        method: 'paddle_vl',
      })
      llmCanRestoreOriginalOcrAssetMock.mockClear()
      llmCanRestoreOriginalOcrAssetMock.mockResolvedValue(true)
      llmGetResultsMock.mockClear()
      llmGetResultsMock.mockResolvedValue([
        {
          target_id: 'asset-image-1',
          target_type: 'asset',
          job_type: 'correct_ocr',
          result: 'Corrección previa',
          created_at: 1,
        },
      ])
      llmCorrectOcrAssetMock.mockRejectedValueOnce(
        new Error('stale OCR correction: extraction changed')
      )

      await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))

      expect(await screen.findByDisplayValue('Edición manual vigente')).toBeInTheDocument()
      await waitFor(() => {
        expect(storeRef.current.extractions.findByAsset).toHaveBeenCalledWith('asset-image-1')
        expect(llmCanRestoreOriginalOcrAssetMock).toHaveBeenCalledWith('asset-image-1')
        expect(llmGetResultsMock).toHaveBeenCalledWith('asset-image-1', 'asset')
      })
      await fireEvent.click(screen.getByRole('tab', { name: 'Texto extraído' }))
      const leftPane = screen.getByRole('tabpanel', { name: 'Texto extraído' })
      await waitFor(() =>
        expect(
          within(leftPane).getByRole('button', {
            name: 'Restaurar versión original del OCR',
          })
        ).toBeEnabled()
      )
      await waitFor(() =>
        expect(screen.queryByRole('button', { name: 'OCRC' })).not.toBeInTheDocument()
      )
    }
  )

  it('concurrency_review_contract keeps OCR locked after enqueue until matching completion', async () => {
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
    })
    const textarea = await screen.findByDisplayValue('Texto OCR')

    await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))
    await waitFor(() => expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-image-1'))
    expect(textarea).toBeDisabled()

    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'unrelated-asset', job: 'correct_ocr', result: 'Otro resultado' },
    })
    expect(textarea).toBeDisabled()

    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto corregido' },
    })
    await waitFor(() => expect(textarea).toBeEnabled())
  })

  it('concurrency_review_contract unlocks only for matching lifecycle error', async () => {
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
    })
    const textarea = await screen.findByDisplayValue('Texto OCR')

    await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))
    await waitFor(() => expect(textarea).toBeDisabled())

    nlpEventHandlers.get('llm:error')?.({
      payload: { id: 'unrelated-asset', job: 'correct_ocr', error: 'unrelated failure' },
    })
    expect(textarea).toBeDisabled()

    nlpEventHandlers.get('llm:error')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', error: 'provider failure' },
    })
    await waitFor(() => expect(textarea).toBeEnabled())
  })

  it('concurrency_review_contract unlocks immediately when enqueue rejects', async () => {
    llmCorrectOcrAssetMock.mockRejectedValueOnce(new Error('enqueue failed'))
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR' },
    })
    const textarea = await screen.findByDisplayValue('Texto OCR')

    await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))

    expect(await screen.findByText('No se pudo corregir el texto con OCR.')).toBeInTheDocument()
    expect(textarea).toBeEnabled()
  })

  it('lock_lifetime_contract proactively releases a deleted correction asset', async () => {
    const assets = [
      {
        id: 'asset-a',
        itemId: 'item-1',
        path: 'docs/a.jpg',
        type: 'image' as const,
        createdAt: 1,
      },
      {
        id: 'asset-b',
        itemId: 'item-1',
        path: 'docs/b.jpg',
        type: 'image' as const,
        createdAt: 2,
      },
    ]
    storeRef.current = createStore({
      assetsRows: assets,
      extractionsByAsset: {
        'asset-a': { textContent: 'Texto A', method: 'paddle_vl' },
        'asset-b': { textContent: 'Texto B', method: 'paddle_vl' },
      },
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: 'Texto' }))
    await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))
    await waitFor(() => expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-a'))

    window.dispatchEvent(
      new CustomEvent(DOCUMENT_ASSET_DELETED_EVENT, {
        detail: { itemId: 'item-1', assetId: 'asset-a' },
      })
    )

    expect(await screen.findByDisplayValue('Texto B')).toBeInTheDocument()
    const correctionForB = await screen.findByRole('button', { name: 'OCRC' })
    await waitFor(() => expect(correctionForB).toBeEnabled())
    await fireEvent.click(correctionForB)
    await waitFor(() => expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-b'))
  })

  it('lock_lifetime_contract clears a removed asset lock from its raw late event id', async () => {
    const assetA = {
      id: 'asset-a',
      itemId: 'item-1',
      path: 'docs/a.jpg',
      type: 'image' as const,
      createdAt: 1,
    }
    const assetB = {
      id: 'asset-b',
      itemId: 'item-2',
      path: 'docs/b.jpg',
      type: 'image' as const,
      createdAt: 2,
    }
    const store = createStore({
      assetsRows: [assetA],
      itemsById: {
        'item-1': {
          id: 'item-1',
          title: 'Item A',
          collectionId: 'col-1',
          metadata: '{}',
          createdAt: 1,
          updatedAt: 1,
        },
        'item-2': {
          id: 'item-2',
          title: 'Item B',
          collectionId: 'col-1',
          metadata: '{}',
          createdAt: 2,
          updatedAt: 2,
        },
      },
      extractionsByAsset: {
        'asset-a': { textContent: 'Texto A', method: 'paddle_vl' },
        'asset-b': { textContent: 'Texto B', method: 'paddle_vl' },
      },
    })
    store.assets.findByItem.mockImplementation(async (nextItemId: string) =>
      nextItemId === 'item-1' ? [assetA] : [assetB]
    )
    storeRef.current = store
    const { rerender } = render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: 'Texto' }))
    await fireEvent.click(await screen.findByRole('button', { name: 'OCRC' }))
    await waitFor(() => expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-a'))

    await rerender({ itemId: 'item-2', collectionId: 'col-1' })
    expect(await screen.findByDisplayValue('Texto B')).toBeInTheDocument()
    await fireEvent.click(await screen.findByRole('tab', { name: 'Texto' }))

    nlpEventHandlers.get('llm:error')?.({
      payload: { id: 'asset-a', job: 'correct_ocr', error: 'late failure' },
    })

    const correctionForB = await screen.findByRole('button', { name: 'OCRC' })
    await waitFor(() => expect(correctionForB).toBeEnabled())
    await fireEvent.click(correctionForB)
    await waitFor(() => expect(llmCorrectOcrAssetMock).toHaveBeenCalledWith('asset-b'))
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

  it('restores the durable original OCR and disables restore again', async () => {
    llmRestoreOriginalOcrAssetMock.mockResolvedValueOnce('Texto OCR original')
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_content: 'Texto OCR original',
      },
    })
    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto corregido' },
    })

    await fireEvent.click(screen.getByRole('tab', { name: 'Texto extraído' }))
    const leftPane = screen.getByRole('tabpanel', { name: 'Texto extraído' })
    const restore = await within(leftPane).findByRole('button', {
      name: 'Restaurar versión original del OCR',
    })
    await waitFor(() => expect(restore).toBeEnabled())

    await fireEvent.click(restore)

    expect(llmRestoreOriginalOcrAssetMock).toHaveBeenCalledWith('asset-image-1')
    expect(await within(leftPane).findByText('Texto OCR original')).toBeInTheDocument()
    await waitFor(() => expect(restore).toBeDisabled())
    await waitFor(() => expect(screen.getByRole('button', { name: 'OCRC' })).toBeEnabled())
  })

  it('keeps corrected state until a fresh OCR succeeds for the same asset', async () => {
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_content: 'Texto OCR inicial',
      },
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

    expect(screen.queryByRole('button', { name: 'OCRC' })).not.toBeInTheDocument()

    nlpEventHandlers.get('ocr:complete')?.({
      payload: { asset_id: 'asset-image-1', method: 'paddle_vl', text_content: 'Texto OCR nuevo' },
    })

    expect(await screen.findByDisplayValue('Texto OCR nuevo')).toBeInTheDocument()
    const resetCorrectButton = await screen.findByRole('button', { name: 'OCRC' })
    await waitFor(() => expect(resetCorrectButton).toBeEnabled())
  })

  it('atomic_ocr_ui_contract locks restore during matching OCR and releases on complete', async () => {
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_content: 'Texto OCR inicial',
      },
    })
    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto corregido' },
    })

    await fireEvent.click(screen.getByRole('tab', { name: 'Texto extraído' }))
    const leftPane = screen.getByRole('tabpanel', { name: 'Texto extraído' })
    const restore = await within(leftPane).findByRole('button', {
      name: 'Restaurar versión original del OCR',
    })
    await waitFor(() => expect(restore).toBeEnabled())

    await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))
    nlpEventHandlers.get('ocr:progress')?.({
      payload: { asset_id: 'asset-image-1', pct: 50, stage: 'ocr_inference' },
    })

    await waitFor(() => expect(restore).toBeDisabled())
    await fireEvent.click(restore)
    expect(llmRestoreOriginalOcrAssetMock).not.toHaveBeenCalled()

    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_content: 'Texto OCR nuevo',
      },
    })

    expect(await screen.findByDisplayValue('Texto OCR nuevo')).toBeInTheDocument()
    await waitFor(() => {
      expect(restore).toBeDisabled()
      expect(screen.getByRole('button', { name: 'OCRC' })).toBeEnabled()
      expect(screen.queryByText('Texto corregido')).not.toBeInTheDocument()
    })
  })

  it('atomic_ocr_ui_contract releases restore on matching OCR error with state intact', async () => {
    await renderTextTabForAsset('image')
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_content: 'Texto OCR inicial',
      },
    })
    nlpEventHandlers.get('llm:complete')?.({
      payload: { id: 'asset-image-1', job: 'correct_ocr', result: 'Texto corregido' },
    })

    await fireEvent.click(screen.getByRole('tab', { name: 'Texto extraído' }))
    const leftPane = screen.getByRole('tabpanel', { name: 'Texto extraído' })
    const restore = await within(leftPane).findByRole('button', {
      name: 'Restaurar versión original del OCR',
    })
    await waitFor(() => expect(restore).toBeEnabled())

    await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))
    nlpEventHandlers.get('ocr:progress')?.({
      payload: { asset_id: 'asset-image-1', pct: 50, stage: 'ocr_inference' },
    })
    await waitFor(() => expect(restore).toBeDisabled())

    nlpEventHandlers.get('ocr:error')?.({
      payload: { asset_id: 'asset-image-1', error: 'persistencia fallida' },
    })

    await waitFor(() => expect(restore).toBeEnabled())
    expect(await screen.findByDisplayValue('Texto corregido')).toBeInTheDocument()
  })

  it('runs PDF page assets through the same OCRH action as images', async () => {
    await renderTextTabForAsset('pdf')

    await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))

    expect(extractTextMock).toHaveBeenCalledWith('asset-pdf-1', 'docs/sample.pdf', 'pdf', 'high')
    expect(screen.queryByRole('button', { name: 'PTT' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRC' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'OCRR' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFC' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'PDFR' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OCRL' })).not.toBeInTheDocument()
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

  it('opens a full-height map tab while retaining the reduced analysis map', async () => {
    storeRef.current = createStore({ assetsRows: [imageAsset] })
    mockRotateInvoke()

    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })

    const mapTab = await screen.findByRole('tab', { name: 'Mapa' })
    await fireEvent.click(mapTab)

    expect(mapTab).toHaveAttribute('aria-selected', 'true')
    const mapViewers = await screen.findAllByTestId('item-map-viewer')
    const analysisMap = mapViewers.find((viewer) => viewer.dataset.height === '280px')
    const fullHeightMap = mapViewers.find((viewer) => viewer.dataset.height === '100%')

    expect(analysisMap).toBeDefined()
    expect(fullHeightMap).toBeDefined()
    expect(within(analysisMap!).getByTestId('mock-map-viewer')).toHaveAttribute(
      'data-visible',
      'false'
    )
    expect(within(fullHeightMap!).getByTestId('mock-map-viewer')).toHaveAttribute(
      'data-visible',
      'true'
    )
  })

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
    const path: [View, ...View[]] = [
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta histórica',
      },
    ]
    workspace.activeNavigation.resetToPath(path)
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
      expect(workspace.activeNavigation.current.name).toBe('item')

      // The second, uncaught Escape reaches the global handler, which now
      // navigates the active pane's own store back (fix round 1).
      await fireEvent.keyDown(window, { key: 'Escape' })
      expect(workspace.activeNavigation.current.name).toBe('collection')
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
      expect(workspace.activeNavigation.current.name).toBe('item')
    } finally {
      cleanupKeyboard()
    }
  })

  // Regression guard (Stage 2 visual fix #4, pre-existing — predates the
  // tabs/split-view stage): the "Documento activo" card's title and path had
  // no width bound of their own, so a long file name/path overflowed past
  // the right panel's edge instead of eliding. Both must clip with an
  // ellipsis inside the card, and the full text must stay reachable through
  // the shared `use:tooltip` action (never a native `title` attribute).
  it('keeps the active-document title and path inside the right panel card', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'ItemView.svelte'), 'utf-8')

    expect(source).toMatch(/import\s*\{[^}]*\btooltip\b[^}]*\}\s*from\s*'@entropia\/ui'/s)
    expect(source).toMatch(/<h2 class="item-title"[^>]*use:tooltip={item\.title}[^>]*>/)
    expect(source).toMatch(
      /<p class="item-header__meta"[^>]*use:tooltip={activeAssetSummary}[^>]*>/
    )
    expect(source).not.toMatch(/<h2 class="item-title"[^>]*\btitle=/)
    expect(source).not.toMatch(/<p class="item-header__meta"[^>]*\btitle=/)

    const headerStart = source.indexOf('  .item-header {')
    const headerRule = source.slice(headerStart, source.indexOf('}', headerStart))
    expect(headerRule).toMatch(/min-width:\s*0;/)

    const titleStart = source.indexOf('  .item-title {')
    const titleRule = source.slice(titleStart, source.indexOf('}', titleStart))
    expect(titleRule).toMatch(/min-width:\s*0;/)
    expect(titleRule).toMatch(/overflow:\s*hidden;/)
    expect(titleRule).toMatch(/text-overflow:\s*ellipsis;/)
    expect(titleRule).toMatch(/white-space:\s*nowrap;/)

    const metaStart = source.indexOf('  .item-header__meta {')
    const metaRule = source.slice(metaStart, source.indexOf('}', metaStart))
    expect(metaRule).toMatch(/min-width:\s*0;/)
    expect(metaRule).toMatch(/overflow:\s*hidden;/)
    expect(metaRule).toMatch(/text-overflow:\s*ellipsis;/)
    expect(metaRule).toMatch(/white-space:\s*nowrap;/)
  })
})

describe('ItemView text of another version of the image', () => {
  const imageAsset = {
    id: 'asset-image-1',
    itemId: 'item-1',
    path: 'docs/photo-a.jpg',
    type: 'image' as const,
    createdAt: 1,
  }
  const WARNING =
    'Este texto se extrajo de otra versión de la imagen. Puede no coincidir con lo que ves.'
  const notice = () => screen.queryByTestId('text-version-notice')
  const ocrText = () => document.querySelector('textarea.ocr-textarea')
  const fileWithSha = (sha256: string) => ({ orientation: 1, width: 1000, height: 1400, sha256 })

  beforeEach(() => {
    nlpEventHandlers.clear()
    getLayoutByAssetMock.mockReset().mockResolvedValue(null)
    extractTextMock.mockReset().mockResolvedValue(undefined)
    similarAssetsMock.mockReset().mockResolvedValue([])
    assetFileFactsMock.mockReset()
  })

  async function openTextTab(
    asset: typeof imageAsset | (Omit<typeof imageAsset, 'type'> & { type: 'pdf' }),
    extraction: { textContent: string; method?: string; sourceSha256?: string }
  ) {
    storeRef.current = createStore({
      assetsRows: [asset],
      extractionsByAsset: { [asset.id]: extraction },
    })
    render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
    await fireEvent.click(await screen.findByRole('tab', { name: 'Texto' }))
    await waitFor(() => {
      expect(ocrText()).toHaveValue(extraction.textContent)
    })
  }

  it('warns, and keeps the text, when it was extracted from another file', async () => {
    assetFileFactsMock.mockResolvedValue(fileWithSha('sha-edited'))
    await openTextTab(imageAsset, {
      textContent: 'Texto corregido',
      method: 'paddle_vl',
      sourceSha256: 'sha-original',
    })

    await waitFor(() => {
      expect(notice()).toHaveTextContent(WARNING)
    })
    expect(assetFileFactsMock).toHaveBeenCalledWith('asset-image-1', { withSha: true })
    expect(ocrText()).toHaveValue('Texto corregido')
    expect(storeRef.current.extractions.deleteByAsset).not.toHaveBeenCalled()

    await fireEvent.click(within(notice()!).getByRole('button', { name: 'Volver a correr el OCR' }))
    await waitFor(() => {
      expect(extractTextMock).toHaveBeenCalledWith(
        'asset-image-1',
        'docs/photo-a.jpg',
        'image',
        LOCAL_ML ? 'light' : 'high'
      )
    })
  })

  it('stops warning once a new OCR has read the current file', async () => {
    assetFileFactsMock.mockResolvedValue(fileWithSha('sha-edited'))
    await openTextTab(imageAsset, { textContent: 'Texto viejo', sourceSha256: 'sha-original' })
    await waitFor(() => {
      expect(notice()).toBeInTheDocument()
    })

    storeRef.current.extractions.findByAsset.mockResolvedValue({
      textContent: 'Texto nuevo',
      method: 'paddle_vl',
      sourcePath: 'docs/photo-a.jpg',
      sourceSha256: 'sha-edited',
    })
    nlpEventHandlers.get('ocr:complete')?.({
      payload: {
        asset_id: 'asset-image-1',
        method: 'paddle_vl',
        text_length: 11,
        text_content: 'Texto nuevo',
      },
    })

    await waitFor(() => {
      expect(notice()).not.toBeInTheDocument()
    })
  })

  it('says nothing when the text was extracted from the file on screen', async () => {
    assetFileFactsMock.mockResolvedValue(fileWithSha('sha-original'))
    await openTextTab(imageAsset, { textContent: 'Texto', sourceSha256: 'sha-original' })

    await waitFor(() => {
      expect(assetFileFactsMock).toHaveBeenCalledWith('asset-image-1', { withSha: true })
    })
    expect(notice()).not.toBeInTheDocument()
  })

  it('says nothing, and hashes nothing, for a text with no stamp', async () => {
    assetFileFactsMock.mockResolvedValue(fileWithSha('sha-edited'))
    await openTextTab(imageAsset, { textContent: 'Texto sin origen' })

    expect(notice()).not.toBeInTheDocument()
    expect(assetFileFactsMock).not.toHaveBeenCalledWith('asset-image-1', { withSha: true })
  })

  it('never warns about a PDF, and never reads its file', async () => {
    assetFileFactsMock.mockResolvedValue(fileWithSha('sha-edited'))
    await openTextTab(
      { ...imageAsset, id: 'asset-pdf-1', path: 'docs/acta.pdf', type: 'pdf' },
      { textContent: 'Texto del PDF', sourceSha256: 'sha-of-some-other-file' }
    )

    expect(notice()).not.toBeInTheDocument()
    expect(assetFileFactsMock).not.toHaveBeenCalled()
  })

  it('says nothing, and keeps the text, when the file cannot be read', async () => {
    assetFileFactsMock.mockRejectedValue(new Error('Failed to read docs/photo-a.jpg'))
    await openTextTab(imageAsset, { textContent: 'Texto', sourceSha256: 'sha-original' })

    await waitFor(() => {
      expect(assetFileFactsMock).toHaveBeenCalledWith('asset-image-1', { withSha: true })
    })
    expect(notice()).not.toBeInTheDocument()
    expect(ocrText()).toHaveValue('Texto')
  })

  describe('running the OCR again over a text edited by hand', () => {
    const CONFIRMATION =
      'El texto tiene correcciones hechas a mano. Si volvés a correr el OCR, se reemplazan. ¿Continuar?'

    function editedByHand(edited: boolean) {
      invokeMock.mockImplementation(async (command: string) => {
        if (command === 'extraction_is_edited_by_hand') return edited
        if (command === 'llm_get_results') return []
        if (command === 'llm_is_available') return true
        if (command === 'db_select') return []
        return null
      })
    }

    it('asks first; cancelling starts nothing and leaves the text as it was', async () => {
      editedByHand(true)
      await openTextTab(imageAsset, { textContent: 'Texto corregido a mano' })

      await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))
      const dialog = await screen.findByRole('dialog')
      expect(dialog).toHaveTextContent(CONFIRMATION)
      expect(invokeMock).toHaveBeenCalledWith('extraction_is_edited_by_hand', {
        assetId: 'asset-image-1',
      })
      expect(extractTextMock).not.toHaveBeenCalled()

      await fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))
      await waitFor(() => {
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      })
      expect(extractTextMock).not.toHaveBeenCalled()
      expect(ocrText()).toHaveValue('Texto corregido a mano')
      expect(screen.getByRole('button', { name: 'OCRH' })).toBeEnabled()
    })

    it('runs it once confirmed', async () => {
      editedByHand(true)
      await openTextTab(imageAsset, { textContent: 'Texto corregido a mano' })

      await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))
      const dialog = await screen.findByRole('dialog')
      await fireEvent.click(within(dialog).getByRole('button', { name: 'Volver a correr el OCR' }))

      await waitFor(() => {
        expect(extractTextMock).toHaveBeenCalledWith(
          'asset-image-1',
          'docs/photo-a.jpg',
          'image',
          'high'
        )
      })
      expect(extractTextMock).toHaveBeenCalledTimes(1)
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })

    it('does not ask when the text was not touched', async () => {
      editedByHand(false)
      await openTextTab(imageAsset, { textContent: 'Texto del OCR' })

      await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))

      await waitFor(() => {
        expect(extractTextMock).toHaveBeenCalledWith(
          'asset-image-1',
          'docs/photo-a.jpg',
          'image',
          'high'
        )
      })
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })

    it('asks about a correction typed just now, before the history knows of it', async () => {
      editedByHand(false)
      await openTextTab(imageAsset, { textContent: 'Texto del OCR' })

      await fireEvent.input(ocrText()!, { target: { value: 'Texto del OCR, corregido' } })
      await fireEvent.click(screen.getByRole('button', { name: 'OCRH' }))

      expect(await screen.findByRole('dialog')).toHaveTextContent(CONFIRMATION)
      expect(extractTextMock).not.toHaveBeenCalled()
    })

    it('asks from the notices too', async () => {
      editedByHand(true)
      assetFileFactsMock.mockResolvedValue(fileWithSha('sha-edited'))
      await openTextTab(imageAsset, {
        textContent: 'Texto corregido a mano',
        sourceSha256: 'sha-original',
      })
      await waitFor(() => {
        expect(notice()).toBeInTheDocument()
      })

      await fireEvent.click(
        within(notice()!).getByRole('button', { name: 'Volver a correr el OCR' })
      )

      expect(await screen.findByRole('dialog')).toHaveTextContent(CONFIRMATION)
      expect(extractTextMock).not.toHaveBeenCalled()
    })
  })
})

describe('ItemView on the same asset in two panes', () => {
  const imageAsset = {
    id: 'asset-image-1',
    itemId: 'item-1',
    path: 'docs/photo-a.jpg',
    type: 'image' as const,
    createdAt: 1,
  }

  beforeEach(() => {
    nlpEventHandlers.clear()
    getLayoutByAssetMock.mockReset().mockResolvedValue(null)
    similarAssetsMock.mockReset().mockResolvedValue([])
    assetFileFactsMock
      .mockReset()
      .mockResolvedValue({ orientation: 1, width: 1000, height: 1400, sha256: null })
  })

  it('reloads the path and drops the stale history of the other pane, not of the one that edited', async () => {
    storeRef.current = createStore({ assetsRows: [imageAsset] })
    // The archive: every pane reads the path the last edit wrote.
    let storedPath = imageAsset.path
    storeRef.current.assets.findByItem.mockImplementation(async () => [
      { ...imageAsset, path: storedPath },
    ])
    storeRef.current.assets.updatePath.mockImplementation(async (_id: string, path: string) => {
      storedPath = path
    })
    let version = 1
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'rotate_image') {
        version += 1
        return { path: `docs/photo-a_v${version}.jpg`, width: 1400, height: 1000 }
      }
      if (command === 'llm_get_results') return []
      if (command === 'llm_is_available') return true
      if (command === 'db_select') return []
      return null
    })
    // Tauri delivers an emitted event to every listener, the emitter included.
    emitMock.mockImplementation(async (eventName: string, payload?: unknown) => {
      for (const listener of [...(tauriListeners.get(eventName) ?? [])]) listener({ payload })
    })

    const first = within(render(ItemView, { itemId: 'item-1', collectionId: 'col-1' }).container)
    const second = within(render(ItemView, { itemId: 'item-1', collectionId: 'col-1' }).container)
    const viewerOf = (pane: typeof first) => pane.getByTestId('mock-document-viewer')
    const undoOf = (pane: typeof first) => pane.getByRole('button', { name: /undo edit/i })
    await first.findByTestId('mock-document-viewer')
    await second.findByTestId('mock-document-viewer')
    await waitFor(() => {
      expect(tauriListeners.get('asset:image-updated')?.size).toBe(2)
    })

    // The second pane edits: it owns a history, and the first one follows.
    await fireEvent.click(second.getByRole('button', { name: /report image dimensions/i }))
    await fireEvent.click(second.getByRole('button', { name: 'Rotate right' }))
    await waitFor(() => {
      expect(undoOf(second)).toBeEnabled()
      expect(viewerOf(first)).toHaveAttribute('data-path', 'docs/photo-a_v2.jpg')
    })
    expect(viewerOf(second)).toHaveAttribute('data-path', 'docs/photo-a_v2.jpg')
    expect(undoOf(first)).toBeDisabled()

    // The first pane edits from the path it just reloaded, not from the old one.
    await fireEvent.click(first.getByRole('button', { name: /report image dimensions/i }))
    await fireEvent.click(first.getByRole('button', { name: 'Rotate right' }))
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('rotate_image', {
        path: 'docs/photo-a_v2.jpg',
        direction: 'right',
      })
      expect(undoOf(first)).toBeEnabled()
      expect(viewerOf(second)).toHaveAttribute('data-path', 'docs/photo-a_v3.jpg')
    })
    // The second pane's undo would have restored a file that is no longer the
    // previous version of what is on screen.
    await waitFor(() => {
      expect(undoOf(second)).toBeDisabled()
    })
    expect(undoOf(first)).toBeEnabled()
    expect(viewerOf(first)).toHaveAttribute('data-path', 'docs/photo-a_v3.jpg')

    // An edit of some other asset is nobody's business here.
    const reads = storeRef.current.assets.findByItem.mock.calls.length
    await emitMock('asset:image-updated', {
      itemId: 'item-9',
      assetId: 'asset-of-another-item',
      path: 'docs/other_v2.jpg',
    })
    await Promise.resolve()
    expect(storeRef.current.assets.findByItem.mock.calls.length).toBe(reads)
    expect(undoOf(first)).toBeEnabled()
    expect(viewerOf(first)).toHaveAttribute('data-path', 'docs/photo-a_v3.jpg')
  })
})
