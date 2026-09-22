import { beforeEach, describe, expect, it, vi } from 'vitest'
import { pickWritingImage } from './writing-image-picker'
import type { WritingImageIo } from './writing-images'

const PNG = new Uint8Array([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52,
  0, 0, 0, 4, 0, 0, 0, 3, 8, 6, 0, 0, 0, 0, 0, 0, 0,
])

/** The same shape writing-images.test.ts injects — storage is Task 3's own
 *  tested seam, not something this picker re-proves. */
function fakeIo(): WritingImageIo & { written: Map<string, Uint8Array> } {
  const written = new Map<string, Uint8Array>()
  return {
    written,
    dataDir: vi.fn(async () => 'C:/datos'),
    join: vi.fn(async (...parts: string[]) => parts.join('/')),
    mkdir: vi.fn(async () => undefined),
    exists: vi.fn(async (path: string) => written.has(path)),
    writeFile: vi.fn(async (path: string, bytes: Uint8Array) => {
      written.set(path, bytes)
    }),
  }
}

describe('the toolbar picker', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('filters the dialog to the accepted formats, single selection only', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    vi.mocked(open).mockResolvedValue(null)

    await pickWritingImage(fakeIo())

    expect(open).toHaveBeenCalledWith(
      expect.objectContaining({
        multiple: false,
        filters: [expect.objectContaining({ extensions: ['png', 'jpg', 'jpeg', 'gif'] })],
      })
    )
  })

  it('returns null without reading anything when the user cancels', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const { readFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(open).mockResolvedValue(null)

    const attrs = await pickWritingImage(fakeIo())

    expect(attrs).toBeNull()
    expect(readFile).not.toHaveBeenCalled()
  })

  it('builds insertable attrs from a supported file', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const { readFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(open).mockResolvedValue('C:/photos/sunset.png')
    vi.mocked(readFile).mockResolvedValue(PNG)

    const attrs = await pickWritingImage(fakeIo())

    expect(attrs?.src).toMatch(/^writing-images\/[0-9a-f]{64}\.png$/)
    expect(attrs?.width).toBe(4)
    expect(attrs?.height).toBe(3)
    expect(attrs?.align).toBe('center')
  })

  it('returns null for an unsupported file, writes nothing, and logs the refusal', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const { readFile } = await import('@tauri-apps/plugin-fs')
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(open).mockResolvedValue('C:/photos/notes.txt')
    vi.mocked(readFile).mockResolvedValue(new Uint8Array([1, 2, 3]))
    const io = fakeIo()

    const attrs = await pickWritingImage(io)
    // appendLog fires-and-forgets; give its microtask a turn to run.
    await Promise.resolve()
    await Promise.resolve()

    expect(attrs).toBeNull()
    expect(io.writeFile).not.toHaveBeenCalled()
    expect(invoke).toHaveBeenCalledWith(
      'logs_append',
      expect.objectContaining({ level: 'error', source: 'writing-image' })
    )
  })
})
