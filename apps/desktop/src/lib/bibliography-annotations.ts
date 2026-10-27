import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/**
 * Zotero annotations of the Biblioteca (docs/zotero-annotations-plan.md).
 * Thin wrappers over the `bibliography_annotations_*` commands. The Zotero
 * API key never crosses this boundary: every call names a library row and an
 * annotation key, and Rust reads the key.
 */

/** The Zotero `data` object of one annotation item, as stored. */
export type ZoteroAnnotationData = Record<string, unknown> & {
  key?: string
  parentItem?: string
  annotationType?: ZoteroAnnotationType
  annotationText?: string
  annotationComment?: string
  annotationColor?: string
  annotationPageLabel?: string
  annotationSortIndex?: string
  /** A JSON string on the wire; an object is accepted on writes. */
  annotationPosition?: string | ZoteroAnnotationPosition
  tags?: { tag: string; type?: number }[]
  deleted?: number | boolean
}

export type ZoteroAnnotationType = 'highlight' | 'underline' | 'note' | 'image' | 'text' | 'ink'

/** Parsed `annotationPosition`. Unknown fields are kept on round trips. */
export interface ZoteroAnnotationPosition {
  pageIndex: number
  rects?: number[][]
  nextPageRects?: number[][]
  paths?: number[][]
  width?: number
  fontSize?: number
  rotation?: number
  [field: string]: unknown
}

export type AnnotationSyncState = 'synced' | 'pending' | 'waiting_parent' | 'conflict' | 'error'

/** Why `__existence` is disputed, as stored in `conflictRemote.__existence`. */
export type ExistenceReason = 'trashed' | 'modified' | 'deleted' | 'attachment_deleted'

export const EXISTENCE_FIELD = '__existence'

export interface AnnotationView {
  key: string
  parentKey: string
  /** The data the user currently sees: the pending edit, or Zotero's. */
  data: ZoteroAnnotationData | null
  syncState: AnnotationSyncState
  conflictFields: string[]
  /** Zotero's values of the disputed fields; `__existence` holds the reason. */
  conflictRemote: Record<string, unknown> | null
  remoteTrashed: boolean
  lastError: string | null
  /** Epoch milliseconds of the last local write of the row. */
  updatedAt: number
}

export interface AnnotationList {
  /** `no_key`: no verified Zotero key; `not_personal`: a group library. */
  status: 'ready' | 'no_key' | 'not_personal'
  annotations: AnnotationView[]
}

export interface AnnotationSyncReport {
  pulled: number
  removed: number
  sent: number
  paused: string | null
  uncertain: number
}

export interface AnnotationSyncResult {
  status:
    | 'synced'
    | 'skipped'
    | 'no_key'
    | 'no_write'
    | 'not_personal'
    | 'other_account'
    | 'account_unknown'
    | 'invalid_key'
    | 'unreachable'
  report: AnnotationSyncReport | null
}

export type AnnotationSyncReason = 'open' | 'edit' | 'catalog_sync'

export type ConflictDecision = 'keep' | 'delete' | 'local' | 'zotero'

export const ANNOTATIONS_CHANGED_EVENT = 'bibliography-annotations:changed'

export function listAnnotations(libraryRowId: string, parentKey: string): Promise<AnnotationList> {
  return invoke<AnnotationList>('bibliography_annotations_list', { libraryRowId, parentKey })
}

export function createAnnotation(
  libraryRowId: string,
  parentKey: string,
  data: ZoteroAnnotationData
): Promise<AnnotationView> {
  return invoke<AnnotationView>('bibliography_annotations_create', {
    libraryRowId,
    parentKey,
    data,
  })
}

/** `data` is the full desired object, not a patch. */
export function updateAnnotation(
  libraryRowId: string,
  key: string,
  data: ZoteroAnnotationData
): Promise<AnnotationView | null> {
  return invoke<AnnotationView | null>('bibliography_annotations_update', {
    libraryRowId,
    key,
    data,
  })
}

/**
 * Deletes an annotation. On a row in conflict the backend answers
 * `needs_confirmation` unless `confirmed`: confirming resolves the conflict
 * as a delete.
 */
export function deleteAnnotation(
  libraryRowId: string,
  key: string,
  confirmed = false
): Promise<AnnotationView | null> {
  return invoke<AnnotationView | null>('bibliography_annotations_delete', {
    libraryRowId,
    key,
    confirmed,
  })
}

/** Restores an annotation that is in Zotero's trash. */
export function restoreAnnotation(libraryRowId: string, key: string): Promise<AnnotationView | null> {
  return invoke<AnnotationView | null>('bibliography_annotations_restore', { libraryRowId, key })
}

/** One decision per entry of `conflictFields`, `__existence` included. */
export function resolveAnnotationConflict(
  libraryRowId: string,
  key: string,
  decisions: Record<string, ConflictDecision>
): Promise<AnnotationView | null> {
  return invoke<AnnotationView | null>('bibliography_annotations_resolve', {
    libraryRowId,
    key,
    decisions,
  })
}

export function syncAnnotations(
  libraryRowId: string,
  reason: AnnotationSyncReason
): Promise<AnnotationSyncResult> {
  return invoke<AnnotationSyncResult>('bibliography_annotations_sync', { libraryRowId, reason })
}

export function onAnnotationsChanged(
  handler: (libraryRowId: string) => void
): Promise<UnlistenFn> {
  return listen<{ libraryRowId: string }>(ANNOTATIONS_CHANGED_EVENT, (event) =>
    handler(event.payload.libraryRowId)
  )
}

/** Parses `annotationPosition`, accepting the wire string or an object. */
export function parsePosition(data: ZoteroAnnotationData | null): ZoteroAnnotationPosition | null {
  const raw = data?.annotationPosition
  if (raw == null) return null
  if (typeof raw === 'object') return raw
  try {
    const parsed: unknown = JSON.parse(raw)
    if (parsed && typeof parsed === 'object' && typeof (parsed as { pageIndex?: unknown }).pageIndex === 'number') {
      return parsed as ZoteroAnnotationPosition
    }
  } catch {
    // An unreadable position is shown nowhere rather than guessed.
  }
  return null
}

export function existenceReason(view: AnnotationView): ExistenceReason | null {
  const reason = view.conflictRemote?.[EXISTENCE_FIELD]
  return typeof reason === 'string' ? (reason as ExistenceReason) : null
}

/** `pending`, `waiting_parent` and `error` hold a local intent not yet sent. */
function holdsLocalIntent(view: AnnotationView): boolean {
  return view.syncState === 'pending' || view.syncState === 'waiting_parent' || view.syncState === 'error'
}

function markedDeleted(view: AnnotationView): boolean {
  const flag = view.data?.deleted
  return flag === 1 || flag === true
}

/**
 * In Zotero's trash for the panel: the read-only group with Restore. A row in
 * conflict stays out of it, since its decisions live in the list.
 */
export function isInZoteroTrash(view: AnnotationView): boolean {
  if (view.syncState === 'conflict') return false
  return markedDeleted(view) || (view.remoteTrashed && !holdsLocalIntent(view))
}

/**
 * A trashed annotation stays out of the viewer until a restore is pending
 * (plan section 9), conflicts included: keeping it is a decision in the panel.
 */
export function isVisibleInViewer(view: AnnotationView): boolean {
  if (markedDeleted(view)) return false
  return !view.remoteTrashed || holdsLocalIntent(view)
}
