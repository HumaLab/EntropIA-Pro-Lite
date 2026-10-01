/**
 * Copying a saved web PDF into a collection.
 *
 * A copy is a normal corpus item: the saved file goes through the same import
 * the "Importar fuentes" dialog uses (container asset plus one asset per page),
 * so it is processed, searched and cited like any other document. It is
 * independent of the web source: the file is a new one under `assets/`, and
 * deleting the source later touches nothing of it. What ties it to where it came
 * from is only the provenance written into `items.metadata`
 * (`__entropia_web_capture`), which Rust builds from its own rows
 * (`navegador_copy_ticket`).
 *
 * Copying is always explicit: this module has no default destination and never
 * moves anything.
 */

import { getStore } from './db'
import {
  importClassifiedPathsIntoCollection,
  type ImportItemOverrides,
  type ImportStage,
} from './collection-import'
import { WEB_CAPTURE_METADATA_KEY, type WebCaptureProvenance } from './item-metadata'
import { navegadorCopyTicket, parseSourceError, type SourceErrorCode } from './navegador-sources'

/** Longest stored file name, extension included. */
const FILE_NAME_MAX = 100
const EXTENSION = '.pdf'
const FALLBACK_NAME = 'PDF'
const RESERVED_DEVICE_NAMES = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i

/** The last path segment of an address without a `.pdf` ending, decoded; or null. */
function nameInAddress(address: string): string | null {
  try {
    const segment = new URL(address).pathname.split('/').filter(Boolean).pop()
    if (!segment) return null
    let decoded = segment
    try {
      decoded = decodeURIComponent(segment)
    } catch {
      // Not valid percent-encoding: the raw segment is as good a name as any.
    }
    const name = decoded.replace(/\.pdf$/i, '').trim()
    return name || null
  } catch {
    return null
  }
}

function hostOfAddress(address: string): string | null {
  try {
    return new URL(address).hostname.replace(/^www\./, '') || null
  } catch {
    return null
  }
}

/**
 * The title of the item a copy creates: the page title, else the file name in
 * the address, else its host. Never empty.
 */
export function copyTitle(provenance: WebCaptureProvenance): string {
  const title = provenance.pageTitle?.replace(/\s+/g, ' ').trim()
  return (
    title ||
    nameInAddress(provenance.finalUrl) ||
    hostOfAddress(provenance.finalUrl) ||
    FALLBACK_NAME
  )
}

/** A name that is safe as a file name on every platform, with no extension. */
function safeStem(text: string): string {
  const cleaned = text
    .replace(/[\p{Cc}\p{Cf}]/gu, (character) => (/\s/u.test(character) ? ' ' : ''))
    .replace(/[<>:"/\\|?*]/g, '')
    .replace(/\s+/g, ' ')
    .replace(/^[\s.]+|[\s.]+$/g, '')
  const stem = Array.from(cleaned)
    .slice(0, FILE_NAME_MAX - EXTENSION.length - 1)
    .join('')
    .replace(/[\s.]+$/g, '')
  if (!stem) return FALLBACK_NAME
  // Windows treats `NUL.txt` as the device: the name before the first dot counts.
  return RESERVED_DEVICE_NAMES.test(stem.split('.')[0]!) ? `_${stem}` : stem
}

/** The name the stored copy carries: the title as a safe file name, `.pdf`. */
export function copyFileName(provenance: WebCaptureProvenance): string {
  return `${safeStem(copyTitle(provenance))}${EXTENSION}`
}

/**
 * What the import is told about the file it copies. `allowDuplicate`: the flow
 * that calls this already looked the capture up and, when a copy existed, asked
 * the person, so the import's own "same file again" check must not skip it.
 */
export function copyOverrides(provenance: WebCaptureProvenance): ImportItemOverrides {
  return {
    title: copyTitle(provenance),
    fileName: copyFileName(provenance),
    extraMetadata: { [WEB_CAPTURE_METADATA_KEY]: provenance },
    allowDuplicate: true,
  }
}

/**
 * The item `collectionId` already holds as a copy of this capture, or null. A
 * failed lookup answers null: it must not stop a copy the person asked for.
 */
export async function findExistingCopy(
  collectionId: string,
  captureId: string
): Promise<{ id: string; title: string } | null> {
  try {
    return await getStore().items.findByWebCapture(collectionId, captureId)
  } catch (reason) {
    console.warn('[navegador] could not look for an earlier copy:', reason)
    return null
  }
}

export type CopyErrorCode = SourceErrorCode | 'import_failed' | 'not_created'

/** Why a copy failed: Rust's refusal of the capture or the import's own failure. */
export class CopyError extends Error {
  readonly code: CopyErrorCode
  readonly detail: string | null

  constructor(code: CopyErrorCode, detail: string | null = null) {
    super(detail ? `${code}: ${detail}` : code)
    this.name = 'CopyError'
    this.code = code
    this.detail = detail
  }
}

export type CopiedCapture = {
  item: { id: string; title: string }
  collectionId: string
}

/**
 * Copy the saved PDF of `captureId` into `collectionId` as a new, independent
 * item. The file comes from Rust by capture id; nothing here accepts a path.
 */
export async function copyCaptureToCollection(options: {
  captureId: string
  collectionId: string
  onStage?: (stage: ImportStage) => void
}): Promise<CopiedCapture> {
  const { captureId, collectionId, onStage } = options

  let ticket
  try {
    ticket = await navegadorCopyTicket(captureId)
  } catch (reason) {
    const { code, detail } = parseSourceError(reason)
    throw new CopyError(code, detail)
  }

  const result = await importClassifiedPathsIntoCollection([ticket.path], collectionId, {
    baseErrorMessage: 'Copy to collection',
    onProgress: (progress) => onStage?.(progress.stage),
    overrides: copyOverrides(ticket.provenance),
  })

  const created = result.createdItems[0]
  if (!created) {
    if (result.importErrors.length > 0) {
      throw new CopyError('import_failed', result.importErrors[0]!)
    }
    throw new CopyError('not_created')
  }
  return { item: created, collectionId }
}
