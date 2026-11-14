/**
 * Local GLM-OCR frontend client for EntropIA Pro desktop app.
 * Communicates with the Rust GLM-OCR backend (llama-server + GGUF models).
 * Mirrors the llm.ts local-model surface.
 */

import { invoke } from '@tauri-apps/api/core'

// Only the Pro (local-ML) build ships the local engine; under the API-only
// variant these wrappers stay inert like the llm.ts download surface.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

export interface GlmOcrLocalInfo {
  model_present: boolean
  mmproj_present: boolean
  server_present: boolean
  models_dir: string
  repo_url: string
}

export interface GlmOcrDownloadProgressPayload {
  pct: number
  downloaded_bytes: number
  total_bytes: number | null
}

export interface GlmOcrDownloadCompletePayload {
  path: string
}

export interface GlmOcrDownloadErrorPayload {
  error: string
}

/** Status of the local GLM-OCR install (models + server binary). */
export function glmOcrLocalInfo(): Promise<GlmOcrLocalInfo | null> {
  if (OFF) return Promise.resolve(null)
  return invoke<GlmOcrLocalInfo>('glm_ocr_local_info')
}

/** Start downloading the missing local GLM-OCR GGUF files. */
export function glmOcrDownloadModels(): Promise<string> {
  if (OFF) return Promise.resolve('')
  return invoke<string>('glm_ocr_download_models')
}
