import { afterEach, describe, expect, it } from 'vitest'
import {
  FONT_PRESET_DEFAULT,
  FONT_PRESETS,
  FONT_STORAGE_KEY,
  applyFontPreset,
  readFontPreset,
  restoreFontPreset,
} from './typography'

/**
 * Choosing a typography preset: the third axis beside theme and contrast.
 *
 * What each preset looks like lives in `tokens.css`, and
 * `typography-tokens.test.ts` holds the stylesheet and this registry to each
 * other. This is only about which preset is on, and about keeping it.
 */

afterEach(() => {
  delete document.documentElement.dataset.font
  localStorage.clear()
})

describe('the registry', () => {
  it('offers the four presets, academic first as the default', () => {
    expect(FONT_PRESETS.map((preset) => preset.id)).toEqual([
      'academic',
      'modern',
      'editorial',
      'archive',
    ])
    expect(FONT_PRESET_DEFAULT).toBe('academic')
  })

  it('names three families for every preset', () => {
    for (const preset of FONT_PRESETS) {
      expect(preset.families.ui, preset.id).toBeTruthy()
      expect(preset.families.reading, preset.id).toBeTruthy()
      expect(preset.families.mono, preset.id).toBeTruthy()
    }
  })
})

describe('reading a stored preset', () => {
  it('reads one back', () => {
    expect(readFontPreset('editorial')).toBe('editorial')
  })

  /** A build from before presets existed has nothing stored at all. */
  it('falls back to the default when nothing was stored', () => {
    expect(readFontPreset(null)).toBe('academic')
  })

  it('falls back to the default for a value no stylesheet answers', () => {
    expect(readFontPreset('comic-sans')).toBe('academic')
    expect(readFontPreset('')).toBe('academic')
    expect(readFontPreset('toString')).toBe('academic')
  })
})

describe('applying a preset', () => {
  it('puts it on the root element and remembers it', () => {
    applyFontPreset('modern')

    expect(document.documentElement.dataset.font).toBe('modern')
    expect(localStorage.getItem(FONT_STORAGE_KEY)).toBe('modern')
  })

  /** The default is stated, not implied: `data-font` is always present. */
  it('writes the default out too', () => {
    applyFontPreset('academic')

    expect(document.documentElement.dataset.font).toBe('academic')
  })

  it('leaves theme and contrast alone', () => {
    document.documentElement.dataset.theme = 'light'
    document.documentElement.dataset.contrast = 'high'

    applyFontPreset('archive')

    expect(document.documentElement.dataset.theme).toBe('light')
    expect(document.documentElement.dataset.contrast).toBe('high')
    delete document.documentElement.dataset.theme
    delete document.documentElement.dataset.contrast
  })
})

describe('restoring at start-up', () => {
  it('applies what was stored', () => {
    localStorage.setItem(FONT_STORAGE_KEY, 'archive')

    expect(restoreFontPreset()).toBe('archive')
    expect(document.documentElement.dataset.font).toBe('archive')
  })

  /** Existing installs keep their theme and contrast and simply gain the default. */
  it('gives an install with no stored preset the default', () => {
    localStorage.setItem('entropia-theme', 'light')

    expect(restoreFontPreset()).toBe('academic')
    expect(document.documentElement.dataset.font).toBe('academic')
    expect(localStorage.getItem('entropia-theme')).toBe('light')
  })

  it('replaces a stored value it does not recognise', () => {
    localStorage.setItem(FONT_STORAGE_KEY, 'geist')

    expect(restoreFontPreset()).toBe('academic')
    expect(localStorage.getItem(FONT_STORAGE_KEY)).toBe('academic')
  })
})
