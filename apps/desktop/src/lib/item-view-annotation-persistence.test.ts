import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Annotation as StoreAnnotation } from '@entropia/store'
import type { ViewerAnnotation } from '@entropia/ui'
import {
  DebouncedAnnotationPersistor,
  loadViewerAnnotationsForAsset,
  toAnnotationPersistenceInputs,
  toViewerAnnotations,
} from './item-view-annotation-persistence'

function annotation(overrides: Partial<ViewerAnnotation> = {}): ViewerAnnotation {
  const base: ViewerAnnotation = {
    id: 'annotation-1',
    assetId: 'asset-1',
    page: 1,
    kind: 'rectangle',
    color: 'var(--color-accent)',
    x: 0.1,
    y: 0.2,
    width: 0.3,
    height: 0.4,
    createdAt: 1,
    updatedAt: 1,
  }

  return Object.assign(base, overrides)
}

function storeAnnotation(overrides: Partial<StoreAnnotation> = {}): StoreAnnotation {
  const base: StoreAnnotation = {
    id: 'annotation-1',
    assetId: 'asset-1',
    page: 1,
    kind: 'rectangle',
    color: 'var(--color-accent)',
    x: 0.1,
    y: 0.2,
    width: 0.3,
    height: 0.4,
    createdAt: 1,
    updatedAt: 1,
  }

  return Object.assign(base, overrides)
}

describe('toAnnotationPersistenceInputs', () => {
  it('keeps only persistence fields', () => {
    const inputs = toAnnotationPersistenceInputs([
      annotation({
        id: 'annotation-1',
        kind: 'underline',
        color: '#ff0000',
        x: 0.15,
        y: 0.25,
        width: 0.35,
        height: 0.45,
      }),
    ])

    expect(inputs).toEqual([
      {
        kind: 'underline',
        color: '#ff0000',
        x: 0.15,
        y: 0.25,
        width: 0.35,
        height: 0.45,
      },
    ])
  })
})

describe('toViewerAnnotations', () => {
  it('maps stored annotation kinds to viewer annotation kinds', () => {
    const annotations = toViewerAnnotations([
      storeAnnotation({
        id: 'annotation-1',
        kind: 'underline',
        color: '#ff0000',
      }),
    ])

    expect(annotations).toEqual([
      {
        id: 'annotation-1',
        assetId: 'asset-1',
        page: 1,
        kind: 'underline',
        color: '#ff0000',
        x: 0.1,
        y: 0.2,
        width: 0.3,
        height: 0.4,
        createdAt: 1,
        updatedAt: 1,
      },
    ])
  })
})

describe('loadViewerAnnotationsForAsset', () => {
  it('loads annotations for the requested page through the injected finder', async () => {
    const storedAnnotations = [storeAnnotation({ id: 'annotation-2' })]
    const findByAsset = vi.fn().mockResolvedValue(storedAnnotations)

    const annotations = await loadViewerAnnotationsForAsset('asset-1', 2, findByAsset)

    expect(findByAsset).toHaveBeenCalledWith('asset-1', 2)
    expect(annotations).toEqual(toViewerAnnotations(storedAnnotations))
  })
})

describe('DebouncedAnnotationPersistor', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('persists the latest scheduled annotations after the debounce delay', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const persistor = new DebouncedAnnotationPersistor({ delayMs: 500, persist })
    const oldAnnotations = [annotation({ id: 'old', x: 0.1 })]
    const newAnnotations = [annotation({ id: 'new', x: 0.5 })]

    persistor.schedule('asset-1', 2, oldAnnotations)
    persistor.schedule('asset-1', 2, newAnnotations)

    await vi.advanceTimersByTimeAsync(499)
    expect(persist).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1)

    expect(persist).toHaveBeenCalledTimes(1)
    expect(persist).toHaveBeenCalledWith('asset-1', 2, newAnnotations)
    expect(persist).not.toHaveBeenCalledWith('asset-1', 2, oldAnnotations)
  })

  it('keeps independent pending saves for different pages', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const persistor = new DebouncedAnnotationPersistor({ delayMs: 500, persist })
    const pageOne = [annotation({ id: 'page-1', page: 1 })]
    const pageTwo = [annotation({ id: 'page-2', page: 2 })]

    persistor.schedule('asset-1', 1, pageOne)
    persistor.schedule('asset-1', 2, pageTwo)
    await vi.advanceTimersByTimeAsync(500)

    expect(persist).toHaveBeenCalledTimes(2)
    expect(persist).toHaveBeenCalledWith('asset-1', 1, pageOne)
    expect(persist).toHaveBeenCalledWith('asset-1', 2, pageTwo)
  })

  it('flushes a pending annotation save immediately', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const persistor = new DebouncedAnnotationPersistor({ delayMs: 500, persist })
    const annotations = [annotation()]

    persistor.schedule('asset-1', 1, annotations)
    await persistor.flushPending()
    await vi.advanceTimersByTimeAsync(500)

    expect(persist).toHaveBeenCalledTimes(1)
    expect(persist).toHaveBeenCalledWith('asset-1', 1, annotations)
    expect(persistor.getPendingAssetId()).toBeNull()
  })

  it('exposes the pending asset id until the save runs', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const persistor = new DebouncedAnnotationPersistor({ delayMs: 500, persist })

    expect(persistor.getPendingAssetId()).toBeNull()

    persistor.schedule('asset-1', 1, [annotation()])
    expect(persistor.getPendingAssetId()).toBe('asset-1')

    await vi.advanceTimersByTimeAsync(500)
    expect(persistor.getPendingAssetId()).toBeNull()
  })

  it('cancels all pending annotation persistence', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const persistor = new DebouncedAnnotationPersistor({ delayMs: 500, persist })

    persistor.schedule('asset-1', 1, [annotation()])
    persistor.cancelAll()
    await vi.advanceTimersByTimeAsync(500)

    expect(persist).not.toHaveBeenCalled()
    expect(persistor.getPendingAssetId()).toBeNull()
  })

  it('reports a failed scheduled persist through onError instead of rejecting', async () => {
    const error = new Error('persist failed')
    const persist = vi.fn().mockRejectedValue(error)
    const onError = vi.fn()
    const persistor = new DebouncedAnnotationPersistor({ delayMs: 500, persist, onError })

    persistor.schedule('asset-1', 1, [annotation()])
    await vi.advanceTimersByTimeAsync(500)

    expect(persist).toHaveBeenCalledTimes(1)
    expect(onError).toHaveBeenCalledWith(error)
    expect(persistor.getPendingAssetId()).toBe('asset-1')
  })

  it('reports a failed flushPending persist through onError instead of rejecting', async () => {
    const error = new Error('persist failed')
    const persist = vi.fn().mockRejectedValue(error)
    const onError = vi.fn()
    const persistor = new DebouncedAnnotationPersistor({ delayMs: 500, persist, onError })

    persistor.schedule('asset-1', 1, [annotation()])
    await expect(persistor.flushPending()).resolves.toBeUndefined()

    expect(persist).toHaveBeenCalledTimes(1)
    expect(onError).toHaveBeenCalledWith(error)
    expect(persistor.getPendingAssetId()).toBe('asset-1')
  })
})
