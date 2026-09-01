import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CollectionView from './CollectionView.svelte'
import { locale } from '$lib/i18n'
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
  deleteAssetFile: vi.fn().mockResolvedValue(undefined),
  deleteImageThumbnail: vi.fn().mockResolvedValue(undefined),
  deletePdfThumbnail: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('$lib/export', () => ({ exportCollectionById: vi.fn() }))
vi.mock('@tauri-apps/api/path', () => ({ appDataDir: vi.fn(), join: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }))
vi.mock('@tauri-apps/plugin-fs', () => ({ remove: vi.fn(), stat: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(() => {}) }),
}))

function summary(index: number): CollectionItemCardSummary {
  const id = `doc-${index}`
  return {
    id,
    title: `Documento ${index}`,
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

/** The grid renders 12 cards; the viewport is two rows tall. */
function renderGrid(count = 12) {
  const items = Array.from({ length: count }, (_, index) => summary(index))
  const findCardSummariesPage = vi
    .fn()
    .mockResolvedValue({ items, hasMore: false, nextCursor: null })
  storeRef.current = createPagedStore(findCardSummariesPage)
  return { ...render(CollectionView, { props: { collectionId: 'col-1' } }), findCardSummariesPage }
}

function card(index: number): HTMLElement | null {
  return document.querySelector(`[data-virtual-key="doc-${index}"]`)
}

function focusableIn(element: HTMLElement): HTMLElement {
  return element.querySelector('button') as HTMLElement
}

function renderedKeys(): string[] {
  return Array.from(document.querySelectorAll('[data-virtual-key]')).map(
    (node) => (node as HTMLElement).dataset.virtualKey ?? ''
  )
}

/**
 * happy-dom reports no layout, so the grid's geometry has to be declared.
 * Two columns of 200px rows in a 400px viewport: four cards visible.
 */
async function setViewport(scrollTop: number) {
  const scroller = screen.getByTestId('collection-scroll')
  Object.defineProperty(scroller, 'clientHeight', { value: 400, configurable: true })
  Object.defineProperty(scroller, 'scrollHeight', { value: 1200, configurable: true })
  scroller.scrollTop = scrollTop
  await fireEvent.scroll(scroller)
}

describe('CollectionView focus', () => {
  beforeEach(() => {
    locale.set('es')
    vi.clearAllMocks()
  })

  it('keeps the focused card focused while it stays in the rendered window', async () => {
    renderGrid()
    await waitFor(() => expect(card(2)).not.toBeNull())

    const target = focusableIn(card(2)!)
    target.focus()
    expect(target).toHaveFocus()

    await setViewport(0)

    expect(card(2)).not.toBeNull()
    expect(focusableIn(card(2)!)).toHaveFocus()
  })

  it('hands focus to the nearest surviving card when the focused one is evicted', async () => {
    renderGrid(40)
    await waitFor(() => expect(card(0)).not.toBeNull())

    focusableIn(card(0)!).focus()
    await setViewport(1000)

    await waitFor(() => expect(card(0)).toBeNull())
    // Focus never lands on nothing: it moves to the nearest card still rendered.
    const survivor = document.activeElement?.closest('[data-virtual-key]') as HTMLElement | null
    expect(survivor).not.toBeNull()
    expect(renderedKeys()).toContain(survivor!.dataset.virtualKey)
  })

  it('moves focus to the next card when the focused one is deleted', async () => {
    renderGrid(6)
    await waitFor(() => expect(card(2)).not.toBeNull())
    focusableIn(card(2)!).focus()

    await fireEvent.click(screen.getByLabelText('Delete Documento 2'))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar ítem' }))

    await waitFor(() => expect(card(2)).toBeNull())
    await waitFor(() => {
      const focused = document.activeElement?.closest('[data-virtual-key]') as HTMLElement | null
      expect(focused?.dataset.virtualKey).toBe('doc-3')
    })
  })

  it('returns focus to the collection search input when pagination resets', async () => {
    renderGrid()
    await waitFor(() => expect(card(2)).not.toBeNull())
    focusableIn(card(2)!).focus()

    await fireEvent.input(screen.getByRole('searchbox', { name: 'Buscar documentos...' }), {
      target: { value: 'documento' },
    })

    await waitFor(() =>
      expect(screen.getByRole('searchbox', { name: 'Buscar documentos...' })).toHaveFocus()
    )
  })

  it('renders only a window of the collection, not every card', async () => {
    renderGrid(400)

    await waitFor(() => expect(renderedKeys().length).toBeGreaterThan(0))
    await setViewport(0)

    // Two columns, 200px rows, a 400px viewport and overscan: far fewer than 400.
    expect(renderedKeys().length).toBeLessThan(40)
    expect(renderedKeys()[0]).toBe('doc-0')
  })
})
