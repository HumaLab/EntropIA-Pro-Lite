/**
 * Copying a saved web source to Zotero.
 *
 * The copy is a durable row on the Rust side (`navegador_zotero_copy_*`): it is
 * queued, it waits while Zotero is not running and a drain sends it when Zotero
 * answers. This module asks by source and capture id and by library, never with
 * a path or with the words that vouch for the copy; Rust finds the file, checks
 * it and builds the item. Copying is always explicit and never moves anything.
 */

import { invoke } from '@tauri-apps/api/core'

export type ZoteroCopyState =
  | 'queued'
  | 'waiting'
  | 'running'
  | 'copied'
  | 'linked'
  | 'failed'
  | 'cancelled'

/** What a run found, as Rust recorded it. */
export type ZoteroCopyDetail = {
  /** The item was already in the library: it was linked, not created. */
  existing: boolean
  pdf: 'none' | 'attached' | 'not_attached' | 'parent_exists' | 'already_there'
  /** Fields of the existing item that would be filled or updated. */
  pendingFields: string[]
  /** Fields the person edited in Zotero: never overwritten. */
  keptFields: string[]
  /** What the Web API did for an existing item; absent on rows from before it. */
  web?: {
    state: ZoteroWebState
    completed: string[]
    pdf?: ZoteroWebPdf
    /** Where a failed state stopped, e.g. `patch:500`. Never a secret. */
    reason?: string
    /** Where a failed PDF stopped, e.g. `upload:500`. */
    pdfReason?: string
  }
}

/** What the Web API did with the PDF of an existing item. */
export type ZoteroWebPdf = 'attached' | 'already_there' | 'quota' | 'failed'

export type ZoteroWebState =
  | 'completed'
  | 'nothing_missing'
  | 'conflict'
  | 'no_key'
  | 'invalid_key'
  | 'no_write'
  | 'other_account'
  | 'account_unknown'
  | 'not_synced_yet'
  | 'failed'

/** The outcomes that earn a message; no key or no write access says nothing. */
const WEB_NOTE_STATES: readonly ZoteroWebState[] = [
  'completed',
  'nothing_missing',
  'conflict',
  'invalid_key',
  'other_account',
  'account_unknown',
  'not_synced_yet',
  'failed',
]

export type ZoteroCopy = {
  id: string
  sourceId: string
  captureId: string | null
  libraryType: 'user' | 'group'
  libraryId: string
  libraryName: string | null
  state: ZoteroCopyState
  itemKey: string | null
  detail: ZoteroCopyDetail | null
  errorCode: string | null
  errorMessage: string | null
  attempts: number
  createdAt: number
  updatedAt: number
}

export type ZoteroLibraryChoice = {
  libraryType: 'user' | 'group'
  libraryId: string
  libraryName: string | null
}

export type ZoteroDrain = { reachable: boolean; copies: ZoteroCopy[] }

export type ZoteroLaunch =
  | 'already_running'
  | 'started'
  | 'not_found'
  | 'too_soon'
  | 'unsupported'
  | 'spawn_failed'

export function navegadorZoteroRequest(
  sourceId: string,
  captureId: string | null,
  library: ZoteroLibraryChoice
): Promise<ZoteroCopy> {
  return invoke<ZoteroCopy>('navegador_zotero_copy_request', { sourceId, captureId, library })
}

export function navegadorZoteroList(sourceId?: string): Promise<ZoteroCopy[]> {
  return invoke<ZoteroCopy[]>('navegador_zotero_copy_list', { sourceId: sourceId ?? null })
}

/** Sends what waits, if Zotero answers. Safe to call as often as the view likes. */
export function navegadorZoteroRun(): Promise<ZoteroDrain> {
  return invoke<ZoteroDrain>('navegador_zotero_copy_run')
}

export function navegadorZoteroCancel(copyId: string): Promise<ZoteroCopy> {
  return invoke<ZoteroCopy>('navegador_zotero_copy_cancel', { copyId })
}

/** Starts Zotero. Only ever from a button the person pressed. */
export function navegadorZoteroLaunch(): Promise<ZoteroLaunch> {
  return invoke<ZoteroLaunch>('navegador_zotero_launch')
}

const PENDING: readonly ZoteroCopyState[] = ['queued', 'waiting', 'running']

export function isPending(copy: Pick<ZoteroCopy, 'state'>): boolean {
  return PENDING.includes(copy.state)
}

export function hasPending(copies: readonly Pick<ZoteroCopy, 'state'>[]): boolean {
  return copies.some(isPending)
}

export type CopyTone = 'pending' | 'good' | 'bad' | 'muted'

const TONES: Record<ZoteroCopyState, CopyTone> = {
  queued: 'pending',
  waiting: 'pending',
  running: 'pending',
  copied: 'good',
  linked: 'good',
  failed: 'bad',
  cancelled: 'muted',
}

/** What a row of copies shows. Labels are i18n keys, never text. */
export function describeCopy(copy: ZoteroCopy) {
  const personal = copy.libraryType === 'user' && copy.libraryId === '0'
  const notes: string[] = []
  const detail = copy.detail
  if (detail) {
    if (detail.existing) notes.push('navegador.zotero.note.existing')
    const webPdf = detail.web?.pdf
    // A full quota or a failure has its own explanation; the old note (attaching
    // only works when creating) would contradict it.
    const explained = webPdf === 'quota' || webPdf === 'failed'
    if (detail.pdf !== 'none' && !(explained && detail.pdf === 'parent_exists')) {
      notes.push(`navegador.zotero.note.pdf.${detail.pdf}`)
    }
    if (detail.existing && detail.web && WEB_NOTE_STATES.includes(detail.web.state)) {
      notes.push(`navegador.zotero.note.web.${detail.web.state}`)
    }
    if (detail.existing && webPdf && webPdf !== 'already_there') {
      notes.push(`navegador.zotero.note.web.pdf.${webPdf}`)
    }
    if (detail.existing && (detail.pendingFields.length > 0 || detail.keptFields.length > 0)) {
      notes.push('navegador.zotero.note.differs')
    }
  }
  return {
    id: copy.id,
    stateKey: `navegador.zotero.state.${copy.state}`,
    tone: TONES[copy.state],
    libraryKey: personal ? ('personal' as const) : ('named' as const),
    libraryName: personal
      ? null
      : copy.libraryName?.trim() || `${copy.libraryType}/${copy.libraryId}`,
    notes,
    /** Labels of the fields the Web API completed. */
    completedKeys: fieldLabels(detail?.web?.state === 'completed' ? detail.web.completed : []),
    /** Where the Web API steps stopped, compactly, for a technical line. */
    reasons: [detail?.web?.reason, detail?.web?.pdfReason].filter((reason): reason is string =>
      Boolean(reason)
    ),
    canLaunch: copy.state === 'waiting',
    canCancel: copy.state === 'queued' || copy.state === 'waiting',
    canRetry: copy.state === 'failed' || copy.state === 'cancelled',
    errorCode: copy.errorCode,
    errorMessage: copy.errorMessage,
  }
}

export type ZoteroErrorCode = 'invalid_library' | 'not_found' | 'not_a_pdf' | 'db_error' | 'unknown'

/** The backend rejects with a stable code, optionally followed by `: detail`. */
export function parseZoteroError(reason: unknown): {
  code: ZoteroErrorCode
  detail: string | null
} {
  const message = reason instanceof Error ? reason.message : String(reason)
  const [head, ...rest] = message.split(': ')
  const known: readonly ZoteroErrorCode[] = [
    'invalid_library',
    'not_found',
    'not_a_pdf',
    'db_error',
  ]
  const code = known.find((candidate) => candidate === head)
  if (!code) return { code: 'unknown', detail: message }
  return { code, detail: rest.length > 0 ? rest.join(': ') : null }
}

export type LiveLibrary = { libraryType: 'user' | 'group'; libraryId: string; name: string | null }

/** `reachable: false` means Zotero did not answer and the list is empty. */
export type LiveLibraryList = { reachable: boolean; libraries: LiveLibrary[] }

export type CopyStatus = {
  state: 'absent' | 'present' | 'unreachable'
  /** `zotero`: found by address. `record`: from our own record, Zotero closed. */
  source: 'zotero' | 'record' | 'none'
  itemKey: string | null
  pdf: ZoteroCopyDetail['pdf'] | 'unknown'
  pendingFields: string[]
  keptFields: string[]
  /** The saved PDF that goes along: the named capture or the source's latest. */
  pdfCapture?: { id: string; savedAt: string } | null
  /** The item is there, a Web API key is stored and something can be completed. */
  canComplete?: boolean
}

/** The libraries Zotero itself offers for writing, personal first. */
export function navegadorZoteroLibraries(): Promise<LiveLibraryList> {
  return invoke<LiveLibraryList>('navegador_zotero_libraries')
}

/** Whether the source (and its PDF) is already in the library. Reads only. */
export function navegadorZoteroStatus(
  sourceId: string,
  captureId: string | null,
  library: ZoteroLibraryChoice
): Promise<CopyStatus> {
  return invoke<CopyStatus>('navegador_zotero_status', { sourceId, captureId, library })
}

/** Selects an item in Zotero (the `zotero://select/...` link, built in Rust). */
export function navegadorZoteroOpenItem(
  libraryType: 'user' | 'group',
  libraryId: string,
  itemKey: string
): Promise<void> {
  return invoke<void>('writing_zotero_open_item', { libraryType, libraryId, itemKey })
}

const FIELDS = ['title', 'url', 'accessDate', 'websiteTitle']

function fieldLabels(fields: string[]): string[] {
  return fields
    .filter((field) => FIELDS.includes(field))
    .map((field) => `navegador.zotero.field.${field}`)
}

/** What the dialog shows about a status. Labels are i18n keys. */
export function describeStatus(status: CopyStatus) {
  return {
    present: status.state === 'present' && status.itemKey !== null,
    fromRecord: status.source === 'record',
    pendingKeys: fieldLabels(status.pendingFields),
    keptKeys: fieldLabels(status.keptFields),
    /** `undefined` while the backend has not said; `null` when no PDF goes along. */
    pdfCapture: status.pdfCapture,
    canComplete: status.canComplete === true,
    pdfKey:
      status.state === 'present' && status.pdf !== 'none' && status.pdf !== 'unknown'
        ? `navegador.zotero.note.pdf.${status.pdf}`
        : null,
  }
}
