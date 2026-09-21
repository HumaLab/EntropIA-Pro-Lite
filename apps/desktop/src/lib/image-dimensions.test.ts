import { describe, expect, it } from 'vitest'
import { imageSize } from './image-dimensions'

/** A 2×3 PNG: the IHDR is what the size is read from, not the pixels.
 *  Identical to export-images.test.ts's fixture — the parity proof. */
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

/** A JPEG whose first frame header declares the size. Identical to
 *  export-images.test.ts's fixture — the parity proof. */
function jpeg(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(20)
  bytes.set([0xff, 0xd8], 0)
  bytes.set([0xff, 0xfe, 0x00, 0x04, 0x00, 0x00], 2)
  bytes.set([0xff, 0xc0, 0x00, 0x11, 0x08], 8)
  const view = new DataView(bytes.buffer)
  view.setUint16(13, height)
  view.setUint16(15, width)
  return bytes
}

function gif(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(10)
  bytes.set([0x47, 0x49, 0x46, 0x38, 0x39, 0x61], 0)
  const view = new DataView(bytes.buffer)
  view.setUint16(6, width, true)
  view.setUint16(8, height, true)
  return bytes
}

describe('reading a size out of the bytes', () => {
  it('reads a PNG the same way the exporter always has', () => {
    expect(imageSize(png(40, 20))).toEqual({ width: 40, height: 20 })
  })

  it('reads a JPEG the same way the exporter always has', () => {
    expect(imageSize(jpeg(300, 150))).toEqual({ width: 300, height: 150 })
  })

  it('reads a GIF logical screen descriptor', () => {
    expect(imageSize(gif(64, 48))).toEqual({ width: 64, height: 48 })
  })

  it('refuses what it does not understand', () => {
    expect(imageSize(new Uint8Array([1, 2, 3, 4]))).toBeNull()
  })
})
