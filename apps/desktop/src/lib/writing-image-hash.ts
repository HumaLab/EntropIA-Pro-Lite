/**
 * SHA-256 of arbitrary bytes, as lowercase hex (spec, Managed Image Storage).
 *
 * Content addressing is what makes the same image inserted twice, in one
 * manuscript or many, resolve to one stored file: the name is the content.
 */

function bufferToHex(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer)
  let hex = ''
  for (const byte of bytes) hex += byte.toString(16).padStart(2, '0')
  return hex
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  // Copied into a fresh, non-shared ArrayBuffer: `crypto.subtle.digest`'s
  // `BufferSource` type rejects a `Uint8Array` typed over `ArrayBufferLike`
  // (which also covers `SharedArrayBuffer`), even though the caller's bytes
  // are always a plain `ArrayBuffer` in practice.
  const view = new Uint8Array(bytes.byteLength)
  view.set(bytes)
  const digest = await crypto.subtle.digest('SHA-256', view)
  return bufferToHex(digest)
}
