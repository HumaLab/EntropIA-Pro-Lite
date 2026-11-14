/**
 * Local Qwen3-VL correction frontend client for EntropIA Pro desktop app.
 * Communicates with the Rust Qwen correction backend (llama-server + GGUF).
 * Mirrors the glm-ocr-local.ts surface.
 */

import { invoke } from '@tauri-apps/api/core'

// Only the Pro (local-ML) build ships the local engine; under the API-only
// variant these wrappers stay inert like the llm.ts download surface.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

export interface QwenLocalInfo {
  model_present: boolean
  mmproj_present: boolean
  server_present: boolean
  models_dir: string
  repo_url: string
}

export interface QwenCorrectionDownloadProgressPayload {
  pct: number
  downloaded_bytes: number
  total_bytes: number | null
}

export interface QwenCorrectionDownloadCompletePayload {
  path: string
}

export interface QwenCorrectionDownloadErrorPayload {
  error: string
}

/** Status of the local Qwen correction install (models + server binary). */
export function qwenCorrectionLocalInfo(): Promise<QwenLocalInfo | null> {
  if (OFF) return Promise.resolve(null)
  return invoke<QwenLocalInfo>('qwen_correction_local_info')
}

/** Start downloading the missing local Qwen3-VL correction GGUF files. */
export function qwenCorrectionDownloadModels(): Promise<string> {
  if (OFF) return Promise.resolve('')
  return invoke<string>('qwen_correction_download_models')
}
