import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { PRINT_COLORS, WRITING_COLORS } from '@entropia/ui'
import { describe, expect, it } from 'vitest'

/**
 * The manuscript palette, held equal everywhere it has to be spelled out.
 *
 * A palette name lives in four places at once: the list in `writing-colors.ts`,
 * a text and a highlight token in each theme of `tokens.css`, a print colour
 * for the exporters, and a label in each locale. Nothing at compile time
 * connects the stylesheet or the locales to the list, and each gap fails
 * quietly: a missing token draws the colour as nothing in one theme, a missing
 * label announces `writing.color.teal` to a screen reader.
 */

const TOKENS = readFileSync(
  resolve(import.meta.dirname, '../../../../packages/ui/src/tokens/tokens.css'),
  'utf-8'
)
const I18N = readFileSync(resolve(import.meta.dirname, 'i18n.ts'), 'utf-8')

/** Each theme's own rule. The dark theme is the bare `:root`. */
const THEME_RULES = [
  ':root {',
  ":root[data-theme='dim'] {",
  ":root[data-theme='light'] {",
  ":root[data-theme='lite'] {",
]

function rule(selector: string): string {
  const at = TOKENS.indexOf(selector)
  expect(at, `no rule ${selector}`).toBeGreaterThan(-1)
  return TOKENS.slice(at, TOKENS.indexOf('}', at))
}

describe('the manuscript palette', () => {
  it('is not empty, or the checks below prove nothing', () => {
    expect(WRITING_COLORS.length).toBeGreaterThanOrEqual(6)
  })

  it('has a text and a highlight token for every colour in every theme', () => {
    const missing = THEME_RULES.flatMap((selector) => {
      const body = rule(selector)
      return WRITING_COLORS.flatMap((name) =>
        [`--writing-text-${name}:`, `--writing-highlight-${name}:`]
          .filter((token) => !body.includes(token))
          .map((token) => `${selector} ${token}`)
      )
    })

    expect(missing).toEqual([])
  })

  it('has a print colour for every colour, for text and for highlight', () => {
    const missing = WRITING_COLORS.filter(
      (name) => !PRINT_COLORS[name]?.text || !PRINT_COLORS[name]?.highlight
    )

    expect(missing).toEqual([])
  })

  it('names every colour in both locales', () => {
    const unlabelled = WRITING_COLORS.filter(
      (name) => [...I18N.matchAll(new RegExp(`'writing\\.color\\.${name}':`, 'g'))].length !== 2
    )

    expect(unlabelled).toEqual([])
  })

  it('has no token for a colour the palette does not have', () => {
    const declared = new Set(
      [...TOKENS.matchAll(/--writing-(?:text|highlight)-([a-z]+):/g)].map(([, name]) => name!)
    )

    expect(
      [...declared].filter((name) => !(WRITING_COLORS as readonly string[]).includes(name))
    ).toEqual([])
  })
})
