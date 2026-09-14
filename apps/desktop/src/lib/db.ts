import { invoke } from '@tauri-apps/api/core'
import { initStore, type StoreApi } from '@entropia/store'
import { createTauriDbClient } from './tauri-db-client'
import { ensureSyncCapture } from '$lib/sync'

let _store: StoreApi | null = null

export interface ProcessingInitSummary {
  ready: boolean
  migration: string
  pendingBatches: number
  activeTasks: number
  interruptedTasks: number
  failedTasks: number
  succeededTasks: number
}

let _processing: ProcessingInitSummary | null = null
let _processingError: string | null = null

/**
 * Verifies the batch-processing schema gate after migrations ran. The backend
 * checks the 0032 migration row plus the effective PRAGMAs and returns the
 * durable queue counters for the recovery banner (Unidad 3+).
 *
 * A `schema_not_ready` failure leaves the batch UI in a recoverable error
 * state via {@link getProcessingInitState} — it never starts queue workers
 * regardless (no best-effort catch that proceeds anyway).
 */
export async function ensureProcessingInitialized(): Promise<ProcessingInitSummary> {
  const summary = await invoke<ProcessingInitSummary>('processing_initialize')
  _processing = summary
  _processingError = null
  return summary
}

export function getProcessingInitState(): {
  summary: ProcessingInitSummary | null
  error: string | null
} {
  return { summary: _processing, error: _processingError }
}

export async function initDb(): Promise<void> {
  _store = await initStore(createTauriDbClient())
  // Sync capture bootstrap (DESIGN §6.1): now that every migration has run and
  // all synced tables exist, ensure the sync schema + capture triggers. The
  // backend already ran this at setup for tables that existed then; this covers
  // tables created by the JS migrations. Best-effort — never block app init.
  try {
    await ensureSyncCapture()
  } catch (error) {
    console.error('[sync] ensureSyncCapture failed:', error)
  }
  // Batch queue gate (plan-lote.md Unidad 1): verify the durable schema after
  // the JS migrations. Records the outcome for the settings tab; queue
  // admission/scheduling (Unidades 2-4) enforce readiness on top of this.
  try {
    await ensureProcessingInitialized()
  } catch (error) {
    _processing = null
    _processingError = error instanceof Error ? error.message : String(error)
    console.error('[processing] processing_initialize failed:', error)
  }
}

export function getStore(): StoreApi {
  if (!_store) throw new Error('Store not initialized. Call initDb() first.')
  return _store
}
