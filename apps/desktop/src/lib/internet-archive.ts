/**
 * The Internet Archive connector, seen from the frontend.
 *
 * Rust owns search, download staging and job state
 * (`src-tauri/src/internet_archive/`): the renderer asks by command and
 * names jobs, items and files by identifier, never by URL or path.
 * Failures arrive as `"<code>: <message>"`; this module reads the code
 * and carries no UI copy.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/** File availability, as `availability_tag` reports it. */
export type IaAvailability =
  | 'available'
  | 'encrypted'
  | 'private'
  | 'item_restricted'
  | 'unsafe_name'

/** Per-file state, as `FileState` serializes it (`snake_case`). */
export type IaFileState =
  | 'queued'
  | 'downloading'
  | 'verifying'
  | 'verified'
  | 'importing'
  | 'imported'
  | 'skipped_duplicate'
  | 'unavailable'
  | 'failed'
  | 'cancelled'

/** Job-level state, as `JobState` serializes it (`snake_case`). */
export type IaJobState =
  | 'draft'
  | 'ready'
  | 'running'
  | 'cancelling'
  | 'completed'
  | 'partial'
  | 'failed'
  | 'cancelled'

/** What the frontend must do with a verified file (`BeginAction`). */
export type IaBeginAction = 'import' | 'confirm_existing' | 'discard_partial'

/** What the frontend must do with a recovery leftover (`ReconcileAction`). */
export type IaReconcileAction = 'keep' | 'discard' | 'none'

/** Duplicate classification of one prepared file (`DuplicateStatusDto`). */
export type IaDuplicateStatus =
  | { status: 'new' }
  | { status: 'already_in_target'; itemId: string }
  | { status: 'new_version'; previousItemId: string }
  | { status: 'in_other_collection'; itemId: string; collectionId: string }

/** Mirrors `SearchQueryDto`. Blank filters are no filter on the Rust side. */
export type IaSearchQuery = {
  text?: string
  title?: string
  creator?: string
  language?: string
  collection?: string
  yearFrom?: number
  yearTo?: number
  textsOnly?: boolean
}

/** Mirrors `SearchItemDto`. */
export type IaSearchItem = {
  identifier: string
  title: string | null
  creators: string[]
  date: string | null
  collections: string[]
  restricted: boolean
}

/** Mirrors `SearchPageDto`. */
export type IaSearchPage = {
  numFound: number
  items: IaSearchItem[]
}

/** Mirrors `PdfFileDto`. */
export type IaPdfFile = {
  name: string
  format: string | null
  source: string
  size: number | null
  mtime: number | null
  md5: string | null
  sha1: string | null
  availability: IaAvailability
}

/** Mirrors `ItemDetailsDto`. */
export type IaItemDetails = {
  identifier: string
  title: string | null
  creators: string[]
  date: string | null
  languages: string[]
  collections: string[]
  rights: string | null
  licenseUrl: string | null
  mediatype: string | null
  restricted: boolean
  pdfs: IaPdfFile[]
  truncated: boolean
}

/** One requested `(identifier, files)` pair, as `SelectionDto` reads it. */
export type IaSelection = {
  identifier: string
  fileNames: string[]
}

/** Mirrors `PrepareEntryDto`. */
export type IaPrepareEntry = {
  identifier: string
  fileName: string
  availability: IaAvailability
  size: number | null
  duplicate: IaDuplicateStatus | null
}

/** Mirrors `PrepareSummaryDto`. */
export type IaPrepareSummary = {
  totalFiles: number
  availableFiles: number
  restrictedFiles: number
  knownBytes: number
  unknownSizeFiles: number
  newFiles: number
  alreadyInTarget: number
  inOtherCollection: number
  newVersion: number
  entries: IaPrepareEntry[]
}

/** One requested file that did not enter the job, as `RejectedDto` reads it. */
export type IaPrepareRejected = {
  identifier: string
  fileName: string
  reason: string
}

/** Mirrors `PrepareResultDto`. */
export type IaPrepareResult = {
  jobId: string
  summary: IaPrepareSummary
  rejected: IaPrepareRejected[]
}

/** Mirrors `JobFileStatusDto`. */
export type IaJobFileStatus = {
  index: number
  identifier: string
  fileName: string
  displayFileName: string
  state: IaFileState
  attempts: number
  error: string | null
  expectedSize: number | null
  pageCount: number | null
}

/** Mirrors `JobStatusDto`. */
export type IaJobStatus = {
  id: string
  state: IaJobState
  targetCollectionId: string | null
  files: IaJobFileStatus[]
  running: boolean
}

/** Mirrors `JobSummaryDto`. */
export type IaJobSummary = {
  id: string
  state: IaJobState
  targetCollectionId: string | null
  totalFiles: number
  verifiedFiles: number
}

/** Mirrors `VerifiedFileDto`; `extraMetadata` goes to the importer unchanged. */
export type IaVerifiedFile = {
  index: number
  identifier: string
  fileName: string
  displayFileName: string
  itemTitle: string
  stagingPath: string
  pageCount: number
  sha256: string
  extraMetadata: unknown
}

/** Mirrors `BeginDto`. */
export type IaBeginResult = {
  action: IaBeginAction
  itemId?: string
  file?: IaVerifiedFile
}

/** Mirrors `ReconcileDto`. */
export type IaReconcileEntry = {
  jobId: string
  index: number
  identifier: string
  fileName: string
  action: IaReconcileAction
  itemId?: string
}

/** Resolve user input (a bare identifier or an archive.org URL) to an identifier. */
export function iaResolveIdentifier(input: string): Promise<string> {
  return invoke<string>('ia_resolve_identifier', { input })
}

/** Run a structured search over `page` (1-based) with `rows` per page. */
export function iaSearch(query: IaSearchQuery, page: number, rows: number): Promise<IaSearchPage> {
  return invoke<IaSearchPage>('ia_search', { query, page, rows })
}

/** List one page of a collection by identifier. */
export function iaListCollection(
  collection: string,
  page: number,
  rows: number
): Promise<IaSearchPage> {
  return invoke<IaSearchPage>('ia_list_collection', { collection, page, rows })
}

/** Fetch tolerant item metadata with its PDF availability. */
export function iaGetItem(identifier: string): Promise<IaItemDetails> {
  return invoke<IaItemDetails>('ia_get_item', { identifier })
}

/** Prepare an import job over an existing collection id. */
export function iaPrepareImport(
  collectionId: string,
  selections: IaSelection[]
): Promise<IaPrepareResult> {
  return invoke<IaPrepareResult>('ia_prepare_import', { collectionId, selections })
}

/** Start (or resume) a job download; one job at a time. */
export function iaStartImport(jobId: string): Promise<void> {
  return invoke<void>('ia_start_import', { jobId })
}

/** Cancel a job download. */
export function iaCancelImport(jobId: string): Promise<void> {
  return invoke<void>('ia_cancel_import', { jobId })
}

/** Current job state with its files, plus whether a runner works on it. */
export function iaGetImportStatus(jobId: string): Promise<IaJobStatus> {
  return invoke<IaJobStatus>('ia_get_import_status', { jobId })
}

/** Non-terminal jobs for "Retomar importación". */
export function iaListJobs(): Promise<IaJobSummary[]> {
  return invoke<IaJobSummary[]>('ia_list_jobs')
}

/** Verified files of a job with staging paths and `extraMetadata`. */
export function iaListVerifiedFiles(jobId: string): Promise<IaVerifiedFile[]> {
  return invoke<IaVerifiedFile[]>('ia_list_verified_files', { jobId })
}

/** Begin the frontend import of one verified file. */
export function iaBeginFileImport(jobId: string, index: number): Promise<IaBeginResult> {
  return invoke<IaBeginResult>('ia_begin_file_import', { jobId, index })
}

/** Confirm the item the frontend imported for one `Importing` file. */
export function iaConfirmFileImported(
  jobId: string,
  index: number,
  itemId: string
): Promise<void> {
  return invoke<void>('ia_confirm_file_imported', { jobId, index, itemId })
}

/** Report a failed frontend import. */
export function iaFailFileImport(jobId: string, index: number): Promise<void> {
  return invoke<void>('ia_fail_file_import', { jobId, index })
}

/** Classified recovery leftovers for the reconcile screen. */
export function iaListReconcile(): Promise<IaReconcileEntry[]> {
  return invoke<IaReconcileEntry[]>('ia_list_reconcile')
}

/** Release one recovery leftover. */
export function iaFinishReconcile(jobId: string, index: number): Promise<void> {
  return invoke<void>('ia_finish_reconcile', { jobId, index })
}

/**
 * Why an IA command failed: every code the Rust side can produce. The
 * `http_<status>` family is not listed (it keeps its status number instead).
 */
export const IA_ERROR_CODES = [
  'invalid_query',
  'not_found',
  'blocked',
  'too_many_redirects',
  'too_large',
  'network',
  'decode',
  'already_exists',
  'io',
  'verify_failed',
  'too_many_pages',
  'invalid_request',
  'collection_not_found',
  'nothing_to_import',
  'db',
  'job_running',
  'invalid_state',
  'staging_missing',
  'item_not_found',
  'wrong_collection',
  'provenance_mismatch',
  'incomplete_import',
  'partial_remains',
] as const

/** The `http_<status>` family, with the status number kept. */
export type IaHttpErrorCode = `http_${number}`

export type IaErrorCode = (typeof IA_ERROR_CODES)[number] | IaHttpErrorCode | 'unknown'

const HTTP_CODE = /^http_(\d+)$/

/** The backend rejects with a stable code, optionally followed by `: detail`. */
export function parseIaError(reason: unknown): {
  code: IaErrorCode
  status?: number
  message: string
} {
  const message = reason instanceof Error ? reason.message : String(reason)
  const [head = '', ...rest] = message.split(': ')
  const detail = rest.length > 0 ? rest.join(': ') : message
  const http = HTTP_CODE.exec(head)
  if (http) return { code: head as IaHttpErrorCode, status: Number(http[1]), message: detail }
  const code = IA_ERROR_CODES.find((known) => known === head)
  if (!code) return { code: 'unknown', message }
  return { code, message: detail }
}

/** The `ia://job-updated` payload, as `emit_job_updated` sends it. */
export type IaJobUpdated = {
  jobId: string
}

/** Run `handler` with the job id on every job change; returns the unlisten function. */
export function onIaJobUpdated(handler: (jobId: string) => void): Promise<UnlistenFn> {
  return listen<IaJobUpdated>('ia://job-updated', (event) => {
    handler(event.payload.jobId)
  })
}
