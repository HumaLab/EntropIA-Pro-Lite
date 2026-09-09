import { open } from '@tauri-apps/plugin-dialog'
import { copyFile, mkdir, readFile, remove, stat } from '@tauri-apps/plugin-fs'
import { join } from '@tauri-apps/api/path'
import { convertFileSrc } from '@tauri-apps/api/core'
import { invoke } from '@tauri-apps/api/core'
import { getAssetPathLabel } from './item-metadata'

const SUPPORTED_IMAGES = ['png', 'jpg', 'jpeg', 'webp', 'tiff', 'tif']
const SUPPORTED_AUDIO = ['wav', 'mp3', 'flac', 'm4a', 'aac', 'ogg']
export const SUPPORTED_FORMATS = [...SUPPORTED_IMAGES, 'pdf', ...SUPPORTED_AUDIO]

export interface ImportedFile {
  originalName: string
  originalPath: string
  destPath: string
  type: 'image' | 'pdf' | 'audio'
  size: number
  originalMetadata: ImportedFileMetadata
}

export interface ImportedFileMetadata {
  originalName: string
  originalPath: string
  importedAt: string
  sizeBytes: number
  readonly?: boolean
  isFile?: boolean
  isDirectory?: boolean
  createdAt?: number | null
  modifiedAt?: number | null
  accessedAt?: number | null
}

export interface ImportFromPathsResult {
  imported: ImportedFile[]
  rejected: string[]
  skippedDuplicatePaths: number
}

/** A single rendered PDF page returned by the backend. */
export interface RenderedPage {
  page_number: number
  png_path: string
}

/**
 * Classify a filename by its extension.
 * Returns 'image', 'pdf', 'audio', or null if unsupported.
 */
export function classifyFileType(filename: string): 'image' | 'pdf' | 'audio' | null {
  const ext = filename.split('.').pop()?.toLowerCase() ?? ''
  if (SUPPORTED_IMAGES.includes(ext)) return 'image'
  if (ext === 'pdf') return 'pdf'
  if (SUPPORTED_AUDIO.includes(ext)) return 'audio'
  return null
}

/**
 * Open a file picker dialog and return the selected file paths.
 * Does NOT copy or classify files — the caller handles that.
 */
export async function pickFiles(): Promise<string[]> {
  try {
    const selected = await open({
      multiple: true,
      filters: [
        {
          name: 'Documents',
          extensions: SUPPORTED_FORMATS,
        },
      ],
    })

    if (!selected) return []
    return Array.isArray(selected) ? selected : [selected]
  } catch (e) {
    console.error('[file-import] pickFiles error:', e)
    throw new Error(`Failed to open file dialog: ${e instanceof Error ? e.message : String(e)}`)
  }
}

/**
 * Open a file picker dialog, copy selected files into the app data directory,
 * and return metadata about imported files.
 */
export async function pickAndImportFiles(
  collectionId: string,
  itemId: string
): Promise<ImportedFile[]> {
  try {
    const selected = await open({
      multiple: true,
      filters: [
        {
          name: 'Documents',
          extensions: SUPPORTED_FORMATS,
        },
      ],
    })

    if (!selected) return []

    const files = Array.isArray(selected) ? selected : [selected]

    const result = await importFilesFromPaths(files, collectionId, itemId)
    return result.imported
  } catch (e) {
    console.error('[file-import] pickAndImportFiles error:', e)
    throw new Error(`Failed to open file dialog: ${e instanceof Error ? e.message : String(e)}`)
  }
}

/**
 * Copy a single file into the app data directory under `{collectionId}/{itemId}/`.
 * Returns the destination path.
 */
async function copyFileToItem(
  sourcePath: string,
  collectionId: string,
  itemId: string
): Promise<string> {
  const dataDir = await invoke<string>('resolve_data_dir')
  const destDir = await join(dataDir, 'assets', collectionId, itemId)
  await mkdir(destDir, { recursive: true })

  const name = sourcePath.split(/[/\\]/).pop() ?? 'unknown'
  const destPath = await join(destDir, `${crypto.randomUUID()}_${name}`)
  await copyFile(sourcePath, destPath)
  // The copy needs the absolute destination; the caller stores the return value
  // in `assets.path`, which holds the relative key.
  return toStoredAssetPath(destPath)
}

function timestampFromFsDate(value: unknown): number | null {
  if (value instanceof Date) return value.getTime()
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (typeof value === 'string') {
    const parsed = Date.parse(value)
    return Number.isFinite(parsed) ? parsed : null
  }
  return null
}

async function readOriginalFileMetadata(sourcePath: string, originalName: string): Promise<ImportedFileMetadata> {
  const metadata = await stat(sourcePath)
  const sizeBytes = Number(metadata.size ?? 0)

  return {
    originalName,
    originalPath: sourcePath,
    importedAt: new Date().toISOString(),
    sizeBytes: Number.isFinite(sizeBytes) ? sizeBytes : 0,
    readonly: metadata.readonly,
    isFile: metadata.isFile,
    isDirectory: metadata.isDirectory,
    createdAt: timestampFromFsDate(metadata.birthtime),
    modifiedAt: timestampFromFsDate(metadata.mtime),
    accessedAt: timestampFromFsDate(metadata.atime),
  }
}

/**
 * Classify and validate a batch of file paths.
 * Returns classified files ready to be imported and rejected filenames.
 */
export function classifyFiles(filePaths: string[]): {
  classified: { sourcePath: string; name: string; type: 'image' | 'pdf' | 'audio' }[]
  rejected: string[]
} {
  const classified: { sourcePath: string; name: string; type: 'image' | 'pdf' | 'audio' }[] = []
  const rejected: string[] = []
  const seenSourcePaths = new Set<string>()

  for (const filePath of filePaths) {
    const normalizedSource = filePath.toLowerCase()
    if (seenSourcePaths.has(normalizedSource)) {
      continue // silently skip duplicates — caller can track if needed
    }
    seenSourcePaths.add(normalizedSource)

    const name = filePath.split(/[/\\]/).pop() ?? 'unknown'
    const type = classifyFileType(name)
    if (!type) {
      rejected.push(name)
      continue
    }

    classified.push({ sourcePath: filePath, name, type })
  }

  return { classified, rejected }
}

export async function importFilesFromPaths(
  filePaths: string[],
  collectionId: string,
  itemId: string
): Promise<ImportFromPathsResult> {
  const { classified, rejected } = classifyFiles(filePaths)
  const skippedDuplicatePaths = filePaths.length - classified.length - rejected.length

  const imported: ImportedFile[] = []

  for (const file of classified) {
    const originalMetadata = await readOriginalFileMetadata(file.sourcePath, file.name)
    const destPath = await copyFileToItem(file.sourcePath, collectionId, itemId)
    imported.push({
      originalName: file.name,
      originalPath: file.sourcePath,
      destPath,
      type: file.type,
      size: originalMetadata.sizeBytes,
      originalMetadata,
    })
  }

  return {
    imported,
    rejected,
    skippedDuplicatePaths,
  }
}

/**
 * Import a single file: copy it to the app data directory under its own item.
 * Returns the ImportedFile metadata.
 */
export async function importSingleFile(
  sourcePath: string,
  collectionId: string,
  itemId: string
): Promise<ImportedFile> {
  const name = sourcePath.split(/[/\\]/).pop() ?? 'unknown'
  const type = classifyFileType(name)
  if (!type) {
    throw new Error(`Unsupported file format: ${name}`)
  }

  const destPath = await copyFileToItem(sourcePath, collectionId, itemId)
  const originalMetadata = await readOriginalFileMetadata(sourcePath, name)
  return {
    originalName: name,
    originalPath: sourcePath,
    destPath,
    type,
    size: originalMetadata.sizeBytes,
    originalMetadata,
  }
}

/**
 * The data directory, resolved once.
 *
 * `getAssetUrl` is called synchronously from markup in seven places, so it
 * cannot await. The directory is primed at startup instead.
 */
let cachedDataDir: string | null = null

/**
 * Resolve and cache the data directory. Call once, at startup.
 *
 * Asks the backend rather than `appDataDir()`: that API resolves the
 * per-variant directory from the Tauri identifier, which is precisely what Lite
 * and Pro stopped using when they began sharing one archive.
 */
export async function primeDataDir(): Promise<void> {
  cachedDataDir = await invoke<string>('resolve_data_dir')
}

/** Test seam: forget the cached directory. */
export function resetDataDirCache(): void {
  cachedDataDir = null
}

const ABSOLUTE_PATH = /^([a-zA-Z]:[\\/]|\\\\|\/)/

/**
 * Convert a stored asset path to a URL that can be used in the webview.
 *
 * A stored path is either absolute — a row written before the relative-path
 * migration, or an external file that was never copied in — or a key relative
 * to the data directory. An absolute path is passed through unchanged, which
 * is also the fallback when the cache has not been primed yet.
 */
export function getAssetUrl(storedPath: string): string {
  return convertFileSrc(resolveStoredAssetPath(storedPath))
}

/**
 * Resolve a stored asset path to a real filesystem path.
 *
 * Anything that hands a stored path to a filesystem call — `remove`,
 * `copyFile`, a sibling-directory derivation — must go through this first. An
 * absolute value is returned unchanged, which is also the fallback when the
 * cache has not been primed.
 */
export function resolveStoredAssetPath(storedPath: string): string {
  if (cachedDataDir === null || ABSOLUTE_PATH.test(storedPath)) {
    return storedPath
  }
  const separator = /[\\/]$/.test(cachedDataDir) ? '' : '/'
  return `${cachedDataDir}${separator}${storedPath}`
}

/**
 * The inverse of {@link resolveStoredAssetPath}: turn an absolute path into the
 * value to write into `assets.path`.
 *
 * A path outside the data directory is returned unchanged — an external file
 * that was never copied in is still a legitimate row, and refusing it would
 * lose the asset.
 */
export function toStoredAssetPath(absolutePath: string): string {
  if (cachedDataDir === null) return absolutePath

  const normalize = (value: string) => value.replace(/\\/g, '/').replace(/\/+$/, '')
  const path = normalize(absolutePath)
  const root = normalize(cachedDataDir)
  // Windows paths are case-insensitive; elsewhere the comparison is exact.
  const matches = path.toLowerCase().startsWith(`${root.toLowerCase()}/`)
  if (!matches) return absolutePath

  const relative = path.slice(root.length + 1)
  return relative.startsWith('assets/') ? relative : absolutePath
}

/**
 * Delete an asset file from the filesystem.
 *
 * - If the file does not exist (ENOENT/not-found), logs a warning and returns
 *   successfully — the DB cleanup should still proceed.
 * - If a permission error or other filesystem error occurs, throws so the
 *   caller can abort the deletion flow.
 */
export async function deleteAssetFile(storedPath: string): Promise<void> {
  try {
    await remove(resolveStoredAssetPath(storedPath))
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    // ENOENT / NotFound — file already gone, continue with DB cleanup
    if (
      message.includes('ENOENT') ||
      message.includes('not found') ||
      message.includes('NotFound')
    ) {
      console.warn('[file-import] Asset file not found, continuing with DB cleanup:', storedPath)
      return
    }
    // Permission error or other FS error — abort
    throw new Error(`Failed to delete asset file: ${message}`)
  }
}

// ---------------------------------------------------------------------------
// PDF Thumbnails
// ---------------------------------------------------------------------------

/**
 * Generate or retrieve a cached thumbnail for the first page of a PDF.
 *
 * Calls the Rust `generate_pdf_thumbnail` command, which renders the first
 * page at 400px width and caches the PNG at `{app_data_dir}/thumbnails/{asset_id}.png`.
 * If a cached thumbnail already exists, the cached path is returned immediately.
 *
 * Returns a webview-accessible URL via `convertFileSrc`.
 */
export async function generatePdfThumbnail(
  assetPath: string,
  assetId: string
): Promise<string> {
  const nativePath: string = await invoke('generate_pdf_thumbnail', {
    assetPath,
    assetId,
  })
  return convertFileSrc(nativePath)
}

function splitFileName(fileName: string) {
  const extensionIndex = fileName.lastIndexOf('.')
  if (extensionIndex <= 0) return { stem: fileName, extension: '' }
  return {
    stem: fileName.slice(0, extensionIndex),
    extension: fileName.slice(extensionIndex),
  }
}

function stripInternalVersionSuffix(stem: string) {
  return stem.replace(/_v\d+$/i, '')
}

function stripCopySuffix(stem: string) {
  return stem.replace(/_c\d+$/i, '')
}

/** Return the next visible copy name for an asset family. */
export function getNextAssetCopyName(sourcePath: string, existingPaths: string[]): string {
  const sourceName = getAssetPathLabel(sourcePath)
  const { stem: sourceStem, extension } = splitFileName(sourceName)
  const baseStem = stripCopySuffix(stripInternalVersionSuffix(sourceStem))
  const normalizedBase = baseStem.toLowerCase()
  let highestCopyNumber = 0

  for (const existingPath of existingPaths) {
    const existingName = getAssetPathLabel(existingPath)
    const { stem } = splitFileName(existingName)
    const normalizedStem = stripInternalVersionSuffix(stem).toLowerCase()
    const copyPrefix = `${normalizedBase}_c`
    if (!normalizedStem.startsWith(copyPrefix)) continue

    const suffix = normalizedStem.slice(copyPrefix.length)
    if (!/^\d+$/.test(suffix)) continue
    highestCopyNumber = Math.max(highestCopyNumber, Number(suffix))
  }

  return `${baseStem}_c${highestCopyNumber + 1}${extension}`
}

/** Copy an asset beside its source while keeping its current file format. */
export async function duplicateAssetFile(
  sourcePath: string,
  existingPaths: string[]
): Promise<{ name: string; path: string }> {
  const name = getNextAssetCopyName(sourcePath, existingPaths)
  const separatorIndex = Math.max(sourcePath.lastIndexOf('/'), sourcePath.lastIndexOf('\\'))
  const directory = separatorIndex >= 0 ? sourcePath.slice(0, separatorIndex + 1) : ''

  // The returned path is stored in `assets.path`, so it keeps the shape of the
  // source it was derived from. Only the filesystem copy needs resolving.
  const path = `${directory}${crypto.randomUUID()}_${name}`

  await copyFile(resolveStoredAssetPath(sourcePath), resolveStoredAssetPath(path))
  return { name, path }
}

export async function generateImageThumbnail(
  assetPath: string,
  assetId: string
): Promise<string> {
  const nativePath: string = await invoke('generate_image_thumbnail', {
    assetPath,
    assetId,
  })
  return convertFileSrc(nativePath)
}

export async function loadAudioPreviewBlob(assetPath: string): Promise<Blob> {
  try {
    const previewPath: string = await invoke('prepare_audio_preview', { assetPath })
    const bytes = await readFile(previewPath)
    return new Blob([bytes], { type: 'audio/wav' })
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`Audio preview preparation failed: ${message}`)
  }
}

/**
 * Delete a cached PDF thumbnail for an asset.
 *
 * Should be called when a PDF asset is deleted to clean up the thumbnail cache.
 * Silently succeeds even if the thumbnail doesn't exist.
 */
export async function deletePdfThumbnail(assetId: string): Promise<void> {
  await invoke('delete_pdf_thumbnail', { assetId })
}

export async function deleteImageThumbnail(assetId: string): Promise<void> {
  await invoke('delete_image_thumbnail', { assetId })
}

// ---------------------------------------------------------------------------
// Scanned PDF detection and page conversion
// ---------------------------------------------------------------------------

export type PageKind = 'Native' | 'ImageOnly' | 'ImageWithOcr' | 'Uncertain'

export type PageProfile = {
  page_number: number
  has_text: boolean
  text_chars: number
  image_count: number
  largest_image_ratio: number
  full_page_image_like: boolean
  kind: PageKind
}

export type DocumentProfile = {
  pages: PageProfile[]
  native_pages: number
  image_only_pages: number
  image_with_ocr_pages: number
  uncertain_pages: number
  dominant_kind: PageKind
  mixed: boolean
  should_render_as_images: boolean
}

/**
 * Build a conservative per-page PDF profile. Only confidently native PDFs stay
 * as PDF; uncertain, mixed, image-only, and image-with-OCR PDFs are routed to
 * per-page image rendering.
 */
export async function probePdf(assetPath: string): Promise<DocumentProfile> {
  return invoke<DocumentProfile>('probe_pdf', { assetPath })
}

/**
 * Check whether a PDF file should be split into per-page image assets.
 *
 * Backward-compatible wrapper around the conservative backend profile: returns
 * true for image-only, image-with-OCR, mixed, or uncertain PDFs.
 */
export async function isScannedPdf(assetPath: string): Promise<boolean> {
  return invoke<boolean>('is_scanned_pdf', { assetPath })
}

/**
 * Render all pages of a scanned PDF as individual PNG images.
 *
 * Calls the Rust `render_pdf_pages` command which renders each page at 300 DPI
 * and saves them to the specified output directory.
 *
 * @param pdfPath Absolute path to the source PDF file on the filesystem
 * @param outputDir Directory where PNG files will be saved
 * @param filenamePrefix Prefix for output filenames (e.g. "document" → "document_page_1.png")
 * @returns Array of {page_number, png_path} objects with absolute filesystem paths
 */
export async function renderPdfPages(
  pdfPath: string,
  outputDir: string,
  filenamePrefix: string
): Promise<RenderedPage[]> {
  return invoke<RenderedPage[]>('render_pdf_pages', {
    pdfPath,
    outputDir,
    filenamePrefix,
  })
}

/** A single-page PDF produced by splitting a multi-page PDF. */
export interface SplitPage {
  page_number: number
  pdf_path: string
}

/**
 * Split a multi-page PDF into one single-page PDF file per page.
 *
 * Each page is preserved as an independent PDF — no rasterization and no
 * recompression — so original quality is retained. Page order is preserved.
 *
 * @param pdfPath Absolute path to the source PDF file on the filesystem
 * @param outputDir Directory where single-page PDF files will be saved
 * @param filenamePrefix Prefix for output filenames (e.g. "document" → "document_page_1.pdf")
 * @returns Array of {page_number, pdf_path} objects with absolute filesystem paths
 */
export async function splitPdfPages(
  pdfPath: string,
  outputDir: string,
  filenamePrefix: string
): Promise<SplitPage[]> {
  return invoke<SplitPage[]>('split_pdf_pages', {
    pdfPath,
    outputDir,
    filenamePrefix,
  })
}
