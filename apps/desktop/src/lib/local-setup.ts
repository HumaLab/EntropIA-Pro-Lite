/**
 * Everything Pro needs to run locally, as the backend sees it
 * (`automation/models_setup.rs`): the same inventory the assistant
 * `models_setup` tool answers, so Dependencias de IA and the assistant say the
 * same thing. Lite has no local engines: both calls answer empty.
 */

import { invoke } from '@tauri-apps/api/core'

const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

export type LocalComponentId =
  | 'runtime'
  | 'llm'
  | 'embeddings'
  | 'reranker'
  | 'parakeet'
  | 'llama_server'
  | 'glm_ocr'
  | 'qwen_correct'

export interface LocalComponent {
  id: LocalComponentId
  label: string
  ready: boolean
  /** What is installed or missing, in the backend's words. */
  detail: string
  /** What the component enables. */
  enables: string
  /** Set when this machine cannot use it (a GPU-only model without a GPU it fits on). */
  unavailableReason?: string
}

export interface LocalSetupStatus {
  localMl: boolean
  components: LocalComponent[]
}

/** Download events each component emits (`<scope>:download_*`). */
export const DOWNLOAD_SCOPES: Record<string, LocalComponentId> = {
  llm: 'llm',
  embedding: 'embeddings',
  reranker: 'reranker',
  parakeet: 'parakeet',
  'llama-server': 'llama_server',
  'glm-ocr': 'glm_ocr',
  'qwen-correct': 'qwen_correct',
}

export function localSetupStatus(): Promise<LocalSetupStatus> {
  if (OFF) return Promise.resolve({ localMl: false, components: [] })
  return invoke<LocalSetupStatus>('local_setup_status')
}

/** What one install request did for one component, in the backend's words. */
export interface InstallOutcome {
  id: LocalComponentId
  /** A download started, or one was already running or complete. */
  started: boolean
  /** What started, or why it did not. */
  message: string
}

/** Start the missing installs this machine can use (`only` narrows it). */
export function localSetupInstall(only: LocalComponentId[] = []): Promise<InstallOutcome[]> {
  if (OFF) return Promise.resolve([])
  return invoke<InstallOutcome[]>('local_setup_install', { only })
}

/** Still to install, and installable on this machine. */
export function pendingComponents(status: LocalSetupStatus | null): LocalComponent[] {
  return (status?.components ?? []).filter(
    (component) => !component.ready && !component.unavailableReason
  )
}
