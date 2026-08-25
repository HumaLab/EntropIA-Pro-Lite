import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DebouncedAssetTextPersistor } from './item-view-text-persistence'

describe('DebouncedAssetTextPersistor', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('persists the latest scheduled text after the debounce delay', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const afterPersist = vi.fn()
    const persistor = new DebouncedAssetTextPersistor({ delayMs: 500, persist, afterPersist })

    persistor.schedule('asset-1', 'old text')
    persistor.schedule('asset-1', 'new text')

    await vi.advanceTimersByTimeAsync(499)
    expect(persist).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1)

    expect(persist).toHaveBeenCalledTimes(1)
    expect(persist).toHaveBeenCalledWith('asset-1', 'new text')
    expect(afterPersist).toHaveBeenCalledWith('asset-1', 'new text')
  })

  it('does not call afterPersist when persistence fails', async () => {
    const error = new Error('persist failed')
    const persist = vi.fn().mockRejectedValue(error)
    const afterPersist = vi.fn()
    const onError = vi.fn()
    const persistor = new DebouncedAssetTextPersistor({
      delayMs: 500,
      persist,
      afterPersist,
      onError,
    })

    persistor.schedule('asset-1', 'text')
    await vi.advanceTimersByTimeAsync(500)

    expect(afterPersist).not.toHaveBeenCalled()
    expect(onError).toHaveBeenCalledWith(error)
  })

  it('cancels all pending text persistence timers', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const persistor = new DebouncedAssetTextPersistor({ delayMs: 500, persist })

    persistor.schedule('asset-1', 'text')
    persistor.schedule('asset-2', 'other')
    persistor.cancelAll()
    await vi.advanceTimersByTimeAsync(500)

    expect(persist).not.toHaveBeenCalled()
  })

  it('cancels pending work and waits for an in-flight persistence', async () => {
    let resolvePersist!: () => void
    const persist = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolvePersist = resolve
        })
    )
    const persistor = new DebouncedAssetTextPersistor({ delayMs: 500, persist })

    persistor.schedule('pending', 'pending text')
    persistor.schedule('in-flight', 'started text')
    const pendingWait = persistor.cancelAndWait('pending')
    await vi.advanceTimersByTimeAsync(500)
    expect(persist).toHaveBeenCalledWith('in-flight', 'started text')
    await pendingWait

    let waitFinished = false
    const wait = persistor.cancelAndWait('in-flight').then(() => {
      waitFinished = true
    })
    await vi.advanceTimersByTimeAsync(500)
    expect(waitFinished).toBe(false)
    expect(persist).not.toHaveBeenCalledWith('pending', 'pending text')

    resolvePersist()
    await wait
    expect(waitFinished).toBe(true)
  })

  it('waits for every overlapping in-flight persistence for the same asset', async () => {
    const resolvers = new Map<string, () => void>()
    const persist = vi.fn(
      (_assetId: string, text: string) =>
        new Promise<void>((resolve) => {
          resolvers.set(text, resolve)
        })
    )
    const persistor = new DebouncedAssetTextPersistor({ delayMs: 500, persist })

    persistor.schedule('asset-1', 'first')
    await vi.advanceTimersByTimeAsync(500)
    persistor.schedule('asset-1', 'second')
    await vi.advanceTimersByTimeAsync(500)

    let waitFinished = false
    const wait = persistor.cancelAndWait('asset-1').then(() => {
      waitFinished = true
    })
    resolvers.get('second')?.()
    await vi.advanceTimersByTimeAsync(0)
    await Promise.resolve()
    expect(waitFinished).toBe(false)

    resolvers.get('first')?.()
    await wait
    expect(waitFinished).toBe(true)
  })

  it('flushAndWait persists the latest queued payload immediately and waits all asset writes', async () => {
    const resolvers = new Map<string, () => void>()
    const persist = vi.fn(
      (_assetId: string, text: string) =>
        new Promise<void>((resolve) => {
          resolvers.set(text, resolve)
        })
    )
    const afterPersist = vi.fn()
    const persistor = new DebouncedAssetTextPersistor({ delayMs: 500, persist, afterPersist })

    persistor.schedule('asset-1', 'in flight')
    await vi.advanceTimersByTimeAsync(500)
    persistor.schedule('asset-1', 'queued old')
    persistor.schedule('asset-1', 'queued latest')

    let flushFinished = false
    const flush = persistor.flushAndWait('asset-1').then(() => {
      flushFinished = true
    })

    expect(persist).toHaveBeenCalledTimes(1)
    expect(persist).toHaveBeenCalledWith('asset-1', 'in flight')
    expect(persist).not.toHaveBeenCalledWith('asset-1', 'queued old')
    expect(persist).not.toHaveBeenCalledWith('asset-1', 'queued latest')
    expect(flushFinished).toBe(false)

    resolvers.get('in flight')?.()
    await vi.advanceTimersByTimeAsync(0)
    await Promise.resolve()
    expect(persist).toHaveBeenNthCalledWith(2, 'asset-1', 'queued latest')
    expect(persist).not.toHaveBeenCalledWith('asset-1', 'queued old')
    resolvers.get('queued latest')?.()
    await flush
    expect(flushFinished).toBe(true)
    expect(afterPersist).toHaveBeenCalledWith('asset-1', 'queued latest')
  })

  it('flushAndWait rejects persistence failure and keeps the latest payload retryable', async () => {
    const error = new Error('persist failed')
    const persist = vi
      .fn()
      .mockRejectedValueOnce(error)
      .mockResolvedValueOnce(undefined)
    const afterPersist = vi.fn()
    const onError = vi.fn()
    const persistor = new DebouncedAssetTextPersistor({
      delayMs: 500,
      persist,
      afterPersist,
      onError,
    })
    persistor.schedule('asset-1', 'latest local edit')

    await expect(persistor.flushAndWait('asset-1')).rejects.toBe(error)

    expect(onError).toHaveBeenCalledWith(error)
    expect(afterPersist).not.toHaveBeenCalled()
    await persistor.flushAndWait('asset-1')
    expect(persist).toHaveBeenNthCalledWith(2, 'asset-1', 'latest local edit')
    expect(afterPersist).toHaveBeenCalledWith('asset-1', 'latest local edit')
  })

  it('concurrency_review_contract serializes durable writes and commits the newest payload last', async () => {
    const resolvers = new Map<string, () => void>()
    let durableText = ''
    const persist = vi.fn(
      (_assetId: string, text: string) =>
        new Promise<void>((resolve) => {
          resolvers.set(text, () => {
            durableText = text
            resolve()
          })
        })
    )
    const persistor = new DebouncedAssetTextPersistor({ delayMs: 500, persist })

    persistor.schedule('asset-1', 'older in flight')
    await vi.advanceTimersByTimeAsync(500)
    persistor.schedule('asset-1', 'queued before flush')
    const flush = persistor.flushAndWait('asset-1')
    persistor.schedule('asset-1', 'newest during flush')

    expect(persist).toHaveBeenCalledTimes(1)
    expect(persist).toHaveBeenCalledWith('asset-1', 'older in flight')

    resolvers.get('older in flight')?.()
    await vi.advanceTimersByTimeAsync(0)
    await Promise.resolve()

    expect(persist).toHaveBeenCalledTimes(2)
    expect(persist).toHaveBeenLastCalledWith('asset-1', 'newest during flush')
    expect(persist).not.toHaveBeenCalledWith('asset-1', 'queued before flush')

    resolvers.get('newest during flush')?.()
    await flush
    expect(durableText).toBe('newest during flush')
  })
})
