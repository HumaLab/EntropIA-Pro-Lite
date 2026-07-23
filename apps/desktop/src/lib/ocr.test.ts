import { describe, it, expect, beforeEach, vi } from 'vitest'
import { OcrStore, extractText } from './ocr'

// Mocks are set up in test-setup.ts:
//   @tauri-apps/api/core  → invoke vi.fn()
//   @tauri-apps/api/event → listen vi.fn() returning Promise<vi.fn()>

const { invoke } = await import('@tauri-apps/api/core')
const { listen } = await import('@tauri-apps/api/event')

describe('OcrStore', () => {
  let store: OcrStore

  beforeEach(() => {
    store = new OcrStore()
    vi.clearAllMocks()
  })

  // ─────────────────────────────────────────────────────────────────────────
  // getState
  // ─────────────────────────────────────────────────────────────────────────

  it('getState returns idle for unknown assetId', () => {
    const state = store.getState('unknown-asset')
    expect(state.status).toBe('idle')
    expect(state.progress).toBe(0)
  })

  it('getState returns idle with no error or textLength for unknown assetId', () => {
    const state = store.getState('another-asset')
    expect(state.status).toBe('idle')
    expect(state.progress).toBe(0)
    expect(state.error).toBeUndefined()
    expect(state.textLength).toBeUndefined()
  })

  // ─────────────────────────────────────────────────────────────────────────
  // extractText
  // ─────────────────────────────────────────────────────────────────────────

  it('extractText invokes extract_text with default light mode', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined)

    await extractText('asset-1', '/path/to/file.pdf', 'pdf')

    expect(invoke).toHaveBeenCalledWith('extract_text', {
      assetId: 'asset-1',
      assetPath: '/path/to/file.pdf',
      assetType: 'pdf',
      mode: 'light',
    })
  })

  it('extractText calls invoke with correct assetId for images', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined)

    await extractText('asset-img', '/images/photo.jpg', 'image')

    expect(invoke).toHaveBeenCalledWith('extract_text', {
      assetId: 'asset-img',
      assetPath: '/images/photo.jpg',
      assetType: 'image',
      mode: 'light',
    })
  })

  it('extractText passes high mode when specified', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined)

    await extractText('asset-1', '/path/to/file.pdf', 'pdf', 'high')

    expect(invoke).toHaveBeenCalledWith('extract_text', {
      assetId: 'asset-1',
      assetPath: '/path/to/file.pdf',
      assetType: 'pdf',
      mode: 'high',
    })
  })

  // ─────────────────────────────────────────────────────────────────────────
  // startListening — ocr:progress
  // ─────────────────────────────────────────────────────────────────────────

  it('startListening on ocr:progress event updates state to running with pct', async () => {
    let progressCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:progress') {
        progressCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    expect(progressCallback).not.toBeNull()

    progressCallback!({ payload: { asset_id: 'asset-1', pct: 45, stage: 'ocr' } })

    const state = store.getState('asset-1')
    expect(state.status).toBe('running')
    expect(state.progress).toBe(45)
    expect(state.stage).toBe('ocr')
  })

  it('startListening on ocr:progress updates to correct pct value', async () => {
    let progressCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:progress') {
        progressCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    progressCallback!({ payload: { asset_id: 'asset-2', pct: 100, stage: 'done' } })

    const state = store.getState('asset-2')
    expect(state.progress).toBe(100)
    expect(state.status).toBe('running')
    expect(state.stage).toBe('done')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // startListening — ocr:complete
  // ─────────────────────────────────────────────────────────────────────────

  it('startListening on ocr:complete event updates status to done with textLength and method', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({
      payload: {
        asset_id: 'asset-3',
        method: 'ocr',
        text_length: 1234,
        text_content: 'hello world',
      },
    })

    const state = store.getState('asset-3')
    expect(state.status).toBe('done')
    expect(state.textLength).toBe(1234)
    expect(state.method).toBe('ocr')
    expect(state.textContent).toBe('hello world')
    expect(state.stage).toBe('done')
  })

  it('startListening on ocr:complete with native method', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({
      payload: {
        asset_id: 'asset-native',
        method: 'native',
        text_length: 500,
        text_content: 'native text',
      },
    })

    const state = store.getState('asset-native')
    expect(state.status).toBe('done')
    expect(state.method).toBe('native')
    expect(state.progress).toBe(100)
    expect(state.textContent).toBe('native text')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // startListening — ocr:error
  // ─────────────────────────────────────────────────────────────────────────

  it('startListening on ocr:error event updates status to error with error message', async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:error') {
        errorCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    errorCallback!({
      payload: { asset_id: 'asset-err', error: 'OCR model not found' },
    })

    const state = store.getState('asset-err')
    expect(state.status).toBe('error')
    expect(state.error).toBe('OCR model not found')
    expect(state.stage).toBe('error')
  })

  it('startListening on ocr:error with different error message', async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:error') {
        errorCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    errorCallback!({
      payload: { asset_id: 'asset-err2', error: 'File not found' },
    })

    const state = store.getState('asset-err2')
    expect(state.status).toBe('error')
    expect(state.error).toBe('File not found')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // stopListening
  // ─────────────────────────────────────────────────────────────────────────

  it('stopListening calls cleanup functions returned by listen', async () => {
    const cleanup1 = vi.fn()
    const cleanup2 = vi.fn()
    const cleanup3 = vi.fn()

    let callCount = 0
    vi.mocked(listen).mockImplementation(() => {
      callCount++
      if (callCount === 1) return Promise.resolve(cleanup1)
      if (callCount === 2) return Promise.resolve(cleanup2)
      return Promise.resolve(cleanup3)
    })

    await store.startListening(listen)
    store.stopListening()

    expect(cleanup1).toHaveBeenCalledOnce()
    expect(cleanup2).toHaveBeenCalledOnce()
    expect(cleanup3).toHaveBeenCalledOnce()
  })

  it('stopListening is safe to call without startListening', () => {
    expect(() => store.stopListening()).not.toThrow()
  })

  it('stopListening before startListening resolves unlistens late registrations', async () => {
    const cleanup = vi.fn()
    let resolveFirstListen: ((unlisten: () => void) => void) | null = null

    let callCount = 0
    vi.mocked(listen).mockImplementation(() => {
      callCount++
      if (callCount === 1) {
        return new Promise((resolve) => {
          resolveFirstListen = resolve
        })
      }
      return Promise.resolve(cleanup)
    })

    const startPromise = store.startListening(listen)

    // Unmount happens while the listen() registrations are still in flight
    store.stopListening()

    resolveFirstListen!(cleanup)
    await startPromise

    // All late registrations must be unlistened immediately, not leaked
    expect(cleanup).toHaveBeenCalledTimes(3)
  })

  // ─────────────────────────────────────────────────────────────────────────
  // onComplete callback — refresh OCR-dependent UI after OCR
  // ─────────────────────────────────────────────────────────────────────────

  it('calls onComplete callback with assetId and method when ocr:complete fires', async () => {
    const onComplete = vi.fn()
    const storeWithCallback = new OcrStore({ onComplete })

    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await storeWithCallback.startListening(listen)

    completeCallback!({
      payload: {
        asset_id: 'asset-ocr-done',
        method: 'ocr',
        text_length: 500,
        text_content: 'extracted text',
      },
    })

    // The onComplete callback must be called with the assetId and OCR method
    expect(onComplete).toHaveBeenCalledWith('asset-ocr-done', 'ocr')
    // State must still be updated correctly
    const state = storeWithCallback.getState('asset-ocr-done')
    expect(state.status).toBe('done')
  })

  it('forwards created PDF page count to refresh page asset views', async () => {
    const onComplete = vi.fn()
    const storeWithCallback = new OcrStore({ onComplete })
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })
    await storeWithCallback.startListening(listen)

    completeCallback!({
      payload: {
        asset_id: 'pdf-asset',
        method: 'pdf_glm_ocr',
        text_length: 0,
        text_content: '',
        created_page_asset_count: 2,
      },
    })

    expect(onComplete).toHaveBeenCalledWith('pdf-asset', 'pdf_glm_ocr', 2)
  })

  it('preserves a completed aggregate PDF result when page splitting degrades', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null
    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'ocr:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })
    await store.startListening(listen)

    completeCallback!({
      payload: {
        asset_id: 'pdf-asset',
        method: 'pdf_glm_ocr',
        text_length: 18,
        text_content: 'aggregate GLM text',
        degradation_reason: 'failed to render PDF page 2',
      },
    })

    expect(store.getState('pdf-asset')).toMatchObject({
      status: 'done',
      textContent: 'aggregate GLM text',
      warning: 'failed to render PDF page 2',
    })
  })
})
