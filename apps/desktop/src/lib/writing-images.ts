import { invoke } from '@tauri-apps/api/core'
import { join } from '@tauri-apps/api/path'
import { exists, mkdir, writeFile } from '@tauri-apps/plugin-fs'
import { sha256Hex } from './writing-image-hash'

/**
 * Content-addressed storage for manuscript images (spec, Managed Image
 * Storage). One import function for every entry path: the toolbar picker,
 * paste, and drop all call this, so storage and deduplication have one
 * implementation. Follows writing-crops.ts's pattern and fixes its one gap:
 * the file name is the content's own hash, not a fresh UUID per insertion.
 */

export const WRITING_IMAGES_DIR = 'writing-images'

export type WritingImageMediaType = 'image/png' | 'image/jpeg' | 'image/gif'

const EXTENSIONS: Record<WritingImageMediaType, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
}

export interface WritingImageIo {
  dataDir(): Promise<string>
  join(...parts: string[]): Promise<string>
  mkdir(path: string): Promise<void>
  exists(path: string): Promise<boolean>
  writeFile(path: string, bytes: Uint8Array): Promise<void>
}

const tauriIo: WritingImageIo = {
  dataDir: () => invoke<string>('resolve_data_dir'),
  join: (...parts) => join(...parts),
  mkdir: (path) => mkdir(path, { recursive: true }),
  exists: (path) => exists(path),
  writeFile: (path, bytes) => writeFile(path, bytes),
}

/** The media type detected from the bytes' own header, never from a
 *  filename — never PNG/JPEG/GIF just because the extension said so. */
export function detectWritingImageType(bytes: Uint8Array): WritingImageMediaType | null {
  if (
    bytes.length >= 8 &&
    [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a].every((byte, index) => bytes[index] === byte)
  ) {
    return 'image/png'
  }
  if (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
    return 'image/jpeg'
  }
  if (
    bytes.length >= 6 &&
    bytes[0] === 0x47 &&
    bytes[1] === 0x49 &&
    bytes[2] === 0x46 &&
    bytes[3] === 0x38 &&
    (bytes[4] === 0x37 || bytes[4] === 0x39) &&
    bytes[5] === 0x61
  ) {
    return 'image/gif'
  }
  return null
}

export interface ImportedWritingImage {
  /** Relative to the data directory: `writing-images/<sha256>.<ext>`. */
  path: string
  mediaType: WritingImageMediaType
}

/**
 * Reads the bytes' own hash and writes them once, at a name only their
 * content decides. `null` means the bytes are not an accepted format:
 * nothing is written, nothing is returned to insert (spec, Failure Handling).
 */
export async function importWritingImage(
  bytes: Uint8Array,
  io: WritingImageIo = tauriIo
): Promise<ImportedWritingImage | null> {
  const mediaType = detectWritingImageType(bytes)
  if (!mediaType) return null

  const hash = await sha256Hex(bytes)
  const extension = EXTENSIONS[mediaType]
  const relativePath = `${WRITING_IMAGES_DIR}/${hash}.${extension}`

  const dataDir = await io.dataDir()
  const root = await io.join(dataDir, WRITING_IMAGES_DIR)
  await io.mkdir(root)

  const absolute = await io.join(dataDir, relativePath)
  if (!(await io.exists(absolute))) {
    await io.writeFile(absolute, bytes)
  }

  return { path: relativePath, mediaType }
}
