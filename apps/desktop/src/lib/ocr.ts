/**
 * OCR frontend client for EntropIA desktop app.
 * Plain TypeScript (not .svelte.ts) for full testability in Vitest.
 *
 * Communicates with the Rust backend via Tauri invoke + event listeners.
 */

import { invoke } from '@tauri-apps/api/core'

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

export type OcrStatus = 'idle' | 'pending' | 'running' | 'done' | 'error'

export interface OcrProgress {
  assetId: string
  pct: number
  stage: string
}

export interface OcrResult {
  assetId: string
  method: 'native' | 'ocr'
  textLength: number
}

export interface AssetOcrState {
  status: OcrStatus
  progress: number
  stage?: string
  error?: string
  textLength?: number
  method?: string
  textContent?: string
  warning?: string
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload shapes emitted by the Rust backend
// ─────────────────────────────────────────────────────────────────────────────

interface ProgressPayload {
  asset_id: string
  pct: number
  stage: string
}

interface CompletePayload {
  asset_id: string
  method: string
  text_length: number
  text_content: string
  created_page_asset_count?: number
  degradation_reason?: string
}

interface ErrorPayload {
  asset_id: string
  error: string
}

// ─────────────────────────────────────────────────────────────────────────────
// OcrStore
// ─────────────────────────────────────────────────────────────────────────────

const IDLE_STATE: AssetOcrState = { status: 'idle', progress: 0 }

export interface OcrStoreOptions {
  /** Called when an OCR job completes successfully with the assetId and OCR method. */
  onComplete?: (assetId: string, method: string, createdPageAssetCount?: number) => void
}

export class OcrStore {
  private states = new Map<string, AssetOcrState>()
  private cleanupFns: Array<() => void> = []
  private listenGeneration = 0
  private onComplete?: (assetId: string, method: string, createdPageAssetCount?: number) => void

  constructor(options?: OcrStoreOptions) {
    this.onComplete = options?.onComplete
  }

  /** Returns the current OCR state for an asset, or idle if unknown. */
  getState(assetId: string): AssetOcrState {
    return this.states.get(assetId) ?? { ...IDLE_STATE }
  }

  /**
   * Registers Tauri event listeners for ocr:progress, ocr:complete, ocr:error.
   * The `listen` function is injected (from @tauri-apps/api/event) for testability.
   */
  async startListening(
    listen: (event: string, callback: (e: { payload: unknown }) => void) => Promise<() => void>
  ): Promise<void> {
    const generation = ++this.listenGeneration

    const unlistenProgress = await listen('ocr:progress', (e) => {
      const p = e.payload as ProgressPayload
      this._updateState(p.asset_id, { status: 'running', progress: p.pct, stage: p.stage })
    })

    const unlistenComplete = await listen('ocr:complete', (e) => {
      const p = e.payload as CompletePayload
      this._updateState(p.asset_id, {
        status: 'done',
        progress: 100,
        stage: 'done',
        textLength: p.text_length,
        method: p.method,
        textContent: p.text_content,
        warning: p.degradation_reason,
      })
      // Notify caller so views can refresh visible OCR-dependent state.
      if (p.created_page_asset_count === undefined) {
        this.onComplete?.(p.asset_id, p.method)
      } else {
        this.onComplete?.(p.asset_id, p.method, p.created_page_asset_count)
      }
    })

    const unlistenError = await listen('ocr:error', (e) => {
      const p = e.payload as ErrorPayload
      this._updateState(p.asset_id, { status: 'error', error: p.error, stage: 'error' })
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

  /** Merges partial state into the map for the given assetId. */
  _updateState(assetId: string, partial: Partial<AssetOcrState>): void {
    const current = this.states.get(assetId) ?? { ...IDLE_STATE }
    this.states.set(assetId, { ...current, ...partial })
  }

  /** Updates the extracted text content for an asset (user edit). */
  setTextContent(assetId: string, text: string): void {
    const current = this.states.get(assetId)
    if (!current) return
    this.states.set(assetId, { ...current, textContent: text, textLength: text.length })
  }

  /** Returns the extracted text content for an asset, or undefined if not available. */
  getTextContent(assetId: string): string | undefined {
    return this.states.get(assetId)?.textContent
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// OCR Mode
// ─────────────────────────────────────────────────────────────────────────────

export type OcrMode = 'light' | 'high'

// ─────────────────────────────────────────────────────────────────────────────
// extractText — triggers a backend OCR job
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Calls the Rust `extract_text` command to kick off an OCR job.
 * Does not mutate store state; callers own pending-state handling
 * (see runPendingAssetJob in item-view-media-jobs.ts).
 *
 * @param mode - 'light' for native extraction, 'high' for GLM-OCR.
 */
export async function extractText(
  assetId: string,
  assetPath: string,
  assetType: string,
  mode: OcrMode = 'light'
): Promise<void> {
  await invoke('extract_text', { assetId, assetPath, assetType, mode })
}
