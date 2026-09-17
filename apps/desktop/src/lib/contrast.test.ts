import { describe, expect, it } from 'vitest'
import {
  CONTRAST_CYCLE,
  CONTRAST_DEFAULT,
  contrastAttribute,
  nextContrast,
  readContrast,
} from './contrast'

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
