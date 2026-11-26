/**
 * Local PaddleOCR-VL on llama-server: frontend client for the GGUF models
 * (`ocr/paddle_vl_local.rs`). Same surface as `glm-ocr-local.ts`.
 */

import { invoke } from '@tauri-apps/api/core'

import type { LlamaServerSource } from './glm-ocr-local'

// Only the Pro (local-ML) build ships the local engine.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

export interface PaddleVlLocalInfo {
  model_present: boolean
  mmproj_present: boolean
  server_present: boolean
  server_source: LlamaServerSource
  /** This machine has a GPU with enough memory for PaddleOCR-VL. */
  gpu_fits: boolean
  /** The memory floor for a non-NVIDIA GPU, in nominal GB. */
  min_vram_gb: number
  /** Both GGUF files together. */
  download_bytes: number
  models_dir: string
  repo_url: string
}

/** Status of the PaddleOCR-VL GGUF install (models + server binary). */
export function paddleVlLocalInfo(): Promise<PaddleVlLocalInfo | null> {
  if (OFF) return Promise.resolve(null)
  return invoke<PaddleVlLocalInfo>('paddle_vl_local_info')
}

/**
 * Start downloading the missing PaddleOCR-VL GGUF files. Progress arrives as
 * `paddle-vl:download_*` events.
 */
export function paddleVlDownloadModels(): Promise<string> {
  if (OFF) return Promise.resolve('')
  return invoke<string>('paddle_vl_download_models')
}

/** Delete the PaddleOCR-VL GGUF files; resolves to the bytes freed. */
export function paddleVlDeleteModels(): Promise<number> {
  if (OFF) return Promise.resolve(0)
  return invoke<number>('paddle_vl_delete_models')
}
