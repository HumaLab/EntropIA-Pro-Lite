import { describe, expect, it, vi } from 'vitest'
import { importWritingImage, WRITING_IMAGES_DIR, type WritingImageIo } from './writing-images'

const PNG_HEADER = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

function pngBytes(): Uint8Array {
  return new Uint8Array([
    ...PNG_HEADER,
    0,
    0,
    0,
    13,
    0x49,
    0x48,
    0x44,
    0x52,
    0,
    0,
    0,
    4,
    0,
    0,
    0,
    3,
  ])
}

/** A JFIF-flavored JPEG: SOI, then an APP0 marker naming "JFIF". */
function jpegBytes(): Uint8Array {
  return new Uint8Array([
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00,
  ])
}

/** A GIF signature (either era) plus a minimal logical screen descriptor. */
function gifBytes(signature: 'GIF87a' | 'GIF89a'): Uint8Array {
  const header = Array.from(signature, (char) => char.charCodeAt(0))
  return new Uint8Array([...header, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00])
}

/** A real WebP container: RIFF size WEBP, followed by a VP8 chunk header —
 *  not just bytes that happen to start with "RIFF". */
function webpBytes(): Uint8Array {
  return new Uint8Array([
    0x52, 0x49, 0x46, 0x46, 0x24, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x20,
    0x18, 0x00, 0x00, 0x00,
  ])
}

/** Real SVG markup, encoded the way a dropped .svg file's bytes would be. */
function svgBytes(): Uint8Array {
  return new TextEncoder().encode(
    '<?xml version="1.0" encoding="UTF-8"?><svg xmlns="http://www.w3.org/2000/svg"></svg>'
  )
}

function fakeIo(existingPaths: Set<string> = new Set()): WritingImageIo & {
  written: Map<string, Uint8Array>
} {
  const written = new Map<string, Uint8Array>()
  return {
    written,
    dataDir: vi.fn(async () => 'C:/datos'),
    join: vi.fn(async (...parts: string[]) => parts.join('/')),
    mkdir: vi.fn(async () => undefined),
    exists: vi.fn(async (path: string) => existingPaths.has(path) || written.has(path)),
    writeFile: vi.fn(async (path: string, bytes: Uint8Array) => {
      written.set(path, bytes)
    }),
  }
}

describe('importing a manuscript image', () => {
  it('stores it under writing-images/, named by its own hash', async () => {
    const io = fakeIo()

    const result = await importWritingImage(pngBytes(), io)

    expect(result).not.toBeNull()
    expect(result?.mediaType).toBe('image/png')
    expect(result?.path).toMatch(new RegExp(`^${WRITING_IMAGES_DIR}/[0-9a-f]{64}\\.png$`))
  })

  it('is a relative path with no drive letter or home directory', async () => {
    const result = await importWritingImage(pngBytes(), fakeIo())

    expect(result?.path.startsWith('/')).toBe(false)
    expect(result?.path).not.toMatch(/^[a-zA-Z]:/)
    expect(result?.path).not.toContain('\\')
  })

  it('writes the same bytes only once', async () => {
    const io = fakeIo()

    const first = await importWritingImage(pngBytes(), io)
    const second = await importWritingImage(pngBytes(), io)

    expect(second?.path).toBe(first?.path)
    expect(io.writeFile).toHaveBeenCalledTimes(1)
  })

  it('refuses a format it was not built to store, and stores nothing', async () => {
    const io = fakeIo()

    const result = await importWritingImage(new Uint8Array([1, 2, 3, 4]), io)

    expect(result).toBeNull()
    expect(io.writeFile).not.toHaveBeenCalled()
  })

  it('accepts a JPEG, named by its own hash with a .jpg extension', async () => {
    const result = await importWritingImage(jpegBytes(), fakeIo())

    expect(result?.mediaType).toBe('image/jpeg')
    expect(result?.path).toMatch(new RegExp(`^${WRITING_IMAGES_DIR}/[0-9a-f]{64}\\.jpg$`))
  })

  it('accepts a GIF87a, named by its own hash with a .gif extension', async () => {
    const result = await importWritingImage(gifBytes('GIF87a'), fakeIo())

    expect(result?.mediaType).toBe('image/gif')
    expect(result?.path).toMatch(new RegExp(`^${WRITING_IMAGES_DIR}/[0-9a-f]{64}\\.gif$`))
  })

  it('accepts a GIF89a, named by its own hash with a .gif extension', async () => {
    const result = await importWritingImage(gifBytes('GIF89a'), fakeIo())

    expect(result?.mediaType).toBe('image/gif')
    expect(result?.path).toMatch(new RegExp(`^${WRITING_IMAGES_DIR}/[0-9a-f]{64}\\.gif$`))
  })

  it('refuses a real WebP container, and stores nothing', async () => {
    const io = fakeIo()

    const result = await importWritingImage(webpBytes(), io)

    expect(result).toBeNull()
    expect(io.writeFile).not.toHaveBeenCalled()
  })

  it('refuses real SVG markup, and stores nothing', async () => {
    const io = fakeIo()

    const result = await importWritingImage(svgBytes(), io)

    expect(result).toBeNull()
    expect(io.writeFile).not.toHaveBeenCalled()
  })
})
