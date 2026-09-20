import { describe, expect, it, vi } from 'vitest'
import { CROPS_DIR, storeQuoteImages } from './writing-crops'
import type { QuotePart } from './rendered-selection'

const PNG = 'data:image/png;base64,iVBORw0KGgo='

function io() {
  const written: Array<{ path: string; bytes: Uint8Array }> = []
  let next = 0
  return {
    written,
    dataDir: vi.fn(async () => 'C:/datos'),
    join: vi.fn(async (...parts: string[]) => parts.join('/')),
    mkdir: vi.fn(async () => {}),
    writeFile: vi.fn(async (path: string, bytes: Uint8Array) => {
      written.push({ path, bytes })
    }),
    newId: () => `crop-${++next}`,
  }
}

describe('keeping the images a quote took in', () => {
  it('writes each image into the archive and points the quote at the file', async () => {
    const parts: QuotePart[] = [
      { kind: 'text', text: 'antes' },
      { kind: 'image', source: PNG },
      { kind: 'text', text: 'después' },
    ]
    const disk = io()

    const stored = await storeQuoteImages(parts, disk)

    expect(stored).toEqual([
      { kind: 'text', text: 'antes' },
      { kind: 'image', source: `${CROPS_DIR}/crop-1.png` },
      { kind: 'text', text: 'después' },
    ])
    expect(disk.written).toHaveLength(1)
    expect(disk.written[0]?.path).toBe(`C:/datos/${CROPS_DIR}/crop-1.png`)
    // "iVBORw0KGgo=" is the PNG signature: what was decoded is what was read.
    expect([...(disk.written[0]?.bytes ?? [])].slice(0, 4)).toEqual([0x89, 0x50, 0x4e, 0x47])
  })

  it('gives every image its own file', async () => {
    const disk = io()

    const stored = await storeQuoteImages(
      [
        { kind: 'image', source: PNG },
        { kind: 'image', source: PNG },
      ],
      disk
    )

    expect(stored.map((part) => (part.kind === 'image' ? part.source : ''))).toEqual([
      `${CROPS_DIR}/crop-1.png`,
      `${CROPS_DIR}/crop-2.png`,
    ])
  })

  it('keeps the format the renderer produced', async () => {
    const disk = io()

    const stored = await storeQuoteImages(
      [{ kind: 'image', source: 'data:image/jpeg;base64,/9j/4AAQ' }],
      disk
    )

    expect(stored[0]).toEqual({ kind: 'image', source: `${CROPS_DIR}/crop-1.jpg` })
  })

  it('leaves alone an image that is already a file, and text', async () => {
    const disk = io()
    const parts: QuotePart[] = [
      { kind: 'text', text: 'solo palabras' },
      { kind: 'image', source: `${CROPS_DIR}/ya-guardada.png` },
    ]

    expect(await storeQuoteImages(parts, disk)).toEqual(parts)
    expect(disk.writeFile).not.toHaveBeenCalled()
  })
})
