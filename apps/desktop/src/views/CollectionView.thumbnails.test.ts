import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CollectionView from './CollectionView.svelte'
import { locale } from '$lib/i18n'
import { generateImageThumbnail } from '$lib/file-import'
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
  generateImageThumbnail: vi.fn(async (path: string) => `/thumbs${path}`),
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

const thumbnailMock = vi.mocked(generateImageThumbnail)

function imageSummary(index: number): CollectionItemCardSummary {
  const id = `doc-${String(index).padStart(3, '0')}`
  return {
    id,
    title: `Documento ${String(index).padStart(3, '0')}`,
    collectionId: 'col-1',
    metadata: null,
    createdAt: 0,
    updatedAt: 0,
    assetCount: 1,
    primaryAssetId: `asset-${id}`,
    primaryAssetPath: `/a/${id}.png`,
    primaryAssetType: 'image',
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

function renderWithPage(items: CollectionItemCardSummary[], hasMore = false) {
  const last = items[items.length - 1]
  const findCardSummariesPage = vi.fn().mockResolvedValue({
    items,
    hasMore,
    nextCursor: hasMore && last ? { title: last.title, id: last.id } : null,
  })
  storeRef.current = createPagedStore(findCardSummariesPage)
  return { ...render(CollectionView, { props: { collectionId: 'col-1' } }), findCardSummariesPage }
}

function requestedPaths(): string[] {
  return thumbnailMock.mock.calls.map((call) => call[0] as string)
}

describe('CollectionView thumbnails', () => {
  beforeEach(() => {
    locale.set('es')
    vi.clearAllMocks()
    thumbnailMock.mockImplementation(async (path: string) => `/thumbs${path}`)
  })

  it('generates thumbnails only for the loaded page, never the whole collection', async () => {
    const page = Array.from({ length: 12 }, (_, index) => imageSummary(index))
    renderWithPage(page, true)

    await waitFor(() => expect(thumbnailMock).toHaveBeenCalled())
    await waitFor(() => expect(requestedPaths()).toHaveLength(12))
    expect(new Set(requestedPaths()).size).toBe(12)
  })

  it('never has more than four thumbnail requests in flight at once', async () => {
    let inFlight = 0
    let peak = 0
    const release: Array<() => void> = []
    thumbnailMock.mockImplementation(
      (path: string) =>
        new Promise((resolve) => {
          inFlight += 1
          peak = Math.max(peak, inFlight)
          release.push(() => {
            inFlight -= 1
            resolve(`/thumbs${path}`)
          })
        })
    )
    renderWithPage(Array.from({ length: 20 }, (_, index) => imageSummary(index)))

    await waitFor(() => expect(release.length).toBe(4))
    while (release.length > 0) {
      release.shift()?.()
      await Promise.resolve()
    }

    expect(peak).toBeLessThanOrEqual(4)
  })

  it('shows a generated thumbnail on its card', async () => {
    renderWithPage([imageSummary(0)])

    await waitFor(() => {
      const img = document.querySelector('.item-card__img') as HTMLImageElement | null
      expect(img?.getAttribute('src')).toBe('/thumbs/a/doc-000.png')
    })
  })

  it('keeps the first page thumbnails when a second page arrives', async () => {
    const firstPage = Array.from({ length: 4 }, (_, index) => imageSummary(index))
    const secondPage = Array.from({ length: 4 }, (_, index) => imageSummary(index + 4))
    const findCardSummariesPage = vi
      .fn()
      .mockResolvedValueOnce({
        items: firstPage,
        hasMore: true,
        nextCursor: { title: firstPage[3]!.title, id: firstPage[3]!.id },
      })
      .mockResolvedValueOnce({ items: secondPage, hasMore: false, nextCursor: null })
    storeRef.current = createPagedStore(findCardSummariesPage)
    render(CollectionView, { props: { collectionId: 'col-1' } })

    await waitFor(() => expect(requestedPaths()).toHaveLength(4))

    const container = screen.getByTestId('collection-scroll')
    Object.defineProperty(container, 'scrollHeight', { value: 6000, configurable: true })
    Object.defineProperty(container, 'clientHeight', { value: 600, configurable: true })
    container.scrollTop = 5400
    await fireEvent.scroll(container)

    await waitFor(() => expect(requestedPaths()).toHaveLength(8))
    // Every card still carries its thumbnail: page 2 must not cancel page 1.
    await waitFor(() => {
      const sources = Array.from(document.querySelectorAll('.item-card__img')).map((img) =>
        img.getAttribute('src')
      )
      expect(sources).toHaveLength(8)
      expect(sources.every((src) => src?.startsWith('/thumbs/'))).toBe(true)
    })
  })

  it('asks for a thumbnail once per asset even across repeated renders', async () => {
    const page = [imageSummary(0), imageSummary(1)]
    const { findCardSummariesPage } = renderWithPage(page)
    await waitFor(() => expect(requestedPaths()).toHaveLength(2))

    await fireEvent.input(screen.getByPlaceholderText('Buscar documentos...'), {
      target: { value: 'documento' },
    })

    await waitFor(() => expect(findCardSummariesPage).toHaveBeenCalledTimes(2))
    // The same two assets came back: the on-disk thumbnails are already known,
    // so no second round of IPC is spent on them.
    expect(requestedPaths()).toHaveLength(2)
  })

  it('leaves a card without a thumbnail when generation fails, and keeps the rest', async () => {
    thumbnailMock.mockImplementation(async (path: string) => {
      if (path === '/a/doc-001.png') throw new Error('decode failed')
      return `/thumbs${path}`
    })
    renderWithPage([imageSummary(0), imageSummary(1), imageSummary(2)])

    await waitFor(() => {
      const sources = Array.from(document.querySelectorAll('.item-card__img')).map((img) =>
        img.getAttribute('src')
      )
      expect(sources).toEqual(['/thumbs/a/doc-000.png', '/thumbs/a/doc-002.png'])
    })
    expect(screen.getAllByTestId('item-placeholder')).toHaveLength(1)
  })

  it('never rasterizes a PDF for a card', async () => {
    const pdf: CollectionItemCardSummary = {
      ...imageSummary(0),
      primaryAssetType: 'pdf',
      primaryAssetPath: '/a/doc-000.pdf',
    }
    renderWithPage([pdf])

    await waitFor(() => expect(screen.getByTestId('item-pdf-icon')).toBeVisible())
    expect(thumbnailMock).not.toHaveBeenCalled()
  })
})
