import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import DocumentExplorer from './DocumentExplorer.svelte'
import type { CollectionItemCardSummary } from '@entropia/store'

const state = vi.hoisted(() => {
  const subscribers = new Set<(value: unknown) => void>()

  const snapshot = {
    // Sitting on the collections list means nothing is auto-expanded, so each
    // test drives the expansion itself.
    history: [{ name: 'collections' as const }],
    current: { name: 'collections' as const },
    canGoBack: false,
    breadcrumb: ['Colecciones'],
  }

  const findCardSummariesPage = vi.fn()

  const store = {
    collections: {
      findAll: vi
        .fn()
        .mockResolvedValue([
          { id: 'col-1', name: 'Colección 1', description: null, createdAt: 1, updatedAt: 1 },
        ]),
      countItems: vi.fn().mockResolvedValue(3),
    },
    items: {
      findCardSummariesPage,
      findCardSummariesByCollection: vi.fn().mockResolvedValue([]),
      findByCollection: vi.fn().mockResolvedValue([]),
    },
    assets: { findByItem: vi.fn().mockResolvedValue([]) },
  }

  function emit() {
    const payload = {
      history: [...snapshot.history],
      current: { ...snapshot.current },
      canGoBack: snapshot.canGoBack,
      breadcrumb: [...snapshot.breadcrumb],
    }
    subscribers.forEach((run) => run(payload))
  }

  return { subscribers, snapshot, store, findCardSummariesPage, emit, navigate: vi.fn() }
})

vi.mock('$lib/navigation', () => ({
  navigation: {
    subscribe(run: (value: unknown) => void) {
      state.subscribers.add(run)
      state.emit()
      return () => state.subscribers.delete(run)
    },
    navigate: state.navigate,
    replace: vi.fn(),
    resetToPath: vi.fn(),
  },
}))

vi.mock('$lib/db', () => ({ getStore: () => state.store }))

function summary(id: string, title: string): CollectionItemCardSummary {
  return {
    id,
    title,
    collectionId: 'col-1',
    metadata: null,
    createdAt: 1,
    updatedAt: 1,
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

function page(rows: Array<[string, string]>, hasMore: boolean): PageResult {
  const items = rows.map(([id, title]) => summary(id, title))
  const last = items[items.length - 1]
  return {
    items,
    hasMore,
    nextCursor: hasMore && last ? { title: last.title, id: last.id } : null,
  }
}

const initialPage = page(
  [
    ['doc-a', 'Alpha'],
    ['doc-b', 'Bravo'],
  ],
  true
)

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => (resolve = res))
  return { promise, resolve }
}

/** The sidebar shows document titles; that is what "a row" means here. */
function documentTitles(): string[] {
  return Array.from(document.querySelectorAll('.explorer__node--item .explorer__node-main')).map(
    (node) => node.textContent?.trim() ?? ''
  )
}

async function expandCollection() {
  await fireEvent.click(
    await screen.findByRole('button', { name: 'Expandir colección Colección 1' })
  )
}

function loadMoreButton() {
  return screen.getByRole('button', { name: 'Cargar más documentos' })
}

describe('DocumentExplorer pagination', () => {
  beforeEach(() => {
    locale.set('es')
    localStorage.clear()
    vi.clearAllMocks()
    state.store.collections.findAll.mockResolvedValue([
      { id: 'col-1', name: 'Colección 1', description: null, createdAt: 1, updatedAt: 1 },
    ])
    state.store.collections.countItems.mockResolvedValue(3)
    state.store.assets.findByItem.mockResolvedValue([])
  })

  it('loads only the first page when a collection is expanded', async () => {
    state.findCardSummariesPage.mockResolvedValue(initialPage)
    render(DocumentExplorer)

    await expandCollection()

    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo']))
    expect(state.findCardSummariesPage).toHaveBeenCalledTimes(1)
    expect(state.findCardSummariesPage).toHaveBeenCalledWith(
      'col-1',
      expect.objectContaining({ cursor: null })
    )
  })

  it('loads page 2 from the current cursor when the continuation button is activated', async () => {
    state.findCardSummariesPage
      .mockResolvedValueOnce(initialPage)
      .mockResolvedValueOnce(page([['doc-c', 'Charlie']], false))
    render(DocumentExplorer)
    await expandCollection()
    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo']))

    await fireEvent.click(loadMoreButton())

    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo', 'Charlie']))
    expect(state.findCardSummariesPage).toHaveBeenLastCalledWith(
      'col-1',
      expect.objectContaining({ cursor: { title: 'Bravo', id: 'doc-b' } })
    )
  })

  it.each([
    ['Enter', '{Enter}'],
    ['Space', ' '],
  ])('activates with %s and issues exactly one request', async (_label, key) => {
    const gate = deferred<PageResult>()
    state.findCardSummariesPage
      .mockResolvedValueOnce(initialPage)
      .mockImplementationOnce(() => gate.promise)
    render(DocumentExplorer)
    await expandCollection()
    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo']))

    const button = loadMoreButton()
    button.focus()
    // A native <button> activates on both keys without any keydown handling of
    // our own; the click it synthesizes is what must fire exactly once.
    await fireEvent.keyDown(button, { key: key === '{Enter}' ? 'Enter' : ' ' })
    await fireEvent.click(button)

    expect(state.findCardSummariesPage).toHaveBeenCalledTimes(2)
    expect(state.findCardSummariesPage).toHaveBeenLastCalledWith(
      'col-1',
      expect.objectContaining({ cursor: { title: 'Bravo', id: 'doc-b' } })
    )
    await waitFor(() => expect(button).toBeDisabled())
    gate.resolve(page([['doc-c', 'Charlie']], false))
  })

  it('deduplicates page 2 rows, preserves the cursor on failure, and retries with it', async () => {
    state.findCardSummariesPage
      .mockResolvedValueOnce(initialPage)
      .mockRejectedValueOnce(new Error('page 2 failed'))
      .mockResolvedValueOnce(
        page(
          [
            ['doc-b', 'Bravo'],
            ['doc-c', 'Charlie'],
          ],
          false
        )
      )
    render(DocumentExplorer)
    await expandCollection()
    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo']))

    await fireEvent.click(loadMoreButton())

    // A failed continuation keeps every row already loaded.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Reintentar' })).toBeInTheDocument()
    )
    expect(documentTitles()).toEqual(['Alpha', 'Bravo'])

    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))

    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo', 'Charlie']))
    expect(state.findCardSummariesPage).toHaveBeenLastCalledWith(
      'col-1',
      expect.objectContaining({ cursor: { title: 'Bravo', id: 'doc-b' } })
    )
  })

  it('removes the continuation button when the collection is exhausted', async () => {
    state.findCardSummariesPage.mockResolvedValue(
      page(
        [
          ['doc-a', 'Alpha'],
          ['doc-b', 'Bravo'],
        ],
        false
      )
    )
    render(DocumentExplorer)
    await expandCollection()

    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo']))
    expect(
      screen.queryByRole('button', { name: 'Cargar más documentos' })
    ).not.toBeInTheDocument()
  })

  it('never walks the whole collection through the all-pages loader', async () => {
    state.findCardSummariesPage
      .mockResolvedValueOnce(initialPage)
      .mockResolvedValueOnce(page([['doc-c', 'Charlie']], false))
    render(DocumentExplorer)
    await expandCollection()
    await waitFor(() => expect(documentTitles()).toEqual(['Alpha', 'Bravo']))

    await fireEvent.click(loadMoreButton())
    await waitFor(() => expect(documentTitles()).toHaveLength(3))

    expect(state.store.items.findCardSummariesByCollection).not.toHaveBeenCalled()
    expect(state.store.items.findByCollection).not.toHaveBeenCalled()
  })
})
