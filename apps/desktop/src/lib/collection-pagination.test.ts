import { describe, expect, it, vi } from 'vitest'
import type { CollectionItemCardSummary } from '@entropia/store'
import {
  COLLECTION_PAGE_SIZE,
  PREFETCH_ROWS,
  appendPage,
  createPaginationState,
  loadNextPage,
  resetOnOrderKeyChange,
  shouldLoadNextPage,
  visibleRowFromScroll,
  type CollectionPaginationState,
} from './collection-pagination'

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

function stateWith(
  overrides: Partial<CollectionPaginationState> = {}
): CollectionPaginationState {
  return { ...createPaginationState(), ...overrides }
}

describe('shouldLoadNextPage', () => {
  const triggerState = {
    lastVisibleRow: 94,
    loadedRows: 100,
    hasMore: true,
    loadingPage: false,
    prefetchRows: 6,
  }

  it('fires exactly at the prefetch threshold and not one row earlier', () => {
    expect(shouldLoadNextPage(triggerState)).toBe(true)
    expect(shouldLoadNextPage({ ...triggerState, lastVisibleRow: 93 })).toBe(false)
    expect(shouldLoadNextPage({ ...triggerState, lastVisibleRow: 99 })).toBe(true)
  })

  it('never fires while a page request is already in flight', () => {
    expect(shouldLoadNextPage({ ...triggerState, loadingPage: true })).toBe(false)
  })

  it('never fires once the collection is exhausted', () => {
    expect(shouldLoadNextPage({ ...triggerState, hasMore: false })).toBe(false)
  })

  it('does not fire on an empty collection', () => {
    expect(
      shouldLoadNextPage({ ...triggerState, lastVisibleRow: 0, loadedRows: 0, hasMore: false })
    ).toBe(false)
  })

  it('ships a page size and prefetch distance the caller can rely on', () => {
    expect(COLLECTION_PAGE_SIZE).toBe(100)
    expect(PREFETCH_ROWS).toBeGreaterThan(0)
    expect(PREFETCH_ROWS).toBeLessThan(COLLECTION_PAGE_SIZE)
  })
})

describe('visibleRowFromScroll', () => {
  // The grid is uniform, so how far down the scroll range the viewport bottom
  // sits is the same fraction as how far through the loaded rows the user is.
  // No row height needed, which matters because happy-dom has no layout.
  const geometry = { scrollTop: 0, viewportHeight: 600, scrollHeight: 6000, loadedRows: 100 }

  it('reports the last row once the viewport reaches the bottom', () => {
    expect(visibleRowFromScroll({ ...geometry, scrollTop: 5400 })).toBe(100)
  })

  it('reports a proportional row part-way down', () => {
    expect(visibleRowFromScroll({ ...geometry, scrollTop: 2400 })).toBe(50)
  })

  it('reports only the first screenful before any scrolling', () => {
    expect(visibleRowFromScroll(geometry)).toBe(10)
  })

  it('never reports past the loaded rows, even when over-scrolled', () => {
    expect(visibleRowFromScroll({ ...geometry, scrollTop: 99999 })).toBe(100)
  })

  it('reports zero when nothing is loaded or the container has no height', () => {
    expect(visibleRowFromScroll({ ...geometry, loadedRows: 0 })).toBe(0)
    expect(visibleRowFromScroll({ ...geometry, scrollHeight: 0 })).toBe(0)
  })
})

describe('appendPage', () => {
  it('appends new rows and advances the cursor', () => {
    const state = stateWith({
      items: [summary('doc-a', 'Alpha')],
      loadedIds: new Set(['doc-a']),
      cursor: { title: 'Alpha', id: 'doc-a' },
    })

    const next = appendPage(state, {
      items: [summary('doc-b', 'Bravo'), summary('doc-c', 'Charlie')],
      nextCursor: { title: 'Charlie', id: 'doc-c' },
      hasMore: true,
    })

    expect(next.items.map((row) => row.id)).toEqual(['doc-a', 'doc-b', 'doc-c'])
    expect(next.cursor).toEqual({ title: 'Charlie', id: 'doc-c' })
    expect(next.hasMore).toBe(true)
  })

  it('drops a row that is already loaded instead of rendering it twice', () => {
    const state = stateWith({
      items: [summary('doc-a', 'Alpha'), summary('doc-b', 'Bravo')],
      loadedIds: new Set(['doc-a', 'doc-b']),
      cursor: { title: 'Bravo', id: 'doc-b' },
    })

    const next = appendPage(state, {
      items: [summary('doc-b', 'Bravo'), summary('doc-c', 'Charlie')],
      nextCursor: { title: 'Charlie', id: 'doc-c' },
      hasMore: false,
    })

    expect(next.items.map((row) => row.id)).toEqual(['doc-a', 'doc-b', 'doc-c'])
    expect(next.hasMore).toBe(false)
    expect(next.cursor).toBeNull()
  })

  it('clears any previous page error and leaves the input state untouched', () => {
    const state = stateWith({ pageError: 'page 2 failed' })

    const next = appendPage(state, { items: [summary('doc-a', 'Alpha')], nextCursor: null, hasMore: false })

    expect(next.pageError).toBeNull()
    expect(next.loadingPage).toBe(false)
    expect(state.items).toEqual([])
    expect(state.pageError).toBe('page 2 failed')
  })
})

describe('resetOnOrderKeyChange', () => {
  it('clears every piece of pagination state so page 1 can be reloaded', () => {
    const state = stateWith({
      items: [summary('doc-a', 'Alpha'), summary('doc-b', 'Bravo')],
      loadedIds: new Set(['doc-a', 'doc-b']),
      cursor: { title: 'Bravo', id: 'doc-b' },
      hasMore: true,
      loadingPage: true,
      pageError: 'stale',
    })

    const next = resetOnOrderKeyChange(state)

    expect(next.items).toEqual([])
    expect(next.loadedIds.size).toBe(0)
    expect(next.cursor).toBeNull()
    expect(next.hasMore).toBe(true)
    expect(next.loadingPage).toBe(false)
    expect(next.pageError).toBeNull()
  })
})

describe('loadNextPage', () => {
  it('announces the in-flight page before awaiting and appends on success', async () => {
    const state = stateWith({ cursor: { title: 'Bravo', id: 'doc-b' } })
    const seen: CollectionPaginationState[] = []
    const fetchPage = vi.fn().mockResolvedValue({
      items: [summary('doc-c', 'Charlie')],
      nextCursor: null,
      hasMore: false,
    })

    await loadNextPage(state, fetchPage, (next) => seen.push(next))

    expect(fetchPage).toHaveBeenCalledWith({ cursor: { title: 'Bravo', id: 'doc-b' } })
    expect(seen[0]?.loadingPage).toBe(true)
    expect(seen[1]?.loadingPage).toBe(false)
    expect(seen[1]?.items.map((row) => row.id)).toEqual(['doc-c'])
  })

  it('keeps the loaded rows and the cursor when the page request fails', async () => {
    const state = stateWith({
      items: [summary('doc-a', 'Alpha'), summary('doc-b', 'Bravo')],
      loadedIds: new Set(['doc-a', 'doc-b']),
      cursor: { title: 'Bravo', id: 'doc-b' },
    })
    const seen: CollectionPaginationState[] = []

    await loadNextPage(
      state,
      vi.fn().mockRejectedValue(new Error('page 2 failed')),
      (next) => seen.push(next)
    )

    const final = seen[seen.length - 1]!
    expect(final.items.map((row) => row.id)).toEqual(['doc-a', 'doc-b'])
    expect(final.cursor).toEqual({ title: 'Bravo', id: 'doc-b' })
    expect(final.hasMore).toBe(true)
    expect(final.loadingPage).toBe(false)
    expect(final.pageError).toBe('page 2 failed')
  })

  it('retries from the same cursor after a failure', async () => {
    let state = stateWith({
      items: [summary('doc-a', 'Alpha')],
      loadedIds: new Set(['doc-a']),
      cursor: { title: 'Alpha', id: 'doc-a' },
    })
    const fetchPage = vi
      .fn()
      .mockRejectedValueOnce(new Error('page 2 failed'))
      .mockResolvedValueOnce({ items: [summary('doc-b', 'Bravo')], nextCursor: null, hasMore: false })

    await loadNextPage(state, fetchPage, (next) => (state = next))
    await loadNextPage(state, fetchPage, (next) => (state = next))

    expect(fetchPage).toHaveBeenNthCalledWith(1, { cursor: { title: 'Alpha', id: 'doc-a' } })
    expect(fetchPage).toHaveBeenNthCalledWith(2, { cursor: { title: 'Alpha', id: 'doc-a' } })
    expect(state.items.map((row) => row.id)).toEqual(['doc-a', 'doc-b'])
    expect(state.pageError).toBeNull()
  })

  it('refuses to start a second request while one is in flight', async () => {
    const state = stateWith({ loadingPage: true, cursor: { title: 'Alpha', id: 'doc-a' } })
    const fetchPage = vi.fn()

    await loadNextPage(state, fetchPage, () => {})

    expect(fetchPage).not.toHaveBeenCalled()
  })

  it('refuses to request a page past the end of the collection', async () => {
    const state = stateWith({ hasMore: false })
    const fetchPage = vi.fn()

    await loadNextPage(state, fetchPage, () => {})

    expect(fetchPage).not.toHaveBeenCalled()
  })
})
