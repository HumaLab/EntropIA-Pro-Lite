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
}

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
    if (detail.pdf !== 'none') notes.push(`navegador.zotero.note.pdf.${detail.pdf}`)
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
