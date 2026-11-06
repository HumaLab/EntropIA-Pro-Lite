/**
 * Shared file-import engine: classify, then import one file at a time into one
 * collection, deduping against an earlier import of the same unchanged file.
 *
 * Extracted from `CollectionView.svelte` (T4, home-view) so the "Importar
 * fuentes" dialog on Inicio can run the exact same pipeline into a
 * chosen/newly-created collection. The import itself runs on the Rust ingest
 * service (`corpus::ingest`, `odd/tasks/agent-corpus-tools.md` D10), the same
 * one the agent path uses: the renderer classifies the paths and asks
 * `corpus_import_path` once per file, so the copy, the PDF split, the item, its
 * metadata and its asset rows are written by one service with one set of rules.
 *
 * What stays here is what the renderer legitimately owns: which files the app
 * recognizes (`classifyFiles`), the in-process claim map that keeps two imports
 * of one path apart, per-file error collection so one bad file never aborts the
 * batch, and the progress the views show. Progress is reported through
 * `onProgress` instead of mutating component state directly.
 */
import { getStore } from './db'
import { classifyFiles } from './file-import'
import { join } from '@tauri-apps/api/path'
import { invoke } from '@tauri-apps/api/core'
import { remove } from '@tauri-apps/plugin-fs'

export type ImportStage =
  | 'creatingDocument'
  | 'copyingFile'
  | 'savingDocument'
  | 'inspectingPdf'
  | 'renderingPdf'
  | 'completed'

export interface ImportProgress {
  total: number
  completed: number
  imported: number
  failed: number
  skipped: number
  currentFileName: string | null
  stage: ImportStage
}

export interface ImportClassifiedPathsResult {
  /** How many source paths classifyFiles recognized, before any were imported. */
  classifiedCount: number
  rejected: string[]
  createdItems: Array<{ id: string; title: string }>
  importErrors: string[]
  alreadyImported: string[]
  /**
   * The backend refused the import because the target collection no longer
   * exists (`not_found: no collection <id>`): it was deleted on this device or
   * applied from a synced one while the view still held its id. Callers say so
   * in their own words instead of repeating the raw error.
   */
  missingCollection: boolean
}

/**
 * What a single imported file may bring of its own. Only meaningful when one
 * path is imported (a file that already has an identity, such as a saved web
 * capture); a batch of picked files takes everything from the files.
 */
export interface ImportItemOverrides {
  /** The item's title; by default the file name without its extension. */
  title?: string
  /** The name the stored copy and its original-name metadata carry. */
  fileName?: string
  /**
   * Reserved metadata written beside `__entropia_file_metadata`. It can add keys
   * but never replace that one.
   */
  extraMetadata?: Record<string, unknown>
  /** Import even when this same source file was already imported here. */
  allowDuplicate?: boolean
}

export interface ImportClassifiedPathsOptions {
  baseErrorMessage: string
  onProgress?: (progress: ImportProgress) => void
  overrides?: ImportItemOverrides
}

export interface ImportSummary {
  imported: number
  skipped: number
  errors: string[]
  rejected: string[]
  alreadyImported: string[]
  lastItemTitle: string | null
}

/** One file's import as `corpus_import_path` answers it. */
type ImportPathOutcome =
  | { outcome: 'imported'; itemId: string; title: string; assets: number }
  | { outcome: 'duplicate'; itemId: string }

/**
 * Reduce an {@link ImportClassifiedPathsResult} to the counts and message
 * every caller shows: `CollectionView`'s own import summary banner and the
 * "Importar fuentes" dialog's result screen both call this, so "skipped
 * means rejected + already imported" and "the last item only opens when
 * nothing failed" stay defined in exactly one place.
 */
export function buildImportSummary(result: ImportClassifiedPathsResult): ImportSummary {
  const hasFailures = result.importErrors.length > 0 || result.rejected.length > 0
  const lastCreated = result.createdItems.at(-1) ?? null
  return {
    imported: result.createdItems.length,
    skipped: result.rejected.length + result.alreadyImported.length,
    errors: result.importErrors,
    rejected: result.rejected,
    alreadyImported: result.alreadyImported,
    lastItemTitle: hasFailures ? null : (lastCreated?.title ?? null),
  }
}

/**
 * Remove everything a failed import created: its assets, its item and the
 * folder its files were copied into.
 *
 * Exported (unchanged) so the Internet Archive import flow reuses it to drop
 * partial remnants and failed items.
 *
 * The import service cleans up after itself, so this is for the flows that
 * begin an import, keep its item and then cannot confirm it.
 */
export async function discardFailedImport(collectionId: string, itemId: string) {
  const store = getStore()
  try {
    const assets = await store.assets.findByItem(itemId)
    for (const asset of assets.filter((candidate) => !candidate.parentAssetId)) {
      await store.assets.deleteWithCascade(asset.id)
    }
    await store.items.delete(itemId)
  } catch (e) {
    console.warn('[CollectionView] A failed import left its item behind:', e)
  }

  try {
    const dataDir = await invoke<string>('resolve_data_dir')
    await remove(await join(dataDir, 'assets', collectionId, itemId), { recursive: true })
  } catch (e) {
    console.warn('[CollectionView] A failed import left its files behind:', e)
  }
}

function getErrorDetails(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

/**
 * The backend's code for an import whose collection is gone
 * (`CorpusError::NotFound` in `corpus::ingest`). Matched as a prefix, never
 * against the localized sentence, so any rewording of the message keeps it
 * working.
 */
const MISSING_COLLECTION_PREFIX = 'not_found: no collection'

/** Whether a refused import failed because its collection no longer exists. */
function isMissingCollectionError(error: unknown): boolean {
  return getErrorDetails(error).trimStart().startsWith(MISSING_COLLECTION_PREFIX)
}

export function formatImportStageError(baseMessage: string, stage: string, e: unknown): string {
  return `${baseMessage} (${stage}): ${getErrorDetails(e)}`
}

/**
 * What a refused file says in the summary. The backend answers with a stable
 * code and a sentence (`missing: …`, `too_large: …`); `unsupported` keeps the
 * message the renderer has always shown for it, and every other code keeps the
 * service's own sentence, which names the file it is about.
 */
function importFailureDetail(name: string, error: unknown): string {
  const detail = getErrorDetails(error)
  const separator = detail.indexOf(': ')
  if (separator === -1) return detail
  if (detail.slice(0, separator) === 'unsupported') {
    return `Unsupported file format: ${name}`
  }
  return detail.slice(separator + 2)
}

// `corpus_import_path` is check-then-act: two imports of the same file running
// at once (one drop reaching several live handlers, a double drop, the Inicio
// dialog alongside a drop) would both be told "not here" before either created
// its item. Each call claims `collectionId + path` before its request and holds
// it until the file settles. A concurrent call finding the claim waits for that
// outcome: the file is reported as already imported only if the other import
// actually left it in the collection; if that import failed, this call claims
// the path and imports the file itself (drop-dup fix).
//
// The value resolves to whether the file is in the collection once the claim
// is released.
const importsInFlight = new Map<string, Promise<boolean>>()

// Intentionally case-sensitive: folding case here could skip a distinct file
// on a case-sensitive filesystem. Explorer hands every handler of one drop the
// same string, which is the duplicate this guards against; a case variant of
// an already-imported path is still caught afterwards by the service's own
// duplicate check, which matches the original path case-insensitively, with
// its size and modification time.
function inFlightKey(collectionId: string, sourcePath: string) {
  return `${collectionId}\u0000${sourcePath.trim().replace(/\\/g, '/')}`
}

/** Claims `claim` and returns its release, which reports the outcome. */
function claimImport(claim: string): (inCollection: boolean) => void {
  let settle: (inCollection: boolean) => void = () => {}
  importsInFlight.set(
    claim,
    new Promise<boolean>((resolve) => {
      settle = resolve
    })
  )
  return (inCollection) => {
    importsInFlight.delete(claim)
    settle(inCollection)
  }
}

/**
 * Classify, import and finalize a batch of source paths into one collection.
 *
 * Failures are collected per file so every error stays visible to the
 * caller; one bad file never aborts the remaining imports. `onProgress` is
 * called after every state change, mirroring the exact points
 * `CollectionView`'s own `importProgress` used to update.
 */
export async function importClassifiedPathsIntoCollection(
  paths: string[],
  collectionId: string,
  options: ImportClassifiedPathsOptions
): Promise<ImportClassifiedPathsResult> {
  const { baseErrorMessage, onProgress, overrides } = options

  // Classify files before asking the backend for anything.
  const { classified, rejected } = classifyFiles(paths)

  if (classified.length === 0) {
    return {
      classifiedCount: 0,
      rejected,
      createdItems: [],
      importErrors: [],
      alreadyImported: [],
      missingCollection: false,
    }
  }

  // One request per file. Failures are collected per file so every error
  // stays visible in the import summary; one bad file no longer aborts the
  // remaining imports.
  const createdItems: Array<{ id: string; title: string }> = []
  const importErrors: string[] = []
  const alreadyImported: string[] = []
  let missingCollection = false
  let progress: ImportProgress = {
    total: classified.length,
    completed: 0,
    imported: 0,
    failed: 0,
    skipped: rejected.length,
    currentFileName: null,
    stage: 'creatingDocument',
  }
  onProgress?.(progress)

  function updateProgress(update: Partial<ImportProgress>) {
    progress = { ...progress, ...update }
    onProgress?.(progress)
  }

  for (const file of classified) {
    const claim = inFlightKey(collectionId, file.sourcePath)
    let release: ((inCollection: boolean) => void) | null = null
    let inCollection = false
    try {
      updateProgress({ currentFileName: file.name, stage: 'creatingDocument' })
      let importedConcurrently = false
      for (let other = importsInFlight.get(claim); other; other = importsInFlight.get(claim)) {
        if (await other) {
          importedConcurrently = true
          break
        }
      }
      if (!importedConcurrently) {
        // Claimed synchronously after the last lookup found no claim: no
        // await sits between that lookup and this claim, so a concurrent
        // call cannot slip in between.
        release = claimImport(claim)
        const outcome = await invoke<ImportPathOutcome>('corpus_import_path', {
          collectionId,
          sourcePath: file.sourcePath,
          overrides: overrides ?? null,
        })
        if (outcome.outcome === 'duplicate') {
          inCollection = true
          alreadyImported.push(file.name)
          updateProgress({ skipped: progress.skipped + 1 })
          continue
        }
        createdItems.push({ id: outcome.itemId, title: outcome.title })
        inCollection = true
        updateProgress({ imported: progress.imported + 1 })
        continue
      }
      inCollection = true
      alreadyImported.push(file.name)
      updateProgress({ skipped: progress.skipped + 1 })
    } catch (e) {
      missingCollection = missingCollection || isMissingCollectionError(e)
      importErrors.push(
        formatImportStageError(
          baseErrorMessage,
          `importing ${file.name}`,
          importFailureDetail(file.name, e)
        )
      )
      updateProgress({ failed: progress.failed + 1 })
    } finally {
      release?.(inCollection)
      // Every classified source file completes exactly once, including failures.
      updateProgress({ completed: progress.completed + 1, stage: 'completed' })
    }
  }

  return {
    classifiedCount: classified.length,
    rejected,
    createdItems,
    importErrors,
    alreadyImported,
    missingCollection,
  }
}
