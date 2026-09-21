import { describe, expect, it, vi } from 'vitest'
import { importWritingImage, WRITING_IMAGES_DIR, type WritingImageIo } from './writing-images'

const PNG_HEADER = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

function pngBytes(): Uint8Array {
  return new Uint8Array([...PNG_HEADER, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 4, 0, 0, 0, 3])
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
})
