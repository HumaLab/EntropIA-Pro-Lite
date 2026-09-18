import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { PRINT_COLORS, WRITING_COLORS } from '../WritingEditor/writing-colors'

/**
 * Every text token, at every contrast level, measured against its own theme.
 *
 * A contrast control is the one setting in an application that can make text
 * unreadable while looking like an accessibility feature. So the values are not
 * trusted for having been computed once: they are read back out of `tokens.css`
 * and measured here, which is what makes tuning one of them safe.
 *
 * Measuring rather than reading is also how the sepia theme's muted text was
 * found at 4.40:1 — under the floor the file's own comment claimed it kept.
 */

const TOKENS = readFileSync(resolve(import.meta.dirname, '../../tokens/tokens.css'), 'utf-8')

/** WCAG AA for body text. Nothing here is allowed below it. */
const AA = 4.5

type Rgb = [number, number, number]

function parse(hex: string): Rgb {
  const value = hex.replace('#', '')
  const full =
    value.length === 3
      ? value
          .split('')
          .map((c) => c + c)
          .join('')
      : value
  return [0, 2, 4].map((at) => parseInt(full.slice(at, at + 2), 16)) as Rgb
}

function luminance([r, g, b]: Rgb): number {
  const channel = (raw: number) => {
    const c = raw / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

function ratio(a: string, b: string): number {
  const [high, low] = [luminance(parse(a)), luminance(parse(b))].sort((x, y) => y - x)
  return (high! + 0.05) / (low! + 0.05)
}

/** The body of one rule, by its exact selector. */
function block(selector: string): string {
  const at = TOKENS.indexOf(`${selector} {`)
  expect(at, `no rule for ${selector}`).toBeGreaterThan(-1)
  return TOKENS.slice(at, TOKENS.indexOf('}', at))
}

function tokenIn(body: string, name: string): string | null {
  const found = body.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{3,8})`))
  return found ? found[1]! : null
}

/** A theme's own rule, and the two departures from it. */
const THEMES = [
  {
    name: 'dark',
    base: ':root',
    soft: ":root:not([data-theme])[data-contrast='soft']",
    high: ":root:not([data-theme])[data-contrast='high']",
  },
  {
    name: 'dim',
    base: ":root[data-theme='dim']",
    soft: ":root[data-theme='dim'][data-contrast='soft']",
    high: ":root[data-theme='dim'][data-contrast='high']",
  },
  {
    name: 'light',
    base: ":root[data-theme='light']",
    soft: ":root[data-theme='light'][data-contrast='soft']",
    high: ":root[data-theme='light'][data-contrast='high']",
  },
  {
    name: 'lite',
    base: ":root[data-theme='lite']",
    soft: ":root[data-theme='lite'][data-contrast='soft']",
    high: ":root[data-theme='lite'][data-contrast='high']",
  },
] as const

const TEXT = ['color-text-primary', 'color-text-secondary', 'color-text-muted'] as const

/** What a level resolves to, falling back to the theme's own value. */
function resolved(theme: (typeof THEMES)[number], level: 'base' | 'soft' | 'high') {
  const base = block(theme.base)
  const body = level === 'base' ? base : block(theme[level])
  const background = tokenIn(base, 'color-bg')
  expect(background, `${theme.name} declares no --color-bg`).not.toBeNull()

  return TEXT.map((token) => ({
    token,
    colour: tokenIn(body, token) ?? tokenIn(base, token)!,
    background: background!,
  }))
}

describe('the contrast levels', () => {
  it('declares both departures for every theme', () => {
    for (const theme of THEMES) {
      expect(block(theme.soft), `${theme.name} soft`).toContain('--color-text-primary')
      expect(block(theme.high), `${theme.name} high`).toContain('--color-text-primary')
    }
  })

  /**
   * The rule the whole feature stands on. Below this the control is not a
   * contrast setting — it is a way to make the application fail AA while
   * wearing the badge of an accessibility feature.
   */
  it.each(THEMES.map((theme) => theme.name))(
    'keeps every text token at or above AA in the %s theme, at every level',
    (name) => {
      const theme = THEMES.find((entry) => entry.name === name)!
      const failures: string[] = []

      for (const level of ['base', 'soft', 'high'] as const) {
        for (const { token, colour, background } of resolved(theme, level)) {
          const measured = ratio(colour, background)
          if (measured < AA) {
            failures.push(
              `${level}/${token}: ${colour} on ${background} = ${measured.toFixed(2)}:1`
            )
          }
        }
      }

      expect(failures, `${name}: text below ${AA}:1`).toEqual([])
    }
  )

  /**
   * And the levels have to be in the order their names claim, or the control
   * does something other than what the person pressing it asked for.
   */
  it.each(THEMES.map((theme) => theme.name))('orders soft below high in the %s theme', (name) => {
    const theme = THEMES.find((entry) => entry.name === name)!
    const of = (level: 'base' | 'soft' | 'high') => {
      const found = resolved(theme, level).find((entry) => entry.token === 'color-text-primary')!
      return ratio(found.colour, found.background)
    }

    expect(of('soft')).toBeLessThan(of('base'))
    expect(of('base')).toBeLessThan(of('high'))
  })

  /**
   * `:not([data-theme])` on the dark blocks is load-bearing: without it they tie
   * with `:root[data-theme='dim']` on specificity and win on source order, so
   * changing contrast would drag the dark palette into the sepia theme.
   */
  it('keeps the dark contrast blocks from leaking into the other themes', () => {
    expect(TOKENS).toContain(":root:not([data-theme])[data-contrast='soft']")
    expect(TOKENS).toContain(":root:not([data-theme])[data-contrast='high']")
  })
})

/**
 * The manuscript's text colours and highlights (writing-colors.ts).
 *
 * The page they sit on is the editor's surface, so that is what they are
 * measured against, and against the app background too, since the two are a
 * shade apart. A highlight is a background, so what is measured on it is the
 * text that can end up on top: the body text at every contrast level, and every
 * text colour, since the two marks combine freely.
 */
describe('the manuscript palette', () => {
  function palette(theme: (typeof THEMES)[number]) {
    const base = block(theme.base)
    const find = (name: string) => {
      const found = tokenIn(base, name)
      expect(found, `${theme.name} declares no --${name}`).not.toBeNull()
      return found!
    }
    return {
      surfaces: [find('color-surface'), find('color-bg')],
      text: WRITING_COLORS.map((name) => ({ name, colour: find(`writing-text-${name}`) })),
      highlight: WRITING_COLORS.map((name) => ({
        name,
        colour: find(`writing-highlight-${name}`),
      })),
      body: (['base', 'soft', 'high'] as const).map((level) => ({
        level,
        colour: resolved(theme, level).find((entry) => entry.token === 'color-text-primary')!
          .colour,
      })),
    }
  }

  it.each(THEMES.map((theme) => theme.name))(
    'keeps every text colour at or above AA on the %s surface',
    (name) => {
      const { surfaces, text } = palette(THEMES.find((entry) => entry.name === name)!)
      const failures = text.flatMap(({ name: colour, colour: hex }) =>
        surfaces
          .filter((surface) => ratio(hex, surface) < AA)
          .map((surface) => `${colour} ${hex} on ${surface} = ${ratio(hex, surface).toFixed(2)}`)
      )

      expect(failures).toEqual([])
    }
  )

  it.each(THEMES.map((theme) => theme.name))(
    'keeps highlighted text at or above AA in the %s theme, coloured or not',
    (name) => {
      const { highlight, text, body } = palette(THEMES.find((entry) => entry.name === name)!)
      const inks = [
        ...body.map(({ level, colour }) => ({ name: `body/${level}`, colour })),
        ...text,
      ]
      const failures = highlight.flatMap((background) =>
        inks
          .filter((ink) => ratio(ink.colour, background.colour) < AA)
          .map(
            (ink) =>
              `${ink.name} on ${background.name} = ${ratio(ink.colour, background.colour).toFixed(2)}`
          )
      )

      expect(failures).toEqual([])
    }
  )

  /** An export is read on white paper, whatever theme it was written in. */
  it('prints every text colour at AA on white and on every printed highlight', () => {
    const failures: string[] = []
    for (const ink of WRITING_COLORS) {
      for (const paper of [
        '#ffffff',
        ...WRITING_COLORS.map((name) => PRINT_COLORS[name].highlight),
      ]) {
        const measured = ratio(PRINT_COLORS[ink].text, paper)
        if (measured < AA) failures.push(`${ink} on ${paper} = ${measured.toFixed(2)}`)
      }
    }
    for (const name of WRITING_COLORS) {
      const measured = ratio('#000000', PRINT_COLORS[name].highlight)
      if (measured < AA) failures.push(`black on ${name} = ${measured.toFixed(2)}`)
    }

    expect(failures).toEqual([])
  })
})
