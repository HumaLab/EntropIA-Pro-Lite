import { describe, expect, it, vi } from 'vitest'
import { SvelteMap } from 'svelte/reactivity'
import {
  THUMBNAIL_CONCURRENCY,
  ThumbnailQueue,
  thumbnailCacheKey,
  updateThumbnailMeta,
} from './thumbnail-queue'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => (resolve = res))
  return { promise, resolve }
}

function createQueue(
  generate = vi.fn(async (path: string) => `/thumbs${path}`),
  onThumbnail = vi.fn()
) {
  return { queue: new ThumbnailQueue({ generate, onThumbnail }), generate, onThumbnail }
}

const visibleRows = [
  { assetId: 'doc-11', path: '/a/11.png', row: 11 },
  { assetId: 'doc-12', path: '/a/12.png', row: 12 },
  { assetId: 'doc-09', path: '/a/09.png', row: 9 },
  { assetId: 'doc-15', path: '/a/15.png', row: 15 },
]

describe('ThumbnailQueue priority', () => {
  it('prioritizes thumbnails nearest the viewport center first', async () => {
    const { queue } = createQueue()

    const order = await queue.flushVisible(visibleRows, { centerRow: 11 })

    expect(order).toEqual(['doc-11', 'doc-12', 'doc-09', 'doc-15'])
  })

  it('re-prioritizes a pending entry when the viewport moves to it', async () => {
    const gate = deferred<string>()
    const generate = vi.fn((path: string) =>
      path === '/a/01.png' ? gate.promise : Promise.resolve(`/thumbs${path}`)
    )
    const queue = new ThumbnailQueue({ generate, concurrency: 1 })

    queue.enqueueMany(
      [
        { assetId: 'doc-01', path: '/a/01.png', row: 1 },
        { assetId: 'doc-40', path: '/a/40.png', row: 40 },
        { assetId: 'doc-41', path: '/a/41.png', row: 41 },
      ],
      { centerRow: 1 }
    )
    // The user scrolls to row 41 while doc-01 is still in flight.
    queue.reprioritize(41)
    gate.resolve('/thumbs/a/01.png')
    await queue.idle()

    expect(generate.mock.calls.map((call) => call[0])).toEqual([
      '/a/01.png',
      '/a/41.png',
      '/a/40.png',
    ])
  })
})

describe('ThumbnailQueue concurrency, cancellation and cache', () => {
  it('caps IPC concurrency at four, cancels before dispatch, and reuses cache', async () => {
    const pending = deferred<string>()
    const generate = vi.fn(() => pending.promise)
    const queue = new ThumbnailQueue({ generate })

    queue.enqueueMany([
      { assetId: 'doc-01', path: '/a/01.png', row: 1 },
      { assetId: 'doc-02', path: '/a/02.png', row: 2 },
      { assetId: 'doc-03', path: '/a/03.png', row: 3 },
      { assetId: 'doc-04', path: '/a/04.png', row: 4 },
      { assetId: 'doc-05', path: '/a/05.png', row: 5 },
    ])

    queue.cancel('doc-05', '/a/05.png')

    expect(queue.activeCount()).toBe(4)
    expect(generate).toHaveBeenCalledTimes(4)
    expect(generate).not.toHaveBeenCalledWith('/a/05.png', 'doc-05')

    pending.resolve('/thumbs/shared.png')
    await queue.idle()
    // doc-05 was cancelled before dispatch, so it never cost an IPC call.
    expect(generate).toHaveBeenCalledTimes(4)
  })

  it('serves a repeated request from cache instead of calling IPC twice', async () => {
    const { queue, generate } = createQueue()

    await queue.load({ assetId: 'doc-11', path: '/a/11.png' })
    await queue.load({ assetId: 'doc-11', path: '/a/11.png' })

    expect(queue.cache.get('doc-11:/a/11.png')).toBe('/thumbs/a/11.png')
    expect(generate).toHaveBeenCalledTimes(1)
  })

  it('keys the cache on the path as well as the asset, so an edited image regenerates', async () => {
    const { queue, generate } = createQueue()

    await queue.load({ assetId: 'doc-11', path: '/a/11.png' })
    await queue.load({ assetId: 'doc-11', path: '/a/11.v2.png' })

    expect(generate).toHaveBeenCalledTimes(2)
    expect(thumbnailCacheKey('doc-11', '/a/11.png')).not.toBe(
      thumbnailCacheKey('doc-11', '/a/11.v2.png')
    )
  })

  it('never dispatches an asset that is already cached', async () => {
    const { queue, generate } = createQueue()
    await queue.load({ assetId: 'doc-11', path: '/a/11.png' })
    expect(generate).toHaveBeenCalledTimes(1)

    // doc-11 is one of the four visible rows, but its result is already known.
    const dispatched = await queue.flushVisible(visibleRows, { centerRow: 11 })

    expect(dispatched).toEqual(['doc-12', 'doc-09', 'doc-15'])
    expect(generate).toHaveBeenCalledTimes(4)
    expect(
      generate.mock.calls.filter(([path]) => path === '/a/11.png')
    ).toHaveLength(1)
  })

  it('drops everything outside the window when the viewport jumps', async () => {
    const gate = deferred<string>()
    let calls = 0
    const generate = vi.fn(() => {
      calls += 1
      return calls === 1 ? gate.promise : Promise.resolve('/thumbs/later.png')
    })
    const queue = new ThumbnailQueue({ generate, concurrency: 1 })

    // doc-11 is dispatched; 12, 09 and 15 are still queued behind it.
    queue.enqueueMany(visibleRows, { centerRow: 11 })
    expect(queue.pendingCount()).toBe(3)

    queue.retainOnly(['doc-15'])
    expect(queue.pendingCount()).toBe(1)

    gate.resolve('/thumbs/11.png')
    await queue.idle()

    // Only doc-11 (already in flight) and the retained doc-15 ever cost IPC.
    expect(generate).toHaveBeenCalledTimes(2)
    expect(generate).toHaveBeenCalledWith('/a/15.png', 'doc-15')
    expect(generate).not.toHaveBeenCalledWith('/a/12.png', 'doc-12')
    expect(queue.pendingCount()).toBe(0)
  })

  it('reports a failed thumbnail without stalling the rest of the queue', async () => {
    const generate = vi.fn((path: string) =>
      path === '/a/12.png' ? Promise.reject(new Error('decode failed')) : Promise.resolve(`/t${path}`)
    )
    const onThumbnail = vi.fn()
    const queue = new ThumbnailQueue({ generate, onThumbnail })

    await queue.flushVisible(visibleRows, { centerRow: 11 })

    expect(onThumbnail).toHaveBeenCalledTimes(3)
    expect(onThumbnail).not.toHaveBeenCalledWith(
      expect.objectContaining({ assetId: 'doc-12' }),
      expect.anything()
    )
    expect(queue.activeCount()).toBe(0)
  })

  it('exposes the concurrency ceiling it enforces', () => {
    expect(THUMBNAIL_CONCURRENCY).toBe(4)
  })
})

describe('updateThumbnailMeta', () => {
  it('mutates the same SvelteMap instance in place', () => {
    const map = new SvelteMap([['doc-11', { thumbnailUrl: '/thumbs/doc-11.png' }]])

    const next = updateThumbnailMeta(map, 'doc-11', { thumbnailUrl: '/thumbs/doc-11-2.png' })

    expect(next).toBe(map)
    expect(next.get('doc-11')).toEqual({ thumbnailUrl: '/thumbs/doc-11-2.png' })
  })

  it('leaves an entry that is no longer present alone', () => {
    const map = new SvelteMap<string, { thumbnailUrl: string | null }>()

    const next = updateThumbnailMeta(map, 'doc-gone', { thumbnailUrl: '/thumbs/x.png' })

    expect(next).toBe(map)
    expect(next.has('doc-gone')).toBe(false)
  })

  it('merges the patch instead of replacing the whole entry', () => {
    type CardMeta = { assetCount: number; thumbnailUrl: string | null; primaryAssetPath: string }
    const map = new SvelteMap<string, CardMeta>([
      ['doc-11', { assetCount: 3, thumbnailUrl: null, primaryAssetPath: '/a/11.png' }],
    ])

    updateThumbnailMeta(map, 'doc-11', { thumbnailUrl: '/thumbs/11.png' })

    expect(map.get('doc-11')).toEqual({
      assetCount: 3,
      thumbnailUrl: '/thumbs/11.png',
      primaryAssetPath: '/a/11.png',
    })
  })
})
