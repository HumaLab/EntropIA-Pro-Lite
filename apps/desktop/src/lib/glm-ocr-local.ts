/**
 * Local GLM-OCR frontend client for EntropIA Pro desktop app.
 * Communicates with the Rust GLM-OCR backend (llama-server + GGUF models).
 * Mirrors the llm.ts local-model surface.
 */

import { invoke } from '@tauri-apps/api/core'

// Only the Pro (local-ML) build ships the local engine; under the API-only
// variant these wrappers stay inert like the llm.ts download surface.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

/**
 * Where the build gets llama-server: shipped by the installer (the Linux
 * CUDA build for NVIDIA), downloaded on demand in the build the machine's GPU
 * runs (CUDA, HIP, Vulkan or Metal), or not offered (no GPU with enough
 * memory, where neither local engine runs).
 */
export type LlamaServerSource = 'bundled' | 'download' | 'requires_gpu'

export interface GlmOcrLocalInfo {
  model_present: boolean
  mmproj_present: boolean
  server_present: boolean
  server_source: LlamaServerSource
  /** This machine has a GPU with enough memory for GLM-OCR. */
  gpu_fits: boolean
  /** The memory floor for a non-NVIDIA GPU, in nominal GB. */
  min_vram_gb: number
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

/**
 * Start downloading llama-server, shared by GLM-OCR and the Qwen corrector.
 * Progress arrives as `llama-server:download_*` events.
 */
export function llamaServerDownload(): Promise<string> {
  if (OFF) return Promise.resolve('')
  return invoke<string>('llama_server_download')
}
