import { readdirSync, readFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const TOKEN_DEFINITION = /(--[a-z0-9-]+)\s*:/g
const TOKEN_USE_WITHOUT_FALLBACK = /var\((--[a-z0-9-]+)\s*\)/g

/**
 * Every custom property a desktop view may name: the design system's tokens
 * plus the app's own sheet. A `var()` with no fallback that names anything
 * else is dead on arrival — the browser drops the whole declaration, silently,
 * and the element quietly inherits instead.
 */
function publishedTokens(): Set<string> {
  const tokens = new Set<string>()
  const sheets = [
    ...readdirSync(resolve(import.meta.dirname, '../../../packages/ui/src/tokens'))
      .filter((name) => name.endsWith('.css'))
      .map((name) => resolve(import.meta.dirname, '../../../packages/ui/src/tokens', name)),
    resolve(import.meta.dirname, 'app.css'),
  ]

  for (const sheet of sheets) {
    for (const [, token] of readFileSync(sheet, 'utf-8').matchAll(TOKEN_DEFINITION)) {
      tokens.add(token)
    }
  }

  return tokens
}

function componentsUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) return componentsUnder(full)
    return entry.name.endsWith('.svelte') ? [full] : []
  })
}

/**
 * Every component the user can end up looking at, wherever it is authored. The
 * button rules below describe the app's surface, not one package's, and a rule
 * that only watched half of it would keep passing while the other half drifted.
 */
function everyComponent(): string[] {
  return [
    ...componentsUnder(import.meta.dirname),
    ...componentsUnder(resolve(import.meta.dirname, '../../../packages/ui/src')),
  ]
}

function undefinedTokensIn(componentPath: string, published: Set<string>): string[] {
  const source = readFileSync(componentPath, 'utf-8')
  const styles = source.split('<style>').slice(1).join('<style>')
  if (!styles) return []

  // A component may declare its own properties; those are not system tokens.
  const local = new Set(Array.from(source.matchAll(TOKEN_DEFINITION), ([, token]) => token))

  return Array.from(
    new Set(
      Array.from(styles.matchAll(TOKEN_USE_WITHOUT_FALLBACK), ([, token]) => token).filter(
        (token) => !published.has(token) && !local.has(token),
      ),
    ),
  )
}

/**
 * Colour on a button surface has to mean something. Success, info and warning
 * describe an outcome, never the action that starts one, so tinting a button
 * with them says nothing — and four such buttons in a row read as a rainbow
 * toolbar rather than one system. Two families stay allowed: danger, whose
 * colour IS its meaning, and the accent wash, which is the app's neutral hover
 * on every ghost control. Thin accents — borders, text, badges, progress fills
 * — are untouched by this rule; only the surface is.
 */
const DECORATIVE_TINT = /background(-color)?:\s*var\(--color-(success|info|warning)[a-z-]*\)/
const CSS_RULE = /([^{}]+)\{([^{}]*)\}/g

function tintedButtonRulesIn(componentPath: string): string[] {
  const styles = readFileSync(componentPath, 'utf-8').split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(([selector, body]) => /btn|button/i.test(selector) && DECORATIVE_TINT.test(body))
    .map(([selector]) => selector.trim().replace(/\s+/g, ' '))
}

/**
 * The control scale, in pixels: IconButton's containers (24/28/32/40, one per
 * icon step) and Button's heights (30/36/40), plus 44 for the banner step.
 * A button sized off this ladder is the drift that makes two neighbouring
 * controls look like they came from different apps, so a view that hardcodes
 * its own height has to land on a rung.
 *
 * Known limits, both of them narrowing what a green run proves:
 *  - The button rules find their subjects by selector NAME, so a control class
 *    called something else — `.explorer__chevron`, say — is invisible to them.
 *  - A height reaches this check only as a literal or as one of the three
 *    control-height tokens below. Any other `calc()`, `clamp()` or token
 *    arithmetic passes unread.
 * Passing is evidence about the rules these can see, not a survey of every
 * clickable surface in the app.
 */
const CONTROL_SCALE_PX = new Set([24, 28, 30, 32, 36, 40, 44])
const BUTTON_HEIGHT = /(?:min-)?height:\s*(\d+px|var\(--control-height-(?:sm|md|lg)\))/g

/** The control-height tokens, in the pixels they actually resolve to. */
const CONTROL_HEIGHT_TOKENS: Record<string, number> = {
  'var(--control-height-sm)': 30,
  'var(--control-height-md)': 36,
  'var(--control-height-lg)': 40,
}

function heightInPx(value: string): number {
  return CONTROL_HEIGHT_TOKENS[value] ?? Number.parseInt(value, 10)
}

/** A BEM child of a button — `.btn__spinner` — is furniture inside the control,
 *  not the control, and it is sized to whatever it draws. */
const BUTTON_CHILD = /\.(btn|button)__/i

/**
 * Controls that earn their size from their job rather than the ladder. Each
 * entry states why, because an exception with no reason is just a hole.
 */
const OFF_SCALE_BY_DESIGN: Record<string, string> = {
  '.audio-player__btn--play':
    "transport control: the play button is the player's one hero target and is sized for the thumb, not for a toolbar row",
  '.entity-viewer__action':
    'sized by its container, not the ladder: it sits inside a 24px chip whose content box is 20px, so the next rung up would push every chip to 28px',
}

function offScaleButtonHeightsIn(componentPath: string): string[] {
  const styles = readFileSync(componentPath, 'utf-8').split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(
      ([selector]) =>
        /btn|button|action(?!s\b)/i.test(selector) &&
        !BUTTON_CHILD.test(selector) &&
        !(selector.trim() in OFF_SCALE_BY_DESIGN)
    )
    .flatMap(([selector, body]) =>
      Array.from(body.matchAll(BUTTON_HEIGHT))
        .filter((match) => !CONTROL_SCALE_PX.has(heightInPx(match[1] ?? '')))
        .map((match) => `${selector.trim().replace(/\s+/g, ' ')} -> ${match[0]}`)
    )
}

/**
 * Keyboard focus is the only thing telling someone who does not use a mouse
 * where they are, so it has to look the same everywhere. A view is free to
 * skip the ring; what it may not do is draw its own.
 *
 * The ring may sit anywhere in the shadow list — a dialog composes it with its
 * own elevation — so what is checked is that it is in there at all.
 */
const FOCUS_SHADOW = /box-shadow:\s*([^;}]+)/

function bespokeFocusRingsIn(componentPath: string): string[] {
  const styles = readFileSync(componentPath, 'utf-8').split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(([selector, body]) => {
      if (!selector.includes(':focus-visible')) return false
      const shadow = body.match(FOCUS_SHADOW)?.[1]?.trim()
      // `none` is a rule clearing an inherited shadow, not drawing a ring.
      if (!shadow || shadow === 'none') return false
      return !shadow.includes('var(--focus-ring')
    })
    .map(([selector]) => selector.trim().replace(/\s+/g, ' '))
}

describe('desktop design tokens', () => {
  it('draws keyboard focus with the shared ring', () => {
    const offenders = everyComponent().flatMap((path) =>
      bespokeFocusRingsIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('sizes action buttons from the control scale', () => {
    const offenders = everyComponent().flatMap((path) =>
      offScaleButtonHeightsIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('keeps action buttons off decorative semantic tints', () => {
    const offenders = everyComponent().flatMap((path) =>
      tintedButtonRulesIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('names only published tokens in var() calls that carry no fallback', () => {
    const published = publishedTokens()
    const offenders = componentsUnder(import.meta.dirname)
      .map((path) => [path, undefinedTokensIn(path, published)] as const)
      .filter(([, missing]) => missing.length > 0)
      .map(([path, missing]) => `${basename(path)}: ${missing.join(', ')}`)

    expect(offenders).toEqual([])
  })
})
