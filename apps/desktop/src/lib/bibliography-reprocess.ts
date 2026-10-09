import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/**
 * The text reprocess adapter (plan-texto-nativo-parte-b 2.3): the damaged-text
 * candidate scan, the read-only preview (progress + cancellation) and the
 * owner-approved confirm. Every function is a thin wrapper over one Tauri
 * command; the money-spending step is `bibliographyReprocessConfirm` alone,
 * and it only ever queues exactly the entries the owner approved.
 */

/** Why one attachment is a candidate (machine reason codes, stable). */
export type ReprocessCandidateReason =
  | 'garbled_stored_pages'
  | 'empty_without_ocr'
  | 'failed_ocr_attempt'

/** One attachment the reprocess action should consider (no file is read). */
export interface ReprocessCandidate {
  attachmentId: string
  itemId: string
  title: string
  filename: string | null
  /** At least one reason code. */
  reasons: ReprocessCandidateReason[]
  /** How many stored page rows the detectors flag. */
  flaggedPages: number
}

/** The PDF attachments whose stored text needs the owner's repair action. */
export function bibliographyReprocessCandidates(): Promise<ReprocessCandidate[]> {
  return invoke<ReprocessCandidate[]>('bibliography_reprocess_candidates')
}

/** Why a previewed attachment cannot be read; stable machine strings. */
export type ReprocessUnreadableReason =
  | 'attachment_missing'
  | 'file_missing'
  | 'file_too_large'
  | 'not_a_pdf'
  | 'read_failed'

/** One previewed attachment. `unreadable` ones carry no `planHash`. */
export interface ReprocessPreviewAttachment {
  attachmentId: string
  itemId: string
  title: string
  filename: string | null
  pageCount: number
  ocrPages: number
  reusedOcrPages: number
  fixedWithoutOcr: number
  planHash: string | null
  /** A live task owns the attachment: the confirm will answer `busy`. */
  busy: boolean
  unreadable: ReprocessUnreadableReason | string | null
}

/** The preview totals the dialog shows; the USD is an estimate. */
export interface ReprocessPreviewTotals {
  attachments: number
  pages: number
  ocrPages: number
  reusedOcrPages: number
  fixedWithoutOcr: number
  estimatedUsd: number
}

/** The whole preview answer. `cancelled` holds only what was processed. */
export interface ReprocessPreview {
  attachments: ReprocessPreviewAttachment[]
  totals: ReprocessPreviewTotals
  cancelled: boolean
}

/**
 * The read-only preview over the given attachments (reads each PDF, hashes it
 * and plans). Progress arrives on `bibliography-reprocess-preview-progress`;
 * cancel with {@link bibliographyReprocessPreviewCancel}.
 */
export function bibliographyReprocessPreview(attachmentIds: string[]): Promise<ReprocessPreview> {
  return invoke<ReprocessPreview>('bibliography_reprocess_preview', { attachmentIds })
}

/** Stops the running preview between attachments or page batches. */
export function bibliographyReprocessPreviewCancel(): Promise<void> {
  return invoke<void>('bibliography_reprocess_preview_cancel')
}

/** What happened to one confirm entry. */
export type ReprocessEntryStatus = 'queued' | 'busy' | 'unknown_attachment'

/** One owner-approved entry, exactly `{attachmentId, planHash}`. */
export interface ReprocessConfirmEntry {
  attachmentId: string
  planHash: string
}

export interface ReprocessConfirmResult {
  attachmentId: string
  status: ReprocessEntryStatus
}

/** The new user batch when anything was queued, plus one status per entry. */
export interface ReprocessConfirm {
  batchId: string | null
  results: ReprocessConfirmResult[]
}

/**
 * Queues the approved reprocess: one user batch plus one fresh task per
 * entry. Never attaches to a live task — those answer `busy`.
 */
export function bibliographyReprocessConfirm(
  entries: ReprocessConfirmEntry[]
): Promise<ReprocessConfirm> {
  return invoke<ReprocessConfirm>('bibliography_reprocess_confirm', { entries })
}

/** Per-attachment preview step, as the backend reports it. */
export interface ReprocessPreviewProgress {
  done: number
  total: number
}

export const REPROCESS_PREVIEW_PROGRESS_EVENT = 'bibliography-reprocess-preview-progress'

/** Subscribes to the preview progress; dispose with the returned unlisten. */
export function onBibliographyReprocessPreviewProgress(
  handler: (progress: ReprocessPreviewProgress) => void
): Promise<UnlistenFn> {
  return listen<ReprocessPreviewProgress>(REPROCESS_PREVIEW_PROGRESS_EVENT, (event) =>
    handler(event.payload)
  )
}

const ESTIMATED_USD_NUMBER_LOCALE = { es: 'es-AR', en: 'en-US' } as const

/**
 * The estimated USD as the dialog shows it (`≈ USD 0,24 (estimado)`): the
 * locale's decimal separator, cents for ordinary amounts, and up to four
 * decimals so a real per-page cost stays distinguishable from zero.
 */
export function formatEstimatedUsd(usd: number, locale: 'es' | 'en' = 'es'): string {
  return new Intl.NumberFormat(ESTIMATED_USD_NUMBER_LOCALE[locale], {
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(usd)
}

/**
 * The backend's `is_pdf_attachment` rule, mirrored for the work view: a PDF
 * is named by its content type or by its file name, never by a path.
 */
export function isPdfAttachment(
  contentType: string | null | undefined,
  filename: string | null | undefined
): boolean {
  if ((contentType ?? '').toLowerCase().includes('pdf')) return true
  return (filename ?? '').toLowerCase().endsWith('.pdf')
}
