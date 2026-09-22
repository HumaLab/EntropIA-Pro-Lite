import { describe, expect, it } from 'vitest'
import { importWritingImage } from './writing-images'
import { imageSize } from './image-dimensions'

const PNG = new Uint8Array([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52,
  0, 0, 0, 4, 0, 0, 0, 3, 8, 6, 0, 0, 0, 0, 0, 0, 0,
])

function io() {
  const written = new Map<string, Uint8Array>()
  return {
    written,
    dataDir: async () => 'C:/datos',
    join: async (...parts: string[]) => parts.join('/'),
    mkdir: async () => undefined,
    exists: async (path: string) => written.has(path),
    writeFile: async (path: string, bytes: Uint8Array) => {
      written.set(path, bytes)
    },
  }
}

/** What WritingView.svelte's picker handler does with the bytes it read,
 *  isolated from the Tauri dialog itself: import, then size, then the attrs
 *  it will pass to `insertImage`. */
async function attrsForPicked(bytes: Uint8Array) {
  const imported = await importWritingImage(bytes, io())
  if (!imported) return null
  const size = imageSize(bytes)
  return {
    src: imported.path,
    width: size?.width ?? null,
    height: size?.height ?? null,
    align: 'center' as const,
  }
}

describe('what the toolbar picker hands the editor', () => {
  it('builds insertable attrs from a supported file', async () => {
    const attrs = await attrsForPicked(PNG)

    expect(attrs?.src).toMatch(/^writing-images\/[0-9a-f]{64}\.png$/)
    expect(attrs?.width).toBe(4)
    expect(attrs?.height).toBe(3)
  })

  it('returns null for an unsupported file, so nothing is inserted', async () => {
    expect(await attrsForPicked(new Uint8Array([1, 2, 3]))).toBeNull()
  })
})
