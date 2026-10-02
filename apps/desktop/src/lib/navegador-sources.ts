/**
 * The saved web sources, seen from the frontend.
 *
 * Rust owns both tables (`navegador/sources.rs`): the renderer asks by command
 * and names a source by id, never by path. Everything in a source came from a
 * web page, so the view renders it as text, never as markup.
 */

import { invoke } from '@tauri-apps/api/core'
import type { WebCaptureProvenance } from './item-metadata'
import { formatBytes, previewText, shortHash } from './navegador-capture'
import { hostOf } from './navegador-tabs'

export type CaptureKind = 'page' | 'selection' | 'pdf'

/** Mirrors `SourceSummary` in `navegador/sources.rs`. */
export type SourceSummary = {
  id: string
  title: string | null
  finalUrl: string
  siteName: string | null
  /** Epoch milliseconds. */
  updatedAt: number
  captureCount: number
  /** Distinct kinds among its captures, sorted. */
  kinds: string[]
}

/** Mirrors `CaptureDetail`. */
export type CaptureDetail = {
  id: string
  kind: string
  mimeType: string
  /** UTC, RFC 3339, exactly as recorded. */
  accessedAt: string
  finalUrl: string
  title: string | null
  sha256: string
  /** What `sha256` covers: the HTML snapshot, the exact quote or the PDF. */
  hashOf: string
  sizeBytes: number
  /** The start of the text kept in the row (the quote of a selection). */
  textPreview: string | null
  /** The text was too large for the row and lives in a file. */
  textInFile: boolean
  quotePrefix: string | null
  quoteSuffix: string | null
  /** Whether the saved file is on disk; `null` when the capture has none. */
  filePresent: boolean | null
  /** The file came from another device and sync is still downloading it. */
  filePending: boolean
  createdAt: number
}

/** Mirrors `SourceDetail`. */
export type SourceDetail = {
  id: string
  originalUrl: string
  finalUrl: string
  canonicalUrl: string | null
  title: string | null
  siteName: string | null
  firstAccessedAt: string
  createdAt: number
  updatedAt: number
  captures: CaptureDetail[]
}

/** Mirrors `DeleteOutcome`. */
export type DeleteOutcome = {
  /** Some files could not be removed now; the startup sweep takes them later. */
  leftoverFiles: boolean
}

/** The saved sources, newest first; `query` filters by title, address and text. */
export function navegadorListSources(query?: string, limit?: number): Promise<SourceSummary[]> {
  return invoke<SourceSummary[]>('navegador_list_sources', {
    query: query?.trim() || null,
    limit: limit ?? null,
  })
}

/** One source with its captures, or `null` when it no longer exists. */
export function navegadorSourceDetail(sourceId: string): Promise<SourceDetail | null> {
  return invoke<SourceDetail | null>('navegador_source_detail', { sourceId })
}

/** Delete a source, its captures and its saved files. Copies elsewhere stay. */
export function navegadorDeleteSource(sourceId: string): Promise<DeleteOutcome> {
  return invoke<DeleteOutcome>('navegador_delete_source', { sourceId })
}

/**
 * Where the saved PDF of a capture is, resolved by the backend from the capture
 * id (never a path from here). The viewer loads it from disk: nothing is
 * downloaded again, so it works offline.
 */
export function navegadorPdfFile(captureId: string): Promise<string> {
  return invoke<string>('navegador_pdf_file', { captureId })
}

/** Mirrors `CopyTicket`: what copying a saved PDF into a collection needs. */
export type CopyTicket = {
  /** Absolute path of the saved PDF, found by Rust from the capture id. */
  path: string
  provenance: WebCaptureProvenance
}

/**
 * The saved PDF of a capture and where it came from, for a copy into a
 * collection. Rust finds the file, re-hashes it against the sha256 recorded when
 * it was verified and reads the provenance from its own rows: this side names a
 * capture and never supplies a path or the words that vouch for it.
 */
export function navegadorCopyTicket(captureId: string): Promise<CopyTicket> {
  return invoke<CopyTicket>('navegador_copy_ticket', { captureId })
}

/** Why a source command failed; mirrors `sources::code`. */
export const SOURCE_ERROR_CODES = [
  'invalid_id',
  'not_found',
  'not_a_pdf',
  'file_missing',
  'file_changed',
  'no_text',
  'db_error',
] as const

export type SourceErrorCode = (typeof SOURCE_ERROR_CODES)[number] | 'unknown'

/** The backend rejects with a stable code, optionally followed by `: detail`. */
export function parseSourceError(reason: unknown): {
  code: SourceErrorCode
  detail: string | null
} {
  const message = reason instanceof Error ? reason.message : String(reason)
  const [head, ...rest] = message.split(': ')
  const code = SOURCE_ERROR_CODES.find((known) => known === head)
  if (!code) return { code: 'unknown', detail: message }
  return { code, detail: rest.length > 0 ? rest.join(': ') : null }
}

/**
 * An RFC 3339 instant in the person's own time zone (or `timeZone`, for tests).
 * Anything unreadable comes back as it was recorded.
 */
export function formatLocalTime(iso: string, locale: 'es' | 'en', timeZone?: string): string {
  const instant = new Date(iso)
  if (!iso || Number.isNaN(instant.getTime())) return iso
  try {
    return new Intl.DateTimeFormat(locale === 'es' ? 'es-AR' : 'en-US', {
      dateStyle: 'medium',
      timeStyle: 'short',
      // 24-hour clock in Spanish, as people read it there; English keeps its own.
      ...(locale === 'es' ? { hourCycle: 'h23' as const } : {}),
      timeZone,
    }).format(instant)
  } catch {
    return iso
  }
}

/**
 * What the source-level "open" button does. A source made of PDFs only is the
 * page those files came from, so it opens "the page of origin"; any other source
 * just opens its address in the browser. Both load the address in the active
 * tab through the same navigate path.
 */
export function sourceOpenAction(captures: readonly CaptureDetail[]): 'origin' | 'browser' {
  return captures.length > 0 && captures.every((capture) => capture.kind === 'pdf')
    ? 'origin'
    : 'browser'
}

/** What a row of the list shows. */
export function describeSource(source: SourceSummary) {
  return {
    id: source.id,
    title: source.title?.trim() || source.finalUrl,
    url: source.finalUrl,
    host: (hostOf(source.finalUrl) ?? '').replace(/^www\./, ''),
    captureCount: source.captureCount,
    kinds: source.kinds,
    updatedAt: source.updatedAt,
  }
}

const CONTEXT_CHARS = 160

/** What a capture shows in the detail. */
export function describeCapture(
  capture: CaptureDetail,
  locale: 'es' | 'en' = 'es',
  timeZone?: string
) {
  const preview = previewText(capture.textPreview)
  const before = Array.from(capture.quotePrefix ?? '')
  const after = Array.from(capture.quoteSuffix ?? '')
  return {
    id: capture.id,
    kind: capture.kind,
    title: capture.title,
    finalUrl: capture.finalUrl,
    accessedUtc: capture.accessedAt,
    accessedLocal: formatLocalTime(capture.accessedAt, locale, timeZone),
    shortSha: shortHash(capture.sha256),
    hashOf: capture.hashOf,
    size: formatBytes(capture.sizeBytes),
    preview,
    quote:
      capture.kind === 'selection' && capture.textPreview !== null
        ? {
            before: before.slice(-CONTEXT_CHARS).join(''),
            quote: preview,
            after: after.slice(0, CONTEXT_CHARS).join(''),
          }
        : null,
    textInFile: capture.textInFile,
    /** A PDF whose file is on disk can be opened in the app's own viewer. */
    canViewPdf: capture.kind === 'pdf' && capture.filePresent === true,
    /**
     * A PDF whose file is on disk is copied as it is; a page or a selection is
     * copied as a PDF rendered from the text kept in its row or in its file.
     */
    canCopy:
      capture.kind === 'pdf'
        ? capture.filePresent === true
        : capture.textPreview !== null || capture.textInFile,
    file: (capture.filePresent === null
      ? 'none'
      : capture.filePresent
        ? 'present'
        : capture.filePending
          ? 'downloading'
          : 'missing') as 'present' | 'downloading' | 'missing' | 'none',
  }
}
