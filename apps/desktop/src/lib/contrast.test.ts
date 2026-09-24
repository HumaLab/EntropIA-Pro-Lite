import { afterEach, describe, expect, it } from 'vitest'
import {
  CONTRAST_CYCLE,
  CONTRAST_DEFAULT,
  CONTRAST_STORAGE_KEY,
  applyContrast,
  contrastAttribute,
  contrastLabels,
  nextContrast,
  readContrast,
  restoreContrast,
} from './contrast'

afterEach(() => {
  delete document.documentElement.dataset.contrast
  localStorage.clear()
})

/**
 * Choosing a contrast level (plan-editor.md §18).
 *
 * What each level looks like is measured in `contrast-floor.test.ts`, against
 * the tokens themselves. This is only about which one is on — and about the two
 * ways that goes wrong quietly: a cycle that stops at one end, and a stored
 * value nothing in the stylesheet answers.
 */

describe('walking the levels', () => {
  /** Forward is more contrast, so the button does what its icon suggests. */
  it('goes up and wraps round', () => {
    expect(nextContrast('soft')).toBe('normal')
    expect(nextContrast('normal')).toBe('high')
    expect(nextContrast('high')).toBe('soft')
  })

  it('reaches every level from any of them', () => {
    let level = CONTRAST_DEFAULT
    const seen = new Set([level])
    for (let step = 0; step < CONTRAST_CYCLE.length; step += 1) {
      level = nextContrast(level)
      seen.add(level)
    }

    expect([...seen].sort()).toEqual([...CONTRAST_CYCLE].sort())
  })
})

describe('reading a stored level', () => {
  it('reads one back', () => {
    expect(readContrast('high')).toBe('high')
    expect(readContrast('soft')).toBe('soft')
  })

  /**
   * `localStorage` holds strings anything may have written. An unrecognised one
   * must not become an attribute the stylesheet has no rule for — the
   * application would look as though the theme had failed to load.
   */
  it('falls back to the default rather than to something unanswerable', () => {
    expect(readContrast(null)).toBe(CONTRAST_DEFAULT)
    expect(readContrast('')).toBe(CONTRAST_DEFAULT)
    expect(readContrast('máximo')).toBe(CONTRAST_DEFAULT)
    expect(readContrast('[object Object]')).toBe(CONTRAST_DEFAULT)
  })
})

describe('what lands on the root element', () => {
  /**
   * The default has no block in `tokens.css` — it is what each theme already
   * declares — so it is expressed by the attribute's absence. Writing
   * `data-contrast="normal"` would match nothing and mean nothing.
   */
  it('removes the attribute for the default level', () => {
    expect(contrastAttribute('normal')).toBeNull()
  })

  it('names the level for the two departures', () => {
    expect(contrastAttribute('soft')).toBe('soft')
    expect(contrastAttribute('high')).toBe('high')
  })
})

describe('a label for every level', () => {
  it('has one', () => {
    for (const level of CONTRAST_CYCLE) {
      expect(contrastLabels[level]).toBeTruthy()
    }
  })
})

describe('applying a level', () => {
  it('sets the attribute for a departure level and persists it', () => {
    applyContrast('high')

    expect(document.documentElement.dataset.contrast).toBe('high')
    expect(localStorage.getItem(CONTRAST_STORAGE_KEY)).toBe('high')
  })

  it('removes the attribute for the default level', () => {
    applyContrast('high')
    applyContrast('normal')

    expect(document.documentElement.dataset.contrast).toBeUndefined()
    expect(localStorage.getItem(CONTRAST_STORAGE_KEY)).toBe('normal')
  })
})

describe('restoring at startup', () => {
  it('applies the stored level with no component mounted', () => {
    localStorage.setItem(CONTRAST_STORAGE_KEY, 'soft')

    const restored = restoreContrast()

    expect(restored).toBe('soft')
    expect(document.documentElement.dataset.contrast).toBe('soft')
  })

  it('applies the default when nothing is stored', () => {
    const restored = restoreContrast()

    expect(restored).toBe(CONTRAST_DEFAULT)
    expect(document.documentElement.dataset.contrast).toBeUndefined()
  })
})
