/**
 * Shared file-import engine: classify -> create item -> copy file -> create
 * asset (splitting a PDF into one asset per page) -> dedupe against an
 * earlier import, one collection at a time.
 *
 * Extracted from `CollectionView.svelte` (T4, home-view) so the "Importar
 * fuentes" dialog on Inicio can run the exact same pipeline into a
 * chosen/newly-created collection, instead of duplicating it. The logic below
 * mirrors `CollectionView`'s former inline implementation function-for-function;
 * only the view-only concerns (Svelte `$state`, the import summary banner,
 * reloading the item list, the single-item auto-navigate) stayed in the view
 * and now call this module. Progress is reported through `onProgress` instead
 * of mutating component state directly, so any caller can observe it.
 */
import { getStore } from './db'
import {
  classifyFiles,
  importSingleFile,
  splitPdfPages,
  readSourceFingerprint,
  type ImportedFile,
} from './file-import'
import { join } from '@tauri-apps/api/path'
import { invoke } from '@tauri-apps/api/core'
import { remove, stat } from '@tauri-apps/plugin-fs'

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
}

export interface ImportClassifiedPathsOptions {
  baseErrorMessage: string
  onProgress?: (progress: ImportProgress) => void
}

export interface ImportSummary {
  imported: number
  skipped: number
  errors: string[]
  rejected: string[]
  alreadyImported: string[]
  lastItemTitle: string | null
}

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

async function readAssetSize(path: string): Promise<number | null> {
  try {
    const metadata = await stat(path)
    const size = Number(metadata.size ?? 0)
    return Number.isFinite(size) ? size : null
  } catch (e) {
    console.warn('[CollectionView] Failed to read rendered page size:', e)
    return null
  }
}

/**
 * Split a multi-page PDF into one single-page PDF asset per page.
 *
 * Each page is preserved as an independent PDF (no rasterization) and linked
 * to the parent asset via parentAssetId/pageNumber. Returns the list of
 * created child asset IDs.
 */
async function splitPdfIntoPageAssets(
  imported: ImportedFile,
  collectionId: string,
  itemId: string,
  store: ReturnType<typeof getStore>,
  parentAssetId: string
): Promise<string[]> {
  const dataDir = await invoke<string>('resolve_data_dir')
  const outputDir = await join(dataDir, 'assets', collectionId, itemId)

  const baseName = imported.originalName.replace(/\.[^.]+$/, '')
  const pages = await splitPdfPages(imported.destPath, outputDir, baseName)
  if (pages.length === 0) {
    throw new Error('PDF splitting produced no pages')
  }

  const assetIds: string[] = []
  for (const page of pages) {
    const asset = await store.assets.create({
      itemId,
      path: page.pdf_path,
      type: 'pdf',
      sortIndex: page.page_number - 1,
      size: await readAssetSize(page.pdf_path),
      parentAssetId,
      pageNumber: page.page_number,
    })
    assetIds.push(asset.id)
  }

  console.log(`[CollectionView] Split PDF into ${pages.length} single-page PDF assets`)
  return assetIds
}

/**
 * Remove everything a failed import created: its assets, its item and the
 * folder its files were copied into.
 *
 * When splitting fails the parent asset already exists, so deleting the item
 * alone trips the assets foreign key. The assets go first. The item cascade
 * is not an option: it also deletes the collection when this was its first
 * document.
 */
async function discardFailedImport(collectionId: string, itemId: string) {
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

export function formatImportStageError(baseMessage: string, stage: string, e: unknown): string {
  return `${baseMessage} (${stage}): ${getErrorDetails(e)}`
}

const IMPORTED_FILE_METADATA_KEY = '__entropia_file_metadata'

function buildImportedItemMetadata(imported: ImportedFile): string {
  return JSON.stringify({
    [IMPORTED_FILE_METADATA_KEY]: imported.originalMetadata,
  })
}

async function finalizeImportedItem(
  collectionId: string,
  itemId: string,
  imported: ImportedFile,
  onStage: (stage: ImportStage) => void
) {
  const store = getStore()

  // Every PDF is decomposed into one single-page PDF asset per page. The
  // original stays only as the parent container and is never processed itself.
  if (imported.type === 'pdf') {
    const parentAsset = await store.assets.create({
      itemId,
      path: imported.destPath,
      type: 'pdf',
      size: imported.size,
      sortIndex: 0,
    })

    onStage('renderingPdf')
    await splitPdfIntoPageAssets(imported, collectionId, itemId, store, parentAsset.id)
    return
  }

  // Default: create a single asset for the imported file
  await store.assets.create({
    itemId,
    path: imported.destPath,
    type: imported.type,
    size: imported.size,
    sortIndex: 0,
  })
}

// The same file, unchanged, already imported into this collection: importing
// it again would only duplicate the document. When the check itself fails,
// the file is imported as before rather than silently dropped.
async function isAlreadyImported(collectionId: string, sourcePath: string) {
  try {
    const source = await readSourceFingerprint(sourcePath)
    return (await getStore().items.findImportedFromSource(collectionId, source)) !== null
  } catch (e) {
    console.warn('[CollectionView] Could not check for an earlier import:', e)
    return false
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
  const { baseErrorMessage, onProgress } = options
  const store = getStore()

  // Classify files before creating items or copying assets.
  const { classified, rejected } = classifyFiles(paths)

  if (classified.length === 0) {
    return {
      classifiedCount: 0,
      rejected,
      createdItems: [],
      importErrors: [],
      alreadyImported: [],
    }
  }

  // Create one item per file, copy file, create asset.
  // Failures are collected per file so every error stays visible in the
  // import summary; one bad file no longer aborts the remaining imports.
  const createdItems: Array<{ id: string; title: string }> = []
  const importErrors: string[] = []
  const alreadyImported: string[] = []
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
    const title = file.name.replace(/\.[^.]+$/, '')
    let itemId: string | null = null
    try {
      updateProgress({ currentFileName: file.name, stage: 'creatingDocument' })
      if (await isAlreadyImported(collectionId, file.sourcePath)) {
        alreadyImported.push(file.name)
        updateProgress({ skipped: progress.skipped + 1 })
        continue
      }
      const item = await store.items.create({
        title,
        collectionId,
        metadata: null,
      })
      itemId = item.id

      updateProgress({ stage: 'copyingFile' })
      const imported = await importSingleFile(file.sourcePath, collectionId, itemId)
      updateProgress({ stage: 'savingDocument' })
      await store.items.update(itemId, { metadata: buildImportedItemMetadata(imported) })
      await finalizeImportedItem(collectionId, itemId, imported, (stage) =>
        updateProgress({ stage })
      )
      createdItems.push({ id: itemId, title })
      updateProgress({ imported: progress.imported + 1 })
    } catch (e) {
      if (itemId) await discardFailedImport(collectionId, itemId)
      const stage = itemId ? `importing ${file.name}` : 'creating item'
      importErrors.push(formatImportStageError(baseErrorMessage, stage, e))
      updateProgress({ failed: progress.failed + 1 })
    } finally {
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
  }
}
