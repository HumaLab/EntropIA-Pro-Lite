import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CollectionView from './CollectionView.svelte'
import { locale } from '$lib/i18n'
import { DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT } from '$lib/document-explorer'
import type { CollectionItemCardSummary } from '@entropia/store'

const { storeRef } = vi.hoisted(() => ({
  storeRef: { current: null as unknown as ReturnType<typeof createPagedStore> },
}))

vi.mock('$lib/db', () => ({ getStore: () => storeRef.current }))

vi.mock('$lib/navigation', () => ({
  navigation: {
    current: { name: 'collection', collectionName: 'Colección' },
    navigate: vi.fn(),
  },
}))

vi.mock('$lib/file-import', () => ({
  pickFiles: vi.fn(),
  classifyFiles: vi.fn(),
  importSingleFile: vi.fn(),
  splitPdfPages: vi.fn(),
  getAssetUrl: (path: string) => path,
  generateImageThumbnail: vi.fn().mockResolvedValue(null),
  deleteAssetFile: vi.fn(),
  deleteImageThumbnail: vi.fn(),
  deletePdfThumbnail: vi.fn(),
}))

vi.mock('$lib/export', () => ({ exportCollectionById: vi.fn() }))

vi.mock('@tauri-apps/api/path', () => ({ appDataDir: vi.fn(), join: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }))
vi.mock('@tauri-apps/plugin-fs', () => ({ remove: vi.fn(), stat: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(() => {}) }),
}))

function summary(id: string, title: string): CollectionItemCardSummary {
  return {
    id,
    title,
    collectionId: 'col-1',
    metadata: null,
    createdAt: 0,
    updatedAt: 0,
    assetCount: 0,
    primaryAssetId: null,
    primaryAssetPath: null,
    primaryAssetType: null,
  }
}

type PageResult = {
  items: CollectionItemCardSummary[]
  nextCursor: { title: string; id: string } | null
  hasMore: boolean
}

function page(titles: string[], hasMore: boolean): PageResult {
  const items = titles.map((title) => summary(`doc-${title.toLowerCase()}`, title))
  const last = items[items.length - 1]
  return {
    items,
    hasMore,
    nextCursor: hasMore && last ? { title: last.title, id: last.id } : null,
  }
}

function createPagedStore(findCardSummariesPage: ReturnType<typeof vi.fn>) {
  return {
    items: {
      findCardSummariesPage,
      findCardSummariesByCollection: vi.fn(),
      findByCollection: vi.fn().mockResolvedValue([]),
      searchByText: vi.fn().mockResolvedValue([]),
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
      findByItem: vi.fn().mockResolvedValue([]),
      findById: vi.fn().mockResolvedValue(null),
      findByParentAssetId: vi.fn().mockResolvedValue([]),
      deleteWithCascade: vi.fn().mockResolvedValue(undefined),
    },
    extractions: { findTextByCollection: vi.fn().mockResolvedValue([]) },
    transcriptions: { findTextByCollection: vi.fn().mockResolvedValue([]) },
  }
}

function renderCollectionView(findCardSummariesPage: ReturnType<typeof vi.fn>) {
  storeRef.current = createPagedStore(findCardSummariesPage)
  return render(CollectionView, { props: { collectionId: 'col-1' } })
}

function renderedTitles(): string[] {
  return Array.from(document.querySelectorAll('.item-card__title')).map(
    (node) => node.textContent?.trim() ?? ''
  )
}

/**
 * happy-dom has no layout, so scroll geometry has to be declared rather than
 * produced. These four numbers are the only ones the trigger reads.
 */
async function scrollGridTo(fraction: number) {
  const container = screen.getByTestId('collection-scroll')
  const scrollHeight = 6000
  const viewportHeight = 600
  Object.defineProperty(container, 'scrollHeight', { value: scrollHeight, configurable: true })
  Object.defineProperty(container, 'clientHeight', { value: viewportHeight, configurable: true })
  container.scrollTop = (scrollHeight - viewportHeight) * fraction
  await fireEvent.scroll(container)
}

describe('CollectionView pagination', () => {
  beforeEach(() => {
    locale.set('es')
    vi.clearAllMocks()
  })

  it('loads only the first page when the collection opens', async () => {
    const fetchPage = vi.fn().mockResolvedValue(page(['Alpha'], false))
    renderCollectionView(fetchPage)

    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha']))
    expect(fetchPage).toHaveBeenCalledTimes(1)
    expect(fetchPage).toHaveBeenCalledWith(
      'col-1',
      expect.objectContaining({ cursor: null, limit: 100 })
    )
  })

  it('keeps an initial load failure separate from the grid', async () => {
    const fetchPage = vi.fn().mockRejectedValue(new Error('cannot open collection'))
    renderCollectionView(fetchPage)

    await waitFor(() => expect(screen.getByText('cannot open collection')).toBeVisible())
    expect(renderedTitles()).toEqual([])
    expect(screen.queryByTestId('collection-page-retry')).not.toBeInTheDocument()
  })

  it('appends the next page when the scroll trigger fires', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(['Alpha', 'Bravo'], true))
      .mockResolvedValueOnce(page(['Charlie'], false))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo']))

    await scrollGridTo(1)

    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo', 'Charlie']))
    expect(fetchPage).toHaveBeenLastCalledWith(
      'col-1',
      expect.objectContaining({ cursor: { title: 'Bravo', id: 'doc-bravo' } })
    )
  })

  it('issues one request even when the scroll trigger fires repeatedly', async () => {
    let releasePage2: ((value: PageResult) => void) | undefined
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(['Alpha', 'Bravo'], true))
      .mockImplementationOnce(() => new Promise<PageResult>((resolve) => (releasePage2 = resolve)))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo']))

    await scrollGridTo(1)
    await scrollGridTo(1)
    await scrollGridTo(1)

    expect(fetchPage).toHaveBeenCalledTimes(2)
    releasePage2?.(page(['Charlie'], false))
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo', 'Charlie']))
  })

  it('does not request another page once the collection is exhausted', async () => {
    const fetchPage = vi.fn().mockResolvedValue(page(['Alpha'], false))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha']))

    await scrollGridTo(1)

    expect(fetchPage).toHaveBeenCalledTimes(1)
  })

  it('keeps loaded rows and retries page 2 from the same cursor after a failure', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(['Alpha', 'Bravo'], true))
      .mockRejectedValueOnce(new Error('page 2 failed'))
      .mockResolvedValueOnce(page(['Charlie'], false))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo']))

    await scrollGridTo(1)
    await waitFor(() => expect(screen.getByTestId('collection-page-retry')).toBeVisible())

    // The already-loaded rows survive a failed continuation.
    expect(renderedTitles()).toEqual(['Alpha', 'Bravo'])
    expect(screen.getByText('page 2 failed')).toBeVisible()

    await fireEvent.click(screen.getByTestId('collection-page-retry'))

    // The cursor is internal state, not UI: it is proven to have survived by
    // the retry reusing it, never by rendering it.
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo', 'Charlie']))
    expect(fetchPage).toHaveBeenLastCalledWith(
      'col-1',
      expect.objectContaining({ cursor: { title: 'Bravo', id: 'doc-bravo' } })
    )
    expect(screen.queryByTestId('collection-page-retry')).not.toBeInTheDocument()
  })

  it('renders a row returned twice by the server only once', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(['Alpha', 'Bravo'], true))
      .mockResolvedValueOnce(page(['Bravo', 'Charlie'], false))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo']))

    await scrollGridTo(1)

    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo', 'Charlie']))
  })

  it('resets to page 1 when the collection ordering changes under it', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(['Alpha', 'Bravo'], true))
      .mockResolvedValueOnce(page(['Charlie'], false))
      .mockResolvedValueOnce(page(['Aaron', 'Alpha'], false))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo']))
    await scrollGridTo(1)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo', 'Charlie']))

    const scroller = screen.getByTestId('collection-scroll')
    scroller.scrollTop = 400
    window.dispatchEvent(
      new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
        detail: { collectionId: 'col-1' },
      })
    )

    await waitFor(() => expect(renderedTitles()).toEqual(['Aaron', 'Alpha']))
    expect(fetchPage).toHaveBeenLastCalledWith('col-1', expect.objectContaining({ cursor: null }))
    expect(scroller.scrollTop).toBe(0)
  })

  it('ignores an ordering change announced for a different collection', async () => {
    const fetchPage = vi.fn().mockResolvedValue(page(['Alpha'], false))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha']))

    window.dispatchEvent(
      new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
        detail: { collectionId: 'col-other' },
      })
    )

    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(fetchPage).toHaveBeenCalledTimes(1)
  })

  it('restarts pagination from page 1 when the search query changes', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(['Alpha', 'Bravo'], true))
      .mockResolvedValueOnce(page(['Bravo'], false))
    renderCollectionView(fetchPage)
    await waitFor(() => expect(renderedTitles()).toEqual(['Alpha', 'Bravo']))

    await fireEvent.input(screen.getByPlaceholderText('Buscar documentos...'), {
      target: { value: 'bravo' },
    })

    await waitFor(() => expect(renderedTitles()).toEqual(['Bravo']))
    expect(fetchPage).toHaveBeenLastCalledWith(
      'col-1',
      expect.objectContaining({ cursor: null, search: 'bravo' })
    )
  })
})
