/**
 * Intrinsic pixel size, decoded from a file's own header bytes rather than
 * measured by the browser (spec, Rendering and Layout). Shared by the
 * writing-image node's insert path and by citation-quote image export, which
 * this module was extracted out of (export-images.ts:114-152, pre-extraction).
 */

export function imageSize(bytes: Uint8Array): { width: number; height: number } | null {
  return pngSize(bytes) ?? jpegSize(bytes) ?? gifSize(bytes)
}

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

function pngSize(bytes: Uint8Array): { width: number; height: number } | null {
  if (bytes.length < 24) return null
  if (PNG_SIGNATURE.some((byte, index) => bytes[index] !== byte)) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return { width: view.getUint32(16), height: view.getUint32(20) }
}

/** The frame headers that declare a JPEG's size; the rest are skipped. */
function isFrameHeader(marker: number): boolean {
  if (marker < 0xc0 || marker > 0xcf) return false
  // C4 is the Huffman table, C8 an extension, CC the arithmetic table.
  return marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc
}

function jpegSize(bytes: Uint8Array): { width: number; height: number } | null {
  if (bytes.length < 4 || bytes[0] !== 0xff || bytes[1] !== 0xd8) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)

  let offset = 2
  while (offset + 9 < bytes.length) {
    if (bytes[offset] !== 0xff) {
      offset += 1
      continue
    }
    const marker = bytes[offset + 1] ?? 0
    if (isFrameHeader(marker)) {
      return { width: view.getUint16(offset + 7), height: view.getUint16(offset + 5) }
    }
    offset += 2 + view.getUint16(offset + 2)
  }

  return null
}

function gifSize(bytes: Uint8Array): { width: number; height: number } | null {
  if (bytes.length < 10) return null
  const header = String.fromCharCode(...bytes.subarray(0, 6))
  if (header !== 'GIF87a' && header !== 'GIF89a') return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return { width: view.getUint16(6, true), height: view.getUint16(8, true) }
}
