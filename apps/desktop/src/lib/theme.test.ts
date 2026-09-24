import { afterEach, describe, expect, it } from 'vitest'
import {
  THEME_CYCLE,
  THEME_DEFAULT,
  THEME_STORAGE_KEY,
  applyTheme,
  nextTheme,
  readPersistedTheme,
  restoreTheme,
  themeLabels,
} from './theme'

/**
 * Choosing a theme, beside contrast.test.ts (plan-editor.md §18).
 *
 * The theme used to live inside TopBar.svelte and only applied once the top
 * bar mounted. It is now a plain module so it can be applied at startup with
 * no component mounted at all, and so the Apariencia tab can drive it too.
 */

afterEach(() => {
  delete document.documentElement.dataset.theme
  localStorage.clear()
})

describe('walking the cycle', () => {
  it('goes to the next theme and wraps round', () => {
    expect(nextTheme('dark')).toBe('dim')
    expect(nextTheme('dim')).toBe('light')
    expect(nextTheme('light')).toBe('lite')
    expect(nextTheme('lite')).toBe('dark')
  })

  it('reaches every theme from any of them', () => {
    let theme = THEME_DEFAULT
    const seen = new Set([theme])
    for (let step = 0; step < THEME_CYCLE.length; step += 1) {
      theme = nextTheme(theme)
      seen.add(theme)
    }
    expect([...seen].sort()).toEqual([...THEME_CYCLE].sort())
  })

  it('has a label for every theme', () => {
    for (const theme of THEME_CYCLE) {
      expect(themeLabels[theme]).toBeTruthy()
    }
  })
})

describe('reading a stored theme', () => {
  it('reads one back', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'lite')
    expect(readPersistedTheme()).toBe('lite')
  })

  it('falls back to the default rather than to something unanswerable', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'máximo')
    expect(readPersistedTheme()).toBe(THEME_DEFAULT)
  })

  it('falls back to the default when nothing is stored', () => {
    expect(readPersistedTheme()).toBe(THEME_DEFAULT)
  })
})

describe('applying a theme', () => {
  it('sets the attribute for a non-default theme and persists it', () => {
    applyTheme('dim')

    expect(document.documentElement.dataset.theme).toBe('dim')
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dim')
  })

  /**
   * The default has no block scoped to an attribute — it is the bare `:root`
   * — so it is expressed by the attribute's absence, exactly as the default
   * contrast level is.
   */
  it('removes the attribute for the default theme', () => {
    applyTheme('dim')
    applyTheme('dark')

    expect(document.documentElement.dataset.theme).toBeUndefined()
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark')
  })
})

describe('restoring at startup', () => {
  it('applies the stored theme with no component mounted', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'light')

    const restored = restoreTheme()

    expect(restored).toBe('light')
    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('applies the default when nothing is stored', () => {
    const restored = restoreTheme()

    expect(restored).toBe(THEME_DEFAULT)
    expect(document.documentElement.dataset.theme).toBeUndefined()
  })
})
