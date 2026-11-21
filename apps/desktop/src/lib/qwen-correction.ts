/**
 * Local Qwen3-VL correction frontend client for EntropIA Pro desktop app.
 * Communicates with the Rust Qwen correction backend (llama-server + GGUF).
 * Mirrors the glm-ocr-local.ts surface.
 */

import { invoke } from '@tauri-apps/api/core'
import type { LlamaServerSource } from './glm-ocr-local'

// Only the Pro (local-ML) build ships the local engine; under the API-only
// variant these wrappers stay inert like the llm.ts download surface.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

export interface QwenLocalInfo {
  model_present: boolean
  mmproj_present: boolean
  server_present: boolean
  server_source: LlamaServerSource
  /** This machine has a GPU with enough memory for the corrector. */
  gpu_fits: boolean
  /** The memory floor for a non-NVIDIA GPU, in nominal GB. */
  min_vram_gb: number
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
