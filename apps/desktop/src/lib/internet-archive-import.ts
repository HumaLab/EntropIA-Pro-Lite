/**
 * Importing verified Internet Archive files into a collection.
 *
 * Rust owns the dedup and completeness decisions: each file is begun, then
 * imported through the same `importClassifiedPathsIntoCollection` the "copy
 * to collection" flow uses (with the payload Rust handed over, duplicates
 * allowed because Rust already classified them), then confirmed or reported
 * as failed. A file that is already there is only confirmed; a partial
 * remnant is discarded and the file begun once more. One file's failure
 * never stops the others, and cancel is checked between files (the UI
 * cancels the job on the Rust side; nothing here cancels anything).
 */

import {
  iaBeginFileImport,
  iaConfirmFileImported,
  iaFailFileImport,
  iaFinishReconcile,
  iaGetImportStatus,
  iaListReconcile,
  iaListVerifiedFiles,
  parseIaError,
  type IaBeginResult,
  type IaErrorCode,
  type IaVerifiedFile,
} from './internet-archive'
import {
  discardFailedImport,
  importClassifiedPathsIntoCollection,
  type ImportStage,
} from './collection-import'

export type IaImportErrorCode = 'missing_collection'

/** The orchestrator refused to start: the job names no target collection. */
export class IaImportError extends Error {
  readonly code: IaImportErrorCode
  readonly detail: string | null

  constructor(code: IaImportErrorCode, detail: string | null = null) {
    super(detail ? `${code}: ${detail}` : code)
    this.name = 'IaImportError'
    this.code = code
    this.detail = detail
  }
}

/** Where one verified file ended up. */
export type IaImportOutcome = {
  imported: Array<{ index: number; itemId: string }>
  confirmedExisting: Array<{ index: number; itemId: string }>
  failed: Array<{ index: number; code: IaErrorCode }>
  notProcessed: number[]
  cancelled: boolean
}

/** The file in progress with the counts so far. */
export type IaImportProgress = {
  /** Index of the file being imported, as `iaListVerifiedFiles` numbers it. */
  index: number
  /** Display name of the file being imported. */
  displayFileName: string
  /** Files fully imported so far. */
  imported: number
  /** Files confirmed as already in the collection so far. */
  confirmedExisting: number
  /** Files that failed so far. */
  failed: number
  /** The importer's stage while it copies the file; absent outside the import. */
  stage?: ImportStage
}

export type IaImportOptions = {
  /**
   * The message the importer wraps its per-file errors in. Neutral English
   * here; the UI passes the translated one.
   */
  baseErrorMessage?: string
  onProgress?: (progress: IaImportProgress) => void
  shouldCancel?: () => boolean
}

/** Leftover entries released from the reconcile list, with what failed. */
export type IaReconcileOutcome = {
  released: number
  failed: Array<{ jobId: string; index: number; code: IaErrorCode }>
}

const DEFAULT_BASE_ERROR_MESSAGE = 'Import Internet Archive file'

type IaImportRun = {
  jobId: string
  collectionId: string
  baseErrorMessage: string
  onProgress?: (progress: IaImportProgress) => void
  outcome: IaImportOutcome
}

function report(run: IaImportRun, file: IaVerifiedFile, stage?: ImportStage) {
  run.onProgress?.({
    index: file.index,
    displayFileName: file.displayFileName,
    imported: run.outcome.imported.length,
    confirmedExisting: run.outcome.confirmedExisting.length,
    failed: run.outcome.failed.length,
    ...(stage ? { stage } : {}),
  })
}

function fail(run: IaImportRun, index: number, reason: unknown) {
  run.outcome.failed.push({ index, code: parseIaError(reason).code })
}

// The file is `Importing`: tell Rust its frontend import failed. A refusal
// keeps the recorded failure; the file stays for reconcile to classify.
async function reportFailed(run: IaImportRun, index: number) {
  try {
    await iaFailFileImport(run.jobId, index)
  } catch (reason) {
    console.warn('[internet-archive] could not report the failed import:', reason)
  }
}

// The item exists but will not be confirmed: drop it so no broken remnant
// stays in the collection. A later run then begins the file from scratch.
// A refusal never fails the run: it is logged and reported as `false`, so
// the caller keeps the file's original failure code. The remnant surfaces
// as `discard_partial` on a later run, so nothing is lost.
async function dropUnconfirmed(run: IaImportRun, itemId: string): Promise<boolean> {
  try {
    await discardFailedImport(run.collectionId, itemId)
    return true
  } catch (reason) {
    console.warn('[internet-archive] could not discard the failed import:', reason)
    return false
  }
}

// Confirm the item for one `Importing` file. A refusal means Rust already
// moved the file back to `verified`: record it and continue, never retry.
async function confirm(run: IaImportRun, file: IaVerifiedFile, itemId: string) {
  try {
    await iaConfirmFileImported(run.jobId, file.index, itemId)
  } catch (reason) {
    await dropUnconfirmed(run, itemId)
    fail(run, file.index, reason)
    return
  }
  run.outcome.imported.push({ index: file.index, itemId })
}

// Import the payload Rust handed over for one file, then confirm the single
// item it must create. Anything else (a rejection, an import error, or not
// exactly one item) drops what will not be confirmed and reports the file.
async function importPayload(run: IaImportRun, file: IaVerifiedFile) {
  let created: Array<{ id: string; title: string }>
  let importErrors: string[]
  try {
    const result = await importClassifiedPathsIntoCollection([file.stagingPath], run.collectionId, {
      baseErrorMessage: run.baseErrorMessage,
      overrides: {
        title: file.itemTitle,
        fileName: file.displayFileName,
        extraMetadata: file.extraMetadata as Record<string, unknown> | undefined,
        // Rust already classified the file; the import's own
        // "same file again" check must not skip it.
        allowDuplicate: true,
      },
      onProgress: (progress) => report(run, file, progress.stage),
    })
    created = result.createdItems
    importErrors = result.importErrors
  } catch (reason) {
    await reportFailed(run, file.index)
    fail(run, file.index, reason)
    return
  }
  if (created.length !== 1) {
    // The importer collects per-file errors instead of throwing; keep them
    // visible, since the outcome only carries a code. There is no Rust code
    // for "nothing (or too much) came out", so this records `unknown`.
    if (importErrors.length > 0) {
      console.warn('[internet-archive] the import failed:', importErrors.join('; '))
    }
    for (const item of created) await dropUnconfirmed(run, item.id)
    await reportFailed(run, file.index)
    run.outcome.failed.push({ index: file.index, code: 'unknown' })
    return
  }
  await confirm(run, file, created[0]!.id)
}

// One verified file: begin, then confirm the existing item, drop a partial
// remnant and begin once more, or import the payload. A second remnant for
// the same file records `partial_remains` and the loop moves on.
async function importFile(run: IaImportRun, listed: IaVerifiedFile) {
  let begin: IaBeginResult
  try {
    begin = await iaBeginFileImport(run.jobId, listed.index)
  } catch (reason) {
    fail(run, listed.index, reason)
    return
  }
  if (begin.action === 'confirm_existing') {
    if (!begin.itemId) {
      run.outcome.failed.push({ index: listed.index, code: 'unknown' })
      return
    }
    try {
      await iaConfirmFileImported(run.jobId, listed.index, begin.itemId)
    } catch (reason) {
      fail(run, listed.index, reason)
      return
    }
    run.outcome.confirmedExisting.push({ index: listed.index, itemId: begin.itemId })
    return
  }
  if (begin.action === 'discard_partial') {
    if (begin.itemId) {
      const discarded = await dropUnconfirmed(run, begin.itemId)
      if (!discarded) {
        // The remnant is still there and the discard itself failed: leave
        // it for a later run instead of beginning again. The file stays
        // `verified`: nothing to report.
        run.outcome.failed.push({ index: listed.index, code: 'partial_remains' })
        return
      }
    }
    let again: IaBeginResult
    try {
      again = await iaBeginFileImport(run.jobId, listed.index)
    } catch (reason) {
      fail(run, listed.index, reason)
      return
    }
    if (again.action === 'discard_partial') {
      // The remnant is still there after one discard; leave it for a later
      // run instead of looping. The file stays `verified`: nothing to report.
      run.outcome.failed.push({ index: listed.index, code: 'partial_remains' })
      return
    }
    begin = again
  }
  // `import`: the payload rides on begin, falling back to the listed file.
  await importPayload(run, begin.file ?? listed)
}

/**
 * Import every verified file of `jobId` into its target collection, in index
 * order. Reads the job once; fails with `IaImportError` when it names no
 * target collection.
 */
export async function importVerifiedFiles(
  jobId: string,
  options: IaImportOptions = {}
): Promise<IaImportOutcome> {
  const { baseErrorMessage = DEFAULT_BASE_ERROR_MESSAGE, onProgress, shouldCancel } = options
  const status = await iaGetImportStatus(jobId)
  const collectionId = status.targetCollectionId
  if (!collectionId) {
    throw new IaImportError('missing_collection', `job ${jobId} has no target collection`)
  }
  const run: IaImportRun = {
    jobId,
    collectionId,
    baseErrorMessage,
    onProgress,
    outcome: {
      imported: [],
      confirmedExisting: [],
      failed: [],
      notProcessed: [],
      cancelled: false,
    },
  }
  const files = [...(await iaListVerifiedFiles(jobId))].sort((a, b) => a.index - b.index)
  for (const [at, file] of files.entries()) {
    // The UI cancels the job on the Rust side; stop here and name the files
    // this run never reached.
    if (shouldCancel?.()) {
      run.outcome.cancelled = true
      run.outcome.notProcessed.push(...files.slice(at).map((rest) => rest.index))
      break
    }
    report(run, file)
    await importFile(run, file)
  }
  return run.outcome
}

/**
 * Release every recovery leftover: `discard` drops the partial remnant from
 * the owning job's collection first, `keep` and `none` release straight
 * away. Failures are recorded per entry; one never stops the others.
 */
export async function resolveReconcileLeftovers(): Promise<IaReconcileOutcome> {
  const outcome: IaReconcileOutcome = { released: 0, failed: [] }
  for (const entry of await iaListReconcile()) {
    try {
      if (entry.action === 'discard') {
        const status = await iaGetImportStatus(entry.jobId)
        const collectionId = status.targetCollectionId
        if (!collectionId) {
          throw new IaImportError(
            'missing_collection',
            `job ${entry.jobId} has no target collection`
          )
        }
        if (entry.itemId) await discardFailedImport(collectionId, entry.itemId)
      }
      await iaFinishReconcile(entry.jobId, entry.index)
      outcome.released += 1
    } catch (reason) {
      outcome.failed.push({
        jobId: entry.jobId,
        index: entry.index,
        code: parseIaError(reason).code,
      })
    }
  }
  return outcome
}
