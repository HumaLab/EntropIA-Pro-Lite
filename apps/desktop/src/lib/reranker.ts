import { invoke } from '@tauri-apps/api/core'

const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

export interface LocalRerankerModelFileInfo {
  filename: string
  sourcePath: string
  destination: string
  expectedSizeBytes: number
  actualSizeBytes: number | null
  valid: boolean
}

export interface LocalRerankerModelInfo {
  available: boolean
  canAutoDownload: boolean
  directory: string
  path: string
  requiredFiles: LocalRerankerModelFileInfo[]
  sourceRepo: string
}

export interface RerankerDownloadProgressPayload {
  pct: number
  downloaded_bytes: number
  total_bytes: number
  file: string
}

export interface RerankerDownloadCompletePayload {
  path: string
}

export interface RerankerDownloadErrorPayload {
  error: string
}

export function rerankerLocalModelInfo(): Promise<LocalRerankerModelInfo | null> {
  if (OFF) return Promise.resolve(null)
  return invoke<LocalRerankerModelInfo>('rag_reranker_model_info')
}

export function rerankerOpenModelsDir(): Promise<void> {
  if (OFF) return Promise.resolve()
  return invoke<void>('rag_reranker_open_models_dir')
}

export function rerankerDownloadModel(): Promise<string> {
  if (OFF) return Promise.resolve('')
  return invoke<string>('rag_reranker_download_model')
}
