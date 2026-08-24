/**
 * LLM frontend client for EntropIA Pro desktop app.
 * Communicates with the Rust LLM backend (Gemma 4 via llama.cpp).
 * Mirrors the NlpStore architecture.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

// This module stays LIVE in BOTH variants — ItemView statically needs LlmStore,
// the llm* invoke wrappers and llmIsAvailable (they hit the unified backend).
// Only the local-model/download surface flips inert under the API-only variant.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

export type LlmJobType =
  | 'correct_ocr'
  | 'extract_entities'
  | 'extract_triples'
  | 'summarize'
  | 'classify'
  | 'ask'

export type LlmStatus = 'idle' | 'pending' | 'running' | 'done' | 'error'
export type LlmTargetType = 'asset' | 'item' | 'collection'

export interface ItemLlmState {
  status: LlmStatus
  activeJob: LlmJobType | null
  result: string | null
  error: string | null
}

export interface LlmResultEntry {
  target_id: string
  target_type: LlmTargetType | 'unknown'
  job_type: string
  result: string
  created_at: number
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload shapes emitted by the Rust backend
// ─────────────────────────────────────────────────────────────────────────────

interface LlmProgressPayload {
  id: string
  job: string
  pct: number
}

interface LlmCompletePayload {
  id: string
  job: string
  result: string
}

interface LlmErrorPayload {
  id: string
  job: string
  error: string
}

// ─────────────────────────────────────────────────────────────────────────────
// Store
// ─────────────────────────────────────────────────────────────────────────────

export class LlmStore {
  private state: Map<string, ItemLlmState> = new Map()
  private listeners: Array<() => void> = []
  private unlisteners: UnlistenFn[] = []
  private listenGeneration = 0
  private onComplete?: (id: string, job: string, result: string) => void
  private onCorrectOcr?: (id: string, result: string) => void
  private onError?: (id: string, job: string, error: string) => void

  constructor(opts?: {
    onComplete?: (id: string, job: string, result: string) => void
    onCorrectOcr?: (id: string, result: string) => void
    onError?: (id: string, job: string, error: string) => void
  }) {
    this.onComplete = opts?.onComplete
    this.onCorrectOcr = opts?.onCorrectOcr
    this.onError = opts?.onError
  }

  private defaultState(): ItemLlmState {
    return { status: 'idle', activeJob: null, result: null, error: null }
  }

  getState(id: string): ItemLlmState {
    return this.state.get(id) ?? this.defaultState()
  }

  private update(id: string, patch: Partial<ItemLlmState>) {
    const current = this.getState(id)
    this.state.set(id, { ...current, ...patch })
    this.listeners.forEach((fn) => fn())
  }

  /**
   * Hydrate the store from persisted results for a given target.
   * Call this on mount to restore state after a page reload.
   */
  async loadPersistedResults(targetId: string, targetType: LlmTargetType = 'item'): Promise<void> {
    try {
      const results: LlmResultEntry[] = await invoke('llm_get_results', { targetId, targetType })
      for (const entry of results) {
        this.update(entry.target_id, {
          status: 'done',
          activeJob: null,
          result: entry.result,
          error: null,
        })
        // Notify about persisted OCRC results — the text was already replaced
        // in a previous session, so we just need to track that OCRC was done.
        if (entry.job_type === 'correct_ocr') {
          this.onCorrectOcr?.(entry.target_id, entry.result)
        }
      }
    } catch (e) {
      // Silently degrade — persisted results are optional
      console.warn('[LlmStore] Failed to load persisted results:', e)
    }
  }

  onChange(fn: () => void) {
    this.listeners.push(fn)
  }

  async startListening() {
    const generation = ++this.listenGeneration

    const unlisteners = [
      await listen<LlmProgressPayload>('llm:progress', (event) => {
        const { id, job, pct } = event.payload
        this.update(id, {
          status: pct < 100 ? 'running' : 'done',
          activeJob: job as LlmJobType,
        })
      }),
      await listen<LlmCompletePayload>('llm:complete', (event) => {
        const { id, job, result } = event.payload
        this.update(id, {
          status: 'done',
          activeJob: null,
          result,
          error: null,
        })
        this.onComplete?.(id, job, result)
        // Notify about OCRC completion — caller needs to replace OCR text
        if (job === 'correct_ocr') {
          this.onCorrectOcr?.(id, result)
        }
      }),
      await listen<LlmErrorPayload>('llm:error', (event) => {
        const { id, job, error } = event.payload
        this.update(id, {
          status: 'error',
          activeJob: null,
          error,
        })
        this.onError?.(id, job, error)
      }),
    ]

    // stopListening may run while the listen() promises above are still in
    // flight; unlisten late registrations immediately instead of leaking them.
    if (generation !== this.listenGeneration) {
      unlisteners.forEach((fn) => fn())
      return
    }

    this.unlisteners.push(...unlisteners)
  }

  stopListening() {
    this.listenGeneration++
    this.unlisteners.forEach((fn) => fn())
    this.unlisteners = []
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Invoke helpers
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// Invoke helpers — item-level (legacy, concatenates all assets)
// ─────────────────────────────────────────────────────────────────────────────

export function llmCorrectOcr(itemId: string): Promise<string> {
  return invoke<string>('llm_correct_ocr', { itemId })
}

export function llmExtractEntities(itemId: string): Promise<string> {
  return invoke<string>('llm_extract_entities', { itemId })
}

export function llmExtractTriples(itemId: string): Promise<string> {
  return invoke<string>('llm_extract_triples', { itemId })
}

export function llmSummarize(itemId: string): Promise<string> {
  return invoke<string>('llm_summarize', { itemId })
}

export function llmClassify(itemId: string, categories: string[]): Promise<string> {
  return invoke<string>('llm_classify', { itemId, categories })
}

export function llmAsk(collectionId: string, question: string): Promise<string> {
  return invoke<string>('llm_ask', { collectionId, question })
}

// ─────────────────────────────────────────────────────────────────────────────
// Invoke helpers — asset-level (single page, avoids context overflow)
// ─────────────────────────────────────────────────────────────────────────────

export function llmCorrectOcrAsset(assetId: string): Promise<string> {
  return invoke<string>('llm_correct_ocr_asset', { assetId })
}

export function llmCanRestoreOriginalOcrAsset(assetId: string): Promise<boolean> {
  return invoke<boolean>('llm_can_restore_original_ocr_asset', { assetId })
}

export function llmRestoreOriginalOcrAsset(assetId: string): Promise<string> {
  return invoke<string>('llm_restore_original_ocr_asset', { assetId })
}

export function llmExtractEntitiesAsset(assetId: string): Promise<string> {
  return invoke<string>('llm_extract_entities_asset', { assetId })
}

export function llmExtractTriplesAsset(assetId: string): Promise<string> {
  return invoke<string>('llm_extract_triples_asset', { assetId })
}

export function llmSummarizeAsset(assetId: string): Promise<string> {
  return invoke<string>('llm_summarize_asset', { assetId })
}

/** Retrieve all latest LLM results for a target (item or collection). */
export function llmGetResults(
  targetId: string,
  targetType: LlmTargetType = 'item'
): Promise<LlmResultEntry[]> {
  return invoke<LlmResultEntry[]>('llm_get_results', { targetId, targetType })
}

/** Retrieve the latest single LLM result for a target + job type. */
export function llmGetResult(
  targetId: string,
  jobType: string,
  targetType: LlmTargetType = 'item'
): Promise<LlmResultEntry | null> {
  return invoke<LlmResultEntry | null>('llm_get_result', { targetId, jobType, targetType })
}

/** Check if the LLM engine (Gemma 4) is available and ready to accept jobs. */
export function llmIsAvailable(): Promise<boolean> {
  return invoke<boolean>('llm_is_available')
}

/** Check if the remote provider required by asset-level OCR correction is configured. */
export function llmOcrCorrectionIsAvailable(): Promise<boolean> {
  return invoke<boolean>('llm_ocr_correction_is_available')
}

/** Detailed status of the local GGUF model file. */
export interface LocalModelInfo {
  exists: boolean
  available: boolean
  can_auto_download: boolean
  disabled_reason: string | null
  path: string
  size_bytes: number | null
  filename: string
  source_url: string
}

/** Query whether the local model file exists, its resolved path, and size. */
export function llmLocalModelInfo(): Promise<LocalModelInfo> {
  if (OFF) {
    return Promise.resolve({
      exists: false,
      available: false,
      can_auto_download: false,
      disabled_reason: 'API-only build',
      path: '',
      size_bytes: null,
      filename: '',
      source_url: '',
    })
  }
  return invoke<LocalModelInfo>('llm_local_model_info')
}

/** Open the models directory in the system file manager. */
export function llmOpenModelsDir(): Promise<void> {
  if (OFF) return Promise.resolve()
  return invoke<void>('llm_open_models_dir')
}

// ─────────────────────────────────────────────────────────────────────────────
// Download
// ─────────────────────────────────────────────────────────────────────────────

export interface LlmDownloadProgressPayload {
  pct: number
  downloaded_bytes: number
  total_bytes: number | null
}

export interface LlmDownloadCompletePayload {
  path: string
}

export interface LlmDownloadErrorPayload {
  error: string
}

/** Start downloading the local model from the configured source URL. */
export function llmDownloadModel(): Promise<string> {
  if (OFF) return Promise.resolve('')
  return invoke<string>('llm_download_model')
}
