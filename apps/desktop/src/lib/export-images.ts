import { readFile } from '@tauri-apps/plugin-fs'
import { resolveStoredAssetPath } from './file-import'
import { childrenOf, type ExportImage, type Node } from './export-document'
import { imageSize } from './image-dimensions'

/**
 * The images a manuscript quotes, gathered and read before an export runs
 * (plan-editor.md §17).
 *
 * The three exporters are pure functions over the document: they receive
 * everything they cannot compute — the CSL renderings, the bibliography — in a
 * context object. An image is the same kind of thing. It lives in a file, and
 * reading a file is asynchronous, so it is read here, once per path, and handed
 * to the exporters already decoded.
 */

/** One quoted image, ready for every format. */
export type { ExportImage }

export interface ImageIo {
  readFile(path: string): Promise<Uint8Array>
  resolve(storedPath: string): string
}

const tauriIo: ImageIo = {
  readFile: (path) => readFile(path),
  resolve: (storedPath) => resolveStoredAssetPath(storedPath),
}

const MEDIA_TYPES: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  webp: 'image/webp',
  gif: 'image/gif',
}

/** Every image quoted anywhere in the manuscript, once, in reading order. */
export function quotedImagePaths(doc: Node): string[] {
  const paths: string[] = []
  const seen = new Set<string>()

  const walk = (node: Node) => {
    const parts = node.attrs?.quotedParts
    if (Array.isArray(parts)) {
      for (const part of parts) {
        if (!part || typeof part !== 'object') continue
        const { kind, source } = part as { kind?: unknown; source?: unknown }
        if (kind !== 'image' || typeof source !== 'string' || seen.has(source)) continue
        seen.add(source)
        paths.push(source)
      }
    }
    childrenOf(node).forEach(walk)
  }
  walk(doc)

  return paths
}

/**
 * Reads each quoted image. An image whose file is gone is left out rather than
 * refusing the whole export: a manuscript is worth more than one crop, and the
 * exporters draw the words when the image is missing.
 */
export async function loadExportImages(
  doc: Node,
  io: ImageIo = tauriIo
): Promise<Record<string, ExportImage>> {
  const images: Record<string, ExportImage> = {}

  for (const stored of quotedImagePaths(doc)) {
    let bytes: Uint8Array
    try {
      bytes = await io.readFile(io.resolve(stored))
    } catch {
      continue
    }
    const mediaType = MEDIA_TYPES[extensionOf(stored)] ?? 'image/png'
    const size = imageSize(bytes)
    images[stored] = {
      bytes,
      mediaType,
      width: size?.width ?? 0,
      height: size?.height ?? 0,
      dataUrl: `data:${mediaType};base64,${base64(bytes)}`,
    }
  }

  return images
}

function extensionOf(path: string): string {
  const match = /\.([a-z0-9]+)$/i.exec(path)
  return match ? (match[1] ?? '').toLowerCase() : ''
}

function base64(bytes: Uint8Array): string {
  // In chunks: `String.fromCharCode(...bytes)` on a crop of a scan is hundreds
  // of thousands of arguments, which blows the call stack.
  let binary = ''
  for (let index = 0; index < bytes.length; index += 8192) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 8192))
  }
  return btoa(binary)
}

export { imageSize } from './image-dimensions'
