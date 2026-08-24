import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  LlmStore,
  llmCanRestoreOriginalOcrAsset,
  llmGetResult,
  llmGetResults,
  llmRestoreOriginalOcrAsset,
} from './llm'

const { invoke } = await import('@tauri-apps/api/core')
const { listen } = await import('@tauri-apps/api/event')

describe('llm client target scoping', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('llmGetResults sends targetType for asset-scoped hydration', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([])

    await llmGetResults('asset-1', 'asset')

    expect(invoke).toHaveBeenCalledWith('llm_get_results', {
      targetId: 'asset-1',
      targetType: 'asset',
    })
  })

  it('llmGetResult sends targetType for item-scoped lookups', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(null)

    await llmGetResult('item-1', 'summarize', 'item')

    expect(invoke).toHaveBeenCalledWith('llm_get_result', {
      targetId: 'item-1',
      jobType: 'summarize',
      targetType: 'item',
    })
  })

  it('queries whether an asset has a durable original OCR backup', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(true)

    await expect(llmCanRestoreOriginalOcrAsset('asset-1')).resolves.toBe(true)

    expect(invoke).toHaveBeenCalledWith('llm_can_restore_original_ocr_asset', {
      assetId: 'asset-1',
    })
  })

  it('restores the exact original OCR for an asset', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('OCR original')

    await expect(llmRestoreOriginalOcrAsset('asset-1')).resolves.toBe('OCR original')

    expect(invoke).toHaveBeenCalledWith('llm_restore_original_ocr_asset', {
      assetId: 'asset-1',
    })
  })

  it('loadPersistedResults hydrates only results for the requested target scope', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([
      {
        target_id: 'shared-id',
        target_type: 'asset',
        job_type: 'summarize',
        result: 'asset summary',
        created_at: 1710000000000,
      },
    ])

    const store = new LlmStore()

    await store.loadPersistedResults('shared-id', 'asset')

    expect(invoke).toHaveBeenCalledWith('llm_get_results', {
      targetId: 'shared-id',
      targetType: 'asset',
    })
    expect(store.getState('shared-id').result).toBe('asset summary')
  })
})

describe('LlmStore listener lifecycle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('stopListening calls all unlisten functions registered by startListening', async () => {
    const cleanup = vi.fn()
    vi.mocked(listen).mockImplementation(() => Promise.resolve(cleanup))

    const store = new LlmStore()
    await store.startListening()
    store.stopListening()

    expect(cleanup).toHaveBeenCalledTimes(3) // progress, complete, error
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

    const store = new LlmStore()
    const startPromise = store.startListening()

    // Unmount happens while the listen() registrations are still in flight
    store.stopListening()

    resolveFirstListen!(cleanup)
    await startPromise

    // All late registrations must be unlistened immediately, not leaked
    expect(cleanup).toHaveBeenCalledTimes(3)
  })
})
