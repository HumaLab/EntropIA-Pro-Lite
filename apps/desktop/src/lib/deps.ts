/**
 * Dependency manager frontend client for EntropIA Pro desktop app.
 * Wraps Tauri commands for the Python dependency manager (uv-based).
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

// In the API-only (lite) variant, the local dependency manager does not exist.
// This module stays statically importable (TopBar/AppShell/SettingsView pull it
// at module-load regardless of `{#if LOCAL_ML}` template guards), so every
// consumed symbol is kept exported; the local-only ones flip inert under OFF.
// The comparison is inline so the define()'d literal constant-folds.
const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type DependencyId =
  'Python' | 'Fastembed' | 'PaddlePaddle' | 'PaddleOcr' | 'FasterWhisper' | 'Spacy' | 'SpacyModelEs'

export type DependencyStatus =
  | { type: 'unknown' }
  | { type: 'checking' }
  | { type: 'installed'; version?: string }
  | { type: 'missing' }
  | { type: 'installing'; percent: number }
  | { type: 'failed'; message: string }

export interface DepCheckResult {
  id: DependencyId
  status: DependencyStatus
  version: string | null
}

export interface UvStatusResult {
  uv_ready: boolean
  uv_path: string | null
  uv_version: string | null
  uv_source: string | null
  uv_compatible_for_dev: boolean
  venv_exists: boolean
  venv_path: string | null
  uv_warning: string | null
  release_runtime_ready: boolean
  release_runtime_state: string | null
  dev_fallback_available: boolean
  dev_fallback_reason: string | null
}

export interface DepsProgressEvent {
  id: DependencyId
  status: DependencyStatus
  message: string
}

export interface DepsCompleteEvent {
  results: DepCheckResult[]
  all_critical_installed: boolean
}

export interface DepsErrorEvent {
  stage: string
  error: string
  recoverable: boolean
}

function normalizeDepResults(results: DepCheckResult[] | null | undefined): DepCheckResult[] {
  return Array.isArray(results) ? results : []
}

// ---------------------------------------------------------------------------
// Invoke wrappers
// ---------------------------------------------------------------------------

export function checkAllDeps(): Promise<DepCheckResult[]> {
  if (OFF) return Promise.resolve([])
  return invoke<DepCheckResult[]>('deps_check_all').then(normalizeDepResults)
}

export function getCachedDepsStatuses(): Promise<DepCheckResult[]> {
  if (OFF) return Promise.resolve([])
  return invoke<DepCheckResult[]>('deps_get_cached_statuses').then(normalizeDepResults)
}

export function installAllDeps(): Promise<void> {
  return invoke<void>('deps_install_all')
}

export function installOneDep(id: DependencyId): Promise<DepCheckResult> {
  return invoke<DepCheckResult>('deps_install_one', { id })
}

export function getUvStatus(): Promise<UvStatusResult> {
  if (OFF) {
    return Promise.resolve({
      uv_ready: false,
      uv_path: null,
      uv_version: null,
      uv_source: null,
      uv_compatible_for_dev: false,
      venv_exists: false,
      venv_path: null,
      uv_warning: null,
      release_runtime_ready: false,
      release_runtime_state: null,
      dev_fallback_available: false,
      dev_fallback_reason: null,
    })
  }
  return invoke<UvStatusResult>('deps_get_uv_status')
}

export function resetDeps(): Promise<void> {
  return invoke<void>('deps_reset')
}

// ---------------------------------------------------------------------------
// Display metadata (Spanish)
// ---------------------------------------------------------------------------

export const DEP_DISPLAY_NAMES: Record<DependencyId, string> = {
  Python: 'Python 3.11',
  Fastembed: 'Fastembed (legacy embeddings)',
  PaddlePaddle: 'PaddlePaddle (runtime OCR)',
  PaddleOcr: 'PaddleOCR (OCR principal)',
  FasterWhisper: 'Faster Whisper (transcripción)',
  Spacy: 'spaCy (NER)',
  SpacyModelEs: 'Modelo spaCy español',
}

export const CRITICAL_DEPS: DependencyId[] = OFF ? [] : ['Python', 'PaddlePaddle', 'PaddleOcr']

export const DEP_DESCRIPTIONS: Record<DependencyId, string> = {
  Python: 'Intérprete Python requerido para todas las funciones de IA',
  Fastembed: 'Motor legacy de embeddings; el perfil liviano usa OpenRouter BGE-M3',
  PaddlePaddle: 'Base de ejecución requerida por PaddleOCR-VL',
  PaddleOcr: 'Motor principal de reconocimiento óptico de caracteres',
  FasterWhisper: 'Transcripción de audio a texto',
  Spacy: 'Reconocimiento de entidades nombradas',
  SpacyModelEs: 'Modelo de lenguaje español para spaCy',
}

// ---------------------------------------------------------------------------
// Event listener helpers
// ---------------------------------------------------------------------------

export function onDepsProgress(callback: (event: DepsProgressEvent) => void): Promise<UnlistenFn> {
  return listen<DepsProgressEvent>('deps://progress', (e) => callback(e.payload))
}

export function onDepsComplete(callback: (event: DepsCompleteEvent) => void): Promise<UnlistenFn> {
  return listen<DepsCompleteEvent>('deps://complete', (e) => callback(e.payload))
}

export function onDepsError(callback: (event: DepsErrorEvent) => void): Promise<UnlistenFn> {
  return listen<DepsErrorEvent>('deps://error', (e) => callback(e.payload))
}

// ---------------------------------------------------------------------------
// Shared reactive state for critical deps status
// ---------------------------------------------------------------------------

let _criticalMissing = false
const _listeners = new Set<(value: boolean) => void>()

export function setCriticalMissing(value: boolean) {
  if (OFF) return
  _criticalMissing = value
  _listeners.forEach((fn) => fn(value))
}

export function isCriticalMissing(): boolean {
  if (OFF) return false
  return _criticalMissing
}

export function onCriticalMissingChange(fn: (value: boolean) => void): () => void {
  if (OFF) return () => {}
  _listeners.add(fn)
  return () => _listeners.delete(fn)
}
