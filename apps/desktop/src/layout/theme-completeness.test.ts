import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * A theme is either complete or it is a bug (§18).
 *
 * # What this is protecting against
 *
 * A theme lives in four places at once: the palette in `tokens.css`, its two
 * contrast blocks, the cycle the button walks, and the label announced on that
 * button. Nothing at compile time connects them, and each way of half-adding a
 * theme fails quietly in its own way:
 *
 * - no palette block → the button reaches a theme that looks like the dark one;
 * - no contrast blocks → changing contrast drags another theme's text in;
 * - not in the cycle → the palette exists and nothing can reach it;
 * - no label → the button announces `lite` to a screen reader.
 *
 * The one that actually happened while adding Lite: `readPersistedTheme` had
 * the admissible values written out a second time, so the theme could be
 * reached by pressing the button and was forgotten on the next start — which
 * reads as the setting not saving rather than as a missing case.
 */

const TOKENS = readFileSync(
  resolve(import.meta.dirname, '../../../../packages/ui/src/tokens/tokens.css'),
  'utf-8'
)
// The cycle used to live in TopBar.svelte; it is now `lib/theme.ts`, applied
// at startup and consumed by the Apariencia settings tab.
const THEME_MODULE = readFileSync(resolve(import.meta.dirname, '../lib/theme.ts'), 'utf-8')

/** The themes the control actually walks, read from the cycle itself. */
function cycle(): string[] {
  const line = THEME_MODULE.match(/const THEME_CYCLE: AppTheme\[\] = \[([^\]]*)\]/)
  expect(line, 'no THEME_CYCLE in lib/theme.ts').not.toBeNull()
  return [...line![1]!.matchAll(/'([a-z]+)'/g)].map(([, name]) => name!)
}

/** The default theme is the bare `:root`, so it declares no attribute block. */
const DEFAULT_THEME = 'dark'

describe('the theme cycle', () => {
  it('walks the themes that exist, starting at the default', () => {
    expect(cycle()[0]).toBe(DEFAULT_THEME)
    expect(cycle().length).toBeGreaterThanOrEqual(4)
  })

  it('names each theme once', () => {
    expect(new Set(cycle()).size).toBe(cycle().length)
  })
})

describe('every theme in the cycle is complete', () => {
  it('has a palette', () => {
    const missing = cycle()
      .filter((theme) => theme !== DEFAULT_THEME)
      .filter((theme) => !TOKENS.includes(`:root[data-theme='${theme}'] {`))

    expect(missing, 'themes the button reaches with no palette').toEqual([])
  })

  /**
   * Both of them. A theme with no contrast blocks inherits nothing — the
   * selectors are theme-scoped — so changing contrast on it does nothing at
   * best, and at worst the dark blocks win on source order.
   */
  it('has both contrast blocks', () => {
    const missing: string[] = []
    for (const theme of cycle()) {
      for (const level of ['soft', 'high']) {
        const selector =
          theme === DEFAULT_THEME
            ? `:root:not([data-theme])[data-contrast='${level}']`
            : `:root[data-theme='${theme}'][data-contrast='${level}']`
        if (!TOKENS.includes(selector)) missing.push(`${theme}/${level}`)
      }
    }

    expect(missing, 'themes with a missing contrast level').toEqual([])
  })

  it('has a label for the button to announce', () => {
    const labels = THEME_MODULE.match(/const themeLabels: Record<AppTheme, string> = \{([^}]*)\}/)
    expect(labels, 'no themeLabels in lib/theme.ts').not.toBeNull()

    const missing = cycle().filter((theme) => !new RegExp(`\\b${theme}:`).test(labels![1]!))

    expect(missing, 'themes with no label').toEqual([])
  })
})

describe('the stored theme', () => {
  /**
   * The case that was actually broken. A second list of admissible values goes
   * stale the moment a theme is added, and the symptom — chosen, then gone on
   * restart — looks nothing like a missing case in a validator.
   */
  it('is checked against the cycle rather than against a second list', () => {
    const reader = THEME_MODULE.match(/function readPersistedTheme[\s\S]*?\n\}/)
    expect(reader, 'no readPersistedTheme').not.toBeNull()

    expect(reader![0]).toContain('THEME_CYCLE')
    // A literal other than the fallback means the list was written out again.
    const literals = [...reader![0].matchAll(/'([a-z]+)'/g)].map(([, name]) => name!)
    expect(literals.filter((name) => name !== DEFAULT_THEME)).toEqual([])
  })
})
