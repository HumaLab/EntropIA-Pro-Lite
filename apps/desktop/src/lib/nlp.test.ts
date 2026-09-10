import { describe, it, expect, beforeEach, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  NlpStore,
  backfillAssetEmbeddings,
  embedAsset,
  indexFts,
  extractEntities,
  enrichItem,
  ftsSearch,
  similarAssets,
} from './nlp'

// Mocks are set up in test-setup.ts:
//   @tauri-apps/api/core  → invoke vi.fn()
//   @tauri-apps/api/event → listen vi.fn() returning Promise<vi.fn()>

const { invoke } = await import('@tauri-apps/api/core')
const { listen } = await import('@tauri-apps/api/event')
const currentDir = dirname(fileURLToPath(import.meta.url))

function readRepoFile(relativeFromLib: string): string {
  const absolute = resolve(currentDir, relativeFromLib)
  return readFileSync(absolute, 'utf8')
}

describe('NlpStore', () => {
  let store: NlpStore

  beforeEach(() => {
    store = new NlpStore()
    vi.clearAllMocks()
  })

  // ─────────────────────────────────────────────────────────────────────────
  // getState
  // ─────────────────────────────────────────────────────────────────────────

  it('getState returns idle for all job types for unknown itemId', () => {
    const state = store.getState('unknown-item')
    expect(state.fts).toBe('idle')
    expect(state.embed).toBe('idle')
    expect(state.ner).toBe('idle')
  })

  it('getState returns a fresh copy each call so mutations do not bleed', () => {
    const s1 = store.getState('item-x')
    const s2 = store.getState('item-x')
    // Both return idle; they are separate objects (no reference leak)
    expect(s1).not.toBe(s2)
    expect(s1.fts).toBe('idle')
    expect(s2.fts).toBe('idle')
  })

  it('getState returns independent state per itemId', () => {
    store._setJobStatus('item-a', 'fts', 'done')
    const stateA = store.getState('item-a')
    const stateB = store.getState('item-b')
    expect(stateA.fts).toBe('done')
    expect(stateB.fts).toBe('idle')
  })

  it('getState keeps asset-scoped jobs isolated within the same item', () => {
    store._setJobStatus('item-a', 'embed', 'done', undefined, 'asset-a')

    expect(store.getState('item-a', 'asset-a').embed).toBe('done')
    expect(store.getState('item-a', 'asset-b').embed).toBe('idle')
  })

  it('getState falls back to item-wide job state when an asset has no override', () => {
    store._setJobStatus('item-a', 'fts', 'done')
    store._setJobStatus('item-a', 'embed', 'running', undefined, 'asset-a')

    const assetState = store.getState('item-a', 'asset-a')
    expect(assetState.fts).toBe('done')
    expect(assetState.embed).toBe('running')
    expect(store.getState('item-a', 'asset-b').fts).toBe('done')
    expect(store.getState('item-a', 'asset-b').embed).toBe('idle')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // invoke wrappers
  // ─────────────────────────────────────────────────────────────────────────

  it('indexFts calls invoke with correct command and itemId', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('queued')
    await indexFts('item-1')
    expect(invoke).toHaveBeenCalledWith('index_fts', { itemId: 'item-1' })
  })

  it('extractEntities calls invoke with correct command and itemId', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('queued')
    await extractEntities('item-3')
    expect(invoke).toHaveBeenCalledWith('extract_entities', { itemId: 'item-3' })
  })

  it('enrichItem calls invoke with correct command and itemId', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('queued')
    await enrichItem('item-enrich-1')
    expect(invoke).toHaveBeenCalledWith('enrich_item', { itemId: 'item-enrich-1' })
  })

  it('ftsSearch calls invoke with query and optional collectionId', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([])
    await ftsSearch('historia colonial', 'col-1')
    expect(invoke).toHaveBeenCalledWith('fts_search', {
      query: 'historia colonial',
      collectionId: 'col-1',
    })
  })

  it('ftsSearch calls invoke without collectionId when omitted', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([])
    await ftsSearch('revolución de mayo')
    expect(invoke).toHaveBeenCalledWith('fts_search', {
      query: 'revolución de mayo',
      collectionId: undefined,
    })
  })

  it('ftsSearch returns empty array when invoke returns empty array', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([])
    const results = await ftsSearch('xyz123')
    expect(results).toEqual([])
  })

  it('ftsSearch returns the full array returned by invoke', async () => {
    const mockResults = [
      { itemId: 'item-a', title: 'Acta fundación', rank: -1.2 },
      { itemId: 'item-b', title: 'Crónica colonial', rank: -0.8 },
    ]
    vi.mocked(invoke).mockResolvedValueOnce(mockResults)
    const results = await ftsSearch('colonización')
    expect(results).toHaveLength(2)
    const [first, second] = results
    expect(first?.itemId).toBe('item-a')
    expect(second?.rank).toBe(-0.8)
  })

  it('similarAssets calls invoke with assetId and default limit', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([])
    await similarAssets('asset-4')
    expect(invoke).toHaveBeenCalledWith('similar_assets', { assetId: 'asset-4', limit: 5 })
  })

  it('similarAssets calls invoke with custom limit', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([])
    await similarAssets('asset-5', 7)
    expect(invoke).toHaveBeenCalledWith('similar_assets', { assetId: 'asset-5', limit: 7 })
  })

  it('backfillAssetEmbeddings calls invoke with force and limit options', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ requested: 2, succeeded: 2, failed: 0, failures: [] })
    await backfillAssetEmbeddings({ force: true, limit: 25 })
    expect(invoke).toHaveBeenCalledWith('backfill_asset_embeddings', {
      force: true,
      limit: 25,
    })
  })

  it('embedAsset calls the asset-level backend command used by the EMBED button', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('queued')

    await embedAsset('item-local', 'asset-local')

    expect(invoke).toHaveBeenCalledWith('embed_asset', {
      itemId: 'item-local',
      assetId: 'asset-local',
    })
  })

  // ─────────────────────────────────────────────────────────────────────────
  // _setJobStatus — direct state manipulation
  // ─────────────────────────────────────────────────────────────────────────

  it('_setJobStatus sets fts status to running for given itemId', () => {
    store._setJobStatus('item-s1', 'fts', 'running')
    expect(store.getState('item-s1').fts).toBe('running')
  })

  it('_setJobStatus sets embed status to done', () => {
    store._setJobStatus('item-s2', 'embed', 'done')
    expect(store.getState('item-s2').embed).toBe('done')
  })

  it('_setJobStatus sets error message on errors field', () => {
    store._setJobStatus('item-s3', 'ner', 'error', 'NER engine failed')
    const state = store.getState('item-s3')
    expect(state.ner).toBe('error')
    expect(state.errors?.ner).toBe('NER engine failed')
  })

  it('_setJobStatus preserves other job statuses when updating one', () => {
    store._setJobStatus('item-s4', 'fts', 'done')
    store._setJobStatus('item-s4', 'embed', 'running')
    const state = store.getState('item-s4')
    expect(state.fts).toBe('done')
    expect(state.embed).toBe('running')
    expect(state.ner).toBe('idle')
  })

  it('_setJobStatus sets triples status to pending', () => {
    store._setJobStatus('item-s5', 'triples', 'pending')
    expect(store.getState('item-s5').triples).toBe('pending')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // startListening — nlp:progress
  // ─────────────────────────────────────────────────────────────────────────

  it('nlp:progress for fts job sets fts status to running', async () => {
    let progressCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:progress') {
        progressCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    progressCallback!({ payload: { item_id: 'item-fts', job: 'fts', pct: 50 } })

    const state = store.getState('item-fts')
    expect(state.fts).toBe('running')
    expect(state.embed).toBe('idle')
    expect(state.ner).toBe('idle')
  })

  it('nlp:progress for embed job sets embed status to running', async () => {
    let progressCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:progress') {
        progressCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    progressCallback!({ payload: { item_id: 'item-emb', job: 'embed', pct: 10 } })

    const state = store.getState('item-emb')
    expect(state.embed).toBe('running')
    expect(state.fts).toBe('idle')
    expect(state.ner).toBe('idle')
  })

  it('nlp:progress with asset_id updates only that asset state', async () => {
    let progressCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:progress') {
        progressCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    progressCallback!({
      payload: { item_id: 'item-emb', asset_id: 'asset-a', job: 'embed', pct: 10 },
    })

    expect(store.getState('item-emb', 'asset-a').embed).toBe('running')
    expect(store.getState('item-emb', 'asset-b').embed).toBe('idle')
    expect(store.getState('item-emb').embed).toBe('idle')
  })

  it('nlp:progress for ner job sets ner status to running', async () => {
    let progressCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:progress') {
        progressCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    progressCallback!({ payload: { item_id: 'item-ner', job: 'ner', pct: 10 } })

    const state = store.getState('item-ner')
    expect(state.ner).toBe('running')
    expect(state.fts).toBe('idle')
    expect(state.embed).toBe('idle')
  })

  it('nlp:progress for triples job sets triples status to running', async () => {
    let progressCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:progress') {
        progressCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    progressCallback!({ payload: { item_id: 'item-triples', job: 'triples', pct: 40 } })

    const state = store.getState('item-triples')
    expect(state.triples).toBe('running')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // startListening — nlp:complete
  // ─────────────────────────────────────────────────────────────────────────

  it('nlp:complete for fts job transitions fts to done', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-1', job: 'fts' } })

    const state = store.getState('item-1')
    expect(state.fts).toBe('done')
    expect(state.embed).toBe('idle')
    expect(state.ner).toBe('idle')
  })

  it('nlp:complete for embed job transitions embed to done', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-2', job: 'embed' } })

    expect(store.getState('item-2').embed).toBe('done')
  })

  it('nlp:complete with asset_id does not mark sibling assets done', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-2', asset_id: 'asset-a', job: 'embed' } })

    expect(store.getState('item-2', 'asset-a').embed).toBe('done')
    expect(store.getState('item-2', 'asset-b').embed).toBe('idle')
    expect(store.getState('item-2').embed).toBe('idle')
  })

  it('nlp:complete for ner job transitions ner to done', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-3', job: 'ner' } })

    expect(store.getState('item-3').ner).toBe('done')
  })

  it('nlp:complete with entity_count stores the count for the NER job', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-count', job: 'ner', entity_count: 0 } })

    const state = store.getState('item-count')
    expect(state.ner).toBe('done')
    expect(state.entityCount).toBe(0)
  })

  it('nlp:complete with asset-scoped entity_count keeps the count on that asset', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({
      payload: { item_id: 'item-count', asset_id: 'asset-a', job: 'ner', entity_count: 3 },
    })

    expect(store.getState('item-count', 'asset-a').entityCount).toBe(3)
    expect(store.getState('item-count', 'asset-b').entityCount).toBeUndefined()
    expect(store.getState('item-count').entityCount).toBeUndefined()
  })

  it('nlp:complete without entity_count clears a stale NER count', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-count', job: 'ner', entity_count: 7 } })
    expect(store.getState('item-count').entityCount).toBe(7)

    completeCallback!({ payload: { item_id: 'item-count', job: 'ner' } })
    expect(store.getState('item-count').entityCount).toBeUndefined()
  })

  it('nlp:complete for non-NER jobs does not touch entityCount', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-count', job: 'ner', entity_count: 4 } })
    completeCallback!({ payload: { item_id: 'item-count', job: 'fts' } })

    const state = store.getState('item-count')
    expect(state.fts).toBe('done')
    expect(state.entityCount).toBe(4)
  })

  it('nlp:complete for triples job transitions triples to done', async () => {
    let completeCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:complete') {
        completeCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    completeCallback!({ payload: { item_id: 'item-8', job: 'triples' } })

    expect(store.getState('item-8').triples).toBe('done')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // startListening — nlp:error
  // ─────────────────────────────────────────────────────────────────────────

  it('nlp:error for fts job transitions fts to error with message', async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:error') {
        errorCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    errorCallback!({ payload: { item_id: 'item-err', job: 'fts', error: 'FTS index failed' } })

    const state = store.getState('item-err')
    expect(state.fts).toBe('error')
    expect(state.errors?.fts).toBe('FTS index failed')
  })

  it('nlp:error for embed job transitions embed to error', async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:error') {
        errorCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    errorCallback!({ payload: { item_id: 'item-err2', job: 'embed', error: 'BGE-M3 failed' } })

    const state = store.getState('item-err2')
    expect(state.embed).toBe('error')
    expect(state.errors?.embed).toBe('BGE-M3 failed')
  })

  it('nlp:error with asset_id stores the error only on that asset', async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:error') {
        errorCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    errorCallback!({
      payload: { item_id: 'item-err2', asset_id: 'asset-a', job: 'embed', error: 'BGE-M3 failed' },
    })

    expect(store.getState('item-err2', 'asset-a').embed).toBe('error')
    expect(store.getState('item-err2', 'asset-a').errors?.embed).toBe('BGE-M3 failed')
    expect(store.getState('item-err2', 'asset-b').embed).toBe('idle')
    expect(store.getState('item-err2', 'asset-b').errors?.embed).toBeUndefined()
  })

  it('a later nlp:complete for the same asset clears its stale embed error', async () => {
    const callbacks = new Map<string, (event: { payload: unknown }) => void>()

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      callbacks.set(eventName, callback as (event: { payload: unknown }) => void)
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    const target = { item_id: 'item-retry', asset_id: 'asset-a', job: 'embed' }
    callbacks.get('nlp:error')!({ payload: { ...target, error: 'database is locked' } })
    callbacks.get('nlp:complete')!({ payload: target })

    const state = store.getState('item-retry', 'asset-a')
    expect(state.embed).toBe('done')
    expect(state.errors?.embed).toBeUndefined()
  })

  it('a retry clears the stale error as soon as it starts running', async () => {
    const callbacks = new Map<string, (event: { payload: unknown }) => void>()

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      callbacks.set(eventName, callback as (event: { payload: unknown }) => void)
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    const target = { item_id: 'item-retry2', asset_id: 'asset-a', job: 'embed' }
    callbacks.get('nlp:error')!({ payload: { ...target, error: 'database is locked' } })
    callbacks.get('nlp:progress')!({ payload: { ...target, pct: 10 } })

    const state = store.getState('item-retry2', 'asset-a')
    expect(state.embed).toBe('running')
    expect(state.errors?.embed).toBeUndefined()
  })

  it('clearing one job error keeps the errors of other jobs', () => {
    store._setJobStatus('item-mixed', 'ner', 'error', 'NER engine failed', 'asset-a')
    store._setJobStatus('item-mixed', 'embed', 'error', 'database is locked', 'asset-a')
    store._setJobStatus('item-mixed', 'embed', 'done', undefined, 'asset-a')

    const state = store.getState('item-mixed', 'asset-a')
    expect(state.errors?.embed).toBeUndefined()
    expect(state.errors?.ner).toBe('NER engine failed')
  })

  it('nlp:error for ner job transitions ner to error with message', async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:error') {
        errorCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    errorCallback!({ payload: { item_id: 'item-err3', job: 'ner', error: 'NER engine crashed' } })

    const state = store.getState('item-err3')
    expect(state.ner).toBe('error')
    expect(state.errors?.ner).toBe('NER engine crashed')
  })

  it('nlp:error for triples job transitions triples to error with message', async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null

    vi.mocked(listen).mockImplementation((eventName, callback) => {
      if (eventName === 'nlp:error') {
        errorCallback = callback as (event: { payload: unknown }) => void
      }
      return Promise.resolve(vi.fn())
    })

    await store.startListening(listen)

    errorCallback!({ payload: { item_id: 'item-9', job: 'triples', error: 'triples failed' } })

    const state = store.getState('item-9')
    expect(state.triples).toBe('error')
    expect(state.errors?.triples).toBe('triples failed')
  })

  // ─────────────────────────────────────────────────────────────────────────
  // stopListening
  // ─────────────────────────────────────────────────────────────────────────

  it('stopListening calls all cleanup functions', async () => {
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

  it('stopListening clears the cleanup list so a second call is a no-op', async () => {
    const cleanup = vi.fn()
    vi.mocked(listen).mockImplementation(() => Promise.resolve(cleanup))

    await store.startListening(listen)
    store.stopListening()
    store.stopListening() // second call — should not throw and not call cleanup again

    expect(cleanup).toHaveBeenCalledTimes(3) // 3 listeners registered (progress, complete, error)
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
})

describe('embedding architecture governance', () => {
  it('uses BGE-M3 providers without reintroducing legacy embedding crates', () => {
    const cargoToml = readRepoFile('../../src-tauri/Cargo.toml')
    const embeddingsRs = readRepoFile('../../src-tauri/src/nlp/embeddings.rs')

    // Lightweight embeddings use explicit BGE-M3 providers. This keeps the
    // Windows release path away from legacy embedding subprocess failure modes.
    expect(cargoToml).not.toContain('fastembed-upstream')
    expect(cargoToml).not.toContain('fastembed-shim')
    expect(cargoToml).not.toContain('sqlite-vec-shim')
    expect(cargoToml).not.toContain('fastembed =')
    expect(cargoToml).not.toContain('fastembed-core')
    expect(embeddingsRs).toContain('baai/bge-m3')
    expect(embeddingsRs).toContain('https://openrouter.ai/api/v1/embeddings')
    expect(embeddingsRs).not.toContain('fallback is disabled')
    expect(embeddingsRs).not.toContain(`No hay fallback a ${'fastembed'}`)
    expect(embeddingsRs).not.toContain('Command::new')
    expect(embeddingsRs).not.toContain('script_path')
  })
})
