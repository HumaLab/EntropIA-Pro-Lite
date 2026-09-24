import { afterEach, describe, expect, it } from 'vitest'
import { initializeAppearance } from './appearance'
import { THEME_STORAGE_KEY } from './theme'
import { CONTRAST_STORAGE_KEY } from './contrast'
import { FONT_STORAGE_KEY } from './typography'

/**
 * Startup preferences, applied with no component mounted.
 *
 * Theme, contrast and typography used to apply only once the top bar (theme,
 * contrast) or the typography menu (font) mounted — both now live in the
 * Apariencia settings tab instead, which is not on screen at startup. This is
 * the app-start path that used to be implicit in those components' onMount.
 *
 * Zoom and locale already had their own startup paths (`initZoom`,
 * `initLocale` in App.svelte) and are untouched by this module.
 */

afterEach(() => {
  delete document.documentElement.dataset.theme
  delete document.documentElement.dataset.contrast
  delete document.documentElement.dataset.font
  localStorage.clear()
})

describe('initializeAppearance', () => {
  it('applies the saved theme, contrast and font preset with no TopBar mounted', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'dim')
    localStorage.setItem(CONTRAST_STORAGE_KEY, 'high')
    localStorage.setItem(FONT_STORAGE_KEY, 'modern')

    initializeAppearance()

    expect(document.documentElement.dataset.theme).toBe('dim')
    expect(document.documentElement.dataset.contrast).toBe('high')
    expect(document.documentElement.dataset.font).toBe('modern')
  })

  it('applies every default when nothing was ever saved', () => {
    initializeAppearance()

    expect(document.documentElement.dataset.theme).toBeUndefined()
    expect(document.documentElement.dataset.contrast).toBeUndefined()
    expect(document.documentElement.dataset.font).toBe('academic')
  })
})
