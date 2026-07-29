/**
 * NLP frontend client for EntropIA desktop app.
 * Plain TypeScript (not .svelte.ts) for full testability in Vitest.
 *
 * Communicates with the Rust backend via Tauri invoke + event listeners.
 * Mirrors the OcrStore architecture.
 */

import { invoke } from '@tauri-apps/api/core'

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

export type NlpJobType = 'fts' | 'embed' | 'ner' | 'triples'
export type NlpStatus = 'idle' | 'pending' | 'running' | 'done' | 'error'

export interface ItemNlpState {
  fts: NlpStatus
  embed: NlpStatus
  ner: NlpStatus
  triples: NlpStatus
  /** Entities persisted by the last completed NER run (NER jobs only). */
  entityCount?: number
  errors?: {
    fts?: string
    embed?: string
    ner?: string
    triples?: string
  }
}

export interface FtsResult {
  itemId: string
  title: string
  rank: number
}

export interface SimilarAsset {
  assetId: string
  itemId: string
  title: string
  collectionId: string
  assetPath: string
  assetType: string
  textPreview?: string
  similarity: number
}

export interface AssetEmbeddingBackfillFailure {
  assetId: string
  itemId: string
  error: string
}

export interface AssetEmbeddingBackfillReport {
  force: boolean
  limit?: number
  totalAssets: number
  assetsWithText: number
  assetsWithEmbedding: number
  assetsMissingEmbedding: number
  requested: number
  succeeded: number
  failed: number
  failures: AssetEmbeddingBackfillFailure[]
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload shapes emitted by the Rust backend
// ─────────────────────────────────────────────────────────────────────────────

interface ProgressPayload {
  item_id: string
  asset_id?: string
  job: string
  pct: number
}

interface CompletePayload {
  item_id: string
  asset_id?: string
  job: string
  /** Entities persisted by NER jobs; absent for non-NER jobs. */
  entity_count?: number
}

interface ErrorPayload {
  item_id: string
  asset_id?: string
  job: string
  error: string
}

// ─────────────────────────────────────────────────────────────────────────────
// NlpStore
// ─────────────────────────────────────────────────────────────────────────────

const IDLE_STATE: ItemNlpState = { fts: 'idle', embed: 'idle', ner: 'idle', triples: 'idle' }
type StoredNlpState = Partial<ItemNlpState>

export class NlpStore {
  private states = new Map<string, StoredNlpState>()
  private cleanupFns: Array<() => void> = []
  private listenGeneration = 0

  /** Returns item NLP state, optionally overlaid with asset-scoped job state. */
  getState(itemId: string, assetId?: string | null): ItemNlpState {
    const itemState = this.states.get(this._key(itemId)) ?? {}
    const assetState = assetId ? (this.states.get(this._key(itemId, assetId)) ?? {}) : {}
    const errors = { ...itemState.errors, ...assetState.errors }
    return {
      ...IDLE_STATE,
      ...itemState,
      ...assetState,
      ...(Object.keys(errors).length > 0 ? { errors } : {}),
    }
  }

  /**
   * Registers Tauri event listeners for nlp:progress, nlp:complete, nlp:error.
   * The `listen` function is injected (from @tauri-apps/api/event) for testability.
   */
  async startListening(
    listen: (event: string, callback: (e: { payload: unknown }) => void) => Promise<() => void>
  ): Promise<void> {
    const generation = ++this.listenGeneration

    const unlistenProgress = await listen('nlp:progress', (e) => {
      const p = e.payload as ProgressPayload
      this._setJobStatus(p.item_id, p.job as NlpJobType, 'running', undefined, p.asset_id)
    })

    const unlistenComplete = await listen('nlp:complete', (e) => {
      const p = e.payload as CompletePayload
      // Record the count before the status flip so consumers reacting to
      // the 'done' transition already see the fresh count.
      if (p.job === 'ner') {
        this._setEntityCount(p.item_id, p.entity_count, p.asset_id)
      }
      this._setJobStatus(p.item_id, p.job as NlpJobType, 'done', undefined, p.asset_id)
    })

    const unlistenError = await listen('nlp:error', (e) => {
      const p = e.payload as ErrorPayload
      this._setJobStatus(p.item_id, p.job as NlpJobType, 'error', p.error, p.asset_id)
    })

    const cleanupFns = [unlistenProgress, unlistenComplete, unlistenError]

    // stopListening may run while the listen() promises above are still in
    // flight; unlisten late registrations immediately instead of leaking them.
    if (generation !== this.listenGeneration) {
      for (const fn of cleanupFns) {
        fn()
      }
      return
    }

    this.cleanupFns = cleanupFns
  }

  /** Calls all cleanup functions returned by listen(), removing event listeners. */
  stopListening(): void {
    this.listenGeneration++
    for (const fn of this.cleanupFns) {
      fn()
    }
    this.cleanupFns = []
  }

  /**
   * Records the entity count reported by the last completed NER run.
   * `undefined` clears a stale count (e.g. a run skipped for lack of text).
   */
  _setEntityCount(itemId: string, entityCount?: number, assetId?: string | null): void {
    const key = this._key(itemId, assetId)
    const updated: StoredNlpState = { ...(this.states.get(key) ?? {}) }
    if (typeof entityCount === 'number') {
      updated.entityCount = entityCount
    } else {
      delete updated.entityCount
    }
    this.states.set(key, updated)
  }

  /** Updates a single job's status for an item or a specific asset within it. */
  _setJobStatus(
    itemId: string,
    job: NlpJobType,
    status: NlpStatus,
    error?: string,
    assetId?: string | null
  ): void {
    const key = this._key(itemId, assetId)
    const current = this.states.get(key) ?? {}
    const updated: StoredNlpState = { ...current, [job]: status }
    if (error) {
      updated.errors = { ...current.errors, [job]: error }
    }
    this.states.set(key, updated)
  }

  private _key(itemId: string, assetId?: string | null): string {
    return assetId ? `${itemId}::${assetId}` : itemId
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Invoke wrappers
// ─────────────────────────────────────────────────────────────────────────────

/** Submit an FTS5 indexing job for `itemId`. */
export async function indexFts(itemId: string): Promise<void> {
  await invoke('index_fts', { itemId })
}

/** Submit an NER extraction job for `itemId`. */
export async function extractEntities(itemId: string): Promise<void> {
  await invoke('extract_entities', { itemId })
}

/** Backward-compatible alias for item-level FTS indexing. */
export async function enrichItem(itemId: string): Promise<void> {
  await invoke('enrich_item', { itemId })
}

// ── Asset-level NLP commands ─────────────────────────────────────────────────
// These process only the selected asset's text, not the entire item.
// Results are stored with both itemId (ownership) and assetId (filtering).

/** Submit an embedding computation job for a specific asset. */
export async function embedAsset(itemId: string, assetId: string): Promise<void> {
  await invoke('embed_asset', { itemId, assetId })
}

/** Batch backfill asset-level embeddings for assets that already have text. */
export async function backfillAssetEmbeddings(
  options: { force?: boolean; limit?: number } = {}
): Promise<AssetEmbeddingBackfillReport> {
  return await invoke('backfill_asset_embeddings', {
    force: options.force,
    limit: options.limit,
  })
}

/** Submit a NER extraction job for a specific asset. */
export async function extractEntitiesForAsset(itemId: string, assetId: string): Promise<void> {
  await invoke('extract_entities_for_asset', { itemId, assetId })
}

/** Submit a semantic triples extraction job for a specific asset. */
export async function extractTriplesForAsset(itemId: string, assetId: string): Promise<void> {
  await invoke('extract_triples_for_asset', { itemId, assetId })
}

/** Search items using FTS5. Returns results ordered by BM25 relevance. */
export async function ftsSearch(query: string, collectionId?: string): Promise<FtsResult[]> {
  return await invoke('fts_search', { query, collectionId })
}

/** Find assets similar to `assetId` via asset-level kNN vector search. */
export async function similarAssets(assetId: string, limit: number = 5): Promise<SimilarAsset[]> {
  return await invoke('similar_assets', { assetId, limit })
}
