/**
 * What the Navegador captures, seen from the frontend.
 *
 * Captures and downloads are drafts: data the backend read from a page or a
 * quarantined file, shown to the person before anything is kept (nothing is
 * stored yet). Everything in them came from a web page, so the view renders
 * it as text and never as markup.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/** Mirrors `CaptureDraft` in `navegador/capture.rs` (the HTML stays in Rust). */
export type CaptureDraft = {
  kind: 'page' | 'selection'
  finalUrl: string
  title: string | null
  canonicalUrl: string | null
  siteName: string | null
  lang: string | null
  text: string
  quote: string | null
  quotePrefix: string | null
  quoteSuffix: string | null
  htmlBytes: number
  /** Which content `sha256` covers: the HTML snapshot or the exact quote. */
  hashOf: 'html' | 'quote'
  sha256: string
  truncated: boolean
  accessedAt: string
}

export type DownloadStatus = 'downloading' | 'ready' | 'rejected' | 'failed'

/** Mirrors `DownloadDraft` in `navegador/download.rs`. */
export type DownloadDraft = {
  id: string
  url: string
  fileName: string
  size: number | null
  sha256: string | null
  accessedAt: string
  status: DownloadStatus
  /** A stable code for a rejection or failure, `null` otherwise. */
  reason: string | null
}

export const NAVEGADOR_DOWNLOAD_EVENT = 'navegador://download'

export function navegadorCapturePage(): Promise<CaptureDraft> {
  return invoke<CaptureDraft>('navegador_capture_page')
}

export function navegadorCaptureSelection(): Promise<CaptureDraft> {
  return invoke<CaptureDraft>('navegador_capture_selection')
}

/** Follow downloads through quarantine. Only the main webview hears it. */
export function onNavegadorDownload(handler: (draft: DownloadDraft) => void): Promise<UnlistenFn> {
  return listen<DownloadDraft>(NAVEGADOR_DOWNLOAD_EVENT, (event) => handler(event.payload))
}

/** The first characters of a SHA-256, enough to tell two captures apart. */
export function shortHash(sha256: string | null | undefined, length = 12): string {
  return typeof sha256 === 'string' && /^[0-9a-f]{64}$/.test(sha256) ? sha256.slice(0, length) : ''
}

/** A one-line preview, cut on whole characters (never inside a surrogate pair). */
export function previewText(text: string | null | undefined, max = 500): string {
  const line = (text ?? '').replace(/\s+/g, ' ').trim()
  const chars = Array.from(line)
  return chars.length > max ? `${chars.slice(0, max).join('')}…` : line
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  if (bytes < 1024) return `${Math.round(bytes)} B`
  const units = ['KB', 'MB', 'GB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value.toFixed(1)} ${units[unit]}`
}

export const CAPTURE_ERROR_CODES = [
  'no_selection',
  'pdf_document',
  'script_failed',
  'invalid_result',
  'too_large',
  'blocked_url',
  'timeout',
  'not_open',
] as const

export type CaptureErrorCode = (typeof CAPTURE_ERROR_CODES)[number] | 'unknown'

/**
 * The backend rejects with a stable code, optionally followed by `: detail`.
 * Anything else (a Tauri error, "not available in this build") is `unknown`
 * and keeps its message.
 */
export function parseCaptureError(reason: unknown): {
  code: CaptureErrorCode
  detail: string | null
} {
  const message = reason instanceof Error ? reason.message : String(reason)
  const [head, ...rest] = message.split(': ')
  const code = CAPTURE_ERROR_CODES.find((known) => known === head)
  if (!code) return { code: 'unknown', detail: message }
  return { code, detail: rest.length > 0 ? rest.join(': ') : null }
}

/** Reasons a download is refused or fails; mirrors `download::reason`. */
export const DOWNLOAD_REASON_CODES = [
  'not_pdf',
  'too_large',
  'empty',
  'interrupted',
  'io_error',
  'too_many',
  'blocked',
] as const

/** The message key for a reason; anything unrecognised is the generic one. */
export function downloadReasonKey(reason: string | null): string {
  const known = DOWNLOAD_REASON_CODES.find((code) => code === reason)
  return `navegador.download.reason.${known ?? 'unknown'}`
}

/** What the capture panel shows for a draft. */
export function describeCaptureDraft(draft: CaptureDraft) {
  return {
    kind: draft.kind,
    title: draft.title || draft.finalUrl,
    finalUrl: draft.finalUrl,
    accessedAt: draft.accessedAt,
    textLength: draft.text.length,
    htmlBytes: draft.htmlBytes,
    shortSha: shortHash(draft.sha256),
    hashOf: draft.hashOf,
    preview: previewText(draft.kind === 'selection' ? (draft.quote ?? draft.text) : draft.text),
    truncated: draft.truncated,
  }
}

export function describeDownload(draft: DownloadDraft) {
  return {
    id: draft.id,
    fileName: draft.fileName,
    url: draft.url,
    accessedAt: draft.accessedAt,
    size: draft.size === null ? '' : formatBytes(draft.size),
    shortSha: shortHash(draft.sha256),
    status: draft.status,
    reason: draft.reason,
  }
}

/** Newest first; an update replaces the entry in place; only the last few stay. */
export function upsertDownload(
  list: readonly DownloadDraft[],
  next: DownloadDraft,
  max = 5
): DownloadDraft[] {
  const exists = list.some((entry) => entry.id === next.id)
  const merged = exists
    ? list.map((entry) => (entry.id === next.id ? next : entry))
    : [next, ...list]
  return merged.slice(0, max)
}
