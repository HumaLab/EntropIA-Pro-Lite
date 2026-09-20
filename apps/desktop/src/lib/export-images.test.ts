import { describe, expect, it, vi } from 'vitest'
import { imageSize, loadExportImages, quotedImagePaths } from './export-images'
import type { Node } from './export-document'

/** A 2×3 PNG: the IHDR is what the size is read from, not the pixels. */
function png(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(33)
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0)
  const view = new DataView(bytes.buffer)
  view.setUint32(8, 13)
  bytes.set([0x49, 0x48, 0x44, 0x52], 12)
  view.setUint32(16, width)
  view.setUint32(20, height)
  return bytes
}

/** A JPEG whose first frame header declares the size. */
function jpeg(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(20)
  bytes.set([0xff, 0xd8], 0)
  // A comment segment first, so the reader has to walk the segments.
  bytes.set([0xff, 0xfe, 0x00, 0x04, 0x00, 0x00], 2)
  bytes.set([0xff, 0xc0, 0x00, 0x11, 0x08], 8)
  const view = new DataView(bytes.buffer)
  view.setUint16(13, height)
  view.setUint16(15, width)
  return bytes
}

function citation(parts: unknown): Node {
  return { type: 'documentCitation', attrs: { quotedText: 'algo', quotedParts: parts } }
}

function doc(...content: Node[]): Node {
  return { type: 'doc', content: [{ type: 'paragraph', content }] }
}

describe('the images a manuscript quotes', () => {
  it('finds every quoted image, once, in the order they are read', () => {
    const manuscript = doc(
      citation([
        { kind: 'text', text: 'antes' },
        { kind: 'image', source: 'writing-crops/uno.png' },
      ]),
      { type: 'text', text: ' entre ' },
      citation([
        { kind: 'image', source: 'writing-crops/dos.jpg' },
        { kind: 'image', source: 'writing-crops/uno.png' },
      ])
    )

    expect(quotedImagePaths(manuscript)).toEqual([
      'writing-crops/uno.png',
      'writing-crops/dos.jpg',
    ])
  })

  it('ignores a citation that quotes only words', () => {
    expect(quotedImagePaths(doc(citation(null), citation([{ kind: 'text', text: 'hola' }])))).toEqual(
      []
    )
  })
})

describe('loading a quoted image for an export', () => {
  function io(files: Record<string, Uint8Array>) {
    return {
      readFile: vi.fn(async (path: string) => {
        const bytes = files[path]
        if (!bytes) throw new Error(`no such file: ${path}`)
        return bytes
      }),
      resolve: (stored: string) => `C:/datos/${stored}`,
    }
  }

  it('reads the file and says how big it is and what it holds', async () => {
    const disk = io({ 'C:/datos/writing-crops/uno.png': png(40, 20) })

    const images = await loadExportImages(doc(citation([{ kind: 'image', source: 'writing-crops/uno.png' }])), disk)

    const image = images['writing-crops/uno.png']
    expect(image?.mediaType).toBe('image/png')
    expect(image?.width).toBe(40)
    expect(image?.height).toBe(20)
    // Embedded once here, so the three exporters stay pure and synchronous.
    expect(image?.dataUrl.startsWith('data:image/png;base64,')).toBe(true)
  })

  it('leaves out an image whose file is gone, rather than refusing the export', async () => {
    const disk = io({})

    const images = await loadExportImages(
      doc(citation([{ kind: 'image', source: 'writing-crops/ida.png' }])),
      disk
    )

    expect(images).toEqual({})
  })

  it('reads the size of a JPEG too', async () => {
    const disk = io({ 'C:/datos/writing-crops/dos.jpg': jpeg(300, 150) })

    const images = await loadExportImages(
      doc(citation([{ kind: 'image', source: 'writing-crops/dos.jpg' }])),
      disk
    )

    expect(images['writing-crops/dos.jpg']).toMatchObject({
      mediaType: 'image/jpeg',
      width: 300,
      height: 150,
    })
  })

  it('reads nothing when the manuscript quotes no image', async () => {
    const disk = io({ 'C:/datos/writing-crops/uno.png': png(1, 1) })

    expect(await loadExportImages(doc(citation(null)), disk)).toEqual({})
    expect(disk.readFile).not.toHaveBeenCalled()
  })
})

describe('reading a size out of the bytes', () => {
  it('refuses what it does not understand', () => {
    expect(imageSize(new Uint8Array([1, 2, 3, 4]))).toBeNull()
  })
})
