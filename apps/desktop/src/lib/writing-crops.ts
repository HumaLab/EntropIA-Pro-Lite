import { invoke } from '@tauri-apps/api/core'
import { join } from '@tauri-apps/api/path'
import { mkdir, writeFile } from '@tauri-apps/plugin-fs'
import type { QuotePart } from './rendered-selection'

/**
 * The images a citation quotes, kept as files in the archive.
 *
 * # Why a copy and not the region it came from
 *
 * A region is a rectangle of a scan: `![](page=0,bbox=[…])` in the extraction,
 * cropped on the fly for the screen. A citation cannot be that. §10.1 has the
 * citation carry a snapshot of what was quoted precisely so it survives its
 * source being corrected, re-read or deleted — the words already do, and an
 * image that changed under a finished paragraph would be worse than a stale
 * one, because nothing would say it changed.
 *
 * # Why a file and not the document
 *
 * A crop is hundreds of kilobytes. The manuscript is written on every save, in
 * every version and in the recovery journal (§16.1), so an image inside it
 * would be written again and again. A path costs forty bytes.
 *
 * Stored relative, like every asset path, so `getAssetUrl` resolves it and the
 * archive stays movable.
 */

/** Where crops live inside the archive. */
export const CROPS_DIR = 'writing-crops'

const EXTENSIONS: Record<string, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/webp': 'webp',
  'image/gif': 'gif',
}

export interface CropIo {
  dataDir(): Promise<string>
  join(...parts: string[]): Promise<string>
  mkdir(path: string): Promise<void>
  writeFile(path: string, bytes: Uint8Array): Promise<void>
  newId(): string
}

const tauriIo: CropIo = {
  dataDir: () => invoke<string>('resolve_data_dir'),
  join: (...parts) => join(...parts),
  mkdir: (path) => mkdir(path, { recursive: true }),
  writeFile: (path, bytes) => writeFile(path, bytes),
  newId: () => crypto.randomUUID(),
}

/** The bytes and extension of a `data:` URL, or null when it is not one. */
function decode(source: string): { bytes: Uint8Array; extension: string } | null {
  const match = /^data:([^;,]+);base64,(.*)$/.exec(source)
  if (!match) return null
  const binary = atob(match[2] ?? '')
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index)
  return { bytes, extension: EXTENSIONS[match[1] ?? ''] ?? 'png' }
}

/**
 * Writes each image of a quote into the archive and points the quote at the
 * file instead of at the pixels. Parts that are text, or images already kept,
 * are returned as they are.
 */
export async function storeQuoteImages(
  parts: QuotePart[],
  io: CropIo = tauriIo
): Promise<QuotePart[]> {
  if (!parts.some((part) => part.kind === 'image' && part.source.startsWith('data:'))) return parts

  const root = await io.join(await io.dataDir(), CROPS_DIR)
  await io.mkdir(root)

  const stored: QuotePart[] = []
  for (const part of parts) {
    if (part.kind !== 'image') {
      stored.push(part)
      continue
    }
    const decoded = decode(part.source)
    if (!decoded) {
      stored.push(part)
      continue
    }
    const name = `${io.newId()}.${decoded.extension}`
    await io.writeFile(await io.join(root, name), decoded.bytes)
    stored.push({ kind: 'image', source: `${CROPS_DIR}/${name}` })
  }
  return stored
}
