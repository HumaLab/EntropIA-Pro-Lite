import { describe, it, expect, vi } from 'vitest'
import { parseCssRgb, matchWindowBackground } from './window-background'

describe('parseCssRgb', () => {
  it('reads the rgb() form getComputedStyle returns', () => {
    expect(parseCssRgb('rgb(13, 17, 23)')).toEqual([13, 17, 23])
  })

  it('keeps an opaque rgba() colour', () => {
    expect(parseCssRgb('rgba(250, 249, 246, 1)')).toEqual([250, 249, 246])
  })

  it('refuses a transparent colour: painting the window with it would be white again', () => {
    expect(parseCssRgb('rgba(0, 0, 0, 0)')).toBeNull()
    expect(parseCssRgb('transparent')).toBeNull()
  })

  it('refuses anything it cannot read', () => {
    expect(parseCssRgb('')).toBeNull()
    expect(parseCssRgb('color-mix(in srgb, red, blue)')).toBeNull()
  })
})

/**
 * WebKitGTK (Linux) paints nothing for a hidden window, so the main window is
 * revealed before its first frame and, until that frame, shows the webview's
 * default background: white. Painting the window with the active theme's own
 * background first turns that frame into the app's colour.
 */
describe('matchWindowBackground', () => {
  it("paints the window with the root element's background", async () => {
    const setBackground = vi.fn().mockResolvedValue(undefined)
    document.documentElement.style.backgroundColor = 'rgb(13, 17, 23)'
    try {
      await matchWindowBackground(setBackground)
    } finally {
      document.documentElement.style.backgroundColor = ''
    }
    expect(setBackground).toHaveBeenCalledWith([13, 17, 23])
  })

  it('leaves the window alone when the page has no readable background', async () => {
    const setBackground = vi.fn().mockResolvedValue(undefined)
    document.documentElement.style.backgroundColor = 'transparent'
    try {
      await matchWindowBackground(setBackground)
    } finally {
      document.documentElement.style.backgroundColor = ''
    }
    expect(setBackground).not.toHaveBeenCalled()
  })

  it('never blocks the reveal when the window refuses the colour', async () => {
    const setBackground = vi.fn().mockRejectedValue(new Error('not permitted'))
    document.documentElement.style.backgroundColor = 'rgb(1, 2, 3)'
    try {
      await expect(matchWindowBackground(setBackground)).resolves.toBeUndefined()
    } finally {
      document.documentElement.style.backgroundColor = ''
    }
  })
})
