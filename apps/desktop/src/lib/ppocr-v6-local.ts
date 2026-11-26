/**
 * The optional PP-OCRv6 Tiny OCRL profile: frontend client for its three
 * downloaded files (`ocr/ppocr_v6_local.rs`).
 */

import { invoke } from '@tauri-apps/api/core'

// Only the Pro (local-ML) build ships the local OCR engine.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

export interface PpocrV6LocalInfo {
  installed: boolean
  /** The three files together. */
  download_bytes: number
  models_dir: string
  repo_url: string
}

/** Status of the PP-OCRv6 Tiny install. */
export function ppocrV6LocalInfo(): Promise<PpocrV6LocalInfo | null> {
  if (OFF) return Promise.resolve(null)
  return invoke<PpocrV6LocalInfo>('ppocr_v6_local_info')
}

/**
 * Start downloading the missing PP-OCRv6 Tiny files. Progress arrives as
 * `ppocr-v6:download_*` events.
 */
export function ppocrV6DownloadModels(): Promise<string> {
  if (OFF) return Promise.resolve('')
  return invoke<string>('ppocr_v6_download_models')
}

/** Delete the PP-OCRv6 Tiny files; resolves to the bytes freed. */
export function ppocrV6DeleteModels(): Promise<number> {
  if (OFF) return Promise.resolve(0)
  return invoke<number>('ppocr_v6_delete_models')
}
