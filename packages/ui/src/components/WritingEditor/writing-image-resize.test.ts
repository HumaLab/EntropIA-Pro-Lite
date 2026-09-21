import { describe, expect, it } from 'vitest'
import { clampWritingImageWidth, MIN_WRITING_IMAGE_WIDTH } from './writing-image-resize'

describe('clamping a dragged image width', () => {
  it('keeps the aspect ratio', () => {
    const clamped = clampWritingImageWidth(400, 2, 800)
    expect(clamped).toEqual({ width: 400, height: 200 })
  })

  it('never returns more than the available width', () => {
    const clamped = clampWritingImageWidth(900, 2, 500)
    expect(clamped).toEqual({ width: 500, height: 250 })
  })

  it('refuses a negative width', () => {
    expect(clampWritingImageWidth(-10, 2, 500)).toBeNull()
  })

  it('refuses a zero width', () => {
    expect(clampWritingImageWidth(0, 2, 500)).toBeNull()
  })

  it('refuses a width below the usable minimum', () => {
    expect(clampWritingImageWidth(MIN_WRITING_IMAGE_WIDTH - 1, 2, 500)).toBeNull()
  })

  it('accepts exactly the minimum', () => {
    expect(clampWritingImageWidth(MIN_WRITING_IMAGE_WIDTH, 1, 500)).toEqual({
      width: MIN_WRITING_IMAGE_WIDTH,
      height: MIN_WRITING_IMAGE_WIDTH,
    })
  })

  it('refuses a non-finite or non-positive aspect ratio or available width', () => {
    expect(clampWritingImageWidth(200, 0, 500)).toBeNull()
    expect(clampWritingImageWidth(200, 2, 0)).toBeNull()
    expect(clampWritingImageWidth(200, Number.NaN, 500)).toBeNull()
  })
})
