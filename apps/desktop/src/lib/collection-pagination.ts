/**
 * Pagination state for a collection card grid.
 *
 * Everything here is pure and free of Svelte runes so the rules can be tested
 * without a component and without layout. The view owns the `$state` container;
 * this module only ever computes the next value of it.
 */
import type { CollectionItemCardSummary } from '@entropia/store'

/** Position in the collection's `(title COLLATE NOCASE, id)` ordering. */
export type ItemCursor = { title: string; id: string }

/** One page as the repository returns it. */
export type ItemPage = {
  items: CollectionItemCardSummary[]
  nextCursor: ItemCursor | null
  hasMore: boolean
}

export type CollectionPaginationState = {
  items: CollectionItemCardSummary[]
  /** Append defense. It does not replace the reset on an order-key change — a
   *  reordered collection needs page 1 again, not deduplication. */
  loadedIds: Set<string>
  cursor: ItemCursor | null
  hasMore: boolean
  loadingPage: boolean
  /** A failure that cost a continuation, never the initial load. The two are
   *  separate because one leaves a usable grid behind and the other does not. */
  pageError: string | null
}

/** Rows per request. Flat: it does not grow with the collection. */
export const COLLECTION_PAGE_SIZE = 100

/**
 * Rows per request in the sidebar tree. Smaller than the grid's page because
 * the tree is a narrow column with no virtualization: a hundred rows there is
 * a wall, not a page.
 */
export const EXPLORER_PAGE_SIZE = 50

/** How close to the last loaded row the viewport gets before the next page is
 *  requested. Large enough that the fetch usually lands before the user
 *  arrives, small enough that idle scrolling does not pull the whole set. */
export const PREFETCH_ROWS = 6

export function createPaginationState(): CollectionPaginationState {
  return {
    items: [],
    loadedIds: new Set(),
    cursor: null,
    hasMore: true,
    loadingPage: false,
    pageError: null,
  }
}

/**
 * Whether the viewport has come close enough to the end of the loaded rows to
 * justify the next request.
 */
export function shouldLoadNextPage(input: {
  lastVisibleRow: number
  loadedRows: number
  hasMore: boolean
  loadingPage: boolean
  prefetchRows: number
}): boolean {
  if (!input.hasMore || input.loadingPage || input.loadedRows === 0) return false
  return input.lastVisibleRow >= input.loadedRows - input.prefetchRows
}

/**
 * Which loaded row the bottom of the viewport is currently over.
 *
 * The grid is uniform, so the fraction of the scroll range consumed is the same
 * as the fraction of the loaded rows passed. That avoids needing a row height,
 * which matters because happy-dom reports no layout at all.
 */
export function visibleRowFromScroll(input: {
  scrollTop: number
  viewportHeight: number
  scrollHeight: number
  loadedRows: number
}): number {
  if (input.loadedRows === 0 || input.scrollHeight === 0) return 0

  const consumed = (input.scrollTop + input.viewportHeight) / input.scrollHeight
  return Math.min(input.loadedRows, Math.round(consumed * input.loadedRows))
}

/**
 * Append one page, dropping any row already on screen.
 *
 * The cursor is stable under concurrent mutation but the repository can still
 * legitimately hand back a row the client already has — for instance when an
 * item is renamed to a title that sorts before the cursor. Rendering it twice
 * would be the visible bug, so ids win over server order.
 */
export function appendPage(
  state: CollectionPaginationState,
  page: ItemPage
): CollectionPaginationState {
  const loadedIds = new Set(state.loadedIds)
  const fresh = page.items.filter((item) => !loadedIds.has(item.id))
  for (const item of fresh) loadedIds.add(item.id)

  return {
    items: [...state.items, ...fresh],
    loadedIds,
    cursor: page.hasMore ? page.nextCursor : null,
    hasMore: page.hasMore,
    loadingPage: false,
    pageError: null,
  }
}

/**
 * Start over from page 1.
 *
 * A cursor is a position in an ordering. Once the ordering changes — a rename,
 * a new search — every cursor derived from the old one points somewhere that no
 * longer means what it meant, so continuing from it would skip or repeat rows.
 * The only correct answer is to drop the state and reload.
 */
export function resetOnOrderKeyChange(_state: CollectionPaginationState): CollectionPaginationState {
  return createPaginationState()
}

/**
 * Request the next page and fold it into the state.
 *
 * Emits twice on the happy path — once to announce the in-flight request before
 * awaiting, once with the result — so the caller can disable its trigger for
 * the whole round trip rather than only after it returns. Refuses to start when
 * a request is already in flight or the collection is exhausted, which is where
 * "one request at a time" is actually enforced.
 */
export async function loadNextPage(
  state: CollectionPaginationState,
  fetchPage: (args: { cursor: ItemCursor | null }) => Promise<ItemPage>,
  onState: (next: CollectionPaginationState) => void
): Promise<void> {
  if (state.loadingPage || !state.hasMore) return

  const cursor = state.cursor
  onState({ ...state, loadingPage: true, pageError: null })

  try {
    onState(appendPage(state, await fetchPage({ cursor })))
  } catch (e) {
    // The loaded rows and the cursor both survive: the user keeps what they
    // have and the retry resumes from exactly where this attempt started.
    onState({
      ...state,
      loadingPage: false,
      pageError: e instanceof Error ? e.message : String(e),
    })
  }
}
