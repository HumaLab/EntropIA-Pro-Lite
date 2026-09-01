/**
 * Viewport-prioritized thumbnail generation.
 *
 * Generating a thumbnail is the most expensive thing a collection grid does:
 * every one is an IPC round trip into Rust and, on a cold cache, a full-size
 * image decode. Anchoring that work to the data block — "a page arrived, so
 * generate 100 thumbnails" — makes the cost proportional to what was *loaded*.
 * Anchoring it to the viewport makes it proportional to what was *looked at*,
 * which is the whole point: opening a 10,000 document collection, glancing at
 * the first screenful and closing it should cost about a dozen thumbnails.
 *
 * Four things make that true, and each is tested:
 *  - priority by distance from the viewport center, not by query order;
 *  - a hard concurrency ceiling, continuously refilled rather than run in rigid
 *    sequential rounds;
 *  - cancellation *before* dispatch, so scrolling past a card costs nothing;
 *  - a cache keyed by asset **and path**, so a re-render is free but an edited
 *    image still regenerates.
 */

export type ThumbnailRequest = {
  assetId: string
  path: string
  /** Index of the card in the grid. Distance from the viewport center is the
   *  priority; the units do not matter as long as they are consistent. */
  row: number
}

/** Concurrent IPC calls allowed. Four keeps the bridge busy without starving
 *  the rest of the app of it. */
export const THUMBNAIL_CONCURRENCY = 4

/**
 * The path is part of the key on purpose. Image edits (crop, rotate, erase)
 * write a new versioned file, so keying on the asset id alone would serve the
 * pre-edit thumbnail forever.
 */
export function thumbnailCacheKey(assetId: string, path: string): string {
  return `${assetId}:${path}`
}

type PendingEntry<R extends ThumbnailRequest> = R & {
  priority: number
  settle: Array<(url: string | null) => void>
}

export type ThumbnailQueueOptions<R extends ThumbnailRequest> = {
  /** The IPC call. Signature matches `generateImageThumbnail(path, assetId)`. */
  generate: (path: string, assetId: string) => Promise<string | null>
  /** Called once per successfully generated thumbnail. Failures are not
   *  reported here — a card simply keeps its placeholder. */
  onThumbnail?: (request: R, url: string | null) => void
  concurrency?: number
}

/**
 * `R` lets a caller carry its own identifiers through the queue — the grid
 * needs the item id to find the card, while the queue itself only ever keys on
 * the asset and its path.
 */
export class ThumbnailQueue<R extends ThumbnailRequest = ThumbnailRequest> {
  /** Known results by `assetId:path`. `null` records a failure, so a broken
   *  image is not retried on every render. */
  readonly cache = new Map<string, string | null>()

  #pending: Array<PendingEntry<R>> = []
  #active = new Set<string>()
  #idleWaiters: Array<() => void> = []
  #generate: ThumbnailQueueOptions<R>['generate']
  #onThumbnail: ThumbnailQueueOptions<R>['onThumbnail']
  #concurrency: number

  constructor(options: ThumbnailQueueOptions<R>) {
    this.#generate = options.generate
    this.#onThumbnail = options.onThumbnail
    this.#concurrency = options.concurrency ?? THUMBNAIL_CONCURRENCY
  }

  activeCount(): number {
    return this.#active.size
  }

  pendingCount(): number {
    return this.#pending.length
  }

  /**
   * Queue a batch, ordered by distance from `centerRow`. Already cached or
   * already in-flight assets are skipped; an asset that is merely queued gets
   * its priority updated rather than queued twice.
   */
  enqueueMany(requests: R[], options: { centerRow?: number } = {}): void {
    const centerRow = options.centerRow ?? 0

    for (const request of requests) {
      const key = thumbnailCacheKey(request.assetId, request.path)
      if (this.cache.has(key) || this.#active.has(key)) continue

      const priority = Math.abs(request.row - centerRow)
      const queued = this.#pending.find(
        (entry) => thumbnailCacheKey(entry.assetId, entry.path) === key
      )

      if (queued) {
        queued.priority = priority
        continue
      }

      this.#pending.push({ ...request, priority, settle: [] })
    }

    this.#sortPending()
    this.#pump()
  }

  /**
   * Re-rank everything still queued against a new viewport center. Work already
   * dispatched is left alone: the IPC call has been spent, so finishing it is
   * cheaper than abandoning it.
   */
  reprioritize(centerRow: number): void {
    for (const entry of this.#pending) entry.priority = Math.abs(entry.row - centerRow)
    this.#sortPending()
  }

  /** Drop one queued request. Cancelling before dispatch is what makes leaving
   *  the viewport free rather than merely wasted. */
  cancel(assetId: string, path: string): void {
    const key = thumbnailCacheKey(assetId, path)
    this.#pending = this.#pending.filter((entry) => {
      if (thumbnailCacheKey(entry.assetId, entry.path) !== key) return true
      for (const settle of entry.settle) settle(null)
      return false
    })
  }

  /** Drop every queued request whose asset is no longer in the window. */
  retainOnly(assetIds: Iterable<string>): void {
    const keep = new Set(assetIds)
    this.#pending = this.#pending.filter((entry) => {
      if (keep.has(entry.assetId)) return true
      for (const settle of entry.settle) settle(null)
      return false
    })
  }

  /**
   * Drop everything still queued, but keep what is already known.
   *
   * This is the right move when the same collection is reloaded — a new search,
   * a reset after a rename. The queued work is stale, but a thumbnail that was
   * already generated is still the correct thumbnail for that asset, and
   * throwing it away would re-spend the exact IPC calls this queue exists to
   * avoid.
   */
  cancelAll(): void {
    for (const entry of this.#pending) {
      for (const settle of entry.settle) settle(null)
    }
    this.#pending = []
  }

  /** Forget everything: a different collection shares none of this state. */
  clear(): void {
    this.cancelAll()
    this.cache.clear()
  }

  /** One thumbnail, from cache when known. Goes through the same queue, so it
   *  cannot push concurrency past the ceiling. */
  load(request: { assetId: string; path: string; row?: number }): Promise<string | null> {
    const key = thumbnailCacheKey(request.assetId, request.path)
    const cached = this.cache.get(key)
    if (cached !== undefined) return Promise.resolve(cached)

    return new Promise((resolve) => {
      const queued = this.#pending.find(
        (entry) => thumbnailCacheKey(entry.assetId, entry.path) === key
      )
      if (queued) {
        queued.settle.push(resolve)
        return
      }

      this.#pending.push({
        ...(request as unknown as R),
        row: request.row ?? 0,
        // A direct request is what the caller is waiting on right now, so it
        // outranks anything queued speculatively for the viewport.
        priority: -1,
        settle: [resolve],
      })
      this.#sortPending()
      this.#pump()
    })
  }

  /**
   * Queue a whole visible window and wait for it to drain. Returns the asset
   * ids in the order they were actually dispatched, which is the observable
   * form of "nearest the viewport first".
   */
  async flushVisible(requests: R[], options: { centerRow?: number } = {}): Promise<string[]> {
    const before = this.#dispatchLog.length
    this.enqueueMany(requests, options)
    await this.idle()
    return this.#dispatchLog.slice(before)
  }

  /** Resolves once nothing is queued or in flight. */
  idle(): Promise<void> {
    if (this.#pending.length === 0 && this.#active.size === 0) return Promise.resolve()
    return new Promise((resolve) => this.#idleWaiters.push(resolve))
  }

  #dispatchLog: string[] = []

  #sortPending(): void {
    // Stable within equal priority, so equally distant cards keep grid order.
    this.#pending.sort((a, b) => a.priority - b.priority)
  }

  #pump(): void {
    while (this.#active.size < this.#concurrency && this.#pending.length > 0) {
      void this.#dispatch(this.#pending.shift()!)
    }
  }

  async #dispatch(entry: PendingEntry<R>): Promise<void> {
    const key = thumbnailCacheKey(entry.assetId, entry.path)
    this.#active.add(key)
    this.#dispatchLog.push(entry.assetId)

    let url: string | null = null
    try {
      url = await this.#generate(entry.path, entry.assetId)
      this.cache.set(key, url)
      this.#onThumbnail?.(entry, url)
    } catch (e) {
      // A failed decode is a known outcome, not a reason to try forever.
      this.cache.set(key, null)
      console.warn('[thumbnail-queue] Failed to generate thumbnail for', entry.assetId, e)
    } finally {
      this.#active.delete(key)
      for (const settle of entry.settle) settle(url)
      // Refill continuously rather than in rigid rounds: one slow decode must
      // not hold three idle slots hostage.
      this.#pump()
      this.#notifyIfIdle()
    }
  }

  #notifyIfIdle(): void {
    if (this.#pending.length > 0 || this.#active.size > 0) return
    const waiters = this.#idleWaiters
    this.#idleWaiters = []
    for (const resolve of waiters) resolve()
  }
}

/**
 * Patch one entry of a reactive map in place.
 *
 * The map is a `SvelteMap`, so mutating it is what notifies Svelte. Replacing
 * it with `new Map(map)` on every batch — which is what the previous
 * implementation did — copies the whole map per round of four and turns
 * updating N thumbnails into O(n²) writes.
 */
export function updateThumbnailMeta<T extends object>(
  map: Map<string, T>,
  itemId: string,
  patch: Partial<T>
): Map<string, T> {
  const current = map.get(itemId)
  // The card may have been evicted or the collection swapped while the IPC call
  // was in flight; re-creating the entry would resurrect a stale row.
  if (!current) return map

  map.set(itemId, { ...current, ...patch })
  return map
}
